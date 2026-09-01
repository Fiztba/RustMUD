//! Saving throws, affect_update, and the mag_* dispatch routines
//! (damage, affects, groups, masses, areas, summons, points, unaffects,
//! alter_objs, creations, rooms).

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::spells::*;
use mud_data::types::*;

use crate::ch::Affect;
use crate::comm::{self, act, send_to_char};
use crate::fight::{damage, update_pos};
use crate::game::{EventKind, Game};
use crate::handler::{affect_from_char, affect_join, affected_by_spell};

/// CLASS_WARRIOR — the NPC saving-throw class.
const CLASS_WARRIOR: i32 = 3;

/// mag_savingthrow: lower is better; rolls 0 and 1 never
/// save. NPCs use warrior tables "according to some book" (A3: INTENDED).
pub fn mag_savingthrow(g: &mut Game, chid: CharId, save_type: i32, modifier: i32) -> bool {
    let class_sav = if g.ch(chid).is_npc() { CLASS_WARRIOR } else { g.ch(chid).class as i32 };
    let mut save = mud_data::tables::saving_throws(class_sav, save_type, g.ch(chid).level as i32) as i32;
    save += g.ch(chid).apply_saving_throw[save_type as usize] as i32;
    save += modifier;

    // Throwing a 0 is always a failure.
    save.max(1) < g.rng.rand_number(0, 99)
}

/// affect_update: every tick, decrement durations; expired
/// affects print their wear-off message (once per same-spell run) and go.
pub fn affect_update(g: &mut Game) {
    let chars = g.character_list.clone();
    for chid in chars {
        if g.try_ch(chid).is_none() {
            continue;
        }
        // Removal only happens at the cursor, so an index walk that
        // re-checks bounds is safe here.
        let mut i = 0;
        while i < g.ch(chid).affected.len() {
            let (duration, spell) = {
                let af = &g.ch(chid).affected[i];
                (af.duration, af.spell)
            };
            if duration >= 1 {
                g.ch_mut(chid).affected[i].duration -= 1;
                i += 1;
            } else if duration == -1 {
                // No action (permanent).
                i += 1;
            } else {
                if spell > 0 && (spell as i32) <= MAX_SPELLS {
                    let suppress = g
                        .ch(chid)
                        .affected
                        .get(i + 1)
                        .is_some_and(|next| next.spell == spell && next.duration <= 0);
                    if !suppress {
                        if let Some(msg) = spell_info(spell as i32).wear_off_msg {
                            let mut out = msg.as_bytes().to_vec();
                            out.extend_from_slice(b"\r\n");
                            send_to_char(g, chid, &out);
                        }
                    }
                }
                crate::handler::affect_remove(g, chid, i);
                // Do not advance: the next affect shifted into slot i.
            }
        }
    }
}

/// mag_materials: reagent check for clone; ANDed vnums,
/// verbose failure insults are one rand_number(0,2) draw.
fn mag_materials(
    g: &mut Game,
    chid: CharId,
    item0: Idx,
    item1: Idx,
    item2: Idx,
    extract: bool,
    verbose: bool,
) -> bool {
    let mut need0 = item0;
    let mut need1 = item1;
    let mut need2 = item2;
    let mut obj0 = None;
    let mut obj1 = None;
    let mut obj2 = None;
    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        let vnum = {
            let o = g.obj(oid);
            if o.item_number == NOTHING { continue } else { g.world.obj_protos[o.item_number as usize].vnum }
        };
        if need0 != NOTHING && vnum == need0 {
            obj0 = Some(oid);
            need0 = NOTHING;
        } else if need1 != NOTHING && vnum == need1 {
            obj1 = Some(oid);
            need1 = NOTHING;
        } else if need2 != NOTHING && vnum == need2 {
            obj2 = Some(oid);
            need2 = NOTHING;
        }
    }

    if need0 != NOTHING || need1 != NOTHING || need2 != NOTHING {
        if verbose {
            match g.rng.rand_number(0, 2) {
                0 => send_to_char(g, chid, b"A wart sprouts on your nose.\r\n"),
                1 => send_to_char(g, chid, b"Your hair falls out in clumps.\r\n"),
                _ => send_to_char(g, chid, b"A huge corn develops on your big toe.\r\n"),
            }
        }
        return false;
    }

    if extract {
        for o in [obj0, obj1, obj2].into_iter().flatten() {
            crate::handler::extract_obj(g, o);
        }
        if verbose {
            send_to_char(g, chid, b"A puff of smoke rises from your pack.\r\n");
            act(g, b"A puff of smoke rises from $n's pack.", true, Some(chid), None, None, comm::TO_ROOM);
        }
    }
    if !extract && verbose {
        send_to_char(g, chid, b"Your pack rumbles.\r\n");
        act(g, b"Something rumbles in $n's pack.", true, Some(chid), None, None, comm::TO_ROOM);
    }
    true
}

