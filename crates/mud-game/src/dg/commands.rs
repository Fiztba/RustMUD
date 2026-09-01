//! Imm-level script commands: attach, detach, vdelete, tstat, and the
//! script_stat helpers used by stat (stage 8 wires the rest of stat).

use mud_data::ids::CharId;
use mud_data::tables::{OTRIG_TYPES, TRIG_TYPES, WTRIG_TYPES};
use mud_data::types::*;

use super::{
    atoi32, atoi64, find_char, find_obj, find_room, get_char, get_obj, read_trigger,
    remove_trigger, GoId, MOB_TRIGGER, OBJ_TRIGGER, WLD_TRIGGER,
};
use crate::comm::send_to_char;
use crate::game::Game;
use crate::handler::{is_abbrev, isname, eq_ci};
use crate::interpreter::{one_argument, two_arguments};

pub type BStr = Vec<u8>;

pub fn can_edit_zone(g: &Game, chid: CharId, rnum: Option<usize>) -> bool {
    const ALL_PERMISSION: i32 = 666;
    const HEDIT_PERMISSION: i32 = 888;
    const AEDIT_PERMISSION: i32 = 999;
    let ch = g.ch(chid);
    let Some(rnum) = rnum else { return false };
    if ch.desc.is_none() || ch.is_npc() {
        return false;
    }
    let zflag_nobuild = g.world.zones[rnum].zone_flags[0] & (1 << 4) != 0;
    if zflag_nobuild {
        return false;
    }
    let olc_zone = ch.player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    if olc_zone == ALL_PERMISSION {
        return true;
    }
    let _ = (HEDIT_PERMISSION, AEDIT_PERMISSION);
    if ch.level >= LVL_GRGOD {
        return true;
    }
    if let Some(builders) = g.world.zones[rnum].builders.as_deref() {
        if isname(ch.get_name(), builders) {
            return true;
        }
    }
    if olc_zone == NOWHERE as i32 {
        return false;
    }
    if (ch.level as i32) < LVL_BUILDER as i32 {
        return false;
    }
    g.world.zones.get(rnum).is_some_and(|z| z.number as i32 == olc_zone)
}

fn zone_of_room(g: &Game, room: RoomRnum) -> Option<usize> {
    if room == NOWHERE {
        return None;
    }
    Some(g.world.rooms[room as usize].zone as usize)
}

/// real_zone_by_thing wrapper for entity vnums.
fn real_zone_by_thing(g: &Game, vnum: i32) -> Option<usize> {
    super::mobcmd::real_zone_by_thing(g, vnum)
}

