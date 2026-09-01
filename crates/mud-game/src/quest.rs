//! The autoquest system and the questmaster spec-proc.
//!
//! Quest prototypes come from the `.qst` files parsed at boot (mud-world);
//! this module owns the runtime half: assigning `questmaster` over the mobs
//! named by each quest, the `quest` command, the completion triggers fired
//! from combat/movement/give, and the per-mud-hour timeout sweep.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::comm::{act, send_to_char, TO_CHAR, TO_ROOM};
use crate::game::Game;
use crate::handler::{atoi, obj_to_char};
use crate::interpreter::{cmd_is, two_arguments};

// quest types.
pub const AQ_UNDEFINED: i32 = -1;
pub const AQ_OBJ_FIND: i32 = 0;
pub const AQ_ROOM_FIND: i32 = 1;
pub const AQ_MOB_FIND: i32 = 2;
pub const AQ_MOB_KILL: i32 = 3;
pub const AQ_MOB_SAVE: i32 = 4;
pub const AQ_OBJ_RETURN: i32 = 5;
pub const AQ_ROOM_CLEAR: i32 = 6;

pub const AQ_REPEATABLE: u32 = 1 << 0;

// `quest` subcommands, in quest_cmd[] order.
const SCMD_QUEST_LIST: usize = 0;
const SCMD_QUEST_HISTORY: usize = 1;
const SCMD_QUEST_JOIN: usize = 2;
const SCMD_QUEST_LEAVE: usize = 3;
const SCMD_QUEST_PROGRESS: usize = 4;
const SCMD_QUEST_STATUS: usize = 5;

const QUEST_CMD: [&str; 6] = ["list", "history", "join", "leave", "progress", "status"];

const QUEST_MORT_USAGE: &[u8] =
    b"Usage: quest list | history | progress | join <nn> | leave";
const QUEST_IMM_USAGE: &[u8] =
    b"Usage: quest list | history | progress | join <nn> | leave | status <vnum>";

pub const QUEST_TYPES: [&str; 7] = [
    "Object",
    "Room",
    "Find mob",
    "Kill mob",
    "Save mob",
    "Return object",
    "Clear room",
];

pub const AQ_FLAGS: [&str; 1] = ["REPEATABLE"];

// ------------------------------------------------------------- utilities

pub fn real_quest(g: &Game, vnum: i32) -> Option<usize> {
    if vnum < 0 {
        return None;
    }
    g.world.quests.iter().position(|q| q.vnum as i32 == vnum)
}

pub fn is_complete(g: &Game, chid: CharId, vnum: i32) -> bool {
    let Some(ps) = g.ch(chid).player_specials.as_ref() else { return false };
    ps.completed_quests.iter().any(|&v| v as i32 == vnum)
}

/// find_quest_by_qmnum: the Nth quest this questmaster
/// offers, counting from 1 in table order.
fn find_quest_by_qmnum(g: &Game, qm: i32, num: i32) -> Option<i32> {
    let mut found = 0;
    for q in &g.world.quests {
        if qm == q.qm_vnum {
            found += 1;
            if found == num {
                return Some(q.vnum as i32);
            }
        }
    }
    None
}

fn qname(g: &Game, rnum: usize) -> Vec<u8> {
    g.world.quests[rnum].name.clone().unwrap_or_default()
}

fn qdesc(g: &Game, rnum: usize) -> Vec<u8> {
    g.world.quests[rnum].desc.clone().unwrap_or_default()
}

fn qinfo(g: &Game, rnum: usize) -> Vec<u8> {
    g.world.quests[rnum].info.clone().unwrap_or_default()
}

/// A mob prototype is named by its short description,
/// so a questmaster renders as "the recruit", not its keyword list.
fn qm_name(g: &Game, qm_vnum: i32) -> Option<Vec<u8>> {
    let rnum = g.world.real_mobile(qm_vnum as Idx)?;
    Some(g.world.mob_protos[rnum as usize].short_descr.clone().unwrap_or_default())
}

/// assign_the_quests. A questmaster that already carries a
/// spec keeps it as the quest's secondary proc, which `questmaster` calls
/// first.
pub fn assign_the_quests(g: &mut Game) {
    for rnum in 0..g.world.quests.len() {
        let qm = g.world.quests[rnum].qm_vnum;
        if qm == NOBODY as i32 || qm < 0 {
            g.log(format!(
                "SYSERR: Quest #{} has no questmaster specified.",
                g.world.quests[rnum].vnum
            ));
            continue;
        }
        let Some(mrnum) = g.world.real_mobile(qm as Idx) else {
            g.log(format!(
                "SYSERR: Quest #{} has an invalid questmaster.",
                g.world.quests[rnum].vnum
            ));
            continue;
        };
        let existing = g.mob_specs[mrnum as usize];
        if let Some(spec) = existing {
            if spec != crate::spec::MobSpec::QuestMaster {
                g.quest_secondary[rnum] = Some(spec);
            }
        }
        g.mob_specs[mrnum as usize] = Some(crate::spec::MobSpec::QuestMaster);
    }
}