/// IS_MAGIC_USER: false for every NPC by definition.
fn is_magic_user(g: &Game, chid: CharId) -> bool {
    !g.ch(chid).is_npc() && g.ch(chid).class as i32 == 0
}

/// mag_damage: per-spell dice, save-for-half, damage.
pub fn mag_damage(
    g: &mut Game,
    level: i32,
    chid: CharId,
    victim: Option<CharId>,
    spellnum: i32,
    savetype: i32,
) -> i32 {
    let Some(mut victim) = victim else { return 0 };
    if g.try_ch(victim).is_none() {
        return 0;
    }
    let mut dam;

    match spellnum {
        SPELL_MAGIC_MISSILE | SPELL_CHILL_TOUCH => {
            dam = if is_magic_user(g, chid) { g.rng.dice(1, 8) + 1 } else { g.rng.dice(1, 6) + 1 };
        }
        SPELL_BURNING_HANDS => {
            dam = if is_magic_user(g, chid) { g.rng.dice(3, 8) + 3 } else { g.rng.dice(3, 6) + 3 };
        }
        SPELL_SHOCKING_GRASP => {
            dam = if is_magic_user(g, chid) { g.rng.dice(5, 8) + 5 } else { g.rng.dice(5, 6) + 5 };
        }
        SPELL_LIGHTNING_BOLT => {
            dam = if is_magic_user(g, chid) { g.rng.dice(7, 8) + 7 } else { g.rng.dice(7, 6) + 7 };
        }
        SPELL_COLOR_SPRAY => {
            dam = if is_magic_user(g, chid) { g.rng.dice(9, 8) + 9 } else { g.rng.dice(9, 6) + 9 };
        }
        SPELL_FIREBALL => {
            dam = if is_magic_user(g, chid) { g.rng.dice(11, 8) + 11 } else { g.rng.dice(11, 6) + 11 };
        }
        SPELL_DISPEL_EVIL => {
            dam = g.rng.dice(6, 8) + 6;
            if g.ch(chid).alignment <= -350 {
                victim = chid;
                dam = g.ch(chid).points.hit - 1;
            } else if g.ch(victim).alignment >= 350 {
                act(g, b"The gods protect $N.", false, Some(chid), None, Some(victim), comm::TO_CHAR);
                return 0;
            }
        }
        SPELL_DISPEL_GOOD => {
            dam = g.rng.dice(6, 8) + 6;
            if g.ch(chid).alignment >= 350 {
                victim = chid;
                dam = g.ch(chid).points.hit - 1;
            } else if g.ch(victim).alignment <= -350 {
                act(g, b"The gods protect $N.", false, Some(chid), None, Some(victim), comm::TO_CHAR);
                return 0;
            }
        }
        SPELL_CALL_LIGHTNING => {
            dam = g.rng.dice(7, 8) + 7;
        }
        SPELL_HARM => {
            dam = g.rng.dice(8, 8) + 8;
        }
        SPELL_ENERGY_DRAIN => {
            dam = if g.ch(victim).level <= 2 { 100 } else { g.rng.dice(1, 10) };
        }
        SPELL_EARTHQUAKE => {
            dam = g.rng.dice(2, 8) + level;
        }
        _ => {
            dam = 0;
        }
    }

    // Divide damage by two if victim makes his saving throw.
    if mag_savingthrow(g, victim, savetype, 0) {
        dam /= 2;
    }

    damage(g, chid, victim, dam, spellnum)
}

const MAX_SPELL_AFFECTS: usize = 5;

