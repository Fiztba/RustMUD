//! look/exits/examine, score/gold/inventory/equipment,
//! time/weather, who/whois/where/levels/consider/diagnose, help, toggle,
//! commands, gen_ps, history.

use mud_data::flags::{self};
use mud_data::ids::{CharId, ObjId};
use mud_data::tables;
use mud_data::types::*;

use crate::act::{pad_left, pad_right, pad_right_trunc, BStr};
use crate::ch::{DRUNK, HUNGER, THIRST};
use crate::comm::{self, act, cc, send_to_char, C_NRM, C_SPR, KCYN, KGRN, KNRM, KRED, KWHT, KYEL};
use crate::game::{Game, MudlogKind};
use crate::gametime::{age, real_time_passed_hours_days};
use crate::handler::{
    can_see, can_see_obj, fname, get_char_room_vis, get_number, get_obj_in_list_vis, isname, obj_action_desc,
    obj_name, obj_room_desc, obj_short, pers, room_is_dark,
};
use crate::interpreter::{any_one_arg, half_chop, is_abbrev_ci, one_argument, Handler};

pub const SHOW_OBJ_LONG: i32 = 0;
pub const SHOW_OBJ_SHORT: i32 = 1;
pub const SHOW_OBJ_ACTION: i32 = 2;

pub const CLASS_ABBREVS: [&[u8]; 4] = [b"Mu", b"Cl", b"Th", b"Wa"];
pub const PC_CLASS_TYPES: [&[u8]; 4] = [b"Magic User", b"Cleric", b"Thief", b"Warrior"];

pub fn class_abbr(class: i8) -> &'static [u8] {
    CLASS_ABBREVS.get(class as usize).copied().unwrap_or(b"--")
}

fn holylight(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    !ch.is_npc() && ch.prf(flags::PRF_HOLYLIGHT)
}

fn showvnums(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    !ch.is_npc() && ch.prf(flags::PRF_SHOWVNUMS)
}

/// The showvnums "[T#] "/"[TRIGS] " marker for a scripted entity
/// (etc).
fn trig_marker(g: &Game, go: crate::dg::GoId, trailing_space: bool) -> BStr {
    let Some(sc) = g.script_of(go) else { return Vec::new() };
    if sc.trig_list.is_empty() {
        return Vec::new();
    }
    let sp = if trailing_space { " " } else { "" };
    if sc.trig_list.len() == 1 {
        let vnum = g.world.triggers[sc.trig_list[0].nr as usize].vnum;
        format!("[T{}]{}", vnum, sp).into_bytes()
    } else {
        format!("[TRIGS]{}", sp).into_bytes()
    }
}

fn obj_vnum(g: &Game, oid: ObjId) -> i32 {
    // GET_OBJ_VNUM: NOTHING (65535) for unique objects like corpses.
    let rnum = g.obj(oid).item_number;
    if rnum != NOTHING {
        g.world.obj_protos.get(rnum as usize).map(|p| p.vnum as i32).unwrap_or(NOTHING as i32)
    } else {
        NOTHING as i32
    }
}

pub fn show_obj_to_char(g: &mut Game, oid: ObjId, chid: CharId, mode: i32) {
    let mut out: BStr = Vec::new();
    // Furniture "you are sitting upon" branch is stage 3 (SITTING lists).
    match mode {
        SHOW_OBJ_LONG => {
            let desc = obj_room_desc(g, oid).to_vec();
            if desc.first() == Some(&b'.') && !holylight(g, chid) {
                return;
            }
            if showvnums(g, chid) {
                out.extend_from_slice(format!("[{}] ", obj_vnum(g, oid)).as_bytes());
                out.extend_from_slice(&trig_marker(g, crate::dg::GoId::Obj(oid), true));
            }
            out.extend_from_slice(cc(g, chid, C_NRM, KGRN));
            out.extend_from_slice(&desc);
        }
        SHOW_OBJ_SHORT => {
            if showvnums(g, chid) {
                out.extend_from_slice(format!("[{}] ", obj_vnum(g, oid)).as_bytes());
                out.extend_from_slice(&trig_marker(g, crate::dg::GoId::Obj(oid), true));
            }
            out.extend_from_slice(obj_short(g, oid));
        }
        SHOW_OBJ_ACTION => {
            let type_flag = g.obj(oid).type_flag;
            if type_flag == flags::ITEM_NOTE {
                if let Some(action) = obj_action_desc(g, oid).map(|d| d.to_vec()) {
                    let mut notebuf = b"There is something written on it:\r\n\r\n".to_vec();
                    notebuf.extend_from_slice(&action);
                    page_string(g, chid, &notebuf);
                } else {
                    send_to_char(g, chid, b"It's blank.\r\n");
                }
                return;
            } else if type_flag == flags::ITEM_DRINKCON {
                out.extend_from_slice(b"It looks like a drink container.");
            } else {
                out.extend_from_slice(b"You see nothing special..");
            }
        }
        _ => {}
    }
    show_obj_modifiers(g, oid, chid, &mut out);
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
}

fn show_obj_modifiers(g: &Game, oid: ObjId, chid: CharId, out: &mut BStr) {
    let ch = g.ch(chid);
    let o = g.obj(oid);
    if o.extra_flags.is_set(flags::ITEM_INVISIBLE) {
        out.extend_from_slice(b" (invisible)");
    }
    if o.extra_flags.is_set(flags::ITEM_BLESS) && ch.aff(flags::AFF_DETECT_ALIGN) {
        out.extend_from_slice(b" ..It glows blue!");
    }
    if o.extra_flags.is_set(flags::ITEM_MAGIC) && ch.aff(flags::AFF_DETECT_MAGIC) {
        out.extend_from_slice(b" ..It glows yellow!");
    }
    if o.extra_flags.is_set(flags::ITEM_GLOW) {
        out.extend_from_slice(b" ..It has a soft glowing aura!");
    }
    if o.extra_flags.is_set(flags::ITEM_HUM) {
        out.extend_from_slice(b" ..It emits a faint humming sound!");
    }
}

/// list_obj_to_char — with `(2)` stacking, which is intended.
pub fn list_obj_to_char(g: &mut Game, list: &[ObjId], chid: CharId, mode: i32, show: bool) {
    let mut found = false;
    let mut done: Vec<usize> = Vec::new(); // indices already counted into a group
    for (i, &oid) in list.iter().enumerate() {
        if done.contains(&i) {
            continue;
        }
        // Count identical objects (short_description + name equality).
        let mut num = 0;
        let mut display: Option<ObjId> = None;
        for (j, &other) in list.iter().enumerate().skip(i) {
            if obj_short(g, other) == obj_short(g, oid) && obj_name(g, other) == obj_name(g, oid) {
                if can_see_obj(g, chid, other) {
                    if display.is_none() {
                        display = Some(other);
                    }
                    num += 1;
                }
                if j != i {
                    done.push(j);
                }
            }
        }
        let Some(display) = display else { continue };
        if mode == SHOW_OBJ_LONG {
            let desc = obj_room_desc(g, display).to_vec();
            if desc.first() == Some(&b'.') && !holylight(g, chid) {
                continue;
            }
        }
        found = true;
        if mode == SHOW_OBJ_LONG {
            let color = cc(g, chid, C_NRM, KGRN).to_vec();
            send_to_char(g, chid, &color);
        }
        if num != 1 {
            send_to_char(g, chid, format!("({:2}) ", num).as_bytes());
        }
        show_obj_to_char(g, display, chid, mode);
        let color_off = cc(g, chid, C_NRM, KNRM).to_vec();
        send_to_char(g, chid, &color_off);
    }
    if !found && show {
        send_to_char(g, chid, b"  Nothing.\r\n");
    }
}

fn diag_char_to_char(g: &mut Game, target: CharId, chid: CharId) {
    let (hit, max_hit) = {
        let t = g.ch(target);
        (t.points.hit, t.points.max_hit)
    };
    let percent = if max_hit > 0 { (100 * hit) / max_hit } else { -1 };
    let msg: &[u8] = if percent >= 100 {
        b"is in excellent condition."
    } else if percent >= 90 {
        b"has a few scratches."
    } else if percent >= 75 {
        b"has some small wounds and bruises."
    } else if percent >= 50 {
        b"has quite a few wounds."
    } else if percent >= 30 {
        b"has some big nasty wounds and scratches."
    } else if percent >= 15 {
        b"looks pretty hurt."
    } else if percent >= 0 {
        b"is in awful condition."
    } else {
        b"is bleeding awfully from big wounds."
    };
    let mut line = pers(g, chid, target);
    if let Some(c) = line.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    line.push(b' ');
    line.extend_from_slice(msg);
    line.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &line);
}

fn list_one_char(g: &mut Game, i: CharId, chid: CharId) {
    const POSITIONS: [&[u8]; 9] = [
        b" is lying here, dead.",
        b" is lying here, mortally wounded.",
        b" is lying here, incapacitated.",
        b" is lying here, stunned.",
        b" is sleeping here.",
        b" is resting here.",
        b" is sitting here.",
        b"!FIGHTING!",
        b" is standing here.",
    ];
    let mut out: BStr = Vec::new();
    let is_npc = g.ch(i).is_npc();
    if showvnums(g, chid) {
        if is_npc {
            let vnum = g
                .world
                .mob_protos
                .get(g.ch(i).mob_rnum as usize)
                .map(|p| p.vnum as i32)
                .unwrap_or(-1);
            out.extend_from_slice(format!("[{}] ", vnum).as_bytes());
        }
        out.extend_from_slice(&trig_marker(g, crate::dg::GoId::Char(i), true));
    }
    // Group tags are stage 5.

    let at_default_pos = is_npc && g.ch(i).position == g.ch(i).mob_specials.default_pos;
    let has_long = g.ch(i).long_descr.as_deref().is_some_and(|d| !d.is_empty());
    if is_npc && has_long && at_default_pos {
        if g.ch(i).aff(flags::AFF_INVISIBLE) {
            out.push(b'*');
        }
        if g.ch(chid).aff(flags::AFF_DETECT_ALIGN) {
            let align = g.ch(i).alignment;
            if align <= -350 {
                out.extend_from_slice(b"(Red Aura) ");
            } else if align >= 350 {
                out.extend_from_slice(b"(Blue Aura) ");
            }
        }
        out.extend_from_slice(g.ch(i).long_descr.as_deref().unwrap_or(b""));
        send_to_char(g, chid, &out);
        if g.ch(i).aff(flags::AFF_SANCTUARY) {
            act(g, b"...$e glows with a bright light!", false, Some(i), None, Some(chid), comm::TO_VICT);
        }
        if g.ch(i).aff(flags::AFF_BLIND) && g.ch(i).level < LVL_IMMORT {
            act(g, b"...$e is groping around blindly!", false, Some(i), None, Some(chid), comm::TO_VICT);
        }
        return;
    }

    if is_npc {
        let mut short = g.ch(i).short_descr.as_deref().unwrap_or(b"").to_vec();
        if let Some(c) = short.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        out.extend_from_slice(&short);
    } else {
        let ich = g.ch(i);
        out.extend_from_slice(ich.name.as_deref().unwrap_or(b""));
        if let Some(title) = ich.title.as_deref() {
            if !title.is_empty() {
                out.push(b' ');
                out.extend_from_slice(title);
            }
        }
    }
    if g.ch(i).aff(flags::AFF_INVISIBLE) {
        out.extend_from_slice(b" (invisible)");
    }
    if g.ch(i).aff(flags::AFF_HIDE) {
        out.extend_from_slice(b" (hidden)");
    }
    if !is_npc && g.ch(i).desc.is_none() {
        out.extend_from_slice(b" (linkless)");
    }
    if !is_npc && g.ch(i).plr(flags::PLR_WRITING) {
        out.extend_from_slice(b" (writing)");
    }
    if !is_npc && g.ch(i).prf(flags::PRF_BUILDWALK) {
        out.extend_from_slice(b" (buildwalk)");
    }
    if !is_npc && g.ch(i).prf(flags::PRF_AFK) {
        out.extend_from_slice(b" (AFK)");
    }

    let pos = g.ch(i).position;
    if pos != POS_FIGHTING {
        // Furniture display is stage 3.
        out.extend_from_slice(POSITIONS[pos.min(8) as usize]);
    } else if let Some(vict) = g.ch(i).fighting {
        out.extend_from_slice(b" is here, fighting ");
        if vict == chid {
            out.extend_from_slice(b"YOU!");
        } else if g.ch(i).in_room == g.try_ch(vict).map(|v| v.in_room).unwrap_or(NOWHERE) {
            let mut p = pers(g, chid, vict);
            p.push(b'!');
            out.extend_from_slice(&p);
        } else {
            out.extend_from_slice(b"someone who has already left!");
        }
    } else {
        out.extend_from_slice(b" is here struggling with thin air.");
    }

    if g.ch(chid).aff(flags::AFF_DETECT_ALIGN) {
        let align = g.ch(i).alignment;
        if align <= -350 {
            out.extend_from_slice(b" (Red Aura)");
        } else if align >= 350 {
            out.extend_from_slice(b" (Blue Aura)");
        }
    }
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);

    if g.ch(i).aff(flags::AFF_SANCTUARY) {
        act(g, b"...$e glows with a bright light!", false, Some(i), None, Some(chid), comm::TO_VICT);
    }
}

fn list_char_to_char(g: &mut Game, chid: CharId) {
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return;
    }
    let people = g.rooms[room as usize].people.clone();
    for i in people {
        if i == chid {
            continue;
        }
        let Some(_) = g.try_ch(i) else { continue };
        if g.ch(i).is_npc()
            && g.ch(i).long_descr.as_deref().is_some_and(|d| d.first() == Some(&b'.'))
            && !holylight(g, chid)
        {
            continue;
        }
        // The colour wrapper is printed for EVERY non-'.'-hidden char,
        // seen or not, so an unseeable char renders as a bare yellow/reset
        // pair.
        let ccyel = cc(g, chid, C_NRM, KYEL).to_vec();
        send_to_char(g, chid, &ccyel);
        if can_see(g, chid, i) {
            list_one_char(g, i, chid);
        } else if room_is_dark(g, room)
            && !(g.ch(chid).aff(flags::AFF_INFRAVISION) || g.ch(chid).prf(flags::PRF_HOLYLIGHT))
            && g.ch(i).aff(flags::AFF_INFRAVISION)
        {
            send_to_char(g, chid, b"You see a pair of glowing red eyes looking your way.\r\n");
        }
        let ccnrm = cc(g, chid, C_NRM, KNRM).to_vec();
        send_to_char(g, chid, &ccnrm);
    }
}

fn do_auto_exits(g: &mut Game, chid: CharId) {
    let room = g.ch(chid).in_room;
    let mut out: BStr = Vec::new();
    out.extend_from_slice(cc(g, chid, C_NRM, KCYN));
    out.extend_from_slice(b"[ Exits: ");
    let mut slen = 0;
    for door in 0..crate::fight::dir_count(g) {
        let Some(exit) = g.world.rooms[room as usize].dir_option[door].as_deref() else { continue };
        if exit.to_room == NOWHERE {
            continue;
        }
        let closed = exit.exit_info & flags::EX_CLOSED != 0;
        let hidden = exit.exit_info & flags::EX_HIDDEN != 0;
        if closed && !g.config.display_closed_doors {
            continue;
        }
        if hidden && !holylight(g, chid) {
            continue;
        }
        if closed {
            out.extend_from_slice(cc(g, chid, C_NRM, if hidden { KWHT } else { KRED }));
            out.push(b'(');
            out.extend_from_slice(tables::AUTOEXITS[door].as_bytes());
            out.push(b')');
            out.extend_from_slice(cc(g, chid, C_NRM, KCYN));
            out.push(b' ');
        } else if hidden {
            out.extend_from_slice(cc(g, chid, C_NRM, KWHT));
            out.extend_from_slice(tables::AUTOEXITS[door].as_bytes());
            out.extend_from_slice(cc(g, chid, C_NRM, KCYN));
            out.push(b' ');
        } else {
            out.extend_from_slice(b"\t(");
            out.extend_from_slice(tables::AUTOEXITS[door].as_bytes());
            out.extend_from_slice(b"\t) ");
        }
        slen += 1;
    }
    if slen == 0 {
        out.extend_from_slice(b"None!");
    }
    out.push(b']');
    out.extend_from_slice(cc(g, chid, C_NRM, KNRM));
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
}