// ----------------------------------------------------- completion machinery

pub fn set_quest(g: &mut Game, chid: CharId, rnum: usize) {
    let (vnum, time, quantity) = {
        let q = &g.world.quests[rnum];
        (q.vnum as Idx, q.time, q.obj_out)
    };
    let ps = g.ch_mut(chid).ps_mut();
    ps.current_quest = vnum;
    ps.quest_time = time;
    ps.quest_counter = quantity;
    ps.pref.set(flags::PRF_QUEST);
}

pub fn clear_quest(g: &mut Game, chid: CharId) {
    let ps = g.ch_mut(chid).ps_mut();
    ps.current_quest = NOTHING;
    ps.quest_time = -1;
    ps.quest_counter = 0;
    ps.pref.remove(flags::PRF_QUEST);
}

/// add_completed_quest.
/// remove_completed_quest: drop one vnum from the
/// history. The count is decremented once whether or not the vnum was
/// present, so a miss silently loses the last slot; the rebuilt array keeps
/// the same observable length.
pub fn remove_completed_quest(g: &mut Game, chid: CharId, vnum: i32) {
    let ps = g.ch_mut(chid).ps_mut();
    ps.completed_quests.retain(|&q| q as i32 != vnum);
    ps.num_completed_quests -= 1;
    ps.completed_quests.truncate(ps.num_completed_quests.max(0) as usize);
}

/// add_completed_quest, exported for `set <plr> questhistory`.
pub fn add_completed_quest_pub(g: &mut Game, chid: CharId, vnum: i32) {
    add_completed_quest(g, chid, vnum);
}

fn add_completed_quest(g: &mut Game, chid: CharId, vnum: i32) {
    let ps = g.ch_mut(chid).ps_mut();
    ps.completed_quests.push(vnum as Idx);
    ps.num_completed_quests += 1;
}

/// generic_complete_quest. Rewards are happy-hour scaled
/// as a float percentage, truncated to int.
pub fn generic_complete_quest(g: &mut Game, chid: CharId) {
    {
        let ps = g.ch_mut(chid).ps_mut();
        ps.quest_counter -= 1;
        if ps.quest_counter > 0 {
            crate::players_glue::save_char(g, chid);
            return;
        }
    }
    let vnum = g.ch(chid).ps().current_quest as i32;
    let Some(rnum) = real_quest(g, vnum) else {
        crate::players_glue::save_char(g, chid);
        return;
    };

    let (points, gold, exp, obj_reward, qflags, next_quest) = {
        let q = &g.world.quests[rnum];
        (q.value, q.gold_reward, q.exp_reward, q.obj_reward, q.flags, q.next_quest)
    };
    let done = g.world.quests[rnum].done.clone().unwrap_or_default();

    let happy = crate::act::other::is_happyhour(g);
    let awarded_qp = if happy && g.happy.qp_rate > 0 {
        happy_scale(points, g.happy.qp_rate)
    } else {
        points
    };
    g.ch_mut(chid).ps_mut().questpoints += awarded_qp;
    let mut msg = done;
    msg.extend_from_slice(
        format!("\r\nYou have been awarded {} quest points for your service.\r\n", awarded_qp)
            .as_bytes(),
    );
    send_to_char(g, chid, &msg);

    if gold != 0 {
        let awarded = if happy && g.happy.gold_rate > 0 {
            happy_scale(gold, g.happy.gold_rate)
        } else {
            gold
        };
        crate::limits::increase_gold(g, chid, awarded);
        let m = format!("You have been awarded {} gold coins for your service.\r\n", awarded);
        send_to_char(g, chid, m.as_bytes());
    }

    if exp != 0 {
        // NOTE, the order matters: gain_exp is called with the UNSCALED
        // reward and only the *message* is happy-scaled.
        crate::limits::gain_exp(g, chid, exp);
        if happy && g.happy.exp_rate > 0 {
            let m = format!(
                "You have been awarded {} experience for your service.\r\n",
                happy_scale(exp, g.happy.exp_rate)
            );
            send_to_char(g, chid, m.as_bytes());
        } else {
            let m =
                format!("You have been awarded {} experience points for your service.\r\n", exp);
            send_to_char(g, chid, m.as_bytes());
        }
    }

    if obj_reward != 0 && obj_reward != NOTHING as i32 {
        if let Some(ornum) = g.world.real_object(obj_reward as Idx) {
            if let Some(oid) = crate::db::read_object(g, ornum) {
                obj_to_char(g, oid, chid);
                let mut m = b"You have been presented with ".to_vec();
                m.extend_from_slice(crate::handler::obj_short(g, oid));
                m.extend_from_slice(crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM));
                m.extend_from_slice(b" for your service.\r\n");
                send_to_char(g, chid, &m);
            }
        }
    }

    if qflags & AQ_REPEATABLE == 0 {
        add_completed_quest(g, chid, vnum);
    }
    clear_quest(g, chid);

    if let Some(nrnum) = real_quest(g, next_quest) {
        if next_quest != vnum && !is_complete(g, chid, next_quest) {
            set_quest(g, chid, nrnum);
            let mut m = b"The next stage of your quest awaits:\r\n".to_vec();
            m.extend_from_slice(&qinfo(g, nrnum));
            send_to_char(g, chid, &m);
        }
    }
    crate::players_glue::save_char(g, chid);
}