pub fn mag_affects(
    g: &mut Game,
    level: i32,
    chid: CharId,
    victim: Option<CharId>,
    spellnum: i32,
    savetype: i32,
) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() {
        return;
    }

    let mut af: [Affect; MAX_SPELL_AFFECTS] = Default::default();
    for a in af.iter_mut() {
        a.spell = spellnum as i16;
    }
    let mut accum_affect = false;
    let mut accum_duration = false;
    let mut to_vict: Option<&[u8]> = None;
    let mut to_room: Option<&[u8]> = None;
    let caster_level = g.ch(chid).level as i32;

    match spellnum {
        SPELL_CHILL_TOUCH => {
            af[0].location = flags::APPLY_STR as u8;
            af[0].duration = if mag_savingthrow(g, victim, savetype, 0) { 1 } else { 4 };
            af[0].modifier = -1;
            accum_duration = true;
            to_vict = Some(b"You feel your strength wither!");
        }
        SPELL_ARMOR => {
            af[0].location = flags::APPLY_AC as u8;
            af[0].modifier = -20;
            af[0].duration = 24;
            accum_duration = true;
            to_vict = Some(b"You feel someone protecting you.");
        }
        SPELL_BLESS => {
            af[0].location = flags::APPLY_HITROLL as u8;
            af[0].modifier = 2;
            af[0].duration = 6;
            af[1].location = flags::APPLY_SAVING_SPELL as u8;
            af[1].modifier = -1;
            af[1].duration = 6;
            accum_duration = true;
            to_vict = Some(b"You feel righteous.");
        }
        SPELL_BLINDNESS => {
            if g.ch(victim).mob_flagged(flags::MOB_NOBLIND)
                || g.ch(victim).level as i32 >= LVL_IMMORT as i32
                || mag_savingthrow(g, victim, savetype, 0)
            {
                send_to_char(g, chid, b"You fail.\r\n");
                return;
            }
            af[0].location = flags::APPLY_HITROLL as u8;
            af[0].modifier = -4;
            af[0].duration = 2;
            af[0].bitvector.set(flags::AFF_BLIND);
            af[1].location = flags::APPLY_AC as u8;
            af[1].modifier = 40;
            af[1].duration = 2;
            af[1].bitvector.set(flags::AFF_BLIND);
            to_room = Some(b"$n seems to be blinded!");
            to_vict = Some(b"You have been blinded!");
        }
        SPELL_CURSE => {
            if mag_savingthrow(g, victim, savetype, 0) {
                let msg = g.config.noeffect.clone();
                send_to_char(g, chid, &msg);
                return;
            }
            af[0].location = flags::APPLY_HITROLL as u8;
            af[0].duration = (1 + caster_level / 2) as i16;
            af[0].modifier = -1;
            af[0].bitvector.set(flags::AFF_CURSE);
            af[1].location = flags::APPLY_DAMROLL as u8;
            af[1].duration = (1 + caster_level / 2) as i16;
            af[1].modifier = -1;
            af[1].bitvector.set(flags::AFF_CURSE);
            accum_duration = true;
            accum_affect = true;
            to_room = Some(b"$n briefly glows red!");
            to_vict = Some(b"You feel very uncomfortable.");
        }
        SPELL_DETECT_ALIGN => {
            af[0].duration = (12 + level) as i16;
            af[0].bitvector.set(flags::AFF_DETECT_ALIGN);
            accum_duration = true;
            to_vict = Some(b"Your eyes tingle.");
        }
        SPELL_DETECT_INVIS => {
            af[0].duration = (12 + level) as i16;
            af[0].bitvector.set(flags::AFF_DETECT_INVIS);
            accum_duration = true;
            to_vict = Some(b"Your eyes tingle.");
        }
        SPELL_DETECT_MAGIC => {
            af[0].duration = (12 + level) as i16;
            af[0].bitvector.set(flags::AFF_DETECT_MAGIC);
            accum_duration = true;
            to_vict = Some(b"Your eyes tingle.");
        }
        SPELL_FLY => {
            af[0].duration = 24;
            af[0].bitvector.set(flags::AFF_FLYING);
            accum_duration = true;
            to_vict = Some(b"You float above the ground.");
        }
        SPELL_INFRAVISION => {
            af[0].duration = (12 + level) as i16;
            af[0].bitvector.set(flags::AFF_INFRAVISION);
            accum_duration = true;
            to_vict = Some(b"Your eyes glow red.");
            to_room = Some(b"$n's eyes glow red.");
        }
        SPELL_INVISIBLE => {
            af[0].duration = (12 + caster_level / 4) as i16;
            af[0].modifier = -40;
            af[0].location = flags::APPLY_AC as u8;
            af[0].bitvector.set(flags::AFF_INVISIBLE);
            accum_duration = true;
            to_vict = Some(b"You vanish.");
            to_room = Some(b"$n slowly fades out of existence.");
        }
        mud_data::spells::SPELL_POISON => {
            if mag_savingthrow(g, victim, savetype, 0) {
                let msg = g.config.noeffect.clone();
                send_to_char(g, chid, &msg);
                return;
            }
            af[0].location = flags::APPLY_STR as u8;
            af[0].duration = caster_level as i16;
            af[0].modifier = -2;
            af[0].bitvector.set(flags::AFF_POISON);
            to_vict = Some(b"You feel very sick.");
            to_room = Some(b"$n gets violently ill!");
        }
        SPELL_PROT_FROM_EVIL => {
            af[0].duration = 24;
            af[0].bitvector.set(flags::AFF_PROTECT_EVIL);
            accum_duration = true;
            to_vict = Some(b"You feel invulnerable!");
        }
        SPELL_SANCTUARY => {
            af[0].duration = 4;
            af[0].bitvector.set(flags::AFF_SANCTUARY);
            accum_duration = true;
            to_vict = Some(b"A white aura momentarily surrounds you.");
            to_room = Some(b"$n is surrounded by a white aura.");
        }
        SPELL_SLEEP => {
            if g.config.pk_setting == 0 && !g.ch(chid).is_npc() && !g.ch(victim).is_npc() {
                return;
            }
            if g.ch(victim).mob_flagged(flags::MOB_NOSLEEP) {
                return;
            }
            if mag_savingthrow(g, victim, savetype, 0) {
                return;
            }
            af[0].duration = (4 + caster_level / 4) as i16;
            af[0].bitvector.set(flags::AFF_SLEEP);

            if g.ch(victim).position > POS_SLEEPING {
                send_to_char(g, victim, b"You feel very sleepy...  Zzzz......\r\n");
                act(g, b"$n goes to sleep.", true, Some(victim), None, None, comm::TO_ROOM);
                g.ch_mut(victim).position = POS_SLEEPING;
            }
        }
        SPELL_STRENGTH => {
            if g.ch(victim).aff_abils.str_add as i32 == 100 {
                return;
            }
            af[0].location = flags::APPLY_STR as u8;
            af[0].duration = (caster_level / 2 + 4) as i16;
            af[0].modifier = 1 + if level > 18 { 1 } else { 0 };
            accum_duration = true;
            accum_affect = true;
            to_vict = Some(b"You feel stronger!");
        }
        SPELL_SENSE_LIFE => {
            to_vict = Some(b"Your feel your awareness improve.");
            af[0].duration = caster_level as i16;
            af[0].bitvector.set(flags::AFF_SENSE_LIFE);
            accum_duration = true;
        }
        SPELL_WATERWALK => {
            af[0].duration = 24;
            af[0].bitvector.set(flags::AFF_WATERWALK);
            accum_duration = true;
            to_vict = Some(b"You feel webbing between your toes.");
        }
        _ => {}
    }

    // If this is a mob that has this affect set in its mob file, do not
    // perform the affect — you can't un-sanc a mob by
    // sancting it and waiting.
    if g.ch(victim).is_npc() && !affected_by_spell(g, victim, spellnum as i16) {
        for a in af.iter() {
            for j in 1..flags::NUM_AFF_FLAGS {
                if a.bitvector.is_set(j) && g.ch(victim).aff(j) {
                    let msg = g.config.noeffect.clone();
                    send_to_char(g, chid, &msg);
                    return;
                }
            }
        }
    }

    // Already affected and not accumulative → fail.
    if affected_by_spell(g, victim, spellnum as i16) && !(accum_duration || accum_affect) {
        let msg = g.config.noeffect.clone();
        send_to_char(g, chid, &msg);
        return;
    }

    for a in af.iter() {
        if !a.bitvector.is_empty() || a.location != flags::APPLY_NONE as u8 {
            affect_join(g, victim, a.clone(), accum_duration, false, accum_affect, false);
        }
    }

    if let Some(tv) = to_vict {
        act(g, tv, false, Some(victim), None, Some(chid), comm::TO_CHAR);
    }
    if let Some(tr) = to_room {
        act(g, tr, true, Some(victim), None, Some(chid), comm::TO_ROOM);
    }
}

