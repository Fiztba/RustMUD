//! perform_move/do_simple_move, doors, enter/leave,
//! positions, follow. Trigger hooks are stubbed until stage 6 (comments mark
//! each site).

use mud_data::flags::{self};
use mud_data::ids::{CharId, ObjId};
use mud_data::tables;
use mud_data::types::*;

use crate::act::informative::{look_at_room, search_block};
use crate::act::BStr;
use crate::comm::{self, act, send_to_char};
use crate::game::{Game, MudlogKind};
use crate::handler::{char_from_room, char_to_room, extract_char, fname, get_char_room_vis, isname};
use crate::interpreter::{one_argument, skip_spaces, two_arguments};

/// movement_loss per sector.
pub const MOVEMENT_LOSS: [i32; 10] = [1, 1, 2, 3, 4, 6, 4, 1, 1, 5];

pub const REV_DIR: [usize; 10] = [SOUTH, WEST, NORTH, EAST, DOWN, UP, SOUTHEAST, SOUTHWEST, NORTHWEST, NORTHEAST];

fn sect(g: &Game, room: RoomRnum) -> i32 {
    g.world.rooms[room as usize].sector_type
}

pub fn room_flagged(g: &Game, room: RoomRnum, bit: usize) -> bool {
    g.world.rooms[room as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0
}

pub fn zone_flagged(g: &Game, zone: ZoneRnum, bit: usize) -> bool {
    g.world.zones[zone as usize].zone_flags[bit / 32] & (1 << (bit % 32)) != 0
}

fn has_boat(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if ch.level > LVL_IMMORT {
        return true;
    }
    if ch.aff(flags::AFF_WATERWALK) || ch.aff(flags::AFF_FLYING) {
        return true;
    }
    for &oid in &ch.carrying {
        let o = g.obj(oid);
        if o.type_flag == flags::ITEM_BOAT && !o.wear_flags.is_set(flags::ITEM_WEAR_TAKE) {
            return true;
        }
    }
    for pos in 0..NUM_WEARS {
        if let Some(oid) = ch.equipment[pos] {
            if g.obj(oid).type_flag == flags::ITEM_BOAT {
                return true;
            }
        }
    }
    false
}

fn has_flight(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if ch.level > LVL_IMMORT {
        return true;
    }
    if ch.aff(flags::AFF_FLYING) {
        return true;
    }
    for &oid in &ch.carrying {
        if g.obj(oid).perm_affects.is_set(flags::AFF_FLYING) {
            return true;
        }
    }
    for pos in 0..NUM_WEARS {
        if let Some(oid) = ch.equipment[pos] {
            if g.obj(oid).perm_affects.is_set(flags::AFF_FLYING) {
                return true;
            }
        }
    }
    false
}

fn has_scuba(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if ch.level > LVL_IMMORT {
        return true;
    }
    if ch.aff(flags::AFF_SCUBA) {
        return true;
    }
    for &oid in &ch.carrying {
        if g.obj(oid).perm_affects.is_set(flags::AFF_SCUBA) {
            return true;
        }
    }
    for pos in 0..NUM_WEARS {
        if let Some(oid) = ch.equipment[pos] {
            if g.obj(oid).perm_affects.is_set(flags::AFF_SCUBA) {
                return true;
            }
        }
    }
    false
}

fn num_pc_in_room(g: &Game, room: RoomRnum) -> i32 {
    g.rooms[room as usize]
        .people
        .iter()
        .filter(|c| g.try_ch(**c).is_some_and(|ch| !ch.is_npc()))
        .count() as i32
}

pub fn do_simple_move(g: &mut Game, chid: CharId, dir: usize, need_specials_check: bool) -> bool {
    let was_in = g.ch(chid).in_room;
    let Some(exit) = g.world.rooms[was_in as usize].dir_option[dir].as_deref() else {
        return false;
    };
    let going_to = exit.to_room;
    if going_to == NOWHERE {
        return false;
    }

    // Spec procs may activate because of the move and prevent it — special
    // gets the "command" equivalent of the direction (dir+1; the command
    // table starts north at index 1). Only when following/mob-moving, which
    // avoids firing a spec proc twice.
    if need_specials_check && crate::spec::special(g, chid, dir + 1, b"") {
        return false;
    }

    // Leave Trigger Checks: a blocking return-0 or a script-side teleport
    // (room changed) aborts the move.
    if crate::dg::triggers::leave_mtrigger(g, chid, dir as i32) == 0 || g.ch(chid).in_room != was_in
    {
        return false;
    }
    if crate::dg::triggers::leave_wtrigger(g, was_in, chid, dir as i32) == 0
        || g.ch(chid).in_room != was_in
    {
        return false;
    }
    if crate::dg::triggers::leave_otrigger(g, was_in, chid, dir as i32) == 0
        || g.ch(chid).in_room != was_in
    {
        return false;
    }

    // Charm keep-with-master.
    let master = g.ch(chid).master;
    if g.ch(chid).aff(flags::AFF_CHARM)
        && master.is_some()
        && was_in == master.and_then(|m| g.try_ch(m)).map(|m| m.in_room).unwrap_or(NOWHERE)
    {
        send_to_char(g, chid, b"The thought of leaving your master makes you weep.\r\n");
        act(g, b"$n bursts into tears.", false, Some(chid), None, None, comm::TO_ROOM);
        return false;
    }

    // Water.
    if (sect(g, was_in) == flags::SECT_WATER_NOSWIM || sect(g, going_to) == flags::SECT_WATER_NOSWIM)
        && !has_boat(g, chid)
    {
        send_to_char(g, chid, b"You need a boat to go there.\r\n");
        return false;
    }
    // Flying.
    if (sect(g, was_in) == flags::SECT_FLYING || sect(g, going_to) == flags::SECT_FLYING) && !has_flight(g, chid) {
        send_to_char(g, chid, b"You need to be flying to go there!\r\n");
        return false;
    }
    // Underwater.
    let nohassle_imm = {
        let ch = g.ch(chid);
        !ch.is_npc() && ch.prf(flags::PRF_NOHASSLE)
    };
    if (sect(g, was_in) == flags::SECT_UNDERWATER || sect(g, going_to) == flags::SECT_UNDERWATER)
        && !has_scuba(g, chid)
        && !nohassle_imm
    {
        send_to_char(g, chid, b"You need to be able to breathe water to go there!\r\n");
        return false;
    }
    // Houses: can the player walk into the house?
    if room_flagged(g, was_in, flags::ROOM_ATRIUM) {
        let vnum = g.world.rooms[going_to as usize].vnum as i32;
        if !crate::house::house_can_enter(g, chid, vnum) {
            send_to_char(g, chid, b"That's private property -- no trespassing!\r\n");
            return false;
        }
    }

    let (level, is_npc) = {
        let ch = g.ch(chid);
        (ch.level, ch.is_npc())
    };
    let to_zone = g.world.rooms[going_to as usize].zone;
    if !is_npc && g.world.zones[to_zone as usize].min_level > level as i32 {
        send_to_char(g, chid, b"This zone is above your recommended level.\r\n");
    }
    if zone_flagged(g, to_zone, flags::ZONE_CLOSED)
        || (zone_flagged(g, to_zone, flags::ZONE_NOIMMORT) && level >= LVL_IMMORT && level < LVL_GRGOD)
    {
        send_to_char(g, chid, b"A mysterious barrier forces you back! That area is off-limits.\r\n");
        return false;
    }
    if room_flagged(g, going_to, flags::ROOM_TUNNEL) && num_pc_in_room(g, going_to) >= g.config.tunnel_size {
        if g.config.tunnel_size > 1 {
            send_to_char(g, chid, b"There isn't enough room for you to go there!\r\n");
        } else {
            send_to_char(g, chid, b"There isn't enough room there for more than one person!\r\n");
        }
        return false;
    }
    if room_flagged(g, going_to, flags::ROOM_GODROOM) && level < LVL_GOD {
        send_to_char(g, chid, b"You aren't godly enough to use that room!\r\n");
        return false;
    }

    // Movement points.
    let need_movement = (MOVEMENT_LOSS[sect(g, was_in).clamp(0, 9) as usize]
        + MOVEMENT_LOSS[sect(g, going_to).clamp(0, 9) as usize])
        / 2;
    if !is_npc && g.ch(chid).points.mov < need_movement {
        if need_specials_check && g.ch(chid).master.is_some() {
            send_to_char(g, chid, b"You are too exhausted to follow.\r\n");
        } else {
            send_to_char(g, chid, b"You are too exhausted.\r\n");
        }
        return false;
    }
    if !is_npc && level < LVL_IMMORT {
        g.ch_mut(chid).points.mov -= need_movement;
    }

    if !g.ch(chid).aff(flags::AFF_SNEAK) {
        let mut msg = b"$n leaves ".to_vec();
        msg.extend_from_slice(tables::DIRS[dir].as_bytes());
        msg.push(b'.');
        act(g, &msg, true, Some(chid), None, None, comm::TO_ROOM);
    }
    char_from_room(g, chid);
    char_to_room(g, chid, going_to);

    // Move them first, then move them back if they aren't allowed to go
    // (entry_mtrigger can teleport them away) —.
    if crate::dg::triggers::entry_mtrigger(g, chid) == 0
        || crate::dg::triggers::enter_wtrigger(g, going_to, chid, dir as i32) == 0
    {
        char_from_room(g, chid);
        char_to_room(g, chid, was_in);
        return false;
    }

    if !g.ch(chid).aff(flags::AFF_SNEAK) {
        act(g, b"$n has arrived.", true, Some(chid), None, None, comm::TO_ROOM);
    }
    if g.ch(chid).desc.is_some() {
        look_at_room(g, chid, false);
    }
    if room_flagged(g, going_to, flags::ROOM_DEATH) && level < LVL_IMMORT {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let vnum = g.world.rooms[going_to as usize].vnum;
        let rname = String::from_utf8_lossy(
            g.world.rooms[going_to as usize].name.as_deref().unwrap_or(b""),
        )
        .into_owned();
        g.mudlog(MudlogKind::Brf, LVL_IMMORT, true, &format!("{} hit death trap #{} ({})", name, vnum, rname));
        crate::fight::death_cry(g, chid);
        extract_char(g, chid);
        return false;
    }
    crate::dg::triggers::entry_memory_mtrigger(g, chid);
    if !crate::dg::triggers::greet_mtrigger(g, chid, dir as i32) {
        char_from_room(g, chid);
        char_to_room(g, chid, was_in);
        look_at_room(g, chid, false);
    } else {
        crate::dg::triggers::greet_memory_mtrigger(g, chid);
    }
    true
}

pub fn perform_move(g: &mut Game, chid: CharId, dir: i32, need_specials_check: bool) -> bool {
    if dir < 0 || dir as usize >= NUM_OF_DIRS || g.ch(chid).fighting.is_some() {
        return false;
    }
    let dir = dir as usize;
    let room = g.ch(chid).in_room;
    let diagonal = dir >= NORTHWEST;
    if diagonal && !g.config.diagonal_dirs {
        send_to_char(g, chid, b"Alas, you cannot go that way...\r\n");
        return false;
    }
    // (!EXIT && !buildwalk) || EXIT->to_room == NOWHERE — buildwalk digs
    // both the room and the exit, and the move then proceeds into it.
    if g.world.rooms[room as usize].dir_option[dir].is_none()
        && !crate::olc::copy::buildwalk(g, chid, dir)
    {
        send_to_char(g, chid, b"Alas, you cannot go that way...\r\n");
        return false;
    }
    let exit_ok = g.world.rooms[room as usize].dir_option[dir]
        .as_deref()
        .is_some_and(|e| e.to_room != NOWHERE);
    if !exit_ok {
        send_to_char(g, chid, b"Alas, you cannot go that way...\r\n");
        return false;
    }
    let exit_info = g.world.rooms[room as usize].dir_option[dir].as_deref().unwrap().exit_info;
    let nohassle_imm = {
        let ch = g.ch(chid);
        !ch.is_npc() && ch.prf(flags::PRF_NOHASSLE) && ch.level >= LVL_IMMORT
    };
    if exit_info & flags::EX_CLOSED != 0 && !nohassle_imm {
        let keyword = g.world.rooms[room as usize].dir_option[dir].as_deref().unwrap().keyword.clone();
        if let Some(kw) = keyword.filter(|k| !k.is_empty()) {
            let mut msg = b"The ".to_vec();
            msg.extend_from_slice(&fname(&kw));
            msg.extend_from_slice(b" seems to be closed.\r\n");
            send_to_char(g, chid, &msg);
        } else {
            send_to_char(g, chid, b"It seems to be closed.\r\n");
        }
        return false;
    }

    let followers = g.ch(chid).followers.clone();
    if followers.is_empty() {
        return do_simple_move(g, chid, dir, need_specials_check);
    }

    let was_in = g.ch(chid).in_room;
    if !do_simple_move(g, chid, dir, need_specials_check) {
        return false;
    }
    for f in followers {
        let Some(fc) = g.try_ch(f) else { continue };
        if fc.in_room == was_in && fc.position >= POS_STANDING {
            act(g, b"You follow $N.\r\n", false, Some(f), None, Some(chid), comm::TO_CHAR);
            perform_move(g, f, dir as i32, true);
        }
    }
    true
}

pub fn do_move(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, subcmd: i32) {
    perform_move(g, chid, subcmd, false);
}

// doors ----

const SCMD_OPEN: i32 = 0;
const SCMD_CLOSE: i32 = 1;
const SCMD_UNLOCK: i32 = 2;
const SCMD_LOCK: i32 = 3;
const SCMD_PICK: i32 = 4;

const CMD_DOOR: [&[u8]; 5] = [b"open", b"close", b"unlock", b"lock", b"pick"];

// flags_door: NEED_OPEN/NEED_CLOSED/NEED_UNLOCKED/NEED_LOCKED.
const NEED_OPEN: u8 = 1 << 0;
const NEED_CLOSED: u8 = 1 << 1;
const NEED_UNLOCKED: u8 = 1 << 2;
const NEED_LOCKED: u8 = 1 << 3;
const FLAGS_DOOR: [u8; 5] = [
    NEED_CLOSED | NEED_UNLOCKED, // open
    NEED_OPEN,                   // close
    NEED_CLOSED | NEED_LOCKED,   // unlock
    NEED_CLOSED | NEED_UNLOCKED, // lock
    NEED_CLOSED | NEED_LOCKED,   // pick
];

#[derive(Clone, Copy)]
enum DoorTarget {
    Exit { dir: usize },
    Obj { oid: ObjId },
}

fn exit_info(g: &Game, room: RoomRnum, dir: usize) -> u16 {
    g.world.rooms[room as usize].dir_option[dir].as_deref().map(|e| e.exit_info).unwrap_or(0)
}

fn door_key(g: &Game, room: RoomRnum, dir: usize) -> Idx {
    g.world.rooms[room as usize].dir_option[dir].as_deref().map(|e| e.key).unwrap_or(NOTHING)
}

fn door_keyword(g: &Game, room: RoomRnum, dir: usize) -> Option<BStr> {
    g.world.rooms[room as usize].dir_option[dir]
        .as_deref()
        .and_then(|e| e.keyword.clone())
        .filter(|k| !k.is_empty())
}

fn find_door(g: &mut Game, chid: CharId, type_: &[u8], dir_arg: &[u8], cmdname: &[u8]) -> Option<usize> {
    let room = g.ch(chid).in_room;
    if !dir_arg.is_empty() {
        let Some(door) = search_block(dir_arg, &tables::DIRS).or_else(|| search_block(dir_arg, &tables::AUTOEXITS))
        else {
            send_to_char(g, chid, b"That's not a direction.\r\n");
            return None;
        };
        if g.world.rooms[room as usize].dir_option[door].is_some() {
            if let Some(kw) = door_keyword(g, room, door) {
                if isname(type_, &kw) {
                    return Some(door);
                }
                let mut msg = b"I see no ".to_vec();
                msg.extend_from_slice(type_);
                msg.extend_from_slice(b" there.\r\n");
                send_to_char(g, chid, &msg);
                return None;
            }
            return Some(door);
        }
        let mut msg = b"I really don't see how you can ".to_vec();
        msg.extend_from_slice(cmdname);
        msg.extend_from_slice(b" anything there.\r\n");
        send_to_char(g, chid, &msg);
        return None;
    }
    if type_.is_empty() {
        let mut msg = b"What is it you want to ".to_vec();
        msg.extend_from_slice(cmdname);
        msg.extend_from_slice(b"?\r\n");
        send_to_char(g, chid, &msg);
        return None;
    }
    let autodoor = {
        let ch = g.ch(chid);
        !ch.is_npc() && ch.prf(flags::PRF_AUTODOOR)
    };
    let subcmd = CMD_DOOR.iter().position(|c| *c == cmdname).unwrap_or(0);
    for door in 0..crate::fight::dir_count(g) {
        if g.world.rooms[room as usize].dir_option[door].is_none() {
            continue;
        }
        let Some(kw) = door_keyword(g, room, door) else { continue };
        if !isname(type_, &kw) {
            continue;
        }
        if !autodoor {
            return Some(door);
        }
        // PRF_AUTODOOR: match doors in the appropriate state for the verb.
        let info = exit_info(g, room, door);
        let closed = info & flags::EX_CLOSED != 0;
        let locked = info & flags::EX_LOCKED != 0;
        let ok = match subcmd as i32 {
            SCMD_OPEN => closed,
            SCMD_CLOSE => !closed,
            SCMD_LOCK => !locked,
            SCMD_UNLOCK | SCMD_PICK => locked,
            _ => true,
        };
        if ok {
            return Some(door);
        }
    }
    let an = if type_.first().is_some_and(|c| b"aeiouAEIOU".contains(c)) { b"an" as &[u8] } else { b"a" };
    if !autodoor {
        let mut msg = b"There doesn't seem to be ".to_vec();
        msg.extend_from_slice(an);
        msg.push(b' ');
        msg.extend_from_slice(type_);
        msg.extend_from_slice(b" here.\r\n");
        send_to_char(g, chid, &msg);
    } else {
        const VERBED: [&[u8]; 5] = [b"opened", b"closed", b"unlocked", b"locked", b"picked"];
        let mut msg = b"There doesn't seem to be ".to_vec();
        msg.extend_from_slice(an);
        msg.push(b' ');
        msg.extend_from_slice(type_);
        msg.extend_from_slice(b" that can be ");
        msg.extend_from_slice(VERBED[subcmd]);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
    }
    None
}

fn has_key(g: &Game, chid: CharId, key: Idx) -> bool {
    if key == NOTHING {
        return false;
    }
    let ch = g.ch(chid);
    for &oid in &ch.carrying {
        if g.obj(oid).item_number != NOTHING {
            let vnum = g.world.obj_protos.get(g.obj(oid).item_number as usize).map(|p| p.vnum);
            if vnum == Some(key) {
                return true;
            }
        }
    }
    if let Some(oid) = ch.equipment[WEAR_HOLD] {
        let vnum = g.world.obj_protos.get(g.obj(oid).item_number as usize).map(|p| p.vnum);
        if vnum == Some(key) {
            return true;
        }
    }
    false
}

fn do_doorcmd(g: &mut Game, chid: CharId, target: DoorTarget, scmd: i32) {
    // Door triggers fire before the state change, for object doors too
    // (direction -1 renders as "none").
    let trig_dir = match target {
        DoorTarget::Exit { dir } => dir as i32,
        _ => -1,
    };
    if crate::dg::triggers::door_mtrigger(g, chid, scmd, trig_dir) == 0 {
        return;
    }
    if crate::dg::triggers::door_wtrigger(g, chid, scmd, trig_dir) == 0 {
        return;
    }
    let DoorTarget::Exit { dir } = target else {
        do_doorcmd_obj(g, chid, target, scmd);
        return;
    };
    let room = g.ch(chid).in_room;
    let keyword = door_keyword(g, room, dir);
    let back_room = g.world.rooms[room as usize].dir_option[dir].as_deref().map(|e| e.to_room).unwrap_or(NOWHERE);
    // The reverse exit, if it points back at us.
    let back_dir = REV_DIR[dir];
    let back_points_here = back_room != NOWHERE
        && g.world.rooms[back_room as usize].dir_option[back_dir]
            .as_deref()
            .is_some_and(|e| e.to_room == room);

    let apply = |exit_info: &mut u16| match scmd {
        SCMD_OPEN => *exit_info &= !flags::EX_CLOSED,
        SCMD_CLOSE => *exit_info |= flags::EX_CLOSED,
        SCMD_LOCK => *exit_info |= flags::EX_LOCKED,
        SCMD_UNLOCK => *exit_info &= !flags::EX_LOCKED,
        SCMD_PICK => *exit_info &= !flags::EX_LOCKED,
        _ => {}
    };
    if let Some(exit) = g.world.rooms[room as usize].dir_option[dir].as_deref_mut() {
        apply(&mut exit.exit_info);
    }
    if back_points_here {
        if let Some(back) = g.world.rooms[back_room as usize].dir_option[back_dir].as_deref_mut() {
            apply(&mut back.exit_info);
        }
    }

    // Feedback.
    match scmd {
        SCMD_OPEN | SCMD_CLOSE => {
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
        }
        SCMD_LOCK | SCMD_UNLOCK => send_to_char(g, chid, b"*Click*\r\n"),
        SCMD_PICK => send_to_char(g, chid, b"The lock quickly yields to your skills.\r\n"),
        _ => {}
    }
    // Room act.
    let doorname = keyword.as_deref().map(fname).unwrap_or_else(|| b"door".to_vec());
    let mut msg: BStr = Vec::new();
    if scmd == SCMD_PICK {
        msg.extend_from_slice(b"$n skillfully picks the lock on the ");
        msg.extend_from_slice(&doorname);
        msg.push(b'.');
    } else {
        msg.extend_from_slice(b"$n ");
        msg.extend_from_slice(match scmd {
            SCMD_OPEN => b"opens" as &[u8],
            SCMD_CLOSE => b"closes",
            SCMD_UNLOCK => b"unlocks",
            SCMD_LOCK => b"locks",
            _ => b"",
        });
        msg.extend_from_slice(b" the ");
        msg.extend_from_slice(&doorname);
        msg.push(b'.');
    }
    act(g, &msg, false, Some(chid), None, None, comm::TO_ROOM);

    // Other side notification for open/close.
    if (scmd == SCMD_OPEN || scmd == SCMD_CLOSE) && back_points_here {
        let back_kw = door_keyword(g, back_room, back_dir).map(|k| fname(&k)).unwrap_or_else(|| b"door".to_vec());
        let mut note = b"The ".to_vec();
        note.extend_from_slice(&back_kw);
        note.extend_from_slice(b" is ");
        note.extend_from_slice(if scmd == SCMD_OPEN { b"opened" as &[u8] } else { b"closed" });
        note.extend_from_slice(b" from the other side.\r\n");
        comm::send_to_room(g, back_room, &note);
    }
}

/// The container half of do_doorcmd: flips CONT_ bits in values[1]; the room
/// act is suppressed for carried containers (IN_ROOM(obj) == NOWHERE gate).
fn do_doorcmd_obj(g: &mut Game, chid: CharId, target: DoorTarget, scmd: i32) {
    let DoorTarget::Obj { oid } = target else { return };
    match scmd {
        SCMD_OPEN => {
            g.obj_mut(oid).values[1] &= !flags::CONT_CLOSED;
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
        }
        SCMD_CLOSE => {
            g.obj_mut(oid).values[1] |= flags::CONT_CLOSED;
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
        }
        SCMD_LOCK => {
            g.obj_mut(oid).values[1] |= flags::CONT_LOCKED;
            send_to_char(g, chid, b"*Click*\r\n");
        }
        SCMD_UNLOCK => {
            g.obj_mut(oid).values[1] &= !flags::CONT_LOCKED;
            send_to_char(g, chid, b"*Click*\r\n");
        }
        SCMD_PICK => {
            g.obj_mut(oid).values[1] ^= flags::CONT_LOCKED;
            send_to_char(g, chid, b"The lock quickly yields to your skills.\r\n");
        }
        _ => {}
    }
    // "$n <verb>s $p." / pick: "$n skillfully picks the lock on $p."
    let mut msg: BStr = Vec::new();
    if scmd == SCMD_PICK {
        msg.extend_from_slice(b"$n skillfully picks the lock on $p.");
    } else {
        msg.extend_from_slice(b"$n ");
        msg.extend_from_slice(CMD_DOOR[scmd.clamp(0, 4) as usize]);
        msg.extend_from_slice(b"s $p.");
    }
    if g.obj(oid).in_room != NOWHERE {
        act(g, &msg, false, Some(chid), Some(oid), None, comm::TO_ROOM);
    }
}

fn ok_pick(g: &mut Game, chid: CharId, keynum: Idx, pickproof: bool, scmd: i32) -> bool {
    if scmd != SCMD_PICK {
        return true;
    }
    let percent = g.rng.rand_number(1, 101);
    // GET_SKILL(SKILL_PICK_LOCK) + dex_app_skill p_locks.
    let (skill, dex) = {
        let ch = g.ch(chid);
        // SKILL_PICK_LOCK = 135.
        let skill = if ch.is_npc() { 0 } else { ch.ps().skills[135] as i32 };
        (skill, ch.aff_abils.dex.clamp(0, 25) as usize)
    };
    if keynum == NOTHING {
        send_to_char(g, chid, b"Odd - you can't seem to find a keyhole.\r\n");
        false
    } else if pickproof {
        send_to_char(g, chid, b"It resists your attempts to pick it.\r\n");
        false
    } else if percent > skill + tables::DEX_APP_SKILL[dex].1 {
        send_to_char(g, chid, b"You failed to pick the lock.\r\n");
        false
    } else {
        true
    }
}

pub fn do_gen_door(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    let argument = skip_spaces(argument);
    let cmdname = CMD_DOOR[subcmd.clamp(0, 4) as usize];
    if argument.is_empty() {
        let mut msg: BStr = Vec::new();
        msg.extend_from_slice(cmdname);
        if let Some(c) = msg.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        msg.extend_from_slice(b" what?\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let (type_, dir_arg, _) = two_arguments(argument);

    // Objects (containers) are tried first: generic_find over inventory +
    // room; a non-container match falls back to the door path.
    let (_, _, mut obj) = crate::handler::generic_find(
        g,
        chid,
        &type_,
        crate::handler::FIND_OBJ_INV | crate::handler::FIND_OBJ_ROOM,
    );
    let mut door: Option<usize> = None;
    if obj.is_none() {
        door = find_door(g, chid, &type_, &dir_arg, cmdname);
    } else if let Some(o) = obj {
        if g.obj(o).type_flag != flags::ITEM_CONTAINER {
            obj = None;
            door = find_door(g, chid, &type_, &dir_arg, cmdname);
        }
    }
    if obj.is_none() && door.is_none() {
        return;
    }

    // DOOR_* macro dispatch.
    let (openable, open, locked, pickproof, keynum) = match obj {
        Some(o) => {
            let ob = g.obj(o);
            let v1 = ob.values[1];
            let key = if ob.values[2] < 0 { NOTHING } else { ob.values[2] as Idx };
            (
                ob.type_flag == flags::ITEM_CONTAINER && v1 & flags::CONT_CLOSEABLE != 0,
                v1 & flags::CONT_CLOSED == 0,
                v1 & flags::CONT_LOCKED != 0,
                v1 & flags::CONT_PICKPROOF != 0,
                key,
            )
        }
        None => {
            let d = door.unwrap();
            let room = g.ch(chid).in_room;
            let info = exit_info(g, room, d);
            (
                info & flags::EX_ISDOOR != 0,
                info & flags::EX_CLOSED == 0,
                info & flags::EX_LOCKED != 0,
                info & flags::EX_PICKPROOF != 0,
                door_key(g, room, d),
            )
        }
    };
    let target = match obj {
        Some(o) => DoorTarget::Obj { oid: o },
        None => DoorTarget::Exit { dir: door.unwrap() },
    };
    let flags_needed = FLAGS_DOOR[subcmd.clamp(0, 4) as usize];
    let autokey = {
        let ch = g.ch(chid);
        !ch.is_npc() && ch.prf(flags::PRF_AUTOKEY)
    };
    // The message shows unless level >= IMMORT and (NPC or NOHASSLE).
    let locked_msg_applies = {
        let ch = g.ch(chid);
        ch.level < LVL_IMMORT || (!ch.is_npc() && !ch.prf(flags::PRF_NOHASSLE))
    };

    if !openable {
        let mut msg = b"You can't ".to_vec();
        msg.extend_from_slice(cmdname);
        msg.extend_from_slice(b" that!\r\n");
        send_to_char(g, chid, &msg);
    } else if !open && flags_needed & NEED_OPEN != 0 {
        send_to_char(g, chid, b"But it's already closed!\r\n");
    } else if open && flags_needed & NEED_CLOSED != 0 {
        send_to_char(g, chid, b"But it's currently open!\r\n");
    } else if !locked && flags_needed & NEED_LOCKED != 0 {
        send_to_char(g, chid, b"Oh.. it wasn't locked, after all..\r\n");
    } else if locked && flags_needed & NEED_UNLOCKED != 0 && autokey && has_key(g, chid, keynum) {
        send_to_char(g, chid, b"It is locked, but you have the key.\r\n");
        do_doorcmd(g, chid, target, SCMD_UNLOCK);
        do_doorcmd(g, chid, target, subcmd);
    } else if locked && flags_needed & NEED_UNLOCKED != 0 && autokey {
        send_to_char(g, chid, b"It is locked, and you do not have the key!\r\n");
    } else if locked && flags_needed & NEED_UNLOCKED != 0 && locked_msg_applies {
        send_to_char(g, chid, b"It seems to be locked.\r\n");
    } else if !has_key(g, chid, keynum)
        && g.ch(chid).level < LVL_GOD
        && (subcmd == SCMD_LOCK || subcmd == SCMD_UNLOCK)
    {
        send_to_char(g, chid, b"You don't seem to have the proper key.\r\n");
    } else if ok_pick(g, chid, keynum, pickproof, subcmd) {
        do_doorcmd(g, chid, target, subcmd);
    }
}

pub fn do_enter(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    let room = g.ch(chid).in_room;
    if !buf.is_empty() {
        // Enter a door keyword (exact match).
        for door in 0..crate::fight::dir_count(g) {
            if let Some(kw) = door_keyword(g, room, door) {
                if kw.eq_ignore_ascii_case(&buf) {
                    perform_move(g, chid, door as i32, true);
                    return;
                }
            }
        }
        let mut msg = b"There is no ".to_vec();
        msg.extend_from_slice(&buf);
        msg.extend_from_slice(b" here.\r\n");
        send_to_char(g, chid, &msg);
    } else if room_flagged(g, room, flags::ROOM_INDOORS) {
        send_to_char(g, chid, b"You are already indoors.\r\n");
    } else {
        for door in 0..crate::fight::dir_count(g) {
            if let Some(exit) = g.world.rooms[room as usize].dir_option[door].as_deref() {
                if exit.to_room != NOWHERE
                    && exit.exit_info & flags::EX_CLOSED == 0
                    && room_flagged(g, exit.to_room, flags::ROOM_INDOORS)
                {
                    perform_move(g, chid, door as i32, true);
                    return;
                }
            }
        }
        send_to_char(g, chid, b"You can't seem to find anything to enter.\r\n");
    }
}

pub fn do_leave(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    let room = g.ch(chid).in_room;
    if !room_flagged(g, room, flags::ROOM_INDOORS) {
        send_to_char(g, chid, b"You are outside.. where do you want to go?\r\n");
        return;
    }
    for door in 0..crate::fight::dir_count(g) {
        if let Some(exit) = g.world.rooms[room as usize].dir_option[door].as_deref() {
            if exit.to_room != NOWHERE
                && exit.exit_info & flags::EX_CLOSED == 0
                && !room_flagged(g, exit.to_room, flags::ROOM_INDOORS)
            {
                perform_move(g, chid, door as i32, true);
                return;
            }
        }
    }
    send_to_char(g, chid, b"I see no obvious exits to the outside.\r\n");
}

// positions ----

pub fn do_stand(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    match g.ch(chid).position {
        POS_STANDING => send_to_char(g, chid, b"You are already standing.\r\n"),
        POS_SITTING => {
            send_to_char(g, chid, b"You stand up.\r\n");
            act(g, b"$n clambers to $s feet.", true, Some(chid), None, None, comm::TO_ROOM);
            // Furniture release is stage 3.
            let pos = if g.ch(chid).fighting.is_some() { POS_FIGHTING } else { POS_STANDING };
            g.ch_mut(chid).position = pos;
        }
        POS_RESTING => {
            send_to_char(g, chid, b"You stop resting, and stand up.\r\n");
            act(g, b"$n stops resting, and clambers on $s feet.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_STANDING;
        }
        POS_SLEEPING => send_to_char(g, chid, b"You have to wake up first!\r\n"),
        POS_FIGHTING => send_to_char(g, chid, b"Do you not consider fighting as standing?\r\n"),
        _ => {
            send_to_char(g, chid, b"You stop floating around, and put your feet on the ground.\r\n");
            act(
                g,
                b"$n stops floating around, and puts $s feet on the ground.",
                true,
                Some(chid),
                None,
                None,
                comm::TO_ROOM,
            );
            g.ch_mut(chid).position = POS_STANDING;
        }
    }
}

pub fn do_sit(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    // Furniture targets are stage 3; bare sit only.
    match g.ch(chid).position {
        POS_STANDING => {
            send_to_char(g, chid, b"You sit down.\r\n");
            act(g, b"$n sits down.", false, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SITTING;
        }
        POS_SITTING => send_to_char(g, chid, b"You're sitting already.\r\n"),
        POS_RESTING => {
            send_to_char(g, chid, b"You stop resting, and sit up.\r\n");
            act(g, b"$n stops resting.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SITTING;
        }
        POS_SLEEPING => send_to_char(g, chid, b"You have to wake up first.\r\n"),
        POS_FIGHTING => send_to_char(g, chid, b"Sit down while fighting? Are you MAD?\r\n"),
        _ => {
            send_to_char(g, chid, b"You stop floating around, and sit down.\r\n");
            act(g, b"$n stops floating around, and sits down.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SITTING;
        }
    }
}

pub fn do_rest(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    match g.ch(chid).position {
        POS_STANDING => {
            send_to_char(g, chid, b"You sit down and rest your tired bones.\r\n");
            act(g, b"$n sits down and rests.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_RESTING;
        }
        POS_SITTING => {
            send_to_char(g, chid, b"You rest your tired bones.\r\n");
            act(g, b"$n rests.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_RESTING;
        }
        POS_RESTING => send_to_char(g, chid, b"You are already resting.\r\n"),
        POS_SLEEPING => send_to_char(g, chid, b"You have to wake up first.\r\n"),
        POS_FIGHTING => send_to_char(g, chid, b"Rest while fighting?  Are you MAD?\r\n"),
        _ => {
            send_to_char(g, chid, b"You stop floating around, and stop to rest your tired bones.\r\n");
            act(g, b"$n stops floating around, and rests.", false, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SITTING;
        }
    }
}

pub fn do_sleep(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    match g.ch(chid).position {
        POS_STANDING | POS_SITTING | POS_RESTING => {
            send_to_char(g, chid, b"You go to sleep.\r\n");
            act(g, b"$n lies down and falls asleep.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SLEEPING;
        }
        POS_SLEEPING => send_to_char(g, chid, b"You are already sound asleep.\r\n"),
        POS_FIGHTING => send_to_char(g, chid, b"Sleep while fighting?  Are you MAD?\r\n"),
        _ => {
            send_to_char(g, chid, b"You stop floating around, and lie down to sleep.\r\n");
            act(g, b"$n stops floating around, and lie down to sleep.", true, Some(chid), None, None, comm::TO_ROOM);
            g.ch_mut(chid).position = POS_SLEEPING;
        }
    }
}

pub fn do_wake(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    if !arg.is_empty() {
        if g.ch(chid).position == POS_SLEEPING {
            send_to_char(g, chid, b"Maybe you should wake yourself up first.\r\n");
            return;
        }
        let Some(vict) = get_char_room_vis(g, chid, &arg, None) else {
            let msg = g.config.noperson.clone();
            send_to_char(g, chid, &msg);
            return;
        };
        if vict == chid {
            do_wake_self(g, chid);
            return;
        }
        if g.ch(vict).awake() {
            act(g, b"$E is already awake.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        } else if g.ch(vict).aff(flags::AFF_SLEEP) {
            act(g, b"You can't wake $M up!", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        } else if g.ch(vict).position < POS_SLEEPING {
            act(g, b"$E's in pretty bad shape!", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        } else {
            act(g, b"You wake $M up.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
            act(g, b"You are awakened by $n.", false, Some(chid), None, Some(vict), comm::TO_VICT | comm::TO_SLEEP);
            g.ch_mut(vict).position = POS_SITTING;
        }
        return;
    }
    do_wake_self(g, chid);
}

fn do_wake_self(g: &mut Game, chid: CharId) {
    if g.ch(chid).aff(flags::AFF_SLEEP) {
        send_to_char(g, chid, b"You can't wake up!\r\n");
    } else if g.ch(chid).position > POS_SLEEPING {
        send_to_char(g, chid, b"You are already awake...\r\n");
    } else {
        send_to_char(g, chid, b"You awaken, and sit up.\r\n");
        act(g, b"$n awakens.", true, Some(chid), None, None, comm::TO_ROOM);
        g.ch_mut(chid).position = POS_SITTING;
    }
}

// follow ----

pub fn circle_follow(g: &Game, chid: CharId, victim: CharId) -> bool {
    let mut k = Some(victim);
    while let Some(cur) = k {
        if cur == chid {
            return true;
        }
        k = g.try_ch(cur).and_then(|c| c.master);
    }
    false
}

pub fn stop_follower(g: &mut Game, chid: CharId) {
    let Some(master) = g.ch(chid).master else { return };
    if g.ch(chid).aff(flags::AFF_CHARM) {
        act(g, b"You realize that $N is a jerk!", false, Some(chid), None, Some(master), comm::TO_CHAR);
        act(g, b"$n realizes that $N is a jerk!", false, Some(chid), None, Some(master), comm::TO_NOTVICT);
        act(g, b"$n hates your guts!", false, Some(chid), None, Some(master), comm::TO_VICT);
        // SPELL_CHARM affect strip (spell 16) — full spell system is stage 5.
        let idx = g.ch(chid).affected.iter().position(|a| a.spell == 16);
        if let Some(idx) = idx {
            crate::handler::affect_remove(g, chid, idx);
        }
    } else {
        act(g, b"You stop following $N.", false, Some(chid), None, Some(master), comm::TO_CHAR);
        act(g, b"$n stops following $N.", true, Some(chid), None, Some(master), comm::TO_NOTVICT);
        if crate::handler::can_see(g, master, chid) {
            act(g, b"$n stops following you.", true, Some(chid), None, Some(master), comm::TO_VICT);
        }
    }
    if let Some(mc) = g.chars.get_mut(master) {
        mc.followers.retain(|f| *f != chid);
    }
    let ch = g.ch_mut(chid);
    ch.master = None;
    ch.affected_by.remove(flags::AFF_CHARM);
}

pub fn add_follower(g: &mut Game, chid: CharId, leader: CharId) {
    if g.ch(chid).master.is_some() {
        g.log("SYSERR: add_follower: follower already has a master.".to_string());
        return;
    }
    g.ch_mut(chid).master = Some(leader);
    g.ch_mut(leader).followers.insert(0, chid);
    act(g, b"You now follow $N.", false, Some(chid), None, Some(leader), comm::TO_CHAR);
    if crate::handler::can_see(g, leader, chid) {
        act(g, b"$n starts following you.", true, Some(chid), None, Some(leader), comm::TO_VICT);
    }
    act(g, b"$n starts to follow $N.", true, Some(chid), None, Some(leader), comm::TO_NOTVICT);
}

pub fn do_follow(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    if buf.is_empty() {
        if let Some(master) = g.ch(chid).master {
            let mut msg = b"You are following ".to_vec();
            msg.extend_from_slice(&crate::handler::pers(g, chid, master));
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
        } else {
            send_to_char(g, chid, b"Whom do you wish to follow?\r\n");
        }
        return;
    }
    let Some(leader) = get_char_room_vis(g, chid, &buf, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if g.ch(chid).master == Some(leader) {
        act(g, b"You are already following $M.", false, Some(chid), None, Some(leader), comm::TO_CHAR);
        return;
    }
    if g.ch(chid).aff(flags::AFF_CHARM) && g.ch(chid).master.is_some() {
        let master = g.ch(chid).master;
        act(g, b"But you only feel like following $N!", false, Some(chid), None, master, comm::TO_CHAR);
        return;
    }
    if leader == chid {
        if g.ch(chid).master.is_none() {
            send_to_char(g, chid, b"You are already following yourself.\r\n");
            return;
        }
        stop_follower(g, chid);
        return;
    }
    if circle_follow(g, chid, leader) {
        send_to_char(g, chid, b"Sorry, but following in loops is not allowed.\r\n");
        return;
    }
    if g.ch(chid).master.is_some() {
        stop_follower(g, chid);
    }
    g.ch_mut(chid).affected_by.remove(flags::AFF_GROUP);
    add_follower(g, chid, leader);
}

pub fn do_unfollow(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).master.is_some() {
        if g.ch(chid).aff(flags::AFF_CHARM) {
            let master = g.ch(chid).master.unwrap();
            let mut msg = b"You feel compelled to follow ".to_vec();
            msg.extend_from_slice(&crate::handler::pers(g, chid, master));
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
        } else {
            stop_follower(g, chid);
        }
    } else {
        send_to_char(g, chid, b"You are not following anyone.\r\n");
    }
}
