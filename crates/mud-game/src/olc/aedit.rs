//! The social editor, plus `astat`.
//!
//! Socials are not world data: there is one table, one file
//! (`lib/misc/socials.new`), and one editor slot — "Sorry, only one can edit
//! socials at a time." `zone_num` is reused as the **social index**, which is
//! why B57 exists: `cleanup_olc` has no CON_AEDIT branch and logs "stops
//! editing zone %d" by reading it as a zone.
//!
//! Saving is not deferred. `aedit_save_internally` re-merges the command
//! list and then writes the whole file immediately ("autosave by Rumble"),
//! so the save-list entry it adds is removed again three lines later.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::POSITION_TYPES;
use mud_data::types::*;

use crate::act::wizstat::AEDIT_PERMISSION;
use crate::act::BStr;
use crate::comm::{act, send_to_char, write_to_desc, TO_ROOM};
use crate::db::{add_to_save_list, remove_from_save_list, SL_ACT};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, is_abbrev};
use crate::interpreter::{delete_doubledollar, one_argument};
use crate::olc::{can_edit_zone, get_char_colors, OlcData, CLEANUP_ALL, CLEANUP_STRUCTS};
use crate::social::Social;

/// AEDIT connectedness.
pub const AEDIT_CONFIRM_SAVESTRING: i32 = 0;
pub const AEDIT_CONFIRM_EDIT: i32 = 1;
pub const AEDIT_CONFIRM_ADD: i32 = 2;
pub const AEDIT_MAIN_MENU: i32 = 3;
pub const AEDIT_ACTION_NAME: i32 = 4;
pub const AEDIT_SORT_AS: i32 = 5;
pub const AEDIT_MIN_CHAR_POS: i32 = 6;
pub const AEDIT_MIN_VICT_POS: i32 = 7;
pub const AEDIT_MIN_CHAR_LEVEL: i32 = 9;
pub const AEDIT_NOVICT_CHAR: i32 = 10;
pub const AEDIT_NOVICT_OTHERS: i32 = 11;
pub const AEDIT_VICT_CHAR_FOUND: i32 = 12;
pub const AEDIT_VICT_OTHERS_FOUND: i32 = 13;
pub const AEDIT_VICT_VICT_FOUND: i32 = 14;
pub const AEDIT_VICT_NOT_FOUND: i32 = 15;
pub const AEDIT_SELF_CHAR: i32 = 16;
pub const AEDIT_SELF_OTHERS: i32 = 17;
pub const AEDIT_VICT_CHAR_BODY_FOUND: i32 = 18;
pub const AEDIT_VICT_OTHERS_BODY_FOUND: i32 = 19;
pub const AEDIT_VICT_VICT_BODY_FOUND: i32 = 20;
pub const AEDIT_OBJ_CHAR_FOUND: i32 = 21;
pub const AEDIT_OBJ_OTHERS_FOUND: i32 = 22;
pub const AEDIT_CONFIRM_DELETE: i32 = 23;

const POS_DEAD: i32 = 0;
const POS_STANDING: i32 = 8;

/// `%-<w>.<w>s`: truncate to `w`, then pad with spaces to `w`.
fn pad_trunc(out: &mut BStr, s: &[u8], w: usize) {
    let n = s.len().min(w);
    out.extend_from_slice(&s[..n]);
    out.extend(std::iter::repeat(b' ').take(w - n));
}

/// Does this answer start a number at all?
///
/// `atoi` returns 0 for anything that does not, and 0 is inside both ranges
/// these prompts check -- POS_DEAD for the two position prompts, and level 0
/// for the third -- so a typo silently sets the social to Dead or to level 0
/// rather than being refused.
fn starts_a_number(arg: &[u8]) -> bool {
    arg.first().is_some_and(|c| c.is_ascii_digit())
        || (arg.first() == Some(&b'-') && arg.get(1).is_some_and(|c| c.is_ascii_digit()))
}