fn perform_mag_groups(g: &mut Game, level: i32, chid: CharId, tch: CharId, spellnum: i32, savetype: i32) {
    match spellnum {
        SPELL_GROUP_HEAL => mag_points(g, level, chid, Some(tch), SPELL_HEAL, savetype),
        SPELL_GROUP_ARMOR => mag_affects(g, level, chid, Some(tch), SPELL_ARMOR, savetype),
        SPELL_GROUP_RECALL => crate::spells::spell_recall(g, level, chid, Some(tch), None),
        _ => {}
    }
}

/// mag_groups: members in the caster's room, caster last.
pub fn mag_groups(g: &mut Game, level: i32, chid: CharId, spellnum: i32, savetype: i32) {
    let Some(gr) = g.group_of(chid) else { return };
    let members = gr.members.clone();
    let room = g.ch(chid).in_room;
    for tch in members {
        if tch == chid {
            continue;
        }
        if g.try_ch(tch).is_none_or(|c| c.in_room != room) {
            continue;
        }
        perform_mag_groups(g, level, chid, tch, spellnum, savetype);
    }
    perform_mag_groups(g, level, chid, chid, spellnum, savetype);
}

/// mag_masses: a stub over the room — no mass spells exist.
pub fn mag_masses(g: &mut Game, _level: i32, chid: CharId, _spellnum: i32, _savetype: i32) {
    let room = g.ch(chid).in_room;
    let people = g.rooms[room as usize].people.clone();
    for tch in people {
        if tch == chid {
            continue;
        }
        // No spell uses this path.
    }
}

