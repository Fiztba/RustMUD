//! Runtime world database: mob/obj instantiation from prototypes, zone
//! resets, and the reset scheduler. RNG call order at instantiation (mob hp
//! dice) is observable, as are reset_zone's exact
//! last_cmd/tmob/tobj state semantics — error paths skip the clears.

use mud_data::flags::{self, FlagSet};
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::ch::Char;
use crate::game::{Game, MudlogKind, ZO_DEAD};
use crate::handler::{char_to_room, equip_char, extract_obj, obj_to_char, obj_to_obj, obj_to_room};
use crate::obj::Obj;

pub const MOB_TRIGGER: i32 = 0;
pub const OBJ_TRIGGER: i32 = 1;
pub const WLD_TRIGGER: i32 = 2;

/// read_mobile, REAL-type. Returns None for bad rnum.
pub fn read_mobile(g: &mut Game, rnum: Idx) -> Option<CharId> {
    let proto = g.world.mob_protos.get(rnum as usize)?.clone();
    let mut mob = Char {
        mob_rnum: rnum,
        name: proto.keywords.clone(),
        short_descr: proto.short_descr.clone(),
        long_descr: proto.long_descr.clone(),
        description: proto.ddescription.clone(),
        act: FlagSet::from_words(proto.act),
        affected_by: FlagSet::from_words(proto.affected_by),
        alignment: proto.alignment,
        level: proto.level.clamp(0, 255) as u8,
        sex: proto.sex.clamp(0, 255) as u8,
        class: 0,
        weight: 200,
        height: 198,
        position: proto.position.clamp(0, 255) as u8,
        ..Default::default()
    };
    // The creation set, kept with the mob: see `MobSpecials::innate_aff`.
    mob.mob_specials.innate_aff = FlagSet::from_words(proto.affected_by);
    mob.mob_specials.default_pos = proto.default_pos.clamp(0, 255) as u8;
    mob.mob_specials.attack_type = proto.bare_hand_attack.unwrap_or(0);
    mob.mob_specials.damnodice = proto.damnodice.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    mob.mob_specials.damsizedice = proto.damsizedice.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    mob.points.hitroll = proto.hitroll.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    mob.points.damroll = proto.damroll.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    mob.points.armor = proto.armor;
    mob.points.gold = proto.gold;
    mob.points.exp = proto.exp;
    mob.points.max_mana = 10;
    mob.points.max_move = 50;
    mob.real_abils.str_ = proto.str_.unwrap_or(11) as i8;
    mob.real_abils.str_add = proto.str_add.unwrap_or(0) as i8;
    mob.real_abils.intel = proto.intel.unwrap_or(11) as i8;
    mob.real_abils.wis = proto.wis.unwrap_or(11) as i8;
    mob.real_abils.dex = proto.dex.unwrap_or(11) as i8;
    mob.real_abils.con = proto.con.unwrap_or(11) as i8;
    mob.real_abils.cha = proto.cha.unwrap_or(11) as i8;
    mob.aff_abils = mob.real_abils;
    mob.apply_saving_throw = [
        proto.saving_para.unwrap_or(0) as i16,
        proto.saving_rod.unwrap_or(0) as i16,
        proto.saving_petri.unwrap_or(0) as i16,
        proto.saving_breath.unwrap_or(0) as i16,
        proto.saving_spell.unwrap_or(0) as i16,
    ];
    // Hit points: proto (hit, mana, mov) hold XdY+Z. A
    // file-loaded proto always has max_hit 0, selecting the dice path.
    mob.points.max_hit = g.rng.dice(proto.hit, proto.mana) + proto.mov;
    mob.points.hit = mob.points.max_hit;
    mob.points.mana = mob.points.max_mana;
    mob.points.mov = mob.points.max_move;
    mob.time.birth = g.now;
    mob.time.played = 0;
    mob.time.logon = g.now;
    mob.proto_script = proto.proto_script.clone();

    let id = g.chars.insert(mob);
    g.character_list.push_front(id);
    g.mob_counts[rnum as usize] += 1;
    mud_data::rng::rng_trace_note(&format!(
        "read_mobile: {}",
        String::from_utf8_lossy(g.ch(id).get_name())
    ));
    // copy_proto_script + assign_triggers.
    crate::dg::assign_triggers(g, crate::dg::GoId::Char(id));
    Some(id)
}