pub fn do_exits(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).aff(flags::AFF_BLIND) && g.ch(chid).level < LVL_IMMORT {
        send_to_char(g, chid, b"You can't see a damned thing, you're blind!\r\n");
        return;
    }
    send_to_char(g, chid, b"Obvious exits:\r\n");
    let room = g.ch(chid).in_room;
    let mut len = 0;
    for door in 0..crate::fight::dir_count(g) {
        let Some(exit) = g.world.rooms[room as usize].dir_option[door].as_deref() else { continue };
        if exit.to_room == NOWHERE {
            continue;
        }
        let closed = exit.exit_info & flags::EX_CLOSED != 0;
        let hidden = exit.exit_info & flags::EX_HIDDEN != 0;
        if closed && !g.config.display_closed_doors {
            continue;
        }
        if hidden && !holylight(g, chid) {
            continue;
        }
        len += 1;
        let to_room = exit.to_room;
        let keyword = exit.keyword.clone();
        let mut line: BStr = Vec::new();
        line.extend_from_slice(&pad_right(tables::DIRS[door].as_bytes(), 5));
        if showvnums(g, chid) && !closed {
            let vnum = g.world.rooms[to_room as usize].vnum;
            line.extend_from_slice(b" -[");
            line.extend_from_slice(&pad_left(format!("{}", vnum).as_bytes(), 5));
            line.push(b']');
            if hidden {
                line.extend_from_slice(b"[HIDDEN]");
            }
            line.push(b' ');
            line.extend_from_slice(g.world.rooms[to_room as usize].name.as_deref().unwrap_or(b""));
        } else if closed {
            line.extend_from_slice(b" - The ");
            let kw = keyword.as_deref().map(fname).unwrap_or_else(|| b"opening".to_vec());
            line.extend_from_slice(&kw);
            line.extend_from_slice(b" is closed");
            if hidden {
                line.extend_from_slice(b" and hidden.");
            } else {
                line.push(b'.');
            }
        } else {
            line.extend_from_slice(b" - ");
            if room_is_dark(g, to_room) && !can_see_in_dark(g, chid) {
                line.extend_from_slice(b"Too dark to tell.");
            } else {
                line.extend_from_slice(g.world.rooms[to_room as usize].name.as_deref().unwrap_or(b""));
            }
        }
        line.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &line);
    }
    if len == 0 {
        send_to_char(g, chid, b" None.\r\n");
    }
}

pub fn can_see_in_dark(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    ch.aff(flags::AFF_INFRAVISION) || (!ch.is_npc() && ch.prf(flags::PRF_HOLYLIGHT))
}

pub fn look_at_room(g: &mut Game, chid: CharId, ignore_brief: bool) {
    if g.ch(chid).desc.is_none() {
        return;
    }
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return;
    }
    if room_is_dark(g, room) && !can_see_in_dark(g, chid) {
        send_to_char(g, chid, b"It is pitch black...\r\n");
        return;
    }
    if g.ch(chid).aff(flags::AFF_BLIND) && g.ch(chid).level < LVL_IMMORT {
        send_to_char(g, chid, b"You see nothing but infinite darkness...\r\n");
        return;
    }

    let mut out: BStr = Vec::new();
    out.extend_from_slice(cc(g, chid, C_NRM, KYEL));
    if showvnums(g, chid) {
        let r = &g.world.rooms[room as usize];
        let mut flagbuf: BStr = Vec::new();
        sprintbitarray(&r.room_flags, &tables::ROOM_BITS, &mut flagbuf);
        out.extend_from_slice(b"[");
        out.extend_from_slice(&pad_left(format!("{}", r.vnum).as_bytes(), 5));
        out.extend_from_slice(b"] ");
        out.extend_from_slice(r.name.as_deref().unwrap_or(b""));
        out.extend_from_slice(b"[ ");
        out.extend_from_slice(&flagbuf);
        out.extend_from_slice(b"][ ");
        let sect = r.sector_type;
        out.extend_from_slice(
            tables::SECTOR_TYPES.get(sect as usize).copied().unwrap_or("").as_bytes(),
        );
        out.extend_from_slice(b" ]");
        if let Some(sc) = g.rooms[room as usize].script.as_deref() {
            out.extend_from_slice(b"[T");
            for t in &sc.trig_list {
                let vnum = g.world.triggers[t.nr as usize].vnum;
                out.extend_from_slice(format!(" {}", vnum).as_bytes());
            }
            out.extend_from_slice(b"]");
        }
    } else {
        out.extend_from_slice(g.world.rooms[room as usize].name.as_deref().unwrap_or(b""));
    }
    out.extend_from_slice(cc(g, chid, C_NRM, KNRM));
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);

    let is_npc = g.ch(chid).is_npc();
    let brief = g.ch(chid).prf(flags::PRF_BRIEF);
    let death = g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_DEATH) != 0;
    if (!is_npc && !brief) || ignore_brief || death {
        if !is_npc
            && g.ch(chid).prf(flags::PRF_AUTOMAP)
            && crate::asciimap::can_see_map(g, chid)
        {
            // target_room is IN_ROOM(ch).
            let desc = g.world.rooms[room as usize].description.clone().unwrap_or_default();
            crate::asciimap::str_and_map(g, chid, &desc, room);
        } else {
            let desc = g.world.rooms[room as usize].description.clone().unwrap_or_default();
            send_to_char(g, chid, &desc);
        }
    }

    if g.ch(chid).prf(flags::PRF_AUTOEXIT) {
        do_auto_exits(g, chid);
    }

    let contents = g.rooms[room as usize].contents.clone();
    list_obj_to_char(g, &contents, chid, SHOW_OBJ_LONG, false);
    list_char_to_char(g, chid);
}

pub fn sprintbitarray(flags_arr: &[u32; 4], names: &[&str], out: &mut BStr) {
    let mut found = false;
    for (bit, name) in names.iter().enumerate() {
        if flags_arr[bit / 32] & (1 << (bit % 32)) != 0 {
            out.extend_from_slice(name.as_bytes());
            out.push(b' ');
            found = true;
        }
    }
    if !found {
        out.extend_from_slice(b"NOBITS ");
    }
}

fn look_in_direction(g: &mut Game, chid: CharId, dir: usize) {
    let room = g.ch(chid).in_room;
    if let Some(exit) = g.world.rooms[room as usize].dir_option[dir].as_deref() {
        let gen_desc = exit.general_description.clone();
        let keyword = exit.keyword.clone();
        let info = exit.exit_info;
        if let Some(desc) = gen_desc {
            send_to_char(g, chid, &desc);
        } else {
            send_to_char(g, chid, b"You see nothing special.\r\n");
        }
        if info & flags::EX_CLOSED != 0 {
            if let Some(kw) = &keyword {
                let mut line = b"The ".to_vec();
                line.extend_from_slice(&fname(kw));
                line.extend_from_slice(b" is closed.\r\n");
                send_to_char(g, chid, &line);
            }
        } else if info & flags::EX_ISDOOR != 0 {
            if let Some(kw) = &keyword {
                let mut line = b"The ".to_vec();
                line.extend_from_slice(&fname(kw));
                line.extend_from_slice(b" is open.\r\n");
                send_to_char(g, chid, &line);
            }
        }
    } else {
        send_to_char(g, chid, b"Nothing special there...\r\n");
    }
}

/// find_exdesc over an exdesc list.
fn find_exdesc<'a>(word: &[u8], list: &'a [mud_world::model::ExtraDesc]) -> Option<&'a [u8]> {
    for ed in list {
        if let Some(kw) = &ed.keyword {
            if isname(word, kw) {
                return ed.description.as_deref();
            }
        }
    }
    None
}

fn look_at_char(g: &mut Game, i: CharId, chid: CharId) {
    if g.ch(chid).desc.is_none() {
        return;
    }
    let desc = g.ch(i).description.clone();
    if let Some(desc) = desc {
        send_to_char(g, chid, &desc);
    } else {
        act(g, b"You see nothing special about $m.", false, Some(i), None, Some(chid), comm::TO_VICT);
    }
    diag_char_to_char(g, i, chid);

    let mut found = false;
    for pos in 0..NUM_WEARS {
        if let Some(oid) = g.ch(i).equipment[pos] {
            if can_see_obj(g, chid, oid) {
                found = true;
            }
        }
    }
    if found {
        send_to_char(g, chid, b"\r\n");
        act(g, b"$n is using:", false, Some(i), None, Some(chid), comm::TO_VICT);
        for pos in 0..NUM_WEARS {
            if let Some(oid) = g.ch(i).equipment[pos] {
                if can_see_obj(g, chid, oid) {
                    send_to_char(g, chid, tables::WEAR_WHERE[pos].as_bytes());
                    show_obj_to_char(g, oid, chid, SHOW_OBJ_SHORT);
                }
            }
        }
    }
    // Thief/immortal inventory peek.
    let is_thief_or_imm = {
        let ch = g.ch(chid);
        ch.class == CLASS_THIEF || ch.level >= LVL_IMMORT
    };
    if chid != i && is_thief_or_imm {
        act(g, b"\r\nYou attempt to peek at $s inventory:", false, Some(i), None, Some(chid), comm::TO_VICT);
        let carrying = g.ch(i).carrying.clone();
        list_obj_to_char(g, &carrying, chid, SHOW_OBJ_SHORT, true);
    }
}

/// look_at_target, stage-2 scope: chars, extra
/// descs, objects. generic_find equivalent inline.
fn look_at_target(g: &mut Game, chid: CharId, arg: &[u8]) {
    if g.ch(chid).desc.is_none() {
        return;
    }
    if arg.is_empty() {
        send_to_char(g, chid, b"Look at what?\r\n");
        return;
    }

    // Char in room first (generic_find FIND_CHAR_ROOM priority).
    if let Some(target) = get_char_room_vis(g, chid, arg, None) {
        look_at_char(g, target, chid);
        if chid != target {
            if can_see(g, target, chid) {
                act(g, b"$n looks at you.", true, Some(chid), None, Some(target), comm::TO_VICT);
            }
            act(g, b"$n looks at $N.", true, Some(chid), None, Some(target), comm::TO_NOTVICT);
        }
        return;
    }

    let (mut fnum, stripped) = get_number(arg);
    let room = g.ch(chid).in_room;

    // An object in inv/room/eq the arg might name (for modifier suffix).
    let found_obj = find_obj_for_look(g, chid, &stripped);

    // Room extra descriptions.
    let room_ex = g.world.rooms[room as usize].ex_descriptions.clone();
    if let Some(desc) = find_exdesc_counted(&stripped, &room_ex, &mut fnum) {
        page_string(g, chid, &desc);
        return;
    }
    // Equipment extra descriptions.
    for pos in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[pos] {
            if can_see_obj(g, chid, oid) {
                if let Some(desc) = exdesc_of_obj(g, oid, &stripped, &mut fnum) {
                    finish_exdesc(g, chid, &desc, oid, found_obj);
                    return;
                }
            }
        }
    }
    // Inventory extra descriptions.
    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        if can_see_obj(g, chid, oid) {
            if let Some(desc) = exdesc_of_obj(g, oid, &stripped, &mut fnum) {
                finish_exdesc(g, chid, &desc, oid, found_obj);
                return;
            }
        }
    }
    // Room-object extra descriptions.
    let contents = g.rooms[room as usize].contents.clone();
    for oid in contents {
        if can_see_obj(g, chid, oid) {
            if let Some(desc) = exdesc_of_obj(g, oid, &stripped, &mut fnum) {
                finish_exdesc(g, chid, &desc, oid, found_obj);
                return;
            }
        }
    }

    if let Some(oid) = found_obj {
        show_obj_to_char(g, oid, chid, SHOW_OBJ_ACTION);
    } else {
        send_to_char(g, chid, b"You do not see that here.\r\n");
    }
}

fn find_obj_for_look(g: &Game, chid: CharId, name: &[u8]) -> Option<ObjId> {
    let carrying = &g.ch(chid).carrying;
    if let Some(o) = get_obj_in_list_vis(g, chid, name, None, carrying) {
        return Some(o);
    }
    let room = g.ch(chid).in_room;
    if let Some(o) = get_obj_in_list_vis(g, chid, name, None, &g.rooms[room as usize].contents) {
        return Some(o);
    }
    for pos in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[pos] {
            if can_see_obj(g, chid, oid) && isname(name, obj_name(g, oid)) {
                return Some(oid);
            }
        }
    }
    None
}

fn exdesc_of_obj(g: &Game, oid: ObjId, word: &[u8], fnum: &mut i32) -> Option<BStr> {
    let o = g.obj(oid);
    let list: &[mud_world::model::ExtraDesc] = match &o.ex_descriptions {
        Some(l) => l,
        None => match g.world.obj_protos.get(o.item_number as usize) {
            Some(p) => &p.ex_descriptions,
            None => return None,
        },
    };
    // Same as find_exdesc_counted: a negative index never matches
    // `++i == fnum`, but it does satisfy the `<= 0` test below.
    if *fnum == crate::handler::FIND_INDEX_LAST {
        return None;
    }
    if let Some(desc) = find_exdesc(word, list) {
        *fnum -= 1;
        if *fnum <= 0 {
            return Some(desc.to_vec());
        }
    }
    None
}

fn find_exdesc_counted(word: &[u8], list: &[mud_world::model::ExtraDesc], fnum: &mut i32) -> Option<BStr> {
    if word.is_empty() {
        return None;
    }
    // "last." is not offered here: extra descriptions are counted with a
    // private `++i == fnum` loop, which a negative index never matches, and
    // do_look then falls through to the object search, which does honour
    // it.
    if *fnum == crate::handler::FIND_INDEX_LAST {
        return None;
    }
    if let Some(desc) = find_exdesc(word, list) {
        *fnum -= 1;
        if *fnum <= 0 {
            return Some(desc.to_vec());
        }
    }
    None
}

fn finish_exdesc(g: &mut Game, chid: CharId, desc: &[u8], oid: ObjId, found_obj: Option<ObjId>) {
    send_to_char(g, chid, desc);
    if found_obj == Some(oid) {
        let mut out: BStr = Vec::new();
        show_obj_modifiers(g, oid, chid, &mut out);
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);
    }
}

