//! The room editor.
//!
//! Two details of the menu turned out to be bugs rather than style:
//!
//! * **B34** — `redit_disp_extradesc_menu` printed the shared colour
//! globals without calling `get_char_colors` first, so it rendered in
//! whichever builder painted last. (`redit_disp_sector_menu` skips the
//! call too but never touches the globals, so it was never affected.)
//! * **B33** — opening the exit menu for a direction *materialised* an
//! empty exit whether or not anything was changed, and the writer emitted
//! it as `0 0 -1`.

use std::cmp::Ordering;

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{ROOM_BITS, SECTOR_TYPES};
use mud_data::types::*;
use mud_world::model::{ExtraDesc, Exit, Room};

use crate::act::informative::{column_list, sprintbitarray};
use crate::act::wizstat::sprinttype;
use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::{is_number, two_arguments};
use crate::olc::genwld::{add_room, delete_room, save_rooms};
use crate::olc::{
    can_edit_zone, clear_screen, count_non_protocol_chars, genolc_checkstring, get_char_colors,
    send_cannot_edit, str_udup, OlcData, StrTarget, CLEANUP_ALL, MAX_EXIT_DESC, MAX_ROOM_DESC,
    MAX_ROOM_NAME,
};

/// Submodes of REDIT connectedness.
pub const REDIT_MAIN_MENU: i32 = 1;
pub const REDIT_NAME: i32 = 2;
pub const REDIT_DESC: i32 = 3;
pub const REDIT_FLAGS: i32 = 4;
pub const REDIT_SECTOR: i32 = 5;
pub const REDIT_EXIT_MENU: i32 = 6;
pub const REDIT_CONFIRM_SAVEDB: i32 = 7;
pub const REDIT_CONFIRM_SAVESTRING: i32 = 8;
pub const REDIT_EXIT_NUMBER: i32 = 9;
pub const REDIT_EXIT_DESCRIPTION: i32 = 10;
pub const REDIT_EXIT_KEYWORD: i32 = 11;
pub const REDIT_EXIT_KEY: i32 = 12;
pub const REDIT_EXIT_DOORFLAGS: i32 = 13;
pub const REDIT_EXTRADESC_MENU: i32 = 14;
pub const REDIT_EXTRADESC_KEY: i32 = 15;
pub const REDIT_EXTRADESC_DESCRIPTION: i32 = 16;
pub const REDIT_DELETE: i32 = 17;
pub const REDIT_COPY: i32 = 18;

pub fn do_oasis_redit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    // No building as a mob or while being forced.
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    let mut number: i32 = NOWHERE as i32;
    let mut save = false;

    if buf1.is_empty() {
        number = g.world.rooms[g.ch(chid).in_room as usize].vnum as i32;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) != Ordering::Equal {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
        save = true;
        if is_number(&buf2) {
            number = atoi(&buf2);
        } else {
            let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
            if olc_zone != NOWHERE as i32 {
                number = match g.world.real_zone(olc_zone as Idx) {
                    None => NOWHERE as i32,
                    // The zone below is resolved with real_zone, so
                    // this has to be the zone NUMBER. Handing it
                    // a vnum here would stop any
                    // argument-less save from ever resolving.
                    Some(zlok) => g.world.zones[zlok as usize].number as i32,
                };
            }
        }
        if number == NOWHERE as i32 {
            send_to_char(g, chid, b"Save which zone?\r\n");
            return;
        }
    }

    // If a numeric argument was given (like a room number), get it.
    if number == NOWHERE as i32 {
        number = atoi(&buf1);
    }
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That room VNUM can't exist.\r\n");
        return;
    }

    // Check to make sure the room isn't already being edited.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Redit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        let mut msg = b"That room is currently being edited by ".to_vec();
        msg.extend_from_slice(&who);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis_redit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();

    // Find the zone.
    let znum = if save {
        g.world.real_zone(number as Idx).map(|z| z as i32)
    } else {
        crate::dg::mobcmd::real_zone_by_thing(g, number).map(|z| z as i32)
    };
    let Some(znum) = znum else {
        send_to_char(g, chid, b"Sorry, there is no zone for that number!\r\n");
        return;
    };
    olc.zone_num = znum;

    // Make sure the builder is allowed to modify this zone.
    if !can_edit_zone(g, chid, znum) {
        let zvnum = g.world.zones[znum as usize].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }

    if save {
        let zvnum = g.world.zones[znum as usize].number;
        send_to_char(
            g,
            chid,
            format!("Saving all rooms in zone {}.\r\n", zvnum).as_bytes(),
        );
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves room info for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_rooms(g, Some(znum as usize));
        return;
    }

    olc.number = number;

    match g.real_room(number) {
        Some(real_num) => redit_setup_existing(g, &mut olc, real_num as usize),
        None => redit_setup_new(&mut olc),
    }

    redit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Redit;
    }
    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum as usize].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!("OLC: {} starts editing zone {} allowed zone {}", name, zvnum, allowed);
    g.mudlog(MudlogKind::Cmp, level, true, &msg);
}

