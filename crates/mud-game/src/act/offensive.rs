//! assist, hit, kill, backstab, order, flee, bash, rescue,
//! whirlwind (with its mud event), kick, and bandage.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::spells::*;
use mud_data::types::*;

use crate::comm::{act, send_to_char, TO_CHAR, TO_NOTVICT, TO_ROOM, TO_VICT};
use crate::fight::{damage, hit, pk_allowed, raw_kill, stop_fighting};
use crate::game::{EventKind, Game};
use crate::handler::{get_char_room_vis, is_abbrev};
use crate::interpreter::{half_chop, one_argument};

pub fn do_assist(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).fighting.is_some() {
        send_to_char(g, chid, b"You're already fighting!  How can you assist someone else?\r\n");
        return;
    }
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to assist?\r\n");
        return;
    }
    let Some(helpee) = get_char_room_vis(g, chid, &arg, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if helpee == chid {
        send_to_char(g, chid, b"You can't help yourself any more than this!\r\n");
        return;
    }

    // Hit the same enemy the person you're helping is.
    let opponent = match g.ch(helpee).fighting {
        Some(o) => Some(o),
        None => {
            let room = g.ch(chid).in_room;
            g.rooms[room as usize]
                .people
                .iter()
                .copied()
                .find(|&o| g.try_ch(o).is_some_and(|oc| oc.fighting == Some(helpee)))
        }
    };

    let Some(opponent) = opponent else {
        act(g, b"But nobody is fighting $M!", false, Some(chid), None, Some(helpee), TO_CHAR);
        return;
    };
    if !crate::handler::can_see(g, chid, opponent) {
        act(g, b"You can't see who is fighting $M!", false, Some(chid), None, Some(helpee), TO_CHAR);
        return;
    }
    // Prevent accidental pkill.
    if !pk_allowed(g, chid, opponent) {
        send_to_char(g, chid, b"You cannot kill other players.\r\n");
        return;
    }
    send_to_char(g, chid, b"You join the fight!\r\n");
    act(g, b"$N assists you!", false, Some(helpee), None, Some(chid), TO_CHAR);
    act(g, b"$n assists $N.", false, Some(chid), None, Some(helpee), TO_NOTVICT);
    hit(g, chid, opponent, TYPE_UNDEFINED);
}

pub fn do_hit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Hit who?\r\n");
        return;
    }
    let Some(vict) = get_char_room_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"That player is not here.\r\n");
        return;
    };
    if vict == chid {
        send_to_char(g, chid, b"You hit yourself...OUCH!.\r\n");
        act(g, b"$n hits $mself, and says OUCH!", false, Some(chid), None, Some(vict), TO_ROOM);
        return;
    }
    if g.ch(chid).aff(flags::AFF_CHARM) && g.ch(chid).master == Some(vict) {
        act(g, b"$N is just such a good friend, you simply can't hit $M.", false, Some(chid), None, Some(vict), TO_CHAR);
        return;
    }
    if !pk_allowed(g, chid, vict) {
        send_to_char(g, chid, b"Player killing is not allowed.\r\n");
        return;
    }

    if g.ch(chid).position == POS_STANDING && g.ch(chid).fighting != Some(vict) {
        // Initiative: higher DEX swings first; ties flip a coin.
        let cd = g.ch(chid).aff_abils.dex;
        let vd = g.ch(vict).aff_abils.dex;
        if cd > vd || (cd == vd && g.rng.rand_number(1, 2) == 1) {
            hit(g, chid, vict, TYPE_UNDEFINED);
        } else {
            hit(g, vict, chid, TYPE_UNDEFINED);
        }
        // (An indentation quirk: the wait applies on both branches.)
        if g.try_ch(chid).is_some() {
            g.ch_mut(chid).wait = PULSE_VIOLENCE as i32 + 2;
        }
    } else {
        send_to_char(g, chid, b"You're fighting the best you can!\r\n");
    }
}