/// position_types[] indexed by a social's stored position. B58 keeps the
/// index inside the table. Without a working guard on what a builder can
/// type, `position_types[99]` would render as whatever followed it.
fn position_name(p: i32) -> &'static str {
    POSITION_TYPES.get(p.max(0) as usize).copied().unwrap_or("Undefined")
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_aedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    if !g.config.use_new_socials {
        send_to_char(g, chid, b"Socials cannot be edited at the moment.\r\n");
        return;
    }
    if !can_edit_zone(g, chid, AEDIT_PERMISSION) {
        send_to_char(g, chid, b"You don't have access to editing socials.\r\n");
        return;
    }
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) == Some(ConState::Aedit) {
            send_to_char(g, chid, b"Sorry, only one can edit socials at a time.\r\n");
            return;
        }
    }

    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Please specify a social to edit.\r\n");
        return;
    }

    if crate::text::cmp_ci(b"save", &arg) == std::cmp::Ordering::Equal {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(MudlogKind::Cmp, level, true, &format!("OLC: {} saves socials.", name));
        send_to_char(g, chid, b"Writing social file.\r\n");
        if aedit_save_to_disk(g) {
            send_to_char(g, chid, b"Done.\r\n");
        } else {
            send_to_char(g, chid, &crate::olc::save_failed("the social file"));
        }
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();
    olc.number = 0;
    olc.storage = Some(arg.clone());

    // The index walks the social table looking for the first command this
    // argument abbreviates.
    let mut znum = 0usize;
    while znum < g.socials.len() && !is_abbrev(&arg, &g.socials[znum].command) {
        znum += 1;
    }
    olc.zone_num = znum as i32;

    if znum >= g.socials.len() {
        if let Some(i) = aedit_find_command(g, &arg) {
            let mut msg = b"The '".to_vec();
            msg.extend_from_slice(&arg);
            msg.extend_from_slice(b"' command already exists (");
            msg.extend_from_slice(&g.commands[i].command);
            msg.extend_from_slice(b").\r\n");
            send_to_char(g, chid, &msg);
            // The OLC structure is installed before this test and cleaned
            // up here, with the descriptor still in play, so the guard keeps
            // an OLC session that never started from being announced.
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
            return;
        }
        let mut msg = b"Do you wish to add the '".to_vec();
        msg.extend_from_slice(&arg);
        msg.extend_from_slice(b"' action? ");
        send_to_char(g, chid, &msg);
        olc.mode = AEDIT_CONFIRM_ADD;
    } else {
        let mut msg = b"Do you wish to edit the '".to_vec();
        msg.extend_from_slice(&g.socials[znum].command);
        msg.extend_from_slice(b"' action? ");
        send_to_char(g, chid, &msg);
        olc.mode = AEDIT_CONFIRM_EDIT;
    }

    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Aedit;
    }
    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    g.mudlog(MudlogKind::Cmp, level, true, &format!("OLC: {} starts editing actions.", name));
}

fn aedit_setup_new(olc: &mut OlcData) {
    let name = olc.storage.clone().unwrap_or_default();
    olc.action = Some(Box::new(Social {
        command: name.clone(),
        sort_as: name,
        hide: 0,
        min_victim_position: POS_STANDING,
        min_char_position: POS_STANDING,
        min_level_char: 0,
        char_no_arg: Some(b"This action is unfinished.".to_vec()),
        others_no_arg: Some(b"This action is unfinished.".to_vec()),
        ..Default::default()
    }));
}

/// aedit_setup_existing: a field-by-field copy, so a NULL
/// message stays NULL rather than becoming "undefined".
fn aedit_setup_existing(g: &Game, olc: &mut OlcData, real_num: usize) {
    let mut s = g.socials[real_num].clone();
    // act_nr is not part of the scratch copy; it is restored from the
    // table on save.
    s.act_nr = 0;
    olc.action = Some(Box::new(s));
}

/// aedit_find_command: the command table scanned by
/// sort_as prefix, or an exact command match. Index 0 is RESERVED.
fn aedit_find_command(g: &Game, txt: &[u8]) -> Option<usize> {
    for cmd in 1..g.commands.len() {
        let e = &g.commands[cmd];
        if e.sort_as.starts_with(txt) || e.command == txt {
            return Some(cmd);
        }
    }
    None
}

