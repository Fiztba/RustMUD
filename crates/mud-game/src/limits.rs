//! Regen formulas, experience gain/leveling, conditions,
//! idle handling, the full per-tick point_update, and gold/bank clamps.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::spells::{SPELL_POISON, TYPE_SUFFERING};
use mud_data::types::*;

use crate::ch::DRUNK;
use crate::comm::{act, cc, send_to_char, C_SPR, KBRED, KNRM, TO_CHAR, TO_ROOM};
use crate::game::{Game, MudlogKind};

pub use crate::ch::{HUNGER, THIRST};

/// graf: age-curve interpolation.
fn graf(grafage: i32, p0: i32, p1: i32, p2: i32, p3: i32, p4: i32, p5: i32, p6: i32) -> i32 {
    if grafage < 15 {
        p0
    } else if grafage <= 29 {
        p1 + (((grafage - 15) * (p2 - p1)) / 15)
    } else if grafage <= 44 {
        p2 + (((grafage - 30) * (p3 - p2)) / 15)
    } else if grafage <= 59 {
        p3 + (((grafage - 45) * (p4 - p3)) / 15)
    } else if grafage <= 79 {
        p4 + (((grafage - 60) * (p5 - p4)) / 20)
    } else {
        p6
    }
}

fn char_age_years(g: &Game, chid: CharId) -> i32 {
    crate::gametime::age(g.ch(chid).time.birth, g.now).year
}

fn is_caster_class(g: &Game, chid: CharId) -> bool {
    // IS_MAGIC_USER/IS_CLERIC require !IS_NPC.
    let ch = g.ch(chid);
    !ch.is_npc() && (ch.class == CLASS_MAGIC_USER || ch.class == CLASS_CLERIC)
}

/// The practice sessions a character banks on reaching a new level.
///
/// Wisdom sets the figure and the class bounds it: a caster is never given
/// fewer than two, and everyone else gets one or two however wise they are.
/// Wisdom past 17 therefore buys a warrior nothing.
pub fn practices_per_level(class: i32, wis: i32) -> i32 {
    let bonus = mud_data::tables::WIS_APP[wis.clamp(0, 25) as usize];
    if class == CLASS_MAGIC_USER as i32 || class == CLASS_CLERIC as i32 {
        bonus.max(2)
    } else {
        bonus.max(1).min(2)
    }
}

/// How far one practice session moves a skill, in percentage points.
///
/// Intelligence sets the figure and the class bounds it: a warrior gains
/// twelve however clever he is, and a caster twenty-five however dull.
pub fn practice_gain_percent(class: i32, intel: i32) -> i32 {
    let learn = mud_data::tables::INT_APP[intel.clamp(0, 25) as usize];
    let class = class.clamp(0, 3) as usize;
    let maxgain = mud_data::tables::PRAC_PARAMS[1][class];
    let mingain = mud_data::tables::PRAC_PARAMS[2][class];
    maxgain.min(mingain.max(learn))
}

fn starving_or_parched(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    !ch.is_npc()
        && (ch.ps().conditions[HUNGER] == 0 || ch.ps().conditions[THIRST] == 0)
}

pub fn mana_gain(g: &Game, chid: CharId) -> i32 {
    let ch = g.ch(chid);
    let mut gain;
    if ch.is_npc() {
        gain = ch.level as i32;
    } else {
        gain = graf(char_age_years(g, chid), 4, 8, 12, 16, 12, 10, 8);
        match ch.position {
            POS_SLEEPING => gain *= 2,
            POS_RESTING => gain += gain / 2,
            POS_SITTING => gain += gain / 4,
            _ => {}
        }
        if is_caster_class(g, chid) {
            gain *= 2;
        }
        if starving_or_parched(g, chid) {
            gain /= 4;
        }
    }
    if ch.aff(flags::AFF_POISON) {
        gain /= 4;
    }
    gain
}

pub fn hit_gain(g: &Game, chid: CharId) -> i32 {
    let ch = g.ch(chid);
    let mut gain;
    if ch.is_npc() {
        gain = ch.level as i32;
    } else {
        gain = graf(char_age_years(g, chid), 8, 12, 20, 32, 16, 10, 4);
        match ch.position {
            POS_SLEEPING => gain += gain / 2,
            POS_RESTING => gain += gain / 4,
            POS_SITTING => gain += gain / 8,
            _ => {}
        }
        if is_caster_class(g, chid) {
            gain /= 2;
        }
        if starving_or_parched(g, chid) {
            gain /= 4;
        }
    }
    if ch.aff(flags::AFF_POISON) {
        gain /= 4;
    }
    gain
}

