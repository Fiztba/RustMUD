//! The object half of the generic OLC library.
//!
//! `update_all_objects` overwrites every live instance with the whole new
//! prototype, keeping only the placement fields (room, carrier, container,
//! contents, script id). Everything else — values, timer, flags, and the
//! live script container — is replaced, which is why oedit re-attaches
//! triggers to the instances afterwards.

use mud_data::types::*;
use mud_world::model::ObjProto;

use crate::db::{
    add_to_save_list, in_save_list, remove_from_save_list, write_world_file, SL_OBJ, SL_ZON,
};
use crate::game::{Game, MudlogKind};

pub fn add_object(g: &mut Game, newobj: &ObjProto, ovnum: Idx) -> Option<Idx> {
    let rznum = crate::dg::mobcmd::real_zone_by_thing(g, ovnum as i32);
    if let Some(rnum) = g.world.real_object(ovnum) {
        let mut copy = newobj.clone();
        copy.vnum = ovnum;
        g.world.obj_protos[rnum as usize] = copy;
        update_all_objects(g, rnum);
        if let Some(z) = rznum {
            let zvnum = g.world.zones[z].number;
            add_to_save_list(g, zvnum, SL_OBJ);
        }
        return Some(rnum);
    }

    let found = insert_object(g, newobj, ovnum);
    adjust_objects(g, found);
    if let Some(z) = rznum {
        let zvnum = g.world.zones[z].number;
        add_to_save_list(g, zvnum, SL_OBJ);
    }
    Some(found)
}

fn update_all_objects(g: &mut Game, rnum: Idx) -> i32 {
    let proto = g.world.obj_protos[rnum as usize].clone();
    let mut count = 0;
    for id in g.object_list.clone() {
        let Some(o) = g.objs.get_mut(id) else { continue };
        if o.item_number != rnum {
            continue;
        }
        count += 1;
        // *obj = *refobj, then the placement fields are put back.
        o.values = proto.values;
        o.type_flag = proto.type_flag;
        o.wear_flags = mud_data::flags::FlagSet::from_words(proto.wear_flags);
        o.extra_flags = mud_data::flags::FlagSet::from_words(proto.extra_flags);
        o.perm_affects = mud_data::flags::FlagSet::from_words(proto.perm_affects);
        o.weight = proto.weight;
        o.cost = proto.cost;
        o.cost_per_day = proto.cost_per_day;
        o.level = proto.level;
        o.timer = proto.timer;
        o.affected = proto.affected;
        // The instance's own strings came from the prototype, and the
        // prototype's are what it shows again.
        o.name = None;
        o.short_description = None;
        o.description = None;
        o.action_description = None;
        o.ex_descriptions = None;
        o.proto_script = proto.proto_script.clone();
        // SCRIPT(obj) comes from the prototype too — i.e. NULL.
        o.script = None;
    }
    count
}

/// adjust_objects: fix every rnum reference as if an
/// object had been inserted at `refpt`.
pub fn adjust_objects(g: &mut Game, refpt: Idx) -> Option<Idx> {
    if refpt == NOTHING || refpt as usize >= g.world.obj_protos.len() {
        return None;
    }
    for id in g.object_list.clone() {
        if let Some(o) = g.objs.get_mut(id) {
            if o.item_number != NOTHING && o.item_number >= refpt {
                o.item_number += 1;
            }
        }
    }
    for zi in 0..g.world.zones.len() {
        for cmd in g.world.zones[zi].cmds.iter_mut() {
            match cmd.command {
                b'P' => {
                    if cmd.arg3 >= refpt as i32 {
                        cmd.arg3 += 1;
                    }
                    // Deliberate fall-through into the O/G/E arm.
                    if cmd.arg1 >= refpt as i32 {
                        cmd.arg1 += 1;
                    }
                }
                b'O' | b'G' | b'E' => {
                    if cmd.arg1 >= refpt as i32 {
                        cmd.arg1 += 1;
                    }
                }
                b'R' => {
                    if cmd.arg2 >= refpt as i32 {
                        cmd.arg2 += 1;
                    }
                }
                _ => {}
            }
        }
    }
    // Notice boards. No NOTHING guard here — deliberate.
    for r in g.boards.rnum.iter_mut() {
        if *r >= refpt {
            *r += 1;
        }
    }
    // Shop produce.
    for s in g.shops_rt.iter_mut() {
        for p in s.producing.iter_mut() {
            if *p != NOTHING && *p >= refpt {
                *p += 1;
            }
        }
    }
    Some(refpt)
}