fn look_in_obj(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        send_to_char(g, chid, b"Look in what?\r\n");
        return;
    }
    let Some((oid, where_)) = generic_find_obj(g, chid, arg) else {
        let mut line = b"There doesn't seem to be ".to_vec();
        line.extend_from_slice(an_for(arg));
        line.push(b' ');
        line.extend_from_slice(arg);
        line.extend_from_slice(b" here.\r\n");
        send_to_char(g, chid, &line);
        return;
    };
    let type_flag = g.obj(oid).type_flag;
    if type_flag != flags::ITEM_DRINKCON && type_flag != flags::ITEM_FOUNTAIN && type_flag != flags::ITEM_CONTAINER {
        send_to_char(g, chid, b"There's nothing inside that!\r\n");
        return;
    }
    if type_flag == flags::ITEM_CONTAINER {
        let closed = g.obj(oid).values[1] & flags::CONT_CLOSED != 0;
        let bypass = g.ch(chid).level >= LVL_IMMORT && g.ch(chid).prf(flags::PRF_NOHASSLE);
        if closed && !bypass {
            send_to_char(g, chid, b"It is closed.\r\n");
        } else {
            let mut line = fname(obj_name(g, oid));
            line.extend_from_slice(match where_ {
                FoundWhere::Inventory => b" (carried): \r\n" as &[u8],
                FoundWhere::Room => b" (here): \r\n",
                FoundWhere::Equipment => b" (used): \r\n",
            });
            send_to_char(g, chid, &line);
            let contains = g.obj(oid).contains.clone();
            list_obj_to_char(g, &contains, chid, SHOW_OBJ_SHORT, true);
        }
    } else {
        // Drink containers.
        let (v0, v1, v2) = {
            let o = g.obj(oid);
            (o.values[0], o.values[1], o.values[2])
        };
        if v1 == 0 && v0 != -1 {
            send_to_char(g, chid, b"It is empty.\r\n");
        } else if v0 < 0 {
            let mut line = b"It's full of a ".to_vec();
            line.extend_from_slice(liquid_color(v2));
            line.extend_from_slice(b" liquid.\r\n");
            send_to_char(g, chid, &line);
        } else if v1 > v0 {
            send_to_char(g, chid, b"Its contents seem somewhat murky.\r\n");
        } else {
            let amt = (v1 * 3) / v0;
            const FULLNESS: [&[u8]; 4] = [b"less than half ", b"about half ", b"more than half ", b""];
            let mut line = b"It's ".to_vec();
            line.extend_from_slice(FULLNESS[amt.clamp(0, 3) as usize]);
            line.extend_from_slice(b"full of a ");
            line.extend_from_slice(liquid_color(v2));
            line.extend_from_slice(b" liquid.\r\n");
            send_to_char(g, chid, &line);
        }
    }
}

fn liquid_color(liq: i32) -> &'static [u8] {
    tables::COLOR_LIQUID
        .get(liq as usize)
        .map(|s| s.as_bytes())
        .unwrap_or(b"clear")
}

pub fn an_for(word: &[u8]) -> &'static [u8] {
    if word.first().is_some_and(|c| b"aeiouAEIOU".contains(c)) {
        b"an"
    } else {
        b"a"
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FoundWhere {
    Inventory,
    Room,
    Equipment,
}

fn generic_find_obj(g: &Game, chid: CharId, arg: &[u8]) -> Option<(ObjId, FoundWhere)> {
    let carrying = &g.ch(chid).carrying;
    if let Some(o) = get_obj_in_list_vis(g, chid, arg, None, carrying) {
        return Some((o, FoundWhere::Inventory));
    }
    let room = g.ch(chid).in_room;
    if let Some(o) = get_obj_in_list_vis(g, chid, arg, None, &g.rooms[room as usize].contents) {
        return Some((o, FoundWhere::Room));
    }
    let (_, name) = get_number(arg);
    for pos in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[pos] {
            if can_see_obj(g, chid, oid) && isname(&name, obj_name(g, oid)) {
                return Some((oid, FoundWhere::Equipment));
            }
        }
    }
    None
}

pub fn do_look(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_READ;
    if g.ch(chid).desc.is_none() {
        return;
    }
    if g.ch(chid).position < POS_SLEEPING {
        send_to_char(g, chid, b"You can't see anything but stars!\r\n");
        return;
    }
    if g.ch(chid).aff(flags::AFF_BLIND) && g.ch(chid).level < LVL_IMMORT {
        send_to_char(g, chid, b"You can't see a damned thing, you're blind!\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    if room_is_dark(g, room) && !can_see_in_dark(g, chid) {
        send_to_char(g, chid, b"It is pitch black...\r\n");
        list_char_to_char(g, chid); // glowing red eyes
        return;
    }

    let (arg, rest) = any_one_arg(argument);
    let (arg2, _) = one_argument(rest);

    if subcmd == SCMD_READ {
        if arg.is_empty() {
            send_to_char(g, chid, b"Read what?\r\n");
        } else {
            look_at_target(g, chid, &arg);
        }
        return;
    }
    if arg.is_empty() {
        look_at_room(g, chid, true);
    } else if is_abbrev_ci(&arg, b"in") {
        look_in_obj(g, chid, &arg2);
    } else if let Some(dir) = search_block(&arg, &tables::DIRS) {
        look_in_direction(g, chid, dir);
    } else if is_abbrev_ci(&arg, b"at") {
        look_at_target(g, chid, &arg2);
    } else if is_abbrev_ci(&arg, b"around") {
        let room_ex = g.world.rooms[room as usize].ex_descriptions.clone();
        let mut found = false;
        for ed in &room_ex {
            let Some(kw) = &ed.keyword else { continue };
            if kw.first() == Some(&b'.') {
                continue;
            }
            found = true;
            let mut out: BStr = Vec::new();
            out.extend_from_slice(kw);
            out.extend_from_slice(b":\r\n");
            out.extend_from_slice(ed.description.as_deref().unwrap_or(b""));
            send_to_char(g, chid, &out);
        }
        if !found {
            send_to_char(g, chid, b"You couldn't find anything noticeable.\r\n");
        }
    } else {
        look_at_target(g, chid, &arg);
    }
}

/// search_block over string tables (abbreviations allowed).
pub fn search_block(arg: &[u8], list: &[&str]) -> Option<usize> {
    if arg.is_empty() {
        return None;
    }
    if arg.first() == Some(&b'!') {
        return None;
    }
    let lower: BStr = arg.to_ascii_lowercase();
    list.iter().position(|item| item.as_bytes().starts_with(&lower[..]))
}

pub fn do_examine(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Examine what?\r\n");
        return;
    }
    look_at_target(g, chid, &arg);
    if let Some((oid, _)) = generic_find_obj(g, chid, &arg) {
        let t = g.obj(oid).type_flag;
        if t == flags::ITEM_DRINKCON || t == flags::ITEM_FOUNTAIN || t == flags::ITEM_CONTAINER {
            send_to_char(g, chid, b"When you look inside, you see:\r\n");
            look_in_obj(g, chid, &arg);
        }
    }
}

pub fn do_gold(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    let gold = g.ch(chid).points.gold;
    let msg: BStr = if gold == 0 {
        b"You're broke!\r\n".to_vec()
    } else if gold == 1 {
        b"You have one miserable little gold coin.\r\n".to_vec()
    } else {
        format!("You have {} gold coins.\r\n", gold).into_bytes()
    };
    send_to_char(g, chid, &msg);
}

pub fn compute_armor_class(g: &Game, chid: CharId) -> i32 {
    let ch = g.ch(chid);
    let mut armorclass = ch.points.armor;
    if ch.awake() {
        armorclass += tables::DEX_APP[ch.aff_abils.dex.clamp(0, 25) as usize].2 * 10;
    }
    armorclass.max(-100)
}

pub fn do_score(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let now = g.now;
    let birth = g.ch(chid).time.birth;
    let a = age(birth, now);
    let mut out: BStr = format!("You are {} years old.", a.year).into_bytes();
    if a.month == 0 && a.day == 0 {
        out.extend_from_slice(b"  It's your birthday today.\r\n");
    } else {
        out.extend_from_slice(b"\r\n");
    }
    let (p, level, class, alignment) = {
        let ch = g.ch(chid);
        (ch.points, ch.level, ch.class, ch.alignment)
    };
    out.extend_from_slice(
        format!(
            "You have {}({}) hit, {}({}) mana and {}({}) movement points.\r\n",
            p.hit, p.max_hit, p.mana, p.max_mana, p.mov, p.max_move
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!("Your armor class is {}/10, and your alignment is {}.\r\n", compute_armor_class(g, chid), alignment)
            .as_bytes(),
    );
    let questpoints = g.ch(chid).ps().questpoints;
    out.extend_from_slice(
        format!("You have {} exp, {} gold coins, and {} questpoints.\r\n", p.exp, p.gold, questpoints).as_bytes(),
    );
    if level < LVL_IMMORT {
        let need = tables::level_exp(class as i32, level as i32 + 1) - p.exp;
        out.extend_from_slice(format!("You need {} exp to reach your next level.\r\n", need).as_bytes());
    }
    out.extend_from_slice(format!("You have earned {} quest points.\r\n", questpoints).as_bytes());
    let nquests = g.ch(chid).ps().num_completed_quests;
    out.extend_from_slice(
        format!("You have completed {} quest{}, ", nquests, if nquests == 1 { "" } else { "s" }).as_bytes(),
    );
    let current_quest = g.ch(chid).ps().current_quest;
    if current_quest == NOTHING {
        out.extend_from_slice(b"and you are not on a quest at the moment.\r\n");
    } else {
        // Holding a vnum is not the same as it still resolving: `delete_quest`
        // does not clear the vnum from the players who are on that quest, so
        // this has to survive `real_quest` answering None rather than read the
        // table at NOTHING. Every other reader of the field already tests it —
        // quest_progress, quest_quit and autoquest_trigger_check all do — and
        // score was the one that did not. Saying so rather than clearing it
        // here: the quest commands clear the stale vnum the next time one of
        // them is used, and score reports state, it does not repair it.
        match crate::quest::real_quest(g, current_quest as i32) {
            None => out.extend_from_slice(b"and your current quest no longer exists"),
            Some(rnum) => {
                out.extend_from_slice(b"and your current quest is: ");
                out.extend_from_slice(
                    g.world.quests[rnum].name.as_deref().unwrap_or(b""),
                );
            }
        }
        if showvnums(g, chid) {
            out.extend_from_slice(format!(" [{}]\r\n", current_quest).as_bytes());
        } else {
            out.extend_from_slice(b"\r\n");
        }
    }
    let (logon, played) = {
        let t = g.ch(chid).time;
        (t.logon, t.played)
    };
    let (days, hours) = real_time_passed_hours_days((now - logon) + played as i64);
    out.extend_from_slice(
        format!(
            "You have been playing for {} day{} and {} hour{}.\r\n",
            days,
            if days == 1 { "" } else { "s" },
            hours,
            if hours == 1 { "" } else { "s" }
        )
        .as_bytes(),
    );
    {
        let ch = g.ch(chid);
        let name = ch.name.clone().unwrap_or_default();
        let title = ch.title.clone().unwrap_or_default();
        out.extend_from_slice(b"This ranks you as ");
        out.extend_from_slice(&name);
        out.push(b' ');
        out.extend_from_slice(&title);
        out.extend_from_slice(format!(" (level {}).\r\n", level).as_bytes());
    }
    let pos = g.ch(chid).position;
    let pos_line: BStr = match pos {
        POS_DEAD => b"You are DEAD!\r\n".to_vec(),
        POS_MORTALLYW => b"You are mortally wounded!  You should seek help!\r\n".to_vec(),
        POS_INCAP => b"You are incapacitated, slowly fading away...\r\n".to_vec(),
        POS_STUNNED => b"You are stunned!  You can't move!\r\n".to_vec(),
        POS_SLEEPING => b"You are sleeping.\r\n".to_vec(),
        POS_RESTING => b"You are resting.\r\n".to_vec(),
        POS_SITTING => b"You are sitting.\r\n".to_vec(),
        POS_FIGHTING => {
            let vict = g.ch(chid).fighting;
            let name = match vict {
                Some(v) => pers(g, chid, v),
                None => b"thin air".to_vec(),
            };
            let mut l = b"You are fighting ".to_vec();
            l.extend_from_slice(&name);
            l.extend_from_slice(b".\r\n");
            l
        }
        POS_STANDING => b"You are standing.\r\n".to_vec(),
        _ => b"You are floating.\r\n".to_vec(),
    };
    out.extend_from_slice(&pos_line);

    {
        let ch = g.ch(chid);
        let conds = ch.ps().conditions;
        if conds[DRUNK] > 10 {
            out.extend_from_slice(b"You are intoxicated.\r\n");
        }
        if conds[HUNGER] == 0 {
            out.extend_from_slice(b"You are hungry.\r\n");
        }
        if conds[THIRST] == 0 {
            out.extend_from_slice(b"You are thirsty.\r\n");
        }
        if ch.aff(flags::AFF_BLIND) && ch.level < LVL_IMMORT {
            out.extend_from_slice(b"You have been blinded!\r\n");
        }
        if ch.aff(flags::AFF_INVISIBLE) {
            out.extend_from_slice(b"You are invisible.\r\n");
        }
        if ch.aff(flags::AFF_DETECT_INVIS) {
            out.extend_from_slice(b"You are sensitive to the presence of invisible things.\r\n");
        }
        if ch.aff(flags::AFF_SANCTUARY) {
            out.extend_from_slice(b"You are protected by Sanctuary.\r\n");
        }
        if ch.aff(flags::AFF_POISON) {
            out.extend_from_slice(b"You are poisoned!\r\n");
        }
        if ch.aff(flags::AFF_CHARM) {
            out.extend_from_slice(b"You have been charmed!\r\n");
        }
        // SPELL_ARMOR = 1.
        if ch.affected.iter().any(|a| a.spell == 1) {
            out.extend_from_slice(b"You feel protected.\r\n");
        }
        if ch.aff(flags::AFF_INFRAVISION) {
            out.extend_from_slice(b"Your eyes are glowing red.\r\n");
        }
        if ch.prf(flags::PRF_SUMMONABLE) {
            out.extend_from_slice(b"You are summonable by other players.\r\n");
        }
    }
    send_to_char(g, chid, &out);

    if g.ch(chid).level >= LVL_IMMORT {
        let qyel = cc(g, chid, C_SPR, KYEL).to_vec();
        let qcyn = cc(g, chid, C_SPR, KCYN).to_vec();
        let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
        let name = g.ch(chid).name.clone().unwrap_or_default();
        let poofin = g.ch(chid).ps().poofin.clone();
        let poofout = g.ch(chid).ps().poofout.clone();
        let olc_zone = g.ch(chid).ps().olc_zone;
        let mut imm: BStr = Vec::new();
        imm.extend_from_slice(&qyel);
        imm.extend_from_slice(b"POOFIN:  ");
        imm.extend_from_slice(&qcyn);
        imm.extend_from_slice(&name);
        imm.push(b' ');
        imm.extend_from_slice(poofin.as_deref().unwrap_or(b"appears with an ear-splitting bang."));
        imm.extend_from_slice(&qnrm);
        imm.extend_from_slice(b"\r\n");
        imm.extend_from_slice(&qyel);
        imm.extend_from_slice(b"POOFOUT: ");
        imm.extend_from_slice(&qcyn);
        imm.extend_from_slice(&name);
        imm.push(b' ');
        imm.extend_from_slice(poofout.as_deref().unwrap_or(b"disappears in a puff of smoke."));
        imm.extend_from_slice(&qnrm);
        imm.extend_from_slice(b"\r\n");
        imm.extend_from_slice(b"Your current zone: ");
        imm.extend_from_slice(&qcyn);
        imm.extend_from_slice(format!("{}", olc_zone).as_bytes());
        imm.extend_from_slice(&qnrm);
        imm.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &imm);
    }
}