fn redit_setup_new(olc: &mut OlcData) {
    let mut room = Room::default();
    room.name = Some(b"An unfinished room".to_vec());
    room.description = Some(b"You are in an unfinished room.\r\n".to_vec());
    room.vnum = NOWHERE;
    olc.item_type = crate::dg::WLD_TRIGGER;
    olc.script = None;
    olc.room = Some(Box::new(room));
    olc.room_light = 0;
    olc.value = 0;
}

pub fn redit_setup_existing(g: &mut Game, olc: &mut OlcData, real_num: usize) {
    let mut room = g.world.rooms[real_num].clone();
    room.name = Some(str_udup(room.name.as_deref().unwrap_or(b"")));
    room.description = Some(str_udup(room.description.as_deref().unwrap_or(b"")));
    // copy_ex_descriptions runs the same "undefined" substitution.
    for xd in room.ex_descriptions.iter_mut() {
        xd.keyword = Some(str_udup(xd.keyword.as_deref().unwrap_or(b"")));
        xd.description = Some(str_udup(xd.description.as_deref().unwrap_or(b"")));
    }
    // The whole-struct copy carries the live light count with it.
    olc.room_light = g.rooms[real_num].light;
    olc.value = 0;
    olc.item_type = crate::dg::WLD_TRIGGER;
    olc.room = Some(Box::new(room));
    // The proto list moves to OLC_SCRIPT and the room copy keeps neither a
    // proto list nor a live script. It has to go through the shared helper,
    // as medit and oedit do: `dg_olc_script_copy` maps an EMPTY list to None
    // and the main menu renders None as "Not Set.". Open-coding it as
    // `Some(clone)` made every scriptless room report "Set." instead.
    // Nothing caught that until stage9-save became the first script to open
    // redit's main menu at all.
    crate::olc::trigedit::dg_olc_script_copy(olc);
    if let Some(r) = olc.room.as_mut() {
        r.proto_script.clear();
    }
}

/// redit records "the builder never filled this in" three different ways.
/// A field never visited is `None`. One visited and left blank comes back
/// as the literal `"undefined"`, because that is what it
/// substitutes for empty input. The string editor can leave an empty slice.
/// All three mean the same thing to the exit pruning in
/// `redit_save_internally`, and only the first of them is `None`.
fn exit_field_unset(text: Option<&[u8]>) -> bool {
    match text {
        None => true,
        Some(t) => t.is_empty() || t == b"undefined",
    }
}

