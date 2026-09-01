//! Dg_cast, dg_affect, send_char_pos, valid_dg_target,
//! script_damage.

use mud_data::ids::CharId;
use mud_data::spells::{
    skill_name, spell_info, CAST_SPELL, MAG_GROUPS, MAX_SPELLS, TAR_CHAR_ROOM, TAR_CHAR_WORLD,
    TAR_IGNORE, TAR_OBJ_EQUIP, TAR_OBJ_INV, TAR_OBJ_ROOM, TAR_OBJ_WORLD,
};
use mud_data::tables;
use mud_data::types::*;

use super::{atoi32, script_log, trig_log, DgCtx, GoId, DG_ALLOW_GODS, DG_CASTER_PROXY, DG_SPELL_LEVEL, SPELL_DG_AFFECT};
use crate::comm::{act, TO_CHAR, TO_ROOM};
use crate::game::Game;
use crate::handler::eq_ci;

pub fn valid_dg_target(g: &Game, chid: CharId, bitvector: i32) -> bool {
    let ch = g.ch(chid);
    if ch.is_npc() {
        return true;
    }
    if let Some(di) = ch.desc {
        if g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
            return false;
        }
    }
    if (ch.level as i32) < LVL_IMMORT as i32 {
        return true;
    }
    if bitvector & DG_ALLOW_GODS == 0 && ch.level >= LVL_GRGOD {
        return false;
    }
    !ch.prf(mud_data::flags::PRF_NOHASSLE)
}

pub fn send_char_pos(g: &mut Game, chid: CharId, dam: i32) {
    match g.ch(chid).position {
        POS_MORTALLYW => {
            act(g, b"$n is mortally wounded, and will die soon, if not aided.", true, Some(chid), None, None, TO_ROOM);
            crate::comm::send_to_char(g, chid, b"You are mortally wounded, and will die soon, if not aided.\r\n");
        }
        POS_INCAP => {
            act(g, b"$n is incapacitated and will slowly die, if not aided.", true, Some(chid), None, None, TO_ROOM);
            crate::comm::send_to_char(g, chid, b"You are incapacitated and will slowly die, if not aided.\r\n");
        }
        POS_STUNNED => {
            act(g, b"$n is stunned, but will probably regain consciousness again.", true, Some(chid), None, None, TO_ROOM);
            crate::comm::send_to_char(g, chid, b"You're stunned, but will probably regain consciousness again.\r\n");
        }
        POS_DEAD => {
            act(g, b"$n is dead!  R.I.P.", false, Some(chid), None, None, TO_ROOM);
            crate::comm::send_to_char(g, chid, b"You are dead!  Sorry...\r\n");
        }
        _ => {
            let (hit, max_hit) = {
                let p = &g.ch(chid).points;
                (p.hit, p.max_hit)
            };
            if dam > max_hit >> 2 {
                act(g, b"That really did HURT!", false, Some(chid), None, None, TO_CHAR);
            }
            if hit < max_hit >> 2 {
                let red = crate::comm::cc(g, chid, crate::comm::C_SPR, crate::comm::KRED).to_vec();
                let nrm = crate::comm::cc(g, chid, crate::comm::C_SPR, crate::comm::KNRM).to_vec();
                let mut msg = Vec::new();
                msg.extend_from_slice(&red);
                msg.extend_from_slice(b"You wish that your wounds would stop BLEEDING so much!");
                msg.extend_from_slice(&nrm);
                msg.extend_from_slice(b"\r\n");
                crate::comm::send_to_char_color(g, chid, &msg);
            }
        }
    }
}