pub fn do_inventory(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    send_to_char(g, chid, b"You are carrying:\r\n");
    let carrying = g.ch(chid).carrying.clone();
    list_obj_to_char(g, &carrying, chid, SHOW_OBJ_SHORT, true);
}

pub fn do_equipment(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    send_to_char(g, chid, b"You are using:\r\n");
    let mut found = false;
    for pos in 0..NUM_WEARS {
        let Some(oid) = g.ch(chid).equipment[pos] else { continue };
        if can_see_obj(g, chid, oid) {
            send_to_char(g, chid, tables::WEAR_WHERE[pos].as_bytes());
            show_obj_to_char(g, oid, chid, SHOW_OBJ_SHORT);
        } else {
            send_to_char(g, chid, tables::WEAR_WHERE[pos].as_bytes());
            send_to_char(g, chid, b"Something.\r\n");
        }
        found = true;
    }
    if !found {
        send_to_char(g, chid, b" Nothing.\r\n");
    }
}

pub fn do_time(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    let t = g.time_info;
    let am_pm = if t.hours >= 12 { "pm" } else { "am" };
    let hour_12 = if t.hours % 12 == 0 { 12 } else { t.hours % 12 };
    let day = t.day + 1;
    let weekday = tables::WEEKDAYS[(((35 * t.month) + day) % 7) as usize];
    let mut out = format!("It is {} o'clock {}, on {}.\r\n", hour_12, am_pm, weekday).into_bytes();
    let suf = if (day % 100) / 10 != 1 {
        match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    } else {
        "th"
    };
    out.extend_from_slice(
        format!("The {}{} Day of the {}, Year {}.\r\n", day, suf, tables::MONTH_NAME[t.month as usize], t.year)
            .as_bytes(),
    );
    send_to_char(g, chid, &out);
}

pub fn do_weather(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    const SKY_LOOK: [&[u8]; 4] =
        [b"cloudless", b"cloudy", b"rainy", b"lit by flashes of lightning"];
    if comm::outside(g, chid) {
        let sky = g.weather.sky;
        let change: &[u8] = if g.weather.change >= 0 {
            b"you feel a warm wind from south"
        } else {
            b"your foot tells you bad weather is due"
        };
        let mut out = b"The sky is ".to_vec();
        out.extend_from_slice(SKY_LOOK[sky.clamp(0, 3) as usize]);
        out.extend_from_slice(b" and ");
        out.extend_from_slice(change);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
        if g.ch(chid).level >= LVL_GOD {
            let w = g.weather;
            send_to_char(
                g,
                chid,
                format!(
                    "Pressure: {} (change: {}), Sky: {} ({})\r\n",
                    w.pressure,
                    w.change,
                    w.sky,
                    String::from_utf8_lossy(SKY_LOOK[w.sky.clamp(0, 3) as usize])
                )
                .as_bytes(),
            );
        }
    } else {
        send_to_char(g, chid, b"You have no feeling about the weather at all.\r\n");
    }
}

fn space_to_minus(b: &mut BStr) {
    for c in b.iter_mut() {
        if *c == b' ' {
            *c = b'-';
        }
    }
}

/// do_help + search_help.
pub fn do_help(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).desc.is_none() {
        return;
    }
    let argument = crate::interpreter::skip_spaces(argument);
    if argument.is_empty() {
        let screen = if g.ch(chid).level < LVL_IMMORT {
            g.texts.help_screen.clone()
        } else {
            g.texts.ihelp_screen.clone()
        };
        page_string(g, chid, &screen);
        return;
    }
    if g.help_table.is_empty() {
        send_to_char(g, chid, b"No help available.\r\n");
        return;
    }
    let mut query = argument.to_vec();
    space_to_minus(&mut query);
    let level = g.ch(chid).level;

    // search_help: prefix match, back up to first, skip over-level entries.
    let lower_q = query.to_ascii_lowercase();
    let mut found: Option<usize> = None;
    for (i, entry) in g.help_table.iter().enumerate() {
        if entry.keyword.to_ascii_lowercase().starts_with(&lower_q[..]) {
            if entry.min_level > level as i32 {
                continue;
            }
            found = Some(i);
            break;
        }
    }
    match found {
        Some(i) => {
            let entry = g.help_table[i].entry.as_ref().clone();
            page_string(g, chid, &entry);
        }
        None => {
            send_to_char(g, chid, b"There is no help on that word.\r\n");
            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let invis = g.ch(chid).invis_lev();
            let mlevel = (LVL_IMPL as i16).min(invis.max(LVL_IMPL as i16)); // MIN(LVL_IMPL, MAX?) — C: MAX(LVL_IMPL, GET_INVIS_LEV)
            let _ = mlevel;
            g.mudlog(
                MudlogKind::Nrm,
                LVL_IMPL,
                true,
                &format!("{} tried to get help on {}", name, String::from_utf8_lossy(&query)),
            );
            let mut out: BStr = Vec::new();
            let mut first = true;
            let mut printed: Vec<BStr> = Vec::new();
            for entry in g.help_table.iter() {
                if entry.min_level > level as i32 {
                    continue;
                }
                if entry.keyword.first().map(|c| c.to_ascii_lowercase()) != lower_q.first().copied() {
                    continue;
                }
                if crate::interpreter::levenshtein_distance(&lower_q, &entry.keyword.to_ascii_lowercase()) <= 2 {
                    if printed.contains(&entry.keyword) {
                        continue;
                    }
                    if first {
                        out.extend_from_slice(b"\r\nDid you mean:\r\n");
                        first = false;
                    }
                    out.extend_from_slice(b"  \t<send link=\"Help ");
                    out.extend_from_slice(&entry.keyword);
                    out.extend_from_slice(b"\">");
                    out.extend_from_slice(&entry.keyword);
                    out.extend_from_slice(b"\t</send>\r\n");
                    printed.push(entry.keyword.clone());
                }
            }
            if !out.is_empty() {
                send_to_char(g, chid, &out);
            }
        }
    }
}

pub fn do_hindex(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Usage: hindex <string>\r\n");
        return;
    }
    let level = g.ch(chid).level as i32;
    let mut buf: BStr = Vec::new();
    buf.extend_from_slice(b"\t1Help index entries beginning with '");
    buf.extend_from_slice(argument);
    buf.extend_from_slice(b"':\t2\r\n");
    let mut buf2: BStr = Vec::new();
    buf2.extend_from_slice(b"\t1Help index entries containing '");
    buf2.extend_from_slice(argument);
    buf2.extend_from_slice(b"':\t2\r\n");
    let mut count = 0;
    let mut count2 = 0;
    for entry in &g.help_table {
        if entry.min_level > level {
            continue;
        }
        if is_abbrev_ci(argument, &entry.keyword) {
            count += 1;
            buf.extend_from_slice(&pad_right_trunc(&entry.keyword, 20));
            if count % 3 == 0 {
                buf.extend_from_slice(b"\r\n");
            }
        } else if find_sub(&entry.keyword, argument) {
            count2 += 1;
            buf2.extend_from_slice(&pad_right_trunc(&entry.keyword, 20));
            if count2 % 3 == 0 {
                buf2.extend_from_slice(b"\r\n");
            }
        }
    }
    if count % 3 != 0 {
        buf.extend_from_slice(b"\r\n");
    }
    if count2 % 3 != 0 {
        buf2.extend_from_slice(b"\r\n");
    }
    if count == 0 {
        buf.extend_from_slice(b"  None.\r\n");
    }
    if count2 == 0 {
        buf2.extend_from_slice(b"  None.\r\n");
    }
    buf.extend_from_slice(&buf2);
    buf.extend_from_slice(
        format!("\t1Applicable Index Entries: \t3{}\r\n\t1Total Index Entries: \t3{}\tn\r\n", count + count2, g.help_table.len())
            .as_bytes(),
    );
    page_string(g, chid, &buf);
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

pub fn do_who(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    const WHO_FORMAT: &[u8] =
        b"Usage: who [minlev[-maxlev]] [-n name] [-c classlist] [-k] [-l] [-n] [-q] [-r] [-s] [-z]\r\n";
    let mut buf = crate::interpreter::skip_spaces(argument).to_vec();
    let mut name_search: BStr = Vec::new();
    let (mut low, mut high) = (0i32, LVL_IMPL as i32);
    let (mut outlaws, mut localwho, mut short_list, mut questwho, mut who_room) =
        (false, false, false, false, false);
    let mut showclass: i32 = 0;
    let (mut showgroup, mut showleader) = (false, false);
    let _ = (showgroup, showleader);

    while !buf.is_empty() {
        let (arg, rest) = half_chop(&buf);
        if arg.first().is_some_and(|c| c.is_ascii_digit()) {
            let s = String::from_utf8_lossy(&arg).into_owned();
            let mut parts = s.splitn(2, '-');
            if let Some(l) = parts.next().and_then(|x| x.parse::<i32>().ok()) {
                low = l;
            }
            if let Some(h) = parts.next().and_then(|x| x.parse::<i32>().ok()) {
                high = h;
            }
            buf = rest;
        } else if arg.first() == Some(&b'-') {
            match arg.get(1).copied().unwrap_or(0) {
                b'k' => {
                    outlaws = true;
                    buf = rest;
                }
                b'z' => {
                    localwho = true;
                    buf = rest;
                }
                b's' => {
                    short_list = true;
                    buf = rest;
                }
                b'q' => {
                    questwho = true;
                    buf = rest;
                }
                b'n' => {
                    let (ns, rest2) = half_chop(&rest);
                    name_search = ns;
                    buf = rest2;
                }
                b'r' => {
                    who_room = true;
                    buf = rest;
                }
                b'c' => {
                    let (cls, rest2) = half_chop(&rest);
                    showclass = find_class_bitvector(&cls);
                    buf = rest2;
                }
                b'l' => {
                    showleader = true;
                    buf = rest;
                }
                b'g' => {
                    showgroup = true;
                    buf = rest;
                }
                _ => {
                    send_to_char(g, chid, WHO_FORMAT);
                    return;
                }
            }
        } else {
            send_to_char(g, chid, WHO_FORMAT);
            return;
        }
    }

    struct Rank {
        disp: &'static [u8],
        min: i32,
        max: i32,
        count: i32,
    }
    let mut ranks = [
        Rank { disp: b"Immortals\r\n---------\r\n", min: LVL_IMMORT as i32, max: LVL_IMPL as i32, count: 0 },
        Rank { disp: b"Mortals\r\n-------\r\n", min: 1, max: LVL_IMMORT as i32 - 1, count: 0 },
    ];

    let my_zone = {
        let room = g.ch(chid).in_room;
        g.world.rooms[room as usize].zone
    };
    let my_room = g.ch(chid).in_room;

    // Candidate list per descriptor (original preferred).
    let candidates: Vec<CharId> = g
        .descriptors
        .indices()
        .into_iter()
        .filter_map(|di| {
            let d = g.descriptors.get(di)?;
            if !d.is_playing() {
                return None;
            }
            d.original.or(d.character)
        })
        .collect();

    let passes = |g: &Game, tch: CharId| -> bool {
        let t = g.ch(tch);
        if !name_search.is_empty() {
            let nm = t.name.as_deref().unwrap_or(b"");
            let title = t.title.as_deref().unwrap_or(b"");
            if !nm.eq_ignore_ascii_case(&name_search) && !find_sub(title, &name_search) {
                return false;
            }
        }
        if !can_see(g, chid, tch) || (t.level as i32) < low || (t.level as i32) > high {
            return false;
        }
        if outlaws && !t.plr(flags::PLR_KILLER) && !t.plr(flags::PLR_THIEF) {
            return false;
        }
        if questwho && !t.prf(flags::PRF_QUEST) {
            return false;
        }
        if localwho && (t.in_room == NOWHERE || g.world.rooms[t.in_room as usize].zone != my_zone) {
            return false;
        }
        if who_room && t.in_room != my_room {
            return false;
        }
        if showclass != 0 && showclass & (1 << t.class) == 0 {
            return false;
        }
        // Group filters are stage 5; with no group system, -l/-g match nobody.
        if showgroup || showleader {
            return false;
        }
        true
    };

    if !short_list {
        for &tch in &candidates {
            if !can_see(g, chid, tch) {
                continue;
            }
            if !passes(g, tch) {
                continue;
            }
            let lvl = g.ch(tch).level as i32;
            for r in ranks.iter_mut() {
                if lvl >= r.min && lvl <= r.max {
                    r.count += 1;
                }
            }
        }
    }

    let mut num_can_see = 0;
    for (ri, r) in ranks.iter().enumerate() {
        if r.count == 0 && !short_list {
            continue;
        }
        if short_list {
            send_to_char(g, chid, b"Players\r\n-------\r\n");
        } else {
            send_to_char(g, chid, r.disp);
        }
        for &tch in &candidates {
            let lvl = g.ch(tch).level as i32;
            if (lvl < r.min || lvl > r.max) && !short_list {
                continue;
            }
            if !passes(g, tch) {
                continue;
            }
            let imm = lvl >= LVL_IMMORT as i32;
            let yel = if imm { cc(g, chid, C_SPR, KYEL) } else { b"" }.to_vec();
            let nrm = cc(g, chid, C_SPR, KNRM).to_vec();
            let name = g.ch(tch).name.clone().unwrap_or_default();
            let class = g.ch(tch).class;
            if short_list {
                num_can_see += 1;
                let mut line = yel.clone();
                line.extend_from_slice(format!("[{:2} ", lvl).as_bytes());
                line.extend_from_slice(class_abbr(class));
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&pad_right_trunc(&name, 12));
                line.extend_from_slice(&nrm);
                if num_can_see % 4 == 0 {
                    line.extend_from_slice(b"\r\n");
                }
                send_to_char(g, chid, &line);
            } else {
                num_can_see += 1;
                let title = g.ch(tch).title.clone().unwrap_or_default();
                let mut line = yel.clone();
                line.extend_from_slice(format!("[{:2} ", lvl).as_bytes());
                line.extend_from_slice(class_abbr(class));
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&name);
                if !title.is_empty() {
                    line.push(b' ');
                }
                line.extend_from_slice(&title);
                line.extend_from_slice(&nrm);
                let t = g.ch(tch);
                let invis_lev = t.invis_lev();
                if invis_lev != 0 {
                    line.extend_from_slice(format!(" (i{})", invis_lev).as_bytes());
                } else if t.aff(flags::AFF_INVISIBLE) {
                    line.extend_from_slice(b" (invis)");
                }
                if t.plr(flags::PLR_MAILING) {
                    line.extend_from_slice(b" (mailing)");
                } else if t.plr(flags::PLR_WRITING) {
                    line.extend_from_slice(b" (writing)");
                }
                let di = t.desc;
                if let Some(di) = di {
                    if let Some(d) = g.descriptors.get(di) {
                        if d.original.is_some() {
                            line.extend_from_slice(b" (out of body)");
                        }
                        // OLC state tags are stage 9.
                    }
                }
                let t = g.ch(tch);
                if t.prf(flags::PRF_BUILDWALK) {
                    line.extend_from_slice(b" (Buildwalking)");
                }
                if t.prf(flags::PRF_AFK) {
                    line.extend_from_slice(b" (AFK)");
                }
                if t.prf(flags::PRF_NOGOSS) {
                    line.extend_from_slice(b" (nogos)");
                }
                if t.prf(flags::PRF_NOWIZ) {
                    line.extend_from_slice(b" (nowiz)");
                }
                if t.prf(flags::PRF_NOSHOUT) {
                    line.extend_from_slice(b" (noshout)");
                }
                if t.prf(flags::PRF_NOTELL) {
                    line.extend_from_slice(b" (notell)");
                }
                if t.prf(flags::PRF_QUEST) {
                    line.extend_from_slice(b" (quest)");
                }
                if t.plr(flags::PLR_THIEF) {
                    line.extend_from_slice(b" (THIEF)");
                }
                if t.plr(flags::PLR_KILLER) {
                    line.extend_from_slice(b" (KILLER)");
                }
                line.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &line);
            }
        }
        send_to_char(g, chid, b"\r\n");
        if short_list {
            break;
        }
        let _ = ri;
    }
    if short_list && num_can_see % 4 != 0 {
        send_to_char(g, chid, b"\r\n");
    }
    if num_can_see == 0 {
        send_to_char(g, chid, b"Nobody at all!\r\n");
    } else if num_can_see == 1 {
        send_to_char(g, chid, b"One lonely character displayed.\r\n");
    } else {
        send_to_char(g, chid, format!("{} characters displayed.\r\n", num_can_see).as_bytes());
    }
    if crate::act::other::is_happyhour(g) {
        send_to_char(
            g,
            chid,
            b"It's a Happy Hour! Type \tRhappyhour\tW to see the current bonuses.\r\n",
        );
    }
}