/// read_object, REAL-type.
pub fn read_object(g: &mut Game, rnum: Idx) -> Option<ObjId> {
    let proto = g.world.obj_protos.get(rnum as usize)?.clone();
    let obj = Obj {
        item_number: rnum,
        in_room: NOWHERE,
        values: proto.values,
        type_flag: proto.type_flag,
        wear_flags: FlagSet::from_words(proto.wear_flags),
        extra_flags: FlagSet::from_words(proto.extra_flags),
        perm_affects: FlagSet::from_words(proto.perm_affects),
        weight: proto.weight,
        cost: proto.cost,
        cost_per_day: proto.cost_per_day,
        level: proto.level,
        timer: proto.timer,
        affected: proto.affected,
        name: None,
        short_description: None,
        description: None,
        action_description: None,
        ex_descriptions: None,
        carried_by: None,
        worn_by: None,
        worn_on: -1,
        in_obj: None,
        contains: Vec::new(),
        sat_in_by: None,
        proto_script: proto.proto_script.clone(),
        script_id: 0,
        script: None,
    };
    let id = g.objs.insert(obj);
    g.object_list.push_front(id);
    g.obj_counts[rnum as usize] += 1;
    // copy_proto_script + assign_triggers.
    crate::dg::assign_triggers(g, crate::dg::GoId::Obj(id));
    Some(id)
}

/// is_empty(zone).
pub fn zone_is_empty(g: &Game, zone: ZoneRnum) -> bool {
    for &di in &g.descriptors.order {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.in_room == NOWHERE {
            continue;
        }
        if g.world.rooms[ch.in_room as usize].zone != zone {
            continue;
        }
        if ch.level >= LVL_IMMORT && ch.prf(flags::PRF_NOHASSLE) {
            continue;
        }
        return false;
    }
    true
}

/// zone_update: minute-based aging plus at most one queued
/// reset per 10-second call.
pub fn zone_update(g: &mut Game) {
    g.zone_timer += 1;
    if (g.zone_timer * PULSE_ZONE) / PASSES_PER_SEC >= 60 {
        g.zone_timer = 0;
        for zr in 0..g.world.zones.len() {
            let (lifespan, reset_mode) = {
                let z = &g.world.zones[zr];
                (z.lifespan, z.reset_mode)
            };
            if g.zones_rt[zr].age < lifespan && reset_mode != 0 {
                g.zones_rt[zr].age += 1;
            }
            if g.zones_rt[zr].age >= lifespan && g.zones_rt[zr].age < ZO_DEAD && reset_mode != 0 {
                g.reset_q.push_back(zr as ZoneRnum);
                g.zones_rt[zr].age = ZO_DEAD;
            }
        }
    }

    let mut chosen = None;
    for (qi, &zr) in g.reset_q.iter().enumerate() {
        let zone = &g.world.zones[zr as usize];
        if zone.reset_mode == 2 || zone_is_empty(g, zr) {
            chosen = Some((qi, zr));
            break;
        }
    }
    if let Some((qi, zr)) = chosen {
        reset_zone(g, zr as usize);
        let (name, number) = {
            let z = &g.world.zones[zr as usize];
            (String::from_utf8_lossy(z.name.as_deref().unwrap_or(b"")).into_owned(), z.number)
        };
        // mudlog CMP at LVL_IMPL+1 without file echo, so nobody sees it
        // unless they are looking for it.
        g.mudlog(MudlogKind::Cmp, LVL_IMPL + 1, false, &format!("Auto zone reset: {} (Zone {})", name, number));
        for di in g.descriptors.indices() {
            let Some(d) = g.descriptors.get(di) else { continue };
            if !d.is_playing() {
                continue;
            }
            let Some(chid) = d.character else { continue };
            let Some(ch) = g.try_ch(chid) else { continue };
            if !ch.prf(flags::PRF_ZONERESETS) {
                continue;
            }
            let mut line = Vec::new();
            line.extend_from_slice(crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KGRN));
            line.extend_from_slice(format!("[Auto zone reset: {} (Zone {})]", name, number).as_bytes());
            line.extend_from_slice(crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM));
            crate::comm::send_to_char(g, chid, &line);
        }
        g.reset_q.remove(qi);
    }
}

