//! get/put/drop/give, wear/wield/remove, eat/drink/pour, and sacrifice.
//! Message text, check order, and RNG draw order are all observable; DG
//! trigger hooks are stage 6 and noted inline.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::tables;
use mud_data::types::*;

use crate::ch::{Affect, DRUNK, HUNGER, THIRST};
use crate::comm::{self, act, send_to_char};
use crate::game::Game;
use crate::handler::{
    self, can_carry_n, can_carry_w, can_see_obj, extract_obj, generic_find, get_number,
    get_obj_in_list_vis, get_obj_pos_in_equip_vis, isname, money_desc, obj_from_char,
    obj_from_obj, obj_from_room, obj_name, obj_short, obj_to_char, obj_to_obj, obj_to_room,
    obj_weight, FIND_OBJ_INV, FIND_OBJ_ROOM,
};
use crate::interpreter::{
    is_number, one_argument, skip_spaces, two_arguments, SCMD_DONATE, SCMD_DRINK, SCMD_DROP,
    SCMD_EAT, SCMD_JUNK, SCMD_SIP, SCMD_TASTE,
};
use crate::limits::{gain_condition, decrease_gold, increase_gold};

// find_all_dots modes.
pub const FIND_INDIV: i32 = 0;
pub const FIND_ALL: i32 = 1;
pub const FIND_ALLDOT: i32 = 2;

/// find_all_dots: strips "all."/"all" and reports the mode.
pub fn find_all_dots(arg: &[u8]) -> (i32, Vec<u8>) {
    if arg == b"all" {
        (FIND_ALL, arg.to_vec())
    } else if arg.starts_with(b"all.") {
        (FIND_ALLDOT, arg[4..].to_vec())
    } else {
        (FIND_INDIV, arg.to_vec())
    }
}

/// AN: "an" before a vowel.
pub fn an(word: &[u8]) -> &'static [u8] {
    match word.first() {
        Some(c) if b"aeiouAEIOU".contains(c) => b"an",
        _ => b"a",
    }
}

fn atoi(b: &[u8]) -> i32 {
    handler::atoi(b)
}

// ---- put ----

