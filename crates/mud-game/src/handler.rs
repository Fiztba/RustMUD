//! Movement of chars/objs between containers, equipment, affect
//! bookkeeping, visibility, and extraction. Iteration order follows the
//! head-insertion lists, which is observable in room and inventory
//! listings.

use mud_data::flags::{self, FlagSet};
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::ch::{Affect, Char};
use crate::game::Game;
use crate::gametime::{SUN_DARK, SUN_SET};

pub const WHITESPACE: &[u8] = b" \t";

/// fname: leading alphabetic run of a namelist.
pub fn fname(namelist: &[u8]) -> Vec<u8> {
    namelist.iter().take_while(|c| c.is_ascii_alphabetic()).copied().collect()
}

pub fn is_abbrev(arg1: &[u8], arg2: &[u8]) -> bool {
    mud_net::descriptor::is_abbrev(arg1, arg2)
}

/// Case-insensitive equality.
pub fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// is_name: whole-word match over a namelist, no
/// abbreviation. Only letters continue a word, so a digit ends one: "bob"
/// matches the namelist "bob2", and "1" matches nothing at all.
pub fn is_name(s: &[u8], namelist: &[u8]) -> bool {
    if s.is_empty() || namelist.is_empty() {
        return false;
    }
    let at = |p: usize| -> u8 { namelist.get(p).copied().unwrap_or(0) };
    let mut curname = 0usize;
    loop {
        let mut curstr = 0usize;
        loop {
            let sc = s.get(curstr).copied().unwrap_or(0);
            let nc = at(curname);
            if sc == 0 && !nc.is_ascii_alphabetic() {
                return true;
            }
            if nc == 0 {
                return false;
            }
            if sc == 0 || nc == b' ' {
                break;
            }
            if sc.to_ascii_lowercase() != nc.to_ascii_lowercase() {
                break;
            }
            curstr += 1;
            curname += 1;
        }
        // Skip to the next name.
        while at(curname).is_ascii_alphabetic() {
            curname += 1;
        }
        if at(curname) == 0 {
            return false;
        }
        curname += 1;
    }
}

/// isname: word-prefix match over a namelist, abbreviated
/// numbers disallowed. NOTE the quirk: a failed number-abbreviation check
/// aborts the whole scan (returns 0), not just that word.
pub fn isname(s: &[u8], namelist: &[u8]) -> bool {
    if s.is_empty() || namelist.is_empty() {
        return false;
    }
    if s == namelist {
        return true;
    }
    for word in namelist.split(|c| WHITESPACE.contains(c)).filter(|w| !w.is_empty()) {
        if is_abbrev(s, word) {
            if s[0].is_ascii_digit() && atoi(s) != atoi(word) {
                return false;
            }
            return true;
        }
    }
    false
}

pub fn atoi(b: &[u8]) -> i32 {
    let mut i = 0usize;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut n: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        n = n * 10 + (b[i] - b'0') as i64;
        if n > i64::from(i32::MAX) + 1 {
            break;
        }
        i += 1;
    }
    if neg { (-n).max(i32::MIN as i64) as i32 } else { n.min(i32::MAX as i64) as i32 }
}

// ---- lighting / visibility ----

/// room_is_dark.
/// ROOM_FLAGGED over the world's `int[4]` bit array.
pub fn room_flagged(g: &Game, room: RoomRnum, bit: usize) -> bool {
    room != NOWHERE
        && (room as usize) < g.world.rooms.len()
        && g.world.rooms[room as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0
}

pub fn set_room_flag(g: &mut Game, room: RoomRnum, bit: usize) {
    if room != NOWHERE && (room as usize) < g.world.rooms.len() {
        g.world.rooms[room as usize].room_flags[bit / 32] |= 1 << (bit % 32);
    }
}

pub fn remove_room_flag(g: &mut Game, room: RoomRnum, bit: usize) {
    if room != NOWHERE && (room as usize) < g.world.rooms.len() {
        g.world.rooms[room as usize].room_flags[bit / 32] &= !(1 << (bit % 32));
    }
}

pub fn room_is_dark(g: &Game, room: RoomRnum) -> bool {
    let r = room as usize;
    if r >= g.world.rooms.len() {
        return false;
    }
    if g.rooms[r].light != 0 {
        return false;
    }
    if g.world.rooms[r].room_flags[0] & (1 << flags::ROOM_DARK) != 0 {
        return true;
    }
    let sect = g.world.rooms[r].sector_type;
    if sect == flags::SECT_INSIDE || sect == flags::SECT_CITY {
        return false;
    }
    if g.weather.sunlight == SUN_SET || g.weather.sunlight == SUN_DARK {
        return true;
    }
    false
}

/// GET_REAL_LEVEL: level of the original char when switched.
pub fn get_real_level(g: &Game, id: CharId) -> u8 {
    let ch = g.ch(id);
    if let Some(di) = ch.desc {
        if let Some(d) = g.descriptors.get(di) {
            if let Some(orig) = d.original {
                if let Some(oc) = g.try_ch(orig) {
                    return oc.level;
                }
            }
        }
    }
    ch.level
}

fn light_ok(g: &Game, sub: &Char) -> bool {
    !sub.aff(flags::AFF_BLIND)
        && (!room_is_dark(g, sub.in_room) || sub.aff(flags::AFF_INFRAVISION) || sub.level >= LVL_IMMORT)
}

fn invis_ok(sub: &Char, obj: &Char) -> bool {
    (!obj.aff(flags::AFF_INVISIBLE) || sub.aff(flags::AFF_DETECT_INVIS))
        && (!obj.aff(flags::AFF_HIDE) || sub.aff(flags::AFF_SENSE_LIFE))
}

pub fn can_see(g: &Game, sub_id: CharId, obj_id: CharId) -> bool {
    if sub_id == obj_id {
        return true;
    }
    let sub = g.ch(sub_id);
    let obj = g.ch(obj_id);
    let invis_lev = if obj.is_npc() { 0 } else { obj.invis_lev() };
    if (get_real_level(g, sub_id) as i16) < invis_lev {
        return false;
    }
    let mort = light_ok(g, sub) && invis_ok(sub, obj);
    mort || (!sub.is_npc() && sub.prf(flags::PRF_HOLYLIGHT))
}

pub fn can_see_obj(g: &Game, sub_id: CharId, oid: ObjId) -> bool {
    let sub = g.ch(sub_id);
    let obj = g.obj(oid);
    let invis_obj_ok = !obj_flag(g, oid, flags::ITEM_INVISIBLE) || sub.aff(flags::AFF_DETECT_INVIS);
    let carrier_ok = obj.carried_by.is_none_or(|c| can_see(g, sub_id, c))
        && obj.worn_by.is_none_or(|c| can_see(g, sub_id, c));
    let mort = light_ok(g, sub) && invis_obj_ok && carrier_ok;
    mort || (!sub.is_npc() && sub.prf(flags::PRF_HOLYLIGHT))
}

pub fn obj_flag(g: &Game, oid: ObjId, bit: usize) -> bool {
    g.obj(oid).extra_flags.is_set(bit)
}

/// PERS(ch, vict): name if visible else "someone".
pub fn pers(g: &Game, viewer: CharId, target: CharId) -> Vec<u8> {
    if can_see(g, viewer, target) {
        g.ch(target).get_name().to_vec()
    } else {
        b"someone".to_vec()
    }
}

// ---- object name resolution (proto fallbacks) ----

pub fn obj_name<'a>(g: &'a Game, oid: ObjId) -> &'a [u8] {
    let o = g.obj(oid);
    if let Some(n) = &o.name {
        return n;
    }
    proto_str(g, o.item_number, |p| p.name.as_deref()).unwrap_or(b"")
}

pub fn obj_short<'a>(g: &'a Game, oid: ObjId) -> &'a [u8] {
    let o = g.obj(oid);
    if let Some(n) = &o.short_description {
        return n;
    }
    proto_str(g, o.item_number, |p| p.short_description.as_deref()).unwrap_or(b"")
}