/// `(int)(value * ((float)(100 + rate) / 100.0))`, floored at 0.
fn happy_scale(value: i32, rate: i32) -> i32 {
    let scaled = (value as f32) * ((100 + rate) as f32 / 100.0);
    (scaled as i32).max(0)
}

pub fn autoquest_trigger_check(
    g: &mut Game,
    chid: CharId,
    vict: Option<CharId>,
    object: Option<ObjId>,
    type_: i32,
) {
    if g.ch(chid).is_npc() {
        return;
    }
    let cur = g.ch(chid).ps().current_quest;
    if cur == NOTHING {
        return;
    }
    let Some(rnum) = real_quest(g, cur as i32) else { return };
    if g.world.quests[rnum].type_ != type_ {
        return;
    }
    let target = g.world.quests[rnum].target;
    let room = g.ch(chid).in_room;

    match type_ {
        AQ_OBJ_FIND => {
            if let Some(oid) = object {
                if target == crate::dg::obj_vnum(g, oid) {
                    generic_complete_quest(g, chid);
                }
            }
        }
        AQ_ROOM_FIND => {
            if room != NOWHERE && target == g.world.rooms[room as usize].vnum as i32 {
                generic_complete_quest(g, chid);
            }
        }
        AQ_MOB_FIND => {
            if room == NOWHERE {
                return;
            }
            for i in g.rooms[room as usize].people.clone() {
                if g.try_ch(i).is_some_and(|c| c.is_npc()) && target == crate::dg::mob_vnum(g, i) {
                    generic_complete_quest(g, chid);
                }
            }
        }
        AQ_MOB_KILL => {
            if let Some(v) = vict {
                if !g.ch(chid).is_npc()
                    && g.try_ch(v).is_some_and(|c| c.is_npc())
                    && chid != v
                    && target == crate::dg::mob_vnum(g, v)
                {
                    generic_complete_quest(g, chid);
                }
            }
        }
        AQ_MOB_SAVE => {
            // "found" starts TRUE and any *other* live, non-charmed NPC in
            // the room clears it.
            let mut found = vict != Some(chid);
            if room != NOWHERE {
                for i in g.rooms[room as usize].people.clone() {
                    if !found {
                        break;
                    }
                    let Some(c) = g.try_ch(i) else { continue };
                    if c.is_npc() && !c.act.is_set(flags::MOB_NOTDEADYET) {
                        if crate::dg::mob_vnum(g, i) != target
                            && !g.ch(i).affected_by.is_set(flags::AFF_CHARM)
                        {
                            found = false;
                        }
                    }
                }
            }
            if found {
                generic_complete_quest(g, chid);
            }
        }
        AQ_OBJ_RETURN => {
            let return_mob = g.world.quests[rnum].obj_in;
            if let Some(v) = vict {
                if g.try_ch(v).is_some_and(|c| c.is_npc())
                    && crate::dg::mob_vnum(g, v) == return_mob
                {
                    if let Some(oid) = object {
                        if crate::dg::obj_vnum(g, oid) == target {
                            generic_complete_quest(g, chid);
                            crate::handler::extract_obj(g, oid);
                        }
                    }
                }
            }
        }
        AQ_ROOM_CLEAR => {
            if room != NOWHERE && target == g.world.rooms[room as usize].vnum as i32 {
                let mut found = true;
                for i in g.rooms[room as usize].people.clone() {
                    if !found {
                        break;
                    }
                    let Some(c) = g.try_ch(i) else { continue };
                    if c.is_npc() && !c.act.is_set(flags::MOB_NOTDEADYET) {
                        found = false;
                    }
                }
                if found {
                    generic_complete_quest(g, chid);
                }
            }
        }
        _ => g.log("SYSERR: Invalid quest type passed to autoquest_trigger_check".to_string()),
    }
}

