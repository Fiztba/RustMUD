//! The o-commands + obj_command_interpreter (prefix match in
//! table order, case-sensitive).

use mud_data::flags;
use mud_data::ids::ObjId;
use mud_data::types::*;

use super::comm::{send_to_range, send_to_zone, sub_write, TO_CHAR_T, TO_ROOM_T};
use super::misc::{script_damage, valid_dg_target};
use super::mobcmd::real_zone_by_thing;
use super::triggers::{enter_wtrigger, load_mtrigger, load_otrigger};
use super::variables::{can_wear_on_pos, find_eq_pos_script};
use super::{
    atoi32, char_script_id, get_char_by_obj, get_char_near_obj, get_obj_by_obj,
    get_obj_near_obj, get_room, obj_log, obj_room, obj_script_id, GoId, DG_ALLOW_GODS,
};
use crate::game::Game;
use crate::handler::{
    char_from_room, char_to_room, equip_char, extract_char, extract_obj, obj_from_char,
    obj_from_obj, obj_from_room, obj_to_char, obj_to_obj, obj_to_room, eq_ci, unequip_char,
};

pub type BStr = Vec<u8>;

const SCMD_OSEND: i32 = 0;
const SCMD_OECHOAROUND: i32 = 1;

fn find_obj_target_room(g: &mut Game, oid: ObjId, rawroomstr: &[u8]) -> Option<RoomRnum> {
    let (roomstr, _) = crate::interpreter::one_argument(rawroomstr);
    if roomstr.is_empty() {
        return None;
    }
    let location = if roomstr[0].is_ascii_digit() && !roomstr.contains(&b'.') {
        g.real_room(atoi32(&roomstr))?
    } else if let Some(target_mob) = get_char_by_obj(g, oid, &roomstr) {
        g.ch(target_mob).in_room
    } else if let Some(target_obj) = get_obj_by_obj(g, oid, &roomstr) {
        let r = g.obj(target_obj).in_room;
        if r == NOWHERE {
            return None;
        }
        r
    } else {
        return None;
    };

    let rflag = |bit: usize| g.world.rooms[location as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0;
    if rflag(flags::ROOM_GODROOM) || rflag(flags::ROOM_HOUSE) {
        return None;
    }
    if rflag(flags::ROOM_PRIVATE) && g.rooms[location as usize].people.len() > 1 {
        return None;
    }
    Some(location)
}

fn do_oecho(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        obj_log(g, oid, "oecho called with no args");
        return;
    }
    let room = obj_room(g, oid);
    if room != NOWHERE {
        if let Some(&first) = g.rooms[room as usize].people.first() {
            sub_write(g, &argument, first, true, TO_ROOM_T);
            sub_write(g, &argument, first, true, TO_CHAR_T);
        }
    } else {
        obj_log(g, oid, "oecho called by object in NOWHERE");
    }
}

fn do_olog(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument);
    if !argument.is_empty() {
        let msg = String::from_utf8_lossy(argument).into_owned();
        obj_log(g, oid, &msg);
    }
}

fn do_oforce(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg1, line) = crate::interpreter::one_argument(argument);
    if arg1.is_empty() || crate::interpreter::skip_spaces(line).is_empty() {
        obj_log(g, oid, "oforce called with too few args");
        return;
    }
    let line = line.to_vec();
    if eq_ci(&arg1, b"all") {
        let room = obj_room(g, oid);
        if room == NOWHERE {
            obj_log(g, oid, "oforce called by object in NOWHERE");
        } else {
            let people = g.rooms[room as usize].people.clone();
            for ch in people {
                if g.try_ch(ch).is_none() {
                    continue;
                }
                if valid_dg_target(g, ch, 0) {
                    crate::interpreter::command_interpreter(g, ch, &line);
                }
            }
        }
    } else if let Some(ch) = get_char_by_obj(g, oid, &arg1) {
        if valid_dg_target(g, ch, 0) {
            crate::interpreter::command_interpreter(g, ch, &line);
        }
    } else {
        obj_log(g, oid, "oforce: no target found");
    }
}

