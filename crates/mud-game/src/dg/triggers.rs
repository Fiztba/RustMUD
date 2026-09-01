//! Every trigger firing function. The gate order is load-bearing: each
//! rand_number(1,100) draw advances the shared sequence, so reordering the
//! gates changes what every later draw returns.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::tables::DIRS;
use mud_data::types::*;

use super::driver::{script_driver, script_driver_default, uid_var};
use super::misc::{skill_name_b, valid_dg_target};
use super::{
    add_var, char_script_id, has_obj_by_uid_in_lookup_table, obj_script_id, GoId, ScriptMem,
    DG_ALLOW_GODS, MTRIG_ACT, MTRIG_BRIBE, MTRIG_CAST, MTRIG_COMMAND, MTRIG_DAMAGE, MTRIG_DEATH,
    MTRIG_DOOR, MTRIG_ENTRY, MTRIG_FIGHT, MTRIG_GREET, MTRIG_GREET_ALL, MTRIG_HITPRCNT,
    MTRIG_LEAVE, MTRIG_LOAD, MTRIG_MEMORY, MTRIG_RANDOM, MTRIG_RECEIVE, MTRIG_SPEECH, MTRIG_TIME,
    OCMD_DRINK, OCMD_EAT, OCMD_QUAFF, OTRIG_CAST, OTRIG_COMMAND, OTRIG_CONSUME, OTRIG_DROP,
    OTRIG_GET, OTRIG_GIVE, OTRIG_LEAVE, OTRIG_LOAD, OTRIG_RANDOM, OTRIG_REMOVE, OTRIG_TIME,
    OTRIG_TIMER, OTRIG_WEAR, SCRIPT_ERROR_CODE, TRIG_NEW, WTRIG_CAST, WTRIG_COMMAND, WTRIG_DOOR,
    WTRIG_DROP, WTRIG_ENTER, WTRIG_LEAVE, WTRIG_LOGIN, WTRIG_RANDOM, WTRIG_RESET, WTRIG_SPEECH,
    WTRIG_TIME,
};
use crate::game::{Game, MudlogKind};
use crate::handler::can_see;

pub type BStr = Vec<u8>;

/// DEAD(ch): flagged for extraction.
fn dead(g: &Game, chid: CharId) -> bool {
    match g.try_ch(chid) {
        None => true,
        Some(ch) => {
            ch.plr(flags::PLR_NOTDEADYET) || ch.mob_flagged(flags::MOB_NOTDEADYET)
        }
    }
}

fn charmed(g: &Game, chid: CharId) -> bool {
    g.ch(chid).aff(flags::AFF_CHARM)
}

/// TRIGGER_CHECK on an instance snapshot.
fn trig_ok(t: &super::TrigInstance, bit: u32) -> bool {
    t.trigger_type & bit != 0 && t.depth == 0
}

fn one_phrase(arg: &[u8]) -> (BStr, &[u8]) {
    let is_sp = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let mut i = 0;
    while i < arg.len() && is_sp(arg[i]) {
        i += 1;
    }
    if i >= arg.len() {
        return (Vec::new(), &arg[arg.len()..]);
    }
    if arg[i] == b'"' {
        let close = super::expr::matching_quote(arg, i);
        let phrase = arg[i + 1..close.max(i + 1)].to_vec();
        let rest_at = if close < arg.len() { close + 1 } else { close };
        (phrase, &arg[rest_at.min(arg.len())..])
    } else {
        let start = i;
        while i < arg.len() && !is_sp(arg[i]) && arg[i] != b'"' {
            i += 1;
        }
        (arg[start..i].to_vec(), &arg[i..])
    }
}

/// is_substring: word-boundary substring.
pub fn is_substring(sub: &[u8], string: &[u8]) -> bool {
    // Reimplement with position info (str_str returns bool in our port).
    if sub.is_empty() {
        return false;
    }
    let lower = |b: u8| b.to_ascii_lowercase();
    let mut i = 0;
    // Naive scanner: find the FIRST str_str hit only.
    let mut found: Option<usize> = None;
    'outer: while i < string.len() {
        while i < string.len() && lower(string[i]) != lower(sub[0]) {
            i += 1;
        }
        let s = i;
        let mut t = 0;
        while t < sub.len() && i < string.len() && lower(string[i]) == lower(sub[t]) {
            t += 1;
            i += 1;
        }
        if t == sub.len() {
            found = Some(s);
            break 'outer;
        }
        if i >= string.len() {
            break;
        }
    }
    let Some(s) = found else { return false };
    let boundary = |b: u8| b.is_ascii_whitespace() || b.is_ascii_punctuation();
    let front_ok = s == 0 || boundary(string[s - 1]);
    let end = s + sub.len();
    let end_ok = end == string.len() || boundary(string[end]);
    front_ok && end_ok
}

pub fn word_check(str_: &[u8], wordlist: &[u8]) -> bool {
    if wordlist.first() == Some(&b'*') {
        return true;
    }
    let mut rest = wordlist;
    loop {
        let (phrase, r) = one_phrase(rest);
        if phrase.is_empty() {
            return false;
        }
        if is_substring(&phrase, str_) {
            return true;
        }
        rest = r;
    }
}

// ---- Mob triggers ----

