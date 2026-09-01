//! `dig`, buildwalk, and the `[rmost]copy` commands.
//!
//! `dig` and `buildwalk` both borrow an OLC structure for a moment so that
//! `redit_save_internally` can do the insertion, then tear it down with
//! `cleanup_olc`. Left alone, both would emit "$n stops using OLC." and an
//! "OLC: <name> stops editing zone N" mudlog without ever having announced
//! a start. That is **B35**: the announcement belongs to
//! descriptors that really entered an editor, and both commands log the
//! room they actually created.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::DIRS;
use mud_data::types::*;
use mud_world::model::{Exit, Room};

use crate::act::informative::search_block;
use crate::act::movement::REV_DIR;
use crate::act::BStr;
use crate::comm::send_to_char;
use crate::db::{add_to_save_list, SL_WLD};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::interpreter::{is_number, skip_spaces, two_arguments};
use crate::olc::redit::{redit_save_internally, redit_setup_existing};
use crate::olc::{
    can_edit_zone, cleanup_olc, get_char_colors, send_cannot_edit, OlcData, CLEANUP_ALL,
    CLEANUP_STRUCTS,
};

/// do_oasis_copy. `subcmd` is the CON_* state of the
/// editor that owns the data type.
pub fn do_oasis_copy(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    // oasis_copy_info[]: the type table, keyed by connection state.
    let (command, text) = match subcmd {
        s if s == ConState::Redit as i32 => (&b"rcopy"[..], &b"room"[..]),
        s if s == ConState::Oedit as i32 => (&b"ocopy"[..], &b"object"[..]),
        s if s == ConState::Medit as i32 => (&b"mcopy"[..], &b"mobile"[..]),
        s if s == ConState::Sedit as i32 => (&b"scopy"[..], &b"shop"[..]),
        s if s == ConState::Trigedit as i32 => (&b"tcopy"[..], &b"trigger"[..]),
        // "If not found, we don't support copying that type of data."
        _ => return,
    };

    // No copying as a mob or while being forced.
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    if buf2.is_empty() || !is_number(&buf1) || !is_number(&buf2) {
        let mut msg = b"Syntax: ".to_vec();
        msg.extend_from_slice(command);
        msg.extend_from_slice(b" <source vnum> <target vnum>\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    let lookup = |g: &Game, vnum: i32| -> Option<u16> {
        if !(0..=65535).contains(&vnum) {
            return None;
        }
        let v = vnum as u16;
        match subcmd {
            s if s == ConState::Redit as i32 => g.world.real_room(v),
            s if s == ConState::Oedit as i32 => g.world.real_object(v),
            s if s == ConState::Medit as i32 => g.world.real_mobile(v),
            s if s == ConState::Sedit as i32 => {
                g.world.shops.iter().position(|sh| sh.vnum == v).map(|i| i as u16)
            }
            _ => g.world.real_trigger(v),
        }
    };

    let src_vnum = atoi(&buf1);
    let Some(src_rnum) = lookup(g, src_vnum) else {
        let mut msg = b"The source ".to_vec();
        msg.extend_from_slice(text);
        msg.extend_from_slice(format!(" (#{}) does not exist.\r\n", src_vnum).as_bytes());
        send_to_char(g, chid, &msg);
        return;
    };

    let dst_vnum = atoi(&buf2);
    if lookup(g, dst_vnum).is_some() {
        let mut msg = b"The target ".to_vec();
        msg.extend_from_slice(text);
        msg.extend_from_slice(format!(" (#{}) already exists.\r\n", dst_vnum).as_bytes());
        send_to_char(g, chid, &msg);
        return;
    }

    // Check that whatever it is isn't already being edited.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state as i32) != Some(subcmd) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(dst_vnum) {
            continue;
        }
        let who = g
            .descriptors
            .get(other)
            .and_then(|d| d.character)
            .map(|c| g.ch(c).get_name().to_vec())
            .unwrap_or_else(|| b"(null)".to_vec());
        let mut msg = b"The target ".to_vec();
        msg.extend_from_slice(text);
        msg.extend_from_slice(format!(" (#{}) is currently being edited by ", dst_vnum).as_bytes());
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
            "SYSERR: do_oasis_copy: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }
    let mut olc = OlcData::new();

    let Some(znum) = crate::dg::mobcmd::real_zone_by_thing(g, dst_vnum) else {
        send_to_char(
            g,
            chid,
            format!("Sorry, there is no zone for the given vnum (#{})!\r\n", dst_vnum).as_bytes(),
        );
        return;
    };
    olc.zone_num = znum as i32;

    if !can_edit_zone(g, chid, znum as i32) {
        let zvnum = g.world.zones[znum].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }

    olc.number = dst_vnum;

    let mut msg = b"Copying ".to_vec();
    msg.extend_from_slice(text);
    msg.extend_from_slice(format!(": source: #{}, dest: #{}.\r\n", src_vnum, dst_vnum).as_bytes());
    send_to_char(g, chid, &msg);

    match subcmd {
        s if s == ConState::Redit as i32 => {
            redit_setup_existing(g, &mut olc, src_rnum as usize);
            redit_save_internally(g, di, &mut olc);
        }
        s if s == ConState::Medit as i32 => {
            crate::olc::medit::medit_setup_existing(g, &mut olc, src_rnum as usize);
            crate::olc::medit::medit_save_internally(g, di, &mut olc);
        }
        s if s == ConState::Oedit as i32 => {
            crate::olc::oedit::oedit_setup_existing(g, &mut olc, src_rnum as usize);
            crate::olc::oedit::oedit_save_internally(g, di, &mut olc);
        }
        _ => {
            // The remaining editors land with their own modules.
            return;
        }
    }

    cleanup_olc(g, di, olc, CLEANUP_ALL);
    send_to_char(g, chid, b"Done.\r\n");
}