pub fn do_kill(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, subcmd: i32) {
    {
        let ch = g.ch(chid);
        if ch.level < LVL_GRGOD || ch.is_npc() || !ch.prf(flags::PRF_NOHASSLE) {
            do_hit(g, chid, argument, cmd, subcmd);
            return;
        }
    }
    let (arg, _) = one_argument(argument);

    if arg.is_empty() {
        send_to_char(g, chid, b"Kill who?\r\n");
        return;
    }
    let Some(vict) = get_char_room_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"That player is not here.\r\n");
        return;
    };
    if chid == vict {
        send_to_char(g, chid, b"Your mother would be so sad.. :(\r\n");
        return;
    }
    act(g, b"You chop $M to pieces!  Ah!  The blood!", false, Some(chid), None, Some(vict), TO_CHAR);
    act(g, b"$N chops you to pieces!", false, Some(vict), None, Some(chid), TO_CHAR);
    act(g, b"$n brutally slays $N!", false, Some(chid), None, Some(vict), TO_NOTVICT);
    raw_kill(g, vict, Some(chid));
}

pub fn do_backstab(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_BACKSTAB) == 0 {
        send_to_char(g, chid, b"You have no idea how to do that.\r\n");
        return;
    }
    let (buf, _) = one_argument(argument);

    let Some(vict) = get_char_room_vis(g, chid, &buf, None) else {
        send_to_char(g, chid, b"Backstab who?\r\n");
        return;
    };
    if vict == chid {
        send_to_char(g, chid, b"How can you sneak up on yourself?\r\n");
        return;
    }
    let Some(weapon) = g.ch(chid).equipment[WEAR_WIELD] else {
        send_to_char(g, chid, b"You need to wield a weapon to make it a success.\r\n");
        return;
    };
    if g.obj(weapon).values[3] != TYPE_PIERCE - TYPE_HIT {
        send_to_char(g, chid, b"Only piercing weapons can be used for backstabbing.\r\n");
        return;
    }
    if g.ch(vict).fighting.is_some() {
        send_to_char(g, chid, b"You can't backstab a fighting person -- they're too alert!\r\n");
        return;
    }

    if g.ch(vict).mob_flagged(flags::MOB_AWARE) && g.ch(vict).awake() {
        act(g, b"You notice $N lunging at you!", false, Some(vict), None, Some(chid), TO_CHAR);
        act(g, b"$e notices you lunging at $m!", false, Some(vict), None, Some(chid), TO_VICT);
        act(g, b"$n notices $N lunging at $m!", false, Some(vict), None, Some(chid), TO_NOTVICT);
        hit(g, vict, chid, TYPE_UNDEFINED);
        return;
    }

    let percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let prob = g.ch(chid).get_skill(SKILL_BACKSTAB);

    if g.ch(vict).awake() && percent > prob {
        damage(g, chid, vict, 0, SKILL_BACKSTAB);
    } else {
        hit(g, chid, vict, SKILL_BACKSTAB);
    }
    if g.try_ch(chid).is_some() {
        g.ch_mut(chid).wait = 2 * PULSE_VIOLENCE as i32;
    }
}

pub fn do_order(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (name, message) = half_chop(argument);

    if name.is_empty() || message.is_empty() {
        send_to_char(g, chid, b"Order who to do what?\r\n");
        return;
    }
    let vict = get_char_room_vis(g, chid, &name, None);
    if vict.is_none() && !is_abbrev(&name, b"followers") {
        send_to_char(g, chid, b"That person isn't here.\r\n");
        return;
    }
    if vict == Some(chid) {
        send_to_char(g, chid, b"You obviously suffer from skitzofrenia.\r\n");
        return;
    }
    if g.ch(chid).aff(flags::AFF_CHARM) {
        send_to_char(g, chid, b"Your superior would not aprove of you giving orders.\r\n");
        return;
    }
    if let Some(vict) = vict {
        let mut buf = b"$N orders you to '".to_vec();
        buf.extend_from_slice(&message);
        buf.push(b'\'');
        act(g, &buf, false, Some(vict), None, Some(chid), TO_CHAR);
        act(g, b"$n gives $N an order.", false, Some(chid), None, Some(vict), TO_ROOM);

        if g.ch(vict).master != Some(chid) || !g.ch(vict).aff(flags::AFF_CHARM) {
            act(g, b"$n has an indifferent look.", false, Some(vict), None, None, TO_ROOM);
        } else {
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
            crate::interpreter::command_interpreter(g, vict, &message);
        }
    } else {
        // This is order "followers".
        let mut buf = b"$n issues the order '".to_vec();
        buf.extend_from_slice(&message);
        buf.extend_from_slice(b"'.");
        act(g, &buf, false, Some(chid), None, None, TO_ROOM);

        let room = g.ch(chid).in_room;
        let followers = g.ch(chid).followers.clone();
        let mut found = false;
        for f in followers {
            let Some(fc) = g.try_ch(f) else { continue };
            if fc.in_room == room && fc.aff(flags::AFF_CHARM) {
                found = true;
                crate::interpreter::command_interpreter(g, f, &message);
            }
        }
        if found {
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
        } else {
            send_to_char(g, chid, b"Nobody here is a loyal subject of yours!\r\n");
        }
    }
}