pub fn do_attach(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, trig_name, rest) = two_arguments(argument);
    let (targ_name, loc_name, _) = two_arguments(rest);

    if arg.is_empty() || targ_name.is_empty() || trig_name.is_empty() {
        send_to_char(g, chid, b"Usage: attach { mob | obj | room } { trigger } { name } [ location ]\r\n");
        return;
    }
    let num_arg = atoi32(&targ_name);
    let tn = atoi32(&trig_name);
    let loc = if loc_name.is_empty() { -1 } else { atoi32(&loc_name) };

    if is_abbrev(&arg, b"mobile") || is_abbrev(&arg, b"mtr") {
        let victim = crate::handler::get_char_world_vis(g, chid, &targ_name, None).or_else(|| {
            let room = g.ch(chid).in_room;
            g.rooms[room as usize]
                .people
                .clone()
                .into_iter()
                .find(|&v| super::mob_vnum(g, v) == num_arg)
        });
        let Some(victim) = victim else {
            send_to_char(g, chid, b"That mob does not exist.\r\n");
            return;
        };
        if !g.ch(victim).is_npc() && !g.config.script_players {
            send_to_char(g, chid, b"Players can't have scripts.\r\n");
            return;
        }
        let my_zone = zone_of_room(g, g.ch(chid).in_room);
        if !can_edit_zone(g, chid, my_zone) {
            send_to_char(g, chid, b"You can only attach triggers in your own zone.\r\n");
            return;
        }
        let rn = g.world.trig_map.get(&(tn as Idx)).copied();
        let trig = rn.and_then(|r| read_trigger(g, r));
        let Some(trig) = trig else {
            send_to_char(g, chid, b"That trigger does not exist.\r\n");
            return;
        };
        let tname = trig.name.clone();
        super::add_trigger_at(g.ensure_script(GoId::Char(victim)), trig, loc);
        let msg = if g.ch(victim).is_npc() {
            format!(
                "Trigger {} ({}) attached to {} [{}].\r\n",
                tn,
                String::from_utf8_lossy(&tname),
                String::from_utf8_lossy(g.ch(victim).short_descr.as_deref().unwrap_or(b"")),
                super::mob_vnum(g, victim)
            )
        } else {
            format!(
                "Trigger {} ({}) attached to player named {}.\r\n",
                tn,
                String::from_utf8_lossy(&tname),
                String::from_utf8_lossy(g.ch(victim).get_name())
            )
        };
        send_to_char(g, chid, msg.as_bytes());
    } else if is_abbrev(&arg, b"object") || is_abbrev(&arg, b"otr") {
        let object = obj_vis_of(g, chid, &targ_name)
            .or_else(|| {
                let room = g.ch(chid).in_room;
                g.rooms[room as usize]
                    .contents
                    .clone()
                    .into_iter()
                    .find(|&o| super::obj_vnum(g, o) == num_arg)
            })
            .or_else(|| {
                g.ch(chid)
                    .carrying
                    .clone()
                    .into_iter()
                    .find(|&o| super::obj_vnum(g, o) == num_arg)
            });
        let Some(object) = object else {
            send_to_char(g, chid, b"That object does not exist.\r\n");
            return;
        };
        let my_zone = zone_of_room(g, g.ch(chid).in_room);
        if !can_edit_zone(g, chid, my_zone) {
            send_to_char(g, chid, b"You can only attach triggers in your own zone.\r\n");
            return;
        }
        let rn = g.world.trig_map.get(&(tn as Idx)).copied();
        let trig = rn.and_then(|r| read_trigger(g, r));
        let Some(trig) = trig else {
            send_to_char(g, chid, b"That trigger does not exist.\r\n");
            return;
        };
        let tname = trig.name.clone();
        super::add_trigger_at(g.ensure_script(GoId::Obj(object)), trig, loc);
        let short = crate::handler::obj_short(g, object).to_vec();
        let disp = if short.is_empty() {
            crate::handler::obj_name(g, object).to_vec()
        } else {
            short
        };
        let msg = format!(
            "Trigger {} ({}) attached to {} [{}].\r\n",
            tn,
            String::from_utf8_lossy(&tname),
            String::from_utf8_lossy(&disp),
            super::obj_vnum(g, object)
        );
        send_to_char(g, chid, msg.as_bytes());
    } else if is_abbrev(&arg, b"room") || is_abbrev(&arg, b"wtr") {
        let rnum = if targ_name.contains(&b'.') {
            Some(g.ch(chid).in_room)
        } else if targ_name.first().is_some_and(|b| b.is_ascii_digit()) {
            crate::act::wizard::find_target_room(g, chid, &targ_name)
        } else {
            None
        };
        let Some(rnum) = rnum.filter(|&r| r != NOWHERE) else {
            send_to_char(g, chid, b"You need to supply a room number or . for current room.\r\n");
            return;
        };
        let zone = zone_of_room(g, rnum);
        if !can_edit_zone(g, chid, zone) {
            send_to_char(g, chid, b"You can only attach triggers in your own zone.\r\n");
            return;
        }
        let rn = g.world.trig_map.get(&(tn as Idx)).copied();
        let trig = rn.and_then(|r| read_trigger(g, r));
        let Some(trig) = trig else {
            send_to_char(g, chid, b"That trigger does not exist.\r\n");
            return;
        };
        let tname = trig.name.clone();
        super::add_trigger_at(g.ensure_script(GoId::Room(rnum)), trig, loc);
        let msg = format!(
            "Trigger {} ({}) attached to room {}.\r\n",
            tn,
            String::from_utf8_lossy(&tname),
            g.world.rooms[rnum as usize].vnum
        );
        send_to_char(g, chid, msg.as_bytes());
    } else {
        send_to_char(g, chid, b"Please specify 'mob', 'obj', or 'room'.\r\n");
    }
}