pub fn move_gain(g: &Game, chid: CharId) -> i32 {
    let ch = g.ch(chid);
    let mut gain;
    if ch.is_npc() {
        gain = ch.level as i32;
    } else {
        gain = graf(char_age_years(g, chid), 16, 20, 24, 20, 16, 12, 10);
        match ch.position {
            POS_SLEEPING => gain += gain / 2,
            POS_RESTING => gain += gain / 4,
            POS_SITTING => gain += gain / 8,
            _ => {}
        }
        if starving_or_parched(g, chid) {
            gain /= 4;
        }
    }
    if ch.aff(flags::AFF_POISON) {
        gain /= 4;
    }
    gain
}

/// run_autowiz. Rebuilds lib/text/wizlist and lib/text/immlist from the
/// player index, then loads them back into the cache that `wizlist` and
/// `immlist` read from.
///
/// Generate first, load second. The other way round leaves the cache holding
/// the file as it stood *before* this call rewrote it, so nothing shows until
/// some later autowiz happens to load it -- and that catches every immortal on
/// the list, not just whoever triggered the run. Two wizupdates to see one
/// change reads as the list being broken.
pub fn run_autowiz(g: &mut Game) {
    if !g.config.use_autowiz {
        return;
    }
    g.mudlog(crate::game::MudlogKind::Cmp, LVL_IMMORT, false, "Initiating autowiz.");
    crate::text::write_wizlists(g);
    crate::text::reboot_wizlists(g);
}

pub fn gain_exp(g: &mut Game, chid: CharId, mut gain: i32) {
    let (is_npc, level) = {
        let ch = g.ch(chid);
        (ch.is_npc(), ch.level as i32)
    };
    if !is_npc && (!(1..LVL_IMMORT as i32).contains(&level)) {
        return;
    }
    if is_npc {
        let ch = g.ch_mut(chid);
        ch.points.exp += gain;
        return;
    }
    if gain > 0 {
        if crate::act::other::is_happyhour(g) && g.happy.exp_rate > 0 {
            gain += (gain as f32 * (g.happy.exp_rate as f32 / 100.0)) as i32;
        }
        gain = gain.min(g.config.max_exp_gain);
        g.ch_mut(chid).points.exp += gain;
        let mut num_levels = 0;
        let cap = LVL_IMMORT as i32 - if g.config.no_mort_to_immort { 1 } else { 0 };
        loop {
            let (lvl, class, exp) = {
                let ch = g.ch(chid);
                (ch.level as i32, ch.class as i32, ch.points.exp)
            };
            if lvl < cap && exp >= mud_data::tables::level_exp(class, lvl + 1) {
                g.ch_mut(chid).level += 1;
                num_levels += 1;
                crate::login::advance_level(g, chid);
            } else {
                break;
            }
        }
        if num_levels > 0 {
            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let lvl_now = g.ch(chid).level;
            let loglvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
            g.mudlog(
                MudlogKind::Brf,
                loglvl,
                true,
                &format!(
                    "{} advanced {} level{} to level {}.",
                    name,
                    num_levels,
                    if num_levels == 1 { "" } else { "s" },
                    lvl_now
                ),
            );
            if num_levels == 1 {
                send_to_char(g, chid, b"You rise a level!\r\n");
            } else {
                send_to_char(g, chid, format!("You rise {} levels!\r\n", num_levels).as_bytes());
            }
            crate::login::set_title(g, chid, None);
            if g.ch(chid).level >= LVL_IMMORT && !g.ch(chid).plr(flags::PLR_NOWIZLIST) {
                run_autowiz(g);
            }
        }
    } else if gain < 0 {
        gain = gain.max(-g.config.max_exp_loss);
        let ch = g.ch_mut(chid);
        ch.points.exp += gain;
        if ch.points.exp < 0 {
            ch.points.exp = 0;
        }
    }
    if g.ch(chid).level >= LVL_IMMORT && !g.ch(chid).plr(flags::PLR_NOWIZLIST) {
        run_autowiz(g);
    }
}