pub fn random_mtrigger(g: &mut Game, chid: CharId) {
    if !g.script_check(GoId::Char(chid), MTRIG_RANDOM) || charmed(g, chid) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_RANDOM))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn bribe_mtrigger(g: &mut Game, chid: CharId, actor: CharId, amount: i32) {
    if !g.script_check(GoId::Char(chid), MTRIG_BRIBE) || charmed(g, chid) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_BRIBE))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if amount >= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, b"amount", amount.to_string().as_bytes(), 0);
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn greet_memory_mtrigger(g: &mut Game, actor: CharId) {
    if !valid_dg_target(g, actor, DG_ALLOW_GODS) {
        return;
    }
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return;
    }
    let people = g.rooms[room as usize].people.clone();
    let mut command_performed = false;
    for chid in people {
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.script_mem.is_empty()
            || !ch.awake()
            || ch.fighting.is_some()
            || chid == actor
            || ch.aff(flags::AFF_CHARM)
        {
            continue;
        }
        let actor_id = char_script_id(g, actor);
        let mems: Vec<ScriptMem> = g.ch(chid).script_mem.clone();
        for mem in mems {
            if g.try_ch(chid).is_none() || g.ch(chid).script_mem.is_empty() {
                break;
            }
            if mem.id != actor_id {
                continue;
            }
            if let Some(cmd) = &mem.cmd {
                let cmd = cmd.clone();
                crate::interpreter::command_interpreter(g, chid, &cmd);
                command_performed = true;
                // Break out of the mem loop after a command; the memory
                // deletion below never runs for it in this pass.
                break;
            }
            if !command_performed {
                let snapshot: Vec<(u64, i32)> = g
                    .script_of(GoId::Char(chid))
                    .map(|sc| {
                        sc.trig_list
                            .iter()
                            .filter(|t| t.trigger_type & MTRIG_MEMORY != 0 && t.depth == 0)
                            .map(|t| (t.iid, t.narg))
                            .collect()
                    })
                    .unwrap_or_default();
                for (iid, narg) in snapshot {
                    if g.trig(GoId::Char(chid), iid).is_none() {
                        continue;
                    }
                    if !can_see(g, chid, actor) {
                        continue;
                    }
                    if g.rng.rand_number(1, 100) <= narg {
                        let actor_uid = uid_var(actor_id);
                        if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                            add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                        }
                        script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
                        break;
                    }
                }
            }
            // delete the memory
            if let Some(c) = g.chars.get_mut(chid) {
                if let Some(pos) =
                    c.script_mem.iter().position(|m| m.id == mem.id && m.cmd == mem.cmd)
                {
                    c.script_mem.remove(pos);
                }
            }
        }
    }
}

/// greet_mtrigger: ALL matching triggers on ALL mobs,
/// results ANDed.
pub fn greet_mtrigger(g: &mut Game, actor: CharId, dir: i32) -> bool {
    if !valid_dg_target(g, actor, DG_ALLOW_GODS) {
        return true;
    }
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return true;
    }
    let mut final_ = true;
    let people = g.rooms[room as usize].people.clone();
    let dir_count = crate::fight::dir_count(g) as i32;
    for chid in people {
        let Some(ch) = g.try_ch(chid) else { continue };
        if !g.script_check(GoId::Char(chid), MTRIG_GREET | MTRIG_GREET_ALL)
            || !ch.awake()
            || ch.fighting.is_some()
            || chid == actor
            || ch.aff(flags::AFF_CHARM)
        {
            continue;
        }
        let snapshot: Vec<(u64, u32, i32)> = g
            .script_of(GoId::Char(chid))
            .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.trigger_type, t.narg)).collect())
            .unwrap_or_default();
        for (iid, ttype, narg) in snapshot {
            let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
            if t.depth != 0 {
                continue;
            }
            let greet_ok = ttype & MTRIG_GREET != 0 && can_see(g, chid, actor);
            let greet_all = ttype & MTRIG_GREET_ALL != 0;
            if !(greet_ok || greet_all) {
                continue;
            }
            if g.rng.rand_number(1, 100) <= narg {
                let dir_val: BStr = if dir >= 0 && dir < dir_count {
                    DIRS[rev_dir(dir as usize)].as_bytes().to_vec()
                } else {
                    b"none".to_vec()
                };
                let actor_uid = uid_var(char_script_id(g, actor));
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"direction", &dir_val, 0);
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                }
                let intermediate = script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
                if intermediate == 0 {
                    final_ = false;
                }
            }
        }
    }
    final_
}

/// rev_dir.
pub fn rev_dir(dir: usize) -> usize {
    const REV: [usize; 10] = [2, 3, 0, 1, 5, 4, 7, 6, 9, 8];
    REV.get(dir).copied().unwrap_or(0)
}

pub fn entry_memory_mtrigger(g: &mut Game, chid: CharId) {
    if g.try_ch(chid).is_none() || g.ch(chid).script_mem.is_empty() || charmed(g, chid) {
        return;
    }
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return;
    }
    let people = g.rooms[room as usize].people.clone();
    for actor in people {
        if g.try_ch(chid).is_none() || g.ch(chid).script_mem.is_empty() {
            break;
        }
        if actor == chid || g.try_ch(actor).is_none() {
            continue;
        }
        let actor_id = char_script_id(g, actor);
        let mems: Vec<ScriptMem> = g.ch(chid).script_mem.clone();
        for mem in mems {
            if g.try_ch(chid).is_none() || g.ch(chid).script_mem.is_empty() {
                break;
            }
            if mem.id != actor_id {
                continue;
            }
            if let Some(cmd) = &mem.cmd {
                let cmd = cmd.clone();
                crate::interpreter::command_interpreter(g, chid, &cmd);
            } else {
                let snapshot: Vec<(u64, i32)> = g
                    .script_of(GoId::Char(chid))
                    .map(|sc| {
                        sc.trig_list
                            .iter()
                            .filter(|t| trig_ok(t, MTRIG_MEMORY))
                            .map(|t| (t.iid, t.narg))
                            .collect()
                    })
                    .unwrap_or_default();
                for (iid, narg) in snapshot {
                    if g.trig(GoId::Char(chid), iid).is_none() {
                        continue;
                    }
                    if g.rng.rand_number(1, 100) <= narg {
                        let actor_uid = uid_var(actor_id);
                        if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                            add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                        }
                        script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
                        break;
                    }
                }
            }
            // delete the memory
            if let Some(c) = g.chars.get_mut(chid) {
                if let Some(pos) =
                    c.script_mem.iter().position(|m| m.id == mem.id && m.cmd == mem.cmd)
                {
                    c.script_mem.remove(pos);
                }
            }
        }
    }
}

pub fn entry_mtrigger(g: &mut Game, chid: CharId) -> i32 {
    if !g.script_check(GoId::Char(chid), MTRIG_ENTRY) || charmed(g, chid) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_ENTRY))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            return script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
        }
    }
    1
}