pub fn do_flee(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).position < POS_FIGHTING {
        send_to_char(g, chid, b"You are in pretty bad shape, unable to flee!\r\n");
        return;
    }

    for _ in 0..6 {
        let attempt = g.rng.rand_number(0, crate::fight::dir_count(g) as i32 - 1) as usize;
        let room = g.ch(chid).in_room;
        let viable = crate::fight::can_go(g, room, attempt).is_some_and(|to| {
            g.world.rooms[to as usize].room_flags[0] & (1 << flags::ROOM_DEATH) == 0
        });
        if !viable {
            continue;
        }
        act(g, b"$n panics, and attempts to flee!", true, Some(chid), None, None, TO_ROOM);
        let was_fighting = g.ch(chid).fighting;
        if crate::act::movement::do_simple_move(g, chid, attempt, true) {
            send_to_char(g, chid, b"You flee head over heels.\r\n");
            if let Some(opp) = was_fighting {
                if !g.ch(chid).is_npc() && g.try_ch(opp).is_some() {
                    let loss = (g.ch(opp).points.max_hit - g.ch(opp).points.hit)
                        * g.ch(opp).level as i32;
                    crate::limits::gain_exp(g, chid, -loss);
                }
            }
            if g.ch(chid).fighting.is_some() {
                stop_fighting(g, chid);
            }
            if let Some(opp) = was_fighting {
                if g.try_ch(opp).is_some() && g.ch(opp).fighting == Some(chid) {
                    stop_fighting(g, opp);
                }
            }
        } else {
            act(g, b"$n tries to flee, but can't!", true, Some(chid), None, None, TO_ROOM);
        }
        return;
    }
    send_to_char(g, chid, b"PANIC!  You couldn't escape!\r\n");
}

pub fn do_bash(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_BASH) == 0 {
        send_to_char(g, chid, b"You have no idea how.\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    if g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_PEACEFUL) != 0 {
        send_to_char(g, chid, b"This room just has such a peaceful, easy feeling...\r\n");
        return;
    }
    if g.ch(chid).equipment[WEAR_WIELD].is_none() {
        send_to_char(g, chid, b"You need to wield a weapon to make it a success.\r\n");
        return;
    }
    let vict = match get_char_room_vis(g, chid, &arg, None) {
        Some(v) => v,
        None => {
            let fighting = g.ch(chid).fighting;
            match fighting.filter(|&f| {
                g.try_ch(f).is_some_and(|fc| fc.in_room == g.ch(chid).in_room)
            }) {
                Some(f) => f,
                None => {
                    send_to_char(g, chid, b"Bash who?\r\n");
                    return;
                }
            }
        }
    };
    if vict == chid {
        send_to_char(g, chid, b"Aren't we funny today...\r\n");
        return;
    }
    if g.ch(vict).mob_flagged(flags::MOB_NOKILL) {
        send_to_char(g, chid, b"This mob is protected.\r\n");
        return;
    }

    let mut percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let prob = g.ch(chid).get_skill(SKILL_BASH);

    if g.ch(vict).mob_flagged(flags::MOB_NOBASH) {
        percent = 101;
    }

    if percent > prob {
        damage(g, chid, vict, 0, SKILL_BASH);
        if g.try_ch(chid).is_some() {
            g.ch_mut(chid).position = POS_SITTING;
        }
    } else {
        // Only set them sitting if they didn't flee. -gg 9/21/98
        if damage(g, chid, vict, 1, SKILL_BASH) > 0 {
            if g.try_ch(vict).is_some() {
                g.ch_mut(vict).wait = PULSE_VIOLENCE as i32;
                if g.ch(chid).in_room == g.ch(vict).in_room {
                    g.ch_mut(vict).position = POS_SITTING;
                }
            }
        }
    }
    if g.try_ch(chid).is_some() {
        g.ch_mut(chid).wait = PULSE_VIOLENCE as i32 * 2;
    }
}

