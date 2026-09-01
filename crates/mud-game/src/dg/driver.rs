//! script_driver — plus the control-flow scanners
//! (find_end / find_else_end / find_case / find_done), the wait machinery,
//! and every built-in command processor.

use mud_data::types::{Idx, PASSES_PER_SEC, SECS_PER_MUD_HOUR};

use super::expr::{eval_expr, process_if};
use super::variables::var_subst;
use super::{
    add_var, atoi32, atoi64, extract_script, find_char, find_obj, find_room, line_state,
    read_trigger, remove_trigger, remove_var, script_log, trig_log, DgCtx, GoId,
    MAX_SCRIPT_DEPTH, SCRIPT_ERROR_CODE, TRIG_NEW, TRIG_RESTART, UID_CHAR,
};
use crate::game::{EventKind, Game};
use crate::handler::eq_ci;

pub type BStr = Vec<u8>;

const PULSES_PER_MUD_HOUR: i64 = (SECS_PER_MUD_HOUR * PASSES_PER_SEC) as i64;
const SECS_PER_MUD_DAY: i64 = 24 * SECS_PER_MUD_HOUR as i64;

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Case-insensitive prefix test.
fn has_prefix(line: &[u8], kw: &[u8]) -> bool {
    line.len() >= kw.len() && line[..kw.len()].eq_ignore_ascii_case(kw)
}

fn skip_ws(line: &[u8]) -> &[u8] {
    let i = line.iter().position(|&b| !is_ws(b)).unwrap_or(line.len());
    &line[i..]
}

/// Clone one prototype line (already whitespace-stripped by callers as
/// needed). None past the end.
fn raw_line(g: &Game, nr: Idx, idx: usize) -> Option<BStr> {
    g.world.triggers.get(nr as usize)?.cmdlist.get(idx).cloned()
}

fn line_count(g: &Game, nr: Idx) -> usize {
    g.world.triggers.get(nr as usize).map_or(0, |t| t.cmdlist.len())
}

/// find_end: index of the matching 'end' line, or the
/// last line, logging the error.
fn find_end(g: &mut Game, ctx: DgCtx, nr: Idx, cl: usize) -> usize {
    let vnum = g.world.triggers[nr as usize].vnum;
    let len = line_count(g, nr);
    if cl + 1 >= len {
        script_log(g, &format!("Trigger VNum {} has 'if' without 'end'. (error 1)", vnum));
        return cl;
    }
    let mut c = cl + 1;
    loop {
        let line = raw_line(g, nr, c).unwrap_or_default();
        let p = skip_ws(&line);
        if has_prefix(p, b"if ") {
            c = find_end(g, ctx, nr, c);
        } else if has_prefix(p, b"end") {
            return c;
        }
        if c + 1 >= len {
            script_log(g, &format!("Trigger VNum {} has 'if' without 'end'. (error 2)", vnum));
            return c;
        }
        c += 1;
    }
}

/// find_else_end: scan for elseif (evaluating it), else,
/// or end. Bumps trig depth when a branch is taken.
fn find_else_end(g: &mut Game, ctx: DgCtx, nr: Idx, cl: usize) -> usize {
    let vnum = g.world.triggers[nr as usize].vnum;
    let len = line_count(g, nr);
    if cl + 1 >= len {
        return cl;
    }
    let mut c = cl + 1;
    while c + 1 < len {
        let line = raw_line(g, nr, c).unwrap_or_default();
        let p = skip_ws(&line).to_vec();
        if has_prefix(&p, b"if ") {
            c = find_end(g, ctx, nr, c);
        } else if has_prefix(&p, b"elseif ") {
            if process_if(g, ctx, &p[7..]) {
                if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
                    t.depth += 1;
                }
                return c;
            }
        } else if has_prefix(&p, b"else") {
            if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
                t.depth += 1;
            }
            return c;
        } else if has_prefix(&p, b"end") {
            return c;
        }
        if c + 1 >= len {
            script_log(g, &format!("Trigger VNum {} has 'if' without 'end'. (error 4)", vnum));
            return c;
        }
        c += 1;
    }
    // Last line: silently fine if it's 'end', else error 5.
    let line = raw_line(g, nr, c).unwrap_or_default();
    let p = skip_ws(&line);
    if !has_prefix(p, b"end") {
        script_log(g, &format!("Trigger VNum {} has 'if' without 'end'. (error 5)", vnum));
    }
    c
}