pub fn command_mtrigger(g: &mut Game, actor: CharId, cmd: &[u8], argument: &[u8]) -> bool {
    if !valid_dg_target(g, actor, 0) {
        return false;
    }
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return false;
    }
    let people = g.rooms[room as usize].people.clone();
    for chid in people {
        if g.try_ch(chid).is_none() {
            continue;
        }
        if !g.script_check(GoId::Char(chid), MTRIG_COMMAND)
            || charmed(g, chid)
            || (actor == chid && !g.config.script_players)
        {
            continue;
        }
        let snapshot: Vec<(u64, BStr)> = g
            .script_of(GoId::Char(chid))
            .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone())).collect())
            .unwrap_or_default();
        for (iid, arglist) in snapshot {
            let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
            if !(t.trigger_type & MTRIG_COMMAND != 0 && t.depth == 0) {
                continue;
            }
            if arglist.is_empty() {
                let vnum = super::trig_vnum(g, GoId::Char(chid), iid);
                g.mudlog(
                    MudlogKind::Nrm,
                    LVL_BUILDER,
                    true,
                    &format!("SYSERR: Command Trigger #{} has no text argument!", vnum),
                );
                continue;
            }
            let matches = arglist.first() == Some(&b'*')
                || (arglist.len() <= cmd.len()
                    && arglist.eq_ignore_ascii_case(&cmd[..arglist.len()]));
            if matches {
                let actor_uid = uid_var(char_script_id(g, actor));
                let arg_trim = crate::interpreter::skip_spaces(argument).to_vec();
                let cmd_trim = crate::interpreter::skip_spaces(cmd).to_vec();
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                    add_var(&mut t.var_list, b"arg", &arg_trim, 0);
                    add_var(&mut t.var_list, b"cmd", &cmd_trim, 0);
                }
                if script_driver(g, GoId::Char(chid), iid, TRIG_NEW) != 0 {
                    return true;
                }
            }
        }
    }
    false
}

pub fn speech_mtrigger(g: &mut Game, actor: CharId, str_: &[u8]) {
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return;
    }
    let people = g.rooms[room as usize].people.clone();
    for chid in people {
        let Some(ch) = g.try_ch(chid) else { continue };
        if !g.script_check(GoId::Char(chid), MTRIG_SPEECH)
            || !ch.awake()
            || charmed(g, chid)
            || (actor == chid && !g.config.script_players)
        {
            continue;
        }
        let snapshot: Vec<(u64, BStr, i32)> = g
            .script_of(GoId::Char(chid))
            .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone(), t.narg)).collect())
            .unwrap_or_default();
        for (iid, arglist, narg) in snapshot {
            let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
            if !(t.trigger_type & MTRIG_SPEECH != 0 && t.depth == 0) {
                continue;
            }
            if arglist.is_empty() {
                let vnum = super::trig_vnum(g, GoId::Char(chid), iid);
                g.mudlog(
                    MudlogKind::Nrm,
                    LVL_BUILDER,
                    true,
                    &format!("SYSERR: Speech Trigger #{} has no text argument!", vnum),
                );
                continue;
            }
            let hit = (narg != 0 && word_check(str_, &arglist))
                || (narg == 0 && is_substring(&arglist, str_));
            if hit {
                let actor_uid = uid_var(char_script_id(g, actor));
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                    add_var(&mut t.var_list, b"speech", str_, 0);
                }
                script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
                break;
            }
        }
    }
}

/// act_mtrigger. `str_` includes the trailing \r\n.
pub fn act_mtrigger(
    g: &mut Game,
    chid: CharId,
    str_: &[u8],
    actor: Option<CharId>,
    victim: Option<CharId>,
    object: Option<ObjId>,
    target: Option<ObjId>,
    arg: Option<&[u8]>,
) {
    if !g.script_check(GoId::Char(chid), MTRIG_ACT) || charmed(g, chid) {
        return;
    }
    if actor == Some(chid) {
        return;
    }
    let snapshot: Vec<(u64, BStr, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone(), t.narg)).collect())
        .unwrap_or_default();
    for (iid, arglist, narg) in snapshot {
        let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
        if !(t.trigger_type & MTRIG_ACT != 0 && t.depth == 0) {
            continue;
        }
        if arglist.is_empty() {
            let vnum = super::trig_vnum(g, GoId::Char(chid), iid);
            g.mudlog(
                MudlogKind::Nrm,
                LVL_BUILDER,
                true,
                &format!("SYSERR: Act Trigger #{} has no text argument!", vnum),
            );
            continue;
        }
        let hit = (narg != 0 && word_check(str_, &arglist))
            || (narg == 0 && is_substring(&arglist, str_));
        if hit {
            let mut vars: Vec<(&'static [u8], BStr)> = Vec::new();
            if let Some(a) = actor {
                vars.push((b"actor", uid_var(char_script_id(g, a))));
            }
            if let Some(v) = victim {
                vars.push((b"victim", uid_var(char_script_id(g, v))));
            }
            if let Some(o) = object {
                vars.push((b"object", uid_var(obj_script_id(g, o))));
            }
            if let Some(tg) = target {
                vars.push((b"target", uid_var(obj_script_id(g, tg))));
            }
            {
                // arg: leading spaces skipped, cut at first \r.
                let src = arg.unwrap_or(str_);
                let cut = src.iter().position(|&b| b == b'\r').unwrap_or(src.len());
                let trimmed = crate::interpreter::skip_spaces(&src[..cut]).to_vec();
                vars.push((b"arg", trimmed));
            }
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                for (name, value) in vars {
                    add_var(&mut t.var_list, name, &value, 0);
                }
            }
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn fight_mtrigger(g: &mut Game, chid: CharId) {
    if !g.script_check(GoId::Char(chid), MTRIG_FIGHT)
        || g.ch(chid).fighting.is_none()
        || charmed(g, chid)
    {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_FIGHT))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor = g.ch(chid).fighting;
            let var: (&'static [u8], BStr) = match actor {
                Some(a) => (b"actor", uid_var(char_script_id(g, a))),
                None => (b"actor", b"nobody".to_vec()),
            };
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, var.0, &var.1, 0);
            }
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn hitprcnt_mtrigger(g: &mut Game, chid: CharId) {
    if !g.script_check(GoId::Char(chid), MTRIG_HITPRCNT)
        || g.ch(chid).fighting.is_none()
        || charmed(g, chid)
    {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_HITPRCNT))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        let (hit, max_hit) = {
            let p = &g.ch(chid).points;
            (p.hit, p.max_hit)
        };
        if max_hit != 0 && (hit * 100) / max_hit <= narg {
            let actor = g.ch(chid).fighting.expect("guarded above");
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn receive_mtrigger(g: &mut Game, chid: CharId, actor: CharId, obj: ObjId) -> i32 {
    if !g.script_check(GoId::Char(chid), MTRIG_RECEIVE) || charmed(g, chid) {
        return 1;
    }
    let object_id = obj_script_id(g, obj);
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_RECEIVE))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            let obj_uid = uid_var(object_id);
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"object", &obj_uid, 0);
            }
            let ret_val = script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            if dead(g, actor)
                || dead(g, chid)
                || !has_obj_by_uid_in_lookup_table(g, object_id)
                || g.try_obj(obj).map(|o| o.carried_by) != Some(Some(actor))
            {
                return 0;
            }
            return ret_val;
        }
    }
    1
}