pub fn redit_save_internally(g: &mut Game, di: usize, olc: &mut OlcData) {
    let mut new_room = false;
    {
        let room = olc.room.as_mut().expect("redit without a room");
        if room.vnum == NOWHERE {
            new_room = true;
        }
        room.vnum = olc.number as Idx;
        room.zone = olc.zone_num as ZoneRnum;

        // `redit_disp_exit_menu` materialises an exit as soon as it
        // displays, so merely *looking* at a direction leaves one behind —
        // and the writer emits every exit, baking `0 0 -1` into the.wld.
        // Kept narrow: an exit that leads nowhere but carries a description,
        // keyword or door flags is the idiom for a direction you can look
        // at but not walk (the shipped world has 95 of those, and 13 of the
        // phantoms this drops).
        //
        // R2: the fields are tested with `exit_field_unset`, not against
        // None. Opening the keyword prompt and pressing return does not
        // leave a null: REDIT_EXIT_KEYWORD defaults the answer, which
        // turns empty input into the literal "undefined" — so testing for
        // None alone kept the exit in the commonest way of making a blank
        // one, so the phantom exit survived the save.
        for slot in room.dir_option.iter_mut() {
            let empty = slot.as_deref().is_some_and(|e| {
                e.to_room == NOWHERE
                    && e.exit_info == 0
                    && exit_field_unset(e.keyword.as_deref())
                    && exit_field_unset(e.general_description.as_deref())
            });
            if empty {
                *slot = None;
            }
        }
    }
    let room = olc.room.as_ref().unwrap().as_ref().clone();
    let Some(room_num) = add_room(g, &room, olc.room_light) else {
        write_to_desc(g, di, b"Something went wrong...\r\n");
        g.log(format!("SYSERR: redit_save_internally: Something failed! ({})", NOWHERE));
        return;
    };

    // Update triggers and free the old proto list.
    let script = olc.script.clone().unwrap_or_default();
    g.world.rooms[room_num as usize].proto_script = script;
    crate::dg::assign_triggers(g, crate::dg::GoId::Room(room_num));

    // Don't adjust numbers on a room update.
    if !new_room {
        return;
    }

    // Every other builder's in-progress copy shifts with the table
    // Note zedit's 'D' case bumps arg2 as well — a
    // door direction, not a room rnum — and then falls through to arg1.
    let others: Vec<usize> = g.descriptors.order.clone();
    for dsc in others {
        if dsc == di {
            continue;
        }
        let state = g.descriptors.get(dsc).map(|d| d.state);
        let Some(olc_other) = g.olc.get_mut(&dsc) else { continue };
        match state {
            Some(ConState::Zedit) => {
                let Some(zone) = olc_other.zone.as_mut() else { continue };
                for cmd in zone.cmds.iter_mut() {
                    match cmd.command {
                        b'O' | b'M' | b'T' | b'V' => {
                            if cmd.arg3 >= room_num as i32 {
                                cmd.arg3 += 1;
                            }
                        }
                        b'D' => {
                            if cmd.arg2 >= room_num as i32 {
                                cmd.arg2 += 1;
                            }
                            if cmd.arg1 >= room_num as i32 {
                                cmd.arg1 += 1;
                            }
                        }
                        b'R' => {
                            if cmd.arg1 >= room_num as i32 {
                                cmd.arg1 += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(ConState::Redit) => {
                let Some(room) = olc_other.room.as_mut() else { continue };
                for ex in room.dir_option.iter_mut() {
                    if let Some(ex) = ex {
                        if ex.to_room >= room_num {
                            ex.to_room += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn redit_save_to_disk(g: &mut Game, zone_num: Option<usize>) -> bool {
    save_rooms(g, zone_num)
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/// redit_disp_extradesc_menu. No get_char_colors call.
fn redit_disp_extradesc_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    // This menu prints the colour globals directly, and they are shared
    // by every builder. Without setting them first it rendered in whoever
    // painted last — a colour-off builder blanked it for everyone, and a
    // colour-on builder handed raw escapes to someone who turned colour off.
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let idx = olc.desc.unwrap_or(0);
    let (keyword, description, has_next) = {
        let room = olc.room.as_ref().unwrap();
        let xd = room.ex_descriptions.get(idx);
        (
            xd.and_then(|x| x.keyword.clone()),
            xd.and_then(|x| x.description.clone()),
            idx + 1 < room.ex_descriptions.len(),
        )
    };
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"1");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Keywords: ");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(keyword.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"2");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Description:\r\n");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(description.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"3");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Goto next description: ");
    write_to_desc(g, di, &out);
    write_to_desc(g, di, if has_next { b"Set.\r\n" } else { b"Not Set.\r\n" });
    write_to_desc(g, di, b"Enter choice (0 to quit) : ");
    olc.mode = REDIT_EXTRADESC_MENU;
}

fn redit_disp_exit_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let dir = olc.value as usize;
    {
        let room = olc.room.as_mut().unwrap();
        if room.dir_option[dir].is_none() {
            room.dir_option[dir] = Some(Box::new(Exit {
                general_description: None,
                keyword: None,
                exit_info: 0,
                key: 0,
                to_room_vnum: 0,
                to_room: NOWHERE,
            }));
        }
    }
    let ex = olc.room.as_ref().unwrap().dir_option[dir].as_ref().unwrap().clone();

    // Weird door handling!
    let door_buf: &[u8] = if ex.exit_info & flags::EX_ISDOOR != 0 {
        if ex.exit_info & flags::EX_PICKPROOF != 0 && ex.exit_info & flags::EX_HIDDEN != 0 {
            b"Hidden Pickproof"
        } else if ex.exit_info & flags::EX_PICKPROOF != 0 {
            b"Pickproof"
        } else if ex.exit_info & flags::EX_HIDDEN != 0 {
            b"Is a Hidden Door"
        } else {
            b"Is a door"
        }
    } else {
        b"No door"
    };

    let chid = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(chid) = chid {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let to_room = if ex.to_room != NOWHERE {
        g.world.rooms[ex.to_room as usize].vnum as i32
    } else {
        -1
    };
    let key = if ex.key != NOTHING { ex.key as i32 } else { -1 };
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let push = |out: &mut BStr, n: &[u8], label: &[u8]| {
        out.extend_from_slice(c.grn());
        out.extend_from_slice(n);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
    };
    push(&mut out, b"1", b") Exit to     : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(format!("{}\r\n", to_room).as_bytes());
    push(&mut out, b"2", b") Description :-\r\n");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(ex.general_description.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    push(&mut out, b"3", b") Door name   : ");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(ex.keyword.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    push(&mut out, b"4", b") Key         : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(format!("{}\r\n", key).as_bytes());
    push(&mut out, b"5", b") Door flags  : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(door_buf);
    out.extend_from_slice(b"\r\n");
    push(&mut out, b"6", b") Purge exit.\r\n");
    out.extend_from_slice(b"Enter choice, 0 to quit : ");
    write_to_desc(g, di, &out);

    olc.mode = REDIT_EXIT_MENU;
}

fn redit_disp_exit_flag_menu(g: &mut Game, di: usize) {
    let chid = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(chid) = chid {
        get_char_colors(g, chid);
    }
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    for (n, label) in [
        (&b"0"[..], &b") No door\r\n"[..]),
        (b"1", b") Closeable door\r\n"),
        (b"2", b") Pickproof Door\r\n"),
        (b"3", b") Hidden Door\r\n"),
        (b"4", b") Hidden, Pickproof Door\r\n"),
    ] {
        out.extend_from_slice(c.grn());
        out.extend_from_slice(n);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
    }
    out.extend_from_slice(b"Enter choice : ");
    write_to_desc(g, di, &out);
}

fn redit_disp_flag_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let chid = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(chid) = chid {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    if let Some(chid) = chid {
        let names: Vec<BStr> = ROOM_BITS
            .iter()
            .take(flags::NUM_ROOM_FLAGS)
            .map(|s| s.as_bytes().to_vec())
            .collect();
        column_list(g, chid, 0, &names, true);
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(
        &olc.room.as_ref().unwrap().room_flags,
        &ROOM_BITS[..flags::NUM_ROOM_FLAGS],
        &mut bits,
    );
    let c = g.olc_colors;
    let mut out: BStr = b"\r\nRoom flags: ".to_vec();
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter room flags, 0 to quit : ");
    write_to_desc(g, di, &out);
    olc.mode = REDIT_FLAGS;
}

/// redit_disp_sector_menu. No get_char_colors call.
fn redit_disp_sector_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let names: Vec<BStr> = SECTOR_TYPES
            .iter()
            .take(flags::NUM_ROOM_SECTORS)
            .map(|s| s.as_bytes().to_vec())
            .collect();
        column_list(g, chid, 0, &names, true);
    }
    write_to_desc(g, di, b"\r\nEnter sector type : ");
    olc.mode = REDIT_SECTOR;
}

/// The exit vnum shown on the main menu.
fn exit_vnum(g: &Game, room: &Room, dir: usize) -> i32 {
    match room.dir_option[dir].as_deref() {
        Some(ex) if ex.to_room != NOWHERE => g.world.rooms[ex.to_room as usize].vnum as i32,
        _ => -1,
    }
}

fn redit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let chid = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(chid) = chid {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let room = olc.room.as_ref().unwrap().as_ref().clone();

    let mut buf1: BStr = Vec::new();
    sprintbitarray(&room.room_flags, &ROOM_BITS[..flags::NUM_ROOM_FLAGS], &mut buf1);
    let buf2 = sprinttype(room.sector_type, &SECTOR_TYPES[..flags::NUM_ROOM_SECTORS]);

    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"-- Room number : [");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(format!("{}", olc.number).as_bytes());
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"] Room zone: [");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(
        format!("{}", g.world.zones[olc.zone_num as usize].number).as_bytes(),
    );
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"]\r\n");

    let item = |out: &mut BStr, n: &[u8], label: &[u8], color: &[u8], value: &[u8]| {
        out.extend_from_slice(c.grn());
        out.extend_from_slice(n);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
        out.extend_from_slice(color);
        out.extend_from_slice(value);
    };
    item(&mut out, b"1", b") Name        : ", c.yel(), room.name.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    item(
        &mut out,
        b"2",
        b") Description :\r\n",
        c.yel(),
        room.description.as_deref().unwrap_or(b""),
    );
    item(&mut out, b"3", b") Room flags  : ", c.cyn(), &buf1);
    out.extend_from_slice(b"\r\n");
    item(&mut out, b"4", b") Sector type : ", c.cyn(), &buf2);
    out.extend_from_slice(b"\r\n");
    write_to_desc(g, di, &out);

    let mut out: BStr = Vec::new();
    if !g.config.diagonal_dirs {
        for (n, label, dir) in [
            (&b"5"[..], &b") Exit north  : "[..], NORTH),
            (b"6", b") Exit east   : ", EAST),
            (b"7", b") Exit south  : ", SOUTH),
            (b"8", b") Exit west   : ", WEST),
        ] {
            item(
                &mut out,
                n,
                label,
                c.cyn(),
                format!("{}", exit_vnum(g, &room, dir)).as_bytes(),
            );
            out.extend_from_slice(b"\r\n");
        }
    } else {
        for (n, label, dir, n2, label2, dir2) in [
            (&b"5"[..], &b") Exit north  : "[..], NORTH, &b"B"[..], &b") Exit northwest : "[..], NORTHWEST),
            (b"6", b") Exit east   : ", EAST, b"C", b") Exit northeast : ", NORTHEAST),
            (b"7", b") Exit south  : ", SOUTH, b"D", b") Exit southeast : ", SOUTHEAST),
            (b"8", b") Exit west   : ", WEST, b"E", b") Exit southwest : ", SOUTHWEST),
        ] {
            item(
                &mut out,
                n,
                label,
                c.cyn(),
                format!("{:<6}", exit_vnum(g, &room, dir)).as_bytes(),
            );
            out.extend_from_slice(c.nrm());
            out.extend_from_slice(b",  ");
            item(
                &mut out,
                n2,
                label2,
                c.cyn(),
                format!("{}", exit_vnum(g, &room, dir2)).as_bytes(),
            );
            out.extend_from_slice(b"\r\n");
        }
    }
    write_to_desc(g, di, &out);

    let mut out: BStr = Vec::new();
    item(&mut out, b"9", b") Exit up     : ", c.cyn(), format!("{}", exit_vnum(g, &room, UP)).as_bytes());
    out.extend_from_slice(b"\r\n");
    item(&mut out, b"A", b") Exit down   : ", c.cyn(), format!("{}", exit_vnum(g, &room, DOWN)).as_bytes());
    out.extend_from_slice(b"\r\n");
    item(&mut out, b"F", b") Extra descriptions menu\r\n", b"", b"");
    item(
        &mut out,
        b"S",
        b") Script      : ",
        c.cyn(),
        if olc.script.is_some() { &b"Set."[..] } else { &b"Not Set."[..] },
    );
    out.extend_from_slice(b"\r\n");
    item(&mut out, b"W", b") Copy Room\r\n", b"", b"");
    item(&mut out, b"X", b") Delete Room\r\n", b"", b"");
    item(&mut out, b"Q", b") Quit\r\n", b"", b"");
    out.extend_from_slice(b"Enter choice : ");
    write_to_desc(g, di, &out);

    olc.mode = REDIT_MAIN_MENU;
}

// ---------------------------------------------------------------------------
// The main loop
// ---------------------------------------------------------------------------

/// redit_parse. Returns the OLC data unless the editor tore it down.
pub fn redit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let mut arg = arg.to_vec();
    match olc.mode {
        REDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    redit_save_internally(g, di, &mut olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg = format!("OLC: {} edits room {}.", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    if g.config.auto_save_olc {
                        let zone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if redit_save_to_disk(g, zone) {
                            write_to_desc(g, di, b"Room saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the room"));
                        }
                    } else {
                        write_to_desc(g, di, b"Room saved to memory.\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    // Hand the script list back to the room so the room's
                    // own cleanup frees it.
                    let script = olc.script.take();
                    if let (Some(room), Some(script)) = (olc.room.as_mut(), script) {
                        room.proto_script = script;
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to save your changes ? : ",
                    );
                }
            }
            return Some(olc);
        }

        REDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = REDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    return Some(olc);
                }
                Some(b'1') => {
                    write_to_desc(g, di, b"Enter room name:-\r\n] ");
                    olc.mode = REDIT_NAME;
                }
                Some(b'2') => {
                    olc.mode = REDIT_DESC;
                    clear_screen(g, di);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter room description:\r\n\r\n");
                    let old = olc.room.as_ref().unwrap().description.clone();
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, MAX_ROOM_DESC, 0, old);
                    }
                    olc.str_target = Some(StrTarget::RoomDesc);
                    olc.value = 1;
                }
                Some(b'3') => redit_disp_flag_menu(g, di, &mut olc),
                Some(b'4') => redit_disp_sector_menu(g, di, &mut olc),
                Some(b'5') => {
                    olc.value = NORTH as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(b'6') => {
                    olc.value = EAST as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(b'7') => {
                    olc.value = SOUTH as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(b'8') => {
                    olc.value = WEST as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(b'9') => {
                    olc.value = UP as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(b'a') | Some(b'A') => {
                    olc.value = DOWN as i32;
                    redit_disp_exit_menu(g, di, &mut olc);
                }
                Some(c @ (b'b' | b'B' | b'c' | b'C' | b'd' | b'D' | b'e' | b'E')) => {
                    if !g.config.diagonal_dirs {
                        write_to_desc(g, di, b"Invalid choice!");
                        redit_disp_menu(g, di, &mut olc);
                    } else {
                        olc.value = match c.to_ascii_lowercase() {
                            b'b' => NORTHWEST as i32,
                            b'c' => NORTHEAST as i32,
                            b'd' => SOUTHEAST as i32,
                            _ => SOUTHWEST as i32,
                        };
                        redit_disp_exit_menu(g, di, &mut olc);
                    }
                }
                Some(b'f') | Some(b'F') => {
                    let room = olc.room.as_mut().unwrap();
                    if room.ex_descriptions.is_empty() {
                        room.ex_descriptions.push(ExtraDesc::default());
                    }
                    olc.desc = Some(0);
                    redit_disp_extradesc_menu(g, di, &mut olc);
                }
                Some(b'w') | Some(b'W') => {
                    write_to_desc(g, di, b"Copy what room? ");
                    olc.mode = REDIT_COPY;
                }
                Some(b'x') | Some(b'X') => {
                    write_to_desc(g, di, b"Are you sure you want to delete this room? ");
                    olc.mode = REDIT_DELETE;
                }
                Some(b's') | Some(b'S') => {
                    olc.script_mode = crate::olc::trigedit::SCRIPT_MAIN_MENU;
                    crate::olc::trigedit::dg_script_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!");
                    redit_disp_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        crate::olc::trigedit::OLC_SCRIPT_EDIT => {
            if crate::olc::trigedit::dg_script_edit_parse(g, di, &mut olc, &arg) {
                return Some(olc);
            }
        }

        REDIT_NAME => {
            if !genolc_checkstring(&mut arg) {
                // genolc_checkstring always returns TRUE.
            } else if count_non_protocol_chars(&arg) > (MAX_ROOM_NAME / 2) as i32 {
                write_to_desc(
                    g,
                    di,
                    format!(
                        "Size limited to {} non-protocol characters.\r\n",
                        MAX_ROOM_NAME / 2
                    )
                    .as_bytes(),
                );
            } else {
                arg.truncate(MAX_ROOM_NAME - 1);
                olc.room.as_mut().unwrap().name = Some(str_udup(&arg));
            }
        }

        REDIT_DESC => {
            // We will NEVER get here, we hope.
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: Reached REDIT_DESC case in parse_redit().",
            );
            write_to_desc(g, di, b"Oops, in REDIT_DESC.\r\n");
        }

        REDIT_FLAGS => {
            let number = atoi(&arg);
            if number < 0 || number > flags::NUM_ROOM_FLAGS as i32 {
                write_to_desc(g, di, b"That is not a valid choice!\r\n");
                redit_disp_flag_menu(g, di, &mut olc);
            } else if number == 0 {
                // fall through to "something changed"
            } else {
                let bit = (number - 1) as usize;
                let f = &mut olc.room.as_mut().unwrap().room_flags;
                f[bit / 32] ^= 1 << (bit % 32);
                redit_disp_flag_menu(g, di, &mut olc);
                return Some(olc);
            }
            if number != 0 {
                return Some(olc);
            }
        }

        REDIT_SECTOR => {
            let number = atoi(&arg) - 1;
            if number < 0 || number >= flags::NUM_ROOM_SECTORS as i32 {
                write_to_desc(g, di, b"Invalid choice!");
                redit_disp_sector_menu(g, di, &mut olc);
                return Some(olc);
            }
            olc.room.as_mut().unwrap().sector_type = number;
        }

        REDIT_EXIT_MENU => {
            match arg.first().copied() {
                Some(b'0') => {}
                Some(b'1') => {
                    olc.mode = REDIT_EXIT_NUMBER;
                    write_to_desc(g, di, b"Exit to room number : ");
                    return Some(olc);
                }
                Some(b'2') => {
                    olc.mode = REDIT_EXIT_DESCRIPTION;
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter exit description:\r\n\r\n");
                    let dir = olc.value as usize;
                    let old = olc.room.as_ref().unwrap().dir_option[dir]
                        .as_ref()
                        .and_then(|e| e.general_description.clone());
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, MAX_EXIT_DESC, 0, old);
                    }
                    olc.str_target = Some(StrTarget::ExitDesc);
                    return Some(olc);
                }
                Some(b'3') => {
                    olc.mode = REDIT_EXIT_KEYWORD;
                    write_to_desc(g, di, b"Enter keywords : ");
                    return Some(olc);
                }
                Some(b'4') => {
                    olc.mode = REDIT_EXIT_KEY;
                    write_to_desc(g, di, b"Enter key number : ");
                    return Some(olc);
                }
                Some(b'5') => {
                    olc.mode = REDIT_EXIT_DOORFLAGS;
                    redit_disp_exit_flag_menu(g, di);
                    return Some(olc);
                }
                Some(b'6') => {
                    let dir = olc.value as usize;
                    olc.room.as_mut().unwrap().dir_option[dir] = None;
                }
                _ => {
                    write_to_desc(g, di, b"Try again : ");
                    return Some(olc);
                }
            }
        }

        REDIT_EXIT_NUMBER => {
            let mut number = atoi(&arg);
            if number != -1 {
                match g.real_room((number as u16) as i32) {
                    Some(r) => number = r as i32,
                    None => {
                        write_to_desc(g, di, b"That room does not exist, try again : ");
                        return Some(olc);
                    }
                }
            }
            let dir = olc.value as usize;
            if let Some(ex) = olc.room.as_mut().unwrap().dir_option[dir].as_mut() {
                ex.to_room = number as u16;
            }
            redit_disp_exit_menu(g, di, &mut olc);
            return Some(olc);
        }

        REDIT_EXIT_DESCRIPTION => {
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: Reached REDIT_EXIT_DESC case in parse_redit",
            );
            write_to_desc(g, di, b"Oops, in REDIT_EXIT_DESCRIPTION.\r\n");
        }

        REDIT_EXIT_KEYWORD => {
            let dir = olc.value as usize;
            if let Some(ex) = olc.room.as_mut().unwrap().dir_option[dir].as_mut() {
                ex.keyword = Some(str_udup(&arg));
            }
            redit_disp_exit_menu(g, di, &mut olc);
            return Some(olc);
        }

        REDIT_EXIT_KEY => {
            let number = atoi(&arg);
            let dir = olc.value as usize;
            if let Some(ex) = olc.room.as_mut().unwrap().dir_option[dir].as_mut() {
                ex.key = if number < 0 { NOTHING } else { number as u16 };
            }
            redit_disp_exit_menu(g, di, &mut olc);
            return Some(olc);
        }

        REDIT_EXIT_DOORFLAGS => {
            let number = atoi(&arg);
            if number < 0 || number > 4 {
                write_to_desc(g, di, b"That's not a valid choice!\r\n");
                redit_disp_exit_flag_menu(g, di);
            } else {
                let info = match number {
                    0 => 0,
                    1 => flags::EX_ISDOOR,
                    2 => flags::EX_ISDOOR | flags::EX_PICKPROOF,
                    3 => flags::EX_ISDOOR | flags::EX_HIDDEN,
                    _ => flags::EX_ISDOOR | flags::EX_PICKPROOF | flags::EX_HIDDEN,
                };
                let dir = olc.value as usize;
                if let Some(ex) = olc.room.as_mut().unwrap().dir_option[dir].as_mut() {
                    ex.exit_info = info;
                }
                redit_disp_exit_menu(g, di, &mut olc);
            }
            return Some(olc);
        }

        REDIT_EXTRADESC_KEY => {
            if genolc_checkstring(&mut arg) {
                let idx = olc.desc.unwrap_or(0);
                if let Some(xd) = olc.room.as_mut().unwrap().ex_descriptions.get_mut(idx) {
                    xd.keyword = Some(str_udup(&arg));
                }
            }
            redit_disp_extradesc_menu(g, di, &mut olc);
            return Some(olc);
        }

        REDIT_EXTRADESC_MENU => {
            let number = atoi(&arg);
            match number {
                0 => {
                    // An incomplete entry is dropped on the way out.
                    let idx = olc.desc.unwrap_or(0);
                    let room = olc.room.as_mut().unwrap();
                    let incomplete = room
                        .ex_descriptions
                        .get(idx)
                        .map(|x| x.keyword.is_none() || x.description.is_none())
                        .unwrap_or(false);
                    if incomplete {
                        room.ex_descriptions.remove(idx);
                        olc.desc = None;
                    }
                }
                1 => {
                    olc.mode = REDIT_EXTRADESC_KEY;
                    write_to_desc(g, di, b"Enter keywords, separated by spaces : ");
                    return Some(olc);
                }
                2 => {
                    olc.mode = REDIT_EXTRADESC_DESCRIPTION;
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter extra description:\r\n\r\n");
                    let idx = olc.desc.unwrap_or(0);
                    let old = olc
                        .room
                        .as_ref()
                        .unwrap()
                        .ex_descriptions
                        .get(idx)
                        .and_then(|x| x.description.clone());
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, crate::boards::MAX_MESSAGE_LENGTH, 0, old);
                    }
                    olc.str_target = Some(StrTarget::ExtraDesc);
                    return Some(olc);
                }
                3 => {
                    let idx = olc.desc.unwrap_or(0);
                    let room = olc.room.as_mut().unwrap();
                    let incomplete = room
                        .ex_descriptions
                        .get(idx)
                        .map(|x| x.keyword.is_none() || x.description.is_none())
                        .unwrap_or(true);
                    if incomplete {
                        write_to_desc(
                            g,
                            di,
                            b"You can't edit the next extra description without completing this one.\r\n",
                        );
                        redit_disp_extradesc_menu(g, di, &mut olc);
                    } else {
                        if idx + 1 < room.ex_descriptions.len() {
                            olc.desc = Some(idx + 1);
                        } else {
                            room.ex_descriptions.push(ExtraDesc::default());
                            olc.desc = Some(room.ex_descriptions.len() - 1);
                        }
                        redit_disp_extradesc_menu(g, di, &mut olc);
                    }
                    return Some(olc);
                }
                _ => {}
            }
        }

        REDIT_COPY => {
            match g.real_room(atoi(&arg)) {
                Some(number) => redit_setup_existing(g, &mut olc, number as usize),
                None => write_to_desc(g, di, b"That room does not exist.\r\n"),
            }
        }

        REDIT_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let vnum = olc.room.as_ref().unwrap().vnum;
                    let rnum = g.world.real_room(vnum).unwrap_or(NOWHERE);
                    if delete_room(g, rnum) {
                        write_to_desc(g, di, b"Room deleted.\r\n");
                        // Same toggle the save path honours.
                        if g.config.auto_save_olc {
                            crate::db::save_all(g);
                        }
                    } else {
                        write_to_desc(g, di, b"Couldn't delete the room!.\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    redit_disp_menu(g, di, &mut olc);
                    olc.mode = REDIT_MAIN_MENU;
                    return Some(olc);
                }
                _ => write_to_desc(g, di, b"Please answer 'Y' or 'N': "),
            }
        }

        _ => {
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: Reached default case in parse_redit",
            );
        }
    }

    // If we get this far, something has been changed.
    olc.value = 1;
    redit_disp_menu(g, di, &mut olc);
    Some(olc)
}

/// redit_string_cleanup plus the write-back half that
/// gets for free by pointing `d->str` at the field.
pub fn redit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    match olc.str_target.take() {
        Some(StrTarget::RoomDesc) => {
            olc.room.as_mut().unwrap().description = text;
        }
        Some(StrTarget::ExitDesc) => {
            let dir = olc.value as usize;
            if let Some(ex) = olc.room.as_mut().unwrap().dir_option[dir].as_mut() {
                ex.general_description = text;
            }
        }
        Some(StrTarget::ExtraDesc) => {
            let idx = olc.desc.unwrap_or(0);
            if let Some(xd) = olc.room.as_mut().unwrap().ex_descriptions.get_mut(idx) {
                xd.description = text;
            }
        }
        _ => {}
    }
    match olc.mode {
        REDIT_DESC => redit_disp_menu(g, di, &mut olc),
        REDIT_EXIT_DESCRIPTION => redit_disp_exit_menu(g, di, &mut olc),
        REDIT_EXTRADESC_DESCRIPTION => redit_disp_extradesc_menu(g, di, &mut olc),
        _ => {}
    }
    Some(olc)
}
