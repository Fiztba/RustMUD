//! The combat message editor.
//!
//! One slot per attack type (60 of them), each holding a list of message
//! records the fight code picks from at random. `msg_index` is the cursor
//! into that list. The loader prepends, so the Vec is
//! in reverse file order and writing it back in Vec order reproduces the
//! file — the same order `for (msg = list->msg; msg; msg = msg->next)`
//! walks.
//!
//! The list is copied on the way in AND on the way out, prepending as it
//! walks, so the working copy is in reverse table order
//! and the save reverses it back. Those two cancel only while the list is
//! untouched: `N` appends to the END of the working copy, so an added record
//! lands at the FRONT of the table and is written first. Both reversals are
//! reproduced here; skipping them put an appended record at the wrong end of
//! lib/misc/messages, which is how the file comparison caught it.
//!
//! For a slot with no messages, the copy seeds one blank record
//! and leaves `number_of_attacks` at the source's zero — so an empty slot
//! opens editable and reads `[45x0]`.
//!
//! One shape is fixed rather than kept:
//!
//! * **B64**: the `quit` flag that carries "the builder asked to leave"
//! into the save confirmation is a file-scope `static bool`, shared by
//! every descriptor in the editor. It lives per-descriptor here. Filed
//! from the declaration, not from a run: it takes two builders in the
//! editor at once to see it.

use mud_data::ids::CharId;
use mud_data::spells::{skill_name, TOP_SPELL_DEFINE};
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::{act, send_to_char, write_to_desc, TO_ROOM};
use crate::fight::{MessageType, MAX_MESSAGES};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::interpreter::{delete_doubledollar, skip_spaces};
use crate::olc::{genolc_checkstring, get_char_colors, OlcData, CLEANUP_ALL};

pub const MSGEDIT_MAIN_MENU: i32 = 1;
pub const MSGEDIT_CONFIRM_SAVE: i32 = 2;
pub const MSGEDIT_TYPE: i32 = 3;
pub const MSGEDIT_DEATH_CHAR: i32 = 4;
pub const MSGEDIT_DEATH_VICT: i32 = 5;
pub const MSGEDIT_DEATH_ROOM: i32 = 6;
pub const MSGEDIT_MISS_CHAR: i32 = 7;
pub const MSGEDIT_MISS_VICT: i32 = 8;
pub const MSGEDIT_MISS_ROOM: i32 = 9;
pub const MSGEDIT_HIT_CHAR: i32 = 10;
pub const MSGEDIT_HIT_VICT: i32 = 11;
pub const MSGEDIT_HIT_ROOM: i32 = 12;
pub const MSGEDIT_GOD_CHAR: i32 = 13;
pub const MSGEDIT_GOD_VICT: i32 = 14;
pub const MSGEDIT_GOD_ROOM: i32 = 15;
pub const MSGEDIT_CONFIRM_DELETE: i32 = 16;

fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

/// The name the menu and the listing put beside an attack type. The test
/// is
/// `a_type < TOP_SPELL_DEFINE` with no lower bound, so slot 0 renders
/// spell_info[0].
fn a_type_name(a_type: i32) -> &'static str {
    if a_type < TOP_SPELL_DEFINE {
        mud_data::spells::spell_info(a_type).name
    } else {
        "Unknown"
    }
}