pub fn death_mtrigger(g: &mut Game, chid: CharId, actor: Option<CharId>) -> i32 {
    if !g.script_check(GoId::Char(chid), MTRIG_DEATH) || charmed(g, chid) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_DEATH))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            if let Some(a) = actor {
                let actor_uid = uid_var(char_script_id(g, a));
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                }
            }
            return script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
        }
    }
    1
}

pub fn load_mtrigger(g: &mut Game, chid: CharId) {
    if !g.script_check(GoId::Char(chid), MTRIG_LOAD) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_LOAD))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    let mut result = 0;
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            result = script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
    if result == SCRIPT_ERROR_CODE {
        // Strip the PROTOTYPE's script to break load loops.
        let rnum = g.ch(chid).mob_rnum;
        if rnum != NOBODY {
            if let Some(p) = g.world.mob_protos.get_mut(rnum as usize) {
                p.proto_script.clear();
            }
        }
    }
}

pub fn cast_mtrigger(g: &mut Game, actor: CharId, chid: CharId, spellnum: i32) -> i32 {
    if !g.script_check(GoId::Char(chid), MTRIG_CAST) || charmed(g, chid) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_CAST))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"spell", spellnum.to_string().as_bytes(), 0);
                add_var(&mut t.var_list, b"spellname", skill_name_b(spellnum), 0);
            }
            return script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
        }
    }
    1
}

/// damage_mtrigger: the driver's return REPLACES the damage.
pub fn damage_mtrigger(g: &mut Game, actor: CharId, victim: CharId, dam: i32, attacktype: i32) -> i32 {
    if !g.script_check(GoId::Char(victim), MTRIG_DAMAGE) || charmed(g, victim) {
        return dam;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(victim))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_DAMAGE))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(victim), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            let victim_uid = uid_var(char_script_id(g, victim));
            if let Some(t) = g.trig_mut(GoId::Char(victim), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"victim", &victim_uid, 0);
                // The amount is `damdealt`. Naming it `damage` shadowed
                // the `%damage%` command alias inside the one trigger type
                // most likely to want it, so `%damage% %actor% %damage%`
                // expanded to a garbage no-op line.
                add_var(&mut t.var_list, b"damdealt", dam.to_string().as_bytes(), 0);
                add_var(&mut t.var_list, b"attacktype", skill_name_b(attacktype), 0);
            }
            // No `return` in the script leaves the hit as it was.
            return script_driver_default(g, GoId::Char(victim), iid, TRIG_NEW, dam);
        }
    }
    dam
}

pub fn leave_mtrigger(g: &mut Game, actor: CharId, dir: i32) -> i32 {
    if !valid_dg_target(g, actor, DG_ALLOW_GODS) {
        return 1;
    }
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return 1;
    }
    let dir_count = crate::fight::dir_count(g) as i32;
    let people = g.rooms[room as usize].people.clone();
    for chid in people {
        let Some(ch) = g.try_ch(chid) else { continue };
        if !g.script_check(GoId::Char(chid), MTRIG_LEAVE)
            || !ch.awake()
            || ch.fighting.is_some()
            || chid == actor
            || ch.aff(flags::AFF_CHARM)
        {
            continue;
        }
        let snapshot: Vec<(u64, u32, i32)> = g
            .script_of(GoId::Char(chid))
            .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.trigger_type, t.narg)).collect())
            .unwrap_or_default();
        for (iid, ttype, narg) in snapshot {
            let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
            if !(ttype & MTRIG_LEAVE != 0 && can_see(g, chid, actor)) || t.depth != 0 {
                continue;
            }
            if g.rng.rand_number(1, 100) <= narg {
                let dir_val: BStr = if dir >= 0 && dir < dir_count {
                    DIRS[dir as usize].as_bytes().to_vec()
                } else {
                    b"none".to_vec()
                };
                let actor_uid = uid_var(char_script_id(g, actor));
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"direction", &dir_val, 0);
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                }
                return script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            }
        }
    }
    1
}

