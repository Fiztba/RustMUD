//! The w-commands + wld_command_interpreter. Room messages go
//! through act (act_to_room), so $-codes expand and Act triggers can fire.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use super::comm::{send_to_range, send_to_zone, sub_write, TO_CHAR_T, TO_ROOM_T};
use super::misc::{script_damage, valid_dg_target};
use super::mobcmd::real_zone_by_thing;
use super::objcmd::door_command;
use super::triggers::{enter_wtrigger, load_mtrigger, load_otrigger};
use super::variables::{can_wear_on_pos, find_eq_pos_script};
use super::{
    atoi32, char_script_id, get_char, get_char_by_room, get_char_in_room, get_obj,
    get_obj_by_room, get_obj_in_room, obj_script_id, wld_log, GoId, DG_ALLOW_GODS, UID_CHAR,
};
use crate::comm::{act, TO_CHAR, TO_ROOM};
use crate::game::Game;
use crate::handler::{
    char_from_room, char_to_room, equip_char, extract_char, extract_obj, obj_from_room,
    obj_to_char, obj_to_obj, obj_to_room, eq_ci,
};

pub type BStr = Vec<u8>;

const SCMD_WSEND: i32 = 0;
const SCMD_WECHOAROUND: i32 = 1;

/// act_to_room: act TO_ROOM + TO_CHAR anchored at the
/// first person in the room; nothing if empty.
fn act_to_room(g: &mut Game, s: &[u8], room: RoomRnum) {
    let Some(&first) = g.rooms[room as usize].people.first() else { return };
    act(g, s, false, Some(first), None, None, TO_ROOM);
    act(g, s, false, Some(first), None, None, TO_CHAR);
}

fn do_wasound(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        wld_log(g, room, "wasound called with no argument");
        return;
    }
    let dir_count = crate::fight::dir_count(g) as usize;
    for door in 0..dir_count {
        let to = g.world.rooms[room as usize].dir_option[door]
            .as_ref()
            .map(|e| e.to_room)
            .filter(|&t| t != NOWHERE && t != room);
        if let Some(to_room) = to {
            act_to_room(g, &argument, to_room);
        }
    }
}

fn do_wecho(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        wld_log(g, room, "wecho called with no args");
    } else {
        act_to_room(g, &argument, room);
    }
}

fn do_wlog(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument);
    if !argument.is_empty() {
        let msg = String::from_utf8_lossy(argument).into_owned();
        wld_log(g, room, &msg);
    }
}

fn do_wsend(g: &mut Game, room: RoomRnum, argument: &[u8], subcmd: i32) {
    let (buf, rest) = crate::interpreter::any_one_arg(argument);
    if buf.is_empty() {
        wld_log(g, room, "wsend called with no args");
        return;
    }
    let msg = crate::interpreter::skip_spaces(rest).to_vec();
    if msg.is_empty() {
        wld_log(g, room, "wsend called without a message");
        return;
    }
    if let Some(ch) = get_char_by_room(g, room, &buf) {
        if subcmd == SCMD_WSEND {
            sub_write(g, &msg, ch, true, TO_CHAR_T);
        } else if subcmd == SCMD_WECHOAROUND {
            sub_write(g, &msg, ch, true, TO_ROOM_T);
        }
    } else {
        wld_log(g, room, "no target found for wsend");
    }
}

fn do_wzoneecho(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (room_num, rest) = crate::interpreter::any_one_arg(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if room_num.is_empty() || msg.is_empty() {
        wld_log(g, room, "wzoneecho called with too few args");
    } else if let Some(zone) = real_zone_by_thing(g, atoi32(&room_num)) {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_zone(g, &buf, zone);
    } else {
        wld_log(g, room, "wzoneecho called for nonexistant zone");
    }
}

fn do_wrecho(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (start, finish, rest) = crate::interpreter::two_arguments(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if msg.is_empty()
        || start.is_empty()
        || finish.is_empty()
        || !crate::interpreter::is_number(&start)
        || !crate::interpreter::is_number(&finish)
    {
        wld_log(g, room, "wrecho: too few args");
    } else {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_range(g, atoi32(&start), atoi32(&finish), &buf);
    }
}

fn do_wdoor(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    door_command(g, argument, "wdoor", |g, msg| wld_log(g, room, msg));
}

fn do_wteleport(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, _) = crate::interpreter::two_arguments(argument);
    if arg1.is_empty() || arg2.is_empty() {
        wld_log(g, room, "wteleport called with too few args");
        return;
    }
    let nr = atoi32(&arg2);
    let target = g.real_room(nr);
    let Some(target) = target else {
        wld_log(g, room, "wteleport target is an invalid room");
        return;
    };
    if eq_ci(&arg1, b"all") {
        if nr == g.world.rooms[room as usize].vnum as i32 {
            wld_log(g, room, "wteleport all target is itself");
            return;
        }
        let people = g.rooms[room as usize].people.clone();
        for ch in people {
            if g.try_ch(ch).is_none() {
                continue;
            }
            if !valid_dg_target(g, ch, DG_ALLOW_GODS) {
                continue;
            }
            char_from_room(g, ch);
            char_to_room(g, ch, target);
            let r = g.ch(ch).in_room;
            enter_wtrigger(g, r, ch, -1);
        }
    } else if let Some(ch) = get_char_by_room(g, room, &arg1) {
        if valid_dg_target(g, ch, DG_ALLOW_GODS) {
            char_from_room(g, ch);
            char_to_room(g, ch, target);
            let r = g.ch(ch).in_room;
            enter_wtrigger(g, r, ch, -1);
        }
    } else {
        wld_log(g, room, "wteleport: no target found");
    }
}

fn do_wforce(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg1, line) = crate::interpreter::one_argument(argument);
    if arg1.is_empty() || crate::interpreter::skip_spaces(line).is_empty() {
        wld_log(g, room, "wforce called with too few args");
        return;
    }
    let line = line.to_vec();
    if eq_ci(&arg1, b"all") {
        let people = g.rooms[room as usize].people.clone();
        for ch in people {
            if g.try_ch(ch).is_none() {
                continue;
            }
            if valid_dg_target(g, ch, 0) {
                crate::interpreter::command_interpreter(g, ch, &line);
            }
        }
    } else if let Some(ch) = get_char_by_room(g, room, &arg1) {
        if valid_dg_target(g, ch, 0) {
            crate::interpreter::command_interpreter(g, ch, &line);
        }
    } else {
        wld_log(g, room, "wforce: no target found");
    }
}