fn do_ozoneecho(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (room_number, rest) = crate::interpreter::any_one_arg(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if room_number.is_empty() || msg.is_empty() {
        obj_log(g, oid, "ozoneecho called with too few args");
    } else if let Some(zone) = real_zone_by_thing(g, atoi32(&room_number)) {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_zone(g, &buf, zone);
    } else {
        obj_log(g, oid, "ozoneecho called for nonexistant zone");
    }
}

fn do_osend(g: &mut Game, oid: ObjId, argument: &[u8], subcmd: i32) {
    let (buf, rest) = crate::interpreter::any_one_arg(argument);
    if buf.is_empty() {
        obj_log(g, oid, "osend called with no args");
        return;
    }
    let msg = crate::interpreter::skip_spaces(rest).to_vec();
    if msg.is_empty() {
        obj_log(g, oid, "osend called without a message");
        return;
    }
    if let Some(ch) = get_char_by_obj(g, oid, &buf) {
        if subcmd == SCMD_OSEND {
            sub_write(g, &msg, ch, true, TO_CHAR_T);
        } else if subcmd == SCMD_OECHOAROUND {
            sub_write(g, &msg, ch, true, TO_ROOM_T);
        }
    } else {
        obj_log(g, oid, "no target found for osend");
    }
}

fn do_orecho(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (start, finish, rest) = crate::interpreter::two_arguments(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if msg.is_empty()
        || start.is_empty()
        || finish.is_empty()
        || !crate::interpreter::is_number(&start)
        || !crate::interpreter::is_number(&finish)
    {
        obj_log(g, oid, "orecho: too few args");
    } else {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_range(g, atoi32(&start), atoi32(&finish), &buf);
    }
}

fn do_otimer(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        obj_log(g, oid, "otimer: missing argument");
    } else if !arg[0].is_ascii_digit() {
        obj_log(g, oid, "otimer: bad argument");
    } else {
        g.obj_mut(oid).timer = atoi32(&arg);
    }
}

fn do_otransform(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        obj_log(g, oid, "otransform: missing argument");
        return;
    }
    if !arg[0].is_ascii_digit() {
        obj_log(g, oid, "otransform: bad argument");
        return;
    }
    let o_rnum = g.world.real_object(atoi32(&arg) as Idx);
    let o = o_rnum.and_then(|r| crate::db::read_object(g, r));
    let Some(o) = o else {
        obj_log(g, oid, "otransform: bad object vnum");
        return;
    };

    let (wearer, pos) = {
        let ob = g.obj(oid);
        (ob.worn_by, ob.worn_on)
    };
    if let Some(w) = wearer {
        unequip_char(g, w, pos as usize);
    }

    // Copy the fresh instance over, preserving location + script identity.
    // NOTE: item_number is NOT restored — %self.vnum% reports the new vnum.
    let new_body = g.obj(o).clone();
    {
        let old = g.obj(oid);
        let keep_in_room = old.in_room;
        let keep_carried_by = old.carried_by;
        let keep_worn_by = old.worn_by;
        let keep_worn_on = old.worn_on;
        let keep_in_obj = old.in_obj;
        let keep_contains = old.contains.clone();
        let keep_script_id = old.script_id;
        let keep_proto_script = old.proto_script.clone();
        let keep_script = old.script.clone();

        let ob = g.obj_mut(oid);
        *ob = new_body;
        ob.in_room = keep_in_room;
        ob.carried_by = keep_carried_by;
        ob.worn_by = keep_worn_by;
        ob.worn_on = keep_worn_on;
        ob.in_obj = keep_in_obj;
        ob.contains = keep_contains;
        ob.script_id = keep_script_id;
        ob.proto_script = keep_proto_script;
        ob.script = keep_script;
    }

    if let Some(w) = wearer {
        equip_char(g, w, oid, pos as usize);
    }
    extract_obj(g, o);
}

fn do_opurge(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        let rm = obj_room(g, oid);
        if rm != NOWHERE {
            let people = g.rooms[rm as usize].people.clone();
            for ch in people {
                if g.try_ch(ch).is_some_and(|c| c.is_npc()) {
                    extract_char(g, ch);
                }
            }
            let contents = g.rooms[rm as usize].contents.clone();
            for o in contents {
                if o != oid && g.try_obj(o).is_some() {
                    extract_obj(g, o);
                }
            }
        }
        return;
    }
    let ch = get_char_by_obj(g, oid, &arg);
    let Some(ch) = ch else {
        let o = get_obj_by_obj(g, oid, &arg);
        if let Some(o) = o {
            if o == oid {
                g.dg_owner_purged = true;
            }
            extract_obj(g, o);
        } else {
            obj_log(g, oid, "opurge: bad argument");
        }
        return;
    };
    if !g.ch(ch).is_npc() {
        obj_log(g, oid, "opurge: purging a PC");
        return;
    }
    extract_char(g, ch);
}