pub fn obj_room_desc<'a>(g: &'a Game, oid: ObjId) -> &'a [u8] {
    let o = g.obj(oid);
    if let Some(n) = &o.description {
        return n;
    }
    proto_str(g, o.item_number, |p| p.description.as_deref()).unwrap_or(b"")
}

pub fn obj_action_desc<'a>(g: &'a Game, oid: ObjId) -> Option<&'a [u8]> {
    let o = g.obj(oid);
    if let Some(n) = &o.action_description {
        return Some(n);
    }
    proto_str(g, o.item_number, |p| p.action_description.as_deref())
}

fn proto_str<'a>(
    g: &'a Game,
    rnum: Idx,
    f: impl Fn(&'a mud_world::model::ObjProto) -> Option<&'a [u8]>,
) -> Option<&'a [u8]> {
    g.world.obj_protos.get(rnum as usize).and_then(f)
}

// ---- char <-> room ----

/// char_to_room. Prepends to people list; moves light.
pub fn char_to_room(g: &mut Game, chid: CharId, room: RoomRnum) {
    if room == NOWHERE || (room as usize) >= g.world.rooms.len() {
        g.log(format!(
            "SYSERR: Illegal value(s) passed to char_to_room. (Room: {}/{} Ch: ?)",
            room,
            g.world.rooms.len().saturating_sub(1)
        ));
        return;
    }
    g.rooms[room as usize].people.insert(0, chid);
    g.ch_mut(chid).in_room = room;

    crate::quest::autoquest_trigger_check(g, chid, None, None, crate::quest::AQ_ROOM_FIND);
    crate::quest::autoquest_trigger_check(g, chid, None, None, crate::quest::AQ_MOB_FIND);

    if let Some(light) = g.ch(chid).equipment[WEAR_LIGHT] {
        if g.obj(light).type_flag == flags::ITEM_LIGHT && g.obj(light).values[2] != 0 {
            g.rooms[room as usize].light += 1;
        }
    }
    // Stop fighting if we left our opponent behind.
    if let Some(vict) = g.ch(chid).fighting {
        if g.try_ch(vict).map(|v| v.in_room) != Some(room) {
            stop_fighting(g, vict);
            stop_fighting(g, chid);
        }
    }
}

/// char_from_room.
pub fn char_from_room(g: &mut Game, chid: CharId) {
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        g.log("SYSERR: NULL character or NOWHERE in char_from_room".to_string());
        return;
    }
    if g.ch(chid).fighting.is_some() {
        stop_fighting(g, chid);
    }
    if let Some(light) = g.ch(chid).equipment[WEAR_LIGHT] {
        if g.obj(light).type_flag == flags::ITEM_LIGHT && g.obj(light).values[2] != 0 {
            g.rooms[room as usize].light -= 1;
        }
    }
    g.rooms[room as usize].people.retain(|c| *c != chid);
    let ch = g.ch_mut(chid);
    ch.in_room = NOWHERE;
}

/// stop_fighting lives in fight.rs (combat_list + cursor semantics); this
/// re-export keeps the stage-2/3 call sites.
pub use crate::fight::stop_fighting;

// ---- obj <-> char/room/obj ----

/// GET_OBJ_WEIGHT: instance weight (containers already include contents).
pub fn obj_weight(g: &Game, oid: ObjId) -> i32 {
    g.obj(oid).weight
}

/// obj_to_char: prepend to carrying.
pub fn obj_to_char(g: &mut Game, oid: ObjId, chid: CharId) {
    let w = obj_weight(g, oid);
    {
        let o = g.obj_mut(oid);
        o.carried_by = Some(chid);
        o.in_room = NOWHERE;
    }
    {
        let ch = g.ch_mut(chid);
        ch.carrying.insert(0, oid);
        ch.carry_weight += w;
        ch.carry_items += 1;
    }

    crate::quest::autoquest_trigger_check(g, chid, None, Some(oid), crate::quest::AQ_OBJ_FIND);

    // Set the crash-save flag, but not on mobs.
    if !g.ch(chid).is_npc() {
        g.ch_mut(chid).act.set(flags::PLR_CRASH);
    }
}

pub fn obj_from_char(g: &mut Game, oid: ObjId) {
    let Some(chid) = g.obj(oid).carried_by else {
        g.log("SYSERR: NULL object passed to obj_from_char.".to_string());
        return;
    };
    let w = obj_weight(g, oid);
    let ch = g.ch_mut(chid);
    ch.carrying.retain(|o| *o != oid);
    if !ch.is_npc() {
        ch.act.set(flags::PLR_CRASH);
    }
    ch.carry_weight -= w;
    ch.carry_items -= 1;
    let o = g.obj_mut(oid);
    o.carried_by = None;
}

/// obj_to_room: APPENDS to the room contents tail. List order is visible
/// in room listings, so this is deliberate.
pub fn obj_to_room(g: &mut Game, oid: ObjId, room: RoomRnum) {
    if room == NOWHERE || (room as usize) >= g.world.rooms.len() {
        g.log(format!("SYSERR: Illegal value(s) passed to obj_to_room. (Room #{}/{}, obj ?)", room, g.world.rooms.len()));
        return;
    }
    g.rooms[room as usize].contents.push(oid);
    let o = g.obj_mut(oid);
    o.in_room = room;
    o.carried_by = None;
    o.worn_by = None;
    if room_flagged(g, room, flags::ROOM_HOUSE) {
        set_room_flag(g, room, flags::ROOM_HOUSE_CRASH);
    }
}

pub fn obj_from_room(g: &mut Game, oid: ObjId) {
    let room = g.obj(oid).in_room;
    if room == NOWHERE || (room as usize) >= g.world.rooms.len() {
        g.log(format!("SYSERR: NULL object (?) or obj not in a room (?) passed to obj_from_room"));
        return;
    }
    g.rooms[room as usize].contents.retain(|o| *o != oid);
    if room_flagged(g, room, flags::ROOM_HOUSE) {
        set_room_flag(g, room, flags::ROOM_HOUSE_CRASH);
    }
    let o = g.obj_mut(oid);
    o.in_room = NOWHERE;
}

pub fn is_corpse(g: &Game, oid: ObjId) -> bool {
    let o = g.obj(oid);
    o.type_flag == flags::ITEM_CONTAINER && o.values[3] == 1
}

/// Corpses carve through the val0 weight gate so their contents account
/// honestly.
pub(crate) fn weight_gate_open(g: &Game, container: ObjId) -> bool {
    g.obj(container).values[0] > 0 || is_corpse(g, container)
}

/// obj_to_obj: put obj into container. Weight propagates up the
/// chain and onto a carrying char ONLY when the immediate container's val0
/// is > 0 — zero-capacity containers are weightless-unlimited, the deliberate
/// 2007 feature (140fcc2). Quirk A2: INTENDED for containers; corpses are
/// the B14 carve-out (honest accounting).
pub fn obj_to_obj(g: &mut Game, oid: ObjId, into: ObjId) {
    if oid == into {
        g.log("SYSERR: same source and target obj passed to obj_to_obj.".to_string());
        return;
    }
    let w = obj_weight(g, oid);
    g.obj_mut(into).contains.insert(0, oid);
    {
        let o = g.obj_mut(oid);
        o.in_obj = Some(into);
    }
    // "Add weight to container, unless unlimited." — gate is the IMMEDIATE
    // container's val0 only; outer containers then gain regardless.
    if weight_gate_open(g, into) {
        let mut tmp = into;
        loop {
            g.obj_mut(tmp).weight += w;
            match g.obj(tmp).in_obj {
                Some(up) => tmp = up,
                None => break,
            }
        }
        if let Some(carrier) = g.obj(tmp).carried_by {
            g.ch_mut(carrier).carry_weight += w;
        }
    }
}