fn aedit_save_internally(g: &mut Game, olc: &mut OlcData) -> bool {
    let action = olc.action.as_ref().unwrap().as_ref().clone();
    let znum = olc.zone_num as usize;
    if znum >= g.socials.len() {
        g.socials.push(action);
    } else {
        // The table's act_nr carries over.
        let act_nr = g.socials[znum].act_nr;
        g.socials[znum] = action;
        g.socials[znum].act_nr = act_nr;
    }
    crate::interpreter::create_command_list(g);
    add_to_save_list(g, AEDIT_PERMISSION as Idx, SL_ACT);
    aedit_save_to_disk(g)
}

/// aedit_save_to_disk: the whole table, in table order,
/// which `create_command_list` has already sorted by sort_as.
pub fn aedit_save_to_disk(g: &mut Game) -> bool {
    let mut out: BStr = Vec::new();
    for s in &g.socials {
        out.push(b'~');
        out.extend_from_slice(&s.command);
        out.push(b' ');
        out.extend_from_slice(&s.sort_as);
        out.extend_from_slice(
            format!(
                " {} {} {} {}\n",
                s.hide, s.min_char_position, s.min_victim_position, s.min_level_char
            )
            .as_bytes(),
        );
        // Four convert_from_tabs'd groups: 4, 4, 3, then 2 + a blank line.
        let groups: [&[&Option<BStr>]; 4] = [
            &[&s.char_no_arg, &s.others_no_arg, &s.char_found, &s.others_found],
            &[&s.vict_found, &s.not_found, &s.char_auto, &s.others_auto],
            &[&s.char_body_found, &s.others_body_found, &s.vict_body_found],
            &[&s.char_obj_found, &s.others_obj_found],
        ];
        for (n, group) in groups.iter().enumerate() {
            let mut buf: BStr = Vec::new();
            for field in group.iter() {
                match field {
                    Some(t) => buf.extend_from_slice(t),
                    None => buf.push(b'#'),
                }
                buf.push(b'\n');
            }
            if n == 3 {
                buf.push(b'\n');
            }
            mud_net::editor::parse_tab(&mut buf);
            out.extend_from_slice(&buf);
        }
    }
    out.extend_from_slice(b"$\n");

    let path = g.lib_dir.join("misc").join("socials.new");
    if std::fs::write(&path, &out).is_err() {
        // Log and carry on: a failed write here is not worth taking the
        // MUD down for.
        g.log(format!("SYSERR: Can't open socials file '{}'", path.display()));
        return false;
    }
    remove_from_save_list(g, AEDIT_PERMISSION as Idx, SL_ACT);
    true
}

// ---------------------------------------------------------------------------
// The menu
// ---------------------------------------------------------------------------

fn aedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    let c = g.olc_colors;
    let (nrm, grn, cyn, yel) = (c.nrm(), c.grn(), c.cyn(), c.yel());
    let a = olc.action.as_ref().unwrap().as_ref().clone();

    let mut out: BStr = Vec::new();
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"-- Action editor\r\n");

    out.extend_from_slice(grn);
    out.extend_from_slice(b"n");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Command         : ");
    out.extend_from_slice(yel);
    pad_trunc(&mut out, &a.command, 15);
    out.extend_from_slice(nrm);
    out.push(b' ');
    out.extend_from_slice(grn);
    out.extend_from_slice(b"1");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Sort as Command  : ");
    out.extend_from_slice(yel);
    pad_trunc(&mut out, &a.sort_as, 15);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(grn);
    out.extend_from_slice(b"2");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Min Position[CH]: ");
    out.extend_from_slice(cyn);
    pad_trunc(&mut out, position_name(a.min_char_position).as_bytes(), 8);
    out.extend_from_slice(b"        ");
    out.extend_from_slice(grn);
    out.extend_from_slice(b"3");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Min Position [VT]: ");
    out.extend_from_slice(cyn);
    pad_trunc(&mut out, position_name(a.min_victim_position).as_bytes(), 8);
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(grn);
    out.extend_from_slice(b"4");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Min Level   [CH]: ");
    out.extend_from_slice(cyn);
    let lvl = a.min_level_char.to_string();
    out.extend_from_slice(lvl.as_bytes());
    out.extend(std::iter::repeat(b' ').take(3usize.saturating_sub(lvl.len())));
    out.extend_from_slice(b"             ");
    out.extend_from_slice(grn);
    out.extend_from_slice(b"5");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Show if Invisible: ");
    out.extend_from_slice(cyn);
    out.extend_from_slice(if a.hide != 0 { &b"HIDDEN"[..] } else { &b"NOT HIDDEN"[..] });
    out.extend_from_slice(b"\r\n");

    for (key, label, text) in menu_rows(&a) {
        out.extend_from_slice(grn);
        out.push(key);
        out.extend_from_slice(nrm);
        out.extend_from_slice(b") ");
        out.extend_from_slice(label);
        out.extend_from_slice(b": ");
        out.extend_from_slice(cyn);
        out.extend_from_slice(text.unwrap_or(b"<Null>"));
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(grn);
    out.extend_from_slice(b"X");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Delete this social\r\n");
    out.extend_from_slice(grn);
    out.extend_from_slice(b"q");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Quit\r\nEnter Choice:");
    write_to_desc(g, di, &out);
    olc.mode = AEDIT_MAIN_MENU;
}