/// PRINT_MSG: a NULL message is written and shown as "#".
fn print_msg(m: &Option<BStr>) -> &[u8] {
    match m {
        Some(t) => t,
        None => b"#",
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

fn show_messages(g: &mut Game, chid: CharId) {
    let mut out: BStr = b"\t1Message List:\tn \r\n".to_vec();
    let mut count = 0i32;
    let half_start = MAX_MESSAGES / 2;
    for i in 0..half_start {
        let half = half_start + i;
        let filled = !g.fight_messages[i].msg.is_empty();
        let right_filled = half < MAX_MESSAGES && !g.fight_messages[half].msg.is_empty();
        if !filled {
            // The two columns are printed independently: printing the
            // right-hand entry from inside the left-hand one would let an
            // empty slot on the left hide a live attack type on the right,
            // in the only listing the editor offers. Nothing
            // could empty a slot before the deletes below, so nothing
            // had ever run into it. It takes a row of its own rather
            // than being padded across, which keeps the format the same
            // as the paired line further down.
            if right_filled {
                let r = &g.fight_messages[half];
                out.extend_from_slice(
                    format!(
                        "{:<2}) [{:<3}] {}, {:<18}\r\n",
                        half,
                        r.a_type,
                        r.number_of_attacks,
                        a_type_name(r.a_type)
                    )
                    .as_bytes(),
                );
            }
            continue;
        }
        count += g.fight_messages[i].number_of_attacks;
        let l = &g.fight_messages[i];
        out.extend_from_slice(
            format!(
                "{:<2}) [{:<3}] {}, {:<18}{}",
                i,
                l.a_type,
                l.number_of_attacks,
                a_type_name(l.a_type),
                if right_filled { "   " } else { "\r\n" }
            )
            .as_bytes(),
        );
        if right_filled {
            let r = &g.fight_messages[half];
            out.extend_from_slice(
                format!(
                    "{:<2}) [{:<3}] {}, {:<18}\r\n",
                    half,
                    r.a_type,
                    r.number_of_attacks,
                    a_type_name(r.a_type)
                )
                .as_bytes(),
            );
        }
    }
    out.extend_from_slice(format!("Total Messages: {}\r\n", count).as_bytes());
    crate::act::informative::page_string(g, chid, &out);
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn save_messages_to_disk(g: &mut Game) {
    let mut out: BStr = Vec::new();
    // The on-disk header. Rebranding it would change the file format, so
    // it stays until that is a deliberate change.
    out.extend_from_slice(b"* TBAMUD 3.64 Combat Message File\n");

    for i in 0..MAX_MESSAGES {
        if g.fight_messages[i].msg.is_empty() {
            continue;
        }
        let a_type = g.fight_messages[i].a_type;
        if a_type > 0 && a_type < TOP_SPELL_DEFINE {
            out.extend_from_slice(
                format!("* {} {}\n", skill_name(a_type), a_type).as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("* {}\n", a_type).as_bytes());
        }
        for m in &g.fight_messages[i].msg {
            out.extend_from_slice(b"M\n");
            out.extend_from_slice(format!("{}\n", a_type).as_bytes());
            for t in [&m.die, &m.miss, &m.hit, &m.god] {
                for field in [&t.attacker, &t.victim, &t.room] {
                    out.extend_from_slice(print_msg(field));
                    out.push(b'\n');
                }
            }
            out.push(b'\n');
        }
    }

    let path = g.lib_dir.join("misc").join("messages");
    if std::fs::write(&path, &out).is_err() {
        // Log and carry on: a failed write here is not worth taking the
        // MUD down for.
        g.log(format!("SYSERR: Error writing combat message file {}", path.display()));
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_msgedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let arg = skip_spaces(argument);
    if arg.is_empty() {
        show_messages(g, chid);
        return;
    }

    let num = atoi(arg);
    if num < 0 {
        let msg = format!("You must select a message # between 0 and {}.\r\n", MAX_MESSAGES);
        send_to_char(g, chid, msg.as_bytes());
        return;
    }
    if num >= MAX_MESSAGES as i32 {
        let msg =
            format!("You must select a message # between 0 and {}.\r\n", MAX_MESSAGES - 1);
        send_to_char(g, chid, msg.as_bytes());
        return;
    }

    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Msgedit) {
            continue;
        }
        let editing = crate::olc::olc_of(g, other)
            .map(|o| o.msg_list.is_some() && o.number == num)
            .unwrap_or(false);
        if editing {
            send_to_char(g, chid, b"Someone is already editing that message.\r\n");
            return;
        }
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_msg_edit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();
    olc.number = num;
    olc.value = 0;

    // The working copy is the slot's list reversed. An empty slot gets one
    // blank record while keeping the source's attack count of zero.
    let mut list = g.fight_messages[num as usize].clone();
    list.msg.reverse();
    if list.msg.is_empty() {
        list.msg.push(MessageType::default());
    }
    olc.msg_list = Some(Box::new(list));
    olc.msg_index = 0;

    msgedit_main_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Msgedit;
    }

    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(mud_data::flags::PLR_WRITING);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    g.mudlog(
        MudlogKind::Cmp,
        level,
        true,
        &format!("OLC: {} starts editing message {}", name, num),
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

fn msgedit_main_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    let c = g.olc_colors;
    let (nrm, grn, cyn, yel) = (
        c.nrm_s().to_string(),
        c.grn_s().to_string(),
        c.cyn_s().to_string(),
        c.yel_s().to_string(),
    );
    let list = olc.msg_list.as_ref().unwrap();
    let m = list.msg[olc.msg_index].clone();
    let (a_type, attacks) = (list.a_type, list.number_of_attacks);
    let has_next = olc.msg_index + 1 < list.msg.len();
    let at_first = olc.msg_index == 0;
    let n_sets = list.msg.len();

    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!(
            "{}Msg Edit: {}[{}{}x{}{}] [{}$n: Attacker | $N: Victim{}]{}\r\n",
            cyn, grn, yel, olc.number, attacks, grn, yel, grn, nrm
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{}1{}) {}Action Type: {}{} {}[{}{}{}]{}\r\n",
            grn,
            yel,
            cyn,
            yel,
            a_type,
            grn,
            yel,
            a_type_name(a_type),
            grn,
            nrm
        )
        .as_bytes(),
    );

    for (title, keys, triple) in [
        ("Death", [b'A', b'B', b'C'], &m.die),
        ("Miss", [b'D', b'E', b'F'], &m.miss),
        ("Hit", [b'G', b'H', b'I'], &m.hit),
        ("God", [b'J', b'K', b'L'], &m.god),
    ] {
        out.extend_from_slice(format!("   {}{} Messages:\r\n", cyn, title).as_bytes());
        for (k, field, label) in [
            (keys[0], &triple.attacker, "CHAR"),
            (keys[1], &triple.victim, "VICT"),
            (keys[2], &triple.room, "ROOM"),
        ] {
            out.extend_from_slice(format!("{}{}{}) {} : {} ", grn, k as char, yel, label, nrm).as_bytes());
            out.extend_from_slice(print_msg(field));
            out.extend_from_slice(b"\r\n");
        }
    }

    out.extend_from_slice(
        format!(
            "\r\n{}N{}){} {}",
            grn,
            yel,
            nrm,
            if has_next { "Next" } else { "New" }
        )
        .as_bytes(),
    );
    if !at_first {
        out.extend_from_slice(format!(" {}P{}){} Previous", grn, yel, nrm).as_bytes());
    }
    if n_sets > 1 {
        out.extend_from_slice(format!(" {}X{}){} Delete Set", grn, yel, nrm).as_bytes());
    }
    out.extend_from_slice(format!(" {}Z{}){} Delete Type", grn, yel, nrm).as_bytes());
    if olc.value != 0 {
        out.extend_from_slice(format!(" {}S{}){} Save", grn, yel, nrm).as_bytes());
    }
    out.extend_from_slice(
        format!(" {}Q{}){} Quit\r\nEnter Selection : ", grn, yel, nrm).as_bytes(),
    );
    write_to_desc(g, di, &out);
    olc.mode = MSGEDIT_MAIN_MENU;
}

/// Which message field a main-menu letter opens.
fn field_mode(c: u8) -> Option<i32> {
    Some(match c.to_ascii_uppercase() {
        b'A' => MSGEDIT_DEATH_CHAR,
        b'B' => MSGEDIT_DEATH_VICT,
        b'C' => MSGEDIT_DEATH_ROOM,
        b'D' => MSGEDIT_MISS_CHAR,
        b'E' => MSGEDIT_MISS_VICT,
        b'F' => MSGEDIT_MISS_ROOM,
        b'G' => MSGEDIT_HIT_CHAR,
        b'H' => MSGEDIT_HIT_VICT,
        b'I' => MSGEDIT_HIT_ROOM,
        b'J' => MSGEDIT_GOD_CHAR,
        b'K' => MSGEDIT_GOD_VICT,
        b'L' => MSGEDIT_GOD_ROOM,
        _ => return None,
    })
}

/// The example line each field prompt leads with.
fn field_example(mode: i32) -> &'static [u8] {
    match mode {
        MSGEDIT_DEATH_CHAR => b"Example: You kill $N!\r\n",
        MSGEDIT_DEATH_VICT => b"Example: $n kills you!\r\n",
        MSGEDIT_DEATH_ROOM => b"Example: $n kills $N!\r\n",
        MSGEDIT_MISS_CHAR => b"Example: You miss $N!\r\n",
        MSGEDIT_MISS_VICT => b"Example: $n misses you!\r\n",
        MSGEDIT_MISS_ROOM => b"Example: $n misses $N!\r\n",
        MSGEDIT_HIT_CHAR => b"Example: You hit $N!\r\n",
        MSGEDIT_HIT_VICT => b"Example: $n hits you!\r\n",
        MSGEDIT_HIT_ROOM => b"Example: $n hits $N!\r\n",
        MSGEDIT_GOD_CHAR => b"Example: You can't hit $N!\r\n",
        MSGEDIT_GOD_VICT => b"Example: $n can't hit you!\r\n",
        _ => b"Example: $n can't hit $N!\r\n",
    }
}