pub fn obj_from_obj(g: &mut Game, oid: ObjId) {
    let Some(from) = g.obj(oid).in_obj else {
        g.log("SYSERR: trying to illegally extract obj from obj.".to_string());
        return;
    };
    let w = obj_weight(g, oid);
    g.obj_mut(from).contains.retain(|o| *o != oid);
    // Same val0 gate as obj_to_obj (checked on the container being left).
    if weight_gate_open(g, from) {
        let mut tmp = from;
        loop {
            g.obj_mut(tmp).weight -= w;
            match g.obj(tmp).in_obj {
                Some(up) => tmp = up,
                None => break,
            }
        }
        if let Some(carrier) = g.obj(tmp).carried_by {
            g.ch_mut(carrier).carry_weight -= w;
        }
    }
    g.obj_mut(oid).in_obj = None;
}

// ---- affects ----

/// aff_apply_modify: the APPLY_* switch.
fn aff_apply_modify(g: &mut Game, chid: CharId, loc: i32, mod_: i32) {
    let ch = g.ch_mut(chid);
    match loc {
        flags::APPLY_NONE => {}
        flags::APPLY_STR => ch.aff_abils.str_ = (ch.aff_abils.str_ as i32 + mod_) as i8,
        flags::APPLY_DEX => ch.aff_abils.dex = (ch.aff_abils.dex as i32 + mod_) as i8,
        flags::APPLY_INT => ch.aff_abils.intel = (ch.aff_abils.intel as i32 + mod_) as i8,
        flags::APPLY_WIS => ch.aff_abils.wis = (ch.aff_abils.wis as i32 + mod_) as i8,
        flags::APPLY_CON => ch.aff_abils.con = (ch.aff_abils.con as i32 + mod_) as i8,
        flags::APPLY_CHA => ch.aff_abils.cha = (ch.aff_abils.cha as i32 + mod_) as i8,
        flags::APPLY_CLASS | flags::APPLY_LEVEL => {}
        flags::APPLY_AGE => ch.time.birth -= mod_ as i64 * SECS_PER_MUD_YEAR as i64,
        flags::APPLY_CHAR_WEIGHT => ch.weight = (ch.weight as i32 + mod_) as u8,
        flags::APPLY_CHAR_HEIGHT => ch.height = (ch.height as i32 + mod_) as u8,
        flags::APPLY_MANA => ch.points.max_mana += mod_,
        flags::APPLY_HIT => ch.points.max_hit += mod_,
        flags::APPLY_MOVE => ch.points.max_move += mod_,
        flags::APPLY_GOLD | flags::APPLY_EXP => {}
        flags::APPLY_AC => ch.points.armor += mod_,
        flags::APPLY_HITROLL => ch.points.hitroll = (ch.points.hitroll as i32 + mod_) as i8,
        flags::APPLY_DAMROLL => ch.points.damroll = (ch.points.damroll as i32 + mod_) as i8,
        flags::APPLY_SAVING_PARA => ch.apply_saving_throw[0] += mod_ as i16,
        flags::APPLY_SAVING_ROD => ch.apply_saving_throw[1] += mod_ as i16,
        flags::APPLY_SAVING_PETRI => ch.apply_saving_throw[2] += mod_ as i16,
        flags::APPLY_SAVING_BREATH => ch.apply_saving_throw[3] += mod_ as i16,
        flags::APPLY_SAVING_SPELL => ch.apply_saving_throw[4] += mod_ as i16,
        _ => {
            g.log(format!("SYSERR: Unknown apply adjust {} attempt (affect_modify).", loc));
        }
    }
}

/// The AFF bits a mob was made with. Empty for players, who have no mob
/// file to be innate from.
///
/// Taken from the mob's own snapshot, not from `mob_protos[mob_rnum]`. The
/// two drift -- see `MobSpecials::innate_aff` -- and reading the prototype
/// lets the guard below decline to drop a flag the mob never had, which is
/// the one thing it must not do: it may only keep a flag whose source is
/// still there, never confer one.
///
/// `AFF_INVISIBLE` and `AFF_HIDE` are excluded, and that is the whole of the
/// exception. The game has explicit mechanics that reveal a mob — `appear`
/// when it is attacked, and the hide check — so those two are the game's to
/// remove, and a mob it has revealed has to stay revealed. Treating them as
/// innate would hand an invisibility spell cast on a revealed mob the means
/// to give its innate invisibility back when the spell expired.
///
/// Nothing else the mob file sets has such a mechanic: no code path
/// removes SANCTUARY, DETECT_INVIS, SENSE_LIFE or NOTRACK from a mob on
/// purpose, so for those the prototype really is a source that outlives any
/// spell or item. CHARM, POISON and SLEEP cannot arise here at all — the
/// legacy mob line strips all three at parse (`parse/mob.rs`) and medit does
/// the same on save.
fn innate_affects(g: &Game, chid: CharId) -> FlagSet {
    let ch = g.ch(chid);
    if !ch.is_npc() {
        return FlagSet::EMPTY;
    }
    let mut f = ch.mob_specials.innate_aff;
    f.remove(flags::AFF_INVISIBLE);
    f.remove(flags::AFF_HIDE);
    f
}

/// affect_modify_ar.
fn affect_modify_ar(g: &mut Game, chid: CharId, loc: i32, mod_: i32, bitv: FlagSet, add: bool) {
    let mut mod_ = mod_;
    if add {
        for bit in 0..128 {
            if bitv.is_set(bit) {
                g.ch_mut(chid).affected_by.set(bit);
            }
        }
    } else {
        // A mob's mob-file AFF bits are a source like equipment or a spell,
        // and unlike those two they never go away while the mob lives. This
        // branch runs when some *other* source is withdrawn, so clearing a
        // bit the prototype also sets throws away a flag nothing asked to
        // remove: unequip a mob's gear whose perm-affects overlap its innate
        // sanctuary and the sanctuary goes with it, for the rest of that
        // mob's life.
        //
        // Only this bookkeeping path is guarded. Deliberate removals —
        // `appear` revealing an invisible mob, `stop_follower` clearing
        // CHARM — call `affected_by.remove` directly and are untouched, so
        // a revealed mob stays revealed. Nothing here restores a bit; it only
        // declines to drop one whose source is still present. Death and repop
        // remains the only way a mob returns to its mob-file state, which is
        // also the only restoration the game has (zone reset does not touch
        // an existing mob at all).
        let innate = innate_affects(g, chid);
        for bit in 0..128 {
            if bitv.is_set(bit) && !innate.is_set(bit) {
                g.ch_mut(chid).affected_by.remove(bit);
            }
        }
        mod_ = -mod_;
    }
    aff_apply_modify(g, chid, loc, mod_);
}

/// affect_total: strip everything, reset abils, re-apply.
pub fn affect_total(g: &mut Game, chid: CharId) {
    let eq: Vec<(usize, ObjId)> = g
        .ch(chid)
        .equipment
        .iter()
        .enumerate()
        .filter_map(|(i, o)| o.map(|o| (i, o)))
        .collect();
    for &(_, oid) in &eq {
        let (affs, perm) = {
            let o = g.obj(oid);
            (o.affected, o.perm_affects)
        };
        for a in affs {
            affect_modify_ar(g, chid, a.location, a.modifier, perm, false);
        }
    }
    let spell_affects = g.ch(chid).affected.clone();
    for a in &spell_affects {
        affect_modify_ar(g, chid, a.location as i32, a.modifier as i32, a.bitvector, false);
    }

    {
        let ch = g.ch_mut(chid);
        ch.aff_abils = ch.real_abils;
    }

    for &(_, oid) in &eq {
        let (affs, perm) = {
            let o = g.obj(oid);
            (o.affected, o.perm_affects)
        };
        for a in affs {
            affect_modify_ar(g, chid, a.location, a.modifier, perm, true);
        }
    }
    for a in &spell_affects {
        affect_modify_ar(g, chid, a.location as i32, a.modifier as i32, a.bitvector, true);
    }

    let ch = g.ch_mut(chid);
    let cap: i8 = if ch.is_npc() || ch.level >= LVL_GRGOD { 25 } else { 18 };
    ch.aff_abils.dex = ch.aff_abils.dex.clamp(0, cap);
    ch.aff_abils.intel = ch.aff_abils.intel.clamp(0, cap);
    ch.aff_abils.wis = ch.aff_abils.wis.clamp(0, cap);
    ch.aff_abils.con = ch.aff_abils.con.clamp(0, cap);
    ch.aff_abils.cha = ch.aff_abils.cha.clamp(0, cap);
    ch.aff_abils.str_ = ch.aff_abils.str_.max(0);
    if ch.is_npc() || ch.level >= LVL_GRGOD {
        ch.aff_abils.str_ = ch.aff_abils.str_.min(cap);
    } else if ch.aff_abils.str_ > 18 {
        let i = ch.aff_abils.str_add as i32 + ((ch.aff_abils.str_ as i32 - 18) * 10);
        ch.aff_abils.str_add = i.min(100) as i8;
        ch.aff_abils.str_ = 18;
    }
}