fn obj_vis_of(g: &Game, chid: CharId, name: &[u8]) -> Option<mud_data::ids::ObjId> {
    let (mut num, stripped) = crate::handler::get_number(name);
    if num == 0 {
        return None;
    }
    crate::handler::get_obj_vis_counted(g, chid, &stripped, &mut num)
}

pub fn do_detach(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2, rest) = two_arguments(argument);
    let (arg3, _) = one_argument(rest);
    let tn = atoi32(&arg3);
    // The instance is read up front purely for its name; a bad vnum leaves
    // it unset and the later %s prints "(null)".
    let trig_name: BStr = g
        .world
        .trig_map
        .get(&(tn as Idx))
        .and_then(|&rn| g.world.triggers.get(rn as usize))
        .and_then(|t| t.name.clone())
        .unwrap_or_else(|| b"(null)".to_vec());

    if arg1.is_empty() || arg2.is_empty() {
        send_to_char(g, chid, b"Usage: detach [ mob | object | room ] { target } { trigger | 'all' }\r\n");
        return;
    }
    let num_arg = atoi32(&arg2);

    if eq_ci(&arg1, b"room") || eq_ci(&arg1, b"wtr") {
        let rnum = if arg3.is_empty() || arg2.contains(&b'.') {
            Some(g.ch(chid).in_room)
        } else if arg2.first().is_some_and(|b| b.is_ascii_digit()) {
            crate::act::wizard::find_target_room(g, chid, &arg2)
        } else {
            None
        };
        let Some(rnum) = rnum.filter(|&r| r != NOWHERE) else {
            send_to_char(g, chid, b"That's not a valid room.\r\n");
            return;
        };
        let zone = zone_of_room(g, rnum);
        if !can_edit_zone(g, chid, zone) {
            send_to_char(g, chid, b"You can only detach triggers in your own zone\r\n");
            return;
        }
        if g.script_of(GoId::Room(rnum)).is_none() {
            send_to_char(g, chid, b"This room does not have any triggers.\r\n");
        } else if eq_ci(&arg2, b"all") || eq_ci(&arg3, b"all") {
            super::extract_script(g, GoId::Room(rnum));
            let msg = format!("All triggers removed from room {}.\r\n", g.world.rooms[rnum as usize].vnum);
            send_to_char(g, chid, msg.as_bytes());
        } else {
            let snum = if !arg3.is_empty() { &arg3 } else { &arg2 };
            if remove_trigger(g, GoId::Room(rnum), snum) {
                let msg = format!(
                    "Trigger {} ({}) removed from {}.\r\n",
                    tn,
                    String::from_utf8_lossy(&trig_name),
                    g.world.rooms[rnum as usize].vnum
                );
                send_to_char(g, chid, msg.as_bytes());
                if g.script_of(GoId::Room(rnum)).is_some_and(|sc| sc.trig_list.is_empty()) {
                    super::extract_script(g, GoId::Room(rnum));
                }
            } else {
                send_to_char(g, chid, b"That trigger was not found.\r\n");
            }
        }
        return;
    }

    let mut victim: Option<CharId> = None;
    let mut object: Option<mud_data::ids::ObjId> = None;
    let mut trigger: BStr = Vec::new();

    if is_abbrev(&arg1, b"mobile") || eq_ci(&arg1, b"mtr") {
        victim = crate::handler::get_char_world_vis(g, chid, &arg2, None).or_else(|| {
            let room = g.ch(chid).in_room;
            g.rooms[room as usize]
                .people
                .clone()
                .into_iter()
                .find(|&v| super::mob_vnum(g, v) == num_arg)
        });
        if victim.is_none() {
            send_to_char(g, chid, b"No such mobile around.\r\n");
            return;
        }
        if arg3.is_empty() {
            send_to_char(g, chid, b"You must specify a trigger to remove.\r\n");
        } else {
            trigger = arg3.clone();
        }
    } else if is_abbrev(&arg1, b"object") || eq_ci(&arg1, b"otr") {
        object = obj_vis_of(g, chid, &arg2)
            .or_else(|| {
                let room = g.ch(chid).in_room;
                g.rooms[room as usize]
                    .contents
                    .clone()
                    .into_iter()
                    .find(|&o| super::obj_vnum(g, o) == num_arg)
            })
            .or_else(|| {
                g.ch(chid)
                    .carrying
                    .clone()
                    .into_iter()
                    .find(|&o| super::obj_vnum(g, o) == num_arg)
            });
        if object.is_none() {
            send_to_char(g, chid, b"No such object around.\r\n");
            return;
        }
        if arg3.is_empty() {
            send_to_char(g, chid, b"You must specify a trigger to remove.\r\n");
        } else {
            trigger = arg3.clone();
        }
    } else {
        // Guessing form: eq, inventory, char in room, obj in room, char in
        // world, obj in world.
        let eq = super::get_object_in_equip(g, chid, &arg1);
        if let Some(o) = eq {
            object = Some(o);
        } else if let Some(o) = {
            let carrying = g.ch(chid).carrying.clone();
            crate::handler::get_obj_in_list_vis(g, chid, &arg1, None, &carrying)
        } {
            object = Some(o);
        } else if let Some(v) = crate::handler::get_char_room_vis(g, chid, &arg1, None) {
            victim = Some(v);
        } else if let Some(o) = {
            let room = g.ch(chid).in_room;
            let contents = g.rooms[room as usize].contents.clone();
            crate::handler::get_obj_in_list_vis(g, chid, &arg1, None, &contents)
        } {
            object = Some(o);
        } else if let Some(v) = crate::handler::get_char_world_vis(g, chid, &arg1, None) {
            victim = Some(v);
        } else if let Some(o) = obj_vis_of(g, chid, &arg1) {
            object = Some(o);
        } else {
            send_to_char(g, chid, b"Nothing around by that name.\r\n");
        }
        trigger = arg2.clone();
    }

    if let Some(victim) = victim {
        if g.script_of(GoId::Char(victim)).is_none() {
            let msg = format!(
                "That {} doesn't have any triggers.\r\n",
                if g.ch(victim).is_npc() { "mob" } else { "player" }
            );
            send_to_char(g, chid, msg.as_bytes());
        } else if g.ch(victim).is_npc()
            && !can_edit_zone(g, chid, real_zone_by_thing(g, super::mob_vnum(g, victim)))
        {
            send_to_char(g, chid, b"You can only detach triggers in your own zone\r\n");
        } else if !trigger.is_empty() && eq_ci(&trigger, b"all") {
            super::extract_script(g, GoId::Char(victim));
            let msg = format!(
                "All triggers removed from {}.\r\n",
                String::from_utf8_lossy(g.ch(victim).get_name())
            );
            send_to_char(g, chid, msg.as_bytes());
        } else if !trigger.is_empty() && remove_trigger(g, GoId::Char(victim), &trigger) {
            let msg = format!(
                "Trigger {} ({}) removed from {}.\r\n",
                tn,
                String::from_utf8_lossy(&trig_name),
                String::from_utf8_lossy(g.ch(victim).get_name())
            );
            send_to_char(g, chid, msg.as_bytes());
            if g.script_of(GoId::Char(victim)).is_some_and(|sc| sc.trig_list.is_empty()) {
                super::extract_script(g, GoId::Char(victim));
            }
        } else {
            send_to_char(g, chid, b"That trigger was not found.\r\n");
        }
    } else if let Some(object) = object {
        if g.script_of(GoId::Obj(object)).is_none() {
            send_to_char(g, chid, b"That object doesn't have any triggers.\r\n");
        } else if !can_edit_zone(g, chid, real_zone_by_thing(g, super::obj_vnum(g, object))) {
            send_to_char(g, chid, b"You can only detach triggers in your own zone\r\n");
        } else if !trigger.is_empty() && eq_ci(&trigger, b"all") {
            super::extract_script(g, GoId::Obj(object));
            let disp = obj_display(g, object);
            let msg = format!("All triggers removed from {}.\r\n", String::from_utf8_lossy(&disp));
            send_to_char(g, chid, msg.as_bytes());
        } else if remove_trigger(g, GoId::Obj(object), &trigger) {
            let disp = obj_display(g, object);
            let msg = format!(
                "Trigger {} ({}) removed from {}.\r\n",
                tn,
                String::from_utf8_lossy(&trig_name),
                String::from_utf8_lossy(&disp)
            );
            send_to_char(g, chid, msg.as_bytes());
            if g.script_of(GoId::Obj(object)).is_some_and(|sc| sc.trig_list.is_empty()) {
                super::extract_script(g, GoId::Obj(object));
            }
        } else {
            send_to_char(g, chid, b"That trigger was not found.\r\n");
        }
    }
}