pub fn script_damage(g: &mut Game, vict: CharId, dam: i32) {
    if g.ch(vict).level as i32 >= LVL_IMMORT as i32 && dam > 0 {
        crate::comm::send_to_char(
            g,
            vict,
            b"Being the cool immortal you are, you sidestep a trap, obviously placed to kill you.\r\n",
        );
        return;
    }
    {
        let p = &mut g.ch_mut(vict).points;
        p.hit -= dam;
        p.hit = p.hit.min(p.max_hit);
    }
    crate::fight::update_pos(g, vict);
    send_char_pos(g, vict, dam);
    if g.ch(vict).position == POS_DEAD {
        if !g.ch(vict).is_npc() {
            let name = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
            let room = g.ch(vict).in_room;
            let where_ = if room == NOWHERE {
                "NOWHERE".to_string()
            } else {
                String::from_utf8_lossy(g.world.rooms[room as usize].name.as_deref().unwrap_or(b""))
                    .into_owned()
            };
            let lvl = (LVL_IMMORT as i16).max(g.ch(vict).invis_lev()) as u8;
            g.mudlog(
                crate::game::MudlogKind::Brf,
                lvl,
                true,
                &format!("{} killed by script at {}", name, where_),
            );
        }
        crate::fight::die(g, vict, None);
    }
}

/// do_dg_cast. `cmd` is the whole var-substituted line.
pub fn do_dg_cast(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    // caster / caster room
    let (caster, caster_room): (Option<CharId>, Option<RoomRnum>) = match ctx.go {
        GoId::Char(chid) => (Some(chid), None),
        GoId::Room(r) => (None, Some(r)),
        GoId::Obj(oid) => {
            let r = super::obj_room(g, oid);
            if r == NOWHERE {
                script_log(g, "dg_do_cast: unknown room for object-caster!");
                return;
            }
            (None, Some(r))
        }
    };

    let orig_cmd = String::from_utf8_lossy(cmd).into_owned();
    // Split on single quotes: tokens are the non-empty segments, since runs
    // of delimiters are skipped. The final segment is the raw
    // remainder after the second token's closing quote.
    let segments: Vec<&[u8]> = cmd.split(|&b| b == b'\'').collect();
    let mut nonempty = segments.iter().enumerate().filter(|(_, s)| !s.is_empty());
    let Some((_, _first)) = nonempty.next() else {
        trig_log(g, ctx.go, ctx.iid, "dg_cast needs spell name.");
        return;
    };
    let Some((spell_idx, spell_seg)) = nonempty.next() else {
        trig_log(g, ctx.go, ctx.iid, "dg_cast needs spell name in `'s.");
        return;
    };
    let spell_name = spell_seg.to_vec();
    let rest: Vec<u8> = {
        let mut r = Vec::new();
        for (i, p) in segments.iter().enumerate().skip(spell_idx + 1) {
            if i > spell_idx + 1 {
                r.push(b'\'');
            }
            r.extend_from_slice(p);
        }
        r
    };

    let spellnum = crate::spec::find_skill_num(&spell_name).unwrap_or(-1);
    if !(1..=MAX_SPELLS).contains(&spellnum) {
        trig_log(g, ctx.go, ctx.iid, &format!("dg_cast: invalid spell name ({})", orig_cmd));
        return;
    }

    // Target word: one_argument of the rest.
    let (targ, _) = crate::interpreter::one_argument(&rest);
    let si = spell_info(spellnum);

    let mut tch: Option<CharId> = None;
    let mut tobj: Option<mud_data::ids::ObjId> = None;
    let mut target = false;
    if si.targets & TAR_IGNORE == 0 && !targ.is_empty() {
        if si.targets & (TAR_CHAR_ROOM | TAR_CHAR_WORLD) != 0 {
            if let Some(c) = super::get_char(g, &targ) {
                tch = Some(c);
                target = true;
            }
        }
        if !target && si.targets & (TAR_OBJ_INV | TAR_OBJ_EQUIP | TAR_OBJ_ROOM | TAR_OBJ_WORLD) != 0
        {
            if let Some(o) = super::get_obj(g, &targ) {
                tobj = Some(o);
                target = true;
            }
        }
        if !target {
            trig_log(g, ctx.go, ctx.iid, &format!("dg_cast: target not found ({})", orig_cmd));
            return;
        }
    }

    if si.routines & MAG_GROUPS != 0 {
        trig_log(g, ctx.go, ctx.iid, &format!("dg_cast: group spells not permitted ({})", orig_cmd));
        return;
    }

    match caster {
        Some(chid) => {
            let level = g.ch(chid).level as i32;
            crate::spell_parser::call_magic(g, chid, tch, tobj, spellnum, level, CAST_SPELL);
        }
        None => {
            let Some(proxy_rnum) = g.world.real_mobile(DG_CASTER_PROXY as Idx) else {
                script_log(g, "dg_cast: Cannot load the caster mob!");
                return;
            };
            let Some(proxy) = crate::db::read_mobile(g, proxy_rnum) else {
                script_log(g, "dg_cast: Cannot load the caster mob!");
                return;
            };
            let room = caster_room.unwrap();
            match ctx.go {
                GoId::Obj(oid) => {
                    let short = crate::handler::obj_short(g, oid).to_vec();
                    g.ch_mut(proxy).short_descr = Some(short);
                }
                GoId::Room(_) => {
                    g.ch_mut(proxy).short_descr = Some(b"The gods".to_vec());
                }
                _ => {}
            }
            // The proxy is spliced into the room people list directly
            // (prepend), bypassing char_to_room.
            g.rooms[room as usize].people.insert(0, proxy);
            g.ch_mut(proxy).in_room = room;
            crate::spell_parser::call_magic(g, proxy, tch, tobj, spellnum, DG_SPELL_LEVEL as i32, CAST_SPELL);
            crate::handler::extract_char(g, proxy);
        }
    }
}