pub fn door_mtrigger(g: &mut Game, actor: CharId, subcmd: i32, dir: i32) -> i32 {
    let room = g.ch(actor).in_room;
    if room == NOWHERE {
        return 1;
    }
    const CMD_DOOR: [&[u8]; 5] = [b"open", b"close", b"unlock", b"lock", b"pick"];
    let dir_count = crate::fight::dir_count(g) as i32;
    let people = g.rooms[room as usize].people.clone();
    for chid in people {
        let Some(ch) = g.try_ch(chid) else { continue };
        if !g.script_check(GoId::Char(chid), MTRIG_DOOR)
            || !ch.awake()
            || ch.fighting.is_some()
            || chid == actor
            || ch.aff(flags::AFF_CHARM)
        {
            continue;
        }
        let snapshot: Vec<(u64, u32, i32)> = g
            .script_of(GoId::Char(chid))
            .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.trigger_type, t.narg)).collect())
            .unwrap_or_default();
        for (iid, ttype, narg) in snapshot {
            let Some(t) = g.trig(GoId::Char(chid), iid) else { continue };
            if ttype & MTRIG_DOOR == 0 || !can_see(g, chid, actor) || t.depth != 0 {
                continue;
            }
            if g.rng.rand_number(1, 100) <= narg {
                let dir_val: BStr = if dir >= 0 && dir < dir_count {
                    DIRS[dir as usize].as_bytes().to_vec()
                } else {
                    b"none".to_vec()
                };
                let actor_uid = uid_var(char_script_id(g, actor));
                if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                    add_var(&mut t.var_list, b"cmd", CMD_DOOR[subcmd as usize], 0);
                    add_var(&mut t.var_list, b"direction", &dir_val, 0);
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                }
                return script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            }
        }
    }
    1
}

pub fn time_mtrigger(g: &mut Game, chid: CharId) {
    if !g.script_check(GoId::Char(chid), MTRIG_TIME) || charmed(g, chid) {
        return;
    }
    let hours = g.time_info.hours;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Char(chid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, MTRIG_TIME))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Char(chid), iid).is_none() {
            continue;
        }
        if hours as i32 == narg {
            if let Some(t) = g.trig_mut(GoId::Char(chid), iid) {
                add_var(&mut t.var_list, b"time", hours.to_string().as_bytes(), 0);
            }
            script_driver(g, GoId::Char(chid), iid, TRIG_NEW);
            break;
        }
    }
}

// ---- Object triggers ----

pub fn random_otrigger(g: &mut Game, oid: ObjId) {
    if !g.script_check(GoId::Obj(oid), OTRIG_RANDOM) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_RANDOM))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            break;
        }
    }
}

/// timer_otrigger: ALL timer triggers run (no break).
pub fn timer_otrigger(g: &mut Game, oid: ObjId) {
    if !g.script_check(GoId::Obj(oid), OTRIG_TIMER) {
        return;
    }
    let snapshot: Vec<u64> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| sc.trig_list.iter().filter(|t| trig_ok(t, OTRIG_TIMER)).map(|t| t.iid).collect())
        .unwrap_or_default();
    for iid in snapshot {
        if g.try_obj(oid).is_none() {
            break;
        }
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
    }
}

pub fn get_otrigger(g: &mut Game, oid: ObjId, actor: CharId) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_GET) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_GET))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            if dead(g, actor) || g.try_obj(oid).is_none() {
                return 0;
            }
            return ret_val;
        }
    }
    1
}

fn cmd_otrig(g: &mut Game, oid: ObjId, actor: CharId, cmd: &[u8], argument: &[u8], type_: i32) -> bool {
    if g.try_obj(oid).is_none() || !g.script_check(GoId::Obj(oid), OTRIG_COMMAND) {
        return false;
    }
    let snapshot: Vec<(u64, BStr, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone(), t.narg)).collect())
        .unwrap_or_default();
    for (iid, arglist, narg) in snapshot {
        let Some(t) = g.trig(GoId::Obj(oid), iid) else { continue };
        if !(t.trigger_type & OTRIG_COMMAND != 0 && t.depth == 0) {
            continue;
        }
        if narg & type_ != 0 && arglist.is_empty() {
            let vnum = super::trig_vnum(g, GoId::Obj(oid), iid);
            g.mudlog(
                MudlogKind::Nrm,
                LVL_BUILDER,
                true,
                &format!("SYSERR: O-Command Trigger #{} has no text argument!", vnum),
            );
            continue;
        }
        let matches = narg & type_ != 0
            && (arglist.first() == Some(&b'*')
                || (arglist.len() <= cmd.len()
                    && arglist.eq_ignore_ascii_case(&cmd[..arglist.len()])));
        if matches {
            let actor_uid = uid_var(char_script_id(g, actor));
            let arg_trim = crate::interpreter::skip_spaces(argument).to_vec();
            let cmd_trim = crate::interpreter::skip_spaces(cmd).to_vec();
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"arg", &arg_trim, 0);
                add_var(&mut t.var_list, b"cmd", &cmd_trim, 0);
            }
            if script_driver(g, GoId::Obj(oid), iid, TRIG_NEW) != 0 {
                return true;
            }
        }
    }
    false
}

pub fn command_otrigger(g: &mut Game, actor: CharId, cmd: &[u8], argument: &[u8]) -> bool {
    if !valid_dg_target(g, actor, 0) {
        return false;
    }
    for i in 0..NUM_WEARS {
        if let Some(eq) = g.ch(actor).equipment[i] {
            if cmd_otrig(g, eq, actor, cmd, argument, super::OCMD_EQUIP) {
                return true;
            }
        }
    }
    let carrying = g.ch(actor).carrying.clone();
    for oid in carrying {
        if cmd_otrig(g, oid, actor, cmd, argument, super::OCMD_INVEN) {
            return true;
        }
    }
    let room = g.ch(actor).in_room;
    if room != NOWHERE {
        let contents = g.rooms[room as usize].contents.clone();
        for oid in contents {
            if cmd_otrig(g, oid, actor, cmd, argument, super::OCMD_ROOM) {
                return true;
            }
        }
    }
    false
}

pub fn wear_otrigger(g: &mut Game, oid: ObjId, actor: CharId, _where: i32) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_WEAR) {
        return 1;
    }
    let snapshot: Vec<u64> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| sc.trig_list.iter().filter(|t| trig_ok(t, OTRIG_WEAR)).map(|t| t.iid).collect())
        .unwrap_or_default();
    for iid in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        let actor_uid = uid_var(char_script_id(g, actor));
        if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
            add_var(&mut t.var_list, b"actor", &actor_uid, 0);
        }
        let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
        if g.try_obj(oid).is_none() {
            return 0;
        }
        return ret_val;
    }
    1
}