fn quest_timeout(g: &mut Game, chid: CharId) {
    let ps = g.ch(chid).ps();
    if ps.current_quest != NOTHING && ps.quest_time != -1 {
        clear_quest(g, chid);
        send_to_char(g, chid, b"You have run out of time to complete the quest.\r\n");
    }
}

/// check_timed_quests: one mud hour off every timer.
pub fn check_timed_quests(g: &mut Game) {
    for chid in g.character_list.clone() {
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.is_npc() {
            continue;
        }
        let ps = ch.ps();
        if ps.current_quest == NOTHING || ps.quest_time == -1 {
            continue;
        }
        g.ch_mut(chid).ps_mut().quest_time -= 1;
        if g.ch(chid).ps().quest_time == 0 {
            quest_timeout(g, chid);
        }
    }
}

// ---------------------------------------------------------- command helpers

/// count_quests: quests whose vnum falls in a range.
pub fn count_quests(g: &Game, low: i32, high: i32) -> usize {
    g.world
        .quests
        .iter()
        .filter(|q| (q.vnum as i32) >= low && (q.vnum as i32) <= high)
        .count()
}

/// list_quests — the `qlist` listing, shared with OLC.
pub fn list_quests(g: &mut Game, chid: CharId, zone: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = match zone {
        Some(z) => (g.world.zones[z].bot as i32, g.world.zones[z].top as i32),
        None => (vmin, vmax),
    };
    send_to_char(
        g,
        chid,
        b"Index VNum    Description                                  Questmaster\r\n\
          ----- ------- -------------------------------------------- -----------\r\n",
    );
    let mut counter = 0;
    for rnum in 0..g.world.quests.len() {
        let vnum = g.world.quests[rnum].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let qm = g.world.quests[rnum].qm_vnum;
        let name = qname(g, rnum);
        let mut line = format!("\tg{:4}\tn) [\tg{:<5}\tn] \tc", counter, vnum).into_bytes();
        let mut n = name;
        n.truncate(44);
        let pad = 44usize.saturating_sub(n.len());
        line.extend_from_slice(&n);
        line.extend(std::iter::repeat(b' ').take(pad));
        line.extend_from_slice(
            format!("\tn \ty[{:5}]\tn\r\n", if qm == NOBODY as i32 { 0 } else { qm }).as_bytes(),
        );
        send_to_char(g, chid, &line);
    }
    if counter == 0 {
        send_to_char(g, chid, b"None found.\r\n");
    }
}

fn quest_hist(g: &mut Game, chid: CharId) {
    send_to_char(
        g,
        chid,
        b"Quests that you have completed:\r\n\
          Index Description                                          Questmaster\r\n\
          ----- ---------------------------------------------------- -----------\r\n",
    );
    let completed: Vec<i32> =
        g.ch(chid).ps().completed_quests.iter().map(|&v| v as i32).collect();
    let mut counter = 0;
    for vnum in completed {
        counter += 1;
        match real_quest(g, vnum) {
            Some(rnum) => {
                let mut d = qdesc(g, rnum);
                d.truncate(52);
                let pad = 52usize.saturating_sub(d.len());
                let qm = g.world.quests[rnum].qm_vnum;
                let master = qm_name(g, qm).unwrap_or_else(|| b"Unknown".to_vec());
                let mut line = format!("\tg{:4}\tn) \tc", counter).into_bytes();
                line.extend_from_slice(&d);
                line.extend(std::iter::repeat(b' ').take(pad));
                line.extend_from_slice(b"\tn \ty");
                line.extend_from_slice(&master);
                line.extend_from_slice(b"\tn\r\n");
                send_to_char(g, chid, &line);
            }
            None => {
                let line = format!(
                    "\tg{:4}\tn) \tcUnknown Quest (it no longer exists)\tn\r\n",
                    counter
                );
                send_to_char(g, chid, line.as_bytes());
            }
        }
    }
    if counter == 0 {
        send_to_char(g, chid, b"You haven't completed any quests yet.\r\n");
    }
}