/// The matching `done` for a `while`/`switch`, prefix-matched as "don".
///
/// Two non-obvious results. A block that simply runs off the end returns the
/// LAST line rather than None, so a single unterminated `while` is not an
/// error and the script runs on past it. None means a nested block consumed
/// the rest of the list, which is the only way an unclosed construct is
/// reported at all -- see B102 for what the caller must then do.
fn find_done(g: &Game, nr: Idx, cl: usize) -> Option<usize> {
    let len = line_count(g, nr);
    if len == 0 {
        return Some(cl);
    }
    if cl + 1 >= len {
        return Some(cl);
    }
    let mut c = cl + 1;
    loop {
        if c >= len {
            return None; // walked off the end without finding one
        }
        if c + 1 >= len {
            return Some(c); // the last line; nothing follows it to inspect
        }
        let line = raw_line(g, nr, c).unwrap_or_default();
        let p = skip_ws(&line);
        if has_prefix(p, b"while ") || has_prefix(p, b"switch ") {
            match find_done(g, nr, c) {
                Some(n) => c = n,
                None => return None,
            }
        } else if has_prefix(p, b"done") {
            return Some(c);
        }
        c += 1;
    }
}

fn find_case(g: &mut Game, ctx: DgCtx, nr: Idx, cl: usize, cond: &[u8]) -> usize {
    let result = eval_expr(g, ctx, cond);
    let len = line_count(g, nr);
    if cl + 1 >= len {
        return cl;
    }
    let mut c = cl + 1;
    while c + 1 < len {
        let line = raw_line(g, nr, c).unwrap_or_default();
        let p = skip_ws(&line).to_vec();
        if has_prefix(&p, b"while ") || has_prefix(&p, b"switch") {
            match find_done(g, nr, c) {
                Some(n) => c = n,
                None => return len.saturating_sub(1),
            }
        } else if has_prefix(&p, b"case ") {
            // NOTE: the case value is RAW text (no var_subst).
            let buf = super::expr::eval_op(b"==", &result, &p[5..]);
            if !buf.is_empty() && buf[0] != b'0' {
                return c;
            }
        } else if has_prefix(&p, b"default") {
            return c;
        } else if has_prefix(&p, b"done") {
            return c;
        }
        c += 1;
    }
    c
}

/// process_wait. `cl` = the line index whose `next`
/// becomes curr_state. Returns true if a wait was scheduled.
pub fn process_wait(g: &mut Game, ctx: DgCtx, cmd: &[u8], cl: usize, raw_cl_cmd: &[u8]) -> bool {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let arg = skip_ws(rest);
    if arg.is_empty() {
        let msg = format!("wait w/o an arg: '{}'", String::from_utf8_lossy(raw_cl_cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return false;
    }

    let mut when: i64;
    if has_prefix(arg, b"until ") {
            let after = skip_ws(&arg[5..]);
        let mut i = 0usize;
        let hr = scan_long(after, &mut i);
        let mut min: i64;
        if hr.is_some() && after.get(i) == Some(&b':') {
            i += 1;
            match scan_long(after, &mut i) {
                Some(m) => {
                    min = m;
                    min += hr.unwrap() * 60;
                }
                None => {
                    let h = hr.unwrap();
                    min = (h % 100) + (h / 100) * 60;
                }
            }
        } else {
            let h = hr.unwrap_or(0);
            min = (h % 100) + (h / 100) * 60;
        }
        let ntime = (min * PULSES_PER_MUD_HOUR) / 60;
        let pulse = g.pulse as i64;
        let hours = g.time_info.hours as i64;
        let mut w = (pulse % PULSES_PER_MUD_HOUR) + hours * PULSES_PER_MUD_HOUR;
        if w >= ntime {
            w = (SECS_PER_MUD_DAY * PASSES_PER_SEC as i64) - w + ntime;
        } else {
            w = ntime - w;
        }
        when = w;
    } else {
            let mut i = 0usize;
        match scan_long(arg, &mut i) {
            Some(v) => {
                when = v;
                // A format ' ' skips whitespace; %c reads the next char.
                while arg.get(i).copied().is_some_and(is_ws) {
                    i += 1;
                }
                match arg.get(i) {
                    Some(&b't') => when *= PULSES_PER_MUD_HOUR,
                    Some(&b's') => when *= PASSES_PER_SEC as i64,
                    _ => {}
                }
            }
            None => when = 0, // C reads uninitialized memory; 0 is our stand-in
        }
    }

    let event_id = {
        g.next_dg_event_id += 1;
        g.next_dg_event_id
    };
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        t.wait_event = Some(event_id);
        t.curr_state = cl + 1;
    }
    let fire_in = when.max(1) as u64;
    g.queue_event(fire_in, EventKind::TrigWait { go: ctx.go, iid: ctx.iid, event_id });
    true
}

/// An integer at the cursor: whitespace, optional sign, then digits.
fn scan_long(s: &[u8], i: &mut usize) -> Option<i64> {
    while s.get(*i).copied().is_some_and(is_ws) {
        *i += 1;
    }
    let mut j = *i;
    let neg = match s.get(j) {
        Some(b'-') => {
            j += 1;
            true
        }
        Some(b'+') => {
            j += 1;
            false
        }
        _ => false,
    };
    let start = j;
    let mut v: i64 = 0;
    while let Some(&c) = s.get(j) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        j += 1;
    }
    if j == start {
        return None;
    }
    *i = j;
    Some(if neg { -v } else { v })
}

