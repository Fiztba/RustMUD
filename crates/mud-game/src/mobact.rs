//! Mobile_activity (spec proc, hunt, scavenger, movement,
//! aggressive, memory, charm rebellion, helper) and the mob memory routines.
//! RNG draw order is load-bearing, because a seeded run has to reproduce
//! its draw sequence exactly: the movement roll draws for every standing
//! non-sentinel mob, the scavenger roll only when the room has contents,
//! and the leash only for charmed mobs.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::spells::TYPE_UNDEFINED;
use mud_data::types::*;

use crate::comm::{act, TO_ROOM};
use crate::fight::hit;
use crate::game::Game;
use crate::handler::can_see;

/// mobile_activity, every PULSE_MOBILE.
pub fn mobile_activity(g: &mut Game) {
    let chars = g.character_list.clone();
    for chid in chars {
        let Some(ch) = g.try_ch(chid) else { continue };
        // IS_MOB: an NPC with a real prototype.
        if !ch.is_npc() || ch.mob_rnum == NOBODY {
            continue;
        }
        // MUD_RNG_TRACE marker (no-op without the env): brackets each mob's
        // draws, so a trace can be read back per mob.
        mud_data::rng::rng_trace_note(&String::from_utf8_lossy(ch.get_name()));

        // Examine call for special procedure.
        if g.ch(chid).mob_flagged(flags::MOB_SPEC) && !g.no_specials {
            let rnum = g.ch(chid).mob_rnum;
            match g.mob_specs.get(rnum as usize).copied().flatten() {
                None => {
                    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                    let vnum = g.world.mob_protos[rnum as usize].vnum;
                    g.log(format!(
                        "SYSERR: {} (#{}): Attempting to call non-existing mob function.",
                        name, vnum
                    ));
                    g.ch_mut(chid).act.remove(flags::MOB_SPEC);
                }
                Some(spec) => {
                    if crate::spec::call_mob_spec(g, spec, chid, chid, 0, b"") {
                        continue; // go to next char
                    }
                }
            }
            if g.try_ch(chid).is_none() {
                continue;
            }
        }

        // If the mob has no specproc, do the default actions.
        if g.ch(chid).fighting.is_some() || !g.ch(chid).awake() {
            continue;
        }

        // hunt a victim, if applicable.
        crate::graph::hunt_victim(g, chid);
        if g.try_ch(chid).is_none() {
            continue;
        }

        // Scavenger (picking up objects).
        if g.ch(chid).mob_flagged(flags::MOB_SCAVENGER) {
            let room = g.ch(chid).in_room;
            if room != NOWHERE
                && !g.rooms[room as usize].contents.is_empty()
                && g.rng.rand_number(0, 10) == 0
            {
                let mut max = 1;
                let mut best: Option<mud_data::ids::ObjId> = None;
                for &oid in &g.rooms[room as usize].contents {
                    if crate::act::item::can_get_obj(g, chid, oid) && g.obj(oid).cost > max {
                        best = Some(oid);
                        max = g.obj(oid).cost;
                    }
                }
                if let Some(oid) = best {
                    crate::handler::obj_from_room(g, oid);
                    crate::handler::obj_to_char(g, oid, chid);
                    act(g, b"$n gets $p.", false, Some(chid), Some(oid), None, TO_ROOM);
                }
            }
        }

        // Mob Movement. The rand draw happens iff !sentinel && standing —
        // The condition order is load-bearing: it decides the draw order.
        if !g.ch(chid).mob_flagged(flags::MOB_SENTINEL)
            && g.ch(chid).position == POS_STANDING
        {
            let door = g.rng.rand_number(0, 18) as usize;
            let dc = crate::fight::dir_count(g);
            if door < dc {
                let room = g.ch(chid).in_room;
                if let Some(to) = crate::fight::can_go(g, room, door) {
                    let to_flags = &g.world.rooms[to as usize].room_flags;
                    let nomob = to_flags[0] & (1 << flags::ROOM_NOMOB) != 0;
                    let death = to_flags[0] & (1 << flags::ROOM_DEATH) != 0;
                    let stay_zone = g.ch(chid).mob_flagged(flags::MOB_STAY_ZONE);
                    let same_zone =
                        g.world.rooms[to as usize].zone == g.world.rooms[room as usize].zone;
                    if !nomob && !death && (!stay_zone || same_zone) {
                        // If the mob is charmed, do not move the mob.
                        if g.ch(chid).master.is_none() {
                            crate::act::movement::perform_move(g, chid, door as i32, true);
                        }
                    }
                }
            }
            if g.try_ch(chid).is_none() {
                continue;
            }
        }

        // Aggressive Mobs.
        if !g.ch(chid).mob_flagged(flags::MOB_HELPER)
            && (!g.ch(chid).aff(flags::AFF_BLIND) || !g.ch(chid).aff(flags::AFF_CHARM))
        {
            let room = g.ch(chid).in_room;
            if room != NOWHERE {
                let people = g.rooms[room as usize].people.clone();
                let mut found = false;
                for vict in people {
                    if found {
                        break;
                    }
                    let Some(vc) = g.try_ch(vict) else { continue };
                    if vc.is_npc() || !can_see(g, chid, vict) || vc.prf(flags::PRF_NOHASSLE) {
                        continue;
                    }
                    if g.ch(chid).mob_flagged(flags::MOB_WIMPY) && g.ch(vict).awake() {
                        continue;
                    }
                    // IS_GOOD >= 350, IS_EVIL <= -350, IS_NEUTRAL between.
                    let valign = g.ch(vict).alignment;
                    let aggro = g.ch(chid).mob_flagged(flags::MOB_AGGRESSIVE)
                        || (g.ch(chid).mob_flagged(flags::MOB_AGGR_EVIL) && valign <= -350)
                        || (g.ch(chid).mob_flagged(flags::MOB_AGGR_NEUTRAL)
                            && (-349..=349).contains(&valign))
                        || (g.ch(chid).mob_flagged(flags::MOB_AGGR_GOOD) && valign >= 350);
                    if aggro {
                        let master = g.ch(chid).master;
                        if aggressive_mob_on_a_leash(g, chid, master, Some(vict)) {
                            continue;
                        }
                        hit(g, chid, vict, TYPE_UNDEFINED);
                        found = true;
                    }
                }
            }
            if g.try_ch(chid).is_none() {
                continue;
            }
        }

        // Mob Memory.
        if g.ch(chid).mob_flagged(flags::MOB_MEMORY) && !g.ch(chid).mob_specials.memory.is_empty()
        {
            let room = g.ch(chid).in_room;
            if room != NOWHERE {
                let people = g.rooms[room as usize].people.clone();
                let mut found = false;
                for vict in people {
                    if found {
                        break;
                    }
                    let Some(vc) = g.try_ch(vict) else { continue };
                    if vc.is_npc() || !can_see(g, chid, vict) || vc.prf(flags::PRF_NOHASSLE) {
                        continue;
                    }
                    let id = g.ch(vict).idnum;
                    if !g.ch(chid).mob_specials.memory.contains(&id) {
                        continue;
                    }
                    let master = g.ch(chid).master;
                    if aggressive_mob_on_a_leash(g, chid, master, Some(vict)) {
                        continue;
                    }
                    found = true;
                    act(
                        g,
                        b"'Hey!  You're the fiend that attacked me!!!', exclaims $n.",
                        false,
                        Some(chid),
                        None,
                        None,
                        TO_ROOM,
                    );
                    hit(g, chid, vict, TYPE_UNDEFINED);
                }
            }
            if g.try_ch(chid).is_none() {
                continue;
            }
        }

        // Charmed Mob Rebellion.
        if g.ch(chid).aff(flags::AFF_CHARM) {
            if let Some(master) = g.ch(chid).master {
                let cha = g.ch(master).aff_abils.cha as i32;
                if num_followers_charmed(g, master) > (cha - 2) / 3
                    && !aggressive_mob_on_a_leash(g, chid, Some(master), Some(master))
                {
                    if can_see(g, chid, master) && !g.ch(master).prf(flags::PRF_NOHASSLE) {
                        hit(g, chid, master, TYPE_UNDEFINED);
                    }
                    if g.try_ch(chid).is_some() {
                        crate::act::movement::stop_follower(g, chid);
                    }
                }
            }
            if g.try_ch(chid).is_none() {
                continue;
            }
        }

        // Helper Mobs.
        if g.ch(chid).mob_flagged(flags::MOB_HELPER)
            && (!g.ch(chid).aff(flags::AFF_BLIND) || !g.ch(chid).aff(flags::AFF_CHARM))
        {
            let room = g.ch(chid).in_room;
            if room != NOWHERE {
                let people = g.rooms[room as usize].people.clone();
                let mut found = false;
                for vict in people {
                    if found {
                        break;
                    }
                    if vict == chid {
                        continue;
                    }
                    let Some(vc) = g.try_ch(vict) else { continue };
                    if !vc.is_npc() {
                        continue;
                    }
                    let Some(opp) = vc.fighting else { continue };
                    // never help against a group-mate's foe
                    // when the fighter shares the helper's own group.
                    if vc.group.is_some() && vc.group == g.ch(chid).group {
                        continue;
                    }
                    if g.try_ch(opp).is_none() || g.ch(opp).is_npc() || opp == chid {
                        continue;
                    }
                    act(g, b"$n jumps to the aid of $N!", false, Some(chid), None, Some(vict), TO_ROOM);
                    hit(g, chid, opp, TYPE_UNDEFINED);
                    found = true;
                }
            }
        }
    }
}