fn obj_display(g: &Game, oid: mud_data::ids::ObjId) -> BStr {
    let short = crate::handler::obj_short(g, oid);
    if short.is_empty() {
        crate::handler::obj_name(g, oid).to_vec()
    } else {
        short.to_vec()
    }
}

pub fn do_vdelete(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, buf2, _) = two_arguments(argument);
    if buf.is_empty() || buf2.is_empty() {
        send_to_char(g, chid, b"Usage: vdelete { <variablename> | * | all } <id>\r\n");
        return;
    }
    let uid = atoi64(&buf2);
    if uid <= 0 {
        send_to_char(g, chid, b"vdelete: illegal id specified.\r\n");
        return;
    }
    let target = if let Some(r) = find_room(g, uid) {
        Some(GoId::Room(r))
    } else if let Some(c) = find_char(g, uid) {
        Some(GoId::Char(c))
    } else {
        find_obj(g, uid).map(GoId::Obj)
    };
    let Some(target) = target else {
        send_to_char(g, chid, b"vdelete: cannot resolve specified id.\r\n");
        return;
    };
    let Some(sc) = g.script_of_mut(target) else {
        send_to_char(g, chid, b"That id represents no global variables.(1)\r\n");
        return;
    };
    if sc.global_vars.is_empty() {
        send_to_char(g, chid, b"That id represents no global variables.(2)\r\n");
        return;
    }
    if buf.first() == Some(&b'*') || is_abbrev(&buf, b"all") {
        sc.global_vars.clear();
        send_to_char(g, chid, b"All variables deleted from that id.\r\n");
        return;
    }
    if let Some(pos) = sc.global_vars.iter().position(|v| eq_ci(&v.name, &buf)) {
        sc.global_vars.remove(pos);
        send_to_char(g, chid, b"Deleted.\r\n");
    } else {
        send_to_char(g, chid, b"That variable cannot be located.\r\n");
    }
}

