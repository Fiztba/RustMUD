//! The zone editor: the zone header and the reset command list.
//!
//! zedit is per-*room*, not per-zone: `number` holds the room vnum the
//! builder is standing on (or named), and the scratch zone carries only the
//! reset commands that target that room. Everything else about the zone —
//! name, builders, lifespan, bounds, flags, level range — is edited on the
//! same screen but belongs to the whole zone.
//!
//! Two shapes worth naming because they are observable:
//!
//! * The scratch zone reuses two of its own fields as dirty flags:
//! `number` means "the header changed" and `age` means "the command list
//! changed". Our static `Zone` has no `age` — the reset
//! clock lives in `zones_rt` — so that half sits in `OlcData::zone_age`.
//! * The duplicate-editor scan says "That zone is currently being edited by
//! $N" but compares `number`, which in zedit is a *room* vnum. Two
//! builders in the same zone on different rooms both get in; the same
//! room twice is what it actually blocks.

use std::cmp::Ordering;

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{DIRS, EQUIPMENT_TYPES, ZONE_BITS};
use mud_data::types::*;
use mud_world::model::{Zone, ZoneCommand};

use crate::act::informative::{column_list, sprintbitarray};
use crate::act::BStr;
use crate::comm::{act, send_to_char, write_to_desc, TO_ROOM};
use crate::db::{add_to_save_list, save_zone, SL_ZON};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::{is_number, one_argument, two_arguments};
use crate::olc::genzon::{
    add_cmd_to_list, count_commands, create_new_zone, delete_zone_command, new_command,
    remove_room_zone_commands,
};
use crate::olc::{
    atoidx, can_edit_zone, clear_screen, genolc_checkstring, get_char_colors, send_cannot_edit,
    OlcData, CLEANUP_ALL, MAX_DUPLICATES,
};

/// Submodes of ZEDIT connectedness.
pub const ZEDIT_MAIN_MENU: i32 = 0;
pub const ZEDIT_DELETE_ENTRY: i32 = 1;
pub const ZEDIT_NEW_ENTRY: i32 = 2;
pub const ZEDIT_CHANGE_ENTRY: i32 = 3;
pub const ZEDIT_COMMAND_TYPE: i32 = 4;
pub const ZEDIT_IF_FLAG: i32 = 5;
pub const ZEDIT_ARG1: i32 = 6;
pub const ZEDIT_ARG2: i32 = 7;
pub const ZEDIT_ARG3: i32 = 8;
pub const ZEDIT_ZONE_NAME: i32 = 9;
pub const ZEDIT_ZONE_LIFE: i32 = 10;
pub const ZEDIT_ZONE_BOT: i32 = 11;
pub const ZEDIT_ZONE_TOP: i32 = 12;
pub const ZEDIT_ZONE_RESET: i32 = 13;
pub const ZEDIT_CONFIRM_SAVESTRING: i32 = 14;
pub const ZEDIT_ZONE_BUILDERS: i32 = 15;
pub const ZEDIT_SARG1: i32 = 20;
pub const ZEDIT_SARG2: i32 = 21;
pub const ZEDIT_ZONE_FLAGS: i32 = 22;
pub const ZEDIT_LEVELS: i32 = 23;
pub const ZEDIT_LEV_MIN: i32 = 24;
pub const ZEDIT_LEV_MAX: i32 = 25;

/// LIMIT: `MIN(high, MAX(var, low))`.
fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

fn dir_count(g: &Game) -> i32 {
    if g.config.diagonal_dirs {
        10
    } else {
        6
    }
}

/// The reset command under edit. Indexed raw: every caller reaches it
/// through `start_change_command`, which has already range-checked the
/// index.
fn cmd_of(olc: &OlcData) -> Option<&ZoneCommand> {
    olc.zone.as_ref()?.cmds.get(olc.value as usize)
}

fn cmd_mut(olc: &mut OlcData) -> Option<&mut ZoneCommand> {
    olc.zone.as_mut()?.cmds.get_mut(olc.value as usize)
}

/// The command letter under edit, or `\0` when there is none.
fn cmd_letter(olc: &OlcData) -> u8 {
    cmd_of(olc).map_or(0, |c| c.command)
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_zedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    // No building as a mob or while being forced.
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, rest) = two_arguments(argument);
    let (sbot, stop) = one_argument(rest);
    let mut number: i32 = NOWHERE as i32;
    let mut save = false;

    // If no argument was given, use the zone the builder is standing in.
    if buf1.is_empty() {
        number = g.world.rooms[g.ch(chid).in_room as usize].vnum as i32;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) == Ordering::Equal {
            save = true;
            if is_number(&buf2) {
                number = atoidx(&buf2);
            } else {
                let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
                if olc_zone > 0 {
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
        } else if g.ch(chid).level >= LVL_IMPL {
            if crate::text::cmp_ci(b"new", &buf1) != Ordering::Equal || stop.is_empty() {
                send_to_char(
                    g,
                    chid,
                    b"Format: zedit new <zone number> <bottom-room> <upper-room>\r\n",
                );
            } else if atoi(stop) < 0 || atoi(&sbot) < 0 {
                send_to_char(g, chid, b"Zones cannot contain negative vnums.\r\n");
                return;
            } else {
                // atoidx folds a negative or oversized vnum to NOWHERE,
                // so the `number < 0` guard that follows it never fires.
                let number = atoidx(&buf2);
                let bottom = atoidx(&sbot);
                let top = atoidx(stop);
                zedit_new_zone(g, chid, number, bottom, top);
            }
            return;
        } else {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
    }

    // If a numeric argument was given, retrieve it.
    if number == NOWHERE as i32 {
        number = atoidx(&buf1);
    }

    // Check that nobody is currently editing this zone.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Zedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        let mut msg = b"That zone is currently being edited by ".to_vec();
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
            "SYSERR: do_oasis_zedit: Player already had olc structure.",
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

    // Everyone but IMPLs can only edit zones they have been assigned.
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
            format!("Saving all zone information for zone {}.\r\n", zvnum).as_bytes(),
        );
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves zone information for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_zone(g, znum as usize);
        return;
    }

    olc.number = number;

    let Some(real_num) = g.real_room(number) else {
        write_to_desc(g, di, b"That room does not exist.\r\n");
        return;
    };

    zedit_setup(g, di, &mut olc, real_num as usize);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Zedit;
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