/// The name to offer or to log, read from the TABLE rather than from the
/// working copy. Menu key `n` rewrites the copy and leaves the table alone, so
/// a rename followed by a delete would otherwise offer to remove a social that
/// does not exist while removing the one that does.
fn table_social_name(g: &Game, rnum: i32, fallback: &'static [u8]) -> BStr {
    if rnum >= 0 {
        if let Some(s) = g.socials.get(rnum as usize) {
            if !s.command.is_empty() {
                return s.command.clone();
            }
        }
    }
    fallback.to_vec()
}

/// Remove a social from the table.
///
/// The table's own `act_nr` needs no renumbering: it indexes the MERGED
/// command table, and `create_command_list` rebuilds that from scratch and
/// reassigns every one. Its outside users are a different matter -- the five
/// command numbers in `shop_cmds` are indices into the same merged table and
/// go stale when it moves, which is why `create_command_list` retakes them
/// (tbamud/tbamud#284). Deleting a social that sorts before "slap" is one of
/// the two ways to move them; adding one is the other.
fn aedit_delete_social(g: &mut Game, rnum: i32) -> bool {
    if rnum < 0 || rnum as usize >= g.socials.len() {
        return false;
    }
    let name = String::from_utf8_lossy(&g.socials[rnum as usize].command).into_owned();
    g.log(format!("GenOLC: aedit_delete_social: Deleting social '{}'.", name));
    g.socials.remove(rnum as usize);
    true
}

/// The thirteen message rows, in menu order — which is NOT the struct order
/// (not_found and the two auto messages come before the vict trio).
fn menu_rows(a: &Social) -> Vec<(u8, &'static [u8], Option<&[u8]>)> {
    vec![
        (b'a', b"Char    [NO ARG]", a.char_no_arg.as_deref()),
        (b'b', b"Others  [NO ARG]", a.others_no_arg.as_deref()),
        (b'c', b"Char [NOT FOUND]", a.not_found.as_deref()),
        (b'd', b"Char  [ARG SELF]", a.char_auto.as_deref()),
        (b'e', b"Others[ARG SELF]", a.others_auto.as_deref()),
        (b'f', b"Char      [VICT]", a.char_found.as_deref()),
        (b'g', b"Others    [VICT]", a.others_found.as_deref()),
        (b'h', b"Victim    [VICT]", a.vict_found.as_deref()),
        (b'i', b"Char  [BODY PRT]", a.char_body_found.as_deref()),
        (b'j', b"Others[BODY PRT]", a.others_body_found.as_deref()),
        (b'k', b"Victim[BODY PRT]", a.vict_body_found.as_deref()),
        (b'l', b"Char       [OBJ]", a.char_obj_found.as_deref()),
        (b'm', b"Others     [OBJ]", a.others_obj_found.as_deref()),
    ]
}