pub fn do_dg_affect(g: &mut Game, ctx: DgCtx, cmd: &[u8]) {
    let (_, rest) = crate::interpreter::half_chop(cmd);
    let (charname, rest) = crate::interpreter::half_chop(&rest);
    let (property, rest) = crate::interpreter::half_chop(&rest);
    let (value_p, duration_p) = crate::interpreter::half_chop(&rest);

    if charname.is_empty() || property.is_empty() || value_p.is_empty() || duration_p.is_empty() {
        trig_log(g, ctx.go, ctx.iid, "dg_affect usage: <target> <property> <value> <duration>");
        return;
    }

    let value = atoi32(&value_p);
    let duration = atoi32(&duration_p);
    if duration <= 0 {
        trig_log(g, ctx.go, ctx.iid, "dg_affect: need positive duration!");
        script_log(
            g,
            &format!(
                "Line was: dg_affect {} {} {} {} ({})",
                String::from_utf8_lossy(&charname),
                String::from_utf8_lossy(&property),
                String::from_utf8_lossy(&value_p),
                String::from_utf8_lossy(&duration_p),
                duration
            ),
        );
        return;
    }

    // Property: apply_types first, then affected_bits.
    const APPLY_TYPE: i32 = 1;
    const AFFECT_TYPE: i32 = 2;
    let mut type_ = 0;
    let mut idx = 0usize;
    for (i, name) in tables::APPLY_TYPES.iter().enumerate() {
        if eq_ci(name.as_bytes(), &property) {
            type_ = APPLY_TYPE;
            idx = i;
            break;
        }
    }
    if type_ == 0 {
        for (i, name) in tables::AFFECTED_BITS.iter().enumerate() {
            if eq_ci(name.as_bytes(), &property) {
                type_ = AFFECT_TYPE;
                idx = i;
                break;
            }
        }
    }
    if type_ == 0 {
        trig_log(
            g,
            ctx.go,
            ctx.iid,
            &format!("dg_affect: unknown property '{}'!", String::from_utf8_lossy(&property)),
        );
        return;
    }

    let Some(ch) = super::get_char(g, &charname) else {
        trig_log(g, ctx.go, ctx.iid, "dg_affect: cannot locate target!");
        return;
    };

    if eq_ci(&value_p, b"off") {
        crate::handler::affect_from_char(g, ch, SPELL_DG_AFFECT);
        return;
    }

    let mut af = crate::ch::Affect {
        spell: SPELL_DG_AFFECT,
        duration: (duration - 1) as i16,
        modifier: value as i8,
        location: 0,
        bitvector: mud_data::flags::FlagSet::EMPTY,
    };
    if type_ == APPLY_TYPE {
        af.location = idx as u8;
    } else {
        af.bitvector.set(idx);
    }
    crate::handler::affect_to_char(g, ch, af);
}

/// Convenience: the spell/skill name string for trigger vars.
pub fn skill_name_b(num: i32) -> &'static [u8] {
    skill_name(num).as_bytes()
}