/// perform_set_dg_var — used by do_set (stage 8).
pub fn perform_set_dg_var(g: &mut Game, chid: CharId, vict: CharId, val_arg: &[u8]) -> bool {
    let (var_name, rest) = crate::interpreter::any_one_arg(val_arg);
    let var_value = crate::interpreter::skip_spaces(rest);
    if var_name.is_empty() || var_value.is_empty() {
        send_to_char(g, chid, b"Usage: set <char> <varname> <value>\r\n");
        return false;
    }
    let value = var_value.to_vec();
    let sc = g.ensure_script(GoId::Char(vict));
    super::add_var(&mut sc.global_vars, &var_name, &value, 0);
    true
}

pub fn do_tstat(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (str_, _) = crate::interpreter::half_chop(argument);
    if str_.is_empty() {
        send_to_char(g, chid, b"Usage: tstat <vnum>\r\n");
        return;
    }
    let vnum = atoi32(&str_);
    let Some(&rnum) = g.world.trig_map.get(&(vnum as Idx)) else {
        send_to_char(g, chid, b"That vnum does not exist.\r\n");
        return;
    };
    do_stat_trigger(g, chid, rnum);
}

/// do_stat_trigger on the prototype.
pub fn do_stat_trigger(g: &mut Game, chid: CharId, rnum: Idx) {
    let proto = g.world.triggers[rnum as usize].clone();
    let yel = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KYEL);
    let grn = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KGRN);
    let nrm = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM);

    let mut sb: BStr = Vec::new();
    sb.extend_from_slice(
        format!(
            "Name: '{}{}{}',  VNum: [{}{:5}{}], RNum: [{:5}]\r\n",
            String::from_utf8_lossy(yel),
            String::from_utf8_lossy(proto.name.as_deref().unwrap_or(b"")),
            String::from_utf8_lossy(nrm),
            String::from_utf8_lossy(grn),
            proto.vnum,
            String::from_utf8_lossy(nrm),
            rnum
        )
        .as_bytes(),
    );
    let (label, table): (&str, &[&str]) = match proto.attach_type {
        OBJ_TRIGGER => ("Objects", &OTRIG_TYPES),
        WLD_TRIGGER => ("Rooms", &WTRIG_TYPES),
        _ => ("Mobiles", &TRIG_TYPES),
    };
    let _ = MOB_TRIGGER;
    sb.extend_from_slice(format!("Trigger Intended Assignment: {}\r\n", label).as_bytes());
    let typebuf = sprintbit_str(proto.trigger_type, table);
    sb.extend_from_slice(
        format!(
            "Trigger Type: {}, Numeric Arg: {}, Arg list: {}\r\n",
            typebuf,
            proto.narg,
            match proto.arglist.as_deref() {
                Some(a) if !a.is_empty() => String::from_utf8_lossy(a).into_owned(),
                _ => "None".into(),
            }
        )
        .as_bytes(),
    );
    sb.extend_from_slice(b"Commands:\r\n");
    for line in &proto.cmdlist {
        sb.extend_from_slice(line);
        sb.extend_from_slice(b"\r\n");
        if sb.len() > 16384 - 80 {
            sb.extend_from_slice(b"*** Overflow - script too long! ***\r\n");
            break;
        }
    }
    crate::act::informative::page_string(g, chid, &sb);
}