pub fn do_dig(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let (sdir, sroom, rest) = two_arguments(argument);
    let new_room_name = skip_spaces(rest).to_vec();

    if sdir.is_empty() || sroom.is_empty() {
        send_to_char(
            g,
            chid,
            b"Format: dig <direction> <room> - to create an exit\r\n        dig <direction> -1     - to delete an exit\r\n",
        );
        return;
    }

    let rawvnum = atoi(&sroom);
    // (room_vnum)rawvnum: the cast truncates, and -1 becomes NOWHERE.
    let rvnum: u16 = if rawvnum == -1 { NOWHERE } else { rawvnum as u16 };
    let mut rrnum = g.world.real_room(rvnum).unwrap_or(NOWHERE);
    let dir = search_block(&sdir, &DIRS);
    let room = g.ch(chid).in_room;
    let zone = g.world.rooms[room as usize].zone;

    let Some(dir) = dir else {
        let mut msg = b"You cannot create an exit to the '".to_vec();
        msg.extend_from_slice(&sdir);
        msg.extend_from_slice(b"'.\r\n");
        send_to_char(g, chid, &msg);
        return;
    };

    if zone == NOWHERE || !can_edit_zone(g, chid, zone as i32) {
        send_to_char(g, chid, b"You do not have permission to edit this zone.\r\n");
        return;
    }
    // Lets not allow digging to limbo.
    if rvnum == 0 {
        send_to_char(g, chid, b"The target exists, but you can't dig to limbo!\r\n");
        return;
    }

    // Target room == -1 removes the exit.
    if rvnum == NOTHING {
        if g.world.rooms[room as usize].dir_option[dir].is_some() {
            g.world.rooms[room as usize].dir_option[dir] = None;
            let zvnum = g.world.zones[g.world.rooms[room as usize].zone as usize].number;
            add_to_save_list(g, zvnum, SL_WLD);
            let mut msg = b"You remove the exit to the ".to_vec();
            msg.extend_from_slice(DIRS[dir].as_bytes());
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
            return;
        }
        let mut msg = b"There is no exit to the ".to_vec();
        msg.extend_from_slice(DIRS[dir].as_bytes());
        msg.extend_from_slice(b".\r\nNo exit removed.\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    // Can't dig in a direction, if it's already a door.
    if g.world.rooms[room as usize].dir_option[dir].is_some() {
        let mut msg = b"There already is an exit to the ".to_vec();
        msg.extend_from_slice(DIRS[dir].as_bytes());
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    // Make sure that the builder has access to the zone he's linking to.
    let Some(zone) = crate::dg::mobcmd::real_zone_by_thing(g, rvnum as i32) else {
        send_to_char(g, chid, b"You cannot link to a non-existing zone!\r\n");
        return;
    };
    if !can_edit_zone(g, chid, zone as i32) {
        send_to_char(
            g,
            chid,
            format!("You do not have permission to edit room #{}.\r\n", rvnum).as_bytes(),
        );
        return;
    }

    // If the room doesn't exist, create it.
    if rrnum == NOWHERE {
        if g.olc.contains_key(&di) {
            g.mudlog(
                MudlogKind::Brf,
                LVL_IMMORT,
                true,
                "SYSERR: do_dig: Player already had olc structure.",
            );
            g.olc.remove(&di);
        }
        let mut olc = OlcData::new();
        olc.zone_num = zone as i32;
        olc.number = rvnum as i32;
        let mut new_room = Room::default();
        new_room.name = Some(if !new_room_name.is_empty() {
            new_room_name.clone()
        } else {
            b"An unfinished room".to_vec()
        });
        new_room.description = Some(b"You are in an unfinished room.\r\n".to_vec());
        new_room.zone = zone as ZoneRnum;
        new_room.vnum = NOWHERE;
        olc.room = Some(Box::new(new_room));

        redit_save_internally(g, di, &mut olc);
        olc.value = 0;

        send_to_char(g, chid, format!("New room ({}) created.\r\n", rvnum).as_bytes());
        // dig leaves no other trace, so log the creation here.
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let here = g.world.rooms[g.ch(chid).in_room as usize].vnum;
        let msg = format!("OLC: {} digs room {} {} of room {}", name, rvnum, DIRS[dir], here);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        cleanup_olc(g, di, olc, CLEANUP_ALL);
        rrnum = g.world.real_room(rvnum).unwrap_or(NOWHERE);
    }

    // Now dig. The exit's key is left at 0, not NOTHING.
    let room = g.ch(chid).in_room;
    g.world.rooms[room as usize].dir_option[dir] = Some(Box::new(Exit {
        general_description: None,
        keyword: None,
        exit_info: 0,
        key: 0,
        to_room_vnum: 0,
        to_room: rrnum,
    }));
    let zvnum = g.world.zones[g.world.rooms[room as usize].zone as usize].number;
    add_to_save_list(g, zvnum, SL_WLD);

    let target_name = g.world.rooms[rrnum as usize].name.clone().unwrap_or_default();
    let mut msg = b"You make an exit ".to_vec();
    msg.extend_from_slice(DIRS[dir].as_bytes());
    msg.extend_from_slice(format!(" to room {} (", rvnum).as_bytes());
    msg.extend_from_slice(&target_name);
    msg.extend_from_slice(b").\r\n");
    send_to_char(g, chid, &msg);

    // Check if we can dig from there to here.
    let back = REV_DIR[dir];
    if g.world.rooms[rrnum as usize].dir_option[back].is_some() {
        let mut msg = format!("You cannot dig from {} to here. The target room already has an exit to the ", rvnum).into_bytes();
        msg.extend_from_slice(DIRS[back].as_bytes());
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
    } else {
        g.world.rooms[rrnum as usize].dir_option[back] = Some(Box::new(Exit {
            general_description: None,
            keyword: None,
            exit_info: 0,
            key: 0,
            to_room_vnum: 0,
            to_room: room,
        }));
        let zvnum = g.world.zones[g.world.rooms[rrnum as usize].zone as usize].number;
        add_to_save_list(g, zvnum, SL_WLD);
    }
}

/// redit_find_new_vnum: the next free vnum in a zone.
fn redit_find_new_vnum(g: &Game, zone: usize) -> u16 {
    let mut vnum = g.world.zones[zone].bot;
    let Some(mut rnum) = g.world.real_room(vnum) else { return vnum };
    loop {
        if vnum > g.world.zones[zone].top {
            return NOWHERE;
        }
        if rnum as usize >= g.world.rooms.len() || g.world.rooms[rnum as usize].vnum > vnum {
            break;
        }
        rnum += 1;
        vnum += 1;
    }
    vnum
}

/// buildwalk: create and link a room in the walked
/// direction. Returns true when a room was made.
pub fn buildwalk(g: &mut Game, chid: CharId, dir: usize) -> bool {
    let ch = g.ch(chid);
    if ch.is_npc() || !ch.prf(flags::PRF_BUILDWALK) || ch.level < LVL_BUILDER {
        return false;
    }
    get_char_colors(g, chid);

    let room = g.ch(chid).in_room;
    let zone = g.world.rooms[room as usize].zone;
    if !can_edit_zone(g, chid, zone as i32) {
        send_to_char(g, chid, b"You do not have build permissions in this zone.\r\n");
        return false;
    }
    let vnum = redit_find_new_vnum(g, zone as usize);
    if vnum == NOWHERE {
        send_to_char(g, chid, b"No free vnums are available in this zone!\r\n");
        return false;
    }

    let Some(di) = g.ch(chid).desc else { return false };
    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: buildwalk(): Player already had olc structure.",
        );
        g.olc.remove(&di);
    }
    let mut olc = OlcData::new();
    olc.zone_num = zone as i32;
    olc.number = vnum as i32;

    let name = g.ch(chid).get_name().to_vec();
    let mut desc: BStr = b"This unfinished room was created by ".to_vec();
    desc.extend_from_slice(&name);
    desc.extend_from_slice(b".\r\n");

    let mut new_room = Room::default();
    new_room.name = Some(b"New BuildWalk Room".to_vec());
    new_room.description = Some(desc);
    new_room.zone = zone;
    new_room.vnum = NOWHERE;
    new_room.sector_type = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.buildwalk_sector);
    olc.room = Some(Box::new(new_room));

    redit_save_internally(g, di, &mut olc);
    olc.value = 0;

    // Link rooms.
    let rnum = g.world.real_room(vnum).unwrap_or(NOWHERE);
    let room = g.ch(chid).in_room;
    g.world.rooms[room as usize].dir_option[dir] = Some(Box::new(Exit {
        general_description: None,
        keyword: None,
        exit_info: 0,
        key: 0,
        to_room_vnum: 0,
        to_room: rnum,
    }));
    g.world.rooms[rnum as usize].dir_option[REV_DIR[dir]] = Some(Box::new(Exit {
        general_description: None,
        keyword: None,
        exit_info: 0,
        key: 0,
        to_room_vnum: 0,
        to_room: room,
    }));

    let c = g.olc_colors;
    let mut msg: BStr = c.yel().to_vec();
    msg.extend_from_slice(format!("Room #{} created by BuildWalk.", vnum).as_bytes());
    msg.extend_from_slice(c.nrm());
    msg.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &msg);
    // Buildwalk had no log line at all — a room could appear in a zone
    // with nothing in the OLC log to say who made it.
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
    let logmsg = format!("OLC: {} creates room {} with buildwalk", name, vnum);
    g.mudlog(MudlogKind::Cmp, level, true, &logmsg);
    cleanup_olc(g, di, olc, CLEANUP_STRUCTS);
    true
}