fn log_zone_error(g: &mut Game, zr: usize, cmd_no: usize, message: &str) {
    let (cmd_ch, line, znum) = {
        let z = &g.world.zones[zr];
        let c = &z.cmds[cmd_no];
        (c.command as char, c.line, z.number)
    };
    g.mudlog(MudlogKind::Nrm, LVL_GOD, true, &format!("SYSERR: zone file: {}", message));
    g.mudlog(
        MudlogKind::Nrm,
        LVL_GOD,
        true,
        &format!("SYSERR: ...offending cmd: '{}' cmd in zone #{}, line {}", cmd_ch, znum, line),
    );
}

pub fn reset_zone(g: &mut Game, zr: usize) {
    let mut last_cmd = 0i32;
    let mut mob: Option<CharId> = None;
    let mut tmob: Option<CharId> = None;
    let mut tobj: Option<ObjId> = None;

    let cmd_count = g.world.zones[zr].cmds.len();
    for cmd_no in 0..cmd_count {
        let cmd = g.world.zones[zr].cmds[cmd_no].clone();
        if cmd.command == b'S' {
            break;
        }
        if cmd.if_flag != 0 && last_cmd == 0 {
            continue;
        }
        match cmd.command {
            b'*' => {
                last_cmd = 0;
            }
            b'M' => {
                if g.mob_counts[cmd.arg1 as usize] < cmd.arg2 {
                    let id = read_mobile(g, cmd.arg1 as Idx).expect("renum guarantees mob rnum");
                    char_to_room(g, id, cmd.arg3 as RoomRnum);
                    crate::dg::triggers::load_mtrigger(g, id);
                    mob = Some(id);
                    tmob = Some(id);
                    last_cmd = 1;
                } else {
                    last_cmd = 0;
                }
                tobj = None;
            }
            b'O' => {
                if g.obj_counts[cmd.arg1 as usize] < cmd.arg2 {
                    if cmd.arg3 != NOWHERE as i32 {
                        let id = read_object(g, cmd.arg1 as Idx).expect("renum guarantees obj rnum");
                        obj_to_room(g, id, cmd.arg3 as RoomRnum);
                        last_cmd = 1;
                        crate::dg::triggers::load_otrigger(g, id);
                        tobj = Some(id);
                    } else {
                        let id = read_object(g, cmd.arg1 as Idx).expect("renum guarantees obj rnum");
                        g.obj_mut(id).in_room = NOWHERE;
                        last_cmd = 1;
                        tobj = Some(id);
                    }
                } else {
                    last_cmd = 0;
                }
                tmob = None;
            }
            b'P' => {
                if g.obj_counts[cmd.arg1 as usize] < cmd.arg2 {
                    let id = read_object(g, cmd.arg1 as Idx).expect("renum guarantees obj rnum");
                    let container = g
                        .object_list
                        .iter()
                        .copied()
                        .find(|o| g.objs.get(*o).map(|ob| ob.item_number) == Some(cmd.arg3 as Idx));
                    let Some(cid) = container else {
                        // The error path breaks BEFORE the tmob clear, so
                        // the freshly read object stays in limbo on
                        // object_list.
                        log_zone_error(g, zr, cmd_no, "target obj not found, command disabled");
                        g.world.zones[zr].cmds[cmd_no].command = b'*';
                        last_cmd = 0;
                        continue;
                    };
                    obj_to_obj(g, id, cid);
                    last_cmd = 1;
                    crate::dg::triggers::load_otrigger(g, id);
                    tobj = Some(id);
                } else {
                    last_cmd = 0;
                }
                tmob = None;
            }
            b'G' => {
                let Some(mid) = mob else {
                    let vnum = g.world.obj_protos.get(cmd.arg1 as usize).map(|p| p.vnum).unwrap_or(0);
                    log_zone_error(
                        g,
                        zr,
                        cmd_no,
                        &format!("attempt to give obj #{} to non-existant mob, command disabled", vnum),
                    );
                    g.world.zones[zr].cmds[cmd_no].command = b'*';
                    last_cmd = 0;
                    continue; // tmob/tobj untouched (C breaks pre-clear)
                };
                if g.obj_counts[cmd.arg1 as usize] < cmd.arg2 {
                    let id = read_object(g, cmd.arg1 as Idx).expect("renum guarantees obj rnum");
                    obj_to_char(g, id, mid);
                    last_cmd = 1;
                    crate::dg::triggers::load_otrigger(g, id);
                    tobj = Some(id);
                } else {
                    last_cmd = 0;
                }
                tmob = None;
            }
            b'E' => {
                let Some(mid) = mob else {
                    let vnum = g.world.obj_protos.get(cmd.arg1 as usize).map(|p| p.vnum).unwrap_or(0);
                    log_zone_error(
                        g,
                        zr,
                        cmd_no,
                        &format!("trying to equip non-existant mob with obj #{}, command disabled", vnum),
                    );
                    g.world.zones[zr].cmds[cmd_no].command = b'*';
                    last_cmd = 0;
                    continue;
                };
                if g.obj_counts[cmd.arg1 as usize] < cmd.arg2 {
                    if cmd.arg3 < 0 || cmd.arg3 >= NUM_WEARS as i32 {
                        // A quirk kept deliberately: the message indexes
                        // the prototype named by the LIMIT arg (arg2), not the
                        // object.
                        let mobname = String::from_utf8_lossy(g.ch(mid).get_name()).into_owned();
                        let bogus_vnum =
                            g.world.obj_protos.get(cmd.arg2 as usize).map(|p| p.vnum).unwrap_or(0);
                        log_zone_error(
                            g,
                            zr,
                            cmd_no,
                            &format!(
                                "invalid equipment pos number (mob {}, obj {}, pos {})",
                                mobname, bogus_vnum, cmd.arg3
                            ),
                        );
                        last_cmd = 0;
                    } else {
                        let id = read_object(g, cmd.arg1 as Idx).expect("renum guarantees obj rnum");
                        // IN_ROOM(obj) points at the mob around the trigger
                        // pair so scripts see a location.
                        let mob_room = g.ch(mid).in_room;
                        g.obj_mut(id).in_room = mob_room;
                        crate::dg::triggers::load_otrigger(g, id);
                        if g.try_obj(id).is_some() {
                            if crate::dg::triggers::wear_otrigger(g, id, mid, cmd.arg3) != 0
                                && g.try_obj(id).is_some()
                            {
                                g.obj_mut(id).in_room = NOWHERE;
                                equip_char(g, mid, id, cmd.arg3 as usize);
                            } else if g.try_obj(id).is_some() {
                                g.obj_mut(id).in_room = NOWHERE;
                                obj_to_char(g, id, mid);
                            }
                        }
                        tobj = Some(id);
                        last_cmd = 1;
                    }
                } else {
                    last_cmd = 0;
                }
                tmob = None;
            }
            b'R' => {
                let room = cmd.arg1 as usize;
                let found = g.rooms[room]
                    .contents
                    .iter()
                    .copied()
                    .find(|o| g.objs.get(*o).map(|ob| ob.item_number) == Some(cmd.arg2 as Idx));
                if let Some(oid) = found {
                    extract_obj(g, oid);
                }
                last_cmd = 1;
                tmob = None;
                tobj = None;
            }
            b'D' => {
                let room = cmd.arg1 as usize;
                if cmd.arg2 < 0
                    || cmd.arg2 >= crate::fight::dir_count(g) as i32
                    || g.world.rooms[room].dir_option[cmd.arg2 as usize].is_none()
                {
                    let rvnum = g.world.rooms[room].vnum;
                    log_zone_error(
                        g,
                        zr,
                        cmd_no,
                        &format!("door does not exist in room {} - dir {}, command disabled", rvnum, cmd.arg2),
                    );
                    g.world.zones[zr].cmds[cmd_no].command = b'*';
                } else {
                    let exit = g.world.rooms[room].dir_option[cmd.arg2 as usize].as_mut().unwrap();
                    match cmd.arg3 {
                        0 => {
                            exit.exit_info &= !flags::EX_LOCKED;
                            exit.exit_info &= !flags::EX_CLOSED;
                        }
                        1 => {
                            exit.exit_info |= flags::EX_CLOSED;
                            exit.exit_info &= !flags::EX_LOCKED;
                        }
                        2 => {
                            exit.exit_info |= flags::EX_LOCKED;
                            exit.exit_info |= flags::EX_CLOSED;
                        }
                        _ => {}
                    }
                }
                // Unconditional even on the error path.
                last_cmd = 1;
                tmob = None;
                tobj = None;
            }
            b'T' => {
                // Live attach; renum guarantees arg2 is a
                // trigger rnum. Zone T attach never touches proto_script.
                if cmd.arg1 == MOB_TRIGGER && tmob.is_some() {
                    if let Some(t) = crate::dg::read_trigger(g, cmd.arg2 as Idx) {
                        let go = crate::dg::GoId::Char(tmob.unwrap());
                        crate::dg::add_trigger_at(g.ensure_script(go), t, -1);
                    }
                    last_cmd = 1;
                } else if cmd.arg1 == OBJ_TRIGGER && tobj.is_some() {
                    if let Some(t) = crate::dg::read_trigger(g, cmd.arg2 as Idx) {
                        let go = crate::dg::GoId::Obj(tobj.unwrap());
                        crate::dg::add_trigger_at(g.ensure_script(go), t, -1);
                    }
                    last_cmd = 1;
                } else if cmd.arg1 == WLD_TRIGGER {
                    if cmd.arg3 == NOWHERE as i32 || (cmd.arg3 as usize) >= g.world.rooms.len() {
                        log_zone_error(g, zr, cmd_no, "Invalid room number in trigger assignment");
                        last_cmd = 0;
                        // Do not attach: the room index is out of range (F2).
                    } else {
                        if let Some(t) = crate::dg::read_trigger(g, cmd.arg2 as Idx) {
                            let go = crate::dg::GoId::Room(cmd.arg3 as RoomRnum);
                            crate::dg::add_trigger_at(g.ensure_script(go), t, -1);
                        }
                        last_cmd = 1;
                    }
                }
                // T does not clear tmob/tobj; they carry over.
            }
            b'V' => {
                // Mob/obj V lines pass arg3, which is already a room rnum,
                // as the context.
                if cmd.arg1 == MOB_TRIGGER && tmob.is_some() {
                    let go = crate::dg::GoId::Char(tmob.unwrap());
                    if g.script_of(go).is_none() {
                        log_zone_error(g, zr, cmd_no, "Attempt to give variable to scriptless mobile");
                    } else if let Some(sc) = g.script_of_mut(go) {
                        crate::dg::add_var(&mut sc.global_vars, cmd.sarg1.as_deref().unwrap_or(b""), cmd.sarg2.as_deref().unwrap_or(b""), cmd.arg3 as i64);
                    }
                    last_cmd = 1;
                } else if cmd.arg1 == OBJ_TRIGGER && tobj.is_some() {
                    let go = crate::dg::GoId::Obj(tobj.unwrap());
                    if g.script_of(go).is_none() {
                        log_zone_error(g, zr, cmd_no, "Attempt to give variable to scriptless object");
                    } else if let Some(sc) = g.script_of_mut(go) {
                        crate::dg::add_var(&mut sc.global_vars, cmd.sarg1.as_deref().unwrap_or(b""), cmd.sarg2.as_deref().unwrap_or(b""), cmd.arg3 as i64);
                    }
                    last_cmd = 1;
                } else if cmd.arg1 == WLD_TRIGGER {
                    if cmd.arg3 == NOWHERE as i32 || (cmd.arg3 as usize) >= g.world.rooms.len() {
                        log_zone_error(g, zr, cmd_no, "Invalid room number in variable assignment");
                        last_cmd = 0;
                    } else {
                        let go = crate::dg::GoId::Room(cmd.arg3 as RoomRnum);
                        if g.script_of(go).is_none() {
                            log_zone_error(g, zr, cmd_no, "Attempt to give variable to scriptless object");
                        } else if let Some(sc) = g.script_of_mut(go) {
                            crate::dg::add_var(&mut sc.global_vars, cmd.sarg1.as_deref().unwrap_or(b""), cmd.sarg2.as_deref().unwrap_or(b""), cmd.arg2 as i64);
                        }
                        last_cmd = 1;
                    }
                }
            }
            _ => {
                log_zone_error(g, zr, cmd_no, "unknown cmd in reset table; cmd disabled");
                g.world.zones[zr].cmds[cmd_no].command = b'*';
                last_cmd = 0;
            }
        }
    }
    g.zones_rt[zr].age = 0;

    // reset_wtrigger on every room of the zone.
    let (bot, top) = {
        let z = &g.world.zones[zr];
        (z.bot as i32, z.top as i32)
    };
    for rnum in 0..g.world.rooms.len() {
        let vnum = g.world.rooms[rnum].vnum as i32;
        if vnum >= bot && vnum <= top {
            crate::dg::triggers::reset_wtrigger(g, rnum as RoomRnum);
        }
    }
}