/// quest_join. Every refusal and the success path alike
/// end with the questmaster telling the player, through one shared
/// `do_tell`.
fn quest_join(g: &mut Game, chid: CharId, qm: CharId, argument: &[u8]) {
    let name = g.ch(chid).get_name().to_vec();
    let tell = |g: &mut Game, body: &[u8]| {
        let mut buf = name.clone();
        buf.push(b' ');
        buf.extend_from_slice(body);
        let cmd = g.shop_cmds.tell;
        crate::act::comm::do_tell(g, qm, &buf, cmd, 0);
    };

    let qm_vnum = crate::dg::mob_vnum(g, qm);
    let buf: Vec<u8> = if argument.is_empty() {
        b"What quest did you wish to join?".to_vec()
    } else if g.ch(chid).ps().current_quest != NOTHING {
        b"But you are already part of a quest!".to_vec()
    } else {
        let vnum = find_quest_by_qmnum(g, qm_vnum, atoi(argument));
        let rnum = vnum.and_then(|v| real_quest(g, v));
        match (vnum, rnum) {
            (None, _) | (_, None) => b"I don't know of such a quest!".to_vec(),
            (Some(vnum), Some(rnum)) => {
                let q = &g.world.quests[rnum];
                let (minl, maxl, prev, prereq, time) =
                    (q.min_level, q.max_level, q.prev_quest, q.prereq, q.time);
                let level = g.ch(chid).level as i32;
                if level < minl {
                    b"You are not experienced enough for that quest!".to_vec()
                } else if level > maxl {
                    b"You are too experienced for that quest!".to_vec()
                } else if is_complete(g, chid, vnum) {
                    b"You have already completed that quest!".to_vec()
                } else if prev != NOTHING as i32 && !is_complete(g, chid, prev) {
                    b"That quest is not available to you yet!".to_vec()
                } else if prereq != NOTHING as i32
                    && g.world.real_object(prereq as Idx).is_some()
                    && !carries_obj_vnum(g, chid, prereq)
                {
                    let ornum = g.world.real_object(prereq as Idx).unwrap();
                    let mut m = b"You need to have ".to_vec();
                    m.extend_from_slice(
                        g.world.obj_protos[ornum as usize]
                            .short_description
                            .as_deref()
                            .unwrap_or(b""),
                    );
                    m.extend_from_slice(b" first!");
                    m
                } else {
                    act(g, b"You join the quest.", true, Some(chid), None, None, TO_CHAR);
                    act(g, b"$n has joined a quest.", true, Some(chid), None, None, TO_ROOM);
                    tell(g, b"Listen carefully to the instructions.");
                    set_quest(g, chid, rnum);
                    let info = qinfo(g, rnum);
                    send_to_char(g, chid, &info);
                    if time != -1 {
                        format!(
                            "You have a time limit of {} turn{} to complete the quest.",
                            time,
                            if time == 1 { "" } else { "s" }
                        )
                        .into_bytes()
                    } else {
                        b"You can take however long you want to complete the quest.".to_vec()
                    }
                }
            }
        }
    };
    tell(g, &buf);
    crate::players_glue::save_char(g, chid);
}

fn carries_obj_vnum(g: &Game, chid: CharId, vnum: i32) -> bool {
    g.ch(chid).carrying.iter().any(|&o| {
        g.try_obj(o).is_some_and(|obj| {
            obj.item_number != NOTHING
                && g.world.obj_protos[obj.item_number as usize].vnum as i32 == vnum
        })
    })
}

/// quest_list — the `quest list <n>` detail view.
fn quest_list(g: &mut Game, chid: CharId, qm: CharId, argument: &[u8]) {
    let qm_vnum = crate::dg::mob_vnum(g, qm);
    let vnum = find_quest_by_qmnum(g, qm_vnum, atoi(argument));
    let rnum = vnum.and_then(|v| real_quest(g, v));
    let (Some(vnum), Some(rnum)) = (vnum, rnum) else {
        send_to_char(g, chid, b"That is not a valid quest!\r\n");
        return;
    };
    if g.world.quests[rnum].info.is_none() {
        send_to_char(g, chid, b"There is no further information on that quest.\r\n");
        return;
    }
    let mut out = format!("Complete Details on Quest {} \tc", vnum).into_bytes();
    out.extend_from_slice(&qdesc(g, rnum));
    out.extend_from_slice(b"\tn:\r\n");
    out.extend_from_slice(&qinfo(g, rnum));
    send_to_char(g, chid, &out);

    let (prev, time) = (g.world.quests[rnum].prev_quest, g.world.quests[rnum].time);
    if prev != NOTHING as i32 {
        if let Some(prnum) = real_quest(g, prev) {
            let mut m = b"You have to have completed quest ".to_vec();
            m.extend_from_slice(&qname(g, prnum));
            m.extend_from_slice(b" first.\r\n");
            send_to_char(g, chid, &m);
        }
    }
    if time != -1 {
        let m = format!(
            "There is a time limit of {} turn{} to complete the quest.\r\n",
            time,
            if time == 1 { "" } else { "s" }
        );
        send_to_char(g, chid, m.as_bytes());
    }
}