fn do_oteleport(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, _) = crate::interpreter::two_arguments(argument);
    if arg1.is_empty() || arg2.is_empty() {
        obj_log(g, oid, "oteleport called with too few args");
        return;
    }
    let target = find_obj_target_room(g, oid, &arg2);
    let Some(target) = target else {
        obj_log(g, oid, "oteleport target is an invalid room");
        return;
    };
    if eq_ci(&arg1, b"all") {
        let rm = obj_room(g, oid);
        if target == rm {
            // Logged, but the teleport still happens.
            obj_log(g, oid, "oteleport target is itself");
        }
        if rm == NOWHERE {
            return;
        }
        let people = g.rooms[rm as usize].people.clone();
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
    } else if let Some(ch) = get_char_by_obj(g, oid, &arg1) {
        if valid_dg_target(g, ch, DG_ALLOW_GODS) {
            char_from_room(g, ch);
            char_to_room(g, ch, target);
            let r = g.ch(ch).in_room;
            enter_wtrigger(g, r, ch, -1);
        }
    } else {
        obj_log(g, oid, "oteleport: no target found");
    }
}

fn do_dgoload(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, target) = crate::interpreter::two_arguments(argument);
    let number = atoi32(&arg2);
    if arg1.is_empty() || arg2.is_empty() || !crate::interpreter::is_number(&arg2) || number < 0 {
        obj_log(g, oid, "oload: bad syntax");
        return;
    }
    let room = obj_room(g, oid);
    if room == NOWHERE {
        obj_log(g, oid, "oload: object in NOWHERE trying to load");
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
                "oload: room target vnum doesn't exist (loading mob vnum {} to room {})",
                number,
                String::from_utf8_lossy(&target)
            );
            obj_log(g, oid, &msg);
            return;
        };
        let mob = g.world.real_mobile(number as Idx).and_then(|r| crate::db::read_mobile(g, r));
        let Some(mob) = mob else {
            obj_log(g, oid, "oload: bad mob vnum");
            return;
        };
        char_to_room(g, mob, rnum);
        let uid = super::driver::uid_var(char_script_id(g, mob));
        if let Some(sc) = g.script_of_mut(GoId::Obj(oid)) {
            super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
        }
        load_mtrigger(g, mob);
    } else if crate::handler::is_abbrev(&arg1, b"obj") {
        let object = g.world.real_object(number as Idx).and_then(|r| crate::db::read_object(g, r));
        let Some(object) = object else {
            obj_log(g, oid, "oload: bad object vnum");
            return;
        };
        let uid = super::driver::uid_var(obj_script_id(g, object));
        if let Some(sc) = g.script_of_mut(GoId::Obj(oid)) {
            super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
        }
        if target.is_empty() {
            obj_to_room(g, object, room);
            load_otrigger(g, object);
            return;
        }
        let (targ1, targ2, _) = crate::interpreter::two_arguments(&target);
        if let Some(tch) = get_char_near_obj(g, oid, &targ1) {
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
        if let Some(cnt) = get_obj_near_obj(g, oid, &targ1) {
            if g.obj(cnt).type_flag == flags::ITEM_CONTAINER {
                obj_to_obj(g, object, cnt);
                load_otrigger(g, object);
                return;
            }
        }
        obj_to_room(g, object, room);
        load_otrigger(g, object);
    } else {
        obj_log(g, oid, "oload: bad type");
    }
}

fn do_odamage(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (name, amount, _) = crate::interpreter::two_arguments(argument);
    if name.is_empty() || amount.is_empty() {
        obj_log(g, oid, "odamage: bad syntax");
        return;
    }
    let dam = atoi32(&amount);
    let Some(ch) = get_char_by_obj(g, oid, &name) else {
        obj_log(g, oid, "odamage: target not found");
        return;
    };
    script_damage(g, ch, dam);
}

fn do_oasound(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        obj_log(g, oid, "oasound called with no args");
        return;
    }
    let room = obj_room(g, oid);
    if room == NOWHERE {
        obj_log(g, oid, "oasound called by object in NOWHERE");
        return;
    }
    let dir_count = crate::fight::dir_count(g) as usize;
    for door in 0..dir_count {
        let to = g.world.rooms[room as usize].dir_option[door]
            .as_ref()
            .map(|e| e.to_room)
            .filter(|&t| t != NOWHERE && t != room);
        if let Some(to_room) = to {
            if let Some(&first) = g.rooms[to_room as usize].people.first() {
                sub_write(g, &argument, first, true, TO_ROOM_T);
                sub_write(g, &argument, first, true, TO_CHAR_T);
            }
        }
    }
}

fn do_odoor(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    door_command(g, argument, "odoor", |g, msg| obj_log(g, oid, msg));
}