/// gain_exp_regardless: no cap, no level gate, and it
/// keeps advancing all the way to LVL_IMPL.
pub fn gain_exp_regardless(g: &mut Game, chid: CharId, mut gain: i32) {
    if crate::act::other::is_happyhour(g) && g.happy.exp_rate > 0 {
        gain += (gain as f32 * (g.happy.exp_rate as f32 / 100.0)) as i32;
    }
    g.ch_mut(chid).points.exp += gain;
    if g.ch(chid).points.exp < 0 {
        g.ch_mut(chid).points.exp = 0;
    }

    if !g.ch(chid).is_npc() {
        let mut num_levels = 0;
        loop {
            let (lvl, class, exp) = {
                let ch = g.ch(chid);
                (ch.level as i32, ch.class as i32, ch.points.exp)
            };
            if lvl < LVL_IMPL as i32 && exp >= mud_data::tables::level_exp(class, lvl + 1) {
                g.ch_mut(chid).level += 1;
                num_levels += 1;
                crate::login::advance_level(g, chid);
            } else {
                break;
            }
        }
        if num_levels > 0 {
            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let lvl_now = g.ch(chid).level;
            let loglvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
            g.mudlog(
                MudlogKind::Brf,
                loglvl,
                true,
                &format!(
                    "{} advanced {} level{} to level {}.",
                    name,
                    num_levels,
                    if num_levels == 1 { "" } else { "s" },
                    lvl_now
                ),
            );
            if num_levels == 1 {
                send_to_char(g, chid, b"You rise a level!\r\n");
            } else {
                send_to_char(g, chid, format!("You rise {} levels!\r\n", num_levels).as_bytes());
            }
            crate::login::set_title(g, chid, None);
        }
    }
    if g.ch(chid).level >= LVL_IMMORT && !g.ch(chid).plr(flags::PLR_NOWIZLIST) {
        run_autowiz(g);
    }
}