/// find_class_bitvector: letters m/c/t/w → class bits. Invalid
/// letters contribute 0; the original's undefined shift is fixed here.
/// A letter that names no class. **B42**: writing
/// `ret |= (1 << parse_class(arg[rpos]))` shifts by `parse_class`'s
/// CLASS_UNDEFINED (-1) for an unknown letter — a negative shift count,
/// which is undefined. On the usual compilers the count is masked
/// to 31 and bit 31 gets set, which no real class uses, so `who -c <bad>`
/// and `users -c <bad>` list nobody. That outcome is the sensible one and
/// is kept; only the undefined shift goes.
const CLASS_BIT_UNKNOWN: i32 = 1 << 31;

fn find_class_bitvector(arg: &[u8]) -> i32 {
    let mut bits = 0;
    for c in arg.iter().map(|c| c.to_ascii_lowercase()) {
        match c {
            b'm' => bits |= 1 << CLASS_MAGIC_USER,
            b'c' => bits |= 1 << CLASS_CLERIC,
            b't' => bits |= 1 << CLASS_THIEF,
            b'w' => bits |= 1 << CLASS_WARRIOR,
            _ => bits |= CLASS_BIT_UNKNOWN,
        }
    }
    bits
}

/// do_whois — online-target scope for stage 2;
/// offline lookups arrive with the pfile wiring.
pub fn do_whois(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Who?\r\n");
        return;
    }
    let target = crate::handler::get_player_vis(g, chid, &arg, false);
    let Some(victim) = target else {
        send_to_char(g, chid, b"There is no such player.\r\n");
        return;
    };
    let v = g.ch(victim);
    let name = v.name.clone().unwrap_or_default();
    let title = v.title.clone().unwrap_or_default();
    let sex = v.sex;
    let class = v.class;
    let level = v.level;
    let timer = v.timer;
    let linkless = v.desc.is_none();
    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"Name: ");
    out.extend_from_slice(&name);
    out.push(b' ');
    out.extend_from_slice(&title);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b"Sex: ");
    out.extend_from_slice(tables::GENDERS[sex.min(2) as usize].as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b"Class: ");
    out.extend_from_slice(PC_CLASS_TYPES.get(class as usize).copied().unwrap_or(b"Undefined"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("Level: {}\r\n", level).as_bytes());
    out.extend_from_slice(
        format!("Last Logon: Playing now!  (Idle {} Minutes)", timer * SECS_PER_MUD_HOUR as i32 / 60).as_bytes(),
    );
    if linkless {
        out.extend_from_slice(b"  (Linkless)");
    }
    out.extend_from_slice(b"\r\n");
    if g.ch(victim).prf(flags::PRF_AFK) {
        let grn = cc(g, chid, C_NRM, KGRN).to_vec();
        let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
        out.extend_from_slice(&grn);
        out.extend_from_slice(&name);
        out.extend_from_slice(b" is afk right now, so ");
        out.extend_from_slice(match sex {
            SEX_MALE => b"he" as &[u8],
            SEX_FEMALE => b"she",
            _ => b"it",
        });
        out.extend_from_slice(b" may not respond to communication.");
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
    }
    send_to_char(g, chid, &out);
    // Mail notice + protocol block are later stages.
}

fn perform_mortal_where(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        let mut out: BStr = Vec::new();
        out.extend_from_slice(b"Players in your Zone\r\n--------------------\r\n");
        let my_zone = {
            let room = g.ch(chid).in_room;
            g.world.rooms[room as usize].zone
        };
        let candidates: Vec<CharId> = g
            .descriptors
            .indices()
            .into_iter()
            .filter_map(|di| {
                let d = g.descriptors.get(di)?;
                if d.state != ConState::Playing {
                    return None;
                }
                d.character
            })
            .collect();
        let mut any = false;
        for tch in candidates {
            if tch == chid {
                continue;
            }
            let Some(t) = g.try_ch(tch) else { continue };
            if t.in_room == NOWHERE || !can_see(g, chid, tch) {
                continue;
            }
            if g.world.rooms[t.in_room as usize].zone != my_zone {
                continue;
            }
            any = true;
            let t = g.ch(tch);
            let name = t.name.clone().unwrap_or_default();
            let room_name = g.world.rooms[t.in_room as usize].name.clone().unwrap_or_default();
            out.extend_from_slice(&pad_right(&name, 20));
            out.extend_from_slice(b" - ");
            out.extend_from_slice(&room_name);
            out.extend_from_slice(b"\r\n");
        }
        let _ = any;
        send_to_char(g, chid, &out);
    } else {
        let my_zone = {
            let room = g.ch(chid).in_room;
            g.world.rooms[room as usize].zone
        };
        let candidates: Vec<CharId> = g.character_list.clone();
        for tch in candidates {
            let Some(t) = g.try_ch(tch) else { continue };
            if t.in_room == NOWHERE || !can_see(g, chid, tch) {
                continue;
            }
            if g.world.rooms[t.in_room as usize].zone != my_zone {
                continue;
            }
            if !isname(arg, g.ch(tch).name.as_deref().unwrap_or(b"")) {
                continue;
            }
            let t = g.ch(tch);
            let name = t.get_name().to_vec();
            let room_name = g.world.rooms[t.in_room as usize].name.clone().unwrap_or_default();
            let mut out = pad_right(&name, 25);
            out.extend_from_slice(b" - ");
            out.extend_from_slice(&room_name);
            out.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &out);
            return;
        }
        send_to_char(g, chid, b"Nobody around by that name.\r\n");
    }
}

/// do_where; immortal form is basic for now.
pub fn do_where(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    if g.ch(chid).level >= LVL_IMMORT {
        perform_immort_where(g, chid, &arg);
    } else {
        perform_mortal_where(g, chid, &arg);
    }
}

/// perform_immort_where, stage-2 essentials.
fn perform_immort_where(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        let mut out: BStr = Vec::new();
        out.extend_from_slice(b"Players  Room    Location                       Zone\r\n");
        out.extend_from_slice(b"-------- ------- ------------------------------ -------------------\r\n");
        for di in g.descriptors.indices() {
            let Some(d) = g.descriptors.get(di) else { continue };
            // In play, which includes an OLC editor: someone sitting in one
            // is still listed by `where`.
            if !d.is_playing() {
                continue;
            }
            let Some(tch) = d.character else { continue };
            let Some(t) = g.try_ch(tch) else { continue };
            if !can_see(g, chid, tch) || t.in_room == NOWHERE {
                continue;
            }
            let name = t.name.clone().unwrap_or_default();
            let room = t.in_room as usize;
            let vnum = g.world.rooms[room].vnum;
            let room_name = g.world.rooms[room].name.clone().unwrap_or_default();
            let zone_num = g.world.rooms[room].zone as usize;
            let zone_name = g.world.zones[zone_num].name.clone().unwrap_or_default();
            // "%-8s%s %s[%s%5d%s]%s %-*s%s %s%s\r\n" with QNRM/QCYN/QYEL
            // width 30 + color chars.
            let qnrm = cc(g, chid, C_SPR, KNRM);
            let qcyn = cc(g, chid, C_SPR, KCYN);
            let qyel = cc(g, chid, C_SPR, KYEL);
            out.extend_from_slice(&pad_right(&name, 8));
            out.extend_from_slice(qnrm);
            out.push(b' ');
            out.extend_from_slice(qcyn);
            out.push(b'[');
            out.extend_from_slice(qyel);
            out.extend_from_slice(&pad_left(format!("{}", vnum).as_bytes(), 5));
            out.extend_from_slice(qcyn);
            out.push(b']');
            out.extend_from_slice(qnrm);
            out.push(b' ');
            out.extend_from_slice(&pad_right(&room_name, 30));
            out.extend_from_slice(qnrm);
            out.push(b' ');
            out.extend_from_slice(&zone_name);
            out.extend_from_slice(qnrm);
            out.extend_from_slice(b"\r\n");
        }
        send_to_char(g, chid, &out);
    } else {
        let mut found = false;
        let mut num = 0;
        let mut out: BStr = Vec::new();
        let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
        let verbose = g.ch(chid).prf(flags::PRF_VERBOSE);
        if verbose {
            out.extend_from_slice(b"   ### Mob name                   - Room #  Room name\r\n");
        }
        let list = g.character_list.clone();
        for tch in list {
            let Some(t) = g.try_ch(tch) else { continue };
            if t.in_room == NOWHERE || !can_see(g, chid, tch) {
                continue;
            }
            if !isname(arg, t.name.as_deref().unwrap_or(b"")) {
                continue;
            }
            found = true;
            num += 1;
            let t = g.ch(tch);
            let name = t.get_name().to_vec();
            let room = t.in_room as usize;
            let vnum = g.world.rooms[room].vnum;
            let room_name = g.world.rooms[room].name.clone().unwrap_or_default();
            out.extend_from_slice(b"M");
            out.extend_from_slice(&pad_left(format!("{}", num).as_bytes(), 4));
            out.extend_from_slice(b". ");
            out.extend_from_slice(&pad_right(&name, 25));
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(b" - [");
            out.extend_from_slice(&pad_left(format!("{}", vnum).as_bytes(), 5));
            out.extend_from_slice(b"] ");
            out.extend_from_slice(&pad_right(&room_name, 25));
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(&trig_marker_nosp(g, crate::dg::GoId::Char(tch)));
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(b"\r\n");
        }
        // Object scan: `num` CONTINUES from the mob count (do_stat lookup
        // parity).
        if verbose {
            out.extend_from_slice(b"  ###  Object name                 Location\r\n");
        }
        let objs = g.object_list.clone();
        for oid in objs {
            if g.objs.get(oid).is_none() {
                continue;
            }
            if !can_see_obj(g, chid, oid) || !isname(arg, obj_name(g, oid)) {
                continue;
            }
            found = true;
            num += 1;
            print_object_location(g, chid, num, oid, true, &mut out);
        }
        if !found {
            send_to_char(g, chid, b"Couldn't find any such thing.\r\n");
        } else {
            page_string(g, chid, &out);
        }
    }
}

fn print_object_location(g: &mut Game, chid: CharId, num: i32, oid: ObjId, recur: bool, out: &mut BStr) {
    let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
    if num > 0 {
        out.extend_from_slice(b"O");
        out.extend_from_slice(&pad_left(format!("{}", num).as_bytes(), 4));
        out.extend_from_slice(b". ");
        out.extend_from_slice(&pad_right(obj_short(g, oid), 25));
        out.extend_from_slice(&qnrm);
        out.extend_from_slice(b" - ");
    } else {
        out.extend_from_slice(&pad_left(b" - ", 37));
    }
    // [T#]/[TRIGS] with trailing space. Unconditional, not showvnums.
    if let Some(sc) = g.script_of(crate::dg::GoId::Obj(oid)) {
        if !sc.trig_list.is_empty() {
            if sc.trig_list.len() == 1 {
                let tvnum = g.world.triggers[sc.trig_list[0].nr as usize].vnum;
                out.extend_from_slice(format!("[T{}] ", tvnum).as_bytes());
            } else {
                out.extend_from_slice(b"[TRIGS] ");
            }
        }
    }
    let (in_room, carried_by, worn_by, in_obj) = {
        let o = g.obj(oid);
        (o.in_room, o.carried_by, o.worn_by, o.in_obj)
    };
    let verbose = g.ch(chid).prf(flags::PRF_VERBOSE);
    let showvn = g.ch(chid).prf(flags::PRF_SHOWVNUMS);
    if in_room != NOWHERE {
        let vnum = g.world.rooms[in_room as usize].vnum;
        let rname = g.world.rooms[in_room as usize].name.clone().unwrap_or_default();
        out.extend_from_slice(b"[");
        out.extend_from_slice(&pad_left(format!("{}", vnum).as_bytes(), 5));
        out.extend_from_slice(b"] ");
        out.extend_from_slice(&rname);
        out.extend_from_slice(&qnrm);
        out.extend_from_slice(b"\r\n");
    } else if let Some(carrier) = carried_by {
        if showvn {
            out.extend_from_slice(
                format!("carried by [{:5}] ", crate::dg::mob_vnum(g, carrier)).as_bytes(),
            );
        } else {
            out.extend_from_slice(b"carried by ");
        }
        out.extend_from_slice(&pers(g, chid, carrier));
        out.extend_from_slice(&qnrm);
        out.extend_from_slice(b"\r\n");
        let croom = g.ch(carrier).in_room;
        if verbose && croom != NOWHERE {
            let vnum = g.world.rooms[croom as usize].vnum;
            let rname = g.world.rooms[croom as usize].name.clone().unwrap_or_default();
            out.extend_from_slice(&pad_left(b" - ", 37));
            out.extend_from_slice(format!("in [{:5}] ", vnum).as_bytes());
            out.extend_from_slice(&rname);
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(b"\r\n");
        }
    } else if let Some(wearer) = worn_by {
        if showvn {
            out.extend_from_slice(
                format!("worn by [{:5}] ", crate::dg::mob_vnum(g, wearer)).as_bytes(),
            );
        } else {
            out.extend_from_slice(b"worn by ");
        }
        out.extend_from_slice(&pers(g, chid, wearer));
        out.extend_from_slice(&qnrm);
        out.extend_from_slice(b"\r\n");
        let wroom = g.ch(wearer).in_room;
        if verbose && wroom != NOWHERE {
            let vnum = g.world.rooms[wroom as usize].vnum;
            let rname = g.world.rooms[wroom as usize].name.clone().unwrap_or_default();
            out.extend_from_slice(&pad_left(b" - ", 37));
            out.extend_from_slice(format!("in [{:5}] ", vnum).as_bytes());
            out.extend_from_slice(&rname);
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(b"\r\n");
        }
    } else if let Some(container) = in_obj {
        out.extend_from_slice(b"inside ");
        out.extend_from_slice(obj_short(g, container));
        out.extend_from_slice(&qnrm);
        out.extend_from_slice(if recur { b", which is" } else { b" " });
        out.extend_from_slice(b"\r\n");
        if recur {
            print_object_location(g, chid, 0, container, recur, out);
        }
    } else {
        out.extend_from_slice(b"in an unknown location\r\n");
    }
}