pub fn remove_otrigger(g: &mut Game, oid: ObjId, actor: CharId) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_REMOVE) {
        return 1;
    }
    if !valid_dg_target(g, actor, 0) {
        return 1;
    }
    let snapshot: Vec<u64> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| sc.trig_list.iter().filter(|t| trig_ok(t, OTRIG_REMOVE)).map(|t| t.iid).collect())
        .unwrap_or_default();
    for iid in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        let actor_uid = uid_var(char_script_id(g, actor));
        if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
            add_var(&mut t.var_list, b"actor", &actor_uid, 0);
        }
        let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
        if g.try_obj(oid).is_none() {
            return 0;
        }
        return ret_val;
    }
    1
}

pub fn drop_otrigger(g: &mut Game, oid: ObjId, actor: CharId) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_DROP) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_DROP))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            if g.try_obj(oid).is_none() {
                return 0;
            }
            return ret_val;
        }
    }
    1
}

pub fn give_otrigger(g: &mut Game, oid: ObjId, actor: CharId, victim: CharId) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_GIVE) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_GIVE))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            let victim_uid = uid_var(char_script_id(g, victim));
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"victim", &victim_uid, 0);
            }
            let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            if g.try_obj(oid).is_none() || g.obj(oid).carried_by != Some(actor) {
                return 0;
            }
            return ret_val;
        }
    }
    1
}

pub fn load_otrigger(g: &mut Game, oid: ObjId) {
    if !g.script_check(GoId::Obj(oid), OTRIG_LOAD) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_LOAD))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    let mut result = 0;
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            result = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            break;
        }
    }
    if result == SCRIPT_ERROR_CODE {
        if let Some(o) = g.try_obj(oid) {
            let rnum = o.item_number;
            if rnum != NOTHING {
                if let Some(p) = g.world.obj_protos.get_mut(rnum as usize) {
                    p.proto_script.clear();
                }
            }
        }
    }
}

pub fn cast_otrigger(g: &mut Game, actor: CharId, oid: ObjId, spellnum: i32) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_CAST) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_CAST))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"spell", spellnum.to_string().as_bytes(), 0);
                add_var(&mut t.var_list, b"spellname", skill_name_b(spellnum), 0);
            }
            return script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
        }
    }
    1
}

/// leave_otrigger: ALL matching triggers on ALL floor objects, ANDed.
pub fn leave_otrigger(g: &mut Game, room: RoomRnum, actor: CharId, dir: i32) -> i32 {
    if !valid_dg_target(g, actor, DG_ALLOW_GODS) {
        return 1;
    }
    let mut final_ = 1;
    let dir_count = crate::fight::dir_count(g) as i32;
    let contents = g.rooms[room as usize].contents.clone();
    for oid in contents {
        if g.try_obj(oid).is_none() || !g.script_check(GoId::Obj(oid), OTRIG_LEAVE) {
            continue;
        }
        let snapshot: Vec<(u64, i32)> = g
            .script_of(GoId::Obj(oid))
            .map(|sc| {
                sc.trig_list
                    .iter()
                    .filter(|t| trig_ok(t, OTRIG_LEAVE))
                    .map(|t| (t.iid, t.narg))
                    .collect()
            })
            .unwrap_or_default();
        for (iid, narg) in snapshot {
            if g.trig(GoId::Obj(oid), iid).is_none() {
                continue;
            }
            if g.rng.rand_number(1, 100) <= narg {
                let dir_val: BStr = if dir >= 0 && dir < dir_count {
                    DIRS[dir as usize].as_bytes().to_vec()
                } else {
                    b"none".to_vec()
                };
                let actor_uid = uid_var(char_script_id(g, actor));
                if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                    add_var(&mut t.var_list, b"direction", &dir_val, 0);
                    add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                }
                let temp = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
                if temp == 0 {
                    final_ = 0;
                }
            }
        }
    }
    final_
}

pub fn consume_otrigger(g: &mut Game, oid: ObjId, actor: CharId, cmd: i32) -> i32 {
    if !g.script_check(GoId::Obj(oid), OTRIG_CONSUME) {
        return 1;
    }
    let snapshot: Vec<u64> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| sc.trig_list.iter().filter(|t| trig_ok(t, OTRIG_CONSUME)).map(|t| t.iid).collect())
        .unwrap_or_default();
    for iid in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        let actor_uid = uid_var(char_script_id(g, actor));
        let cmd_name: &'static [u8] = if cmd == OCMD_EAT {
            b"eat"
        } else if cmd == OCMD_DRINK {
            b"drink"
        } else if cmd == OCMD_QUAFF {
            b"quaff"
        } else {
            b""
        };
        if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
            add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            if !cmd_name.is_empty() {
                add_var(&mut t.var_list, b"command", cmd_name, 0);
            }
        }
        let ret_val = script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
        if g.try_obj(oid).is_none() {
            return 0;
        }
        return ret_val;
    }
    1
}

pub fn time_otrigger(g: &mut Game, oid: ObjId) {
    if !g.script_check(GoId::Obj(oid), OTRIG_TIME) {
        return;
    }
    let hours = g.time_info.hours;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Obj(oid))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, OTRIG_TIME))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Obj(oid), iid).is_none() {
            continue;
        }
        if hours as i32 == narg {
            if let Some(t) = g.trig_mut(GoId::Obj(oid), iid) {
                add_var(&mut t.var_list, b"time", hours.to_string().as_bytes(), 0);
            }
            script_driver(g, GoId::Obj(oid), iid, TRIG_NEW);
            break;
        }
    }
}

// ---- World triggers ----