fn do_wpurge(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        let people = g.rooms[room as usize].people.clone();
        for ch in people {
            if g.try_ch(ch).is_some_and(|c| c.is_npc()) {
                extract_char(g, ch);
            }
        }
        let contents = g.rooms[room as usize].contents.clone();
        for o in contents {
            if g.try_obj(o).is_some() {
                extract_obj(g, o);
            }
        }
        return;
    }
    let ch = if arg.first() == Some(&UID_CHAR) {
        get_char(g, &arg)
    } else {
        get_char_in_room(g, room, &arg)
    };
    let Some(ch) = ch else {
        let o = if arg.first() == Some(&UID_CHAR) { get_obj(g, &arg) } else { get_obj_in_room(g, room, &arg) };
        if let Some(o) = o {
            extract_obj(g, o);
        } else {
            wld_log(g, room, "wpurge: bad argument");
        }
        return;
    };
    if !g.ch(ch).is_npc() {
        wld_log(g, room, "wpurge: purging a PC");
        return;
    }
    extract_char(g, ch);
}

fn do_wload(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, target) = crate::interpreter::two_arguments(argument);
    let number = atoi32(&arg2);
    if arg1.is_empty() || arg2.is_empty() || !crate::interpreter::is_number(&arg2) || number < 0 {
        wld_log(g, room, "wload: bad syntax");
        return;
    }
    let target = crate::interpreter::skip_spaces(target).to_vec();

    if crate::handler::is_abbrev(&arg1, b"mob") {
        let rnum = if target.is_empty() {
            Some(room)
        } else if target[0].is_ascii_digit() {
            g.real_room(atoi32(&target))
        } else {
            None
        };
        let Some(rnum) = rnum else {
            let msg = format!(
                "wload: room target vnum doesn't exist (loading mob vnum {} to room {})",
                number,
                String::from_utf8_lossy(&target)
            );
            wld_log(g, room, &msg);
            return;
        };
        let mob = g.world.real_mobile(number as Idx).and_then(|r| crate::db::read_mobile(g, r));
        let Some(mob) = mob else {
            // The message says "mload" — kept as-is.
            wld_log(g, room, "mload: bad mob vnum");
            return;
        };
        char_to_room(g, mob, rnum);
        let uid = super::driver::uid_var(char_script_id(g, mob));
        if let Some(sc) = g.script_of_mut(GoId::Room(room)) {
            super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
        }
        load_mtrigger(g, mob);
    } else if crate::handler::is_abbrev(&arg1, b"obj") {
        let object = g.world.real_object(number as Idx).and_then(|r| crate::db::read_object(g, r));
        let Some(object) = object else {
            wld_log(g, room, "wload: bad object vnum");
            return;
        };
        if target.is_empty() {
            obj_to_room(g, object, room);
            let uid = super::driver::uid_var(obj_script_id(g, object));
            if let Some(sc) = g.script_of_mut(GoId::Room(room)) {
                super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
            }
            load_otrigger(g, object);
            return;
        }
        // NOTE: with a target, lastloaded is not set for objects here.
        let (targ1, targ2, _) = crate::interpreter::two_arguments(&target);
        if let Some(tch) = get_char_in_room(g, room, &targ1) {
            if !targ2.is_empty() {
                let pos = find_eq_pos_script(&targ2);
                if pos >= 0
                    && g.ch(tch).equipment[pos as usize].is_none()
                    && can_wear_on_pos(g, object, pos)
                {
                    equip_char(g, tch, object, pos as usize);
                    load_otrigger(g, object);
                    return;
                }
            }
            obj_to_char(g, object, tch);
            load_otrigger(g, object);
            return;
        }
        if let Some(cnt) = get_obj_in_room(g, room, &targ1) {
            if g.obj(cnt).type_flag == flags::ITEM_CONTAINER {
                obj_to_obj(g, object, cnt);
                load_otrigger(g, object);
                return;
            }
        }
        obj_to_room(g, object, room);
        load_otrigger(g, object);
    } else {
        wld_log(g, room, "wload: bad type");
    }
}

