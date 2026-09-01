//! The 22 m-commands plus script_command_interpreter

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use super::comm::{send_to_range, send_to_zone, sub_write, TO_CHAR_T, TO_ROOM_T};
use super::misc::{script_damage, valid_dg_target};
use super::triggers::{enter_wtrigger, load_mtrigger, load_otrigger};
use super::variables::find_eq_pos_script;
use super::{
    atoi32, char_script_id, get_char, get_obj, mob_log, obj_script_id, GoId, ScriptMem,
    DG_ALLOW_GODS, UID_CHAR,
};
use crate::game::Game;
use crate::handler::{
    char_from_room, char_to_room, equip_char, extract_char, get_char_room_vis, get_char_world_vis,
    get_number, get_obj_in_list_vis, get_obj_pos_in_equip_vis, get_obj_vis_counted, isname,
    obj_to_char, obj_to_obj, obj_to_room, eq_ci, unequip_char,
};

pub type BStr = Vec<u8>;

fn mob_or_impl(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if ch.is_npc() {
        match ch.desc {
            None => return true,
            Some(di) => {
                // switched: original's level must be IMPL.
                if let Some(d) = g.descriptors.get(di) {
                    if let Some(orig) = d.original {
                        if let Some(oc) = g.try_ch(orig) {
                            if oc.level >= LVL_IMPL {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    // Script-players: any entity with a script that has triggers.
    ch.script.as_ref().is_some_and(|sc| !sc.trig_list.is_empty())
}

/// Switched-descriptor guard several commands add ("IMPL" in the doc).
fn impl_guard(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if let Some(di) = ch.desc {
        if let Some(d) = g.descriptors.get(di) {
            if let Some(orig) = d.original {
                if g.try_ch(orig).is_some_and(|oc| oc.level < LVL_IMPL) {
                    return true; // silently return
                }
            }
        }
    }
    false
}

fn huh(g: &mut Game, chid: CharId) {
    let huh = g.config.huh.clone();
    crate::comm::send_to_char(g, chid, &huh);
}

fn charmed(g: &Game, chid: CharId) -> bool {
    g.ch(chid).aff(flags::AFF_CHARM)
}

/// Common victim resolution: UID → get_char (world), else room-vis.
fn victim_uid_or_room(g: &mut Game, chid: CharId, arg: &[u8]) -> Option<CharId> {
    if arg.first() == Some(&UID_CHAR) {
        get_char(g, arg)
    } else {
        get_char_room_vis(g, chid, arg, None)
    }
}

fn victim_uid_or_world(g: &mut Game, chid: CharId, arg: &[u8]) -> Option<CharId> {
    if arg.first() == Some(&UID_CHAR) {
        get_char(g, arg)
    } else {
        get_char_world_vis(g, chid, arg, None)
    }
}

/// get_obj_vis wrapper for the no-number form.
fn obj_vis(g: &Game, chid: CharId, name: &[u8]) -> Option<mud_data::ids::ObjId> {
    let (mut num, stripped) = get_number(name);
    if num == 0 {
        return None;
    }
    get_obj_vis_counted(g, chid, &stripped, &mut num)
}

/// real_zone_by_thing: zone whose [bot,top] holds the vnum.
pub fn real_zone_by_thing(g: &Game, vnum: i32) -> Option<usize> {
    (0..g.world.zones.len())
        .find(|&i| g.world.zones[i].bot as i32 <= vnum && vnum <= g.world.zones[i].top as i32)
}

pub fn do_masound(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if argument.is_empty() {
        mob_log(g, chid, "masound called with no argument");
        return;
    }
    let arg = crate::interpreter::skip_spaces(argument).to_vec();
    let was_in = g.ch(chid).in_room;
    let dir_count = crate::fight::dir_count(g) as usize;
    for door in 0..dir_count {
        let to = g.world.rooms[was_in as usize].dir_option[door]
            .as_ref()
            .map(|e| e.to_room)
            .filter(|&t| t != NOWHERE && t != was_in);
        if let Some(to_room) = to {
            g.ch_mut(chid).in_room = to_room;
            sub_write(g, &arg, chid, true, TO_ROOM_T);
        }
    }
    g.ch_mut(chid).in_room = was_in;
}

pub fn do_mkill(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mkill called with no argument");
        return;
    }
    let Some(victim) = victim_uid_or_room(g, chid, &arg) else {
        let msg = format!("mkill: victim ({}) not found", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    if victim == chid {
        mob_log(g, chid, "mkill: victim is self");
        return;
    }
    if !g.ch(victim).is_npc() && g.ch(victim).prf(flags::PRF_NOHASSLE) {
        mob_log(g, chid, "mkill: target has nohassle on");
        return;
    }
    if g.ch(chid).fighting.is_some() {
        mob_log(g, chid, "mkill: already fighting");
        return;
    }
    crate::fight::hit(g, chid, victim, mud_data::spells::TYPE_UNDEFINED);
}

pub fn do_mjunk(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mjunk called with no argument");
        return;
    }
    let junk_all = eq_ci(&arg, b"all");
    // find_all_dots reduces "all.x" to "x"; `stripped` is that view.
    let (dotmode, stripped) = crate::act::item::find_all_dots(&arg);
    if dotmode != crate::act::item::FIND_INDIV && !junk_all {
        // "all.x": junk ONE matching item — worn first, then inventory.
        if let Some(pos) = get_obj_pos_in_equip_vis(g, chid, &stripped, None) {
            if let Some(o) = unequip_char(g, chid, pos) {
                crate::handler::extract_obj(g, o);
            }
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        if let Some(o) = get_obj_in_list_vis(g, chid, &stripped, None, &carrying) {
            crate::handler::extract_obj(g, o);
        }
    } else {
        // "all" or a plain name: sweep inventory on arg[3]/arg+4 — a very
        // literal check, so a short name junks everything — then the worn
        // set by full keyword.
        let carrying = g.ch(chid).carrying.clone();
        for o in carrying {
            if g.try_obj(o).is_none() {
                continue;
            }
            let hit = arg.len() <= 3 || isname(&arg[4..], crate::handler::obj_name(g, o));
            if hit {
                crate::handler::extract_obj(g, o);
            }
        }
        while let Some(pos) = get_obj_pos_in_equip_vis(g, chid, &arg, None) {
            match unequip_char(g, chid, pos) {
                Some(o) => crate::handler::extract_obj(g, o),
                None => break,
            }
        }
    }
}

pub fn do_mechoaround(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, rest) = crate::interpreter::one_argument(argument);
    let p = crate::interpreter::skip_spaces(rest).to_vec();
    if arg.is_empty() {
        mob_log(g, chid, "mechoaround called with no argument");
        return;
    }
    let Some(victim) = victim_uid_or_room(g, chid, &arg) else {
        let msg = format!("mechoaround: victim ({}) does not exist", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    sub_write(g, &p, victim, true, TO_ROOM_T);
}

pub fn do_msend(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, rest) = crate::interpreter::one_argument(argument);
    let p = crate::interpreter::skip_spaces(rest).to_vec();
    if arg.is_empty() {
        mob_log(g, chid, "msend called with no argument");
        return;
    }
    let Some(victim) = victim_uid_or_room(g, chid, &arg) else {
        let msg = format!("msend: victim ({}) does not exist", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    sub_write(g, &p, victim, true, TO_CHAR_T);
}

pub fn do_mecho(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if argument.is_empty() {
        mob_log(g, chid, "mecho called with no arguments");
        return;
    }
    let p = crate::interpreter::skip_spaces(argument).to_vec();
    sub_write(g, &p, chid, true, TO_ROOM_T);
    sub_write(g, &p, chid, true, TO_CHAR_T);
}

pub fn do_mlog(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if argument.is_empty() {
        return;
    }
    let p = crate::interpreter::skip_spaces(argument);
    let msg = String::from_utf8_lossy(p).into_owned();
    mob_log(g, chid, &msg);
}

pub fn do_mzoneecho(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    let (room_number, rest) = crate::interpreter::any_one_arg(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if room_number.is_empty() || msg.is_empty() {
        mob_log(g, chid, "mzoneecho called with too few args");
    } else if let Some(zone) = real_zone_by_thing(g, atoi32(&room_number)) {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_zone(g, &buf, zone);
    } else {
        mob_log(g, chid, "mzoneecho called for nonexistant zone");
    }
}

pub fn do_mload(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg1, arg2, target) = crate::interpreter::two_arguments(argument);
    let number = atoi32(&arg2);
    if arg1.is_empty() || arg2.is_empty() || !crate::interpreter::is_number(&arg2) || number < 0 {
        mob_log(g, chid, "mload: bad syntax");
        return;
    }
    let target = crate::interpreter::skip_spaces(target).to_vec();

    if crate::handler::is_abbrev(&arg1, b"mob") {
        let rnum = if target.is_empty() {
            Some(g.ch(chid).in_room)
        } else if target[0].is_ascii_digit() {
            g.real_room(atoi32(&target))
        } else {
            None
        };
        let Some(rnum) = rnum else {
            let msg = format!(
                "mload: room target vnum doesn't exist (loading mob vnum {} to room {})",
                number,
                String::from_utf8_lossy(&target)
            );
            mob_log(g, chid, &msg);
            return;
        };
        let Some(mob_rnum) = g.world.real_mobile(number as Idx) else {
            mob_log(g, chid, "mload: bad mob vnum");
            return;
        };
        let Some(mob) = crate::db::read_mobile(g, mob_rnum) else {
            mob_log(g, chid, "mload: bad mob vnum");
            return;
        };
        char_to_room(g, mob, rnum);
        let uid = super::driver::uid_var(char_script_id(g, mob));
        if let Some(sc) = g.script_of_mut(GoId::Char(chid)) {
            super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
        }
        load_mtrigger(g, mob);
    } else if crate::handler::is_abbrev(&arg1, b"obj") {
        let Some(obj_rnum) = g.world.real_object(number as Idx) else {
            mob_log(g, chid, "mload: bad object vnum");
            return;
        };
        let Some(object) = crate::db::read_object(g, obj_rnum) else {
            mob_log(g, chid, "mload: bad object vnum");
            return;
        };
        let uid = super::driver::uid_var(obj_script_id(g, object));
        if let Some(sc) = g.script_of_mut(GoId::Char(chid)) {
            super::add_var(&mut sc.global_vars, b"lastloaded", &uid, 0);
        }
        if target.is_empty() {
            if g.obj(object).can_wear(flags::ITEM_WEAR_TAKE) {
                obj_to_char(g, object, chid);
            } else {
                let room = g.ch(chid).in_room;
                obj_to_room(g, object, room);
            }
            load_otrigger(g, object);
            return;
        }
        let (targ1, targ2, _) = crate::interpreter::two_arguments(&target);
        let tch = if targ1.first() == Some(&UID_CHAR) {
            get_char(g, &targ1)
        } else {
            get_char_room_vis(g, chid, &targ1, None)
        };
        if let Some(tch) = tch {
            if !targ2.is_empty() {
                let pos = find_eq_pos_script(&targ2);
                if pos >= 0
                    && g.ch(tch).equipment[pos as usize].is_none()
                    && super::variables::can_wear_on_pos(g, object, pos)
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
        let cnt = if targ1.first() == Some(&UID_CHAR) {
            get_obj(g, &targ1)
        } else {
            obj_vis(g, chid, &targ1)
        };
        if let Some(cnt) = cnt {
            if g.obj(cnt).type_flag == flags::ITEM_CONTAINER {
                obj_to_obj(g, object, cnt);
                load_otrigger(g, object);
                return;
            }
        }
        let room = g.ch(chid).in_room;
        obj_to_room(g, object, room);
        load_otrigger(g, object);
    } else {
        mob_log(g, chid, "mload: bad type");
    }
}

pub fn do_mpurge(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);

    if arg.is_empty() {
        let room = g.ch(chid).in_room;
        let people = g.rooms[room as usize].people.clone();
        for victim in people {
            if g.try_ch(victim).is_some_and(|c| c.is_npc()) && victim != chid {
                extract_char(g, victim);
            }
        }
        let contents = g.rooms[room as usize].contents.clone();
        for obj in contents {
            if g.try_obj(obj).is_some() {
                crate::handler::extract_obj(g, obj);
            }
        }
        return;
    }

    let victim = if arg.first() == Some(&UID_CHAR) {
        get_char(g, &arg)
    } else {
        get_char_room_vis(g, chid, &arg, None)
    };
    let Some(victim) = victim else {
        let obj = if arg.first() == Some(&UID_CHAR) { get_obj(g, &arg) } else { obj_vis(g, chid, &arg) };
        match obj {
            Some(o) => crate::handler::extract_obj(g, o),
            None => mob_log(g, chid, "mpurge: bad argument"),
        }
        return;
    };
    if !g.ch(victim).is_npc() {
        mob_log(g, chid, "mpurge: purging a PC");
        return;
    }
    if victim == chid {
        g.dg_owner_purged = true;
    }
    extract_char(g, victim);
}

pub fn do_mgoto(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mgoto called with no argument");
        return;
    }
    let Some(location) = crate::act::wizard::find_target_room(g, chid, &arg) else {
        mob_log(g, chid, "mgoto: invalid location");
        return;
    };
    if g.ch(chid).fighting.is_some() {
        crate::fight::stop_fighting(g, chid);
    }
    char_from_room(g, chid);
    char_to_room(g, chid, location);
    let room = g.ch(chid).in_room;
    enter_wtrigger(g, room, chid, -1);
}

pub fn do_mat(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg, rest) = crate::interpreter::one_argument(argument);
    if arg.is_empty() || crate::interpreter::skip_spaces(rest).is_empty() {
        mob_log(g, chid, "mat: bad argument");
        return;
    }
    let Some(location) = crate::act::wizard::find_target_room(g, chid, &arg) else {
        mob_log(g, chid, "mat: invalid location");
        return;
    };
    let original = g.ch(chid).in_room;
    char_from_room(g, chid);
    char_to_room(g, chid, location);
    let cmd = rest.to_vec();
    crate::interpreter::command_interpreter(g, chid, &cmd);
    if g.try_ch(chid).is_some_and(|c| c.in_room == location) {
        char_from_room(g, chid);
        char_to_room(g, chid, original);
    }
}

pub fn do_mteleport(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (arg1, arg2, _) = crate::interpreter::two_arguments(argument);
    if arg1.is_empty() || arg2.is_empty() {
        mob_log(g, chid, "mteleport: bad syntax");
        return;
    }
    let Some(target) = crate::act::wizard::find_target_room(g, chid, &arg2) else {
        mob_log(g, chid, "mteleport target is an invalid room");
        return;
    };
    if eq_ci(&arg1, b"all") {
        if target == g.ch(chid).in_room {
            mob_log(g, chid, "mteleport all target is itself");
            return;
        }
        let room = g.ch(chid).in_room;
        let people = g.rooms[room as usize].people.clone();
        for vict in people {
            if g.try_ch(vict).is_none() {
                continue;
            }
            if valid_dg_target(g, vict, DG_ALLOW_GODS) {
                char_from_room(g, vict);
                char_to_room(g, vict, target);
                let vr = g.ch(vict).in_room;
                enter_wtrigger(g, vr, vict, -1);
            }
        }
    } else {
        let Some(vict) = victim_uid_or_world(g, chid, &arg1) else {
            let msg = format!("mteleport: victim ({}) does not exist", String::from_utf8_lossy(&arg1));
            mob_log(g, chid, &msg);
            return;
        };
        if valid_dg_target(g, vict, DG_ALLOW_GODS) {
            char_from_room(g, vict);
            char_to_room(g, vict, target);
            let vr = g.ch(vict).in_room;
            enter_wtrigger(g, vr, vict, -1);
        }
    }
}

pub fn do_mdamage(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (name, amount, _) = crate::interpreter::two_arguments(argument);
    if name.is_empty() || amount.is_empty() {
        mob_log(g, chid, "mdamage: bad syntax");
        return;
    }
    let dam = atoi32(&amount);
    let Some(vict) = victim_uid_or_room(g, chid, &name) else {
        let msg = format!("mdamage: victim ({}) does not exist", String::from_utf8_lossy(&name));
        mob_log(g, chid, &msg);
        return;
    };
    script_damage(g, vict, dam);
}

pub fn do_mforce(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg, rest) = crate::interpreter::one_argument(argument);
    if arg.is_empty() || crate::interpreter::skip_spaces(rest).is_empty() {
        mob_log(g, chid, "mforce: bad syntax");
        return;
    }
    let cmd = rest.to_vec();
    if eq_ci(&arg, b"all") {
        let my_room = g.ch(chid).in_room;
        let my_level = g.ch(chid).level;
        let order = g.descriptors.order.clone();
        for di in order {
            let Some(d) = g.descriptors.get(di) else { continue };
            if d.state != ConState::Playing {
                continue;
            }
            let Some(vch) = d.character else { continue };
            if vch == chid {
                continue;
            }
            let Some(v) = g.try_ch(vch) else { continue };
            if v.in_room != my_room {
                continue;
            }
            if v.level < my_level
                && crate::handler::can_see(g, chid, vch)
                && valid_dg_target(g, vch, 0)
            {
                crate::interpreter::command_interpreter(g, vch, &cmd);
            }
        }
    } else {
        let victim = if arg.first() == Some(&UID_CHAR) {
            match get_char(g, &arg) {
                Some(v) => v,
                None => {
                    let msg = format!("mforce: victim ({}) does not exist", String::from_utf8_lossy(&arg));
                    mob_log(g, chid, &msg);
                    return;
                }
            }
        } else {
            match get_char_room_vis(g, chid, &arg, None) {
                Some(v) => v,
                None => {
                    mob_log(g, chid, "mforce: no such victim");
                    return;
                }
            }
        };
        if victim == chid {
            mob_log(g, chid, "mforce: forcing self");
            return;
        }
        if valid_dg_target(g, victim, 0) {
            crate::interpreter::command_interpreter(g, victim, &cmd);
        }
    }
}

pub fn do_mhunt(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mhunt called with no argument");
        return;
    }
    if g.ch(chid).fighting.is_some() {
        return;
    }
    let Some(victim) = victim_uid_or_world(g, chid, &arg) else {
        let msg = format!("mhunt: victim ({}) does not exist", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    g.ch_mut(chid).hunting = Some(victim);
}

pub fn do_mremember(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg, rest) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mremember: bad syntax");
        return;
    }
    let Some(victim) = victim_uid_or_world(g, chid, &arg) else {
        let msg = format!("mremember: victim ({}) does not exist", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    let id = char_script_id(g, victim);
    let cmd = if rest.is_empty() { None } else { Some(rest.to_vec()) };
    g.ch_mut(chid).script_mem.push(ScriptMem { id, cmd });
}

pub fn do_mforget(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if impl_guard(g, chid) {
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mforget: bad syntax");
        return;
    }
    let Some(victim) = victim_uid_or_world(g, chid, &arg) else {
        let msg = format!("mforget: victim ({}) does not exist", String::from_utf8_lossy(&arg));
        mob_log(g, chid, &msg);
        return;
    };
    let id = char_script_id(g, victim);
    g.ch_mut(chid).script_mem.retain(|m| m.id != id);
}

pub fn do_mtransform(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    if g.ch(chid).desc.is_some() {
        crate::comm::send_to_char(g, chid, b"You've got no VNUM to return to, dummy! try 'switch'\r\n");
        return;
    }
    let (arg, _) = crate::interpreter::one_argument(argument);
    if arg.is_empty() {
        mob_log(g, chid, "mtransform: missing argument");
        return;
    }
    if !arg[0].is_ascii_digit() && arg[0] != b'-' {
        mob_log(g, chid, "mtransform: bad argument");
        return;
    }
    let keep_hp = arg[0].is_ascii_digit();
    let vnum = if keep_hp { atoi32(&arg) } else { atoi32(&arg[1..]) };

    let m_rnum = g.world.real_mobile(vnum as Idx);
    let m = m_rnum.and_then(|r| crate::db::read_mobile(g, r));
    let Some(m) = m else {
        mob_log(g, chid, "mtransform: bad mobile vnum");
        return;
    };

    // Unequip everything first.
    let mut eq: [Option<mud_data::ids::ObjId>; NUM_WEARS] = [None; NUM_WEARS];
    for pos in 0..NUM_WEARS {
        if g.ch(chid).equipment[pos].is_some() {
            eq[pos] = unequip_char(g, chid, pos);
        }
    }

    let this_rnum = g.ch(chid).mob_rnum;
    let room = g.ch(chid).in_room;
    char_to_room(g, m, room);

    // Copy m over ch, preserving the fields that must survive.
    let new_body = g.ch(m).clone();
    {
        let old = g.ch(chid);
        let keep_script_id = old.script_id;
        let keep_affected = old.affected.clone();
        let keep_carrying = old.carrying.clone();
        let keep_proto_script = old.proto_script.clone();
        let keep_script = old.script.clone();
        let keep_mem = old.script_mem.clone();
        let keep_followers = old.followers.clone();
        let keep_master = old.master;
        let keep_group = old.group;
        let keep_was_in = old.was_in_room;
        let keep_hit = old.points.hit;
        let keep_max_hit = old.points.max_hit;
        let keep_exp = old.points.exp;
        let keep_gold = old.points.gold;
        let keep_pos = old.position;
        let keep_carry_w = old.carry_weight;
        let keep_carry_n = old.carry_items;
        let keep_fighting = old.fighting;
        let keep_hunting = old.hunting;
        let keep_in_room = old.in_room;
        let keep_desc = old.desc;

        let ch = g.ch_mut(chid);
        *ch = new_body;
        ch.script_id = keep_script_id;
        ch.affected = keep_affected;
        ch.carrying = keep_carrying;
        ch.proto_script = keep_proto_script;
        ch.script = keep_script;
        ch.script_mem = keep_mem;
        ch.followers = keep_followers;
        ch.master = keep_master;
        ch.group = keep_group;
        ch.was_in_room = keep_was_in;
        if keep_hp {
            ch.points.hit = keep_hit;
            ch.points.max_hit = keep_max_hit;
            ch.points.exp = keep_exp;
        }
        ch.points.gold = keep_gold;
        ch.position = keep_pos;
        ch.carry_weight = keep_carry_w;
        ch.carry_items = keep_carry_n;
        ch.fighting = keep_fighting;
        ch.hunting = keep_hunting;
        ch.in_room = keep_in_room;
        ch.desc = keep_desc;
    }

    for (pos, slot) in eq.iter().enumerate() {
        if let Some(o) = slot {
            equip_char(g, chid, *o, pos);
        }
    }

    // ch->nr restored: %self.vnum% keeps reporting the original.
    g.ch_mut(chid).mob_rnum = this_rnum;
    extract_char(g, m);
}

pub fn do_mdoor(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    super::objcmd::door_command(g, argument, "mdoor", |g, msg| mob_log(g, chid, msg));
}

pub fn do_mfollow(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    if charmed(g, chid) {
        return;
    }
    let (buf, _) = crate::interpreter::one_argument(argument);
    if buf.is_empty() {
        mob_log(g, chid, "mfollow: bad syntax");
        return;
    }
    let leader = if buf.first() == Some(&UID_CHAR) {
        match get_char(g, &buf) {
            Some(l) => l,
            None => {
                let msg = format!("mfollow: victim ({}) does not exist", String::from_utf8_lossy(&buf));
                mob_log(g, chid, &msg);
                return;
            }
        }
    } else {
        match get_char_room_vis(g, chid, &buf, None) {
            Some(l) => l,
            None => {
                let msg = format!("mfollow: victim ({}) not found", String::from_utf8_lossy(&buf));
                mob_log(g, chid, &msg);
                return;
            }
        }
    };

    if g.ch(chid).master == Some(leader) {
        return;
    }
    if g.ch(chid).aff(flags::AFF_CHARM) && g.ch(chid).master.is_some() {
        return;
    }
    // Silently drop the old master link.
    if let Some(master) = g.ch(chid).master {
        if let Some(mc) = g.chars.get_mut(master) {
            mc.followers.retain(|&f| f != chid);
        }
        g.ch_mut(chid).master = None;
    }
    if chid == leader {
        return;
    }
    if crate::act::movement::circle_follow(g, chid, leader) {
        mob_log(g, chid, "mfollow: Following in circles.");
        return;
    }
    g.ch_mut(chid).master = Some(leader);
    g.ch_mut(leader).followers.insert(0, chid);
}

pub fn do_mrecho(g: &mut Game, chid: CharId, argument: &[u8]) {
    if !mob_or_impl(g, chid) {
        huh(g, chid);
        return;
    }
    let (start, finish, rest) = crate::interpreter::two_arguments(argument);
    let msg = crate::interpreter::skip_spaces(rest);
    if msg.is_empty()
        || start.is_empty()
        || finish.is_empty()
        || !crate::interpreter::is_number(&start)
        || !crate::interpreter::is_number(&finish)
    {
        mob_log(g, chid, "mrecho called with too few args");
    } else {
        let mut buf = msg.to_vec();
        buf.extend_from_slice(b"\r\n");
        send_to_range(g, atoi32(&start), atoi32(&finish), &buf);
    }
}

type MobCmd = fn(&mut Game, CharId, &[u8]);

/// mob_script_commands — exact full-word matches.
const MOB_SCRIPT_COMMANDS: [(&[u8], MobCmd); 22] = [
    (b"masound", do_masound),
    (b"mkill", do_mkill),
    (b"mjunk", do_mjunk),
    (b"mdamage", do_mdamage),
    (b"mdoor", do_mdoor),
    (b"mecho", do_mecho),
    (b"mrecho", do_mrecho),
    (b"mechoaround", do_mechoaround),
    (b"msend", do_msend),
    (b"mload", do_mload),
    (b"mpurge", do_mpurge),
    (b"mgoto", do_mgoto),
    (b"mat", do_mat),
    (b"mteleport", do_mteleport),
    (b"mforce", do_mforce),
    (b"mhunt", do_mhunt),
    (b"mremember", do_mremember),
    (b"mforget", do_mforget),
    (b"mtransform", do_mtransform),
    (b"mzoneecho", do_mzoneecho),
    (b"mfollow", do_mfollow),
    (b"mlog", do_mlog),
];

/// script_command_interpreter: full-word match, then run.
pub fn script_command_interpreter(g: &mut Game, chid: CharId, arg: &[u8]) -> bool {
    let arg = crate::interpreter::skip_spaces(arg);
    if arg.is_empty() {
        return false;
    }
    let (first_arg, line) = crate::interpreter::any_one_arg(arg);
    for (name, func) in MOB_SCRIPT_COMMANDS {
        if eq_ci(&first_arg, name) {
            let line = line.to_vec();
            func(g, chid, &line);
            return true;
        }
    }
    false
}