pub fn reset_wtrigger(g: &mut Game, room: RoomRnum) {
    if !g.script_check(GoId::Room(room), WTRIG_RESET) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_RESET))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            script_driver(g, GoId::Room(room), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn random_wtrigger(g: &mut Game, room: RoomRnum) {
    if !g.script_check(GoId::Room(room), WTRIG_RANDOM) {
        return;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_RANDOM))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            script_driver(g, GoId::Room(room), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn enter_wtrigger(g: &mut Game, room: RoomRnum, actor: CharId, dir: i32) -> i32 {
    if !g.script_check(GoId::Room(room), WTRIG_ENTER) {
        return 1;
    }
    let dir_count = crate::fight::dir_count(g) as i32;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_ENTER))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let dir_val: BStr = if dir >= 0 && dir < dir_count {
                DIRS[rev_dir(dir as usize)].as_bytes().to_vec()
            } else {
                b"none".to_vec()
            };
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"direction", &dir_val, 0);
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW);
        }
    }
    1
}

/// command_wtrigger: the room runs only its FIRST matching trigger.
pub fn command_wtrigger(g: &mut Game, actor: CharId, cmd: &[u8], argument: &[u8]) -> bool {
    let room = g.ch(actor).in_room;
    if room == NOWHERE || !g.script_check(GoId::Room(room), WTRIG_COMMAND) {
        return false;
    }
    if !valid_dg_target(g, actor, 0) {
        return false;
    }
    let snapshot: Vec<(u64, BStr)> = g
        .script_of(GoId::Room(room))
        .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone())).collect())
        .unwrap_or_default();
    for (iid, arglist) in snapshot {
        let Some(t) = g.trig(GoId::Room(room), iid) else { continue };
        if !(t.trigger_type & WTRIG_COMMAND != 0 && t.depth == 0) {
            continue;
        }
        if arglist.is_empty() {
            let vnum = super::trig_vnum(g, GoId::Room(room), iid);
            g.mudlog(
                MudlogKind::Nrm,
                LVL_BUILDER,
                true,
                &format!("SYSERR: W-Command Trigger #{} has no text argument!", vnum),
            );
            continue;
        }
        let matches = arglist.first() == Some(&b'*')
            || (arglist.len() <= cmd.len() && arglist.eq_ignore_ascii_case(&cmd[..arglist.len()]));
        if matches {
            let actor_uid = uid_var(char_script_id(g, actor));
            let arg_trim = crate::interpreter::skip_spaces(argument).to_vec();
            let cmd_trim = crate::interpreter::skip_spaces(cmd).to_vec();
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"arg", &arg_trim, 0);
                add_var(&mut t.var_list, b"cmd", &cmd_trim, 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW) != 0;
        }
    }
    false
}

pub fn speech_wtrigger(g: &mut Game, actor: CharId, str_: &[u8]) {
    let room = g.ch(actor).in_room;
    if room == NOWHERE || !g.script_check(GoId::Room(room), WTRIG_SPEECH) {
        return;
    }
    let snapshot: Vec<(u64, BStr, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| sc.trig_list.iter().map(|t| (t.iid, t.arglist.clone(), t.narg)).collect())
        .unwrap_or_default();
    for (iid, arglist, narg) in snapshot {
        let Some(t) = g.trig(GoId::Room(room), iid) else { continue };
        if !(t.trigger_type & WTRIG_SPEECH != 0 && t.depth == 0) {
            continue;
        }
        if arglist.is_empty() {
            let vnum = super::trig_vnum(g, GoId::Room(room), iid);
            g.mudlog(
                MudlogKind::Nrm,
                LVL_BUILDER,
                true,
                &format!("SYSERR: W-Speech Trigger #{} has no text argument!", vnum),
            );
            continue;
        }
        let hit = arglist.first() == Some(&b'*')
            || (narg != 0 && word_check(str_, &arglist))
            || (narg == 0 && is_substring(&arglist, str_));
        if hit {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"speech", str_, 0);
            }
            script_driver(g, GoId::Room(room), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn drop_wtrigger(g: &mut Game, oid: ObjId, actor: CharId) -> i32 {
    let room = g.ch(actor).in_room;
    if room == NOWHERE || !g.script_check(GoId::Room(room), WTRIG_DROP) {
        return 1;
    }
    let object_id = obj_script_id(g, oid);
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_DROP))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            let obj_uid = uid_var(object_id);
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                add_var(&mut t.var_list, b"object", &obj_uid, 0);
            }
            let ret_val = script_driver(g, GoId::Room(room), iid, TRIG_NEW);
            if !has_obj_by_uid_in_lookup_table(g, object_id)
                || g.try_obj(oid).map(|o| o.carried_by) != Some(Some(actor))
            {
                return 0;
            }
            return ret_val;
        }
    }
    1
}

pub fn cast_wtrigger(
    g: &mut Game,
    actor: CharId,
    vict: Option<CharId>,
    oid: Option<ObjId>,
    spellnum: i32,
) -> i32 {
    let room = g.ch(actor).in_room;
    if room == NOWHERE || !g.script_check(GoId::Room(room), WTRIG_CAST) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_CAST))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            let vict_uid = vict.map(|v| uid_var(char_script_id(g, v)));
            let obj_uid = oid.map(|o| uid_var(obj_script_id(g, o)));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
                if let Some(v) = vict_uid {
                    add_var(&mut t.var_list, b"victim", &v, 0);
                }
                if let Some(o) = obj_uid {
                    add_var(&mut t.var_list, b"object", &o, 0);
                }
                add_var(&mut t.var_list, b"spell", spellnum.to_string().as_bytes(), 0);
                add_var(&mut t.var_list, b"spellname", skill_name_b(spellnum), 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW);
        }
    }
    1
}

pub fn leave_wtrigger(g: &mut Game, room: RoomRnum, actor: CharId, dir: i32) -> i32 {
    if !valid_dg_target(g, actor, DG_ALLOW_GODS) {
        return 1;
    }
    if !g.script_check(GoId::Room(room), WTRIG_LEAVE) {
        return 1;
    }
    let dir_count = crate::fight::dir_count(g) as i32;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_LEAVE))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let dir_val: BStr = if dir >= 0 && dir < dir_count {
                DIRS[dir as usize].as_bytes().to_vec()
            } else {
                b"none".to_vec()
            };
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"direction", &dir_val, 0);
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW);
        }
    }
    1
}