/// The [OLD]/[NEW] prompt each message field opens with.
fn field_prompt(mode: i32) -> &'static [u8] {
    match mode {
        AEDIT_NOVICT_CHAR => b"Enter social shown to the Character when there is no argument supplied.\r\n",
        AEDIT_NOVICT_OTHERS => b"Enter social shown to Others when there is no argument supplied.\r\n",
        AEDIT_VICT_NOT_FOUND => b"Enter text shown to the Character when his victim isnt found.\r\n",
        AEDIT_SELF_CHAR => b"Enter social shown to the Character when it is its own victim.\r\n",
        AEDIT_SELF_OTHERS => b"Enter social shown to Others when the Char is its own victim.\r\n",
        AEDIT_VICT_CHAR_FOUND => b"Enter normal social shown to the Character when the victim is found.\r\n",
        AEDIT_VICT_OTHERS_FOUND => b"Enter normal social shown to Others when the victim is found.\r\n",
        AEDIT_VICT_VICT_FOUND => b"Enter normal social shown to the Victim when the victim is found.\r\n",
        AEDIT_VICT_CHAR_BODY_FOUND => {
            b"Enter 'body part' social shown to the Character when the victim is found.\r\n"
        }
        AEDIT_VICT_OTHERS_BODY_FOUND => {
            b"Enter 'body part' social shown to Others when the victim is found.\r\n"
        }
        AEDIT_VICT_VICT_BODY_FOUND => {
            b"Enter 'body part' social shown to the Victim when the victim is found.\r\n"
        }
        AEDIT_OBJ_CHAR_FOUND => {
            b"Enter 'object' social shown to the Character when the object is found.\r\n"
        }
        _ => b"Enter 'object' social shown to the Room when the object is found.\r\n",
    }
}

fn field_get<'a>(a: &'a Social, mode: i32) -> &'a Option<BStr> {
    match mode {
        AEDIT_NOVICT_CHAR => &a.char_no_arg,
        AEDIT_NOVICT_OTHERS => &a.others_no_arg,
        AEDIT_VICT_NOT_FOUND => &a.not_found,
        AEDIT_SELF_CHAR => &a.char_auto,
        AEDIT_SELF_OTHERS => &a.others_auto,
        AEDIT_VICT_CHAR_FOUND => &a.char_found,
        AEDIT_VICT_OTHERS_FOUND => &a.others_found,
        AEDIT_VICT_VICT_FOUND => &a.vict_found,
        AEDIT_VICT_CHAR_BODY_FOUND => &a.char_body_found,
        AEDIT_VICT_OTHERS_BODY_FOUND => &a.others_body_found,
        AEDIT_VICT_VICT_BODY_FOUND => &a.vict_body_found,
        AEDIT_OBJ_CHAR_FOUND => &a.char_obj_found,
        _ => &a.others_obj_found,
    }
}

fn field_set(a: &mut Social, mode: i32, v: Option<BStr>) {
    let slot = match mode {
        AEDIT_NOVICT_CHAR => &mut a.char_no_arg,
        AEDIT_NOVICT_OTHERS => &mut a.others_no_arg,
        AEDIT_VICT_NOT_FOUND => &mut a.not_found,
        AEDIT_SELF_CHAR => &mut a.char_auto,
        AEDIT_SELF_OTHERS => &mut a.others_auto,
        AEDIT_VICT_CHAR_FOUND => &mut a.char_found,
        AEDIT_VICT_OTHERS_FOUND => &mut a.others_found,
        AEDIT_VICT_VICT_FOUND => &mut a.vict_found,
        AEDIT_VICT_CHAR_BODY_FOUND => &mut a.char_body_found,
        AEDIT_VICT_OTHERS_BODY_FOUND => &mut a.others_body_found,
        AEDIT_VICT_VICT_BODY_FOUND => &mut a.vict_body_found,
        AEDIT_OBJ_CHAR_FOUND => &mut a.char_obj_found,
        _ => &mut a.others_obj_found,
    };
    *slot = v;
}