pub fn remember(g: &mut Game, chid: CharId, victim: CharId) {
    if !g.ch(chid).is_npc() || g.ch(victim).is_npc() || g.ch(victim).prf(flags::PRF_NOHASSLE) {
        return;
    }
    let id = g.ch(victim).idnum;
    let mem = &mut g.ch_mut(chid).mob_specials.memory;
    if !mem.contains(&id) {
        // Prepend the new record.
        mem.insert(0, id);
    }
}

pub fn forget(g: &mut Game, chid: CharId, victim: CharId) {
    let id = g.ch(victim).idnum;
    let mem = &mut g.ch_mut(chid).mob_specials.memory;
    if let Some(pos) = mem.iter().position(|&m| m == id) {
        mem.remove(pos);
    }
}

pub fn clear_memory(g: &mut Game, chid: CharId) {
    g.ch_mut(chid).mob_specials.memory.clear();
}

/// num_followers_charmed.
pub fn num_followers_charmed(g: &Game, chid: CharId) -> i32 {
    let mut total = 0;
    for &f in &g.ch(chid).followers {
        let Some(fc) = g.try_ch(f) else { continue };
        if fc.aff(flags::AFF_CHARM) && fc.master == Some(chid) {
            total += 1;
        }
    }
    total
}

/// aggressive_mob_on_a_leash. true = attack suppressed.
fn aggressive_mob_on_a_leash(
    g: &mut Game,
    slave: CharId,
    master: Option<CharId>,
    attack: Option<CharId>,
) -> bool {
    let Some(master) = master else { return false };
    if !g.ch(slave).aff(flags::AFF_CHARM) {
        return false;
    }

    let dieroll = g.rng.rand_number(1, 20);
    let cha = g.ch(master).aff_abils.cha as i32;
    let int = g.ch(slave).aff_abils.intel as i32;
    if dieroll != 1 && (dieroll == 20 || dieroll > 10 - cha + int) {
        if let Some(attack) = attack {
            if let Some(snarl_cmd) = crate::interpreter::find_command(g, b"snarl") {
                if g.rng.rand_number(0, 3) == 0 {
                    let victbuf = g.ch(attack).get_name().to_vec();
                    crate::act::social::do_action(g, slave, &victbuf, snarl_cmd, 0);
                }
            }
        }
        return true;
    }
    false
}