fn check_idling(g: &mut Game, chid: CharId) {
    if g.ch(chid).timer <= g.config.idle_void {
        return;
    }
    if g.ch(chid).was_in_room == NOWHERE && g.ch(chid).in_room != NOWHERE {
        let here = g.ch(chid).in_room;
        g.ch_mut(chid).was_in_room = here;
        if let Some(opp) = g.ch(chid).fighting {
            crate::fight::stop_fighting(g, opp);
            crate::fight::stop_fighting(g, chid);
        }
        act(g, b"$n disappears into the void.", true, Some(chid), None, None, TO_ROOM);
        send_to_char(g, chid, b"You have been idle, and are pulled into a void.\r\n");
        crate::players_glue::save_char(g, chid);
        crate::objsave::crash_crashsave(g, chid);
        crate::handler::char_from_room(g, chid);
        // Hardcoded rnum 1 -- the void.
        crate::handler::char_to_room(g, chid, 1);
    } else if g.ch(chid).timer > g.config.idle_rent_time {
        if g.ch(chid).in_room != NOWHERE {
            crate::handler::char_from_room(g, chid);
        }
        crate::handler::char_to_room(g, chid, 3);
        if let Some(di) = g.ch(chid).desc {
            if let Some(d) = g.descriptors.get_mut(di) {
                d.state = mud_data::types::ConState::Disconnect;
                d.character = None;
            }
            g.ch_mut(chid).desc = None;
        }
        if g.config.free_rent {
            crate::objsave::crash_rentsave(g, chid, 0);
        } else {
            crate::objsave::crash_idlesave(g, chid);
        }
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let lvl = (LVL_GOD as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(MudlogKind::Cmp, lvl, true, &format!("{} force-rented and extracted (idle).", name));
        crate::llog::add_llog_entry(g, chid, crate::llog::LAST_IDLEOUT);
        // add_llog_entry(LAST_IDLEOUT): stage 8.
        crate::handler::extract_char(g, chid);
    }
}

/// point_update — every mud tick (75s).
pub fn point_update(g: &mut Game) {
    // Character phase.
    let chars = g.character_list.clone();
    for chid in chars {
        if g.try_ch(chid).is_none() {
            continue;
        }
        gain_condition(g, chid, HUNGER, -1);
        gain_condition(g, chid, DRUNK, -1);
        gain_condition(g, chid, THIRST, -1);

        let pos = g.ch(chid).position;
        if pos >= POS_STUNNED {
            let hg = hit_gain(g, chid);
            let mg = mana_gain(g, chid);
            let vg = move_gain(g, chid);
            {
                let ch = g.ch_mut(chid);
                ch.points.hit = (ch.points.hit + hg).min(ch.points.max_hit);
                ch.points.mana = (ch.points.mana + mg).min(ch.points.max_mana);
                ch.points.mov = (ch.points.mov + vg).min(ch.points.max_move);
            }
            if g.ch(chid).aff(flags::AFF_POISON)
                && crate::fight::damage(g, chid, chid, 2, SPELL_POISON) == -1
            {
                continue; // Oops, they died. -gg 6/24/98
            }
            if g.try_ch(chid).is_some() && g.ch(chid).position <= POS_STUNNED {
                crate::fight::update_pos(g, chid);
            }
        } else if pos == POS_INCAP {
            if crate::fight::damage(g, chid, chid, 1, TYPE_SUFFERING) == -1 {
                continue;
            }
        } else if pos == POS_MORTALLYW && crate::fight::damage(g, chid, chid, 2, TYPE_SUFFERING) == -1 {
            continue;
        }
        if g.try_ch(chid).is_none() {
            continue;
        }
        if !g.ch(chid).is_npc() {
            update_char_objects(g, chid);
            g.ch_mut(chid).timer += 1;
            if (g.ch(chid).level as i32) < g.config.idle_max_level {
                check_idling(g, chid);
            }
        }
    }

    // Object phase.
    let objs = g.object_list.clone();
    for oid in objs {
        if !g.try_obj_alive(oid) {
            continue;
        }
        if crate::handler::is_corpse(g, oid) {
            if g.obj(oid).timer > 0 {
                g.obj_mut(oid).timer -= 1;
            }
            if g.obj(oid).timer == 0 {
                if let Some(carrier) = g.obj(oid).carried_by {
                    act(g, b"$p decays in your hands.", false, Some(carrier), Some(oid), None, TO_CHAR);
                } else {
                    let room = g.obj(oid).in_room;
                    if room != NOWHERE {
                        if let Some(&first) = g.rooms[room as usize].people.first() {
                            act(g, b"A quivering horde of maggots consumes $p.", true, Some(first), Some(oid), None, TO_ROOM);
                            act(g, b"A quivering horde of maggots consumes $p.", true, Some(first), Some(oid), None, TO_CHAR);
                        }
                    }
                }
                let contents = g.obj(oid).contains.clone();
                let in_obj = g.obj(oid).in_obj;
                let carrier = g.obj(oid).carried_by;
                let room = g.obj(oid).in_room;
                for c in contents {
                    crate::handler::obj_from_obj(g, c);
                    if let Some(up) = in_obj {
                        crate::handler::obj_to_obj(g, c, up);
                    } else if let Some(cb) = carrier {
                        let r = g.ch(cb).in_room;
                        crate::handler::obj_to_room(g, c, r);
                    } else if room != NOWHERE {
                        crate::handler::obj_to_room(g, c, room);
                    } else {
                        g.log("SYSERR: point_update: corpse contents with nowhere to go".to_string());
                    }
                }
                crate::handler::extract_obj(g, oid);
            }
        } else if g.obj(oid).timer > 0 {
            g.obj_mut(oid).timer -= 1;
            if g.obj(oid).timer == 0 {
                crate::dg::triggers::timer_otrigger(g, oid);
            }
        }
    }

    // Take 1 from the happy-hour tick counter, and end happy-hour if zero
    // The last tick zeroes all three rates as well, so a
    // finished happy hour leaves no residue for the next `happyhour time`.
    if g.happy.ticks_left > 1 {
        g.happy.ticks_left -= 1;
    } else if g.happy.ticks_left == 1 {
        g.happy.qp_rate = 0;
        g.happy.exp_rate = 0;
        g.happy.gold_rate = 0;
        g.happy.ticks_left = 0;
        crate::comm::game_info(g, b"Happy hour has ended!");
    }
}

pub fn gain_condition(g: &mut Game, chid: CharId, condition: usize, value: i32) {
    {
        let ch = g.ch(chid);
        if ch.is_npc() || ch.ps().conditions[condition] == -1 {
            return;
        }
    }
    let intoxicated = g.ch(chid).ps().conditions[DRUNK] > 0;
    {
        let ch = g.ch_mut(chid);
        let c = (ch.ps().conditions[condition] as i32 + value).clamp(0, 24);
        ch.ps_mut().conditions[condition] = c as i16;
    }
    if g.ch(chid).ps().conditions[condition] != 0 || g.ch(chid).plr(flags::PLR_WRITING) {
        return;
    }
    match condition {
        HUNGER => send_to_char(g, chid, b"You are hungry.\r\n"),
        THIRST => send_to_char(g, chid, b"You are thirsty.\r\n"),
        DRUNK => {
            if intoxicated {
                send_to_char(g, chid, b"You are now sober.\r\n");
            }
        }
        _ => {}
    }
}

/// increase_gold. Negative amt decreases (floor 0).
pub fn increase_gold(g: &mut Game, chid: CharId, amt: i32) -> i32 {
    let curr = g.ch(chid).points.gold;
    let new = if amt < 0 {
        let mut v = curr.saturating_add(amt).max(0);
        if v > curr {
            v = 0;
        }
        v
    } else {
        let mut v = curr.saturating_add(amt).min(MAX_GOLD);
        if v < curr {
            v = MAX_GOLD;
        }
        v
    };
    g.ch_mut(chid).points.gold = new;
    if new == MAX_GOLD {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(cc(g, chid, C_SPR, KBRED));
        out.extend_from_slice(b"You have reached the maximum gold!\r\n");
        out.extend_from_slice(cc(g, chid, C_SPR, KNRM));
        out.extend_from_slice(b"You must spend it or bank it before you can gain any more.\r\n");
        send_to_char(g, chid, &out);
    }
    new
}

pub fn decrease_gold(g: &mut Game, chid: CharId, deduction: i32) -> i32 {
    increase_gold(g, chid, -deduction);
    g.ch(chid).points.gold
}

pub fn increase_bank(g: &mut Game, chid: CharId, amt: i32) -> i32 {
    if g.ch(chid).is_npc() {
        return 0;
    }
    let curr = g.ch(chid).points.bank_gold;
    let new = if amt < 0 {
        let mut v = curr.saturating_add(amt).max(0);
        if v > curr {
            v = 0;
        }
        v
    } else {
        let mut v = curr.saturating_add(amt).min(MAX_BANK);
        if v < curr {
            v = MAX_BANK;
        }
        v
    };
    g.ch_mut(chid).points.bank_gold = new;
    if new == MAX_BANK {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(cc(g, chid, C_SPR, KBRED));
        out.extend_from_slice(b"You have reached the maximum bank balance!\r\n");
        out.extend_from_slice(cc(g, chid, C_SPR, KNRM));
        out.extend_from_slice(b"You cannot put more into your account unless you withdraw some first.\r\n");
        send_to_char(g, chid, &out);
    }
    new
}

pub fn decrease_bank(g: &mut Game, chid: CharId, deduction: i32) -> i32 {
    increase_bank(g, chid, -deduction);
    g.ch(chid).points.bank_gold
}

/// update_object: timer countdown, recursing contents
/// and the rest of the content chain.
fn update_object(g: &mut Game, oid: mud_data::ids::ObjId, use_: i32) {
    if g.obj(oid).timer > 0 {
        g.obj_mut(oid).timer -= use_;
    }
    let contents = g.obj(oid).contains.clone();
    for c in contents {
        update_object(g, c, use_);
    }
}

/// update_char_objects: worn-light burn-down + timer
/// aging (equipped ×2, carried ×1). Called once per mud hour from
/// point_update.
pub fn update_char_objects(g: &mut Game, chid: CharId) {
    if let Some(light) = g.ch(chid).equipment[WEAR_LIGHT] {
        if g.obj(light).type_flag == flags::ITEM_LIGHT && g.obj(light).values[2] > 0 {
            g.obj_mut(light).values[2] -= 1;
            let i = g.obj(light).values[2];
            if i == 1 {
                send_to_char(g, chid, b"Your light begins to flicker and fade.\r\n");
                act(g, b"$n's light begins to flicker and fade.", false, Some(chid), None, None, TO_ROOM);
            } else if i == 0 {
                send_to_char(g, chid, b"Your light sputters out and dies.\r\n");
                act(g, b"$n's light sputters out and dies.", false, Some(chid), None, None, TO_ROOM);
                let room = g.ch(chid).in_room;
                if room != mud_data::types::NOWHERE {
                    g.rooms[room as usize].light -= 1;
                }
            }
        }
    }

    for i in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[i] {
            update_object(g, oid, 2);
        }
    }

    let carried = g.ch(chid).carrying.clone();
    for oid in carried {
        update_object(g, oid, 1);
    }
}