pub fn door_wtrigger(g: &mut Game, actor: CharId, subcmd: i32, dir: i32) -> i32 {
    let room = g.ch(actor).in_room;
    if room == NOWHERE || !g.script_check(GoId::Room(room), WTRIG_DOOR) {
        return 1;
    }
    const CMD_DOOR: [&[u8]; 5] = [b"open", b"close", b"unlock", b"lock", b"pick"];
    let dir_count = crate::fight::dir_count(g) as i32;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_DOOR))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let dir_val: BStr = if dir >= 0 && dir < dir_count {
                DIRS[dir as usize].as_bytes().to_vec()
            } else {
                b"none".to_vec()
            };
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"cmd", CMD_DOOR[subcmd as usize], 0);
                add_var(&mut t.var_list, b"direction", &dir_val, 0);
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW);
        }
    }
    1
}

pub fn time_wtrigger(g: &mut Game, room: RoomRnum) {
    if !g.script_check(GoId::Room(room), WTRIG_TIME) {
        return;
    }
    let hours = g.time_info.hours;
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_TIME))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if hours as i32 == narg {
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"time", hours.to_string().as_bytes(), 0);
            }
            script_driver(g, GoId::Room(room), iid, TRIG_NEW);
            break;
        }
    }
}

pub fn login_wtrigger(g: &mut Game, room: RoomRnum, actor: CharId) -> i32 {
    if !g.script_check(GoId::Room(room), WTRIG_LOGIN) {
        return 1;
    }
    let snapshot: Vec<(u64, i32)> = g
        .script_of(GoId::Room(room))
        .map(|sc| {
            sc.trig_list
                .iter()
                .filter(|t| trig_ok(t, WTRIG_LOGIN))
                .map(|t| (t.iid, t.narg))
                .collect()
        })
        .unwrap_or_default();
    for (iid, narg) in snapshot {
        if g.trig(GoId::Room(room), iid).is_none() {
            continue;
        }
        if g.rng.rand_number(1, 100) <= narg {
            let actor_uid = uid_var(char_script_id(g, actor));
            if let Some(t) = g.trig_mut(GoId::Room(room), iid) {
                add_var(&mut t.var_list, b"actor", &actor_uid, 0);
            }
            return script_driver(g, GoId::Room(room), iid, TRIG_NEW);
        }
    }
    1
}

// Periodic drivers ----

/// script_trigger_check: every PULSE_DG_SCRIPT (130 pulses).
pub fn script_trigger_check(g: &mut Game) {
    let chars = g.character_list.clone();
    for chid in chars {
        let Some(ch) = g.try_ch(chid) else { continue };
        let Some(sc) = ch.script.as_deref() else { continue };
        if sc.types & WTRIG_RANDOM != 0 {
            let room = ch.in_room;
            if room == NOWHERE {
                continue;
            }
            let zone = g.world.rooms[room as usize].zone;
            let global = g
                .script_of(GoId::Char(chid))
                .is_some_and(|sc| sc.types & super::WTRIG_GLOBAL != 0);
            if !crate::db::zone_is_empty(g, zone) || global {
                random_mtrigger(g, chid);
            }
        }
    }

    let objs = g.object_list.clone();
    for oid in objs {
        let Some(o) = g.try_obj(oid) else { continue };
        let Some(sc) = o.script.as_deref() else { continue };
        if sc.types & OTRIG_RANDOM != 0 {
            random_otrigger(g, oid);
        }
    }

    for nr in 0..g.rooms.len() {
        let Some(sc) = g.rooms[nr].script.as_deref() else { continue };
        if sc.types & WTRIG_RANDOM != 0 {
            let zone = g.world.rooms[nr].zone;
            let global = sc.types & super::WTRIG_GLOBAL != 0;
            if !crate::db::zone_is_empty(g, zone) || global {
                random_wtrigger(g, nr as RoomRnum);
            }
        }
    }
}

/// check_time_triggers: on every mud-hour tick.
pub fn check_time_triggers(g: &mut Game) {
    let chars = g.character_list.clone();
    for chid in chars {
        let Some(ch) = g.try_ch(chid) else { continue };
        let Some(sc) = ch.script.as_deref() else { continue };
        if sc.types & WTRIG_TIME != 0 {
            let room = ch.in_room;
            if room == NOWHERE {
                continue;
            }
            let zone = g.world.rooms[room as usize].zone;
            let global = g
                .script_of(GoId::Char(chid))
                .is_some_and(|sc| sc.types & super::WTRIG_GLOBAL != 0);
            if !crate::db::zone_is_empty(g, zone) || global {
                time_mtrigger(g, chid);
            }
        }
    }

    let objs = g.object_list.clone();
    for oid in objs {
        let Some(o) = g.try_obj(oid) else { continue };
        let Some(sc) = o.script.as_deref() else { continue };
        if sc.types & OTRIG_TIME != 0 {
            time_otrigger(g, oid);
        }
    }

    for nr in 0..g.rooms.len() {
        let Some(sc) = g.rooms[nr].script.as_deref() else { continue };
        if sc.types & WTRIG_TIME != 0 {
            let zone = g.world.rooms[nr].zone;
            let global = sc.types & super::WTRIG_GLOBAL != 0;
            if !crate::db::zone_is_empty(g, zone) || global {
                time_wtrigger(g, nr as RoomRnum);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_boundaries() {
        assert!(is_substring(b"hello", b"well hello there"));
        assert!(is_substring(b"hello", b"hello"));
        assert!(is_substring(b"hello", b"say 'hello'"));
        assert!(!is_substring(b"ell", b"well hello"));
        // Only the FIRST str_str hit is found: "he" inside "the" fails the
        // boundary check and the later standalone "he" is never seen.
        assert!(!is_substring(b"he", b"the he"));
        assert!(word_check(b"give me bread", b"bread water"));
        assert!(word_check(b"anything", b"*"));
        assert!(word_check(b"the magic word", b"\"magic word\""));
        assert!(!word_check(b"nothing here", b"bread water"));
    }
}