pub fn do_rescue(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_RESCUE) == 0 {
        send_to_char(g, chid, b"You have no idea how to do that.\r\n");
        return;
    }

    let (arg, _) = one_argument(argument);

    let Some(vict) = get_char_room_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"Whom do you want to rescue?\r\n");
        return;
    };
    if vict == chid {
        send_to_char(g, chid, b"What about fleeing instead?\r\n");
        return;
    }
    if g.ch(chid).fighting == Some(vict) {
        send_to_char(g, chid, b"How can you rescue someone you are trying to kill?\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    let mut tmp_ch = g.rooms[room as usize]
        .people
        .iter()
        .copied()
        .find(|&t| g.try_ch(t).is_some_and(|tc| tc.fighting == Some(vict)));

    if let Some(vf) = g.ch(vict).fighting {
        if g.ch(chid).fighting == Some(vf) && tmp_ch.is_none() {
            tmp_ch = Some(vf);
            if g.ch(vf).fighting == Some(chid) {
                let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
                let fname = String::from_utf8_lossy(g.ch(vf).get_name()).into_owned();
                send_to_char(
                    g,
                    chid,
                    format!("You have already rescued {} from {}.\r\n", vname, fname).as_bytes(),
                );
                return;
            }
        }
    }

    let Some(tmp_ch) = tmp_ch else {
        act(g, b"But nobody is fighting $M!", false, Some(chid), None, Some(vict), TO_CHAR);
        return;
    };
    let percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let prob = g.ch(chid).get_skill(SKILL_RESCUE);

    if percent > prob {
        send_to_char(g, chid, b"You fail the rescue!\r\n");
        return;
    }
    send_to_char(g, chid, b"Banzai!  To the rescue...\r\n");
    act(g, b"You are rescued by $N, you are confused!", false, Some(vict), None, Some(chid), TO_CHAR);
    act(g, b"$n heroically rescues $N!", false, Some(chid), None, Some(vict), TO_NOTVICT);

    if g.ch(vict).fighting == Some(tmp_ch) {
        stop_fighting(g, vict);
    }
    if g.ch(tmp_ch).fighting.is_some() {
        stop_fighting(g, tmp_ch);
    }
    if g.ch(chid).fighting.is_some() {
        stop_fighting(g, chid);
    }

    crate::fight::set_fighting(g, chid, tmp_ch);
    crate::fight::set_fighting(g, tmp_ch, chid);

    g.ch_mut(vict).wait = 2 * PULSE_VIOLENCE as i32;
}

/// event_whirlwind. Returns the re-fire delay in
/// pulses, or None to end the event.
pub fn event_whirlwind(g: &mut Game, chid: CharId) -> Option<u64> {
    if g.try_ch(chid).is_none() {
        return None;
    }
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return None;
    }
    let npcs: Vec<CharId> = g.rooms[room as usize]
        .people
        .iter()
        .copied()
        .filter(|&t| g.try_ch(t).is_some_and(|tc| tc.is_npc()))
        .collect();

    if npcs.is_empty() {
        send_to_char(g, chid, b"There is no one in the room to whirlwind!\r\n");
        return None;
    }

    send_to_char(g, chid, b"\t[f313]You deliver a vicious \t[f014]\t[b451]WHIRLWIND!!!\tn\r\n");

    let count = g.rng.dice(1, 4);
    for _ in 0..count {
        // random_from_list: one draw, 1-based index.
        let nr = g.rng.rand_number(1, npcs.len() as i32) as usize;
        let tch = npcs[nr - 1];
        if g.try_ch(tch).is_some() && g.try_ch(chid).is_some() {
            hit(g, chid, tch, TYPE_UNDEFINED);
        }
    }
    if g.try_ch(chid).is_none() {
        return None;
    }

    if g.ch(chid).get_skill(SKILL_WHIRLWIND) < g.rng.rand_number(1, 101) {
        send_to_char(g, chid, b"You stop spinning.\r\n");
        None
    } else {
        Some(3 * PASSES_PER_SEC / 2)
    }
}