fn zedit_setup(g: &mut Game, di: usize, olc: &mut OlcData, room_num: usize) {
    let src = &g.world.zones[olc.zone_num as usize];
    let mut zone = Zone {
        name: Some(src.name.clone().unwrap_or_default()),
        builders: src.builders.clone(),
        lifespan: src.lifespan,
        bot: src.bot,
        top: src.top,
        reset_mode: src.reset_mode,
        // The remaining fields are used as a 'has been modified' flag.
        number: 0,
        zone_flags: src.zone_flags,
        min_level: src.min_level,
        max_level: src.max_level,
        cmds: Vec::new(),
    };
    olc.zone_age = 0;

    // Add every reset command that relates to this room. `cmd_room` is
    // deliberately not reset between iterations: a 'G'/'E'/'P' inherits the
    // room of the 'M'/'O' it follows, which is how OasisOLC groups them.
    let mut cmd_room: i32 = NOWHERE as i32;
    let mut count = 0usize;
    let src_cmds = g.world.zones[olc.zone_num as usize].cmds.clone();
    for cmd in src_cmds {
        match cmd.command {
            b'M' | b'O' | b'T' | b'V' => cmd_room = cmd.arg3,
            b'D' | b'R' => cmd_room = cmd.arg1,
            _ => {}
        }
        if cmd_room == room_num as i32 {
            add_cmd_to_list(&mut zone, cmd, count);
            count += 1;
        }
    }

    olc.zone = Some(Box::new(zone));
    zedit_disp_menu(g, di, olc);
}

fn zedit_new_zone(g: &mut Game, chid: CharId, vzone_num: i32, bottom: i32, top: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let result = match create_new_zone(g, vzone_num, bottom, top) {
        Ok(r) => r as i32,
        Err(error) => {
            write_to_desc(g, di, error.as_bytes());
            return;
        }
    };

    // Every builder already inside an editor shifts with the zone table.
    for dsc in g.descriptors.order.clone() {
        let state = g.descriptors.get(dsc).map(|d| d.state);
        let Some(olc) = g.olc.get_mut(&dsc) else { continue };
        match state {
            Some(ConState::Redit) => {
                if olc.zone_num >= result {
                    if let Some(room) = olc.room.as_mut() {
                        room.zone += 1;
                    }
                    olc.zone_num += 1;
                }
            }
            Some(
                ConState::Zedit
                | ConState::Medit
                | ConState::Sedit
                | ConState::Oedit
                | ConState::Trigedit
                | ConState::Qedit,
            ) => {
                if olc.zone_num >= result {
                    olc.zone_num += 1;
                }
            }
            _ => {}
        }
    }

    save_zone(g, result as usize);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
    let msg = format!("OLC: {} creates new zone #{}", name, vzone_num);
    g.mudlog(MudlogKind::Brf, level, true, &msg);
    write_to_desc(g, di, b"Zone created successfully.\r\n");
}

fn zedit_save_internally(g: &mut Game, di: usize, olc: &mut OlcData) {
    let Some(room_num) = g.real_room(olc.number) else {
        g.log(format!(
            "SYSERR: zedit_save_internally: OLC_NUM(d) room {} not found.",
            olc.number
        ));
        return;
    };
    let zone = olc.zone_num as usize;

    remove_room_zone_commands(g, zone, room_num);

    // Circle does not record which room a 'G'/'E'/'P' belongs to, but Oasis
    // groups reset commands by room — so a give/equip/put with nothing
    // loaded before it would wander the zone list looking for something to
    // latch onto. Those are dropped here instead (C.Raehl 4/27/99).
    //
    // Using `subcmd` as both the scratch index and the insert
    // position lets a skip leave the position running ahead of the real
    // list. `add_cmd_to_list` never inserts past the end — it copies the
    // old 'S' terminator into place and drops the command without a word —
    // so on a zone whose command list is shorter than the scratch index (a
    // new zone, or one where this room owns most of the resets) a single
    // misplaced give/equip/put silently discards **every** reset command
    // for the room. The insert position therefore advances only when
    // something is actually inserted.
    let mut mobloaded = false;
    let mut objloaded = false;
    let mut pos = 0usize;
    let cmds = olc.zone.as_ref().map(|z| z.cmds.clone()).unwrap_or_default();
    for cmd in cmds.into_iter() {
        match cmd.command {
            b'G' | b'E' => {
                if !mobloaded {
                    write_to_desc(
                        g,
                        di,
                        b"Equip/Give command not saved since no mob was loaded first.\r\n",
                    );
                    continue;
                }
            }
            b'P' => {
                if !objloaded {
                    write_to_desc(
                        g,
                        di,
                        b"Put command not saved since another object was not loaded first.\r\n",
                    );
                    continue;
                }
            }
            b'M' => mobloaded = true,
            b'O' => objloaded = true,
            _ => {
                mobloaded = false;
                objloaded = false;
            }
        }
        add_cmd_to_list(&mut g.world.zones[zone], cmd, pos);
        pos += 1;
    }

    // Finally, if zone headers have been changed, copy over.
    if olc.zone.as_ref().is_some_and(|z| z.number != 0) {
        let scratch = olc.zone.as_ref().unwrap();
        let (name, builders) = (scratch.name.clone(), scratch.builders.clone());
        let (bot, top) = (scratch.bot, scratch.top);
        let (reset_mode, lifespan) = (scratch.reset_mode, scratch.lifespan);
        let (min_level, max_level) = (scratch.min_level, scratch.max_level);
        let zone_flags = scratch.zone_flags;
        let dst = &mut g.world.zones[zone];
        dst.name = name;
        dst.builders = builders;
        dst.bot = bot;
        dst.top = top;
        dst.reset_mode = reset_mode;
        dst.lifespan = lifespan;
        dst.min_level = min_level;
        dst.max_level = max_level;
        dst.zone_flags = zone_flags;
    }
    let number = g.world.zones[zone].number;
    add_to_save_list(g, number, SL_ZON);
}