/// mag_areas — earthquake. Savetype hardcoded 1 (ROD).
pub fn mag_areas(g: &mut Game, level: i32, chid: CharId, spellnum: i32, _savetype: i32) {
    let mut to_char: Option<&[u8]> = None;
    let mut to_room: Option<&[u8]> = None;

    if spellnum == SPELL_EARTHQUAKE {
        to_char = Some(b"You gesture and the earth begins to shake all around you!");
        to_room = Some(b"$n gracefully gestures and the earth begins to shake violently!");
    }

    if let Some(tc) = to_char {
        act(g, tc, false, Some(chid), None, None, comm::TO_CHAR);
    }
    if let Some(tr) = to_room {
        act(g, tr, false, Some(chid), None, None, comm::TO_ROOM);
    }

    let room = g.ch(chid).in_room;
    let people = g.rooms[room as usize].people.clone();
    for tch in people {
        if tch == chid {
            continue;
        }
        let Some(t) = g.try_ch(tch) else { continue };
        if !t.is_npc() && t.level as i32 >= LVL_IMMORT as i32 {
            continue;
        }
        if g.config.pk_setting == 0 && !g.ch(chid).is_npc() && !t.is_npc() {
            continue;
        }
        if !g.ch(chid).is_npc() && t.is_npc() && t.aff(flags::AFF_CHARM) {
            continue;
        }
        if !t.is_npc()
            && spell_info(spellnum).violent
            && t.group.is_some()
            && g.ch(chid).group.is_some()
            && t.group == g.ch(chid).group
        {
            continue;
        }
        if spellnum == SPELL_EARTHQUAKE && t.aff(flags::AFF_FLYING) {
            continue;
        }
        // Doesn't matter if they die here so we don't check. -gg 6/24/98
        mag_damage(g, level, chid, Some(tch), spellnum, 1);
    }
}

/// mag_summon_msgs with the missing-comma merge at [6].
const MAG_SUMMON_MSGS: [&[u8]; 12] = [
    b"\r\n",
    b"$n makes a strange magical gesture; you feel a strong breeze!",
    b"$n animates a corpse!",
    b"$N appears from a cloud of thick blue smoke!",
    b"$N appears from a cloud of thick green smoke!",
    b"$N appears from a cloud of thick red smoke!",
    b"$n disappears in a thick black cloud!As $n makes a strange magical gesture, you feel a strong breeze.",
    b"As $n makes a strange magical gesture, you feel a searing heat.",
    b"As $n makes a strange magical gesture, you feel a sudden chill.",
    b"As $n makes a strange magical gesture, you feel the dust swirl.",
    b"$n magically divides!",
    b"$n animates a corpse!",
];