/// sprintbit into a String: trailing space per flag, "NOBITS " when
/// empty.
fn sprintbit_str(bits: u32, names: &[&str]) -> String {
    let mut out = String::new();
    let mut any = false;
    for i in 0..32 {
        if bits & (1 << i) != 0 {
            any = true;
            out.push_str(names.get(i as usize).copied().unwrap_or("UNDEFINED"));
            out.push(' ');
        }
    }
    if !any {
        out.push_str("NOBITS ");
    }
    out
}

pub fn find_uid_name(g: &mut Game, uid: &[u8]) -> BStr {
    if let Some(ch) = get_char(g, uid) {
        g.ch(ch).name.clone().unwrap_or_default()
    } else if let Some(obj) = get_obj(g, uid) {
        crate::handler::obj_name(g, obj).to_vec()
    } else {
        let mut v = b"uid = ".to_vec();
        v.extend_from_slice(&uid[1.min(uid.len())..]);
        v.extend_from_slice(b", (not found)");
        v
    }
}

/// script_stat — shown by stat/sstat for any scripted
/// entity.
pub fn script_stat(g: &mut Game, chid: CharId, go: GoId) {
    let Some(sc) = g.script_of(go) else { return };
    let global_vars = sc.global_vars.clone();
    let context = sc.context;
    let trigs: Vec<super::TrigInstance> = sc.trig_list.clone();

    let empty_note = if global_vars.is_empty() { "None" } else { "" };
    send_to_char(g, chid, format!("Global Variables: {}\r\n", empty_note).as_bytes());
    send_to_char(g, chid, format!("Global context: {}\r\n", context).as_bytes());

    for tv in &global_vars {
        let shown_name = if tv.context != 0 {
            format!("{}:{}", String::from_utf8_lossy(&tv.name), tv.context)
        } else {
            String::from_utf8_lossy(&tv.name).into_owned()
        };
        let value = if tv.value.first() == Some(&super::UID_CHAR) {
            find_uid_name(g, &tv.value)
        } else {
            tv.value.clone()
        };
        send_to_char(
            g,
            chid,
            format!("    {:>15}:  {}\r\n", shown_name, String::from_utf8_lossy(&value)).as_bytes(),
        );
    }

    for t in &trigs {
        let yel = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KYEL).to_vec();
        let grn = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KGRN).to_vec();
        let nrm = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM).to_vec();
        let vnum = g.world.triggers[t.nr as usize].vnum;
        send_to_char(
            g,
            chid,
            format!(
                "\r\n  Trigger: {}{}{}, VNum: [{}{:5}{}], RNum: [{:5}]\r\n",
                String::from_utf8_lossy(&yel),
                String::from_utf8_lossy(&t.name),
                String::from_utf8_lossy(&nrm),
                String::from_utf8_lossy(&grn),
                vnum,
                String::from_utf8_lossy(&nrm),
                t.nr
            )
            .as_bytes(),
        );
        let (label, table): (&str, &[&str]) = match t.attach_type {
            OBJ_TRIGGER => ("Objects", &OTRIG_TYPES),
            WLD_TRIGGER => ("Rooms", &WTRIG_TYPES),
            _ => ("Mobiles", &TRIG_TYPES),
        };
        send_to_char(g, chid, format!("  Trigger Intended Assignment: {}\r\n", label).as_bytes());
        send_to_char(
            g,
            chid,
            format!(
                "  Trigger Type: {}, Numeric Arg: {}, Arg list: {}\r\n",
                sprintbit_str(t.trigger_type, table),
                t.narg,
                if t.arglist.is_empty() {
                    "None".into()
                } else {
                    String::from_utf8_lossy(&t.arglist).into_owned()
                }
            )
            .as_bytes(),
        );
        if let Some(ev) = t.wait_event {
            let remaining = g
                .events
                .iter()
                .find(|e| {
                    matches!(e.kind, crate::game::EventKind::TrigWait { event_id, .. } if event_id == ev)
                })
                .map(|e| e.fire_at.saturating_sub(g.pulse))
                .unwrap_or(0);
            let cur_line = g
                .world
                .triggers
                .get(t.nr as usize)
                .and_then(|p| p.cmdlist.get(t.curr_state))
                .map(|l| String::from_utf8_lossy(l).into_owned())
                .unwrap_or_else(|| "End of Script".into());
            send_to_char(
                g,
                chid,
                format!("    Wait: {}, Current line: {}\r\n", remaining, cur_line).as_bytes(),
            );
            let note = if t.var_list.is_empty() { "None" } else { "" };
            send_to_char(g, chid, format!("  Variables: {}\r\n", note).as_bytes());
            for tv in &t.var_list {
                let value = if tv.value.first() == Some(&super::UID_CHAR) {
                    find_uid_name(g, &tv.value)
                } else {
                    tv.value.clone()
                };
                send_to_char(
                    g,
                    chid,
                    format!(
                        "    {:>15}:  {}\r\n",
                        String::from_utf8_lossy(&tv.name),
                        String::from_utf8_lossy(&value)
                    )
                    .as_bytes(),
                );
            }
        }
    }
}