fn perform_put(g: &mut Game, chid: CharId, oid: ObjId, cont: ObjId) {
    let object_id = crate::dg::obj_script_id(g, oid);
    if crate::dg::triggers::drop_otrigger(g, oid, chid) == 0 {
        return;
    }
    // Object might be extracted by drop_otrigger.
    if !crate::dg::has_obj_by_uid_in_lookup_table(g, object_id) || g.try_obj(oid).is_none() {
        return;
    }
    let cont_v0 = g.obj(cont).values[0];
    // Corpses (val0 == 0) refuse puts: the val0 gate applies to them, and
    // any corpse outweighs a capacity of 0.
    let gated = cont_v0 > 0 || crate::handler::is_corpse(g, cont);
    if gated && obj_weight(g, cont) + obj_weight(g, oid) > cont_v0 {
        comm::act_full(g, b"$p won't fit in $P.", false, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_CHAR);
    } else if g.obj(oid).obj_flagged(flags::ITEM_NODROP) && g.obj(cont).in_room != NOWHERE {
        act(g, b"You can't get $p out of your hand.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
    } else {
        obj_from_char(g, oid);
        obj_to_obj(g, oid, cont);

        comm::act_full(g, b"$n puts $p in $P.", true, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_ROOM);

        // NODROP contagion.
        if g.obj(oid).obj_flagged(flags::ITEM_NODROP) && !g.obj(cont).obj_flagged(flags::ITEM_NODROP) {
            g.obj_mut(cont).extra_flags.set(flags::ITEM_NODROP);
            comm::act_full(g, b"You get a strange feeling as you put $p in $P.", false, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_CHAR);
        } else {
            comm::act_full(g, b"You put $p in $P.", false, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_CHAR);
        }
    }
}

pub fn do_put(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2, rest) = two_arguments(argument);
    let (arg3, _) = one_argument(rest);

    let (howmany, theobj, thecont) = if !arg3.is_empty() && is_number(&arg1) {
        (atoi(&arg1), arg2.clone(), arg3.clone())
    } else {
        (1, arg1.clone(), arg2.clone())
    };
    let (obj_dotmode, theobj_name) = find_all_dots(&theobj);
    let (cont_dotmode, thecont_name) = find_all_dots(&thecont);

    if theobj.is_empty() {
        send_to_char(g, chid, b"Put what in what?\r\n");
    } else if cont_dotmode != FIND_INDIV {
        send_to_char(g, chid, b"You can only put things into one container at a time.\r\n");
    } else if thecont.is_empty() {
        let mut msg = b"What do you want to put ".to_vec();
        msg.extend_from_slice(if obj_dotmode == FIND_INDIV { b"it" } else { b"them" });
        msg.extend_from_slice(b" in?\r\n");
        send_to_char(g, chid, &msg);
    } else {
        let (_, _, cont) = generic_find(g, chid, &thecont_name, FIND_OBJ_INV | FIND_OBJ_ROOM);
        let Some(cont) = cont else {
            let mut msg = b"You don't see ".to_vec();
            msg.extend_from_slice(an(&thecont));
            msg.push(b' ');
            msg.extend_from_slice(&thecont);
            msg.extend_from_slice(b" here.\r\n");
            send_to_char(g, chid, &msg);
            return;
        };
        if g.obj(cont).type_flag != flags::ITEM_CONTAINER {
            act(g, b"$p is not a container.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
        } else if g.obj(cont).values[1] & flags::CONT_CLOSED != 0
            && (g.ch(chid).level < LVL_IMMORT || !g.ch(chid).prf(flags::PRF_NOHASSLE))
        {
            send_to_char(g, chid, b"You'd better open it first!\r\n");
        } else if obj_dotmode == FIND_INDIV {
            // put <obj> <container>
            let carrying = g.ch(chid).carrying.clone();
            let Some(first) = get_obj_in_list_vis(g, chid, &theobj_name, None, &carrying) else {
                let mut msg = b"You aren't carrying ".to_vec();
                msg.extend_from_slice(an(&theobj));
                msg.push(b' ');
                msg.extend_from_slice(&theobj);
                msg.extend_from_slice(b".\r\n");
                send_to_char(g, chid, &msg);
                return;
            };
            if first == cont && howmany == 1 {
                send_to_char(g, chid, b"You attempt to fold it into itself, but fail.\r\n");
                return;
            }
            // Walks the contents chain from each found object; the
            // countdown decrements only for objects actually put.
            let mut howmany = howmany;
            let mut obj = Some(first);
            while let Some(o) = obj {
                if howmany == 0 {
                    break;
                }
                // Snapshot the successor list BEFORE the move.
                let list_after = carrying_after(g, chid, o);
                if o != cont {
                    howmany -= 1;
                    perform_put(g, chid, o, cont);
                }
                obj = get_obj_in_list_vis(g, chid, &theobj_name, None, &list_after);
            }
        } else {
            let mut found = false;
            let carrying = g.ch(chid).carrying.clone();
            for o in carrying {
                if g.try_obj_alive(o) && o != cont && can_see_obj(g, chid, o)
                    && (obj_dotmode == FIND_ALL || isname(&theobj_name, obj_name(g, o)))
                {
                    found = true;
                    perform_put(g, chid, o, cont);
                }
            }
            if !found {
                if obj_dotmode == FIND_ALL {
                    send_to_char(g, chid, b"You don't seem to have anything to put in it.\r\n");
                } else {
                    let mut msg = b"You don't seem to have any ".to_vec();
                    msg.extend_from_slice(&theobj_name);
                    msg.extend_from_slice(b"s.\r\n");
                    send_to_char(g, chid, &msg);
                }
            }
        }
    }
}

/// The inventory slice strictly after `o` in the contents chain.
fn carrying_after(g: &Game, chid: CharId, o: ObjId) -> Vec<ObjId> {
    let carrying = &g.ch(chid).carrying;
    match carrying.iter().position(|&x| x == o) {
        Some(idx) => carrying[idx + 1..].to_vec(),
        None => Vec::new(),
    }
}

fn room_contents_after(g: &Game, room: RoomRnum, o: ObjId) -> Vec<ObjId> {
    let contents = &g.rooms[room as usize].contents;
    match contents.iter().position(|&x| x == o) {
        Some(idx) => contents[idx + 1..].to_vec(),
        None => Vec::new(),
    }
}

fn contains_after(g: &Game, cont: ObjId, o: ObjId) -> Vec<ObjId> {
    let contains = &g.obj(cont).contains;
    match contains.iter().position(|&x| x == o) {
        Some(idx) => contains[idx + 1..].to_vec(),
        None => Vec::new(),
    }
}

// ---- get ----

/// can_take_obj.
/// CAN_GET_OBJ: the silent macro — takeable, carryable, visible.
pub fn can_get_obj(g: &Game, chid: CharId, oid: ObjId) -> bool {
    let ch = g.ch(chid);
    g.obj(oid).can_wear(flags::ITEM_WEAR_TAKE)
        && ch.carry_weight + crate::handler::obj_weight(g, oid) <= crate::handler::can_carry_w(ch)
        && (ch.carry_items as i32) + 1 <= crate::handler::can_carry_n(ch)
        && crate::handler::can_see_obj(g, chid, oid)
}

fn can_take_obj(g: &mut Game, chid: CharId, oid: ObjId) -> bool {
    if !g.obj(oid).can_wear(flags::ITEM_WEAR_TAKE) {
        act(g, b"$p: you can't take that!", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        return false;
    }
    let ch = g.ch(chid);
    if !ch.is_npc() && !ch.prf(flags::PRF_NOHASSLE) {
        if (ch.carry_items as i32) >= can_carry_n(ch) {
            act(g, b"$p: you can't carry that many items.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
            return false;
        } else if ch.carry_weight + obj_weight(g, oid) > can_carry_w(ch) {
            act(g, b"$p: you can't carry that much weight.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
            return false;
        }
    }
    if g.obj(oid).sat_in_by.is_some() {
        act(g, b"It appears someone is sitting on $p..", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        return false;
    }
    true
}

fn get_check_money(g: &mut Game, chid: CharId, oid: ObjId) {
    let value = g.obj(oid).values[0];
    if g.obj(oid).type_flag != flags::ITEM_MONEY || value <= 0 {
        return;
    }
    extract_obj(g, oid);
    increase_gold(g, chid, value);
    if value == 1 {
        send_to_char(g, chid, b"There was 1 coin.\r\n");
    } else {
        send_to_char(g, chid, format!("There were {} coins.\r\n", value).as_bytes());
    }
}

fn perform_get_from_container(g: &mut Game, chid: CharId, oid: ObjId, cont: ObjId, mode: i32) {
    if mode == FIND_OBJ_INV || can_take_obj(g, chid, oid) {
        if (g.ch(chid).carry_items as i32) >= can_carry_n(g.ch(chid)) {
            act(g, b"$p: you can't hold any more items.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        } else if crate::dg::triggers::get_otrigger(g, oid, chid) != 0 {
            if g.try_obj(oid).is_none() {
                return;
            }
            obj_from_obj(g, oid);
            obj_to_char(g, oid, chid);
            comm::act_full(g, b"You get $p from $P.", false, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_CHAR);
            comm::act_full(g, b"$n gets $p from $P.", true, Some(chid), Some(oid), comm::ActArg::Obj(cont), comm::TO_ROOM);
            get_check_money(g, chid, oid);
        }
    }
}

fn get_from_container(g: &mut Game, chid: CharId, cont: ObjId, arg: &[u8], mode: i32, mut howmany: i32) {
    let (obj_dotmode, name) = find_all_dots(arg);

    if g.obj(cont).values[1] & flags::CONT_CLOSED != 0
        && (g.ch(chid).level < LVL_IMMORT || !g.ch(chid).prf(flags::PRF_NOHASSLE))
    {
        act(g, b"$p is closed.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
    } else if obj_dotmode == FIND_INDIV {
        let contains = g.obj(cont).contains.clone();
        let Some(first) = get_obj_in_list_vis(g, chid, &name, None, &contains) else {
            let mut buf = b"There doesn't seem to be ".to_vec();
            buf.extend_from_slice(an(&name));
            buf.push(b' ');
            buf.extend_from_slice(&name);
            buf.extend_from_slice(b" in $p.");
            act(g, &buf, false, Some(chid), Some(cont), None, comm::TO_CHAR);
            return;
        };
        let mut obj = Some(first);
        while let Some(o) = obj {
            if howmany == 0 {
                break;
            }
            howmany -= 1;
            let list_after = contains_after(g, cont, o);
            perform_get_from_container(g, chid, o, cont, mode);
            obj = get_obj_in_list_vis(g, chid, &name, None, &list_after);
        }
    } else {
        if obj_dotmode == FIND_ALLDOT && name.is_empty() {
            send_to_char(g, chid, b"Get all of what?\r\n");
            return;
        }
        let mut found = false;
        let contains = g.obj(cont).contains.clone();
        for o in contains {
            if g.try_obj_alive(o)
                && can_see_obj(g, chid, o)
                && (obj_dotmode == FIND_ALL || isname(&name, obj_name(g, o)))
            {
                found = true;
                perform_get_from_container(g, chid, o, cont, mode);
            }
        }
        if !found {
            if obj_dotmode == FIND_ALL {
                act(g, b"$p seems to be empty.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
            } else {
                let mut buf = b"You can't seem to find any ".to_vec();
                buf.extend_from_slice(&name);
                buf.extend_from_slice(b"s in $p.");
                act(g, &buf, false, Some(chid), Some(cont), None, comm::TO_CHAR);
            }
        }
    }
}

fn perform_get_from_room(g: &mut Game, chid: CharId, oid: ObjId) -> bool {
    if can_take_obj(g, chid, oid) && crate::dg::triggers::get_otrigger(g, oid, chid) != 0 {
        if g.try_obj(oid).is_none() {
            return false;
        }
        obj_from_room(g, oid);
        obj_to_char(g, oid, chid);
        act(g, b"You get $p.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        act(g, b"$n gets $p.", true, Some(chid), Some(oid), None, comm::TO_ROOM);
        get_check_money(g, chid, oid);
        return true;
    }
    false
}

fn get_from_room(g: &mut Game, chid: CharId, arg: &[u8], mut howmany: i32) {
    let (dotmode, name) = find_all_dots(arg);
    let room = g.ch(chid).in_room;

    if dotmode == FIND_INDIV {
        let contents = g.rooms[room as usize].contents.clone();
        let Some(first) = get_obj_in_list_vis(g, chid, &name, None, &contents) else {
            // Room extra description keyword?
            let ex = g.world.rooms[room as usize].ex_descriptions.clone();
            let (num, stripped) = get_number(&name);
            let _ = num;
            let matches_exdesc = ex.iter().any(|e| {
                e.keyword.as_deref().is_some_and(|k| isname(&stripped, k))
            });
            let mut msg;
            if matches_exdesc {
                msg = b"You can't take ".to_vec();
            } else {
                msg = b"You don't see ".to_vec();
            }
            msg.extend_from_slice(an(&name));
            msg.push(b' ');
            msg.extend_from_slice(&name);
            msg.extend_from_slice(if matches_exdesc { b".\r\n" } else { b" here.\r\n" });
            send_to_char(g, chid, &msg);
            return;
        };
        let mut obj = Some(first);
        while let Some(o) = obj {
            if howmany == 0 {
                break;
            }
            howmany -= 1;
            let list_after = room_contents_after(g, room, o);
            perform_get_from_room(g, chid, o);
            obj = get_obj_in_list_vis(g, chid, &name, None, &list_after);
        }
    } else {
        if dotmode == FIND_ALLDOT && name.is_empty() {
            send_to_char(g, chid, b"Get all of what?\r\n");
            return;
        }
        let mut found = false;
        let contents = g.rooms[room as usize].contents.clone();
        for o in contents {
            if g.try_obj_alive(o)
                && can_see_obj(g, chid, o)
                && (dotmode == FIND_ALL || isname(&name, obj_name(g, o)))
            {
                found = true;
                perform_get_from_room(g, chid, o);
            }
        }
        if !found {
            if dotmode == FIND_ALL {
                send_to_char(g, chid, b"There doesn't seem to be anything here.\r\n");
            } else {
                let mut msg = b"You don't see any ".to_vec();
                msg.extend_from_slice(&name);
                msg.extend_from_slice(b"s here.\r\n");
                send_to_char(g, chid, &msg);
            }
        }
    }
}

pub fn do_get(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2, rest) = two_arguments(argument);
    let (arg3, _) = one_argument(rest);

    if arg1.is_empty() {
        send_to_char(g, chid, b"Get what?\r\n");
    } else if arg2.is_empty() {
        get_from_room(g, chid, &arg1, 1);
    } else if is_number(&arg1) && arg3.is_empty() {
        get_from_room(g, chid, &arg2, atoi(&arg1));
    } else {
        let (amount, arg1, arg2) = if is_number(&arg1) {
            (atoi(&arg1), arg2.clone(), arg3.clone())
        } else {
            (1, arg1.clone(), arg2.clone())
        };
        let (cont_dotmode, cont_name) = find_all_dots(&arg2);
        if cont_dotmode == FIND_INDIV {
            let (mode, _, cont) = generic_find(g, chid, &cont_name, FIND_OBJ_INV | FIND_OBJ_ROOM);
            let Some(cont) = cont else {
                let mut msg = b"You don't have ".to_vec();
                msg.extend_from_slice(an(&arg2));
                msg.push(b' ');
                msg.extend_from_slice(&arg2);
                msg.extend_from_slice(b".\r\n");
                send_to_char(g, chid, &msg);
                return;
            };
            if g.obj(cont).type_flag != flags::ITEM_CONTAINER {
                act(g, b"$p is not a container.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
            } else {
                get_from_container(g, chid, cont, &arg1, mode, amount);
            }
        } else {
            if cont_dotmode == FIND_ALLDOT && cont_name.is_empty() {
                send_to_char(g, chid, b"Get from all of what?\r\n");
                return;
            }
            let mut found = false;
            let carrying = g.ch(chid).carrying.clone();
            for cont in carrying {
                if g.try_obj_alive(cont)
                    && can_see_obj(g, chid, cont)
                    && (cont_dotmode == FIND_ALL || isname(&cont_name, obj_name(g, cont)))
                {
                    if g.obj(cont).type_flag == flags::ITEM_CONTAINER {
                        found = true;
                        get_from_container(g, chid, cont, &arg1, FIND_OBJ_INV, amount);
                    } else if cont_dotmode == FIND_ALLDOT {
                        found = true;
                        act(g, b"$p is not a container.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
                    }
                }
            }
            let room = g.ch(chid).in_room;
            let contents = g.rooms[room as usize].contents.clone();
            for cont in contents {
                if g.try_obj_alive(cont)
                    && can_see_obj(g, chid, cont)
                    && (cont_dotmode == FIND_ALL || isname(&cont_name, obj_name(g, cont)))
                {
                    if g.obj(cont).type_flag == flags::ITEM_CONTAINER {
                        get_from_container(g, chid, cont, &arg1, FIND_OBJ_ROOM, amount);
                        found = true;
                    } else if cont_dotmode == FIND_ALLDOT {
                        act(g, b"$p is not a container.", false, Some(chid), Some(cont), None, comm::TO_CHAR);
                        found = true;
                    }
                }
            }
            if !found {
                if cont_dotmode == FIND_ALL {
                    send_to_char(g, chid, b"You can't seem to find any containers.\r\n");
                } else {
                    let mut msg = b"You can't seem to find any ".to_vec();
                    msg.extend_from_slice(&cont_name);
                    msg.extend_from_slice(b"s here.\r\n");
                    send_to_char(g, chid, &msg);
                }
            }
        }
    }
}

// ---- drop / junk / donate ----

fn perform_drop_gold(g: &mut Game, chid: CharId, amount: i32, mode: i32, rdr: RoomRnum) {
    if amount <= 0 {
        send_to_char(g, chid, b"Heh heh heh.. we are jolly funny today, eh?\r\n");
    } else if g.ch(chid).points.gold < amount {
        send_to_char(g, chid, b"You don't have that many coins!\r\n");
    } else {
        if mode != SCMD_JUNK {
            // Anti-coin-bombing wait.
            g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
            let Some(obj) = handler::create_money(g, amount) else { return };
            if mode == SCMD_DONATE {
                send_to_char(g, chid, b"You throw some gold into the air where it disappears in a puff of smoke!\r\n");
                act(g, b"$n throws some gold into the air where it disappears in a puff of smoke!", false, Some(chid), None, None, comm::TO_ROOM);
                obj_to_room(g, obj, rdr);
                act(g, b"$p suddenly appears in a puff of orange smoke!", false, None, Some(obj), None, comm::TO_ROOM);
            } else {
                let object_id = crate::dg::obj_script_id(g, obj);
                if crate::dg::triggers::drop_wtrigger(g, obj, chid) == 0 {
                    if crate::dg::has_obj_by_uid_in_lookup_table(g, object_id)
                        && g.try_obj(obj).is_some()
                    {
                        handler::extract_obj(g, obj);
                    }
                    return;
                }
                let md = money_desc(g, amount).unwrap_or("");
                let mut buf = b"$n drops ".to_vec();
                buf.extend_from_slice(md.as_bytes());
                buf.push(b'.');
                act(g, &buf, true, Some(chid), None, None, comm::TO_ROOM);
                send_to_char(g, chid, b"You drop some gold.\r\n");
                let room = g.ch(chid).in_room;
                obj_to_room(g, obj, room);
            }
        } else {
            let md = money_desc(g, amount).unwrap_or("");
            let mut buf = b"$n drops ".to_vec();
            buf.extend_from_slice(md.as_bytes());
            buf.extend_from_slice(b" which disappears in a puff of smoke!");
            act(g, &buf, false, Some(chid), None, None, comm::TO_ROOM);
            send_to_char(g, chid, b"You drop some gold which disappears in a puff of smoke!\r\n");
        }
        decrease_gold(g, chid, amount);
    }
}

fn vanish(mode: i32) -> &'static [u8] {
    if mode == SCMD_DONATE || mode == SCMD_JUNK {
        b"  It vanishes in a puff of smoke!"
    } else {
        b""
    }
}

/// perform_drop. Returns the junk value accrued.
fn perform_drop(g: &mut Game, chid: CharId, oid: ObjId, mut mode: i32, sname: &[u8], rdr: RoomRnum) -> i32 {
    let object_id = crate::dg::obj_script_id(g, oid);
    if crate::dg::triggers::drop_otrigger(g, oid, chid) == 0 {
        return 0;
    }
    if !crate::dg::has_obj_by_uid_in_lookup_table(g, object_id) || g.try_obj(oid).is_none() {
        return 0; // item was extracted by script
    }
    if mode == SCMD_DROP && crate::dg::triggers::drop_wtrigger(g, oid, chid) == 0 {
        return 0;
    }
    if !crate::dg::has_obj_by_uid_in_lookup_table(g, object_id) || g.try_obj(oid).is_none() {
        return 0; // item was extracted by script
    }
    if g.obj(oid).obj_flagged(flags::ITEM_NODROP) && !g.ch(chid).prf(flags::PRF_NOHASSLE) {
        let mut buf = b"You can't ".to_vec();
        buf.extend_from_slice(sname);
        buf.extend_from_slice(b" $p, it must be CURSED!");
        act(g, &buf, false, Some(chid), Some(oid), None, comm::TO_CHAR);
        return 0;
    }
    {
        let mut buf = b"You ".to_vec();
        buf.extend_from_slice(sname);
        buf.extend_from_slice(b" $p.");
        buf.extend_from_slice(vanish(mode));
        act(g, &buf, false, Some(chid), Some(oid), None, comm::TO_CHAR);
    }
    {
        let mut buf = b"$n ".to_vec();
        buf.extend_from_slice(sname);
        buf.extend_from_slice(b"s $p.");
        buf.extend_from_slice(vanish(mode));
        act(g, &buf, true, Some(chid), Some(oid), None, comm::TO_ROOM);
    }
    obj_from_char(g, oid);

    if mode == SCMD_DONATE && g.obj(oid).obj_flagged(flags::ITEM_NODONATE) {
        mode = SCMD_JUNK;
    }

    match mode {
        SCMD_DROP => {
            let room = g.ch(chid).in_room;
            obj_to_room(g, oid, room);
            0
        }
        SCMD_DONATE => {
            obj_to_room(g, oid, rdr);
            act(g, b"$p suddenly appears in a puff a smoke!", false, None, Some(oid), None, comm::TO_ROOM);
            0
        }
        SCMD_JUNK => {
            let value = 1.max(200.min(g.obj(oid).cost / 16));
            extract_obj(g, oid);
            value
        }
        _ => {
            g.log(format!("SYSERR: Incorrect argument {} passed to perform_drop.", mode));
            0
        }
    }
}

/// do_drop — also junk and donate via subcmd.
pub fn do_drop(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    let mut mode = SCMD_DROP;
    let mut rdr: RoomRnum = 0;
    let sname: &[u8] = match subcmd {
        SCMD_JUNK => {
            mode = SCMD_JUNK;
            b"junk"
        }
        SCMD_DONATE => {
            mode = SCMD_DONATE;
            // Donation room selection: fail + double chance for room 1.
            let cfg = &g.config;
            let num_don_rooms = (cfg.donation_room_1 != NOWHERE as i32) as i32 * 2
                + (cfg.donation_room_2 != NOWHERE as i32) as i32
                + (cfg.donation_room_3 != NOWHERE as i32) as i32
                + 1;
            let (r1, r2, r3) = (cfg.donation_room_1, cfg.donation_room_2, cfg.donation_room_3);
            match g.rng.rand_number(0, num_don_rooms) {
                0 => mode = SCMD_JUNK,
                1 | 2 => rdr = g.real_room(r1).unwrap_or(NOWHERE),
                3 => rdr = g.real_room(r2).unwrap_or(NOWHERE),
                4 => rdr = g.real_room(r3).unwrap_or(NOWHERE),
                _ => {}
            }
            if rdr == NOWHERE {
                send_to_char(g, chid, b"Sorry, you can't donate anything right now.\r\n");
                return;
            }
            b"donate"
        }
        _ => b"drop",
    };

    let (arg, rest) = one_argument(argument);
    let mut amount = 0;

    if arg.is_empty() {
        let mut msg = b"What do you want to ".to_vec();
        msg.extend_from_slice(sname);
        msg.extend_from_slice(b"?\r\n");
        send_to_char(g, chid, &msg);
        return;
    } else if is_number(&arg) {
        let multi = atoi(&arg);
        let (arg, _) = one_argument(rest);
        if arg == b"coins" || arg == b"coin" {
            perform_drop_gold(g, chid, multi, mode, rdr);
        } else if multi <= 0 {
            send_to_char(g, chid, b"Yeah, that makes sense.\r\n");
        } else if arg.is_empty() {
            let mut msg = b"What do you want to ".to_vec();
            msg.extend_from_slice(sname);
            msg.extend_from_slice(format!(" {} of?\r\n", multi).as_bytes());
            send_to_char(g, chid, &msg);
        } else {
            let carrying = g.ch(chid).carrying.clone();
            let Some(first) = get_obj_in_list_vis(g, chid, &arg, None, &carrying) else {
                let mut msg = b"You don't seem to have any ".to_vec();
                msg.extend_from_slice(&arg);
                msg.extend_from_slice(b"s.\r\n");
                send_to_char(g, chid, &msg);
                return;
            };
            let mut multi = multi;
            let mut obj = Some(first);
            while let Some(o) = obj {
                let list_after = carrying_after(g, chid, o);
                let next = get_obj_in_list_vis(g, chid, &arg, None, &list_after);
                amount += perform_drop(g, chid, o, mode, sname, rdr);
                multi -= 1;
                obj = if multi != 0 { next } else { None };
            }
        }
    } else {
        let (dotmode, name) = find_all_dots(&arg);

        // Can't junk or donate all.
        if dotmode == FIND_ALL && (subcmd == SCMD_JUNK || subcmd == SCMD_DONATE) {
            if subcmd == SCMD_JUNK {
                send_to_char(g, chid, b"Go to the dump if you want to junk EVERYTHING!\r\n");
            } else {
                send_to_char(g, chid, b"Go do the donation room if you want to donate EVERYTHING!\r\n");
            }
            return;
        }
        if dotmode == FIND_ALL {
            if g.ch(chid).carrying.is_empty() {
                send_to_char(g, chid, b"You don't seem to be carrying anything.\r\n");
            } else {
                let carrying = g.ch(chid).carrying.clone();
                for o in carrying {
                    if g.try_obj_alive(o) {
                        amount += perform_drop(g, chid, o, mode, sname, rdr);
                    }
                }
            }
        } else if dotmode == FIND_ALLDOT {
            if name.is_empty() {
                let mut msg = b"What do you want to ".to_vec();
                msg.extend_from_slice(sname);
                msg.extend_from_slice(b" all of?\r\n");
                send_to_char(g, chid, &msg);
                return;
            }
            let carrying = g.ch(chid).carrying.clone();
            let mut obj = get_obj_in_list_vis(g, chid, &name, None, &carrying);
            if obj.is_none() {
                let mut msg = b"You don't seem to have any ".to_vec();
                msg.extend_from_slice(&name);
                msg.extend_from_slice(b"s.\r\n");
                send_to_char(g, chid, &msg);
            }
            while let Some(o) = obj {
                let list_after = carrying_after(g, chid, o);
                let next = get_obj_in_list_vis(g, chid, &name, None, &list_after);
                amount += perform_drop(g, chid, o, mode, sname, rdr);
                obj = next;
            }
        } else {
            let carrying = g.ch(chid).carrying.clone();
            match get_obj_in_list_vis(g, chid, &name, None, &carrying) {
                None => {
                    let mut msg = b"You don't seem to have ".to_vec();
                    msg.extend_from_slice(an(&name));
                    msg.push(b' ');
                    msg.extend_from_slice(&name);
                    msg.extend_from_slice(b".\r\n");
                    send_to_char(g, chid, &msg);
                }
                Some(o) => {
                    amount += perform_drop(g, chid, o, mode, sname, rdr);
                }
            }
        }
    }

    if amount != 0 && subcmd == SCMD_JUNK {
        send_to_char(g, chid, b"You have been rewarded by the gods!\r\n");
        act(g, b"$n has been rewarded by the gods!", true, Some(chid), None, None, comm::TO_ROOM);
        // Added directly, bypassing the increase_gold clamp.
        g.ch_mut(chid).points.gold += amount;
    }
}

// ---- give ----

fn perform_give(g: &mut Game, chid: CharId, vict: CharId, oid: ObjId) {
    if crate::dg::triggers::give_otrigger(g, oid, chid, vict) == 0 {
        return;
    }
    if crate::dg::triggers::receive_mtrigger(g, vict, chid, oid) == 0 {
        return;
    }
    if g.try_obj(oid).is_none() {
        return;
    }
    if g.obj(oid).obj_flagged(flags::ITEM_NODROP) && !g.ch(chid).prf(flags::PRF_NOHASSLE) {
        act(g, b"You can't let go of $p!!  Yeech!", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        return;
    }
    let ch_lvl = g.ch(chid).level;
    let vict_lvl = g.ch(vict).level;
    if (g.ch(vict).carry_items as i32) >= can_carry_n(g.ch(vict)) && ch_lvl < LVL_IMMORT && vict_lvl < LVL_IMMORT {
        act(g, b"$N seems to have $S hands full.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        return;
    }
    if obj_weight(g, oid) + g.ch(vict).carry_weight > can_carry_w(g.ch(vict)) && ch_lvl < LVL_IMMORT && vict_lvl < LVL_IMMORT {
        act(g, b"$E can't carry that much weight.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        return;
    }
    obj_from_char(g, oid);
    obj_to_char(g, oid, vict);
    act(g, b"You give $p to $N.", false, Some(chid), Some(oid), Some(vict), comm::TO_CHAR);
    act(g, b"$n gives you $p.", false, Some(chid), Some(oid), Some(vict), comm::TO_VICT);
    act(g, b"$n gives $p to $N.", true, Some(chid), Some(oid), Some(vict), comm::TO_NOTVICT);
    crate::quest::autoquest_trigger_check(g, chid, Some(vict), Some(oid), crate::quest::AQ_OBJ_RETURN);
}

fn give_find_vict(g: &mut Game, chid: CharId, arg: &[u8]) -> Option<CharId> {
    let arg = skip_spaces(arg);
    if arg.is_empty() {
        send_to_char(g, chid, b"To who?\r\n");
        return None;
    }
    let Some(vict) = handler::get_char_room_vis(g, chid, arg, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return None;
    };
    if vict == chid {
        send_to_char(g, chid, b"What's the point of that?\r\n");
        return None;
    }
    Some(vict)
}

fn perform_give_gold(g: &mut Game, chid: CharId, vict: CharId, amount: i32) {
    if amount <= 0 {
        send_to_char(g, chid, b"Heh heh heh ... we are jolly funny today, eh?\r\n");
        return;
    }
    let (gold, is_npc, level) = {
        let ch = g.ch(chid);
        (ch.points.gold, ch.is_npc(), ch.level)
    };
    if gold < amount && (is_npc || level < LVL_GOD) {
        send_to_char(g, chid, b"You don't have that many coins!\r\n");
        return;
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);

    {
        let mut buf = format!("$n gives you {} gold coin{}.", amount, if amount == 1 { "" } else { "s" }).into_bytes();
        act(g, &buf, false, Some(chid), None, Some(vict), comm::TO_VICT);
        buf.clear();
    }
    {
        let md = money_desc(g, amount).unwrap_or("");
        let mut buf = b"$n gives ".to_vec();
        buf.extend_from_slice(md.as_bytes());
        buf.extend_from_slice(b" to $N.");
        act(g, &buf, true, Some(chid), None, Some(vict), comm::TO_NOTVICT);
    }
    if is_npc || level < LVL_GOD {
        decrease_gold(g, chid, amount);
    }
    increase_gold(g, vict, amount);
    crate::dg::triggers::bribe_mtrigger(g, vict, chid, amount);
}

pub fn do_give(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, rest) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Give what to who?\r\n");
    } else if is_number(&arg) {
        let amount = atoi(&arg);
        let (arg, rest2) = one_argument(rest);
        if arg == b"coins" || arg == b"coin" {
            let (arg, _) = one_argument(rest2);
            if let Some(vict) = give_find_vict(g, chid, &arg) {
                perform_give_gold(g, chid, vict, amount);
            }
            return;
        } else if arg.is_empty() {
            // Give multiple code.
            send_to_char(g, chid, format!("What do you want to give {} of?\r\n", amount).as_bytes());
        } else {
            let Some(vict) = give_find_vict(g, chid, rest2) else {
                return;
            };
            let carrying = g.ch(chid).carrying.clone();
            let Some(first) = get_obj_in_list_vis(g, chid, &arg, None, &carrying) else {
                let mut msg = b"You don't seem to have any ".to_vec();
                msg.extend_from_slice(&arg);
                msg.extend_from_slice(b"s.\r\n");
                send_to_char(g, chid, &msg);
                return;
            };
            let mut amount = amount;
            let mut obj = Some(first);
            while let Some(o) = obj {
                if amount == 0 {
                    break;
                }
                amount -= 1;
                let list_after = carrying_after(g, chid, o);
                perform_give(g, chid, vict, o);
                obj = get_obj_in_list_vis(g, chid, &arg, None, &list_after);
            }
        }
    } else {
        let (buf1, _) = one_argument(rest);
        let Some(vict) = give_find_vict(g, chid, &buf1) else {
            return;
        };
        let (dotmode, name) = find_all_dots(&arg);
        if dotmode == FIND_INDIV {
            let carrying = g.ch(chid).carrying.clone();
            match get_obj_in_list_vis(g, chid, &name, None, &carrying) {
                None => {
                    let mut msg = b"You don't seem to have ".to_vec();
                    msg.extend_from_slice(an(&name));
                    msg.push(b' ');
                    msg.extend_from_slice(&name);
                    msg.extend_from_slice(b".\r\n");
                    send_to_char(g, chid, &msg);
                }
                Some(o) => perform_give(g, chid, vict, o),
            }
        } else {
            if dotmode == FIND_ALLDOT && name.is_empty() {
                send_to_char(g, chid, b"All of what?\r\n");
                return;
            }
            if g.ch(chid).carrying.is_empty() {
                send_to_char(g, chid, b"You don't seem to be holding anything.\r\n");
            } else {
                let carrying = g.ch(chid).carrying.clone();
                for o in carrying {
                    if g.try_obj_alive(o)
                        && can_see_obj(g, chid, o)
                        && (dotmode == FIND_ALL || isname(&name, obj_name(g, o)))
                    {
                        perform_give(g, chid, vict, o);
                    }
                }
            }
        }
    }
}

// ---- drink containers ----

pub fn weight_change_object(g: &mut Game, oid: ObjId, weight: i32) {
    if g.obj(oid).in_room != NOWHERE {
        g.obj_mut(oid).weight += weight;
    } else if let Some(tmp_ch) = g.obj(oid).carried_by {
        obj_from_char(g, oid);
        g.obj_mut(oid).weight += weight;
        obj_to_char(g, oid, tmp_ch);
    } else if let Some(tmp_obj) = g.obj(oid).in_obj {
        obj_from_obj(g, oid);
        g.obj_mut(oid).weight += weight;
        obj_to_obj(g, oid, tmp_obj);
    } else {
        g.log("SYSERR: Unknown attempt to subtract weight from an object.".to_string());
    }
}

fn limited_drink_container(g: &Game, oid: ObjId) -> bool {
    g.obj(oid).values[0] >= 0 && g.obj(oid).values[1] >= 0
}

fn empty_drink_container(g: &Game, oid: ObjId) -> bool {
    limited_drink_container(g, oid) && g.obj(oid).values[1] < 1
}

/// name_from_drinkcon: strip the liquid alias.
pub fn name_from_drinkcon(g: &mut Game, oid: ObjId) {
    let tf = g.obj(oid).type_flag;
    if tf != flags::ITEM_DRINKCON && tf != flags::ITEM_FOUNTAIN {
        return;
    }
    let liq = g.obj(oid).values[2].clamp(0, 15) as usize;
    let liqname = tables::DRINKNAMES[liq].as_bytes();
    let name = obj_name(g, oid).to_vec();
    let stripped = remove_from_string(&name, liqname);
    let trimmed = right_trim_whitespace(&stripped);
    g.obj_mut(oid).name = Some(trimmed);
}

/// name_to_drinkcon: append the liquid alias.
pub fn name_to_drinkcon(g: &mut Game, oid: ObjId, liq_type: i32) {
    let tf = g.obj(oid).type_flag;
    if tf != flags::ITEM_DRINKCON && tf != flags::ITEM_FOUNTAIN {
        return;
    }
    let liq = liq_type.clamp(0, 15) as usize;
    let mut new_name = obj_name(g, oid).to_vec();
    new_name.push(b' ');
    new_name.extend_from_slice(tables::DRINKNAMES[liq].as_bytes());
    g.obj_mut(oid).name = Some(new_name);
}

/// remove_from_string: removes every occurrence of the word
/// when followed by whitespace/end. NOTE the quirks kept: no boundary
/// check at the match START, and any leading space survives.
fn remove_from_string(string: &[u8], to_remove: &[u8]) -> Vec<u8> {
    let mut s = string.to_vec();
    if to_remove.is_empty() {
        return s;
    }
    let mut i = 0;
    while s.len() >= to_remove.len() && i <= s.len() - to_remove.len() {
        let matches = s[i..i + to_remove.len()] == *to_remove;
        let boundary = match s.get(i + to_remove.len()) {
            None => true,
            Some(&c) => c == b' ' || c == b'\t' || c == b'\n',
        };
        if matches && boundary {
            s.drain(i..i + to_remove.len());
            // Decrement i to re-check from the same spot.
        } else {
            i += 1;
        }
    }
    s
}

/// right_trim_whitespace: trims trailing space AND
/// non-printables.
fn right_trim_whitespace(string: &[u8]) -> Vec<u8> {
    let mut end = string.len();
    while end > 0 {
        let c = string[end - 1];
        if c.is_ascii_whitespace() || !(0x20..0x7f).contains(&c) {
            end -= 1;
        } else {
            break;
        }
    }
    string[..end].to_vec()
}

/// do_drink — also sip via subcmd.
pub fn do_drink(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if g.ch(chid).is_npc() {
        // Cannot use GET_COND on mobs.
        return;
    }

    if arg.is_empty() {
        let room = g.ch(chid).in_room;
        let sect = g.world.rooms[room as usize].sector_type;
        if sect == flags::SECT_WATER_SWIM || sect == flags::SECT_WATER_NOSWIM || sect == flags::SECT_UNDERWATER {
            // Quirk kept: the full-stomach scold does NOT stop the drink.
            if g.ch(chid).ps().conditions[HUNGER] > 20 && g.ch(chid).ps().conditions[THIRST] > 0 {
                send_to_char(g, chid, b"Your stomach can't contain anymore!\r\n");
            }
            act(g, b"$n takes a refreshing drink.", true, Some(chid), None, None, comm::TO_ROOM);
            send_to_char(g, chid, b"You take a refreshing drink.\r\n");
            gain_condition(g, chid, THIRST, 1);
            if g.ch(chid).ps().conditions[THIRST] > 20 {
                send_to_char(g, chid, b"You don't feel thirsty any more.\r\n");
            }
        } else {
            send_to_char(g, chid, b"Drink from what?\r\n");
        }
        return;
    }

    let carrying = g.ch(chid).carrying.clone();
    let mut on_ground = false;
    let temp = match get_obj_in_list_vis(g, chid, &arg, None, &carrying) {
        Some(o) => o,
        None => {
            let room = g.ch(chid).in_room;
            let contents = g.rooms[room as usize].contents.clone();
            match get_obj_in_list_vis(g, chid, &arg, None, &contents) {
                Some(o) => {
                    on_ground = true;
                    o
                }
                None => {
                    send_to_char(g, chid, b"You can't find it!\r\n");
                    return;
                }
            }
        }
    };

    let tf = g.obj(temp).type_flag;
    if tf != flags::ITEM_DRINKCON && tf != flags::ITEM_FOUNTAIN {
        send_to_char(g, chid, b"You can't drink from that!\r\n");
        return;
    }
    if on_ground && tf == flags::ITEM_DRINKCON {
        send_to_char(g, chid, b"You have to be holding that to drink from it.\r\n");
        return;
    }
    if g.ch(chid).ps().conditions[DRUNK] > 10 && g.ch(chid).ps().conditions[THIRST] > 0 {
        // The pig is drunk.
        send_to_char(g, chid, b"You can't seem to get close enough to your mouth.\r\n");
        act(g, b"$n tries to drink but misses $s mouth!", true, Some(chid), None, None, comm::TO_ROOM);
        return;
    }
    if g.ch(chid).ps().conditions[HUNGER] > 20 && g.ch(chid).ps().conditions[THIRST] > 0 {
        send_to_char(g, chid, b"Your stomach can't contain anymore!\r\n");
        return;
    }
    if empty_drink_container(g, temp) {
        send_to_char(g, chid, b"It is empty.\r\n");
        return;
    }
    if crate::dg::triggers::consume_otrigger(g, temp, chid, crate::dg::OCMD_DRINK) == 0 {
        return;
    }
    if g.try_obj(temp).is_none() {
        return;
    }

    let liq = g.obj(temp).values[2].clamp(0, 15) as usize;
    let mut amount;
    if subcmd == SCMD_DRINK {
        let mut buf = b"$n drinks ".to_vec();
        buf.extend_from_slice(tables::DRINKS[liq].as_bytes());
        buf.extend_from_slice(b" from $p.");
        act(g, &buf, true, Some(chid), Some(temp), None, comm::TO_ROOM);

        send_to_char(g, chid, format!("You drink the {}.\r\n", tables::DRINKS[liq]).as_bytes());

        if tables::DRINK_AFF[liq][DRUNK] > 0 {
            amount = (25 - g.ch(chid).ps().conditions[THIRST] as i32) / tables::DRINK_AFF[liq][DRUNK];
        } else {
            amount = g.rng.rand_number(3, 10);
        }
    } else {
        act(g, b"$n sips from $p.", true, Some(chid), Some(temp), None, comm::TO_ROOM);
        send_to_char(g, chid, format!("It tastes like {}.\r\n", tables::DRINKS[liq]).as_bytes());
        amount = 1;
    }

    if limited_drink_container(g, temp) {
        amount = amount.min(g.obj(temp).values[1]);
    }

    // You can't subtract more than the object weighs, unless its unlimited.
    if limited_drink_container(g, temp) {
        let weight = amount.min(obj_weight(g, temp));
        weight_change_object(g, temp, -weight);
    }

    gain_condition(g, chid, DRUNK, tables::DRINK_AFF[liq][DRUNK] * amount / 4);
    gain_condition(g, chid, HUNGER, tables::DRINK_AFF[liq][HUNGER] * amount / 4);
    gain_condition(g, chid, THIRST, tables::DRINK_AFF[liq][THIRST] * amount / 4);

    if g.ch(chid).ps().conditions[DRUNK] > 10 {
        send_to_char(g, chid, b"You feel drunk.\r\n");
    }
    if g.ch(chid).ps().conditions[THIRST] > 20 {
        send_to_char(g, chid, b"You don't feel thirsty any more.\r\n");
    }
    if g.ch(chid).ps().conditions[HUNGER] > 20 {
        send_to_char(g, chid, b"You are full.\r\n");
    }

    if g.obj(temp).values[3] != 0 && g.ch(chid).level < LVL_IMMORT {
        // The crap was poisoned!
        send_to_char(g, chid, b"Oops, it tasted rather strange!\r\n");
        act(g, b"$n chokes and utters some strange sounds.", true, Some(chid), None, None, comm::TO_ROOM);

        let mut af = Affect { spell: SPELL_POISON, duration: (amount * 3) as i16, ..Default::default() };
        af.bitvector.set(flags::AFF_POISON);
        handler::affect_join(g, chid, af, false, false, false, false);
    }
    // Empty the container (unless unlimited), and no longer poison.
    if limited_drink_container(g, temp) {
        let amount = amount.min(g.obj(temp).values[1]);
        g.obj_mut(temp).values[1] -= amount;
        if g.obj(temp).values[1] == 0 {
            // The last bit.
            name_from_drinkcon(g, temp);
            g.obj_mut(temp).values[2] = 0;
            g.obj_mut(temp).values[3] = 0;
        }
    }
}

/// do_eat — also taste via subcmd.
pub fn do_eat(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if g.ch(chid).is_npc() {
        return;
    }
    if arg.is_empty() {
        send_to_char(g, chid, b"Eat what?\r\n");
        return;
    }
    let carrying = g.ch(chid).carrying.clone();
    let Some(food) = get_obj_in_list_vis(g, chid, &arg, None, &carrying) else {
        let mut msg = b"You don't seem to have ".to_vec();
        msg.extend_from_slice(an(&arg));
        msg.push(b' ');
        msg.extend_from_slice(&arg);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    let tf = g.obj(food).type_flag;
    if subcmd == SCMD_TASTE && (tf == flags::ITEM_DRINKCON || tf == flags::ITEM_FOUNTAIN) {
        do_drink(g, chid, argument, cmd, SCMD_SIP);
        return;
    }
    if tf != flags::ITEM_FOOD && g.ch(chid).level < LVL_IMMORT {
        send_to_char(g, chid, b"You can't eat THAT!\r\n");
        return;
    }
    if g.ch(chid).ps().conditions[HUNGER] > 20 {
        // Stomach full.
        send_to_char(g, chid, b"You are too full to eat more!\r\n");
        return;
    }
    if crate::dg::triggers::consume_otrigger(g, food, chid, crate::dg::OCMD_EAT) == 0 {
        return;
    }
    if g.try_obj(food).is_none() {
        return;
    }

    if subcmd == SCMD_EAT {
        act(g, b"You eat $p.", false, Some(chid), Some(food), None, comm::TO_CHAR);
        act(g, b"$n eats $p.", true, Some(chid), Some(food), None, comm::TO_ROOM);
    } else {
        act(g, b"You nibble a little bit of $p.", false, Some(chid), Some(food), None, comm::TO_CHAR);
        act(g, b"$n tastes a little bit of $p.", true, Some(chid), Some(food), None, comm::TO_ROOM);
    }

    let amount = if subcmd == SCMD_EAT { g.obj(food).values[0] } else { 1 };
    gain_condition(g, chid, HUNGER, amount);

    if g.ch(chid).ps().conditions[HUNGER] > 20 {
        send_to_char(g, chid, b"You are full.\r\n");
    }

    if g.obj(food).values[3] != 0 && g.ch(chid).level < LVL_IMMORT {
        // The crap was poisoned!
        send_to_char(g, chid, b"Oops, that tasted rather strange!\r\n");
        act(g, b"$n coughs and utters some strange sounds.", false, Some(chid), None, None, comm::TO_ROOM);

        let mut af = Affect { spell: SPELL_POISON, duration: (amount * 2) as i16, ..Default::default() };
        af.bitvector.set(flags::AFF_POISON);
        handler::affect_join(g, chid, af, false, false, false, false);
    }
    if subcmd == SCMD_EAT {
        extract_obj(g, food);
    } else {
        g.obj_mut(food).values[0] -= 1;
        if g.obj(food).values[0] == 0 {
            send_to_char(g, chid, b"There's nothing left now.\r\n");
            extract_obj(g, food);
        }
    }
}

/// do_pour — also fill via subcmd.
pub fn do_pour(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    let (arg1, arg2, _) = two_arguments(argument);
    let mut from_obj: Option<ObjId> = None;
    let mut to_obj: Option<ObjId> = None;

    if subcmd == crate::interpreter::SCMD_POUR {
        if arg1.is_empty() {
            // No arguments.
            send_to_char(g, chid, b"From what do you want to pour?\r\n");
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        let Some(fo) = get_obj_in_list_vis(g, chid, &arg1, None, &carrying) else {
            send_to_char(g, chid, b"You can't find it!\r\n");
            return;
        };
        if g.obj(fo).type_flag != flags::ITEM_DRINKCON {
            send_to_char(g, chid, b"You can't pour from that!\r\n");
            return;
        }
        from_obj = Some(fo);
    }
    if subcmd == crate::interpreter::SCMD_FILL {
        if arg1.is_empty() {
            // No arguments.
            send_to_char(g, chid, b"What do you want to fill?  And what are you filling it from?\r\n");
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        let Some(to) = get_obj_in_list_vis(g, chid, &arg1, None, &carrying) else {
            send_to_char(g, chid, b"You can't find it!\r\n");
            return;
        };
        if g.obj(to).type_flag != flags::ITEM_DRINKCON {
            act(g, b"You can't fill $p!", false, Some(chid), Some(to), None, comm::TO_CHAR);
            return;
        }
        to_obj = Some(to);
        if arg2.is_empty() {
            // No 2nd argument.
            act(g, b"What do you want to fill $p from?", false, Some(chid), Some(to), None, comm::TO_CHAR);
            return;
        }
        let room = g.ch(chid).in_room;
        let contents = g.rooms[room as usize].contents.clone();
        let Some(fo) = get_obj_in_list_vis(g, chid, &arg2, None, &contents) else {
            let mut msg = b"There doesn't seem to be ".to_vec();
            msg.extend_from_slice(an(&arg2));
            msg.push(b' ');
            msg.extend_from_slice(&arg2);
            msg.extend_from_slice(b" here.\r\n");
            send_to_char(g, chid, &msg);
            return;
        };
        if g.obj(fo).type_flag != flags::ITEM_FOUNTAIN {
            act(g, b"You can't fill something from $p.", false, Some(chid), Some(fo), None, comm::TO_CHAR);
            return;
        }
        from_obj = Some(fo);
    }
    let from_obj = from_obj.unwrap();
    if empty_drink_container(g, from_obj) {
        act(g, b"The $p is empty.", false, Some(chid), Some(from_obj), None, comm::TO_CHAR);
        return;
    }
    if subcmd == crate::interpreter::SCMD_POUR {
        // pour
        if arg2.is_empty() {
            send_to_char(g, chid, b"Where do you want it?  Out or in what?\r\n");
            return;
        }
        if arg2 == b"out" {
            if !limited_drink_container(g, from_obj) {
                send_to_char(g, chid, b"You can't pour that out! There's simply too much in it.\r\n");
                return;
            }
            // Pour out.
            act(g, b"$n empties $p.", true, Some(chid), Some(from_obj), None, comm::TO_ROOM);
            act(g, b"You empty $p.", false, Some(chid), Some(from_obj), None, comm::TO_CHAR);

            let now = g.obj(from_obj).values[1];
            weight_change_object(g, from_obj, -now); // Empty.

            name_from_drinkcon(g, from_obj);

            g.obj_mut(from_obj).values[1] = 0;
            g.obj_mut(from_obj).values[2] = 0;
            g.obj_mut(from_obj).values[3] = 0;
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        let Some(to) = get_obj_in_list_vis(g, chid, &arg2, None, &carrying) else {
            send_to_char(g, chid, b"You can't find it!\r\n");
            return;
        };
        let tf = g.obj(to).type_flag;
        if tf != flags::ITEM_DRINKCON && tf != flags::ITEM_FOUNTAIN {
            send_to_char(g, chid, b"You can't pour anything into that.\r\n");
            return;
        }
        to_obj = Some(to);
    }
    let to_obj = to_obj.unwrap();
    if to_obj == from_obj {
        send_to_char(g, chid, b"A most unproductive effort.\r\n");
        return;
    }
    if !empty_drink_container(g, to_obj) && g.obj(to_obj).values[2] != g.obj(from_obj).values[2] {
        send_to_char(g, chid, b"There is already another liquid in it!\r\n");
        return;
    }
    // Not allowed to fill an unlimited container, or one that is full.
    if !limited_drink_container(g, to_obj) || g.obj(to_obj).values[1] >= g.obj(to_obj).values[0] {
        send_to_char(g, chid, b"There is no room for more.\r\n");
        return;
    }
    let from_liq = g.obj(from_obj).values[2].clamp(0, 15) as usize;
    if subcmd == crate::interpreter::SCMD_POUR {
        // NOTE: this prints the typed second WORD, not the object name.
        let mut msg = b"You pour the ".to_vec();
        msg.extend_from_slice(tables::DRINKS[from_liq].as_bytes());
        msg.extend_from_slice(b" into the ");
        msg.extend_from_slice(&arg2);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
    }
    if subcmd == crate::interpreter::SCMD_FILL {
        comm::act_full(g, b"You gently fill $p from $P.", false, Some(chid), Some(to_obj), comm::ActArg::Obj(from_obj), comm::TO_CHAR);
        comm::act_full(g, b"$n gently fills $p from $P.", true, Some(chid), Some(to_obj), comm::ActArg::Obj(from_obj), comm::TO_ROOM);
    }
    // New alias.
    if empty_drink_container(g, to_obj) {
        name_to_drinkcon(g, to_obj, g.obj(from_obj).values[2]);
    }
    // First same type liq.
    {
        let liq = g.obj(from_obj).values[2];
        g.obj_mut(to_obj).values[2] = liq;
    }
    // Then how much to pour.
    let amount;
    if limited_drink_container(g, from_obj) {
        amount = g.obj(from_obj).values[1].min(g.obj(to_obj).values[0] - g.obj(to_obj).values[1]);
        g.obj_mut(from_obj).values[1] -= amount;
        g.obj_mut(to_obj).values[1] += amount;

        if g.obj(from_obj).values[1] == 0 {
            // It was emptied.
            name_from_drinkcon(g, from_obj);
            g.obj_mut(from_obj).values[1] = 0;
            g.obj_mut(from_obj).values[2] = 0;
            g.obj_mut(from_obj).values[3] = 0;
        }
    } else {
        amount = g.obj(to_obj).values[0] - g.obj(to_obj).values[1];
        let max = g.obj(to_obj).values[0];
        g.obj_mut(to_obj).values[1] = max;
    }
    // Poisoned?
    let poisoned = (g.obj(to_obj).values[3] != 0) || (g.obj(from_obj).values[3] != 0);
    g.obj_mut(to_obj).values[3] = poisoned as i32;
    // Weight change, except for unlimited.
    if limited_drink_container(g, from_obj) {
        weight_change_object(g, from_obj, -amount);
    }
    weight_change_object(g, to_obj, amount); // Add weight.
}

// ---- wear / wield / hold / remove ----

fn wear_message(g: &mut Game, chid: CharId, oid: ObjId, where_: usize) {
    const WEAR_MESSAGES: [[&[u8]; 2]; 18] = [
        [b"$n lights $p and holds it.", b"You light $p and hold it."],
        [b"$n slides $p on to $s right ring finger.", b"You slide $p on to your right ring finger."],
        [b"$n slides $p on to $s left ring finger.", b"You slide $p on to your left ring finger."],
        [b"$n wears $p around $s neck.", b"You wear $p around your neck."],
        [b"$n wears $p around $s neck.", b"You wear $p around your neck."],
        [b"$n wears $p on $s body.", b"You wear $p on your body."],
        [b"$n wears $p on $s head.", b"You wear $p on your head."],
        [b"$n puts $p on $s legs.", b"You put $p on your legs."],
        [b"$n wears $p on $s feet.", b"You wear $p on your feet."],
        [b"$n puts $p on $s hands.", b"You put $p on your hands."],
        [b"$n wears $p on $s arms.", b"You wear $p on your arms."],
        [b"$n straps $p around $s arm as a shield.", b"You start to use $p as a shield."],
        [b"$n wears $p about $s body.", b"You wear $p around your body."],
        [b"$n wears $p around $s waist.", b"You wear $p around your waist."],
        [b"$n puts $p on around $s right wrist.", b"You put $p on around your right wrist."],
        [b"$n puts $p on around $s left wrist.", b"You put $p on around your left wrist."],
        [b"$n wields $p.", b"You wield $p."],
        [b"$n grabs $p.", b"You grab $p."],
    ];
    act(g, WEAR_MESSAGES[where_][0], true, Some(chid), Some(oid), None, comm::TO_ROOM);
    act(g, WEAR_MESSAGES[where_][1], false, Some(chid), Some(oid), None, comm::TO_CHAR);
}

fn perform_wear(g: &mut Game, chid: CharId, oid: ObjId, mut where_: usize) {
    // ITEM_WEAR_TAKE is the "no special bit needed" sentinel for LIGHT/HOLD.
    const WEAR_BITVECTORS: [usize; 18] = [
        flags::ITEM_WEAR_TAKE,
        flags::ITEM_WEAR_FINGER,
        flags::ITEM_WEAR_FINGER,
        flags::ITEM_WEAR_NECK,
        flags::ITEM_WEAR_NECK,
        flags::ITEM_WEAR_BODY,
        flags::ITEM_WEAR_HEAD,
        flags::ITEM_WEAR_LEGS,
        flags::ITEM_WEAR_FEET,
        flags::ITEM_WEAR_HANDS,
        flags::ITEM_WEAR_ARMS,
        flags::ITEM_WEAR_SHIELD,
        flags::ITEM_WEAR_ABOUT,
        flags::ITEM_WEAR_WAIST,
        flags::ITEM_WEAR_WRIST,
        flags::ITEM_WEAR_WRIST,
        flags::ITEM_WEAR_WIELD,
        flags::ITEM_WEAR_TAKE,
    ];
    const ALREADY_WEARING: [&[u8]; 18] = [
        b"You're already using a light.\r\n",
        b"YOU SHOULD NEVER SEE THIS MESSAGE.  PLEASE REPORT.\r\n",
        b"You're already wearing something on both of your ring fingers.\r\n",
        b"YOU SHOULD NEVER SEE THIS MESSAGE.  PLEASE REPORT.\r\n",
        b"You can't wear anything else around your neck.\r\n",
        b"You're already wearing something on your body.\r\n",
        b"You're already wearing something on your head.\r\n",
        b"You're already wearing something on your legs.\r\n",
        b"You're already wearing something on your feet.\r\n",
        b"You're already wearing something on your hands.\r\n",
        b"You're already wearing something on your arms.\r\n",
        b"You're already using a shield.\r\n",
        b"You're already wearing something about your body.\r\n",
        b"You already have something around your waist.\r\n",
        b"YOU SHOULD NEVER SEE THIS MESSAGE.  PLEASE REPORT.\r\n",
        b"You're already wearing something around both of your wrists.\r\n",
        b"You're already wielding a weapon.\r\n",
        b"You're already holding something.\r\n",
    ];

    // First, make sure that the wear position is valid.
    if !g.obj(oid).can_wear(WEAR_BITVECTORS[where_]) {
        act(g, b"You can't wear $p there.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
        return;
    }
    // For neck, finger, and wrist, try pos 2 if pos 1 is already full.
    if (where_ == WEAR_FINGER_R || where_ == WEAR_NECK_1 || where_ == WEAR_WRIST_R)
        && g.ch(chid).equipment[where_].is_some()
    {
        where_ += 1;
    }
    if g.ch(chid).equipment[where_].is_some() {
        send_to_char(g, chid, ALREADY_WEARING[where_]);
        return;
    }
    // See if a trigger disallows it (or moved the object off the char).
    if crate::dg::triggers::wear_otrigger(g, oid, chid, where_ as i32) == 0
        || g.try_obj(oid).map(|o| o.carried_by) != Some(Some(chid))
    {
        return;
    }
    wear_message(g, chid, oid, where_);
    obj_from_char(g, oid);
    handler::equip_char(g, chid, oid, where_);
}

/// Returns None after printing the bad-keyword message; the no-keyword
/// path picks the LAST matching flag.
fn find_eq_pos(g: &mut Game, chid: CharId, oid: ObjId, arg: &[u8]) -> Option<usize> {
    if arg.is_empty() {
        let mut where_: Option<usize> = None;
        let o = g.obj(oid);
        if o.can_wear(flags::ITEM_WEAR_FINGER) {
            where_ = Some(WEAR_FINGER_R);
        }
        if o.can_wear(flags::ITEM_WEAR_NECK) {
            where_ = Some(WEAR_NECK_1);
        }
        if o.can_wear(flags::ITEM_WEAR_BODY) {
            where_ = Some(WEAR_BODY);
        }
        if o.can_wear(flags::ITEM_WEAR_HEAD) {
            where_ = Some(WEAR_HEAD);
        }
        if o.can_wear(flags::ITEM_WEAR_LEGS) {
            where_ = Some(WEAR_LEGS);
        }
        if o.can_wear(flags::ITEM_WEAR_FEET) {
            where_ = Some(WEAR_FEET);
        }
        if o.can_wear(flags::ITEM_WEAR_HANDS) {
            where_ = Some(WEAR_HANDS);
        }
        if o.can_wear(flags::ITEM_WEAR_ARMS) {
            where_ = Some(WEAR_ARMS);
        }
        if o.can_wear(flags::ITEM_WEAR_SHIELD) {
            where_ = Some(WEAR_SHIELD);
        }
        if o.can_wear(flags::ITEM_WEAR_ABOUT) {
            where_ = Some(WEAR_ABOUT);
        }
        if o.can_wear(flags::ITEM_WEAR_WAIST) {
            where_ = Some(WEAR_WAIST);
        }
        if o.can_wear(flags::ITEM_WEAR_WRIST) {
            where_ = Some(WEAR_WRIST_R);
        }
        where_
    } else {
        const KEYWORDS: [&str; 18] = [
            "!RESERVED!",
            "finger",
            "!RESERVED!",
            "neck",
            "!RESERVED!",
            "body",
            "head",
            "legs",
            "feet",
            "hands",
            "arms",
            "shield",
            "about",
            "waist",
            "wrist",
            "!RESERVED!",
            "!RESERVED!",
            "!RESERVED!",
        ];
        match crate::act::informative::search_block(arg, &KEYWORDS) {
            Some(w) => Some(w),
            None => {
                let mut msg = b"'".to_vec();
                msg.extend_from_slice(arg);
                msg.extend_from_slice(b"'?  What part of your body is THAT?\r\n");
                send_to_char(g, chid, &msg);
                None
            }
        }
    }
}

pub fn do_wear(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2, _) = two_arguments(argument);

    if arg1.is_empty() {
        send_to_char(g, chid, b"Wear what?\r\n");
        return;
    }
    let (dotmode, name) = find_all_dots(&arg1);

    if !arg2.is_empty() && dotmode != FIND_INDIV {
        send_to_char(g, chid, b"You can't specify the same body location for more than one item!\r\n");
        return;
    }
    if dotmode == FIND_ALL {
        let mut items_worn = 0;
        let carrying = g.ch(chid).carrying.clone();
        for o in carrying {
            if !g.try_obj_alive(o) || !can_see_obj(g, chid, o) {
                continue;
            }
            if let Some(where_) = find_eq_pos(g, chid, o, b"") {
                if (g.ch(chid).level as i32) < g.obj(o).level {
                    send_to_char(g, chid, b"You are not experienced enough to use that.\r\n");
                } else {
                    items_worn += 1;
                    perform_wear(g, chid, o, where_);
                }
            }
        }
        if items_worn == 0 {
            send_to_char(g, chid, b"You don't seem to have anything wearable.\r\n");
        }
    } else if dotmode == FIND_ALLDOT {
        if name.is_empty() {
            send_to_char(g, chid, b"Wear all of what?\r\n");
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        let Some(first) = get_obj_in_list_vis(g, chid, &name, None, &carrying) else {
            let mut msg = b"You don't seem to have any ".to_vec();
            msg.extend_from_slice(&name);
            msg.extend_from_slice(b"s.\r\n");
            send_to_char(g, chid, &msg);
            return;
        };
        // NOTE, a quirk: the level gate applies only to the FIRST match.
        if (g.ch(chid).level as i32) < g.obj(first).level {
            send_to_char(g, chid, b"You are not experienced enough to use that.\r\n");
            return;
        }
        let mut obj = Some(first);
        while let Some(o) = obj {
            let list_after = carrying_after(g, chid, o);
            let next = get_obj_in_list_vis(g, chid, &name, None, &list_after);
            match find_eq_pos(g, chid, o, b"") {
                Some(where_) => perform_wear(g, chid, o, where_),
                None => {
                    act(g, b"You can't wear $p.", false, Some(chid), Some(o), None, comm::TO_CHAR);
                }
            }
            obj = next;
        }
    } else {
        let carrying = g.ch(chid).carrying.clone();
        let Some(obj) = get_obj_in_list_vis(g, chid, &name, None, &carrying) else {
            let mut msg = b"You don't seem to have ".to_vec();
            msg.extend_from_slice(an(&name));
            msg.push(b' ');
            msg.extend_from_slice(&name);
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
            return;
        };
        if (g.ch(chid).level as i32) < g.obj(obj).level {
            send_to_char(g, chid, b"You are not experienced enough to use that.\r\n");
        } else {
            match find_eq_pos(g, chid, obj, &arg2) {
                Some(where_) => perform_wear(g, chid, obj, where_),
                None => {
                    if arg2.is_empty() {
                        act(g, b"You can't wear $p.", false, Some(chid), Some(obj), None, comm::TO_CHAR);
                    }
                }
            }
        }
    }
}

pub fn do_wield(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Wield what?\r\n");
        return;
    }
    let carrying = g.ch(chid).carrying.clone();
    let Some(obj) = get_obj_in_list_vis(g, chid, &arg, None, &carrying) else {
        let mut msg = b"You don't seem to have ".to_vec();
        msg.extend_from_slice(an(&arg));
        msg.push(b' ');
        msg.extend_from_slice(&arg);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    if !g.obj(obj).can_wear(flags::ITEM_WEAR_WIELD) {
        send_to_char(g, chid, b"You can't wield that.\r\n");
    } else if obj_weight(g, obj) > tables::STR_APP[handler::strength_apply_index(g.ch(chid))].3 {
        send_to_char(g, chid, b"It's too heavy for you to use.\r\n");
    } else if (g.ch(chid).level as i32) < g.obj(obj).level {
        send_to_char(g, chid, b"You are not experienced enough to use that.\r\n");
    } else {
        perform_wear(g, chid, obj, WEAR_WIELD);
    }
}

pub fn do_grab(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Hold what?\r\n");
        return;
    }
    let carrying = g.ch(chid).carrying.clone();
    let Some(obj) = get_obj_in_list_vis(g, chid, &arg, None, &carrying) else {
        let mut msg = b"You don't seem to have ".to_vec();
        msg.extend_from_slice(an(&arg));
        msg.push(b' ');
        msg.extend_from_slice(&arg);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    if (g.ch(chid).level as i32) < g.obj(obj).level {
        send_to_char(g, chid, b"You are not experienced enough to use that.\r\n");
    } else if g.obj(obj).type_flag == flags::ITEM_LIGHT {
        perform_wear(g, chid, obj, WEAR_LIGHT);
    } else {
        let o = g.obj(obj);
        if !o.can_wear(flags::ITEM_WEAR_HOLD)
            && o.type_flag != flags::ITEM_WAND
            && o.type_flag != flags::ITEM_STAFF
            && o.type_flag != flags::ITEM_SCROLL
            && o.type_flag != flags::ITEM_POTION
        {
            send_to_char(g, chid, b"You can't hold that.\r\n");
        } else {
            perform_wear(g, chid, obj, WEAR_HOLD);
        }
    }
}

fn perform_remove(g: &mut Game, chid: CharId, pos: usize) {
    let Some(obj) = g.ch(chid).equipment[pos] else {
        g.log(format!("SYSERR: perform_remove: bad pos {} passed.", pos));
        return;
    };
    if g.obj(obj).obj_flagged(flags::ITEM_NODROP) && !g.ch(chid).prf(flags::PRF_NOHASSLE) {
        act(g, b"You can't remove $p, it must be CURSED!", false, Some(chid), Some(obj), None, comm::TO_CHAR);
    } else if (g.ch(chid).carry_items as i32) >= can_carry_n(g.ch(chid)) && !g.ch(chid).prf(flags::PRF_NOHASSLE) {
        act(g, b"$p: you can't carry that many items!", false, Some(chid), Some(obj), None, comm::TO_CHAR);
    } else {
        if crate::dg::triggers::remove_otrigger(g, obj, chid) == 0 {
            return;
        }
        if g.try_obj(obj).is_none() {
            return;
        }
        if let Some(o) = handler::unequip_char(g, chid, pos) {
            obj_to_char(g, o, chid);
        }
        act(g, b"You stop using $p.", false, Some(chid), Some(obj), None, comm::TO_CHAR);
        act(g, b"$n stops using $p.", true, Some(chid), Some(obj), None, comm::TO_ROOM);
    }
}

pub fn do_remove(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Remove what?\r\n");
        return;
    }
    let (dotmode, name) = find_all_dots(&arg);

    if dotmode == FIND_ALL {
        let mut found = false;
        for i in 0..NUM_WEARS {
            if g.ch(chid).equipment[i].is_some() {
                perform_remove(g, chid, i);
                found = true;
            }
        }
        if !found {
            send_to_char(g, chid, b"You're not using anything.\r\n");
        }
    } else if dotmode == FIND_ALLDOT {
        if name.is_empty() {
            send_to_char(g, chid, b"Remove all of what?\r\n");
        } else {
            let mut found = false;
            for i in 0..NUM_WEARS {
                let matches = match g.ch(chid).equipment[i] {
                    Some(oid) => can_see_obj(g, chid, oid) && isname(&name, obj_name(g, oid)),
                    None => false,
                };
                if matches {
                    perform_remove(g, chid, i);
                    found = true;
                }
            }
            if !found {
                let mut msg = b"You don't seem to be using any ".to_vec();
                msg.extend_from_slice(&name);
                msg.extend_from_slice(b"s.\r\n");
                send_to_char(g, chid, &msg);
            }
        }
    } else {
        match get_obj_pos_in_equip_vis(g, chid, &name, None) {
            None => {
                let mut msg = b"You don't seem to be using ".to_vec();
                msg.extend_from_slice(an(&name));
                msg.push(b' ');
                msg.extend_from_slice(&name);
                msg.extend_from_slice(b".\r\n");
                send_to_char(g, chid, &msg);
            }
            Some(i) => perform_remove(g, chid, i),
        }
    }
}

// ---- sacrifice ----

/// do_sac. The reversed `\n\r` newlines are on the cosmetic-fix list
/// list; `\r\n` is emitted here instead.
pub fn do_sac(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Sacrifice what?\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    let contents = g.rooms[room as usize].contents.clone();
    let j = match get_obj_in_list_vis(g, chid, &arg, None, &contents) {
        Some(o) => o,
        None => {
            let carrying = g.ch(chid).carrying.clone();
            match get_obj_in_list_vis(g, chid, &arg, None, &carrying) {
                Some(o) => o,
                None => {
                    send_to_char(g, chid, b"It doesn't seem to be here.\r\n");
                    return;
                }
            }
        }
    };

    if !g.obj(j).can_wear(flags::ITEM_WEAR_TAKE) {
        send_to_char(g, chid, b"You can't sacrifice that!\r\n");
        return;
    }

    act(g, b"$n sacrifices $p.", false, Some(chid), Some(j), None, comm::TO_ROOM);

    let short = obj_short(g, j).to_vec();
    let obj_level = g.obj(j).level;
    match g.rng.rand_number(0, 5) {
        0 => {
            let mut msg = b"You sacrifice ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(b" to the Gods.\r\nYou receive one gold coin for your humility.\r\n");
            send_to_char(g, chid, &msg);
            increase_gold(g, chid, 1);
        }
        1 => {
            let mut msg = b"You sacrifice ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(b" to the Gods.\r\nThe Gods ignore your sacrifice.\r\n");
            send_to_char(g, chid, &msg);
        }
        2 => {
            let exp = 1 + 2 * obj_level;
            let mut msg = b"You sacrifice ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(format!(" to the Gods.\r\nThe gods give you {} experience points.\r\n", exp).as_bytes());
            send_to_char(g, chid, &msg);
            g.ch_mut(chid).points.exp += exp;
        }
        3 => {
            let exp = 1 + obj_level;
            let mut msg = b"You sacrifice ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(format!(" to the Gods.\r\nYou receive {} experience points.\r\n", exp).as_bytes());
            send_to_char(g, chid, &msg);
            g.ch_mut(chid).points.exp += exp;
        }
        4 => {
            let gold = 1 + obj_level;
            send_to_char(g, chid, format!("Your sacrifice to the Gods is rewarded with {} gold coins.\r\n", gold).as_bytes());
            increase_gold(g, chid, gold);
        }
        5 => {
            let gold = 1 + 2 * obj_level;
            // No period before the CRLF — kept.
            send_to_char(g, chid, format!("Your sacrifice to the Gods is rewarded with {} gold coins\r\n", gold).as_bytes());
            increase_gold(g, chid, gold);
        }
        _ => {
            let mut msg = b"You sacrifice ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(b" to the Gods.\r\nYou receive one gold coin for your humility.\r\n");
            send_to_char(g, chid, &msg);
            increase_gold(g, chid, 1);
        }
    }
    // Spill contents. B15: a CARRIED container has no room of its own, and
    // dropping its contents into room NOWHERE orphans them with SYSERR
    // spam. The room lookup falls back to the sacrificer's room instead.
    let spill_room = if g.obj(j).in_room != NOWHERE { g.obj(j).in_room } else { room };
    let spill: Vec<ObjId> = g.obj(j).contains.clone();
    for jj in spill {
        obj_from_obj(g, jj);
        obj_to_room(g, jj, spill_room);
    }
    extract_obj(g, j);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_dots_modes() {
        assert_eq!(find_all_dots(b"all"), (FIND_ALL, b"all".to_vec()));
        assert_eq!(find_all_dots(b"all.bread"), (FIND_ALLDOT, b"bread".to_vec()));
        assert_eq!(find_all_dots(b"all."), (FIND_ALLDOT, b"".to_vec()));
        assert_eq!(find_all_dots(b"bread"), (FIND_INDIV, b"bread".to_vec()));
    }

    #[test]
    fn an_vowels() {
        assert_eq!(an(b"apple"), b"an");
        assert_eq!(an(b"Orange"), b"an");
        assert_eq!(an(b"bread"), b"a");
        assert_eq!(an(b""), b"a");
    }

    /// The quirks: end-boundary only, leading space survives, repeated
    /// occurrences all removed.
    #[test]
    fn remove_from_string_quirks() {
        assert_eq!(remove_from_string(b"bottle dark ale", b"ale"), b"bottle dark ".to_vec());
        assert_eq!(remove_from_string(b"water bottle", b"water"), b" bottle".to_vec());
        // No boundary check at match START: "saltwater" tail matches.
        assert_eq!(remove_from_string(b"saltwater jug", b"water"), b"salt jug".to_vec());
        assert_eq!(remove_from_string(b"ale ale keg", b"ale"), b"  keg".to_vec());
        assert_eq!(remove_from_string(b"barrel", b"ale"), b"barrel".to_vec());
    }

    #[test]
    fn right_trim_whitespace_trims_nonprint() {
        assert_eq!(right_trim_whitespace(b"bottle dark "), b"bottle dark".to_vec());
        assert_eq!(right_trim_whitespace(b"  "), b"".to_vec());
        assert_eq!(right_trim_whitespace(b"jug\t\x01"), b"jug".to_vec());
    }
}