const MAG_SUMMON_FAIL_MSGS: [&[u8]; 8] = [
    b"\r\n",
    b"There are no such creatures.\r\n",
    b"Uh oh...\r\n",
    b"Oh dear.\r\n",
    b"Gosh durnit!\r\n",
    b"The elements resist!\r\n",
    b"You failed.\r\n",
    b"There is no corpse!\r\n",
];

const MOB_CLONE: Idx = 10;
const OBJ_CLONE: Idx = 161;
const MOB_ZOMBIE: Idx = 11;

/// IS_CORPSE: container with val3 == 1.
fn is_corpse(g: &Game, oid: ObjId) -> bool {
    let o = g.obj(oid);
    o.type_flag == flags::ITEM_CONTAINER && o.values[3] == 1
}

/// mag_summons — clone and animate dead.
pub fn mag_summons(g: &mut Game, _level: i32, chid: CharId, obj: Option<ObjId>, spellnum: i32, _savetype: i32) {
    let pfail;
    let msg;
    let fmsg;
    let num = 1;
    let mut handle_corpse = false;
    let mob_vnum;

    match spellnum {
        SPELL_CLONE => {
            msg = 10;
            fmsg = g.rng.rand_number(2, 6); // Random fail message.
            mob_vnum = MOB_CLONE;
            if !mag_materials(g, chid, OBJ_CLONE, NOTHING, NOTHING, true, true) {
                pfail = 102; // No materials, spell fails.
            } else {
                pfail = 0;
            }
        }
        SPELL_ANIMATE_DEAD => {
            if obj.is_none() || !is_corpse(g, obj.unwrap()) {
                act(g, MAG_SUMMON_FAIL_MSGS[7], false, Some(chid), None, None, comm::TO_CHAR);
                return;
            }
            handle_corpse = true;
            msg = 11;
            fmsg = g.rng.rand_number(2, 6);
            mob_vnum = MOB_ZOMBIE;
            pfail = 10;
        }
        _ => return,
    }

    if g.ch(chid).aff(flags::AFF_CHARM) {
        send_to_char(g, chid, b"You are too giddy to have any followers!\r\n");
        return;
    }
    if g.rng.rand_number(0, 101) < pfail {
        send_to_char(g, chid, MAG_SUMMON_FAIL_MSGS[fmsg as usize]);
        return;
    }

    let mut mob = None;
    for _ in 0..num {
        let rnum = g.world.real_mobile(mob_vnum);
        let m = rnum.and_then(|r| crate::db::read_mobile(g, r));
        let Some(m) = m else {
            send_to_char(g, chid, b"You don't quite remember how to make that creature.\r\n");
            return;
        };
        mob = Some(m);
        let room = g.ch(chid).in_room;
        crate::handler::char_to_room(g, m, room);
        g.ch_mut(m).carry_weight = 0;
        g.ch_mut(m).carry_items = 0;
        g.ch_mut(m).affected_by.set(flags::AFF_CHARM);
        if spellnum == SPELL_CLONE {
            // Don't mess up the prototype; use new string copies.
            let name = g.ch(chid).get_name().to_vec();
            g.ch_mut(m).name = Some(name.clone());
            g.ch_mut(m).short_descr = Some(name);
        }
        act_char2(g, MAG_SUMMON_MSGS[msg], false, chid, m, comm::TO_ROOM);
        crate::dg::triggers::load_mtrigger(g, m);
        crate::act::movement::add_follower(g, m, chid);

        if let Some(gr) = g.group_of(chid) {
            if gr.leader == Some(chid) {
                let gid = gr.id;
                crate::handler::join_group(g, m, gid);
            }
        }
    }
    if handle_corpse {
        let corpse = obj.unwrap();
        if let Some(m) = mob {
            let contents = g.obj(corpse).contains.clone();
            for tobj in contents {
                crate::handler::obj_from_obj(g, tobj);
                crate::handler::obj_to_char(g, tobj, m);
            }
        }
        crate::handler::extract_obj(g, corpse);
    }
}

/// act with a char as $N (vict_obj) — mag_summons room messages.
fn act_char2(g: &mut Game, msg: &[u8], hide: bool, chid: CharId, vict: CharId, to: i32) {
    act(g, msg, hide, Some(chid), None, Some(vict), to);
}