fn zedit_save_to_disk(g: &mut Game, zone: usize) -> bool {
    save_zone(g, zone)
}

fn start_change_command(olc: &mut OlcData, pos: i32) -> bool {
    let count = olc.zone.as_deref().map_or(0, count_commands) as i32;
    if pos < 0 || pos >= count {
        return false;
    }
    olc.value = pos;
    true
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn zedit_disp_flag_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let names: Vec<BStr> = ZONE_BITS
            .iter()
            .take(flags::NUM_ZONE_FLAGS)
            .map(|s| s.as_bytes().to_vec())
            .collect();
        column_list(g, chid, 0, &names, true);
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(
        &olc.zone.as_ref().unwrap().zone_flags,
        &ZONE_BITS[..flags::NUM_ZONE_FLAGS],
        &mut bits,
    );
    let mut out: BStr = b"\r\nZone flags: \tc".to_vec();
    out.extend_from_slice(&bits);
    out.extend_from_slice(b"\tn\r\nEnter Zone flags, 0 to quit : ");
    write_to_desc(g, di, &out);
    olc.mode = ZEDIT_ZONE_FLAGS;
}

/// zedit_get_levels: the level-range blurb, and whether
/// any recommendation is set at all.
fn zedit_get_levels(olc: &OlcData) -> (String, bool) {
    let zone = olc.zone.as_ref().unwrap();
    let (min, max) = (zone.min_level, zone.max_level);
    if min == -1 && max == -1 {
        return ("<Not Set!>".to_string(), false);
    }
    if min == -1 {
        return (format!("Up to level {}", max), true);
    }
    if max == -1 {
        return (format!("Above level {}", min), true);
    }
    (format!("Levels {} to {}", min, max), true)
}

/// The `%s` a NULL `const char *` renders as under glibc.
fn or_null(s: Option<&[u8]>) -> BStr {
    s.map_or_else(|| b"(null)".to_vec(), |b| b.to_vec())
}

/// zedit_disp_menu — the main menu.
fn zedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    get_char_colors(g, chid);
    clear_screen(g, di);

    let mut buf1: BStr = Vec::new();
    sprintbitarray(
        &olc.zone.as_ref().unwrap().zone_flags,
        &ZONE_BITS[..flags::NUM_ZONE_FLAGS],
        &mut buf1,
    );
    let (lev_string, levels_set) = zedit_get_levels(olc);

    let c = g.olc_colors;
    let zone = olc.zone.as_ref().unwrap();
    let reset: &[u8] = match zone.reset_mode {
        0 => b"Never reset",
        1 => b"Reset when no players are in zone.",
        _ => b"Normal reset.",
    };
    let zvnum = g.world.zones[olc.zone_num as usize].number;
    let lev_color = if levels_set { c.cyn() } else { c.yel() };

    // Menu header.
    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!(
            "Room number: {}{}{} Room zone: {}{}\r\n",
            c.cyn_s(),
            olc.number,
            c.nrm_s(),
            c.cyn_s(),
            zvnum
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!("{}1{}) Builders       : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes(),
    );
    out.extend_from_slice(&or_null(zone.builders.as_deref()));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!("{}Z{}) Zone name      : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes(),
    );
    out.extend_from_slice(&or_null(zone.name.as_deref()));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}L{}) Lifespan       : {}{} minutes\r\n",
            c.grn_s(),
            c.nrm_s(),
            c.yel_s(),
            zone.lifespan
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{}B{}) Bottom of zone : {}{}\r\n",
            c.grn_s(),
            c.nrm_s(),
            c.yel_s(),
            zone.bot
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!("{}T{}) Top of zone    : {}{}\r\n", c.grn_s(), c.nrm_s(), c.yel_s(), zone.top)
            .as_bytes(),
    );
    out.extend_from_slice(
        format!("{}R{}) Reset Mode     : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes(),
    );
    out.extend_from_slice(reset);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!("{}F{}) Zone Flags     : {}", c.grn_s(), c.nrm_s(), c.cyn_s()).as_bytes(),
    );
    out.extend_from_slice(&buf1);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}M{}) Level Range    : ", c.grn_s(), c.nrm_s()).as_bytes());
    out.extend_from_slice(lev_color);
    out.extend_from_slice(lev_string.as_bytes());
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\n[Command list]\r\n");
    send_to_char(g, chid, &out);

    // Print the commands for this room into the display buffer. Each entry
    // goes out as two writes, and the split matters: the descriptor's
    // output buffer switches to the large pool on a byte count.
    let cmds = olc.zone.as_ref().unwrap().cmds.clone();
    for (counter, cmd) in cmds.iter().enumerate() {
        let c = g.olc_colors;
        let (nrm, cyn, yel) =
            (c.nrm().to_vec(), c.cyn().to_vec(), c.yel().to_vec());
        let mut line: BStr = nrm.clone();
        line.extend_from_slice(format!("{} - ", counter).as_bytes());
        line.extend_from_slice(&yel);
        write_to_desc(g, di, &line);

        let then: &[u8] = if cmd.if_flag != 0 { b" then " } else { b"" };
        let mut body: BStr = then.to_vec();
        match cmd.command {
            b'M' => {
                body.extend_from_slice(b"Load ");
                body.extend_from_slice(&proto_mob_desc(g, cmd.arg1));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_mob_vnum(g, cmd.arg1).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(format!("], Max : {}", cmd.arg2).as_bytes());
            }
            b'G' | b'O' => {
                body.extend_from_slice(if cmd.command == b'G' { &b"Give it "[..] } else { b"Load " });
                body.extend_from_slice(&proto_obj_desc(g, cmd.arg1));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_obj_vnum(g, cmd.arg1).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(format!("], Max : {}", cmd.arg2).as_bytes());
            }
            b'E' => {
                body.extend_from_slice(b"Equip with ");
                body.extend_from_slice(&proto_obj_desc(g, cmd.arg1));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_obj_vnum(g, cmd.arg1).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(b"], ");
                body.extend_from_slice(
                    EQUIPMENT_TYPES.get(cmd.arg3 as usize).copied().unwrap_or("\n").as_bytes(),
                );
                body.extend_from_slice(format!(", Max : {}", cmd.arg2).as_bytes());
            }
            b'P' => {
                body.extend_from_slice(b"Put ");
                body.extend_from_slice(&proto_obj_desc(g, cmd.arg1));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_obj_vnum(g, cmd.arg1).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(b"] in ");
                body.extend_from_slice(&proto_obj_desc(g, cmd.arg3));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_obj_vnum(g, cmd.arg3).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(format!("], Max : {}", cmd.arg2).as_bytes());
            }
            b'R' => {
                body.extend_from_slice(b"Remove ");
                body.extend_from_slice(&proto_obj_desc(g, cmd.arg2));
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_obj_vnum(g, cmd.arg2).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(b"] from room.");
            }
            b'D' => {
                body.extend_from_slice(b"Set door ");
                // dirs[] carries a "\n" sentinel at index 10, which the
                // ARG2 off-by-one in zedit_parse can actually reach.
                body.extend_from_slice(
                    DIRS.get(cmd.arg2 as usize).copied().unwrap_or("\n").as_bytes(),
                );
                body.extend_from_slice(b" as ");
                body.extend_from_slice(match cmd.arg3 {
                    0 => &b"open"[..],
                    1 => b"closed",
                    _ => b"locked",
                });
                body.push(b'.');
            }
            b'T' => {
                body.extend_from_slice(b"Attach trigger ");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(&proto_trig_name(g, cmd.arg2));
                body.extend_from_slice(&yel);
                body.extend_from_slice(b" [");
                body.extend_from_slice(&cyn);
                body.extend_from_slice(proto_trig_vnum(g, cmd.arg2).to_string().as_bytes());
                body.extend_from_slice(&yel);
                body.extend_from_slice(b"] to ");
                body.extend_from_slice(trigger_attach_word(cmd.arg1).as_bytes());
            }
            b'V' => {
                body.extend_from_slice(b"Assign global ");
                body.extend_from_slice(&or_null(cmd.sarg1.as_deref()));
                body.extend_from_slice(format!(":{} to ", cmd.arg2).as_bytes());
                body.extend_from_slice(trigger_attach_word(cmd.arg1).as_bytes());
                body.extend_from_slice(b" = ");
                body.extend_from_slice(&or_null(cmd.sarg2.as_deref()));
            }
            _ => {
                // The `then` prefix belongs to the recognised cases only.
                body.clear();
                body.extend_from_slice(b"<Unknown Command>");
            }
        }
        write_to_desc(g, di, &body);
        write_to_desc(g, di, b"\r\n");
    }

    // Finish off menu.
    let c = g.olc_colors;
    let (nrm, grn) = (c.nrm_s(), c.grn_s());
    let footer = format!(
        "{}{} - <END OF LIST>\r\n\
         {}N{}) Insert new command.\r\n\
         {}E{}) Edit a command.\r\n\
         {}D{}) Delete a command.\r\n\
         {}Q{}) Quit\r\nEnter your choice : ",
        nrm,
        cmds.len(),
        grn,
        nrm,
        grn,
        nrm,
        grn,
        nrm,
        grn,
        nrm
    );
    write_to_desc(g, di, footer.as_bytes());

    olc.mode = ZEDIT_MAIN_MENU;
}