/// The TrigWait event fired from the heartbeat: re-validate, then restart.
pub fn trig_wait_event(g: &mut Game, go: GoId, iid: u64, event_id: u64) {
    // event_cancel semantics: only a still-armed matching wait fires.
    let Some(t) = g.trig(go, iid) else { return };
    if t.wait_event != Some(event_id) {
        return;
    }
    if let Some(t) = g.trig_mut(go, iid) {
        t.wait_event = None;
    }
    // Arena liveness stands in for the debug existence scan, whose
    // top-room off-by-one is B19.
    if !g.go_alive(go) {
        return;
    }
    script_driver(g, go, iid, TRIG_RESTART);
}

/// process_eval — exported for var_subst's tmpvr chaining.
pub fn process_eval_line(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::one_argument(cmd);
    let (name, rest2) = crate::interpreter::one_argument(rest);
    let expr = skip_ws(rest2);
    if name.is_empty() {
        let msg = format!("eval w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let result = eval_expr(g, ctx, expr);
    let context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        add_var(&mut t.var_list, &name, &result, context);
    }
}

fn process_set(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, name, rest) = crate::interpreter::two_arguments(cmd);
    let value = skip_ws(rest);
    if name.is_empty() {
        let msg = format!("set w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    let value = value.to_vec();
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        add_var(&mut t.var_list, &name, &value, context);
    }
}

fn process_unset(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let var = skip_ws(rest);
    if var.is_empty() {
        let msg = format!("unset w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let var = var.to_vec();
    let removed_global = match g.script_of_mut(ctx.go) {
        Some(sc) => remove_var(&mut sc.global_vars, &var),
        None => false,
    };
    if !removed_global {
        if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
            remove_var(&mut t.var_list, &var);
        }
    }
}

fn process_global(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let var = skip_ws(rest);
    if var.is_empty() {
        let msg = format!("global w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let var = var.to_vec();
    let found = g
        .trig(ctx.go, ctx.iid)
        .and_then(|t| t.var_list.iter().find(|v| eq_ci(&v.name, &var)).cloned());
    let Some(vd) = found else {
        let msg = format!("local var '{}' not found in global call", String::from_utf8_lossy(&var));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };
    let id = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    if let Some(sc) = g.script_of_mut(ctx.go) {
        add_var(&mut sc.global_vars, &vd.name, &vd.value, id);
    }
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        remove_var(&mut t.var_list, &vd.name);
    }
}

fn process_context(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let var = skip_ws(rest);
    if var.is_empty() {
        let msg = format!("context w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let v = atoi64(var);
    if let Some(sc) = g.script_of_mut(ctx.go) {
        sc.context = v;
    }
}

fn process_rdelete(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let (var, uid_p, _) = crate::interpreter::two_arguments(&rest.to_vec());
    if var.is_empty() || uid_p.is_empty() {
        let msg = format!("rdelete: invalid arguments '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let uid = atoi64(&uid_p);
    if uid <= 0 {
        let msg = format!("rdelete: illegal uid '{}'", String::from_utf8_lossy(&uid_p));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let target = resolve_uid_entity(g, uid);
    let Some(tgt) = target else {
        // The remote message is reused here.
        let msg = format!("remote: uid '{}' invalid", uid);
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };
    let sc_context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    let Some(sc_remote) = g.script_of_mut(tgt) else { return };
    if sc_remote.global_vars.is_empty() {
        return;
    }
    if let Some(pos) = sc_remote
        .global_vars
        .iter()
        .position(|v| eq_ci(&v.name, &var) && (v.context == 0 || v.context == sc_context))
    {
        sc_remote.global_vars.remove(pos);
    }
}

fn process_remote(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::any_one_arg(cmd);
    let (var, uid_p, _) = crate::interpreter::two_arguments(&rest.to_vec());
    if var.is_empty() || uid_p.is_empty() {
        let msg = format!("remote: invalid arguments '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }

    // locals by name, then owner globals by name+context.
    let mut vd = g
        .trig(ctx.go, ctx.iid)
        .and_then(|t| t.var_list.iter().find(|v| eq_ci(&v.name, &var)).cloned());
    if vd.is_none() {
        if let Some(sc) = g.script_of(ctx.go) {
            let sc_context = sc.context;
            vd = sc
                .global_vars
                .iter()
                .find(|v| eq_ci(&v.name, &var) && (v.context == 0 || v.context == sc_context))
                .cloned();
        }
    }
    let Some(vd) = vd else {
        let msg = format!("local var '{}' not found in remote call", String::from_utf8_lossy(&var));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };

    let uid = atoi64(&uid_p);
    if uid <= 0 {
        let msg = format!("remote: illegal uid '{}'", String::from_utf8_lossy(&uid_p));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }

    let mut context = vd.context;
    let target = match resolve_uid_entity(g, uid) {
        Some(t) => t,
        None => {
            let msg = format!("remote: uid '{}' invalid", uid);
            trig_log(g, ctx.go, ctx.iid, &msg);
            return;
        }
    };
    if let GoId::Char(chid) = target {
        if !g.ch(chid).is_npc() {
            context = 0;
        }
    }
    let Some(sc_remote) = g.script_of_mut(target) else { return };
    add_var(&mut sc_remote.global_vars, &vd.name, &vd.value, context);
}

/// The room→char→obj UID resolution shared by remote/rdelete/vdelete —
/// The order is load-bearing: it decides which failed-lookup syslog line
/// comes out.
pub fn resolve_uid_entity(g: &mut Game, uid: i64) -> Option<GoId> {
    if let Some(r) = find_room(g, uid) {
        return Some(GoId::Room(r));
    }
    if let Some(c) = find_char(g, uid) {
        return Some(GoId::Char(c));
    }
    if let Some(o) = find_obj(g, uid) {
        return Some(GoId::Obj(o));
    }
    None
}

fn process_return(g: &mut Game, ctx: DgCtx, cmd: &[u8]) -> i32 {
    let (_, arg2, _) = crate::interpreter::two_arguments(cmd);
    if arg2.is_empty() {
        let msg = format!("return w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return 1;
    }
    atoi32(&arg2)
}

fn process_attach(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, trignum_s, rest) = crate::interpreter::two_arguments(cmd);
    let id_p = skip_ws(rest);
    if trignum_s.is_empty() {
        let msg = format!("attach w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    if id_p.is_empty() || atoi64(id_p) == 0 {
        let msg = format!("attach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let id_p = id_p.to_vec();
    let result = eval_expr(g, ctx, &id_p);
    let id = atoi64(&result);
    if id == 0 {
        let msg = format!("attach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let target = if let Some(c) = find_char(g, id) {
        Some(GoId::Char(c))
    } else if let Some(o) = find_obj(g, id) {
        Some(GoId::Obj(o))
    } else {
        find_room(g, id).map(GoId::Room)
    };
    let Some(target) = target else {
        let msg = format!("attach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };

    let trig_vnum = atoi32(&trignum_s);
    let rnum = if !(0..=65535).contains(&trig_vnum) {
        None
    } else {
        g.world.trig_map.get(&(trig_vnum as Idx)).copied()
    };
    let Some(rnum) = rnum else {
        let msg = format!("attach invalid trigger: '{}'", String::from_utf8_lossy(&trignum_s));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };

    if let GoId::Char(c) = target {
        if !g.ch(c).is_npc() && !g.config.script_players {
            let name = String::from_utf8_lossy(g.ch(c).get_name()).into_owned();
            let msg = format!("attach invalid target: '{}'", name);
            trig_log(g, ctx.go, ctx.iid, &msg);
            return;
        }
    }
    if let Some(newtrig) = read_trigger(g, rnum) {
        super::add_trigger_at(g.ensure_script(target), newtrig, -1);
    }
}

fn process_detach(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, trignum_s, rest) = crate::interpreter::two_arguments(cmd);
    let id_p = skip_ws(rest);
    if trignum_s.is_empty() {
        let msg = format!("detach w/o an arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    if id_p.is_empty() || atoi64(id_p) == 0 {
        let msg = format!("detach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let id_p = id_p.to_vec();
    let result = eval_expr(g, ctx, &id_p);
    let id = atoi64(&result);
    if id == 0 {
        let msg = format!("detach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    let target = if let Some(c) = find_char(g, id) {
        Some(GoId::Char(c))
    } else if let Some(o) = find_obj(g, id) {
        Some(GoId::Obj(o))
    } else {
        find_room(g, id).map(GoId::Room)
    };
    let Some(target) = target else {
        let msg = format!("detach invalid id arg: '{}'", String::from_utf8_lossy(cmd));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    };

    if g.script_of(target).is_none() {
        return;
    }
    if trignum_s == b"all" {
        extract_script(g, target);
        return;
    }
    if remove_trigger(g, target, &trignum_s) {
        let empty = g.script_of(target).is_some_and(|sc| sc.trig_list.is_empty());
        if empty {
            extract_script(g, target);
        }
    }
}

fn makeuid_var(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::half_chop(cmd);
    let (varname, rest) = crate::interpreter::half_chop(&rest);
    let (arg, rest) = crate::interpreter::half_chop(&rest);
    let (name, remainder) = crate::interpreter::half_chop(&rest);

    // The error messages print the final chopped-down remainder.
    if varname.is_empty() {
        let msg = format!("makeuid w/o an arg: '{}'", String::from_utf8_lossy(&remainder));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }
    if arg.is_empty() {
        let msg = format!("makeuid invalid id arg: '{}'", String::from_utf8_lossy(&remainder));
        trig_log(g, ctx.go, ctx.iid, &msg);
        return;
    }

    let mut uid: BStr = Vec::new();
    if atoi64(&arg) != 0 {
        let result = eval_expr(g, ctx, &arg);
        uid.push(UID_CHAR);
        uid.extend_from_slice(&result);
    } else {
        if name.is_empty() {
            let msg = format!("makeuid needs name: '{}'", String::from_utf8_lossy(&remainder));
            trig_log(g, ctx.go, ctx.iid, &msg);
            return;
        }
        if crate::handler::is_abbrev(&arg, b"mob") {
            let c = match ctx.go {
                GoId::Room(r) => super::get_char_in_room(g, r, &name),
                GoId::Obj(o) => super::get_char_near_obj(g, o, &name),
                GoId::Char(chid) => crate::handler::get_char_room_vis(g, chid, &name, None),
            };
            if let Some(c) = c {
                uid.push(UID_CHAR);
                uid.extend_from_slice(super::char_script_id(g, c).to_string().as_bytes());
            }
        } else if crate::handler::is_abbrev(&arg, b"obj") {
            let o = match ctx.go {
                GoId::Room(r) => super::get_obj_in_room(g, r, &name),
                GoId::Obj(oid) => super::get_obj_near_obj(g, oid, &name),
                GoId::Char(chid) => {
                    let inv = g.ch(chid).carrying.clone();
                    crate::handler::get_obj_in_list_vis(g, chid, &name, None, &inv).or_else(|| {
                        let room = g.ch(chid).in_room;
                        if room == mud_data::types::NOWHERE {
                            None
                        } else {
                            let contents = g.rooms[room as usize].contents.clone();
                            crate::handler::get_obj_in_list_vis(g, chid, &name, None, &contents)
                        }
                    })
                }
            };
            if let Some(o) = o {
                uid.push(UID_CHAR);
                uid.extend_from_slice(super::obj_script_id(g, o).to_string().as_bytes());
            }
        } else if crate::handler::is_abbrev(&arg, b"room") {
            let r = match ctx.go {
                GoId::Room(r) => Some(r),
                GoId::Obj(oid) => {
                    let rm = super::obj_room(g, oid);
                    (rm != mud_data::types::NOWHERE).then_some(rm)
                }
                GoId::Char(chid) => {
                    let rm = g.ch(chid).in_room;
                    (rm != mud_data::types::NOWHERE).then_some(rm)
                }
            };
            if let Some(r) = r {
                uid.push(UID_CHAR);
                uid.extend_from_slice(super::room_script_id(g, r).to_string().as_bytes());
            }
        } else {
            let msg = format!("makeuid syntax error: '{}'", String::from_utf8_lossy(&remainder));
            trig_log(g, ctx.go, ctx.iid, &msg);
            return;
        }
    }

    if !uid.is_empty() {
        let context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
        if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
            add_var(&mut t.var_list, &varname, &uid, context);
        }
    }
}

fn extract_value(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, buf3) = crate::interpreter::any_one_arg(cmd);
    let (to, mut buf) = crate::interpreter::half_chop(buf3);
    let num = atoi32(&buf);
    if num < 1 {
        script_log(g, "extract number < 1!");
        return;
    }
    // half_chop(buf, buf3, buf2): pop the count word.
    let (_count_word, rest) = crate::interpreter::half_chop(&buf);
    let mut buf2 = rest;
    let mut n = num;
    buf = Vec::new();
    while n > 0 {
        let (word, rest) = crate::interpreter::half_chop(&buf2);
        buf = word;
        buf2 = rest;
        n -= 1;
    }
    let context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        add_var(&mut t.var_list, &to, &buf, context);
    }
}

fn dg_letter_value(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::half_chop(cmd);
    let (varname, rest) = crate::interpreter::half_chop(&rest);
    let (num_s, string) = crate::interpreter::half_chop(&rest);
    let num = atoi32(&num_s);

    script_log(g, "The use of dg_letter is deprecated");
    script_log(g, "- Use 'set <new variable> %<text/var>.charat(index)% instead.");

    let vnum = super::trig_vnum(g, ctx.go, ctx.iid);
    if num < 1 {
        script_log(g, &format!("Trigger #{} : dg_letter number < 1!", vnum));
        return;
    }
    if num as usize > string.len() {
        script_log(g, &format!("Trigger #{} : dg_letter number > strlen!", vnum));
        return;
    }
    let letter = vec![string[num as usize - 1]];
    let context = g.script_of(ctx.go).map_or(0, |sc| sc.context);
    if let Some(t) = g.trig_mut(ctx.go, ctx.iid) {
        add_var(&mut t.var_list, &varname, &letter, context);
    }
}

pub fn script_driver(g: &mut Game, go: GoId, iid: u64, mode: i32) -> i32 {
    script_driver_default(g, go, iid, mode, 1)
}

/// As `script_driver`, but says what a script that never runs `return`
/// should yield.
///
/// `script_driver` starts at 1 and only overwrites it when a `return`
/// actually runs, and every trigger type but one reads that 1 as "allow".
/// The damage trigger reads its return as an AMOUNT, so a script with no
/// `return` -- the ordinary shape for one that merely reacts -- turned every
/// hit that fired it into 1 damage. The two cases are indistinguishable to
/// the caller, so the default has to come from the caller.
pub fn script_driver_default(
    g: &mut Game,
    go: GoId,
    iid: u64,
    mode: i32,
    default_ret: i32,
) -> i32 {
    let mut ret_val: i32 = default_ret;

    let Some(trig) = g.trig(go, iid) else { return ret_val };
    let nr = trig.nr;
    let ctx = DgCtx { go, iid };

    if g.dg_script_depth > MAX_SCRIPT_DEPTH {
        let vnum = g.world.triggers[nr as usize].vnum;
        script_log(g, &format!("Trigger {} recursed beyond maximum allowed depth.", vnum));
        let ident = match go {
            GoId::Char(c) => format!(
                "It was attached to {} [{}]",
                String::from_utf8_lossy(g.ch(c).get_name()),
                super::mob_vnum(g, c)
            ),
            GoId::Obj(o) => format!(
                "It was attached to {} [{}]",
                String::from_utf8_lossy(crate::handler::obj_short(g, o)),
                super::obj_vnum(g, o)
            ),
            GoId::Room(r) => format!(
                "It was attached to {} [{}]",
                String::from_utf8_lossy(g.world.rooms[r as usize].name.as_deref().unwrap_or(b"")),
                g.world.rooms[r as usize].vnum
            ),
        };
        script_log(g, &ident);
        extract_script(g, go);
        return SCRIPT_ERROR_CODE;
    }
    mud_data::rng::rng_trace_note(&format!(
        "trig {} {}",
        g.world.triggers[nr as usize].vnum,
        if mode == TRIG_NEW { "new" } else { "resume" }
    ));
    g.dg_script_depth += 1;

    if mode == TRIG_NEW {
        if let Some(t) = g.trig_mut(go, iid) {
            t.depth = 1;
            t.loops = 0;
        }
        if let Some(sc) = g.script_of_mut(go) {
            sc.context = 0;
        }
    }
    g.dg_owner_purged = false;

    let len = line_count(g, nr);
    let mut cl: usize = if mode == TRIG_NEW {
        0
    } else {
        g.trig(go, iid).map_or(usize::MAX, |t| t.curr_state)
    };

    'main: while cl < len {
        // GET_TRIG_DEPTH check each iteration.
        let Some(t) = g.trig(go, iid) else { break };
        if t.depth == 0 {
            break;
        }

        let raw = raw_line(g, nr, cl).unwrap_or_default();
        let p = skip_ws(&raw).to_vec();

        if p.first() == Some(&b'*') {
            cl += 1;
            continue;
        }

        if has_prefix(&p, b"if ") {
            if process_if(g, ctx, &p[3..]) {
                if let Some(t) = g.trig_mut(go, iid) {
                    t.depth += 1;
                }
            } else {
                cl = find_else_end(g, ctx, nr, cl);
            }
        } else if has_prefix(&p, b"elseif ") || has_prefix(&p, b"else") {
            let depth = g.trig(go, iid).map_or(0, |t| t.depth);
            if depth == 1 {
                let vnum = g.world.triggers[nr as usize].vnum;
                script_log(g, &format!("Trigger VNum {} has 'else' without 'if'.", vnum));
                cl += 1;
                continue;
            }
            cl = find_end(g, ctx, nr, cl);
            if let Some(t) = g.trig_mut(go, iid) {
                t.depth -= 1;
            }
        } else if has_prefix(&p, b"while ") {
            let temp = find_done(g, nr, cl);
            let Some(temp) = temp else {
                let vnum = g.world.triggers[nr as usize].vnum;
                script_log(g, &format!("Trigger VNum {} has 'while' without 'done'.", vnum));
                // Break rather than return, so this falls into the same
                // finalize as every other completed script. Returning here
                // skipped `t.depth = 0`, which left the trigger matching
                // nothing ever again, and skipped the dg_script_depth
                // decrement, which is shared by every script in the game.
                break 'main;
            };
            if process_if(g, ctx, &p[6..]) {
                line_state(g, nr, temp).original = Some(cl);
            } else {
                line_state(g, nr, cl).loops = 0;
                cl = temp;
            }
        } else if has_prefix(&p, b"switch ") {
            cl = find_case(g, ctx, nr, cl, &p[7..]);
        } else if has_prefix(&p, b"end") {
            let depth = g.trig(go, iid).map_or(0, |t| t.depth);
            if depth == 1 {
                let vnum = g.world.triggers[nr as usize].vnum;
                script_log(g, &format!("Trigger VNum {} has 'end' without 'if'.", vnum));
                cl += 1;
                continue;
            }
            if let Some(t) = g.trig_mut(go, iid) {
                t.depth -= 1;
            }
        } else if has_prefix(&p, b"done") {
            let original = line_state(g, nr, cl).original;
            if let Some(orig_idx) = original {
                let orig_raw = raw_line(g, nr, orig_idx).unwrap_or_default();
                let orig_cmd = skip_ws(&orig_raw).to_vec();
                // Re-evaluate the while condition (orig_cmd + 6).
                let cond = if orig_cmd.len() > 6 { orig_cmd[6..].to_vec() } else { Vec::new() };
                if process_if(g, ctx, &cond) {
                    cl = orig_idx;
                    line_state(g, nr, cl).loops += 1;
                    if let Some(t) = g.trig_mut(go, iid) {
                        t.loops += 1;
                    }
                    if line_state(g, nr, cl).loops == 30 {
                        line_state(g, nr, cl).loops = 0;
                        process_wait(g, ctx, b"wait 1", cl, &raw);
                        g.dg_script_depth -= 1;
                        return ret_val;
                    }
                    let total = g.trig(go, iid).map_or(0, |t| t.loops);
                    if total >= 100 {
                        let vnum = g.world.triggers[nr as usize].vnum;
                        script_log(g, &format!("Trigger VNum {} has looped 100 times!!!", vnum));
                        break 'main;
                    }
                }
                // else: switch fallthrough end — no-op.
            }
        } else if has_prefix(&p, b"break") {
            match find_done(g, nr, cl) {
                Some(n) => cl = n,
                None => break 'main, // unclosed block; end the script here
            }
        } else if has_prefix(&p, b"case") {
            // no-op: allows fallthrough
        } else {
            let cmd = var_subst(g, ctx, &p);
            if has_prefix(&cmd, b"eval ") {
                process_eval_line(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"nop ") {
                // do nothing
            } else if has_prefix(&cmd, b"extract ") {
                extract_value(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"dg_letter ") {
                dg_letter_value(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"makeuid ") {
                makeuid_var(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"halt") {
                break 'main;
            } else if has_prefix(&cmd, b"dg_cast ") {
                super::misc::do_dg_cast(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"dg_affect ") {
                super::misc::do_dg_affect(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"global ") {
                process_global(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"context ") {
                process_context(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"remote ") {
                process_remote(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"rdelete ") {
                process_rdelete(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"return ") {
                ret_val = process_return(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"set ") {
                process_set(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"unset ") {
                process_unset(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"wait ") {
                process_wait(g, ctx, &cmd, cl, &raw);
                g.dg_script_depth -= 1;
                return ret_val;
            } else if has_prefix(&cmd, b"attach ") {
                process_attach(g, ctx, &cmd);
            } else if has_prefix(&cmd, b"detach ") {
                process_detach(g, ctx, &cmd);
            } else {
                match go {
                    GoId::Char(chid) => {
                        if !super::mobcmd::script_command_interpreter(g, chid, &cmd) {
                            crate::interpreter::command_interpreter(g, chid, &cmd);
                        }
                    }
                    GoId::Obj(oid) => {
                        super::objcmd::obj_command_interpreter(g, oid, &cmd);
                    }
                    GoId::Room(room) => {
                        super::wldcmd::wld_command_interpreter(g, room, &cmd);
                    }
                }
                if g.dg_owner_purged {
                    g.dg_script_depth -= 1;
                    return ret_val;
                }
                // A detach-self (or a command that killed our owner's script)
                // ends the run at this point.
                if g.trig(go, iid).is_none() {
                    g.dg_script_depth -= 1;
                    return ret_val;
                }
            }
        }
        cl += 1;
    }

    // Finalize: the var list is freed (when the script survives) and zeroed
    // vars/depth; observably the locals are gone either way.
    if let Some(t) = g.trig_mut(go, iid) {
        t.var_list = Vec::new();
        t.depth = 0;
    }

    g.dg_script_depth -= 1;
    ret_val
}

/// Helper used by triggers.rs: fire one trigger with pre-set variables.
/// Returns script_driver's result.
pub fn fire(g: &mut Game, go: GoId, iid: u64, vars: Vec<(&'static [u8], BStr)>) -> i32 {
    if let Some(t) = g.trig_mut(go, iid) {
        for (name, value) in vars {
            add_var(&mut t.var_list, name, &value, 0);
        }
    }
    script_driver(g, go, iid, TRIG_NEW)
}

/// ADD_UID_VAR helper: "}<id>".
pub fn uid_var(id: i64) -> BStr {
    format!("}}{}", id).into_bytes()
}