/// affect_to_char: prepend, apply, total.
pub fn affect_to_char(g: &mut Game, chid: CharId, af: Affect) {
    g.ch_mut(chid).affected.insert(0, af.clone());
    affect_modify_ar(g, chid, af.location as i32, af.modifier as i32, af.bitvector, true);
    affect_total(g, chid);
}

/// affect_remove, by index.
pub fn affect_remove(g: &mut Game, chid: CharId, index: usize) {
    let af = g.ch_mut(chid).affected.remove(index);
    affect_modify_ar(g, chid, af.location as i32, af.modifier as i32, af.bitvector, false);
    affect_total(g, chid);
}

pub fn affected_by_spell(g: &Game, chid: CharId, spell: i16) -> bool {
    g.ch(chid).affected.iter().any(|a| a.spell == spell)
}

/// affect_from_char: remove every affect of the given spell.
pub fn affect_from_char(g: &mut Game, chid: CharId, spell: i16) {
    loop {
        let idx = g.ch(chid).affected.iter().position(|a| a.spell == spell);
        match idx {
            Some(i) => affect_remove(g, chid, i),
            None => break,
        }
    }
}

/// raw_kill's `while (ch->affected) affect_remove(...)` — silent strip.
pub fn affect_remove_all(g: &mut Game, chid: CharId) {
    while !g.ch(chid).affected.is_empty() {
        affect_remove(g, chid, 0);
    }
}

/// affect_join: merge with an existing same-spell same-location
/// affect (only the first match), else plain affect_to_char.
pub fn affect_join(
    g: &mut Game,
    chid: CharId,
    mut af: Affect,
    add_dur: bool,
    avg_dur: bool,
    add_mod: bool,
    avg_mod: bool,
) {
    let found = g
        .ch(chid)
        .affected
        .iter()
        .position(|hjp| hjp.spell == af.spell && hjp.location == af.location);
    if let Some(idx) = found {
        {
            let hjp = &g.ch(chid).affected[idx];
            if add_dur {
                af.duration += hjp.duration;
            }
            if avg_dur {
                af.duration /= 2;
            }
            if add_mod {
                af.modifier += hjp.modifier;
            }
            if avg_mod {
                af.modifier /= 2;
            }
        }
        affect_remove(g, chid, idx);
        affect_to_char(g, chid, af);
    } else {
        affect_to_char(g, chid, af);
    }
}

// groups (Vatiken's Group System 1.1) ----

/// create_group: allocate, flag GROUP_OPEN (+NPC for
/// NPC leaders), then join_group(leader).
pub fn create_group(g: &mut Game, leader: CharId) -> u64 {
    let id = g.next_group_id;
    g.next_group_id += 1;
    let mut flags = crate::game::GROUP_OPEN;
    if g.ch(leader).is_npc() {
        flags |= crate::game::GROUP_NPC;
    }
    g.groups.push(crate::game::Group { id, leader: None, members: Vec::new(), group_flags: flags });
    join_group(g, leader, id);
    id
}

pub fn join_group(g: &mut Game, chid: CharId, gid: u64) {
    let is_npc = g.ch(chid).is_npc();
    let mut became_leader = false;
    {
        let Some(gr) = g.group_mut(gid) else { return };
        gr.members.push(chid);
        if gr.leader.is_none() {
            gr.leader = Some(chid);
        }
        if gr.group_flags & crate::game::GROUP_NPC != 0 && !is_npc {
            gr.group_flags &= !crate::game::GROUP_NPC;
        }
        if gr.leader == Some(chid) {
            became_leader = true;
        }
    }
    g.ch_mut(chid).group = Some(gid);
    let mut body = g.ch(chid).get_name().to_vec();
    if became_leader {
        body.extend_from_slice(b" becomes leader of the group.\r\n");
    } else {
        body.extend_from_slice(b" joins the group.\r\n");
    }
    crate::comm::send_to_group(g, None, gid, &body);
}

pub fn leave_group(g: &mut Game, chid: CharId) {
    let Some(gid) = g.try_ch(chid).and_then(|c| c.group) else { return };

    let mut body = g.ch(chid).get_name().to_vec();
    body.extend_from_slice(b" has left the group.\r\n");
    crate::comm::send_to_group(g, None, gid, &body);

    let (size, was_leader) = {
        let Some(gr) = g.group_mut(gid) else { return };
        gr.members.retain(|&m| m != chid);
        (gr.members.len(), gr.leader == Some(chid))
    };
    g.ch_mut(chid).group = None;

    if size > 0 {
        let found_pc = g
            .group(gid)
            .map(|gr| gr.members.iter().any(|&m| g.try_ch(m).is_some_and(|c| !c.is_npc())))
            .unwrap_or(false);
        if !found_pc {
            if let Some(gr) = g.group_mut(gid) {
                gr.group_flags |= crate::game::GROUP_NPC;
            }
        }
    }

    if was_leader && size > 0 {
        // random_from_list: one rand_number(1, size) draw.
        let pick = g.rng.rand_number(1, size as i32) as usize - 1;
        let new_leader = g.group(gid).map(|gr| gr.members[pick]);
        if let (Some(gr), Some(nl)) = (g.group_mut(gid), new_leader) {
            gr.leader = Some(nl);
            let mut body = g.ch(nl).get_name().to_vec();
            body.extend_from_slice(b" has assumed leadership of the group.\r\n");
            crate::comm::send_to_group(g, None, gid, &body);
        }
    } else if size == 0 {
        free_group(g, gid);
    }
}

/// free_group: empty out and drop from the list.
pub fn free_group(g: &mut Game, gid: u64) {
    loop {
        let member = g.group(gid).and_then(|gr| gr.members.first().copied());
        match member {
            Some(m) => leave_group(g, m),
            None => break,
        }
    }
    g.groups.retain(|gr| gr.id != gid);
}

// ---- carrying capacity ----

pub fn strength_apply_index(ch: &Char) -> usize {
    let str_ = ch.aff_abils.str_ as i32;
    let add = ch.aff_abils.str_add as i32;
    if add == 0 || str_ != 18 {
        str_.max(0) as usize
    } else if add <= 50 {
        26
    } else if add <= 75 {
        27
    } else if add <= 90 {
        28
    } else if add <= 99 {
        29
    } else {
        30
    }
}

/// CAN_CARRY_W: str_app[..].carry_w.
pub fn can_carry_w(ch: &Char) -> i32 {
    mud_data::tables::STR_APP[strength_apply_index(ch)].2
}

/// CAN_CARRY_N: 5 + dex/2 + level/2.
pub fn can_carry_n(ch: &Char) -> i32 {
    5 + (ch.aff_abils.dex as i32 >> 1) + (ch.level as i32 >> 1)
}

// ---- equipment ----

fn apply_ac(g: &Game, chid: CharId, pos: usize) -> i32 {
    let Some(oid) = g.ch(chid).equipment[pos] else { return 0 };
    if g.obj(oid).type_flag != flags::ITEM_ARMOR {
        return 0;
    }
    let factor = match pos {
        WEAR_BODY => 3,
        WEAR_HEAD | WEAR_LEGS => 2,
        _ => 1,
    };
    factor * g.obj(oid).values[0]
}