fn trigger_attach_word(arg1: i32) -> &'static str {
    if arg1 == crate::dg::MOB_TRIGGER {
        "mobile"
    } else if arg1 == crate::dg::OBJ_TRIGGER {
        "object"
    } else if arg1 == crate::dg::WLD_TRIGGER {
        "room"
    } else {
        "????"
    }
}

fn proto_mob_desc(g: &Game, rnum: i32) -> BStr {
    or_null(
        g.world
            .mob_protos
            .get(rnum as usize)
            .and_then(|p| p.short_descr.as_deref()),
    )
}

fn proto_mob_vnum(g: &Game, rnum: i32) -> i32 {
    g.world.mob_protos.get(rnum as usize).map_or(-1, |p| p.vnum as i32)
}

fn proto_obj_desc(g: &Game, rnum: i32) -> BStr {
    or_null(
        g.world
            .obj_protos
            .get(rnum as usize)
            .and_then(|p| p.short_description.as_deref()),
    )
}

fn proto_obj_vnum(g: &Game, rnum: i32) -> i32 {
    g.world.obj_protos.get(rnum as usize).map_or(-1, |p| p.vnum as i32)
}

fn proto_trig_name(g: &Game, rnum: i32) -> BStr {
    or_null(g.world.triggers.get(rnum as usize).and_then(|t| t.name.as_deref()))
}

fn proto_trig_vnum(g: &Game, rnum: i32) -> i32 {
    g.world.triggers.get(rnum as usize).map_or(-1, |t| t.vnum as i32)
}