fn quest_quit(g: &mut Game, chid: CharId) {
    let cur = g.ch(chid).ps().current_quest;
    if cur == NOTHING {
        send_to_char(g, chid, b"But you currently aren't on a quest!\r\n");
        return;
    }
    let Some(rnum) = real_quest(g, cur as i32) else {
        clear_quest(g, chid);
        send_to_char(g, chid, b"You are now no longer part of the quest.\r\n");
        crate::players_glue::save_char(g, chid);
        return;
    };
    clear_quest(g, chid);
    let quit = g.world.quests[rnum].quit.clone();
    match quit {
        Some(text) if !text.eq_ignore_ascii_case(b"undefined") => send_to_char(g, chid, &text),
        _ => send_to_char(g, chid, b"You are now no longer part of the quest.\r\n"),
    }
    let penalty = g.world.quests[rnum].penalty;
    if penalty != 0 {
        g.ch_mut(chid).ps_mut().questpoints -= penalty;
        let m = format!("You have lost {} quest points for your cowardice.\r\n", penalty);
        send_to_char(g, chid, m.as_bytes());
    }
    crate::players_glue::save_char(g, chid);
}

fn quest_progress(g: &mut Game, chid: CharId) {
    let cur = g.ch(chid).ps().current_quest;
    if cur == NOTHING {
        send_to_char(g, chid, b"But you currently aren't on a quest!\r\n");
        return;
    }
    let Some(rnum) = real_quest(g, cur as i32) else {
        clear_quest(g, chid);
        send_to_char(g, chid, b"Your quest seems to no longer exist.\r\n");
        return;
    };
    let mut out = b"You are on the following quest:\r\n".to_vec();
    out.extend_from_slice(&qdesc(g, rnum));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&qinfo(g, rnum));
    send_to_char(g, chid, &out);

    let quantity = g.world.quests[rnum].obj_out;
    if quantity > 1 {
        let counter = g.ch(chid).ps().quest_counter;
        let m = format!(
            "You still have to achieve {} out of {} goals for the quest.\r\n",
            counter, quantity
        );
        send_to_char(g, chid, m.as_bytes());
    }
    let time = g.ch(chid).ps().quest_time;
    if time > 0 {
        let m = format!(
            "You have {} turn{} remaining to complete the quest.\r\n",
            time,
            if time == 1 { "" } else { "s" }
        );
        send_to_char(g, chid, m.as_bytes());
    }
}

/// quest_show — what a questmaster offers.
fn quest_show(g: &mut Game, chid: CharId, qm: i32) {
    send_to_char(
        g,
        chid,
        b"The following quests are available:\r\n\
          Index Description                                          ( Vnum) Done?\r\n\
          ----- ---------------------------------------------------- ------- -----\r\n",
    );
    let mut counter = 0;
    for rnum in 0..g.world.quests.len() {
        if g.world.quests[rnum].qm_vnum != qm {
            continue;
        }
        counter += 1;
        let vnum = g.world.quests[rnum].vnum as i32;
        let done = is_complete(g, chid, vnum);
        let mut d = qdesc(g, rnum);
        d.truncate(52);
        let pad = 52usize.saturating_sub(d.len());
        let mut line = format!("\tg{:4}\tn) \tc", counter).into_bytes();
        line.extend_from_slice(&d);
        line.extend(std::iter::repeat(b' ').take(pad));
        line.extend_from_slice(
            format!("\tn \ty({:5})\tn \ty({})\tn\r\n", vnum, if done { "Yes" } else { "No " })
                .as_bytes(),
        );
        send_to_char(g, chid, &line);
    }
    if counter == 0 {
        send_to_char(g, chid, b"There are no quests available here at the moment.\r\n");
    }
}