/// invalid_align: ANTI_GOOD/EVIL/NEUTRAL vs char alignment.
pub fn invalid_align(g: &Game, chid: CharId, oid: ObjId) -> bool {
    let ch = g.ch(chid);
    let o = g.obj(oid);
    (o.extra_flags.is_set(flags::ITEM_ANTI_EVIL) && ch.alignment <= -350)
        || (o.extra_flags.is_set(flags::ITEM_ANTI_GOOD) && ch.alignment >= 350)
        || (o.extra_flags.is_set(flags::ITEM_ANTI_NEUTRAL) && ch.alignment > -350 && ch.alignment < 350)
}

pub fn invalid_class(g: &Game, chid: CharId, oid: ObjId) -> bool {
    let ch = g.ch(chid);
    let o = g.obj(oid);
    (o.extra_flags.is_set(flags::ITEM_ANTI_MAGIC_USER) && ch.class == CLASS_MAGIC_USER && !ch.is_npc())
        || (o.extra_flags.is_set(flags::ITEM_ANTI_CLERIC) && ch.class == CLASS_CLERIC && !ch.is_npc())
        || (o.extra_flags.is_set(flags::ITEM_ANTI_WARRIOR) && ch.class == CLASS_WARRIOR && !ch.is_npc())
        || (o.extra_flags.is_set(flags::ITEM_ANTI_THIEF) && ch.class == CLASS_THIEF && !ch.is_npc())
}

/// equip_char. Returns false when the object was zapped to
/// inventory instead (align/class conflict).
pub fn equip_char(g: &mut Game, chid: CharId, oid: ObjId, pos: usize) -> bool {
    if pos >= NUM_WEARS {
        g.log("SYSERR: equip_char with bad pos".to_string());
        return false;
    }
    if g.ch(chid).equipment[pos].is_some() {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let short = String::from_utf8_lossy(obj_short(g, oid)).into_owned();
        g.log(format!("SYSERR: Char is already equipped: {}, {}", name, short));
        return false;
    }
    if g.obj(oid).carried_by.is_some() {
        g.log("SYSERR: EQUIP: Obj is carried_by when equip.".to_string());
        return false;
    }
    if g.obj(oid).in_room != NOWHERE {
        g.log("SYSERR: EQUIP: Obj is in_room when equip.".to_string());
        return false;
    }
    if invalid_align(g, chid, oid) || invalid_class(g, chid, oid) {
        crate::comm::act(g, b"You are zapped by $p and instantly let go of it.", false, Some(chid), Some(oid), None, crate::comm::TO_CHAR);
        crate::comm::act(g, b"$n is zapped by $p and instantly lets go of it.", false, Some(chid), Some(oid), None, crate::comm::TO_ROOM);
        obj_to_char(g, oid, chid);
        return false;
    }

    {
        let ch = g.ch_mut(chid);
        ch.equipment[pos] = Some(oid);
    }
    {
        let o = g.obj_mut(oid);
        o.worn_by = Some(chid);
        o.worn_on = pos as i16;
    }
    if g.obj(oid).type_flag == flags::ITEM_ARMOR {
        let delta = apply_ac(g, chid, pos);
        g.ch_mut(chid).points.armor -= delta;
    }
    let room = g.ch(chid).in_room;
    if room != NOWHERE {
        if pos == WEAR_LIGHT && g.obj(oid).type_flag == flags::ITEM_LIGHT && g.obj(oid).values[2] != 0 {
            g.rooms[room as usize].light += 1;
        }
    } else {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        g.log(format!("SYSERR: IN_ROOM(ch) = NOWHERE when equipping char {}.", name));
    }
    let (affs, perm) = {
        let o = g.obj(oid);
        (o.affected, o.perm_affects)
    };
    for a in affs {
        affect_modify_ar(g, chid, a.location, a.modifier, perm, true);
    }
    affect_total(g, chid);
    true
}

/// unequip_char: removes and returns the object id.
pub fn unequip_char(g: &mut Game, chid: CharId, pos: usize) -> Option<ObjId> {
    let oid = g.ch(chid).equipment[pos]?;
    {
        let o = g.obj_mut(oid);
        o.worn_by = None;
        o.worn_on = -1;
    }
    if g.obj(oid).type_flag == flags::ITEM_ARMOR {
        let delta = apply_ac(g, chid, pos);
        g.ch_mut(chid).points.armor += delta;
    }
    let room = g.ch(chid).in_room;
    if room != NOWHERE
        && pos == WEAR_LIGHT
        && g.obj(oid).type_flag == flags::ITEM_LIGHT
        && g.obj(oid).values[2] != 0
    {
        g.rooms[room as usize].light -= 1;
    }
    g.ch_mut(chid).equipment[pos] = None;
    let (affs, perm) = {
        let o = g.obj(oid);
        (o.affected, o.perm_affects)
    };
    for a in affs {
        affect_modify_ar(g, chid, a.location, a.modifier, perm, false);
    }
    affect_total(g, chid);
    Some(oid)
}

// ---- searching ----

/// get_char_room_vis: "0.name"/"all.name" handled by callers via
/// get_number; this takes a pre-split (number, name).
pub fn get_char_room_vis(g: &Game, chid: CharId, name: &[u8], number: Option<i32>) -> Option<CharId> {
    let ch = g.ch(chid);
    // "self"/"me" special.
    if name.eq_ignore_ascii_case(b"self") || name.eq_ignore_ascii_case(b"me") {
        return Some(chid);
    }
    let (mut num, name) = match number {
        Some(n) => (n, name.to_vec()),
        None => {
            let (n, stripped) = get_number(name);
            (n, stripped)
        }
    };
    if num == 0 {
        // 0.name means "PC with this exact name".
        return get_player_vis(g, chid, &name, true);
    }
    let room = ch.in_room;
    let mut last = None;
    for &other in &g.rooms[room as usize].people {
        let oc = g.ch(other);
        if isname(&name, oc.name.as_deref().unwrap_or(b"")) && can_see(g, chid, other) {
            if num == FIND_INDEX_LAST {
                last = Some(other); // keep looking: we want the far end
                continue;
            }
            num -= 1;
            if num == 0 {
                return Some(other);
            }
        }
    }
    last
}

/// get_player_vis: world-wide, players only, exact name match.
pub fn get_player_vis(g: &Game, chid: CharId, name: &[u8], inroom: bool) -> Option<CharId> {
    let room = g.ch(chid).in_room;
    for &other in &g.character_list {
        let Some(oc) = g.try_ch(other) else { continue };
        if oc.is_npc() {
            continue;
        }
        if inroom && oc.in_room != room {
            continue;
        }
        let pname = oc.name.as_deref().unwrap_or(b"");
        if !pname.eq_ignore_ascii_case(name) {
            continue;
        }
        if !can_see(g, chid, other) {
            continue;
        }
        return Some(other);
    }
    None
}

/// get_char_world_vis: room first, then world by keyword.
pub fn get_char_world_vis(g: &Game, chid: CharId, name: &[u8], number: Option<i32>) -> Option<CharId> {
    let (mut num, name) = match number {
        Some(n) => (n, name.to_vec()),
        None => get_number(name),
    };
    if let Some(found) = get_char_room_vis(g, chid, &name, Some(num)) {
        return Some(found);
    }
    if num == 0 {
        return None;
    }
    let room = g.ch(chid).in_room;
    if num == FIND_INDEX_LAST {
        // No countdown to thread: the room scan above already answered if it
        // had a match at all, so take the last one outside it.
        let mut last = None;
        for &other in &g.character_list {
            let Some(oc) = g.try_ch(other) else { continue };
            if oc.in_room == room {
                continue;
            }
            if isname(&name, oc.name.as_deref().unwrap_or(b"")) && can_see(g, chid, other) {
                last = Some(other);
            }
        }
        return last;
    }
    // Count room matches against the remaining number: the same counter
    // threads through both scans.
    if room != NOWHERE {
        for &other in &g.rooms[room as usize].people {
            let oc = g.ch(other);
            if isname(&name, oc.name.as_deref().unwrap_or(b"")) && can_see(g, chid, other) {
                num -= 1;
            }
        }
    }
    for &other in &g.character_list {
        let Some(oc) = g.try_ch(other) else { continue };
        if oc.in_room == room {
            continue; // already counted
        }
        if isname(&name, oc.name.as_deref().unwrap_or(b"")) && can_see(g, chid, other) {
            num -= 1;
            if num <= 0 {
                return Some(other);
            }
        }
    }
    None
}