fn do_wdamage(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (name, amount, _) = crate::interpreter::two_arguments(argument);
    if name.is_empty() || amount.is_empty() {
        wld_log(g, room, "wdamage: bad syntax");
        return;
    }
    let dam = atoi32(&amount);
    let Some(ch) = get_char_by_room(g, room, &name) else {
        wld_log(g, room, "wdamage: target not found");
        return;
    };
    script_damage(g, ch, dam);
}

fn do_wat(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg, rest) = crate::interpreter::any_one_arg(argument);
    if arg.is_empty() {
        wld_log(g, room, "wat called with no args");
        return;
    }
    let command = crate::interpreter::skip_spaces(rest).to_vec();
    if command.is_empty() {
        wld_log(g, room, "wat called without a command");
        return;
    }
    let loc = if arg[0].is_ascii_digit() {
        g.real_room(atoi32(&arg))
    } else {
        get_char_by_room(g, room, &arg).map(|ch| g.ch(ch).in_room)
    };
    let Some(loc) = loc.filter(|&l| l != NOWHERE) else {
        let msg = format!("wat: location not found ({})", String::from_utf8_lossy(&arg));
        wld_log(g, room, &msg);
        return;
    };
    wld_command_interpreter(g, loc, &command);
}

fn do_wmove(g: &mut Game, room: RoomRnum, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, _) = crate::interpreter::two_arguments(argument);
    if arg1.is_empty() || arg2.is_empty() {
        wld_log(g, room, "wmove called with too few args");
        return;
    }
    let nr = atoi32(&arg2);
    let target = g.real_room(nr);
    let Some(target) = target else {
        wld_log(g, room, "wmove target is an invalid room");
        return;
    };
    if nr == g.world.rooms[room as usize].vnum as i32 {
        wld_log(g, room, "wmove target room is itself");
        return;
    }
    if eq_ci(&arg1, b"all") {
        let contents = g.rooms[room as usize].contents.clone();
        for o in contents {
            if g.try_obj(o).is_none() {
                continue;
            }
            obj_from_room(g, o);
            obj_to_room(g, o, target);
        }
    } else if let Some(o) = get_obj_by_room(g, room, &arg1) {
        obj_from_room(g, o);
        obj_to_room(g, o, target);
    } else {
        wld_log(g, room, "wmove: no target found");
    }
}

type WldCmd = fn(&mut Game, RoomRnum, &[u8], i32);

/// wld_cmd_info; note "wlog" has no trailing space.
const WLD_CMD_INFO: [(&[u8], WldCmd, i32); 16] = [
    (b"RESERVED", do_wlog, 0), // never matched
    (b"wasound ", do_wasound, 0),
    (b"wdoor ", do_wdoor, 0),
    (b"wecho ", do_wecho, 0),
    (b"wechoaround ", do_wsend, SCMD_WECHOAROUND),
    (b"wforce ", do_wforce, 0),
    (b"wload ", do_wload, 0),
    (b"wpurge ", do_wpurge, 0),
    (b"wrecho ", do_wrecho, 0),
    (b"wsend ", do_wsend, SCMD_WSEND),
    (b"wteleport ", do_wteleport, 0),
    (b"wzoneecho ", do_wzoneecho, 0),
    (b"wdamage ", do_wdamage, 0),
    (b"wat ", do_wat, 0),
    (b"wmove ", do_wmove, 0),
    (b"wlog", do_wlog, 0),
];

pub fn wld_command_interpreter(g: &mut Game, room: RoomRnum, argument: &[u8]) {
    let argument = crate::interpreter::skip_spaces(argument);
    if argument.is_empty() {
        return;
    }
    let (arg, line) = crate::interpreter::any_one_arg(argument);
    let length = arg.len();
    for (name, func, subcmd) in WLD_CMD_INFO {
        if name.len() >= length && &name[..length] == &arg[..] {
            let line = line.to_vec();
            func(g, room, &line, subcmd);
            return;
        }
    }
    let msg = format!("Unknown world cmd: '{}'", String::from_utf8_lossy(argument));
    wld_log(g, room, &msg);
}

// Silence unused import when no wld command needs CharId directly.
#[allow(dead_code)]
fn _t(_: CharId) {}