pub fn do_whirlwind(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_WHIRLWIND) == 0 {
        send_to_char(g, chid, b"You have no idea how.\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    if g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_PEACEFUL) != 0 {
        send_to_char(g, chid, b"This room just has such a peaceful, easy feeling...\r\n");
        return;
    }
    if g.ch(chid).position < POS_FIGHTING {
        send_to_char(g, chid, b"You must be on your feet to perform a whirlwind.\r\n");
        return;
    }
    let already = g
        .events
        .iter()
        .any(|e| matches!(e.kind, EventKind::Whirlwind { ch } if ch == chid));
    if already {
        send_to_char(g, chid, b"You are already attempting that!\r\n");
        return;
    }

    send_to_char(g, chid, b"You begin to spin rapidly in circles.\r\n");
    act(g, b"$n begins to rapidly spin in a circle!", false, Some(chid), None, None, TO_ROOM);

    g.queue_event(3 * PASSES_PER_SEC, EventKind::Whirlwind { ch: chid });
    g.ch_mut(chid).wait = PULSE_VIOLENCE as i32 * 3;
}

pub fn do_kick(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_KICK) == 0 {
        send_to_char(g, chid, b"You have no idea how.\r\n");
        return;
    }

    let (arg, _) = one_argument(argument);

    let vict = match get_char_room_vis(g, chid, &arg, None) {
        Some(v) => v,
        None => {
            let fighting = g.ch(chid).fighting;
            match fighting.filter(|&f| {
                g.try_ch(f).is_some_and(|fc| fc.in_room == g.ch(chid).in_room)
            }) {
                Some(f) => f,
                None => {
                    send_to_char(g, chid, b"Kick who?\r\n");
                    return;
                }
            }
        }
    };
    if vict == chid {
        send_to_char(g, chid, b"Aren't we funny today...\r\n");
        return;
    }
    // 101% is a complete failure.
    let ac = crate::act::informative::compute_armor_class(g, vict) / 10;
    let percent = (10 - ac) * 2 + g.rng.rand_number(1, 101);
    let prob = g.ch(chid).get_skill(SKILL_KICK);

    if percent > prob {
        damage(g, chid, vict, 0, SKILL_KICK);
    } else {
        let dam = g.ch(chid).level as i32 / 2;
        damage(g, chid, vict, dam, SKILL_KICK);
    }
    if g.try_ch(chid).is_some() {
        g.ch_mut(chid).wait = PULSE_VIOLENCE as i32 * 3;
    }
}

/// do_bandage: bind a wounded character's wounds, restoring hit points on
/// a successful skill check.
pub fn do_bandage(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).get_skill(SKILL_BANDAGE) == 0 {
        send_to_char(g, chid, b"You are unskilled in the art of bandaging.\r\n");
        return;
    }
    if g.ch(chid).position != POS_STANDING {
        send_to_char(g, chid, b"You are not in a proper position for that!\r\n");
        return;
    }

    let (arg, _) = one_argument(argument);

    let Some(vict) = get_char_room_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"Who do you want to bandage?\r\n");
        return;
    };
    if g.ch(vict).points.hit >= 0 {
        send_to_char(g, chid, b"You can only bandage someone who is close to death.\r\n");
        return;
    }

    g.ch_mut(chid).wait = PULSE_VIOLENCE as i32 * 2;

    let percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let prob = g.ch(chid).get_skill(SKILL_BANDAGE);

    // Succeed at or under the skill, as every skill check does.
    if percent > prob {
        act(g, b"Your attempt to bandage fails.", false, Some(chid), None, None, TO_CHAR);
        act(g, b"$n tries to bandage $N, but fails miserably.", true, Some(chid), None, Some(vict), TO_NOTVICT);
        damage(g, vict, vict, 2, TYPE_SUFFERING);
        return;
    }

    act(g, b"You successfully bandage $N.", false, Some(chid), None, Some(vict), TO_CHAR);
    act(g, b"$n bandages $N, who looks a bit better now.", true, Some(chid), None, Some(vict), TO_NOTVICT);
    act(g, b"Someone bandages you, and you feel a bit better now.", false, Some(chid), None, Some(vict), TO_VICT);
    g.ch_mut(vict).points.hit = 0;
}