/// Boot-time reset of every zone in table order.
pub fn reset_all_zones(g: &mut Game) {
    for zr in 0..g.world.zones.len() {
        let (num, name, bot, top) = {
            let z = &g.world.zones[zr];
            (z.number, String::from_utf8_lossy(z.name.as_deref().unwrap_or(b"")).into_owned(), z.bot, z.top)
        };
        g.log(format!("Resetting #{}: {} (rooms {}-{}).", num, name, bot, top));
        reset_zone(g, zr);
    }
}

// ---------------------------------------------------------------------------
// OLC persistence — the pieces stage 8 needs
// ---------------------------------------------------------------------------

/// save_list types. The values are load-bearing: they index
/// `save_types[]`, whose message column `olc` prints.
pub const SL_MOB: i32 = 0;
pub const SL_OBJ: i32 = 1;
pub const SL_SHP: i32 = 2;
pub const SL_WLD: i32 = 3;
pub const SL_ZON: i32 = 4;
pub const SL_CFG: i32 = 5;
pub const SL_QST: i32 = 6;
pub const SL_MAX: i32 = 6;
pub const SL_ACT: i32 = SL_MAX + 1;
pub const SL_HLP: i32 = SL_MAX + 2;

/// add_to_save_list. The list is a **prepend** list, so `olc` lists the
/// newest pending file first and `save_all` writes in
/// reverse-request order. SL_CFG is rejected outright — which is why cedit's
/// `in_save_list(NOWHERE, SL_CFG)` check can never fire.
pub fn add_to_save_list(g: &mut Game, zone: Idx, type_: i32) -> bool {
    if type_ == SL_CFG {
        return false;
    }
    let rznum = g.world.real_zone(zone);
    if rznum.is_none() {
        let aedit = crate::act::wizstat::AEDIT_PERMISSION;
        let hedit = crate::act::wizstat::HEDIT_PERMISSION;
        if zone as i32 != aedit && zone as i32 != hedit {
            let top = g.world.zones.len().saturating_sub(1);
            g.log(format!(
                "SYSERR: add_to_save_list: Invalid zone number passed. ({} => {}, 0-{})",
                zone, NOWHERE, top
            ));
            return false;
        }
    }
    if g.save_list.iter().any(|&(z, t)| z == zone && t == type_) {
        return false;
    }
    g.save_list.insert(0, (zone, type_));
    true
}