/// The full mdoor/odoor/wdoor body, parameterized by the log prefix.
pub fn door_command(g: &mut Game, argument: &[u8], cmdname: &str, log: impl Fn(&mut Game, &str) + Copy) {
    const DOOR_FIELD: [&str; 6] = ["purge", "description", "flags", "key", "name", "room"];
    let (target, direction, rest) = crate::interpreter::two_arguments(argument);
    let (field, rest2) = crate::interpreter::one_argument(rest);
    let value = crate::interpreter::skip_spaces(rest2).to_vec();

    if target.is_empty() || direction.is_empty() || field.is_empty() {
        log(g, &format!("{} called with too few args", cmdname));
        return;
    }
    let Some(rm) = get_room(g, &target) else {
        log(g, &format!("{}: invalid target (arg == {})", cmdname, String::from_utf8_lossy(&target)));
        return;
    };
    let Some(dir) = search_block_prefix(&direction, &mud_data::tables::DIRS) else {
        let dirs_str = mud_data::tables::DIRS.join(" ");
        log(
            g,
            &format!(
                "{}: invalid direction (arg == {}) not found in: [ {} ]",
                cmdname,
                String::from_utf8_lossy(&direction),
                dirs_str
            ),
        );
        return;
    };
    let Some(fd) = search_block_prefix(&field, &DOOR_FIELD) else {
        let fields_str = DOOR_FIELD.join(" ");
        log(
            g,
            &format!(
                "{}: invalid field (arg == {}) not found in: [ {} ]",
                cmdname,
                String::from_utf8_lossy(&field),
                fields_str
            ),
        );
        return;
    };
    apply_door_field(g, rm, dir, fd, &value, log, cmdname);
}

/// search_block with exact=FALSE: case-sensitive prefix over the table
/// (arguments arrive pre-lowercased by the arg parsers).
pub fn search_block_prefix(arg: &[u8], list: &[&str]) -> Option<usize> {
    for (i, name) in list.iter().enumerate() {
        let nb = name.as_bytes();
        if nb.len() >= arg.len() && &nb[..arg.len()] == arg {
            return Some(i);
        }
    }
    None
}

/// The shared door-field application (mdoor/odoor/wdoor case arms).
pub fn apply_door_field(
    g: &mut Game,
    rm: RoomRnum,
    dir: usize,
    fd: usize,
    value: &[u8],
    log: impl Fn(&mut Game, &str) + Copy,
    cmdname: &str,
) {
    if fd == 0 {
        g.world.rooms[rm as usize].dir_option[dir] = None;
        return;
    }
    if g.world.rooms[rm as usize].dir_option[dir].is_none() {
        g.world.rooms[rm as usize].dir_option[dir] = Some(Box::new(mud_world::model::Exit {
            general_description: None,
            keyword: None,
            exit_info: 0,
            key: 0,
            to_room_vnum: 0,
            to_room: 0,
        }));
    }
    let mut invalid = false;
    {
        let exit = g.world.rooms[rm as usize].dir_option[dir].as_mut().unwrap();
        match fd {
            1 => {
                let mut d = value.to_vec();
                d.extend_from_slice(b"\r\n");
                exit.general_description = Some(d);
            }
            2 => exit.exit_info = mud_world::lex::asciiflag_conv(value) as u16,
            3 => exit.key = atoi32(value) as Idx,
            4 => exit.keyword = Some(value.to_vec()),
            _ => {}
        }
    }
    if fd == 5 {
        match g.real_room(atoi32(value)) {
            Some(to) => g.world.rooms[rm as usize].dir_option[dir].as_mut().unwrap().to_room = to,
            None => {
                g.world.rooms[rm as usize].dir_option[dir].as_mut().unwrap().to_room = NOWHERE;
                invalid = true;
            }
        }
    }
    if invalid {
        let msg =
            format!("{}: invalid door target (arg == {})", cmdname, String::from_utf8_lossy(value));
        log(g, &msg);
    }
}

fn do_osetval(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg1, arg2, _) = crate::interpreter::two_arguments(argument);
    if arg1.is_empty()
        || arg2.is_empty()
        || !crate::interpreter::is_number(&arg1)
        || !crate::interpreter::is_number(&arg2)
    {
        obj_log(g, oid, "osetval: bad syntax");
        return;
    }
    let position = atoi32(&arg1);
    let new_value = atoi32(&arg2);
    if (0..4).contains(&position) {
        let (worn_by, worn_on) = {
            let o = g.obj(oid);
            (o.worn_by, o.worn_on)
        };
        if let Some(w) = worn_by {
            unequip_char(g, w, worn_on as usize);
        }
        g.obj_mut(oid).values[position as usize] = new_value;
        if let Some(w) = worn_by {
            equip_char(g, w, oid, worn_on as usize);
        }
    } else {
        obj_log(g, oid, "osetval: position out of bounds!");
    }
}