/// Which mode a main-menu letter opens, or None if it is not a field key.
fn field_mode(c: u8) -> Option<i32> {
    Some(match c.to_ascii_lowercase() {
        b'a' => AEDIT_NOVICT_CHAR,
        b'b' => AEDIT_NOVICT_OTHERS,
        b'c' => AEDIT_VICT_NOT_FOUND,
        b'd' => AEDIT_SELF_CHAR,
        b'e' => AEDIT_SELF_OTHERS,
        b'f' => AEDIT_VICT_CHAR_FOUND,
        b'g' => AEDIT_VICT_OTHERS_FOUND,
        b'h' => AEDIT_VICT_VICT_FOUND,
        b'i' => AEDIT_VICT_CHAR_BODY_FOUND,
        b'j' => AEDIT_VICT_OTHERS_BODY_FOUND,
        b'k' => AEDIT_VICT_VICT_BODY_FOUND,
        b'l' => AEDIT_OBJ_CHAR_FOUND,
        b'm' => AEDIT_OBJ_OTHERS_FOUND,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn aedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    match olc.mode {
        AEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let saved = aedit_save_internally(g, &mut olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_GOD as i16).max(g.ch(chid).invis_lev()) as u8;
                        let cmd = String::from_utf8_lossy(
                            &olc.action.as_ref().unwrap().command,
                        )
                        .into_owned();
                        g.mudlog(
                            MudlogKind::Cmp,
                            level,
                            true,
                            &format!("OLC: {} edits action {}", name, cmd),
                        );
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_STRUCTS);
                    if saved {
                        write_to_desc(g, di, b"Action saved to disk.\r\n");
                    } else {
                        write_to_desc(g, di, &crate::olc::save_failed("the social file"));
                    }
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to save your changes? : ",
                    );
                }
            }
            return Some(olc);
        }

        AEDIT_CONFIRM_EDIT => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let znum = olc.zone_num as usize;
                    aedit_setup_existing(g, &mut olc, znum);
                    olc.value = 0;
                    aedit_disp_menu(g, di, &mut olc);
                }
                Some(b'q') | Some(b'Q') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    let storage = olc.storage.clone().unwrap_or_default();
                    let mut znum = olc.zone_num as usize + 1;
                    while znum < g.socials.len() && !is_abbrev(&storage, &g.socials[znum].command) {
                        znum += 1;
                    }
                    olc.zone_num = znum as i32;
                    if znum >= g.socials.len() {
                        if aedit_find_command(g, &storage).is_some() {
                            // No message is sent here.
                            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                            return None;
                        }
                        let mut msg = b"Do you wish to add the '".to_vec();
                        msg.extend_from_slice(&storage);
                        msg.extend_from_slice(b"' action? ");
                        write_to_desc(g, di, &msg);
                        olc.mode = AEDIT_CONFIRM_ADD;
                    } else {
                        let mut msg = b"Do you wish to edit the '".to_vec();
                        msg.extend_from_slice(&g.socials[znum].command);
                        msg.extend_from_slice(b"' action? ");
                        write_to_desc(g, di, &msg);
                        olc.mode = AEDIT_CONFIRM_EDIT;
                    }
                }
                _ => {
                    let mut msg = b"Invalid choice!\r\nDo you wish to edit the '".to_vec();
                    msg.extend_from_slice(&g.socials[olc.zone_num as usize].command);
                    msg.extend_from_slice(b"' action? ");
                    write_to_desc(g, di, &msg);
                }
            }
            return Some(olc);
        }

        AEDIT_CONFIRM_ADD => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    aedit_setup_new(&mut olc);
                    aedit_disp_menu(g, di, &mut olc);
                    olc.value = 0;
                }
                Some(b'n') | Some(b'N') | Some(b'q') | Some(b'Q') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    let mut msg = b"Invalid choice!\r\nDo you wish to add the '".to_vec();
                    msg.extend_from_slice(&olc.storage.clone().unwrap_or_default());
                    msg.extend_from_slice(b"' action? ");
                    write_to_desc(g, di, &msg);
                }
            }
            return Some(olc);
        }

        AEDIT_CONFIRM_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    // Captured before the delete drops the row it names.
                    let sname = table_social_name(g, olc.zone_num, b"?");
                    if aedit_delete_social(g, olc.zone_num) {
                        // create_command_list rebuilds the merged table and
                        // reassigns every act_nr -- and retakes the command
                        // numbers cached off it. Both are what a save does
                        // after any other change.
                        crate::interpreter::create_command_list(g);
                        add_to_save_list(g, AEDIT_PERMISSION as Idx, SL_ACT);
                        let saved = aedit_save_to_disk(g);
                        if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                            let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                            let cmd = String::from_utf8_lossy(&sname).into_owned();
                            g.mudlog(
                                MudlogKind::Cmp,
                                level,
                                true,
                                &format!("OLC: {} deletes social {}", name, cmd),
                            );
                        }
                        if saved {
                            write_to_desc(g, di, b"Social deleted.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the social file"));
                        }
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    // Nothing was deleted, so nothing is thrown away either --
                    // cleaning up here would discard unsaved work.
                    write_to_desc(g, di, b"Could not delete that social.\r\n");
                    aedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'n') | Some(b'N') => {
                    aedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    let sname = table_social_name(g, olc.zone_num, b"this social");
                    let mut msg = b"Invalid choice!\r\nDelete '".to_vec();
                    msg.extend_from_slice(&sname);
                    msg.extend_from_slice(b"'? : ");
                    write_to_desc(g, di, &msg);
                    return Some(olc);
                }
            }
        }

        AEDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'x') | Some(b'X') => {
                    if olc.zone_num < 0 || olc.zone_num as usize >= g.socials.len() {
                        write_to_desc(
                            g,
                            di,
                            b"That social has not been saved yet -- quit without saving instead.\r\n",
                        );
                        aedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    let sname = table_social_name(g, olc.zone_num, b"this social");
                    let mut msg = b"Delete '".to_vec();
                    msg.extend_from_slice(&sname);
                    msg.extend_from_slice(b"'? : ");
                    write_to_desc(g, di, &msg);
                    olc.mode = AEDIT_CONFIRM_DELETE;
                    return Some(olc);
                }
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = AEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                }
                // Lower-case only: 'N' is not a menu key.
                Some(b'n') => {
                    write_to_desc(g, di, b"Enter action name: ");
                    olc.mode = AEDIT_ACTION_NAME;
                }
                Some(b'1') => {
                    write_to_desc(
                        g,
                        di,
                        b"Enter sort info for this action (for the command listing): ",
                    );
                    olc.mode = AEDIT_SORT_AS;
                }
                Some(c @ (b'2' | b'3')) => {
                    let who = if c == b'2' { &b"Character"[..] } else { &b"Victim"[..] };
                    let mut msg = b"Enter the minimum position the ".to_vec();
                    msg.extend_from_slice(who);
                    msg.extend_from_slice(b" has to be in to activate social:\r\n");
                    for i in POS_DEAD..=POS_STANDING {
                        msg.extend_from_slice(
                            format!("   {}) {}\r\n", i, position_name(i)).as_bytes(),
                        );
                    }
                    msg.extend_from_slice(b"Enter choice: ");
                    write_to_desc(g, di, &msg);
                    olc.mode = if c == b'2' { AEDIT_MIN_CHAR_POS } else { AEDIT_MIN_VICT_POS };
                }
                Some(b'4') => {
                    write_to_desc(g, di, b"Enter new minimum level for social: ");
                    olc.mode = AEDIT_MIN_CHAR_LEVEL;
                }
                Some(b'5') => {
                    let a = olc.action.as_mut().unwrap();
                    a.hide = if a.hide != 0 { 0 } else { 1 };
                    aedit_disp_menu(g, di, &mut olc);
                    olc.value = 1;
                }
                Some(c) if field_mode(c).is_some() => {
                    let mode = field_mode(c).unwrap();
                    let mut msg = field_prompt(mode).to_vec();
                    msg.extend_from_slice(b"[OLD]: ");
                    let a = olc.action.as_ref().unwrap();
                    msg.extend_from_slice(field_get(a, mode).as_deref().unwrap_or(b"NULL"));
                    msg.extend_from_slice(b"\r\n[NEW]: ");
                    write_to_desc(g, di, &msg);
                    olc.mode = mode;
                }
                _ => {
                    aedit_disp_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        AEDIT_ACTION_NAME | AEDIT_SORT_AS => {
            if arg.is_empty() || arg.contains(&b' ') {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            let a = olc.action.as_mut().unwrap();
            if olc.mode == AEDIT_ACTION_NAME {
                a.command = arg.to_vec();
            } else {
                a.sort_as = arg.to_vec();
            }
        }

        AEDIT_MIN_CHAR_POS | AEDIT_MIN_VICT_POS => {
            if arg.is_empty() {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            // A guard of `(i < POS_DEAD) && (i > POS_STANDING)` is
            // never true, so any integer would land in the field and the
            // menu would then read position_types[] at it. The range is only
            // half the guard, though -- see `starts_a_number`.
            if !starts_a_number(arg) {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            let i = atoi(arg);
            if !(POS_DEAD..=POS_STANDING).contains(&i) {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            let a = olc.action.as_mut().unwrap();
            if olc.mode == AEDIT_MIN_CHAR_POS {
                a.min_char_position = i;
            } else {
                a.min_victim_position = i;
            }
        }

        AEDIT_MIN_CHAR_LEVEL => {
            if arg.is_empty() {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            // B58, the same broken guard, and the same half-guard: level 0
            // is what `atoi` answers for a typo.
            if !starts_a_number(arg) {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            let i = atoi(arg);
            if !(0..=LVL_IMPL as i32).contains(&i) {
                aedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            olc.action.as_mut().unwrap().min_level_char = i;
        }

        m if (AEDIT_NOVICT_CHAR..=AEDIT_OBJ_OTHERS_FOUND).contains(&m) => {
            let mut text = arg.to_vec();
            let v = if text.is_empty() {
                None
            } else {
                delete_doubledollar(&mut text);
                Some(text)
            };
            field_set(olc.action.as_mut().unwrap(), m, v);
        }

        _ => {}
    }

    olc.value = 1;
    aedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_astat(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Astat which social?\r\n");
        return;
    }
    let Some(i) = g.socials.iter().position(|s| is_abbrev(&arg, &s.command)) else {
        send_to_char(g, chid, b"No such social.\r\n");
        return;
    };

    if let Some(c) = g.ch(chid).desc.map(|_| chid) {
        get_char_colors(g, c);
    }
    let c = g.olc_colors;
    let (nrm, cyn, yel) = (c.nrm(), c.cyn(), c.yel());
    let a = g.socials[i].clone();

    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"n) Command         : ");
    out.extend_from_slice(yel);
    pad_trunc(&mut out, &a.command, 15);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b" 1) Sort as Command : ");
    out.extend_from_slice(yel);
    pad_trunc(&mut out, &a.sort_as, 15);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n2) Min Position[CH]: ");
    out.extend_from_slice(cyn);
    pad_trunc(&mut out, position_name(a.min_char_position).as_bytes(), 8);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"        3) Min Position[VT]: ");
    out.extend_from_slice(cyn);
    pad_trunc(&mut out, position_name(a.min_victim_position).as_bytes(), 8);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n4) Min Level   [CH]: ");
    out.extend_from_slice(cyn);
    let lvl = a.min_level_char.to_string();
    out.extend_from_slice(lvl.as_bytes());
    out.extend(std::iter::repeat(b' ').take(3usize.saturating_sub(lvl.len())));
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"             5) Show if Invis   : ");
    out.extend_from_slice(cyn);
    out.extend_from_slice(if a.hide != 0 { &b"HIDDEN"[..] } else { &b"NOT HIDDEN"[..] });
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n");

    for (key, label, text) in menu_rows(&a) {
        out.push(key);
        out.extend_from_slice(b") ");
        out.extend_from_slice(label);
        out.extend_from_slice(b": ");
        out.extend_from_slice(cyn);
        // astat prints "" where the editor prints "<Null>".
        out.extend_from_slice(text.unwrap_or(b""));
        out.extend_from_slice(nrm);
        out.extend_from_slice(b"\r\n");
    }
    send_to_char(g, chid, &out);
}