/// remove_from_save_list: a miss is logged.
pub fn remove_from_save_list(g: &mut Game, zone: Idx, type_: i32) -> bool {
    if !g.save_list.iter().any(|&(z, t)| z == zone && t == type_) {
        g.log(format!(
            "SYSERR: remove_from_save_list: Saved item not found. ({}/{})",
            zone, type_
        ));
        return false;
    }
    g.save_list.retain(|&(z, t)| !(z == zone && t == type_));
    true
}

pub fn in_save_list(g: &Game, zone: Idx, type_: i32) -> bool {
    g.save_list.iter().any(|&(z, t)| z == zone && t == type_)
}

/// Render one world file for a zone and replace it atomically, the way every
/// genolc writer does (`<n>.new`, then remove + rename over `<n>.<ext>`).
///
/// Messages are the caller's business: each `save_*` has its own wording,
/// so this half stays silent and returns false on I/O failure.
pub fn write_world_file(g: &mut Game, zone_rnum: usize, type_: i32) -> Option<usize> {
    let (subdir, ext, body): (&str, &str, Vec<u8>) = match type_ {
        SL_MOB => ("mob", "mob", mud_world::write::mob::write_file(&g.world, zone_rnum as u16)),
        SL_OBJ => ("obj", "obj", mud_world::write::obj::write_file(&g.world, zone_rnum as u16)),
        SL_ZON => ("zon", "zon", mud_world::write::zon::write_file(&g.world, zone_rnum as u16)),
        SL_WLD => ("wld", "wld", mud_world::write::wld::write_file(&g.world, zone_rnum as u16)),
        SL_SHP => ("shp", "shp", mud_world::write::shp::write_file(&g.world, zone_rnum as u16)),
        SL_QST => ("qst", "qst", mud_world::write::qst::write_file(&g.world, zone_rnum as u16)),
        _ => return None,
    };
    let number = g.world.zones[zone_rnum].number;
    let dir = g.lib_dir.join("world").join(subdir);
    let newname = dir.join(format!("{}.new", number));
    let oldname = dir.join(format!("{}.{}", number, ext));
    if std::fs::write(&newname, &body).is_err() {
        return None;
    }
    let _ = std::fs::remove_file(&oldname);
    if std::fs::rename(&newname, &oldname).is_err() {
        return None;
    }
    // ftell after the final fputs: the byte count the save logs report.
    Some(body.len())
}