/// quest_stat — the immortal detail dump.
fn quest_stat(g: &mut Game, chid: CharId, argument: &[u8]) {
    if argument.is_empty() {
        let mut m = QUEST_IMM_USAGE.to_vec();
        m.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &m);
        return;
    }
    let Some(rnum) = real_quest(g, atoi(argument)) else {
        send_to_char(g, chid, b"That quest does not exist.\r\n");
        return;
    };

    let q = g.world.quests[rnum].clone();
    let flagbuf = sprintbit(q.flags as i64, &AQ_FLAGS);

    let targetname: Vec<u8> = match q.type_ {
        AQ_OBJ_FIND | AQ_OBJ_RETURN => match g.world.real_object(q.target as Idx) {
            Some(r) => {
                g.world.obj_protos[r as usize].short_description.clone().unwrap_or_default()
            }
            None => b"An unknown object".to_vec(),
        },
        AQ_ROOM_FIND | AQ_ROOM_CLEAR => match g.world.real_room(q.target as Idx) {
            Some(r) => g.world.rooms[r as usize].name.clone().unwrap_or_default(),
            None => b"An unknown room".to_vec(),
        },
        AQ_MOB_FIND | AQ_MOB_KILL | AQ_MOB_SAVE => match g.world.real_mobile(q.target as Idx) {
            // An NPC prototype is named by its short description.
            Some(r) => g.world.mob_protos[r as usize].short_descr.clone().unwrap_or_default(),
            None => b"An unknown mobile".to_vec(),
        },
        _ => b"Unknown".to_vec(),
    };
    let qmname = qm_name(g, q.qm_vnum).unwrap_or_else(|| b"(Invalid vnum)".to_vec());

    let mut out = format!(
        "VNum  : [\ty{:5}\tn], RNum: [\ty{:5}\tn] -- Questmaster: [\ty{:5}\tn] \ty",
        q.vnum,
        rnum,
        if q.qm_vnum == NOBODY as i32 { -1 } else { q.qm_vnum }
    )
    .into_bytes();
    out.extend_from_slice(&qmname);
    out.extend_from_slice(b"\tn\r\nName  : \ty");
    out.extend_from_slice(q.name.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\tn\r\nDesc  : \ty");
    out.extend_from_slice(q.desc.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\tn\r\nAccept Message:\r\n\tc");
    out.extend_from_slice(q.info.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\tnCompletion Message:\r\n\tc");
    out.extend_from_slice(q.done.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\tnQuit Message:\r\n\tc");
    match &q.quit {
        Some(t) if !t.eq_ignore_ascii_case(b"undefined") => out.extend_from_slice(t),
        _ => out.extend_from_slice(b"Nothing\r\n"),
    }
    out.extend_from_slice(b"\tnType  : \ty");
    // Indexing quest_types[] with the raw type breaks for a quest
    // saved before a type was chosen, which holds AQ_UNDEFINED.
    out.extend_from_slice(crate::olc::qedit::quest_type_name(q.type_).as_bytes());
    out.extend_from_slice(
        format!(
            "\tn\r\nTarget: \ty{}\tn \ty",
            if q.target == NOBODY as i32 { -1 } else { q.target }
        )
        .as_bytes(),
    );
    out.extend_from_slice(&targetname);
    out.extend_from_slice(format!("\tn, Quantity: \ty{}\tn\r\n", q.obj_out).as_bytes());
    out.extend_from_slice(
        format!(
            "Value : \ty{}\tn, Penalty: \ty{}\tn, Min Level: \ty{:2}\tn, Max Level: \ty{:2}\tn\r\n",
            q.value, q.penalty, q.min_level, q.max_level
        )
        .as_bytes(),
    );
    out.extend_from_slice(b"Flags : \tc");
    out.extend_from_slice(&flagbuf);
    out.extend_from_slice(b"\tn\r\n");
    send_to_char(g, chid, &out);

    if q.prereq != NOTHING as i32 {
        let mut m = format!("Preq  : [\ty{:5}\tn] \ty", q.prereq).into_bytes();
        match g.world.real_object(q.prereq as Idx) {
            Some(r) => m.extend_from_slice(
                g.world.obj_protos[r as usize].short_description.as_deref().unwrap_or(b""),
            ),
            None => m.extend_from_slice(b"an unknown object"),
        }
        m.extend_from_slice(b"\tn\r\n");
        send_to_char(g, chid, &m);
    }
    if q.type_ == AQ_OBJ_RETURN {
        let mut m = format!("Mob   : [\ty{:5}\tn] \ty", q.obj_in).into_bytes();
        match g.world.real_mobile(q.obj_in as Idx) {
            Some(r) => m.extend_from_slice(
                g.world.mob_protos[r as usize].short_descr.as_deref().unwrap_or(b""),
            ),
            None => m.extend_from_slice(b"an unknown mob"),
        }
        m.extend_from_slice(b"\tn\r\n");
        send_to_char(g, chid, &m);
    }
    if q.time != -1 {
        let m = format!(
            "Limit : There is a time limit of {} turn{} to complete.\r\n",
            q.time,
            if q.time == 1 { "" } else { "s" }
        );
        send_to_char(g, chid, m.as_bytes());
    } else {
        send_to_char(g, chid, b"Limit : There is no time limit on this quest.\r\n");
    }

    send_to_char(g, chid, b"Prior :");
    if q.prev_quest == NOTHING as i32 {
        send_to_char(g, chid, b" \tyNone.\tn\r\n");
    } else {
        // A chain link outlives the quest it names the moment that
        // quest is deleted, so QST_DESC cannot be handed the rnum. The vnum
        // is still printed -- the quest really does carry it, and qedit's
        // menu shows it too.
        let mut m = format!(" [\ty{:5}\tn] \tc", q.prev_quest).into_bytes();
        match real_quest(g, q.prev_quest) {
            Some(r) => m.extend_from_slice(&qdesc(g, r)),
            None => m.extend_from_slice(b"an unknown quest"),
        }
        m.extend_from_slice(b"\tn\r\n");
        send_to_char(g, chid, &m);
    }
    send_to_char(g, chid, b"Next  :");
    if q.next_quest == NOTHING as i32 {
        send_to_char(g, chid, b" \tyNone.\tn\r\n");
    } else {
        // A chain link outlives the quest it names the moment that
        // quest is deleted, so QST_DESC cannot be handed the rnum. The vnum
        // is still printed -- the quest really does carry it, and qedit's
        // menu shows it too.
        let mut m = format!(" [\ty{:5}\tn] \tc", q.next_quest).into_bytes();
        match real_quest(g, q.next_quest) {
            Some(r) => m.extend_from_slice(&qdesc(g, r)),
            None => m.extend_from_slice(b"an unknown quest"),
        }
        m.extend_from_slice(b"\tn\r\n");
        send_to_char(g, chid, &m);
    }
}

// ------------------------------------------------------- command + spec-proc

pub fn do_quest(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2, _) = two_arguments(argument);
    let usage: &[u8] =
        if g.ch(chid).level < LVL_IMMORT { QUEST_MORT_USAGE } else { QUEST_IMM_USAGE };
    let tp = if arg1.is_empty() {
        None
    } else {
        crate::act::informative::search_block(&arg1, &QUEST_CMD)
    };
    let Some(tp) = tp else {
        let mut m = usage.to_vec();
        m.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &m);
        return;
    };
    match tp {
        // list/join should have been handled by the questmaster spec proc.
        SCMD_QUEST_LIST | SCMD_QUEST_JOIN => {
            send_to_char(g, chid, b"Sorry, but you cannot do that here!\r\n")
        }
        SCMD_QUEST_HISTORY => quest_hist(g, chid),
        SCMD_QUEST_LEAVE => quest_quit(g, chid),
        SCMD_QUEST_PROGRESS => quest_progress(g, chid),
        SCMD_QUEST_STATUS => {
            if g.ch(chid).level < LVL_IMMORT {
                let mut m = QUEST_MORT_USAGE.to_vec();
                m.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &m);
            } else {
                quest_stat(g, chid, &arg2);
            }
        }
        _ => {
            let mut m = usage.to_vec();
            m.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &m);
        }
    }
}