fn zedit_disp_comtype(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let c = g.olc_colors;
    let (nrm, grn) = (c.nrm_s(), c.grn_s());
    let out = format!(
        "\r\n\
         {}M{}) Load Mobile to room             {}O{}) Load Object to room\r\n\
         {}E{}) Equip mobile with object        {}G{}) Give an object to a mobile\r\n\
         {}P{}) Put object in another object    {}D{}) Open/Close/Lock a Door\r\n\
         {}R{}) Remove an object from the room\r\n\
         {}T{}) Assign a trigger                {}V{}) Set a global variable\r\n\
         \r\n\
         What sort of command will this be? : ",
        grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = ZEDIT_COMMAND_TYPE;
}

/// zedit_disp_arg1. `Err` carries the "we should never get here" SYSERR
/// text; the caller runs the cleanup.
fn zedit_disp_arg1(g: &mut Game, di: usize, olc: &mut OlcData) -> Result<(), &'static str> {
    write_to_desc(g, di, b"\r\n");

    match cmd_letter(olc) {
        b'M' => {
            write_to_desc(g, di, b"Input mob's vnum : ");
            olc.mode = ZEDIT_ARG1;
        }
        b'O' | b'E' | b'P' | b'G' => {
            write_to_desc(g, di, b"Input object vnum : ");
            olc.mode = ZEDIT_ARG1;
        }
        b'D' | b'R' => {
            // Arg1 for these is the room number, skip to arg2.
            let room = g.real_room(olc.number).map_or(NOWHERE as i32, |r| r as i32);
            if let Some(cmd) = cmd_mut(olc) {
                cmd.arg1 = room;
            }
            return zedit_disp_arg2(g, di, olc);
        }
        b'T' | b'V' => {
            write_to_desc(g, di, b"Input trigger type (0:mob, 1:obj, 2:room) :");
            olc.mode = ZEDIT_ARG1;
        }
        _ => return Err("SYSERR: OLC: zedit_disp_arg1(): Help!"),
    }
    Ok(())
}

fn zedit_disp_arg2(g: &mut Game, di: usize, olc: &mut OlcData) -> Result<(), &'static str> {
    write_to_desc(g, di, b"\r\n");

    match cmd_letter(olc) {
        b'M' | b'O' | b'E' | b'P' | b'G' => {
            write_to_desc(g, di, b"Input the maximum number that can exist on the mud : ");
        }
        b'D' => {
            // The listing walks dirs[] to its "\n" sentinel, so it always
            // offers all ten even when diagonals are configured off.
            for (i, dir) in DIRS.iter().enumerate() {
                write_to_desc(g, di, format!("{}) Exit {}.\r\n", i, dir).as_bytes());
            }
            write_to_desc(g, di, b"Enter exit number for door : ");
        }
        b'R' => write_to_desc(g, di, b"Input object's vnum : "),
        b'T' => write_to_desc(g, di, b"Enter the trigger VNum : "),
        b'V' => write_to_desc(g, di, b"Global's context (0 for none) : "),
        _ => return Err("SYSERR: OLC: zedit_disp_arg2(): Help!"),
    }
    olc.mode = ZEDIT_ARG2;
    Ok(())
}

fn zedit_disp_arg3(g: &mut Game, di: usize, olc: &mut OlcData) -> Result<(), &'static str> {
    write_to_desc(g, di, b"\r\n");

    match cmd_letter(olc) {
        b'E' => {
            if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                let names: Vec<BStr> = EQUIPMENT_TYPES
                    .iter()
                    .take(NUM_WEARS)
                    .map(|s| s.as_bytes().to_vec())
                    .collect();
                column_list(g, chid, 0, &names, true);
            }
            write_to_desc(g, di, b"Location to equip : ");
        }
        b'P' => write_to_desc(g, di, b"Virtual number of the container : "),
        b'D' => write_to_desc(
            g,
            di,
            b"0)  Door open\r\n1)  Door closed\r\n2)  Door locked\r\nEnter state of the door : ",
        ),
        _ => return Err("SYSERR: OLC: zedit_disp_arg3(): Help!"),
    }
    olc.mode = ZEDIT_ARG3;
    Ok(())
}