/// get_obj_in_list_vis.
pub fn get_obj_in_list_vis(g: &Game, chid: CharId, name: &[u8], number: Option<i32>, list: &[ObjId]) -> Option<ObjId> {
    let (mut num, name) = match number {
        Some(n) => (n, name.to_vec()),
        None => get_number(name),
    };
    if num == 0 {
        return None;
    }
    get_obj_in_list_vis_counted(g, chid, &name, &mut num, list)
}

/// `number` counts DOWN and persists across calls, so "2.bread" spans
/// search domains rather than restarting in each one.
pub fn get_obj_in_list_vis_counted(
    g: &Game,
    chid: CharId,
    name: &[u8],
    number: &mut i32,
    list: &[ObjId],
) -> Option<ObjId> {
    let mut last = None;
    for &oid in list {
        if isname(name, obj_name(g, oid)) && can_see_obj(g, chid, oid) {
            if *number == FIND_INDEX_LAST {
                last = Some(oid); // keep looking: we want the far end
                continue;
            }
            *number -= 1;
            if *number == 0 {
                return Some(oid);
            }
        }
    }
    last
}

/// get_obj_vis: carried → room → whole object_list, threading
/// the countdown. The final world scan revisits carried and room items
/// rather than skipping them, so an item can be counted twice.
pub fn get_obj_vis_counted(g: &Game, chid: CharId, name: &[u8], number: &mut i32) -> Option<ObjId> {
    let carrying = &g.ch(chid).carrying;
    if let Some(oid) = get_obj_in_list_vis_counted(g, chid, name, number, carrying) {
        return Some(oid);
    }
    let room = g.ch(chid).in_room;
    if room != NOWHERE {
        let contents = &g.rooms[room as usize].contents;
        if let Some(oid) = get_obj_in_list_vis_counted(g, chid, name, number, contents) {
            return Some(oid);
        }
    }
    let mut last = None;
    for &oid in &g.object_list {
        if *number == 0 {
            break;
        }
        if g.try_obj(oid).is_none() {
            continue;
        }
        if isname(name, obj_name(g, oid)) && can_see_obj(g, chid, oid) {
            if *number == FIND_INDEX_LAST {
                last = Some(oid);
                continue;
            }
            *number -= 1;
            if *number == 0 {
                return Some(oid);
            }
        }
    }
    last
}

/// get_char_room_vis with a persistent countdown (see above).
pub fn get_char_room_vis_counted(g: &Game, chid: CharId, name: &[u8], number: &mut i32) -> Option<CharId> {
    if name.eq_ignore_ascii_case(b"self") || name.eq_ignore_ascii_case(b"me") {
        return Some(chid);
    }
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return None;
    }
    let mut last = None;
    for &other in &g.rooms[room as usize].people {
        let oc = g.ch(other);
        if isname(name, oc.name.as_deref().unwrap_or(b"")) && can_see(g, chid, other) {
            if *number == FIND_INDEX_LAST {
                last = Some(other);
                continue;
            }
            *number -= 1;
            if *number == 0 {
                return Some(other);
            }
        }
    }
    last
}

/// get_obj_pos_in_equip_vis, counted core.
pub fn get_obj_pos_in_equip_vis_counted(g: &Game, chid: CharId, name: &[u8], number: &mut i32) -> Option<usize> {
    let mut last = None;
    for j in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[j] {
            if can_see_obj(g, chid, oid) && isname(name, obj_name(g, oid)) {
                if *number == FIND_INDEX_LAST {
                    last = Some(j);
                    continue;
                }
                *number -= 1;
                if *number == 0 {
                    return Some(j);
                }
            }
        }
    }
    last
}

// FIND_* bits.
pub const FIND_CHAR_ROOM: i32 = 1 << 0;
pub const FIND_CHAR_WORLD: i32 = 1 << 1;
pub const FIND_OBJ_INV: i32 = 1 << 2;
pub const FIND_OBJ_ROOM: i32 = 1 << 3;
pub const FIND_OBJ_WORLD: i32 = 1 << 4;
pub const FIND_OBJ_EQUIP: i32 = 1 << 5;

/// generic_find. Returns (found-bit, char, obj); the
/// countdown from "N.name" threads through every enabled search domain.
pub fn generic_find(
    g: &Game,
    chid: CharId,
    arg: &[u8],
    bitvector: i32,
) -> (i32, Option<CharId>, Option<ObjId>) {
    let (name, _) = crate::interpreter::one_argument(arg);
    if name.is_empty() {
        return (0, None, None);
    }
    let (mut number, name) = get_number(&name);
    if number == 0 {
        return (0, None, None);
    }

    if bitvector & FIND_CHAR_ROOM != 0 {
        if let Some(ch) = get_char_room_vis_counted(g, chid, &name, &mut number) {
            return (FIND_CHAR_ROOM, Some(ch), None);
        }
    }
    if bitvector & FIND_CHAR_WORLD != 0 {
        if let Some(ch) = get_char_world_vis(g, chid, &name, Some(number)) {
            return (FIND_CHAR_WORLD, Some(ch), None);
        }
    }
    if bitvector & FIND_OBJ_EQUIP != 0 {
        if let Some(pos) = get_obj_pos_in_equip_vis_counted(g, chid, &name, &mut number) {
            return (FIND_OBJ_EQUIP, None, g.ch(chid).equipment[pos]);
        }
    }
    if bitvector & FIND_OBJ_INV != 0 {
        let list = &g.ch(chid).carrying;
        if let Some(oid) = get_obj_in_list_vis_counted(g, chid, &name, &mut number, list) {
            return (FIND_OBJ_INV, None, Some(oid));
        }
    }
    if bitvector & FIND_OBJ_ROOM != 0 {
        let room = g.ch(chid).in_room;
        if room != NOWHERE {
            let list = &g.rooms[room as usize].contents;
            if let Some(oid) = get_obj_in_list_vis_counted(g, chid, &name, &mut number, list) {
                return (FIND_OBJ_ROOM, None, Some(oid));
            }
        }
    }
    if bitvector & FIND_OBJ_WORLD != 0 {
        let mut last = None;
        for &oid in &g.object_list {
            if isname(&name, obj_name(g, oid)) && can_see_obj(g, chid, oid) {
                if number == FIND_INDEX_LAST {
                    last = Some(oid);
                    continue;
                }
                number -= 1;
                if number == 0 {
                    return (FIND_OBJ_WORLD, None, Some(oid));
                }
            }
        }
        if let Some(oid) = last {
            return (FIND_OBJ_WORLD, None, Some(oid));
        }
    }
    (0, None, None)
}

/// get_obj_pos_in_equip_vis: equipment slot by keyword.
pub fn get_obj_pos_in_equip_vis(g: &Game, chid: CharId, arg: &[u8], number: Option<i32>) -> Option<usize> {
    let (mut num, name) = match number {
        Some(n) => (n, arg.to_vec()),
        None => get_number(arg),
    };
    if num == 0 {
        return None;
    }
    let mut last = None;
    for j in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[j] {
            if can_see_obj(g, chid, oid) && isname(&name, obj_name(g, oid)) {
                if num == FIND_INDEX_LAST {
                    last = Some(j);
                    continue;
                }
                num -= 1;
                if num == 0 {
                    return Some(j);
                }
            }
        }
    }
    last
}