pub fn questmaster(g: &mut Game, chid: CharId, qm: CharId, cmd: usize, argument: &[u8]) -> bool {
    let qm_vnum = crate::dg::mob_vnum(g, qm);
    let Some(rnum) = g.world.quests.iter().position(|q| q.qm_vnum == qm_vnum) else {
        return false; // No quests for this mob
    };
    // The secondary spec proc gets first refusal.
    if let Some(spec) = g.quest_secondary.get(rnum).copied().flatten() {
        if crate::spec::call_mob_spec(g, spec, chid, qm, cmd, argument) {
            return true;
        }
    }
    if !cmd_is(g, cmd, b"quest") {
        return false;
    }
    let (arg1, arg2, _) = two_arguments(argument);
    if arg1.is_empty() {
        return false;
    }
    let Some(tp) = crate::act::informative::search_block(&arg1, &QUEST_CMD) else {
        return false;
    };
    match tp {
        SCMD_QUEST_LIST => {
            if arg2.is_empty() {
                quest_show(g, chid, qm_vnum);
            } else {
                quest_list(g, chid, qm, &arg2);
            }
        }
        SCMD_QUEST_JOIN => quest_join(g, chid, qm, &arg2),
        // fall through to the do_quest command processor
        _ => return false,
    }
    true
}

/// sprintbit: space-separated names, "NOBITS " when
/// nothing is set, "UNDEFINED " past the end of the table. The trailing
/// space is part of the output.
pub fn sprintbit(mut bitvector: i64, names: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut nr = 0usize;
    while bitvector != 0 {
        if bitvector & 1 != 0 {
            match names.get(nr) {
                Some(n) => {
                    out.extend_from_slice(n.as_bytes());
                    out.push(b' ');
                }
                None => out.extend_from_slice(b"UNDEFINED "),
            }
        }
        if nr < names.len() {
            nr += 1;
        }
        bitvector = ((bitvector as u64) >> 1) as i64;
    }
    if out.is_empty() {
        out.extend_from_slice(b"NOBITS ");
    }
    out
}