pub fn mag_points(g: &mut Game, level: i32, _chid: CharId, victim: Option<CharId>, spellnum: i32, _savetype: i32) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() {
        return;
    }
    let mut healing = 0;
    let move_ = 0;

    match spellnum {
        SPELL_CURE_LIGHT => {
            healing = g.rng.dice(1, 8) + 1 + level / 4;
            send_to_char(g, victim, b"You feel better.\r\n");
        }
        SPELL_CURE_CRITIC => {
            healing = g.rng.dice(3, 8) + 3 + level / 4;
            send_to_char(g, victim, b"You feel a lot better!\r\n");
        }
        SPELL_HEAL => {
            healing = 100 + g.rng.dice(3, 8);
            send_to_char(g, victim, b"A warm feeling floods your body.\r\n");
        }
        _ => {}
    }
    {
        let v = g.ch_mut(victim);
        v.points.hit = v.points.max_hit.min(v.points.hit + healing);
        v.points.mov = v.points.max_move.min(v.points.mov + move_);
    }
    update_pos(g, victim);
}

pub fn mag_unaffects(g: &mut Game, _level: i32, chid: CharId, victim: Option<CharId>, spellnum: i32, _type: i32) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() {
        return;
    }

    let spell;
    let to_vict: &[u8];
    let mut to_room: Option<&[u8]> = None;
    let mut msg_not_affected = true;

    match spellnum {
        SPELL_HEAL | SPELL_CURE_BLIND => {
            if spellnum == SPELL_HEAL {
                // Heal also restores health, so no "no effect" message when
                // the target isn't blind (deliberate fall-through).
                msg_not_affected = false;
            }
            spell = SPELL_BLINDNESS;
            to_vict = b"Your vision returns!";
            to_room = Some(b"There's a momentary gleam in $n's eyes.");
        }
        SPELL_REMOVE_POISON => {
            spell = mud_data::spells::SPELL_POISON;
            to_vict = b"A warm feeling runs through your body!";
            to_room = Some(b"$n looks better.");
        }
        SPELL_REMOVE_CURSE => {
            spell = SPELL_CURSE;
            to_vict = b"You don't feel so unlucky.";
        }
        _ => {
            g.log(format!("SYSERR: unknown spellnum {} passed to mag_unaffects.", spellnum));
            return;
        }
    }

    if !affected_by_spell(g, victim, spell as i16) {
        if msg_not_affected {
            let msg = g.config.noeffect.clone();
            send_to_char(g, chid, &msg);
        }
        return;
    }

    affect_from_char(g, victim, spell as i16);
    act(g, to_vict, false, Some(victim), None, Some(chid), comm::TO_CHAR);
    if let Some(tr) = to_room {
        act(g, tr, true, Some(victim), None, Some(chid), comm::TO_ROOM);
    }
}

/// mag_alter_objs. The to_char text is echoed to the room
/// too (to_room is always NULL here).
pub fn mag_alter_objs(g: &mut Game, _level: i32, chid: CharId, obj: Option<ObjId>, spellnum: i32, _savetype: i32) {
    let Some(oid) = obj else { return };
    if g.try_obj(oid).is_none() {
        return;
    }
    let mut to_char: Option<&[u8]> = None;
    let caster_level = g.ch(chid).level as i32;

    match spellnum {
        SPELL_BLESS => {
            if !g.obj(oid).extra_flags.is_set(flags::ITEM_BLESS)
                && g.obj(oid).weight <= 5 * caster_level
            {
                g.obj_mut(oid).extra_flags.set(flags::ITEM_BLESS);
                to_char = Some(b"$p glows briefly.");
            }
        }
        SPELL_CURSE => {
            if !g.obj(oid).extra_flags.is_set(flags::ITEM_NODROP) {
                g.obj_mut(oid).extra_flags.set(flags::ITEM_NODROP);
                if g.obj(oid).type_flag == flags::ITEM_WEAPON {
                    g.obj_mut(oid).values[2] -= 1;
                }
                to_char = Some(b"$p briefly glows red.");
            }
        }
        SPELL_INVISIBLE => {
            if !g.obj(oid).extra_flags.is_set(flags::ITEM_NOINVIS)
                && !g.obj(oid).extra_flags.is_set(flags::ITEM_INVISIBLE)
            {
                g.obj_mut(oid).extra_flags.set(flags::ITEM_INVISIBLE);
                to_char = Some(b"$p vanishes.");
            }
        }
        mud_data::spells::SPELL_POISON => {
            let t = g.obj(oid).type_flag;
            if (t == flags::ITEM_DRINKCON || t == flags::ITEM_FOUNTAIN || t == flags::ITEM_FOOD)
                && g.obj(oid).values[3] == 0
            {
                g.obj_mut(oid).values[3] = 1;
                to_char = Some(b"$p steams briefly.");
            }
        }
        SPELL_REMOVE_CURSE => {
            if g.obj(oid).extra_flags.is_set(flags::ITEM_NODROP) {
                g.obj_mut(oid).extra_flags.remove(flags::ITEM_NODROP);
                if g.obj(oid).type_flag == flags::ITEM_WEAPON {
                    g.obj_mut(oid).values[2] += 1;
                }
                to_char = Some(b"$p briefly glows blue.");
            }
        }
        SPELL_REMOVE_POISON => {
            let t = g.obj(oid).type_flag;
            if (t == flags::ITEM_DRINKCON || t == flags::ITEM_FOUNTAIN || t == flags::ITEM_FOOD)
                && g.obj(oid).values[3] != 0
            {
                g.obj_mut(oid).values[3] = 0;
                to_char = Some(b"$p steams briefly.");
            }
        }
        _ => {}
    }

    match to_char {
        None => {
            let msg = g.config.noeffect.clone();
            send_to_char(g, chid, &msg);
        }
        Some(tc) => {
            act(g, tc, true, Some(chid), Some(oid), None, comm::TO_CHAR);
            act(g, tc, true, Some(chid), Some(oid), None, comm::TO_ROOM);
        }
    }
}