fn do_oat(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg, rest) = crate::interpreter::any_one_arg(argument);
    if arg.is_empty() {
        obj_log(g, oid, "oat called with no args");
        return;
    }
    let command = crate::interpreter::skip_spaces(rest).to_vec();
    if command.is_empty() {
        obj_log(g, oid, "oat called without a command");
        return;
    }
    let loc = if arg[0].is_ascii_digit() {
        g.real_room(atoi32(&arg))
    } else {
        get_char_by_obj(g, oid, &arg).map(|ch| g.ch(ch).in_room)
    };
    let Some(loc) = loc.filter(|&l| l != NOWHERE) else {
        let msg = format!("oat: location not found ({})", String::from_utf8_lossy(&arg));
        obj_log(g, oid, &msg);
        return;
    };
    // A fresh copy of this object's prototype acts, then is purged.
    let vnum = super::obj_vnum(g, oid);
    let object = if vnum < 0 {
        None
    } else {
        g.world.real_object(vnum as Idx).and_then(|r| crate::db::read_object(g, r))
    };
    let Some(object) = object else { return };
    obj_to_room(g, object, loc);
    obj_command_interpreter(g, object, &command);
    if g.try_obj(object).is_some_and(|o| o.in_room == loc) {
        extract_obj(g, object);
    }
}

fn do_omove(g: &mut Game, oid: ObjId, argument: &[u8], _subcmd: i32) {
    let (arg1, _) = crate::interpreter::one_argument(argument);
    if arg1.is_empty() {
        obj_log(g, oid, "omove called with too few args");
        return;
    }
    let target = find_obj_target_room(g, oid, &arg1);
    if target.is_none() {
        // Logged, but the move still happens: obj_to_room refuses NOWHERE
        // and the object is left in limbo. Deliberate.
        obj_log(g, oid, "omove target is an invalid room");
    }
    let ob = g.obj(oid);
    if ob.carried_by.is_some() {
        obj_from_char(g, oid);
    } else if ob.in_room != NOWHERE {
        obj_from_room(g, oid);
    } else if ob.in_obj.is_some() {
        obj_from_obj(g, oid);
    } else {
        obj_log(g, oid, "omove: target object is not in a room, held or in a container!");
        return;
    }
    obj_to_room(g, oid, target.unwrap_or(NOWHERE));
}

type ObjCmd = fn(&mut Game, ObjId, &[u8], i32);

/// obj_cmd_info: entries carry a trailing space; matching
/// is strncmp (case-sensitive) over strlen(arg) in table order.
const OBJ_CMD_INFO: [(&[u8], ObjCmd, i32); 19] = [
    (b"RESERVED", do_olog, 0), // never matched: arg is non-empty and differs
    (b"oasound ", do_oasound, 0),
    (b"oat ", do_oat, 0),
    (b"odoor ", do_odoor, 0),
    (b"odamage ", do_odamage, 0),
    (b"oecho ", do_oecho, 0),
    (b"oechoaround ", do_osend, SCMD_OECHOAROUND),
    (b"oforce ", do_oforce, 0),
    (b"oload ", do_dgoload, 0),
    (b"opurge ", do_opurge, 0),
    (b"orecho ", do_orecho, 0),
    (b"osend ", do_osend, SCMD_OSEND),
    (b"osetval ", do_osetval, 0),
    (b"oteleport ", do_oteleport, 0),
    (b"otimer ", do_otimer, 0),
    (b"otransform ", do_otransform, 0),
    (b"ozoneecho ", do_ozoneecho, 0),
    (b"omove ", do_omove, 0),
    (b"olog ", do_olog, 0),
];

pub fn obj_command_interpreter(g: &mut Game, oid: ObjId, argument: &[u8]) {
    let argument = crate::interpreter::skip_spaces(argument);
    if argument.is_empty() {
        return;
    }
    let (arg, line) = crate::interpreter::any_one_arg(argument);
    let length = arg.len();
    for (name, func, subcmd) in OBJ_CMD_INFO {
        if name.len() >= length && &name[..length] == &arg[..] {
            let line = line.to_vec();
            func(g, oid, &line, subcmd);
            return;
        }
    }
    let msg = format!("Unknown object cmd: '{}'", String::from_utf8_lossy(argument));
    obj_log(g, oid, &msg);
}