pub fn money_desc(g: &mut Game, amount: i32) -> Option<&'static str> {
    const MONEY_TABLE: &[(i32, &str)] = &[
        (1, "a gold coin"),
        (10, "a tiny pile of gold coins"),
        (20, "a handful of gold coins"),
        (75, "a little pile of gold coins"),
        (200, "a small pile of gold coins"),
        (1000, "a pile of gold coins"),
        (5000, "a big pile of gold coins"),
        (10000, "a large heap of gold coins"),
        (20000, "a huge mound of gold coins"),
        (75000, "an enormous mound of gold coins"),
        (150000, "a small mountain of gold coins"),
        (250000, "a mountain of gold coins"),
        (500000, "a huge mountain of gold coins"),
        (1000000, "an enormous mountain of gold coins"),
    ];
    if amount <= 0 {
        g.log(format!("SYSERR: Try to create negative or 0 money ({}).", amount));
        return None;
    }
    for &(limit, desc) in MONEY_TABLE {
        if amount <= limit {
            return Some(desc);
        }
    }
    Some("an absolutely colossal mountain of gold coins")
}

/// create_money. Consumes one rand_number draw for amounts
/// in [1000, 99999] (the "You guess" exdesc) — RNG call order is load-bearing.
pub fn create_money(g: &mut Game, amount: i32) -> Option<ObjId> {
    if amount <= 0 {
        g.log(format!("SYSERR: Try to create negative or 0 money. ({})", amount));
        return None;
    }
    let mut obj = crate::obj::create_obj();
    let (name, short, desc, ex_key, ex_desc): (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) =
        if amount == 1 {
            (
                b"coin gold".to_vec(),
                b"a gold coin".to_vec(),
                b"One miserable gold coin is lying here.".to_vec(),
                b"coin gold".to_vec(),
                b"It's just one miserable little gold coin.".to_vec(),
            )
        } else {
            let md = money_desc(g, amount).unwrap_or("").as_bytes().to_vec();
            let mut line = md.clone();
            line.extend_from_slice(b" is lying here.");
            // CAP the description.
            if let Some(c) = line.first_mut() {
                c.make_ascii_uppercase();
            }
            let guess: Vec<u8> = if amount < 10 {
                format!("There are {} coins.", amount).into_bytes()
            } else if amount < 100 {
                format!("There are about {} coins.", 10 * (amount / 10)).into_bytes()
            } else if amount < 1000 {
                format!("It looks to be about {} coins.", 100 * (amount / 100)).into_bytes()
            } else if amount < 100000 {
                let r = g.rng.rand_number(0, amount / 1000);
                format!("You guess there are, maybe, {} coins.", 1000 * ((amount / 1000) + r))
                    .into_bytes()
            } else {
                b"There are a LOT of coins.".to_vec()
            };
            (b"coins gold".to_vec(), md, line, b"coins gold".to_vec(), guess)
        };
    obj.name = Some(name);
    obj.short_description = Some(short);
    obj.description = Some(desc);
    obj.ex_descriptions =
        Some(vec![mud_world::model::ExtraDesc { keyword: Some(ex_key), description: Some(ex_desc) }]);
    obj.type_flag = flags::ITEM_MONEY;
    obj.wear_flags = FlagSet::default();
    obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
    obj.values[0] = amount;
    obj.cost = amount;
    obj.item_number = NOTHING;

    let id = g.objs.insert(obj);
    g.object_list.push_front(id);
    Some(id)
}

/// Returned by get_number for a "last.<name>" prefix: the caller wants the
/// match nearest the END of the list rather than the Nth from the front. A
/// negative value cannot collide with a real index -- the searches count down
/// from a positive one and already read 0 as "not a number".
pub const FIND_INDEX_LAST: i32 = -1;

/// get_number: split "3.sword" → (3, "sword"); plain → (1, name).
/// "last.sword" → (FIND_INDEX_LAST, "sword"): the match nearest the end of the
/// list. Only obj_to_room appends, so that is the newest object in a room --
/// every other list here is built by prepending.
pub fn get_number(name: &[u8]) -> (i32, Vec<u8>) {
    if let Some(dot) = name.iter().position(|c| *c == b'.') {
        let (head, tail) = name.split_at(dot);
        if head.eq_ignore_ascii_case(b"last") {
            return (FIND_INDEX_LAST, tail[1..].to_vec());
        }
        if !head.is_empty() && head.iter().all(|c| c.is_ascii_digit()) {
            return (atoi(head), tail[1..].to_vec());
        }
    }
    (1, name.to_vec())
}

// ---- extraction ----

/// extract_char: mark for the end-of-pulse sweep.
pub fn extract_char(g: &mut Game, chid: CharId) {
    let ch = g.ch_mut(chid);
    if ch.is_npc() {
        ch.act.set(flags::MOB_NOTDEADYET);
    } else {
        ch.act.set(flags::PLR_NOTDEADYET);
    }
    g.extractions_pending += 1;
}

/// extract_pending_chars: the per-pulse sweep in list order.
pub fn extract_pending_chars(g: &mut Game) {
    if g.extractions_pending < 0 {
        g.log(format!("SYSERR: Negative ({}) extractions pending.", g.extractions_pending));
    }
    let list = g.character_list.clone();
    for chid in list {
        if g.extractions_pending <= 0 {
            break;
        }
        let Some(ch) = g.try_ch(chid) else { continue };
        let marked = if ch.is_npc() {
            ch.act.is_set(flags::MOB_NOTDEADYET)
        } else {
            ch.act.is_set(flags::PLR_NOTDEADYET)
        };
        if !marked {
            continue;
        }
        {
            let ch = g.ch_mut(chid);
            if ch.is_npc() {
                ch.act.remove(flags::MOB_NOTDEADYET);
            } else {
                ch.act.remove(flags::PLR_NOTDEADYET);
            }
        }
        // Removed from character_list before the final teardown.
        g.character_list.retain(|c| *c != chid);
        extract_char_final(g, chid);
        g.extractions_pending -= 1;
    }
    if g.extractions_pending > 0 {
        g.log(format!("SYSERR: Couldn't find {} extractions as counted.", g.extractions_pending));
    }
    g.extractions_pending = 0;
}