/// mag_creations — create food (waybread, vnum 10).
pub fn mag_creations(g: &mut Game, _level: i32, chid: CharId, spellnum: i32) {
    let z: Idx = match spellnum {
        SPELL_CREATE_FOOD => 10,
        _ => {
            send_to_char(g, chid, b"Spell unimplemented, it would seem.\r\n");
            return;
        }
    };

    let rnum = g.world.real_object(z);
    let tobj = rnum.and_then(|r| crate::db::read_object(g, r));
    let Some(tobj) = tobj else {
        send_to_char(g, chid, b"I seem to have goofed.\r\n");
        g.log(format!("SYSERR: spell_creations, spell {}, obj {}: obj not found", spellnum, z));
        return;
    };
    crate::handler::obj_to_char(g, tobj, chid);
    act(g, b"$n creates $p.", false, Some(chid), Some(tobj), None, comm::TO_ROOM);
    act(g, b"You create $p.", false, Some(chid), Some(tobj), None, comm::TO_CHAR);
    crate::dg::triggers::load_otrigger(g, tobj);
}

/// mag_rooms — darkness. The ROOM_DARK bit is set inside
/// the case before failure is evaluated (only NOMAGIC and
/// already-dark rooms fail, and there the set is unreachable or a no-op).
/// The darkness lasts 5 MUD hours.
pub fn mag_rooms(g: &mut Game, _level: i32, chid: CharId, spellnum: i32) {
    let rnum = g.ch(chid).in_room;
    let mut failure = false;
    let mut known = false;

    if rnum == NOWHERE {
        return;
    }
    let flag_set = |g: &mut Game, bit: usize| {
        g.world.rooms[rnum as usize].room_flags[bit / 32] |= 1 << (bit % 32);
    };
    let flagged = |g: &Game, bit: usize| {
        g.world.rooms[rnum as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0
    };

    if flagged(g, flags::ROOM_NOMAGIC) {
        failure = true;
    }

    let mut msg: &[u8] = b"";
    let mut room_msg: &[u8] = b"";
    let mut duration_pulses: u64 = 0;
    if spellnum == SPELL_DARKNESS {
        known = true;
        if flagged(g, flags::ROOM_DARK) {
            failure = true;
        }
        duration_pulses = 5 * SECS_PER_MUD_HOUR * PASSES_PER_SEC; // B3: 5 mud hours
        flag_set(g, flags::ROOM_DARK);

        msg = b"You cast a shroud of darkness upon the area.";
        room_msg = b"$n casts a shroud of darkness upon this area.";
    }

    if failure || !known {
        send_to_char(g, chid, b"You failed!\r\n");
        return;
    }

    let mut out = msg.to_vec();
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
    act(g, room_msg, false, Some(chid), None, None, comm::TO_ROOM);

    g.queue_event(duration_pulses, EventKind::SplDarkness { room: rnum });
}