fn zedit_disp_levels(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (lev_string, levels_set) = zedit_get_levels(olc);
    clear_screen(g, di);
    let out = format!(
        "\r\n\
         \ty1\tn) Set minimum level recommendation\r\n\
         \ty2\tn) Set maximum level recommendation\r\n\
         \ty3\tn) Clear level recommendations\r\n\r\n\
         \ty0\tn) Quit to main menu\r\n\
         \tgCurrent Setting: {}{}\tn\r\n\
         \r\n\
         Enter choice (0 to quit): ",
        if levels_set { "\tc" } else { "\ty" },
        lev_string
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = ZEDIT_LEVELS;
}

/// The shared tail of every branch that gives up: cleanup first, then the
/// Log a SYSERR, then send "Oops..." to a descriptor that is back in play.
fn zedit_oops(g: &mut Game, di: usize, olc: Box<OlcData>, msg: &str) {
    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
    g.mudlog(MudlogKind::Brf, LVL_BUILDER, true, msg);
    write_to_desc(g, di, b"Oops...\r\n");
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn zedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    match olc.mode {
        ZEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    zedit_save_internally(g, di, &mut olc);
                    if g.config.auto_save_olc {
                        write_to_desc(g, di, b"Saving zone info to disk.\r\n");
                        if !zedit_save_to_disk(g, olc.zone_num as usize) {
                            write_to_desc(g, di, &crate::olc::save_failed("the zone"));
                        }
                    } else {
                        write_to_desc(g, di, b"Saving zone info in memory.\r\n");
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg =
                            format!("OLC: {} edits zone info for room {}.", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    write_to_desc(g, di, b"Do you wish to save your changes? : ");
                }
            }
        }

        ZEDIT_MAIN_MENU => match arg.first().copied() {
            Some(b'q') | Some(b'Q') => {
                let dirty = olc.zone_age != 0 || olc.zone.as_ref().is_some_and(|z| z.number != 0);
                if dirty {
                    write_to_desc(g, di, b"Do you wish to save your changes? : ");
                    olc.mode = ZEDIT_CONFIRM_SAVESTRING;
                } else {
                    write_to_desc(g, di, b"No changes made.\r\n");
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
            }
            Some(b'n') | Some(b'N') => {
                // New entry. An empty list skips the position prompt.
                let empty = olc.zone.as_ref().is_some_and(|z| z.cmds.is_empty());
                if empty {
                    let ok = new_command(olc.zone.as_mut().unwrap(), 0)
                        && start_change_command(&mut olc, 0);
                    if ok {
                        zedit_disp_comtype(g, di, &mut olc);
                        olc.zone_age = 1;
                        return Some(olc);
                    }
                }
                write_to_desc(g, di, b"What number in the list should the new command be? : ");
                olc.mode = ZEDIT_NEW_ENTRY;
            }
            Some(b'e') | Some(b'E') => {
                write_to_desc(g, di, b"Which command do you wish to change? : ");
                olc.mode = ZEDIT_CHANGE_ENTRY;
            }
            Some(b'd') | Some(b'D') => {
                write_to_desc(g, di, b"Which command do you wish to delete? : ");
                olc.mode = ZEDIT_DELETE_ENTRY;
            }
            Some(b'z') | Some(b'Z') => {
                write_to_desc(g, di, b"Enter new zone name : ");
                olc.mode = ZEDIT_ZONE_NAME;
            }
            Some(b'1') => {
                write_to_desc(g, di, b"Enter new builders list : ");
                olc.mode = ZEDIT_ZONE_BUILDERS;
            }
            Some(b'b') | Some(b'B') => {
                let level = g
                    .descriptors
                    .get(di)
                    .and_then(|d| d.character)
                    .map_or(0, |c| g.ch(c).level);
                if level < LVL_IMPL {
                    zedit_disp_menu(g, di, &mut olc);
                } else {
                    write_to_desc(g, di, b"Enter new bottom of zone : ");
                    olc.mode = ZEDIT_ZONE_BOT;
                }
            }
            Some(b't') | Some(b'T') => {
                let level = g
                    .descriptors
                    .get(di)
                    .and_then(|d| d.character)
                    .map_or(0, |c| g.ch(c).level);
                if level < LVL_IMPL {
                    zedit_disp_menu(g, di, &mut olc);
                } else {
                    write_to_desc(g, di, b"Enter new top of zone : ");
                    olc.mode = ZEDIT_ZONE_TOP;
                }
            }
            Some(b'l') | Some(b'L') => {
                write_to_desc(g, di, b"Enter new zone lifespan : ");
                olc.mode = ZEDIT_ZONE_LIFE;
            }
            Some(b'r') | Some(b'R') => {
                write_to_desc(
                    g,
                    di,
                    b"\r\n0) Never reset\r\n1) Reset only when no players in zone\r\n\
                      2) Normal reset\r\nEnter new zone reset type : ",
                );
                olc.mode = ZEDIT_ZONE_RESET;
            }
            Some(b'f') | Some(b'F') => zedit_disp_flag_menu(g, di, &mut olc),
            Some(b'm') | Some(b'M') => zedit_disp_levels(g, di, &mut olc),
            _ => zedit_disp_menu(g, di, &mut olc),
        },

        ZEDIT_LEVELS => match arg.first().copied() {
            Some(b'1') => {
                let msg = format!(
                    "Enter the min level for this zone (0-{}, -1 = none): ",
                    LVL_IMMORT - 1
                );
                write_to_desc(g, di, msg.as_bytes());
                olc.mode = ZEDIT_LEV_MIN;
            }
            Some(b'2') => {
                let msg = format!(
                    "Enter the max level for this zone (0-{}, -1 = none): ",
                    LVL_IMMORT - 1
                );
                write_to_desc(g, di, msg.as_bytes());
                olc.mode = ZEDIT_LEV_MAX;
            }
            Some(b'3') => {
                if let Some(zone) = olc.zone.as_mut() {
                    zone.min_level = -1;
                    zone.max_level = -1;
                    zone.number = 1;
                }
                zedit_disp_menu(g, di, &mut olc);
            }
            Some(b'0') => zedit_disp_menu(g, di, &mut olc),
            _ => write_to_desc(g, di, b"Invalid choice!\r\n"),
        },

        ZEDIT_LEV_MIN => {
            let pos = atoi(arg);
            if let Some(zone) = olc.zone.as_mut() {
                zone.min_level = pos.max(-1).min(100);
                zone.number = 1;
            }
            zedit_disp_levels(g, di, &mut olc);
        }

        ZEDIT_LEV_MAX => {
            let pos = atoi(arg);
            if let Some(zone) = olc.zone.as_mut() {
                zone.max_level = pos.max(-1).min(100);
                zone.number = 1;
            }
            zedit_disp_levels(g, di, &mut olc);
        }

        ZEDIT_NEW_ENTRY => {
            // Get the line number and insert the new line.
            let pos = atoi(arg);
            let digit = arg.first().is_some_and(|c| c.is_ascii_digit());
            if digit && new_command(olc.zone.as_mut().unwrap(), pos) {
                if start_change_command(&mut olc, pos) {
                    zedit_disp_comtype(g, di, &mut olc);
                    olc.zone_age = 1;
                }
            } else {
                zedit_disp_menu(g, di, &mut olc);
            }
        }

        ZEDIT_DELETE_ENTRY => {
            let pos = atoi(arg);
            if arg.first().is_some_and(|c| c.is_ascii_digit()) {
                delete_zone_command(olc.zone.as_mut().unwrap(), pos);
                olc.zone_age = 1;
            }
            zedit_disp_menu(g, di, &mut olc);
        }

        ZEDIT_CHANGE_ENTRY => {
            // 'A' aborts back to the main menu, retiring a command that was
            // created but never given a type (Mark Garringer's idea).
            if arg.first().copied().map(|c| c.to_ascii_uppercase()) == Some(b'A') {
                if cmd_letter(&olc) == b'N' {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.command = b'*';
                    }
                }
                zedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            }
            let pos = atoi(arg);
            if arg.first().is_some_and(|c| c.is_ascii_digit())
                && start_change_command(&mut olc, pos)
            {
                zedit_disp_comtype(g, di, &mut olc);
                olc.zone_age = 1;
            } else {
                zedit_disp_menu(g, di, &mut olc);
            }
        }

        ZEDIT_COMMAND_TYPE => {
            // The letter is stored before it is validated, so a rejected
            // keystroke still overwrites the command under edit.
            let letter = arg.first().copied().unwrap_or(0).to_ascii_uppercase();
            if let Some(cmd) = cmd_mut(&mut olc) {
                cmd.command = letter;
            }
            if letter == 0 || !b"MOPEDGRTV".contains(&letter) {
                write_to_desc(g, di, b"Invalid choice, try again : ");
            } else if olc.value != 0 {
                // If there was a previous command.
                if letter == b'T' || letter == b'V' {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.if_flag = 1;
                    }
                    if let Err(msg) = zedit_disp_arg1(g, di, &mut olc) {
                        zedit_oops(g, di, olc, msg);
                        return None;
                    }
                } else {
                    write_to_desc(
                        g,
                        di,
                        b"Is this command dependent on the success of the previous one? (y/n)\r\n",
                    );
                    olc.mode = ZEDIT_IF_FLAG;
                }
            } else {
                // 'if-flag' not appropriate.
                if let Some(cmd) = cmd_mut(&mut olc) {
                    cmd.if_flag = 0;
                }
                if let Err(msg) = zedit_disp_arg1(g, di, &mut olc) {
                    zedit_oops(g, di, olc, msg);
                    return None;
                }
            }
        }

        ZEDIT_IF_FLAG => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.if_flag = 1;
                    }
                }
                Some(b'n') | Some(b'N') => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.if_flag = 0;
                    }
                }
                _ => {
                    write_to_desc(g, di, b"Try again : ");
                    return Some(olc);
                }
            }
            if let Err(msg) = zedit_disp_arg1(g, di, &mut olc) {
                zedit_oops(g, di, olc, msg);
                return None;
            }
        }

        ZEDIT_ARG1 => {
            if !arg.first().is_some_and(|c| c.is_ascii_digit()) {
                write_to_desc(g, di, b"Must be a numeric value, try again : ");
                return Some(olc);
            }
            match cmd_letter(&olc) {
                b'M' => match g.world.real_mobile(atoi(arg) as Idx) {
                    Some(pos) => {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg1 = pos as i32;
                        }
                        if let Err(msg) = zedit_disp_arg2(g, di, &mut olc) {
                            zedit_oops(g, di, olc, msg);
                            return None;
                        }
                    }
                    None => write_to_desc(g, di, b"That mobile does not exist, try again : "),
                },
                b'O' | b'P' | b'E' | b'G' => match g.world.real_object(atoi(arg) as Idx) {
                    Some(pos) => {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg1 = pos as i32;
                        }
                        if let Err(msg) = zedit_disp_arg2(g, di, &mut olc) {
                            zedit_oops(g, di, olc, msg);
                            return None;
                        }
                    }
                    None => write_to_desc(g, di, b"That object does not exist, try again : "),
                },
                b'T' | b'V' => {
                    let v = atoi(arg);
                    if v < crate::dg::MOB_TRIGGER || v > crate::dg::WLD_TRIGGER {
                        write_to_desc(g, di, b"Invalid input.");
                    } else {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg1 = v;
                        }
                        if let Err(msg) = zedit_disp_arg2(g, di, &mut olc) {
                            zedit_oops(g, di, olc, msg);
                            return None;
                        }
                    }
                }
                _ => {
                    zedit_oops(g, di, olc, "SYSERR: OLC: zedit_parse(): case ARG1: Ack!");
                    return None;
                }
            }
        }

        ZEDIT_ARG2 => {
            if !arg.first().is_some_and(|c| c.is_ascii_digit()) {
                write_to_desc(g, di, b"Must be a numeric value, try again : ");
                return Some(olc);
            }
            let room = g.real_room(olc.number).map_or(NOWHERE as i32, |r| r as i32);
            match cmd_letter(&olc) {
                b'M' | b'O' => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.arg2 = MAX_DUPLICATES.min(atoi(arg));
                        cmd.arg3 = room;
                    }
                    zedit_disp_menu(g, di, &mut olc);
                }
                b'G' => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.arg2 = MAX_DUPLICATES.min(atoi(arg));
                    }
                    zedit_disp_menu(g, di, &mut olc);
                }
                b'P' | b'E' => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.arg2 = MAX_DUPLICATES.min(atoi(arg));
                    }
                    if let Err(msg) = zedit_disp_arg3(g, di, &mut olc) {
                        zedit_oops(g, di, olc, msg);
                        return None;
                    }
                }
                b'V' => {
                    if let Some(cmd) = cmd_mut(&mut olc) {
                        cmd.arg2 = atoi(arg); // context
                        cmd.arg3 = room;
                    }
                    write_to_desc(g, di, b"Enter the global name : ");
                    olc.mode = ZEDIT_SARG1;
                }
                b'T' => match g.world.real_trigger(atoi(arg) as Idx) {
                    Some(pos) => {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg2 = pos as i32;
                            cmd.arg3 = room;
                        }
                        zedit_disp_menu(g, di, &mut olc);
                    }
                    None => write_to_desc(g, di, b"That trigger does not exist, try again : "),
                },
                b'D' => {
                    let pos = atoi(arg);
                    // Guarding `pos > DIR_COUNT` here while the
                    // reset handler that consumes the command guards
                    // `>= DIR_COUNT` means
                    // the direction one past the last valid one
                    // is accepted here and refused there — the command is
                    // disabled at the next reset with `door does not exist in
                    // room N - dir D`, long after the builder left. With
                    // diagonals on it is worse: `arg2 == 10` runs off
                    // `dir_option[10]` and the menu prints `dirs[10]`, the "\n"
                    // sentinel, breaking the line in half. The bound here
                    // matches the reset handler's.
                    if pos < 0 || pos >= dir_count(g) {
                        write_to_desc(g, di, b"Try again : ");
                    } else {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg2 = pos;
                        }
                        if let Err(msg) = zedit_disp_arg3(g, di, &mut olc) {
                            zedit_oops(g, di, olc, msg);
                            return None;
                        }
                    }
                }
                b'R' => match g.world.real_object(atoi(arg) as Idx) {
                    Some(pos) => {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg2 = pos as i32;
                        }
                        zedit_disp_menu(g, di, &mut olc);
                    }
                    None => write_to_desc(g, di, b"That object does not exist, try again : "),
                },
                _ => {
                    zedit_oops(g, di, olc, "SYSERR: OLC: zedit_parse(): case ARG2: Ack!");
                    return None;
                }
            }
        }

        ZEDIT_ARG3 => {
            if !arg.first().is_some_and(|c| c.is_ascii_digit()) {
                write_to_desc(g, di, b"Must be a numeric value, try again : ");
                return Some(olc);
            }
            match cmd_letter(&olc) {
                b'E' => {
                    let pos = atoi(arg) - 1;
                    if pos < 0 || pos >= NUM_WEARS as i32 {
                        write_to_desc(g, di, b"Try again : ");
                    } else {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg3 = pos;
                        }
                        zedit_disp_menu(g, di, &mut olc);
                    }
                }
                b'P' => match g.world.real_object(atoi(arg) as Idx) {
                    Some(pos) => {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg3 = pos as i32;
                        }
                        zedit_disp_menu(g, di, &mut olc);
                    }
                    None => write_to_desc(g, di, b"That object does not exist, try again : "),
                },
                b'D' => {
                    let pos = atoi(arg);
                    if !(0..=2).contains(&pos) {
                        write_to_desc(g, di, b"Try again : ");
                    } else {
                        if let Some(cmd) = cmd_mut(&mut olc) {
                            cmd.arg3 = pos;
                        }
                        zedit_disp_menu(g, di, &mut olc);
                    }
                }
                _ => {
                    zedit_oops(g, di, olc, "SYSERR: OLC: zedit_parse(): case ARG3: Ack!");
                    return None;
                }
            }
        }

        ZEDIT_SARG1 => {
            if !arg.is_empty() {
                if let Some(cmd) = cmd_mut(&mut olc) {
                    cmd.sarg1 = Some(arg.to_vec());
                }
                olc.mode = ZEDIT_SARG2;
                write_to_desc(g, di, b"Enter the global value : ");
            } else {
                write_to_desc(g, di, b"Must have some name to assign : ");
            }
        }

        ZEDIT_SARG2 => {
            if !arg.is_empty() {
                if let Some(cmd) = cmd_mut(&mut olc) {
                    cmd.sarg2 = Some(arg.to_vec());
                }
                zedit_disp_menu(g, di, &mut olc);
            } else {
                write_to_desc(g, di, b"Must have some value to set it to :");
            }
        }

        ZEDIT_ZONE_NAME => {
            let mut text = arg.to_vec();
            if genolc_checkstring(&mut text) {
                if let Some(zone) = olc.zone.as_mut() {
                    if zone.name.is_none() {
                        g.log("SYSERR: OLC: ZEDIT_ZONE_NAME: no name to free!".to_string());
                    }
                    let zone = olc.zone.as_mut().unwrap();
                    zone.name = Some(text);
                    zone.number = 1;
                }
            }
            zedit_disp_menu(g, di, &mut olc);
        }

        ZEDIT_ZONE_BUILDERS => {
            let mut text = arg.to_vec();
            if genolc_checkstring(&mut text) {
                if let Some(zone) = olc.zone.as_mut() {
                    if zone.builders.is_none() {
                        g.log(
                            "SYSERR: OLC: ZEDIT_ZONE_BUILDERS: no builders list to free!"
                                .to_string(),
                        );
                    }
                    let zone = olc.zone.as_mut().unwrap();
                    zone.builders = Some(text);
                    zone.number = 1;
                }
            }
            zedit_disp_menu(g, di, &mut olc);
        }

        ZEDIT_ZONE_RESET => {
            let pos = atoi(arg);
            if !arg.first().is_some_and(|c| c.is_ascii_digit()) || !(0..=2).contains(&pos) {
                write_to_desc(g, di, b"Try again (0-2) : ");
            } else {
                if let Some(zone) = olc.zone.as_mut() {
                    zone.reset_mode = pos;
                    zone.number = 1;
                }
                zedit_disp_menu(g, di, &mut olc);
            }
        }

        ZEDIT_ZONE_LIFE => {
            let pos = atoi(arg);
            if !arg.first().is_some_and(|c| c.is_ascii_digit()) || !(0..=240).contains(&pos) {
                write_to_desc(g, di, b"Try again (0-240) : ");
            } else {
                if let Some(zone) = olc.zone.as_mut() {
                    zone.lifespan = pos;
                    zone.number = 1;
                }
                zedit_disp_menu(g, di, &mut olc);
            }
        }

        ZEDIT_ZONE_FLAGS => {
            let number = atoi(arg);
            if number < 0 || number > flags::NUM_ZONE_FLAGS as i32 {
                write_to_desc(g, di, b"That is not a valid choice!\r\n");
                zedit_disp_flag_menu(g, di, &mut olc);
            } else if number == 0 {
                zedit_disp_menu(g, di, &mut olc);
            } else {
                let bit = (number - 1) as usize;
                if let Some(zone) = olc.zone.as_mut() {
                    zone.zone_flags[bit / 32] ^= 1 << (bit % 32);
                    zone.number = 1;
                }
                zedit_disp_flag_menu(g, di, &mut olc);
            }
        }

        ZEDIT_ZONE_BOT => {
            let low = if olc.zone_num == 0 {
                0
            } else {
                g.world.zones[olc.zone_num as usize - 1].top as i32 + 1
            };
            if let Some(zone) = olc.zone.as_mut() {
                let high = zone.top as i32;
                zone.bot = limit(atoi(arg), low, high) as Idx;
                zone.number = 1;
            }
            zedit_disp_menu(g, di, &mut olc);
        }

        ZEDIT_ZONE_TOP => {
            let top_of_zone_table = g.world.zones.len().saturating_sub(1) as i32;
            let high = if olc.zone_num == top_of_zone_table {
                32000
            } else {
                g.world.zones[olc.zone_num as usize + 1].bot as i32 - 1
            };
            if let Some(zone) = olc.zone.as_mut() {
                let low = zone.bot as i32;
                zone.top = limit(atoi(arg), low, high) as Idx;
                zone.number = 1;
            }
            zedit_disp_menu(g, di, &mut olc);
        }

        _ => {
            zedit_oops(g, di, olc, "SYSERR: OLC: zedit_parse(): Reached default case!");
            return None;
        }
    }
    Some(olc)
}