/// [T#]/[TRIGS] without the trailing space (targeted-where mob lines).
fn trig_marker_nosp(g: &Game, go: crate::dg::GoId) -> BStr {
    let Some(sc) = g.script_of(go) else { return Vec::new() };
    if sc.trig_list.is_empty() {
        return Vec::new();
    }
    if sc.trig_list.len() == 1 {
        let vnum = g.world.triggers[sc.trig_list[0].nr as usize].vnum;
        format!("[T{}]", vnum).into_bytes()
    } else {
        b"[TRIGS]".to_vec()
    }
}

pub fn do_levels(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"You ain't nothin' but a hound-dog.\r\n");
        return;
    }
    let (arg, _) = one_argument(argument);
    let mut min_lev = 1i32;
    let mut max_lev = LVL_IMMORT as i32;
    if !arg.is_empty() {
        let s = String::from_utf8_lossy(&arg).into_owned();
        if let Some(dash) = s.find('-') {
            let (a, b) = s.split_at(dash);
            let b = &b[1..];
            match (a.parse::<i32>(), b.parse::<i32>()) {
                (Ok(x), Ok(y)) if x > 0 && y > 0 && x <= y && x < LVL_IMMORT as i32 => {
                    min_lev = x;
                    max_lev = (y + 1).min(LVL_IMMORT as i32);
                }
                _ => {
                    send_to_char(g, chid, b"Usage: levels [<min>-<max> | <level>]\r\n");
                    return;
                }
            }
        } else if let Ok(x) = s.parse::<i32>() {
            if x < 1 || x > LVL_IMMORT as i32 {
                send_to_char(g, chid, b"Usage: levels [<min>-<max> | <level>]\r\n");
                return;
            }
            min_lev = (x - 5).max(1);
            max_lev = (x + 5).min(LVL_IMMORT as i32);
        } else {
            send_to_char(g, chid, b"Usage: levels [<min>-<max> | <level>]\r\n");
            return;
        }
    }
    let (class, sex, level) = {
        let ch = g.ch(chid);
        (ch.class, ch.sex, ch.level)
    };
    let _ = level;
    let mut out: BStr = Vec::new();
    for lvl in min_lev..max_lev {
        out.extend_from_slice(format!("[{:2}] {:8}-{:<8} : ", lvl, tables::level_exp(class as i32, lvl), tables::level_exp(class as i32, lvl + 1) - 1).as_bytes());
        let title = if sex == SEX_FEMALE {
            tables::TITLES_FEMALE[class.clamp(0, 3) as usize][lvl as usize]
        } else {
            tables::TITLES_MALE[class.clamp(0, 3) as usize][lvl as usize]
        };
        out.extend_from_slice(title.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if max_lev == LVL_IMMORT as i32 {
        out.extend_from_slice(
            format!("[{:2}] {:8}          : Immortality\r\n", LVL_IMMORT, tables::level_exp(class as i32, LVL_IMMORT as i32))
                .as_bytes(),
        );
    }
    page_string(g, chid, &out);
}

pub fn do_consider(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    let Some(victim) = get_char_room_vis(g, chid, &buf, None) else {
        send_to_char(g, chid, b"Consider killing who?\r\n");
        return;
    };
    if victim == chid {
        send_to_char(g, chid, b"Easy!  Very easy indeed!\r\n");
        return;
    }
    if !g.ch(victim).is_npc() {
        send_to_char(g, chid, b"Would you like to borrow a cross and a shovel?\r\n");
        return;
    }
    let diff = g.ch(victim).level as i32 - g.ch(chid).level as i32;
    let msg: &[u8] = if diff <= -10 {
        b"Now where did that chicken go?\r\n"
    } else if diff <= -5 {
        b"You could do it with a needle!\r\n"
    } else if diff <= -2 {
        b"Easy.\r\n"
    } else if diff <= -1 {
        b"Fairly easy.\r\n"
    } else if diff == 0 {
        b"The perfect match!\r\n"
    } else if diff <= 1 {
        b"You would need some luck!\r\n"
    } else if diff <= 2 {
        b"You would need a lot of luck!\r\n"
    } else if diff <= 3 {
        b"You would need a lot of luck and great equipment!\r\n"
    } else if diff <= 5 {
        b"Do you feel lucky, punk?\r\n"
    } else if diff <= 10 {
        b"Are you mad!?\r\n"
    } else if diff <= 100 {
        b"You ARE mad!\r\n"
    } else {
        b""
    };
    if !msg.is_empty() {
        send_to_char(g, chid, msg);
    }
}

pub fn do_diagnose(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    if !buf.is_empty() {
        match get_char_room_vis(g, chid, &buf, None) {
            Some(vict) => diag_char_to_char(g, vict, chid),
            None => {
                let msg = g.config.noperson.clone();
                send_to_char(g, chid, &msg);
            }
        }
    } else if let Some(vict) = g.ch(chid).fighting {
        diag_char_to_char(g, vict, chid);
    } else {
        send_to_char(g, chid, b"Diagnose who?\r\n");
    }
}

/// do_toggle — status board + named toggles.
pub fn do_toggle(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, _subcmd: i32) {
    use crate::interpreter as int;
    if g.ch(chid).is_npc() {
        return;
    }
    let argument = int::skip_spaces(argument);
    if argument.is_empty() {
        print_toggle_board(g, chid);
        return;
    }
    let (name, rest) = any_one_arg(argument);
    let (value, _) = any_one_arg(rest);

    // The toggle table: name → (flag, level,
    // off-msg, on-msg). Special entries dispatch to do_gen_tog subcommands.
    struct Tog {
        name: &'static [u8],
        flag: usize,
        level: u8,
        off: &'static [u8],
        on: &'static [u8],
    }
    const TOGGLES: &[Tog] = &[
        Tog { name: b"summonable", flag: flags::PRF_SUMMONABLE, level: 0, off: b"You are now safe from summoning by other players.\r\n", on: b"You may now be summoned by other players.\r\n" },
        Tog { name: b"nohassle", flag: flags::PRF_NOHASSLE, level: LVL_IMMORT, off: b"Nohassle disabled.\r\n", on: b"Nohassle enabled.\r\n" },
        Tog { name: b"brief", flag: flags::PRF_BRIEF, level: 0, off: b"Brief mode off.\r\n", on: b"Brief mode on.\r\n" },
        Tog { name: b"compact", flag: flags::PRF_COMPACT, level: 0, off: b"Compact mode off.\r\n", on: b"Compact mode on.\r\n" },
        Tog { name: b"notell", flag: flags::PRF_NOTELL, level: 0, off: b"You can now hear tells.\r\n", on: b"You are now deaf to tells.\r\n" },
        Tog { name: b"noauction", flag: flags::PRF_NOAUCT, level: 0, off: b"You can now hear auctions.\r\n", on: b"You are now deaf to auctions.\r\n" },
        Tog { name: b"noshout", flag: flags::PRF_NOSHOUT, level: 0, off: b"You can now hear shouts.\r\n", on: b"You are now deaf to shouts.\r\n" },
        Tog { name: b"nogossip", flag: flags::PRF_NOGOSS, level: 0, off: b"You can now hear gossip.\r\n", on: b"You are now deaf to gossip.\r\n" },
        Tog { name: b"nograts", flag: flags::PRF_NOGRATZ, level: 0, off: b"You can now hear gratz.\r\n", on: b"You are now deaf to gratz.\r\n" },
        Tog { name: b"nowiz", flag: flags::PRF_NOWIZ, level: LVL_IMMORT, off: b"You can now hear the Wiz-channel.\r\n", on: b"You are now deaf to the Wiz-channel.\r\n" },
        Tog { name: b"quest", flag: flags::PRF_QUEST, level: 0, off: b"You are no longer part of the Quest.\r\n", on: b"Okay, you are part of the Quest.\r\n" },
        Tog { name: b"showvnums", flag: flags::PRF_SHOWVNUMS, level: LVL_IMMORT, off: b"You will no longer see the vnums.\r\n", on: b"You will now see the vnums.\r\n" },
        Tog { name: b"norepeat", flag: flags::PRF_NOREPEAT, level: 0, off: b"You will now have your communication repeated.\r\n", on: b"You will no longer have your communication repeated.\r\n" },
        Tog { name: b"holylight", flag: flags::PRF_HOLYLIGHT, level: LVL_IMMORT, off: b"HolyLight mode off.\r\n", on: b"HolyLight mode on.\r\n" },
        Tog { name: b"autoexits", flag: flags::PRF_AUTOEXIT, level: 0, off: b"Autoexits disabled.\r\n", on: b"Autoexits enabled.\r\n" },
        Tog { name: b"clsolc", flag: flags::PRF_CLS, level: LVL_BUILDER, off: b"You will no longer clear screen in OLC.\r\n", on: b"You will now clear screen in OLC.\r\n" },
        Tog { name: b"buildwalk", flag: flags::PRF_BUILDWALK, level: LVL_BUILDER, off: b"Buildwalk is now Off.\r\n", on: b"Buildwalk is now On.\r\n" },
        Tog { name: b"afk", flag: flags::PRF_AFK, level: 0, off: b"AFK is now Off.\r\n", on: b"AFK is now On.\r\n" },
        Tog { name: b"autoloot", flag: flags::PRF_AUTOLOOT, level: 0, off: b"Autoloot disabled.\r\n", on: b"Autoloot enabled.\r\n" },
        Tog { name: b"autogold", flag: flags::PRF_AUTOGOLD, level: 0, off: b"Autogold disabled.\r\n", on: b"Autogold enabled.\r\n" },
        Tog { name: b"autosplit", flag: flags::PRF_AUTOSPLIT, level: 0, off: b"Autosplit disabled.\r\n", on: b"Autosplit enabled.\r\n" },
        Tog { name: b"autosac", flag: flags::PRF_AUTOSAC, level: 0, off: b"Autosac disabled.\r\n", on: b"Autosac enabled.\r\n" },
        Tog { name: b"autoassist", flag: flags::PRF_AUTOASSIST, level: 0, off: b"Autoassist disabled.\r\n", on: b"Autoassist enabled.\r\n" },
        Tog { name: b"automap", flag: flags::PRF_AUTOMAP, level: 1, off: b"You will no longer see the mini-map.\r\n", on: b"You will now see a mini-map at the side of room descriptions.\r\n" },
        Tog { name: b"autokey", flag: flags::PRF_AUTOKEY, level: 0, off: b"You will now have to unlock doors manually before opening.\r\n", on: b"You will now automatically unlock doors when opening them (if you have the key).\r\n" },
        Tog { name: b"autodoor", flag: flags::PRF_AUTODOOR, level: 0, off: b"You will now need to specify a door direction when opening, closing and unlocking.\r\n", on: b"You will now find the next available door when opening, closing or unlocking.\r\n" },
        Tog { name: b"zoneresets", flag: flags::PRF_ZONERESETS, level: LVL_IMPL, off: b"You will no longer see zone resets.\r\n", on: b"You will now see zone resets.\r\n" },
    ];

    // Special multi-state entries first: color / syslog / wimpy /
    // pagelength / screenwidth.
    if is_abbrev_ci(&name, b"color") {
        toggle_color(g, chid, &value);
        return;
    }
    if is_abbrev_ci(&name, b"syslog") {
        toggle_syslog(g, chid, &value);
        return;
    }
    if is_abbrev_ci(&name, b"wimpy") {
        crate::act::other::gen_tog_wimpy(g, chid, &value);
        return;
    }
    if is_abbrev_ci(&name, b"pagelength") {
        crate::act::other::gen_tog_pagelength(g, chid, &value);
        return;
    }
    if is_abbrev_ci(&name, b"screenwidth") {
        crate::act::other::gen_tog_screenwidth(g, chid, &value);
        return;
    }

    let level = g.ch(chid).level;
    let found = TOGGLES.iter().find(|t| t.name.starts_with(&name[..]) && level >= t.level);
    let Some(tog) = found else {
        send_to_char(g, chid, b"You can't toggle that!\r\n");
        return;
    };
    let (flag, on_msg, off_msg) = (tog.flag, tog.on, tog.off);
    let result = if value == b"on" {
        g.ch_mut(chid).ps_mut().pref.set(flag);
        true
    } else if value == b"off" {
        g.ch_mut(chid).ps_mut().pref.remove(flag);
        false
    } else if value.is_empty() {
        let ps = g.ch_mut(chid).ps_mut();
        ps.pref.toggle(flag);
        ps.pref.is_set(flag)
    } else {
        let mut msg = b"Value for ".to_vec();
        msg.extend_from_slice(&name);
        msg.extend_from_slice(b" must either be 'on' or 'off'.\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    let _ = cmd;
    send_to_char(g, chid, if result { on_msg } else { off_msg });
}

fn onoff(b: bool) -> &'static str {
    if b { "ON" } else { "OFF" }
}

/// The toggle status board — an exact transcription of the format strings
/// the Verbose line's trailing
/// two-space bleed onto the next line and the final line's trailing space.
fn print_toggle_board(g: &mut Game, chid: CharId) {
    let ch = g.ch(chid);
    let level = ch.level;
    let ps = ch.ps();
    let pref = ps.pref;
    let wimp = ps.wimp_level;
    let page_length = ps.page_length;
    let screen_width = ps.screen_width;
    let is = |f: usize| pref.is_set(f);
    const TYPES: [&str; 4] = ["off", "brief", "normal", "on"];
    let wimp_str = if wimp == 0 { "OFF".to_string() } else { format!("{:03}", wimp.min(999)) };
    let mut out = String::new();
    if level == LVL_IMPL {
        out.push_str(&format!(
            " SlowNameserver: {:<3}                            Trackthru Doors: {:<3}\r\n",
            onoff(g.config.nameserver_is_slow),
            onoff(g.config.track_through_doors)
        ));
    }
    if level >= LVL_IMMORT {
        let syslog_level = (is(flags::PRF_LOG1) as usize) + 2 * (is(flags::PRF_LOG2) as usize);
        out.push_str(&format!(
            concat!(
                "      Buildwalk: {:<3}              NoWiz: {:<3}             ClsOLC: {:<3}\r\n",
                "       NoHassle: {:<3}          Holylight: {:<3}          ShowVnums: {:<3}\r\n",
                "         Syslog: {:<3}            Verbose: {:<3}{}  "
            ),
            onoff(is(flags::PRF_BUILDWALK)),
            onoff(is(flags::PRF_NOWIZ)),
            onoff(is(flags::PRF_CLS)),
            onoff(is(flags::PRF_NOHASSLE)),
            onoff(is(flags::PRF_HOLYLIGHT)),
            onoff(is(flags::PRF_SHOWVNUMS)),
            TYPES[syslog_level],
            onoff(is(flags::PRF_VERBOSE)),
            if level == LVL_IMPL { "" } else { "\r\n" }
        ));
    }
    if level >= LVL_IMPL {
        out.push_str(&format!("     ZoneResets: {:<3}\r\n", onoff(is(flags::PRF_ZONERESETS))));
    }
    out.push_str(&format!(
        concat!(
            "Hit Pnt Display: {:<3}              Brief: {:<3}         Summonable: {:<3}\r\n",
            "   Move Display: {:<3}            Compact: {:<3}              Quest: {:<3}\r\n",
            "   Mana Display: {:<3}             NoTell: {:<3}           NoRepeat: {:<3}\r\n",
            "      AutoExits: {:<3}            NoShout: {:<3}              Wimpy: {:<3}\r\n",
            "       NoGossip: {:<3}          NoAuction: {:<3}            NoGrats: {:<3}\r\n",
            "       AutoLoot: {:<3}           AutoGold: {:<3}          AutoSplit: {:<3}\r\n",
            "        AutoSac: {:<3}         AutoAssist: {:<3}            AutoMap: {:<3}\r\n",
            "     Pagelength: {:<3}        Screenwidth: {:<3}                AFK: {:<3}\r\n",
            "        Autokey: {:<3}           Autodoor: {:<3}              Color: {}     \r\n "
        ),
        onoff(is(flags::PRF_DISPHP)),
        onoff(is(flags::PRF_BRIEF)),
        onoff(is(flags::PRF_SUMMONABLE)),
        onoff(is(flags::PRF_DISPMOVE)),
        onoff(is(flags::PRF_COMPACT)),
        onoff(is(flags::PRF_QUEST)),
        onoff(is(flags::PRF_DISPMANA)),
        onoff(is(flags::PRF_NOTELL)),
        onoff(is(flags::PRF_NOREPEAT)),
        onoff(is(flags::PRF_AUTOEXIT)),
        onoff(is(flags::PRF_NOSHOUT)),
        wimp_str,
        onoff(is(flags::PRF_NOGOSS)),
        onoff(is(flags::PRF_NOAUCT)),
        onoff(is(flags::PRF_NOGRATZ)),
        onoff(is(flags::PRF_AUTOLOOT)),
        onoff(is(flags::PRF_AUTOGOLD)),
        onoff(is(flags::PRF_AUTOSPLIT)),
        onoff(is(flags::PRF_AUTOSAC)),
        onoff(is(flags::PRF_AUTOASSIST)),
        onoff(is(flags::PRF_AUTOMAP)),
        page_length,
        screen_width,
        onoff(is(flags::PRF_AFK)),
        onoff(is(flags::PRF_AUTOKEY)),
        onoff(is(flags::PRF_AUTODOOR)),
        TYPES[(is(flags::PRF_COLOR_1) as usize) + 2 * (is(flags::PRF_COLOR_2) as usize)]
    ));
    send_to_char(g, chid, out.as_bytes());
}

fn toggle_color(g: &mut Game, chid: CharId, value: &[u8]) {
    const TYPES: [&[u8]; 4] = [b"off", b"brief", b"normal", b"on"];
    if value.is_empty() {
        let lvl = g.ch(chid).color_lev() as usize;
        let mut msg = b"Your current color level is ".to_vec();
        msg.extend_from_slice(TYPES[lvl]);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let Some(tp) = TYPES.iter().position(|t| t.starts_with(value)) else {
        send_to_char(g, chid, b"Usage: toggle color { Off | Brief | Normal | On }\r\n");
        return;
    };
    {
        let ps = g.ch_mut(chid).ps_mut();
        ps.pref.remove(flags::PRF_COLOR_1);
        ps.pref.remove(flags::PRF_COLOR_2);
        if tp & 1 != 0 {
            ps.pref.set(flags::PRF_COLOR_1);
        }
        if tp & 2 != 0 {
            ps.pref.set(flags::PRF_COLOR_2);
        }
    }
    let red = cc(g, chid, comm::C_SPR, KRED).to_vec();
    // CCNRM at level 0 always emits (quirk: color-off players get the reset).
    let mut msg = b"Your ".to_vec();
    msg.extend_from_slice(&red);
    msg.extend_from_slice(b"color");
    msg.extend_from_slice(KNRM);
    msg.extend_from_slice(b" is now ");
    msg.extend_from_slice(TYPES[tp]);
    msg.extend_from_slice(b".\r\n");
    send_to_char(g, chid, &msg);
}

fn toggle_syslog(g: &mut Game, chid: CharId, value: &[u8]) {
    const TYPES: [&[u8]; 4] = [b"off", b"brief", b"normal", b"on"];
    let cur = {
        let ch = g.ch(chid);
        (ch.prf(flags::PRF_LOG1) as usize) + 2 * (ch.prf(flags::PRF_LOG2) as usize)
    };
    if value.is_empty() {
        let mut msg = b"Your syslog is currently ".to_vec();
        msg.extend_from_slice(TYPES[cur]);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let Some(tp) = TYPES.iter().position(|t| t.starts_with(value)) else {
        send_to_char(g, chid, b"Usage: toggle syslog { Off | Brief | Normal | On }\r\n");
        return;
    };
    {
        let ps = g.ch_mut(chid).ps_mut();
        ps.pref.remove(flags::PRF_LOG1);
        ps.pref.remove(flags::PRF_LOG2);
        if tp & 1 != 0 {
            ps.pref.set(flags::PRF_LOG1);
        }
        if tp & 2 != 0 {
            ps.pref.set(flags::PRF_LOG2);
        }
    }
    let mut msg = b"Your syslog is now ".to_vec();
    msg.extend_from_slice(TYPES[tp]);
    msg.extend_from_slice(b".\r\n");
    send_to_char(g, chid, &msg);
}

/// The sort_as permutation (sort_commands / cmd_sort_info).
fn cmd_sort_order(g: &Game) -> Vec<usize> {
    let mut sorted: Vec<usize> = (1..g.commands.len()).collect();
    sorted.sort_by(|a, b| crate::text::cmp_ci(&g.commands[*a].sort_as, &g.commands[*b].sort_as));
    sorted
}

/// column_list: column-MAJOR layout, paged. With
/// `show_nums` every entry is prefixed `%2d) ` and the column narrows by
/// four — that is the form the OLC flag menus use.
pub fn column_list(
    g: &mut Game,
    chid: CharId,
    num_cols_arg: usize,
    list: &[BStr],
    show_nums: bool,
) {
    let max_len = list.iter().map(|n| n.len()).max().unwrap_or(0);
    let screen_width = if g.ch(chid).is_npc() { 80 } else { g.ch(chid).ps().screen_width } as usize;
    let mut num_cols = if num_cols_arg == 0 {
        screen_width / (max_len + if show_nums { 5 } else { 1 })
    } else {
        num_cols_arg
    };
    num_cols = num_cols.clamp(1, 10);
    let mut col_width = (screen_width / num_cols) as i32;
    if show_nums {
        col_width -= 4;
    }
    if col_width < 0 || (col_width as usize) < max_len {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        g.log(format!(
            "Warning: columns too narrow for correct output to {} in simple_column_list (utils.c)",
            name
        ));
    }
    let pad = col_width.max(0) as usize;
    let num_per_col = list.len().div_ceil(num_cols);
    let mut out: BStr = Vec::new();
    for r in 0..num_per_col {
        for c in 0..num_cols {
            let offset = c * num_per_col + r;
            if offset < list.len() {
                if show_nums {
                    out.extend_from_slice(format!("{:2}) ", offset + 1).as_bytes());
                }
                out.extend_from_slice(&pad_right(&list[offset], pad));
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    page_string(g, chid, &out);
}

/// do_commands: commands/socials; wizhelp is
/// its own function.
pub fn do_commands(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_SOCIALS;
    if subcmd == 2 {
        do_wizhelp(g, chid);
        return;
    }
    let level = g.ch(chid).level;
    let socials = subcmd == SCMD_SOCIALS;
    let mut msg = b"The following ".to_vec();
    msg.extend_from_slice(if socials { b"socials" as &[u8] } else { b"commands" });
    msg.extend_from_slice(b" are available to you:\r\n");
    send_to_char(g, chid, &msg);

    let mut names: Vec<BStr> = Vec::new();
    for idx in cmd_sort_order(g) {
        let entry = &g.commands[idx];
        if entry.minimum_level > level || entry.minimum_level >= LVL_IMMORT {
            continue;
        }
        let is_social = matches!(entry.handler, Handler::Action);
        if socials != is_social {
            continue;
        }
        names.push(entry.command.clone());
    }
    column_list(g, chid, 0, &names, false);
}

fn do_wizhelp(g: &mut Game, chid: CharId) {
    if g.ch(chid).desc.is_none() {
        return;
    }
    send_to_char(g, chid, b"The following privileged commands are available:\r\n");
    let order = cmd_sort_order(g);
    for level in (LVL_IMMORT..=LVL_IMPL).rev() {
        let cyn = cc(g, chid, C_NRM, KCYN).to_vec();
        let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
        let mut out = cyn;
        out.extend_from_slice(format!("Level {}", level).as_bytes());
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b":\r\n");
        let mut no = 1;
        for &idx in &order {
            let entry = &g.commands[idx];
            if entry.minimum_level != level {
                continue;
            }
            out.extend_from_slice(&pad_right(&entry.command, 14));
            if no % 7 == 0 {
                out.extend_from_slice(b"\r\n");
            }
            no += 1;
        }
        if no % 7 != 1 {
            out.extend_from_slice(b"\r\n");
        }
        if level != LVL_IMMORT {
            out.extend_from_slice(b"\r\n");
        }
        send_to_char(g, chid, &out);
    }
}

pub fn do_gen_ps(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::*;
    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"Not for mobiles!\r\n");
        return;
    }
    match subcmd {
        SCMD_CREDITS => {
            let t = g.texts.credits.clone();
            page_string(g, chid, &t);
        }
        SCMD_NEWS => {
            g.ch_mut(chid).ps_mut().last_news = g.now;
            let t = g.texts.news.clone();
            page_string(g, chid, &t);
        }
        SCMD_INFO => {
            let t = g.texts.info.clone();
            page_string(g, chid, &t);
        }
        SCMD_WIZLIST => {
            let t = g.texts.wizlist.clone();
            page_string(g, chid, &t);
        }
        SCMD_IMMLIST => {
            let t = g.texts.immlist.clone();
            page_string(g, chid, &t);
        }
        SCMD_HANDBOOK => {
            let t = g.texts.handbook.clone();
            page_string(g, chid, &t);
        }
        SCMD_POLICIES => {
            let t = g.texts.policies.clone();
            page_string(g, chid, &t);
        }
        SCMD_MOTD => {
            g.ch_mut(chid).ps_mut().last_motd = g.now;
            let t = g.texts.motd.clone();
            page_string(g, chid, &t);
        }
        SCMD_IMOTD => {
            let t = g.texts.imotd.clone();
            page_string(g, chid, &t);
        }
        SCMD_CLEAR => send_to_char(g, chid, b"\x1B[H\x1B[J"),
        SCMD_VERSION => {
            send_to_char(g, chid, tables::TBAMUD_VERSION.as_bytes());
            send_to_char(g, chid, b"\r\n");
        }
        SCMD_WHOAMI => {
            let name = g.ch(chid).name.clone().unwrap_or_default();
            send_to_char(g, chid, &name);
            send_to_char(g, chid, b"\r\n");
        }
        _ => {
            g.log("SYSERR: Unhandled case in do_gen_ps. (?)".to_string());
        }
    }
}

// communication history ----

pub const HIST_ALL: usize = 0;
pub const HIST_SAY: usize = 1;
pub const HIST_GOSSIP: usize = 2;
pub const HIST_WIZNET: usize = 3;
pub const HIST_TELL: usize = 4;
pub const HIST_SHOUT: usize = 5;
pub const HIST_GRATS: usize = 6;
pub const HIST_HOLLER: usize = 7;
pub const HIST_AUCTION: usize = 8;
pub const NUM_HIST: usize = 9;
pub const HIST_LENGTH: usize = 100;

pub fn add_history(g: &mut Game, chid: CharId, msg: &[u8], hist_type: usize) {
    if g.ch(chid).is_npc() {
        return;
    }
    let local = (g.now + g.tz_offset_secs).rem_euclid(86400);
    let (hh, mm) = (local / 3600, (local / 60) % 60);
    let mut line = format!("{:02}:{:02} ", hh, mm).into_bytes();
    line.extend_from_slice(msg);
    let hist = &mut g.ch_mut(chid).ps_mut().comm_hist[hist_type];
    hist.push(line.clone());
    if hist.len() > HIST_LENGTH {
        hist.remove(0);
    }
    if hist_type != HIST_ALL {
        let all = &mut g.ch_mut(chid).ps_mut().comm_hist[HIST_ALL];
        all.push(line);
        if all.len() > HIST_LENGTH {
            all.remove(0);
        }
    }
}

pub fn do_history(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let type_ = tables::HISTORY_TYPES.iter().position(|t| {
        !arg.is_empty() && t.as_bytes().starts_with(&arg[..])
    });
    let Some(type_) = type_ else {
        let mut usage = b"Usage: history <".to_vec();
        for (i, t) in tables::HISTORY_TYPES.iter().enumerate() {
            usage.push(b' ');
            usage.extend_from_slice(t.as_bytes());
            usage.push(b' ');
            if i != tables::HISTORY_TYPES.len() - 1 {
                usage.push(b'|');
            }
        }
        usage.extend_from_slice(b">\r\n");
        send_to_char(g, chid, &usage);
        return;
    };
    let hist = g.ch(chid).ps().comm_hist[type_].clone();
    if hist.is_empty() {
        send_to_char(g, chid, b"You have no history in that channel.\r\n");
    } else {
        for line in hist {
            send_to_char(g, chid, &line);
        }
    }
}

/// page_string for a descriptor whose char may be mid-menu (background story).
pub fn page_string_desc(g: &mut Game, di: usize, text: &[u8]) {
    let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) else {
        // No char: page with defaults.
        let allowed = comm::color_allowed_for_desc(g, di);
        g.descriptors.page_string(di, text, 22, 80, false, allowed);
        return;
    };
    page_string(g, chid, text);
}

/// page_string through the descriptor pager.
pub fn page_string(g: &mut Game, chid: CharId, text: &[u8]) {
    let Some(di) = g.ch(chid).desc else { return };
    let (page_length, screen_width, compact) = {
        let ch = g.ch(chid);
        let ps = ch.ps();
        let mut pl = ps.page_length;
        if pl < 5 {
            pl = 22;
        }
        (pl, ps.screen_width, ch.prf(flags::PRF_COMPACT))
    };
    if page_length != g.ch(chid).ps().page_length && g.ch(chid).ps().page_length < 5 {
        g.ch_mut(chid).ps_mut().page_length = 22;
    }
    let allowed = comm::color_allowed_for_desc(g, di);
    g.descriptors.page_string(di, text, page_length, screen_width, compact, allowed);
}

/// do_practice — the non-guild half.
pub fn do_practice(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let (arg, _) = one_argument(argument);
    if !arg.is_empty() {
        send_to_char(g, chid, b"You can only practice skills in your guild.\r\n");
    } else {
        list_skills(g, chid);
    }
}

fn how_good(percent: i32) -> &'static str {
    if percent < 0 {
        " (error)"
    } else if percent == 0 {
        " (not learned)"
    } else if percent <= 10 {
        " (awful)"
    } else if percent <= 20 {
        " (bad)"
    } else if percent <= 40 {
        " (poor)"
    } else if percent <= 55 {
        " (average)"
    } else if percent <= 70 {
        " (fair)"
    } else if percent <= 80 {
        " (good)"
    } else if percent <= 85 {
        " (very good)"
    } else {
        " (superb)"
    }
}

/// list_skills: alphabetical (sort_spells) listing of
/// everything the class knows at this level, with how_good labels.
pub fn list_skills(g: &mut Game, chid: CharId) {
    let class = g.ch(chid).class.clamp(0, 3) as usize;
    let level = g.ch(chid).level as i32;
    let practices = g.ch(chid).ps().practices;
    let splskl = if mud_data::tables::PRAC_PARAMS[3][class] == 0 { "spell" } else { "skill" };

    let mut out = format!(
        "You have {} practice session{} remaining.\r\nYou know of the following {}s:\r\n",
        practices,
        if practices == 1 { "" } else { "s" },
        splskl
    )
    .into_bytes();

    // sort_spells: indices 1..MAX_SKILLS by name.
    let mut sorted: Vec<i32> = (1..=mud_data::types::MAX_SKILLS as i32).collect();
    sorted.sort_by(|&a, &b| {
        mud_data::spells::spell_info(a)
            .name
            .as_bytes()
            .cmp(mud_data::spells::spell_info(b).name.as_bytes())
    });

    for i in sorted {
        let info = mud_data::spells::spell_info(i);
        if level >= info.min_level[class] {
            let skill = g.ch(chid).get_skill(i);
            out.extend_from_slice(format!("{:<20} {}\r\n", info.name, how_good(skill)).as_bytes());
        }
    }
    page_string(g, chid, &out);
}

const USERS_FORMAT: &[u8] =
    b"format: users [-l minlevel[-maxlevel]] [-n name] [-h host] [-c classlist] [-o] [-p]\r\n";

pub fn do_users(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::interpreter::half_chop;

    let mut host_search: BStr = Vec::new();
    let mut name_search: BStr = Vec::new();
    let (mut low, mut high) = (0i32, LVL_IMPL as i32);
    let (mut showclass, mut outlaws, mut playing, mut deadweight) = (0i32, false, false, false);

    let mut buf = argument.to_vec();
    while !buf.is_empty() {
        let (arg, buf1) = half_chop(&buf);
        if arg.first() != Some(&b'-') {
            send_to_char(g, chid, USERS_FORMAT);
            return;
        }
        match arg.get(1).copied().unwrap_or(0) {
            b'o' | b'k' => {
                outlaws = true;
                playing = true;
                buf = buf1;
            }
            b'p' => {
                playing = true;
                buf = buf1;
            }
            b'd' => {
                deadweight = true;
                buf = buf1;
            }
            b'l' => {
                playing = true;
                let (a, rest) = half_chop(&buf1);
                let (x, y) = crate::act::wizard::parse_range_pub(&a);
                low = x;
                if let Some(y) = y {
                    high = y;
                }
                buf = rest;
            }
            b'n' => {
                playing = true;
                let (a, rest) = half_chop(&buf1);
                name_search = a;
                buf = rest;
            }
            b'h' => {
                playing = true;
                let (a, rest) = half_chop(&buf1);
                host_search = a;
                buf = rest;
            }
            b'c' => {
                playing = true;
                let (a, rest) = half_chop(&buf1);
                showclass = find_class_bitvector(&a);
                buf = rest;
            }
            _ => {
                send_to_char(g, chid, USERS_FORMAT);
                return;
            }
        }
    }

    send_to_char(
        g,
        chid,
        b"Num Class   Name         State          Idl   Login\t*   Site\r\n\
--- ------- ------------ -------------- ----- -------- ------------------------\r\n",
    );

    let mut num_can_see = 0;
    let mut out: BStr = Vec::new();
    for di in g.descriptors.order.clone() {
        let Some(d) = g.descriptors.get(di) else { continue };
        let state = d.state;
        let is_playing = d.is_playing();
        let (desc_num, host, login_time) = (d.desc_num, d.host.clone(), d.login_time);
        let original = d.original;
        let character = d.character;

        if state != ConState::Playing && playing {
            continue;
        }
        if state == ConState::Playing && deadweight {
            continue;
        }

        let mut classname: BStr = b"   -   ".to_vec();
        if is_playing {
            let Some(tch) = original.or(character) else { continue };
            if g.try_ch(tch).is_none() {
                continue;
            }
            if !host_search.is_empty()
                && !host.windows(host_search.len().max(1)).any(|w| w == &host_search[..])
            {
                continue;
            }
            if !name_search.is_empty() && !name_search.eq_ignore_ascii_case(g.ch(tch).get_name()) {
                continue;
            }
            if !crate::handler::can_see(g, chid, tch)
                || (g.ch(tch).level as i32) < low
                || (g.ch(tch).level as i32) > high
            {
                continue;
            }
            if outlaws
                && !g.ch(tch).plr(flags::PLR_KILLER)
                && !g.ch(tch).plr(flags::PLR_THIEF)
            {
                continue;
            }
            if showclass != 0 && showclass & (1 << g.ch(tch).class) == 0 {
                continue;
            }
            if g.ch(tch).invis_lev() > g.ch(chid).level as i16 {
                continue;
            }
            // The banner reads the descriptor's *current* body, not `tch`.
            let who = original.or(character).unwrap();
            classname = format!("[{:2} ", g.ch(who).level).into_bytes();
            classname.extend_from_slice(class_abbr(g.ch(who).class));
            classname.push(b']');
        }

        let timestr = {
            let local = (login_time + g.tz_offset_secs).rem_euclid(86400);
            format!("{:02}:{:02}:{:02}", local / 3600, (local / 60) % 60, local % 60)
        };
        let state_str: BStr = if state == ConState::Playing && original.is_some() {
            b"Switched".to_vec()
        } else {
            mud_data::tables::CONNECTED_TYPES
                .get(state as usize)
                .copied()
                .unwrap_or("")
                .as_bytes()
                .to_vec()
        };
        let idletime: BStr = match character {
            Some(c) if state == ConState::Playing && g.try_ch(c).is_some() => {
                let t = g.ch(c).timer * SECS_PER_MUD_HOUR as i32 / SECS_PER_REAL_MIN as i32;
                crate::act::pad_left(t.to_string().as_bytes(), 5)
            }
            _ => b"     ".to_vec(),
        };
        let display_name: BStr = original
            .and_then(|c| g.try_ch(c))
            .and_then(|c| c.name.clone())
            .or_else(|| character.and_then(|c| g.try_ch(c)).and_then(|c| c.name.clone()))
            .unwrap_or_else(|| b"UNDEFINED".to_vec());

        let mut line = format!("{:3} ", desc_num).into_bytes();
        line.extend_from_slice(&crate::act::pad_right(&classname, 7));
        line.push(b' ');
        line.extend_from_slice(&crate::act::pad_right(&display_name, 12));
        line.push(b' ');
        line.extend_from_slice(&crate::act::pad_right(&state_str, 14));
        line.push(b' ');
        line.extend_from_slice(&crate::act::pad_right(&idletime, 3));
        line.push(b' ');
        line.extend_from_slice(&crate::act::pad_right(timestr.as_bytes(), 8));
        line.push(b' ');
        if !host.is_empty() {
            line.push(b'[');
            line.extend_from_slice(&host);
            line.extend_from_slice(b"]\r\n");
        } else {
            line.extend_from_slice(b"[Hostname unknown]\r\n");
        }

        if state != ConState::Playing {
            let mut l2 = cc(g, chid, C_SPR, KGRN).to_vec();
            l2.extend_from_slice(&line);
            l2.extend_from_slice(cc(g, chid, C_SPR, KNRM));
            line = l2;
        }
        let visible = state != ConState::Playing
            || character.is_some_and(|c| {
                g.try_ch(c).is_some() && crate::handler::can_see(g, chid, c)
            });
        if visible {
            out.extend_from_slice(&line);
            num_can_see += 1;
        }
    }
    send_to_char(g, chid, &out);
    send_to_char(g, chid, format!("\r\n{} visible sockets connected.\r\n", num_can_see).as_bytes());
}

// ---------------------------------------------------------------------------
// do_areas
// ---------------------------------------------------------------------------

/// get_zone_levels. Returns None when the zone
/// has neither bound set — the caller prints "All Levels" for that, not the
/// "<Not Set!>" this builds, because `lev_set` is false.
fn get_zone_levels(g: &Game, znum: usize) -> (BStr, bool) {
    let z = &g.world.zones[znum];
    if z.min_level == -1 && z.max_level == -1 {
        return (b"<Not Set!>".to_vec(), false);
    }
    if z.min_level == -1 {
        return (format!("Up to level {}", z.max_level).into_bytes(), true);
    }
    if z.max_level == -1 {
        return (format!("Above level {}", z.min_level).into_bytes(), true);
    }
    (format!("Levels {} to {}", z.min_level, z.max_level).into_bytes(), true)
}

pub fn do_areas(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    let (mut lolev, mut hilev) = (-1i32, -1i32);
    if !arg.is_empty() {
        match arg.iter().position(|&b| b == b'-') {
            Some(dash) => {
                // A leading '-' means "from zero"; a trailing or non-numeric
                // one means "up to 100".
                lolev = if dash == 0 { 0 } else { crate::handler::atoi(&arg) };
                let after = &arg[dash + 1..];
                hilev = match after.first() {
                    Some(c) if c.is_ascii_digit() => crate::handler::atoi(after),
                    _ => 100,
                };
            }
            None => {
                lolev = crate::handler::atoi(&arg);
                hilev = -1;
            }
        }
    }
    if hilev != -1 && lolev > hilev {
        std::mem::swap(&mut lolev, &mut hilev);
    }

    let (nrm, yel, cyn, red) = (
        cc(g, chid, C_SPR, KNRM),
        cc(g, chid, C_SPR, KYEL),
        cc(g, chid, C_SPR, KCYN),
        cc(g, chid, C_SPR, KRED),
    );

    let mut buf: BStr = Vec::new();
    if hilev != -1 {
        buf.extend_from_slice(b"Checking range: ");
        buf.extend_from_slice(yel);
        buf.extend_from_slice(format!("{} to {}", lolev, hilev).as_bytes());
        buf.extend_from_slice(nrm);
        buf.extend_from_slice(b"\r\n");
    } else if lolev != -1 {
        buf.extend_from_slice(b"Checking level: ");
        buf.extend_from_slice(yel);
        buf.extend_from_slice(lolev.to_string().as_bytes());
        buf.extend_from_slice(nrm);
        buf.extend_from_slice(b"\r\n");
    } else {
        buf.extend_from_slice(b"Checking all areas.\r\n");
    }

    let mut zcount = 0;
    let mut overlap_shown = false;
    for i in 0..g.world.zones.len() {
        if !crate::act::wizard::zone_flagged(g, i, flags::ZONE_GRID) {
            continue;
        }
        let (min, max) = (g.world.zones[i].min_level, g.world.zones[i].max_level);
        let mut overlap = false;
        let show = if lolev == -1 {
            true
        } else if hilev == -1 && lolev >= min && lolev <= max {
            true
        } else if hilev != -1 && lolev >= min && hilev <= max {
            true
        } else if hilev != -1
            && ((lolev >= min && lolev <= max) || (hilev <= max && hilev >= min))
        {
            overlap = true;
            true
        } else if max < 0 && lolev >= min {
            true
        } else if max < 0 && hilev >= min {
            overlap = true;
            true
        } else {
            false
        };
        if !show {
            continue;
        }
        if overlap {
            overlap_shown = true;
        }
        zcount += 1;
        let (lev_str, lev_set) = get_zone_levels(g, i);
        let name = g.world.zones[i].name.as_deref().unwrap_or(b"").to_vec();
        // "%-*s" with the width widened by the name's own colour codes, so
        // the visible column lands in the same place either way.
        let width = crate::act::other::count_color_chars(&name) + 30;
        buf.extend_from_slice(b"\tn(");
        buf.extend_from_slice(format!("{:>3}", zcount).as_bytes());
        buf.extend_from_slice(b") ");
        buf.extend_from_slice(if overlap { red } else { cyn });
        buf.extend_from_slice(&name);
        for _ in name.len()..width {
            buf.push(b' ');
        }
        buf.extend_from_slice(b"\tn ");
        buf.extend_from_slice(if lev_set { b"\tc" } else { b"\tn" });
        if lev_set {
            buf.extend_from_slice(&lev_str);
        } else {
            buf.extend_from_slice(b"All Levels");
        }
        buf.extend_from_slice(b"\tn\r\n");
    }

    buf.extend_from_slice(yel);
    buf.extend_from_slice(zcount.to_string().as_bytes());
    buf.extend_from_slice(nrm);
    buf.extend_from_slice(b" area");
    if zcount != 1 {
        buf.push(b's');
    }
    buf.extend_from_slice(b" found.\r\n");

    if overlap_shown {
        buf.extend_from_slice(
            b"Areas shown in \trred\tn may have some creatures outside the specified range.\r\n",
        );
    }

    if zcount == 0 {
        send_to_char(g, chid, b"No areas found.\r\n");
    } else {
        page_string(g, chid, &buf);
    }
}

/// `list_scanned_chars`: the one sentence a scan prints for one room.
///
/// The visible count is taken first so the grammar knows where the "and"
/// goes, and the last name carries the distance phrase and the direction.
///
/// The return value says whether anything was printed. A room whose
/// occupants cannot all be seen counts zero and prints nothing, so
/// `do_scan` must not raise `found` for it -- raising it before knowing
/// left the command answering with complete silence, the "You don't see
/// anything nearby!" fallback included.
fn list_scanned_chars(
    g: &mut Game,
    room: RoomRnum,
    chid: CharId,
    distance: usize,
    door: usize,
) -> bool {
    const HOW_FAR: [&[u8]; 3] = [b"close by", b"a ways off", b"far off to the"];

    let list = g.rooms[room as usize].people.clone();
    let mut count = list.iter().filter(|&&i| can_see(g, chid, i)).count();
    if count == 0 {
        return false;
    }

    let mut buf: BStr = Vec::new();
    for i in list {
        if !can_see(g, chid, i) {
            continue;
        }
        if buf.is_empty() {
            buf.extend_from_slice(b"You see ");
        }
        buf.extend_from_slice(g.ch(i).get_name());
        count -= 1;
        if count > 1 {
            buf.extend_from_slice(b", ");
        } else if count == 1 {
            buf.extend_from_slice(b" and ");
        } else {
            buf.push(b' ');
            buf.extend_from_slice(HOW_FAR[distance]);
            buf.push(b' ');
            buf.extend_from_slice(tables::DIRS[door].as_bytes());
            buf.extend_from_slice(b".\r\n");
        }
    }
    send_to_char(g, chid, &buf);
    true
}

/// `scan`: look up to three rooms along each direction.
///
/// The walk stops at the first missing exit and at any door that is closed
/// or hidden, but a dark room does not stop it -- that room reports itself
/// and the walk carries on through. Note the blind check has no immortal
/// exemption here, unlike `do_exits`, and that the argument is never read.
pub fn do_scan(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    const MAXRANGE: usize = 3;

    if g.ch(chid).aff(flags::AFF_BLIND) {
        send_to_char(g, chid, b"You can't see a damned thing, you're blind!\r\n");
        return;
    }

    let dir_count = crate::fight::dir_count(g);
    let mut found = false;

    for door in 0..dir_count {
        let mut scanned_room = g.ch(chid).in_room;
        for range in 1..=MAXRANGE {
            let next = match g.world.rooms[scanned_room as usize].dir_option[door].as_deref() {
                Some(e)
                    if e.to_room != NOWHERE
                        && e.exit_info & flags::EX_CLOSED == 0
                        && e.exit_info & flags::EX_HIDDEN == 0 =>
                {
                    e.to_room
                }
                _ => break,
            };
            scanned_room = next;
            let occupied = !g.rooms[scanned_room as usize].people.is_empty();
            if room_is_dark(g, scanned_room) && !can_see_in_dark(g, chid) {
                let dir = tables::DIRS[door];
                let line = if occupied {
                    format!("{}: It's too dark to see, but you can hear shuffling.\r\n", dir)
                } else {
                    format!("{}: It is too dark to see anything.\r\n", dir)
                };
                send_to_char(g, chid, line.as_bytes());
                found = true;
            } else if occupied && list_scanned_chars(g, scanned_room, chid, range - 1, door) {
                found = true;
            }
        }
    }

    if !found {
        send_to_char(g, chid, b"You don't see anything nearby!\r\n");
    }
}