/// extract_char_final, stage-2 subset: no corpse/group
/// interactions yet, but the exact descriptor semantics — a connected PC is
/// returned to the main menu with the char kept alive on the descriptor.
pub fn extract_char_final(g: &mut Game, chid: CharId) {
    mud_data::rng::rng_trace_note(&format!(
        "extract_char: {}",
        String::from_utf8_lossy(g.ch(chid).get_name())
    ));
    if g.ch(chid).in_room == NOWHERE {
        // Log and drop the char in room 0 to recover, rather than
        // aborting.
        g.log("SYSERR: NOWHERE extracting char. (extract_char_final) [F5: recovering]".to_string());
        char_to_room(g, chid, 0);
    }

    // Booting the body of someone who switched: stuff them back into their
    // own first, which is what gives this character its descriptor back
    if !g.ch(chid).is_npc() && g.ch(chid).desc.is_none() {
        let switched = g
            .descriptors
            .order
            .iter()
            .copied()
            .find(|&di| g.descriptors.get(di).and_then(|d| d.original) == Some(chid))
            .and_then(|di| g.descriptors.get(di).and_then(|d| d.character));
        if let Some(body) = switched {
            crate::act::wizard::do_return(g, body, b"", 0, 0);
        }
    }

    let desc = g.ch(chid).desc;
    if let Some(di) = desc {
        // Boot same-idnum dupes trying to log in (anti-dupe).
        let id = g.ch(chid).idnum;
        for odi in g.descriptors.indices() {
            if odi == di {
                continue;
            }
            let Some(od) = g.descriptors.get(odi) else { continue };
            let boot = od.character.and_then(|c| g.try_ch(c)).is_some_and(|c| !c.is_npc() && c.idnum == id);
            if boot {
                if let Some(od) = g.descriptors.get_mut(odi) {
                    od.state = ConState::Close;
                }
            }
        }
        if let Some(d) = g.descriptors.get_mut(di) {
            d.state = ConState::Menu;
        }
        let menu = g.config.menu.clone();
        crate::comm::write_to_desc(g, di, &menu);
    }

    // Follower/master cleanup (die_follower messages come with stage 4-5).
    let followers: Vec<CharId> = g.ch(chid).followers.clone();
    for f in followers {
        crate::act::movement::stop_follower(g, f);
    }
    if g.ch(chid).master.is_some() {
        crate::act::movement::stop_follower(g, chid);
    }

    // "Check to see if we are grouped!".
    if g.ch(chid).group.is_some() {
        leave_group(g, chid);
    }

    // Dump inventory and equipment on the ground.
    let room = g.ch(chid).in_room;
    let carried: Vec<ObjId> = g.ch(chid).carrying.clone();
    for oid in carried {
        obj_from_char(g, oid);
        obj_to_room(g, oid, room);
    }
    for pos in 0..NUM_WEARS {
        if let Some(oid) = unequip_char(g, chid, pos) {
            obj_to_room(g, oid, room);
        }
    }

    if g.ch(chid).fighting.is_some() {
        stop_fighting(g, chid);
    }
    let fighters: Vec<CharId> = g
        .character_list
        .iter()
        .copied()
        .filter(|c| g.try_ch(*c).is_some_and(|ch| ch.fighting == Some(chid)))
        .collect();
    for f in fighters {
        stop_fighting(g, f);
    }

    // Wipe character from the memory of hunters and other intelligent NPCs
    let dead_pc = !g.ch(chid).is_npc() && g.ch(chid).position == POS_DEAD;
    let npcs: Vec<CharId> = g
        .character_list
        .iter()
        .copied()
        .filter(|&c| c != chid && g.try_ch(c).is_some_and(|t| t.is_npc()))
        .collect();
    for t in npcs {
        if g.ch(t).hunting == Some(chid) {
            g.ch_mut(t).hunting = None;
        }
        if dead_pc && !g.ch(t).mob_specials.memory.is_empty() {
            crate::mobact::forget(g, t, chid);
        }
    }

    char_from_room(g, chid);

    let is_npc = g.ch(chid).is_npc();
    if is_npc {
        let rnum = g.ch(chid).mob_rnum;
        if rnum != NOBODY {
            g.mob_counts[rnum as usize] -= 1;
        }
        // clearMemory + extract_script + extract_script_mem.
        if g.ch(chid).script.is_some() {
            crate::dg::extract_script(g, crate::dg::GoId::Char(chid));
        }
        crate::dg::extract_script_mem(g, chid);
    } else {
        crate::players_glue::save_char(g, chid);
        // Only for a character who was actually in play. `stat file`
        // builds a scratch char, drops it in room 0 and extracts it — with
        // an unconditional call here would delete the crash file of a
        // player who is offline *because they crashed*, which is exactly
        // when they still need it. `pfilepos < 0` is the same "never
        // entered the game" marker save_char already gates on.
        if g.ch(chid).pfilepos >= 0 {
            crate::objsave::crash_delete_crashfile(g, chid);
        }
    }

    // Remove any pending event for/from this character.
    g.events
        .retain(|e| !matches!(e.kind, crate::game::EventKind::Whirlwind { ch } if ch == chid));

    // "If there's a descriptor, they're in the menu now." — only NPCs and
    // desc-less PCs are freed.
    if is_npc || desc.is_none() {
        free_char(g, chid);
    }
}

/// The free_char DG bookkeeping: drop the script (waits
/// cancel), the memory list, and the lookup-table entry, then free.
pub fn free_char(g: &mut Game, chid: CharId) {
    if let Some(ch) = g.try_ch(chid) {
        let sid = ch.script_id;
        if g.ch(chid).script.is_some() {
            crate::dg::extract_script(g, crate::dg::GoId::Char(chid));
        }
        crate::dg::extract_script_mem(g, chid);
        if sid != 0 {
            crate::dg::remove_from_lookup_table(g, sid);
        }
    }
    g.chars.remove(chid);
}

/// extract_obj: recursive teardown.
pub fn extract_obj(g: &mut Game, oid: ObjId) {
    if let Some(chid) = g.obj(oid).worn_by {
        let pos = g.obj(oid).worn_on;
        if pos >= 0 && unequip_char(g, chid, pos as usize) != Some(oid) {
            g.log("SYSERR: Inconsistent worn_by and worn_on pointers!!".to_string());
        }
    }
    if g.obj(oid).in_room != NOWHERE {
        obj_from_room(g, oid);
    } else if g.obj(oid).carried_by.is_some() {
        obj_from_char(g, oid);
    } else if g.obj(oid).in_obj.is_some() {
        obj_from_obj(g, oid);
    }
    let contents: Vec<ObjId> = g.obj(oid).contains.clone();
    for c in contents {
        extract_obj(g, c);
    }
    g.object_list.retain(|o| *o != oid);
    let rnum = g.obj(oid).item_number;
    if rnum != NOTHING {
        g.obj_counts[rnum as usize] -= 1;
    }
    // extract_script + free_obj's lookup removal.
    if g.obj(oid).script.is_some() {
        crate::dg::extract_script(g, crate::dg::GoId::Obj(oid));
    }
    let sid = g.obj(oid).script_id;
    if sid != 0 {
        crate::dg::remove_from_lookup_table(g, sid);
    }
    g.objs.remove(oid);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_with_str(str_: i8, add: i8) -> Char {
        let mut ch = Char::default();
        ch.aff_abils.str_ = str_;
        ch.aff_abils.str_add = add;
        ch
    }

    /// STRENGTH_APPLY_INDEX ladder.
    #[test]
    fn strength_apply_index_ladder() {
        assert_eq!(strength_apply_index(&char_with_str(11, 0)), 11);
        assert_eq!(strength_apply_index(&char_with_str(18, 0)), 18);
        assert_eq!(strength_apply_index(&char_with_str(18, 1)), 26);
        assert_eq!(strength_apply_index(&char_with_str(18, 50)), 26);
        assert_eq!(strength_apply_index(&char_with_str(18, 51)), 27);
        assert_eq!(strength_apply_index(&char_with_str(18, 75)), 27);
        assert_eq!(strength_apply_index(&char_with_str(18, 76)), 28);
        assert_eq!(strength_apply_index(&char_with_str(18, 90)), 28);
        assert_eq!(strength_apply_index(&char_with_str(18, 91)), 29);
        assert_eq!(strength_apply_index(&char_with_str(18, 99)), 29);
        assert_eq!(strength_apply_index(&char_with_str(18, 100)), 30);
        // str_add only matters at exactly 18.
        assert_eq!(strength_apply_index(&char_with_str(17, 100)), 17);
        assert_eq!(strength_apply_index(&char_with_str(25, 0)), 25);
    }

    #[test]
    fn can_carry_n_formula() {
        let mut ch = Char::default();
        ch.aff_abils.dex = 14;
        ch.level = 10;
        assert_eq!(can_carry_n(&ch), 5 + 7 + 5);
    }

    #[test]
    fn get_number_split() {
        assert_eq!(get_number(b"3.sword"), (3, b"sword".to_vec()));
        assert_eq!(get_number(b"sword"), (1, b"sword".to_vec()));
        assert_eq!(get_number(b"0.bob"), (0, b"bob".to_vec()));
        assert_eq!(get_number(b"a.b"), (1, b"a.b".to_vec()));
    }
}

/// rev_dir — the reverse-direction table.
pub fn rev_dir(dir: usize) -> usize {
    const REV: [usize; 10] = [2, 3, 0, 1, 5, 4, 8, 9, 6, 7];
    REV[dir % 10]
}

/// CAN_SEE_IN_DARK — re-exported so map/look share one definition.
pub fn can_see_in_dark(g: &Game, chid: CharId) -> bool {
    crate::act::informative::can_see_in_dark(g, chid)
}