fn field_set(m: &mut MessageType, mode: i32, v: BStr) {
    let slot = match mode {
        MSGEDIT_DEATH_CHAR => &mut m.die.attacker,
        MSGEDIT_DEATH_VICT => &mut m.die.victim,
        MSGEDIT_DEATH_ROOM => &mut m.die.room,
        MSGEDIT_MISS_CHAR => &mut m.miss.attacker,
        MSGEDIT_MISS_VICT => &mut m.miss.victim,
        MSGEDIT_MISS_ROOM => &mut m.miss.room,
        MSGEDIT_HIT_CHAR => &mut m.hit.attacker,
        MSGEDIT_HIT_VICT => &mut m.hit.victim,
        MSGEDIT_HIT_ROOM => &mut m.hit.room,
        MSGEDIT_GOD_CHAR => &mut m.god.attacker,
        MSGEDIT_GOD_VICT => &mut m.god.victim,
        _ => &mut m.god.room,
    };
    *slot = Some(v);
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn msgedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    match olc.mode {
        MSGEDIT_MAIN_MENU => {
            if arg.is_empty() {
                write_to_desc(g, di, b"Enter Option : ");
                return Some(olc);
            }
            match arg[0] {
                b'1' => {
                    write_to_desc(g, di, b"Enter Action Type : ");
                    olc.mode = MSGEDIT_TYPE;
                    return Some(olc);
                }
                c if field_mode(c).is_some() => {
                    let mode = field_mode(c).unwrap();
                    write_to_desc(g, di, field_example(mode));
                    write_to_desc(g, di, b"Enter new string : ");
                    olc.mode = mode;
                    return Some(olc);
                }
                b'N' | b'n' => {
                    let list = olc.msg_list.as_mut().unwrap();
                    if olc.msg_index + 1 >= list.msg.len() {
                        list.msg.push(MessageType::default());
                        list.number_of_attacks += 1;
                    }
                    olc.msg_index += 1;
                    msgedit_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
                b'P' | b'p' => {
                    if olc.msg_index > 0 {
                        olc.msg_index -= 1;
                    }
                    msgedit_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
                b'X' | b'x' => {
                    // The last set does not go this way. combat_message
                    // indexes msg[] after clamping to len()-1, which is 0
                    // for an empty list -- a type left with no sets at all
                    // panics on the next swing of it. Z takes the type and
                    // its sets together instead.
                    if olc.msg_list.as_ref().unwrap().msg.len() < 2 {
                        write_to_desc(g, di, b"That is the only message set for this attack type. Use Z to delete the type itself.\r\n");
                        msgedit_main_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    let idx = olc.msg_index;
                    {
                        let list = olc.msg_list.as_mut().unwrap();
                        list.msg.remove(idx);
                        if list.number_of_attacks > 0 {
                            list.number_of_attacks -= 1;
                        }
                    }
                    // Land on the set before the one that went, so N and P
                    // keep meaning what they did. Removing the first leaves
                    // the new first.
                    if olc.msg_index > 0 {
                        olc.msg_index -= 1;
                    }
                    olc.value = 1;
                    msgedit_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
                b'Z' | b'z' => {
                    // Both numbers come from the LIVE slot, not the
                    // working copy. 1) and X) change the copy and only
                    // reach the live slot on save, while Z deletes the
                    // live slot outright -- so naming the copy would ask
                    // a builder to confirm one record and then destroy a
                    // different one. Counted rather than read off
                    // number_of_attacks, which is one short for a type
                    // first written in this editor.
                    let slot = olc.number as usize;
                    let live_a_type = g.fight_messages[slot].a_type;
                    let sets = g.fight_messages[slot].msg.len();
                    write_to_desc(
                        g,
                        di,
                        format!(
                            "Delete attack type {} and {} message set{} with it? y/n : ",
                            live_a_type,
                            sets,
                            if sets == 1 { "" } else { "s" }
                        )
                        .as_bytes(),
                    );
                    olc.mode = MSGEDIT_CONFIRM_DELETE;
                    return Some(olc);
                }
                b'S' | b's' => {
                    write_to_desc(g, di, b"Do you wish to save? Y/N : ");
                    olc.mode = MSGEDIT_CONFIRM_SAVE;
                    return Some(olc);
                }
                b'Q' | b'q' => {
                    if olc.value != 0 {
                        olc.mode = MSGEDIT_CONFIRM_SAVE;
                        // Held per-descriptor. A shared flag would be
                        // inherited by a second builder in the editor.
                        olc.msg_quit = true;
                        write_to_desc(g, di, b"Do you wish to save? Y/N : ");
                        return Some(olc);
                    }
                    write_to_desc(g, di, b"Exiting message editor.\r\n");
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    // Fall out of both switches and redraw the menu
                    // below.
                }
            }
        }

        MSGEDIT_CONFIRM_SAVE => {
            if matches!(arg.first(), Some(b'Y') | Some(b'y')) {
                // Copied again on the way out, which reverses the working
                // copy back into table order.
                let mut list = olc.msg_list.as_ref().unwrap().as_ref().clone();
                list.msg.reverse();
                g.fight_messages[olc.number as usize] = list;
                save_messages_to_disk(g);
                olc.value = 0;
                write_to_desc(g, di, b"Messages saved.\r\n");
            } else {
                write_to_desc(g, di, b"Save aborted.\r\n");
            }
            if olc.msg_quit {
                olc.msg_quit = false;
                write_to_desc(g, di, b"Exiting message editor.\r\n");
                crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                return None;
            }
            msgedit_main_menu(g, di, &mut olc);
            return Some(olc);
        }

        MSGEDIT_CONFIRM_DELETE => {
            if matches!(arg.first(), Some(b'Y') | Some(b'y')) {
                // Before the clear below, and it has to stay there:
                // a_type is read out of the live slot that the next
                // statement empties, so moving this after it would
                // quietly start recording every deletion as type 0.
                //
                // mudlog, not log: merely opening this editor already
                // reaches the god channel, and this is the one action
                // here that cannot be undone. Same class and level as
                // that one, so an invisible builder stays invisible.
                let chid = g.descriptors.get(di).and_then(|d| d.character);
                let name = chid
                    .map(|c| String::from_utf8_lossy(g.ch(c).get_name()).into_owned())
                    .unwrap_or_else(|| "someone".to_string());
                let level = chid
                    .map(|c| (LVL_IMMORT as i16).max(g.ch(c).invis_lev()) as u8)
                    .unwrap_or(LVL_IMMORT as u8);
                let a_type = g.fight_messages[olc.number as usize].a_type;
                g.mudlog(
                    MudlogKind::Cmp,
                    level,
                    true,
                    &format!(
                        "OLC: {} deletes attack type {} in message slot {}",
                        name, a_type, olc.number
                    ),
                );
                // Back to the state every unused slot is already in at
                // boot: no messages, no attack type, no count.
                g.fight_messages[olc.number as usize] =
                    crate::fight::FightMessageList::default();
                save_messages_to_disk(g);
                write_to_desc(g, di, b"Attack type deleted.\r\n");
                crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                return None;
            }
            write_to_desc(g, di, b"Delete aborted.\r\n");
            msgedit_main_menu(g, di, &mut olc);
            return Some(olc);
        }

        MSGEDIT_TYPE => {
            olc.msg_list.as_mut().unwrap().a_type = limit(atoi(arg), 0, 500);
        }

        mode if (MSGEDIT_DEATH_CHAR..=MSGEDIT_GOD_ROOM).contains(&mode) => {
            let mut text = arg.to_vec();
            genolc_checkstring(&mut text);
            delete_doubledollar(&mut text);
            let idx = olc.msg_index;
            let list = olc.msg_list.as_mut().unwrap();
            field_set(&mut list.msg[idx], mode, text);
        }

        _ => {}
    }

    olc.value = 1;
    msgedit_main_menu(g, di, &mut olc);
    Some(olc)
}