pub fn save_zone(g: &mut Game, zone_rnum: usize) -> bool {
    if zone_rnum >= g.world.zones.len() {
        let top = g.world.zones.len().saturating_sub(1);
        g.log(format!(
            "SYSERR: GenOLC: save_zone: Invalid real zone number {}. (0-{})",
            zone_rnum, top
        ));
        return false;
    }
    let number = g.world.zones[zone_rnum].number;
    if write_world_file(g, zone_rnum, SL_ZON).is_none() {
        let msg = format!("SYSERR: OLC: save_zones:  Can't write zone {}.", number);
        g.mudlog(MudlogKind::Brf, LVL_BUILDER, true, &msg);
        return false;
    }
    if in_save_list(g, number, SL_ZON) {
        remove_from_save_list(g, number, SL_ZON);
    }
    true
}

/// The `save_types[]` function column: dispatch one pending entry.
fn save_dispatch(g: &mut Game, type_: i32, zone_rnum: Option<usize>) -> bool {
    match type_ {
        SL_WLD => crate::olc::genwld::save_rooms(g, zone_rnum),
        SL_ZON => match zone_rnum {
            Some(r) => save_zone(g, r),
            None => {
                let top = g.world.zones.len().saturating_sub(1);
                g.log(format!(
                    "SYSERR: GenOLC: save_zone: Invalid real zone number {}. (0-{})",
                    NOWHERE, top
                ));
                false
            }
        },
        SL_MOB => crate::olc::genmob::save_mobiles(g, zone_rnum),
        SL_OBJ => crate::olc::genobj::save_objects(g, zone_rnum),
        // Types whose editors do not exist yet. The save-list entry still
        // has to be cleared, or save_all finds it again and never
        // terminates.
        _ => match zone_rnum {
            Some(r) => {
                let zvnum = g.world.zones[r].number;
                let ok = write_world_file(g, r, type_).is_some();
                if ok && in_save_list(g, zvnum, type_) {
                    remove_from_save_list(g, zvnum, type_);
                }
                ok
            }
            None => false,
        },
    }
}

/// save_all: drain the pending-save list from the head.
/// Each writer removes its own entry on success; a writer that fails leaves
/// the entry in place and it is skipped.
pub fn save_all(g: &mut Game) -> bool {
    let mut skipped: Vec<(Idx, i32)> = Vec::new();
    loop {
        let Some(&(zone, type_)) = g.save_list.iter().find(|e| !skipped.contains(e)) else {
            break;
        };
        match type_ {
            SL_ACT => {
                g.log("Actions not saved - can not autosave. Use 'aedit save'.".to_string());
                skipped.push((zone, type_));
            }
            SL_HLP => {
                g.log("Help not saved - can not autosave. Use 'hedit save'.".to_string());
                skipped.push((zone, type_));
            }
            t if !(0..=SL_MAX).contains(&t) => {
                // Unknown type: log once and move on. Leaving the entry
                // without advancing would spin forever.
                g.log(format!("SYSERR: GenOLC: Invalid save type {} in save list.\n", t));
                skipped.push((zone, type_));
            }
            t => {
                let rnum = g.world.zones.iter().position(|z| z.number == zone);
                if !save_dispatch(g, t, rnum) {
                    skipped.push((zone, t));
                }
            }
        }
    }
    true
}