/// insert_object + index_object.
fn insert_object(g: &mut Game, obj: &ObjProto, ovnum: Idx) -> Idx {
    let old_len = g.world.obj_protos.len();
    let mut found: usize = 0;
    for i in (1..=old_len).rev() {
        if ovnum > g.world.obj_protos[i - 1].vnum {
            found = i;
            break;
        }
    }
    let mut copy = obj.clone();
    copy.vnum = ovnum;
    g.world.obj_protos.insert(found, copy);
    g.obj_counts.insert(found, 0);
    g.obj_specs.insert(found, None);
    for v in g.world.obj_map.values_mut() {
        if *v as usize >= found {
            *v += 1;
        }
    }
    g.world.obj_map.insert(ovnum, found as Idx);
    found as Idx
}

pub fn delete_object(g: &mut Game, rnum: Idx) -> Option<Idx> {
    if rnum == NOTHING || rnum as usize >= g.world.obj_protos.len() {
        return None;
    }
    let vnum = g.world.obj_protos[rnum as usize].vnum;
    let zrnum = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32);

    let sdesc = g.world.obj_protos[rnum as usize]
        .short_description
        .clone()
        .map(|s| String::from_utf8_lossy(&s).into_owned())
        .unwrap_or_else(|| "(null)".to_string());
    g.log(format!("GenOLC: delete_object: Deleting object #{} ({}).", vnum, sdesc));

    for oid in g.object_list.clone() {
        if g.objs.get(oid).map(|o| o.item_number) != Some(rnum) {
            continue;
        }
        // extract_obj would just axe the contents; move them out first.
        let contents = g.objs.get(oid).map(|o| o.contains.clone()).unwrap_or_default();
        for cid in contents {
            let (in_room, worn_by, carried_by, in_obj) = {
                let o = g.obj(oid);
                (o.in_room, o.worn_by, o.carried_by, o.in_obj)
            };
            // Testing `if (IN_ROOM(tmp))` is a truth test on an
            // rnum, where NOWHERE is 65535 and therefore true — so carried
            // and worn containers took the room branch and spilled into
            // obj_to_room(NOWHERE). Its person branch then used
            // obj_from_char on a contained object and handed a worn
            // container's NULL carried_by to obj_to_char. Same mistake
            // B15 fixed in do_sac.
            if in_room != NOWHERE {
                crate::handler::obj_from_obj(g, cid);
                crate::handler::obj_to_room(g, cid, in_room);
            } else if carried_by.is_some() || worn_by.is_some() {
                crate::handler::obj_from_obj(g, cid);
                if let Some(ch) = carried_by.or(worn_by) {
                    crate::handler::obj_to_char(g, cid, ch);
                }
            } else if in_obj.is_some() {
                crate::handler::obj_from_obj(g, cid);
                if let Some(parent) = in_obj {
                    crate::handler::obj_to_obj(g, cid, parent);
                }
            }
        }
        crate::handler::extract_obj(g, oid);
    }

    // Adjust rnums of all other objects.
    for id in g.object_list.clone() {
        if let Some(o) = g.objs.get_mut(id) {
            if o.item_number > rnum {
                o.item_number -= 1;
            }
        }
    }

    g.world.obj_protos.remove(rnum as usize);
    g.obj_counts.remove(rnum as usize);
    g.obj_specs.remove(rnum as usize);
    g.world.obj_map.remove(&vnum);
    for v in g.world.obj_map.values_mut() {
        if *v > rnum {
            *v -= 1;
        }
    }

    for r in g.boards.rnum.iter_mut() {
        if *r > rnum {
            *r -= 1;
        }
    }
    for s in g.shops_rt.iter_mut() {
        for p in s.producing.iter_mut() {
            if *p != NOTHING && *p > rnum {
                *p -= 1;
            }
        }
    }

    // Zone commands. The 'P' arm falls through to O/G/E, so a
    // deleted P command is followed by an arg1 test against whatever
    // command shifted into its slot.
    // Every zone whose table changes here needs writing back out.
    let mut touched: Vec<Idx> = Vec::new();
    for zi in 0..g.world.zones.len() {
        let mut ci = 0usize;
        let mut zone_touched = false;
        while ci < g.world.zones[zi].cmds.len() {
            let command = g.world.zones[zi].cmds[ci].command;
            match command {
                b'P' => {
                    if g.world.zones[zi].cmds[ci].arg3 == rnum as i32 {
                        g.world.zones[zi].cmds.remove(ci);
                        zone_touched = true;
                    } else if g.world.zones[zi].cmds[ci].arg3 > rnum as i32 {
                        g.world.zones[zi].cmds[ci].arg3 -= 1;
                        zone_touched = true;
                    }
                    if ci < g.world.zones[zi].cmds.len() {
                        if g.world.zones[zi].cmds[ci].arg1 == rnum as i32 {
                            g.world.zones[zi].cmds.remove(ci);
                            zone_touched = true;
                        } else if g.world.zones[zi].cmds[ci].arg1 > rnum as i32 {
                            g.world.zones[zi].cmds[ci].arg1 -= 1;
                            zone_touched = true;
                        }
                    }
                }
                b'O' | b'G' | b'E' => {
                    if g.world.zones[zi].cmds[ci].arg1 == rnum as i32 {
                        g.world.zones[zi].cmds.remove(ci);
                        zone_touched = true;
                    } else if g.world.zones[zi].cmds[ci].arg1 > rnum as i32 {
                        g.world.zones[zi].cmds[ci].arg1 -= 1;
                        zone_touched = true;
                    }
                }
                b'R' => {
                    if g.world.zones[zi].cmds[ci].arg2 == rnum as i32 {
                        g.world.zones[zi].cmds.remove(ci);
                        zone_touched = true;
                    } else if g.world.zones[zi].cmds[ci].arg2 > rnum as i32 {
                        g.world.zones[zi].cmds[ci].arg2 -= 1;
                        zone_touched = true;
                    }
                }
                _ => {}
            }
            ci += 1;
        }
        if zone_touched {
            touched.push(g.world.zones[zi].number);
        }
    }
    for zvnum in touched {
        add_to_save_list(g, zvnum, SL_ZON);
    }

    // Flag rather than write; oedit's delete branch honours the toggle.
    if let Some(z) = zrnum {
        let zvnum = g.world.zones[z].number;
        add_to_save_list(g, zvnum, SL_OBJ);
    }
    Some(rnum)
}

pub fn save_objects(g: &mut Game, zone_num: Option<usize>) -> bool {
    let top = g.world.zones.len().saturating_sub(1);
    let Some(zone_num) = zone_num.filter(|&z| z < g.world.zones.len()) else {
        g.log(format!(
            "SYSERR: GenOLC: save_objects: Invalid real zone number {}. (0-{})",
            NOWHERE, top
        ));
        return false;
    };
    let vznum = g.world.zones[zone_num].number;
    if write_world_file(g, zone_num, SL_OBJ).is_none() {
        let msg = format!("SYSERR: OLC: Cannot open objects file world/obj/{}.new!", vznum);
        g.mudlog(MudlogKind::Brf, LVL_IMMORT, true, &msg);
        return false;
    }
    if in_save_list(g, vznum, SL_OBJ) {
        remove_from_save_list(g, vznum, SL_OBJ);
    }
    true
}
