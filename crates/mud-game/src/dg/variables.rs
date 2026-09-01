//! Variable substitution, including the tmpvr chained-access buffer
//! surgery, the persistent subfield accumulator, and the '\x1'
//! unknown-field sentinel flow.

use mud_data::ids::{CharId, ObjId};
use mud_data::tables;
use mud_data::spells::{SAVING_BREATH, SAVING_PARA, SAVING_PETRI, SAVING_ROD, SAVING_SPELL};
use mud_data::types::*;

use super::{
    atoi32, char_script_id, obj_room, obj_script_id, room_script_id, script_log, trig_log,
    DgCtx, GoId, TrigVar, DG_ALLOW_GODS, ROOM_ID_BASE, UID_CHAR,
};
use crate::game::Game;
use crate::handler::{can_see, isname, obj_name, obj_short, eq_ci};

pub type BStr = Vec<u8>;

/// How much of a variable expansion is kept: MAX_INPUT_LENGTH less 20, less
/// one for the terminator.
const REPL_CAP: usize = 256 - 20 - 1;

/// Case-insensitive substring test. An empty needle never matches.
pub fn str_str(cs: &[u8], ct: &[u8]) -> bool {
    if ct.is_empty() {
        return false;
    }
    let lower = |b: u8| b.to_ascii_lowercase();
    let mut i = 0;
    while i < cs.len() {
        while i < cs.len() && lower(cs[i]) != lower(ct[0]) {
            i += 1;
        }
        let mut t = 0;
        while t < ct.len() && i < cs.len() && lower(cs[i]) == lower(ct[t]) {
            t += 1;
            i += 1;
        }
        if t == ct.len() {
            return true;
        }
    }
    false
}

/// is_number: optional '-', then all digits, non-empty.
pub fn c_is_number(s: &[u8]) -> bool {
    crate::interpreter::is_number(s)
}

pub fn find_eq_pos_script(arg: &[u8]) -> i32 {
    const EQ_POS: [(&[u8], i32); 20] = [
        (b"hold", WEAR_HOLD as i32),
        (b"held", WEAR_HOLD as i32),
        (b"light", WEAR_LIGHT as i32),
        (b"wield", WEAR_WIELD as i32),
        (b"rfinger", WEAR_FINGER_R as i32),
        (b"lfinger", WEAR_FINGER_L as i32),
        (b"neck1", WEAR_NECK_1 as i32),
        (b"neck2", WEAR_NECK_2 as i32),
        (b"body", WEAR_BODY as i32),
        (b"head", WEAR_HEAD as i32),
        (b"legs", WEAR_LEGS as i32),
        (b"feet", WEAR_FEET as i32),
        (b"hands", WEAR_HANDS as i32),
        (b"arms", WEAR_ARMS as i32),
        (b"shield", WEAR_SHIELD as i32),
        (b"about", WEAR_ABOUT as i32),
        (b"waist", WEAR_WAIST as i32),
        (b"rwrist", WEAR_WRIST_R as i32),
        (b"lwrist", WEAR_WRIST_L as i32),
        (b"none", -1),
    ];
    if c_is_number(arg) {
        let i = atoi32(arg);
        if (0..NUM_WEARS as i32).contains(&i) {
            return i;
        }
    }
    for (pos, where_) in EQ_POS {
        if where_ != -1 && eq_ci(pos, arg) {
            return where_;
        }
    }
    -1
}

pub fn can_wear_on_pos(g: &Game, oid: ObjId, pos: i32) -> bool {
    use mud_data::flags::*;
    let o = g.obj(oid);
    let w = |bit: usize| o.can_wear(bit);
    match pos as usize {
        WEAR_HOLD | WEAR_LIGHT => w(ITEM_WEAR_HOLD),
        WEAR_WIELD => w(ITEM_WEAR_WIELD),
        WEAR_FINGER_R | WEAR_FINGER_L => w(ITEM_WEAR_FINGER),
        WEAR_NECK_1 | WEAR_NECK_2 => w(ITEM_WEAR_NECK),
        WEAR_BODY => w(ITEM_WEAR_BODY),
        WEAR_HEAD => w(ITEM_WEAR_HEAD),
        WEAR_LEGS => w(ITEM_WEAR_LEGS),
        WEAR_FEET => w(ITEM_WEAR_FEET),
        WEAR_HANDS => w(ITEM_WEAR_HANDS),
        WEAR_ARMS => w(ITEM_WEAR_ARMS),
        WEAR_SHIELD => w(ITEM_WEAR_SHIELD),
        WEAR_ABOUT => w(ITEM_WEAR_ABOUT),
        WEAR_WAIST => w(ITEM_WEAR_WAIST),
        WEAR_WRIST_R | WEAR_WRIST_L => w(ITEM_WEAR_WRIST),
        _ => false,
    }
}

/// item_in_list: counts matches, recursing containers.
pub fn item_in_list(g: &Game, item: &[u8], list: &[ObjId]) -> i32 {
    if item.is_empty() {
        return 0;
    }
    let mut count = 0;
    if item.first() == Some(&UID_CHAR) {
        let id = super::atoi64(&item[1..]);
        for &o in list {
            let Some(ob) = g.try_obj(o) else { continue };
            if ob.script_id == id {
                count += 1;
            }
            if ob.type_flag == mud_data::flags::ITEM_CONTAINER as i32 {
                count += item_in_list(g, item, &ob.contains.clone());
            }
        }
    } else if c_is_number(item) {
        let ovnum = atoi32(item);
        for &o in list {
            let Some(ob) = g.try_obj(o) else { continue };
            if super::obj_vnum(g, o) == ovnum {
                count += 1;
            }
            if ob.type_flag == mud_data::flags::ITEM_CONTAINER as i32 {
                count += item_in_list(g, item, &ob.contains.clone());
            }
        }
    } else {
        for &o in list {
            let Some(ob) = g.try_obj(o) else { continue };
            if isname(item, obj_name(g, o)) {
                count += 1;
            }
            if ob.type_flag == mud_data::flags::ITEM_CONTAINER as i32 {
                count += item_in_list(g, item, &ob.contains.clone());
            }
        }
    }
    count
}

pub fn char_has_item(g: &Game, item: &[u8], chid: CharId) -> bool {
    if super::get_object_in_equip(g, chid, item).is_some() {
        return true;
    }
    item_in_list(g, item, &g.ch(chid).carrying.clone()) != 0
}

pub fn trig_is_attached(g: &Game, go: GoId, trig_num: i32) -> bool {
    match g.script_of(go) {
        Some(sc) => sc
            .trig_list
            .iter()
            .any(|t| g.world.triggers[t.nr as usize].vnum as i32 == trig_num),
        None => false,
    }
}

/// check_flags_by_name_ar: CASE-SENSITIVE exact name.
fn check_flags_by_name_ar(flags: &mud_data::flags::FlagSet, numflags: usize, search: &[u8], namelist: &[&str]) -> bool {
    let mut item = None;
    for (i, name) in namelist.iter().enumerate().take(numflags) {
        if name.as_bytes() == search {
            item = Some(i);
            break;
        }
    }
    match item {
        Some(i) => flags.is_set(i),
        None => false,
    }
}

/// get_flag_by_name: case-insensitive full match; -1 = NOFLAG.
fn get_flag_by_name(namelist: &[&str], name: &[u8]) -> i32 {
    for (i, f) in namelist.iter().enumerate() {
        if *f == "\n" {
            break;
        }
        if eq_ci(f.as_bytes(), name) {
            return i as i32;
        }
    }
    -1
}

/// sprinttype: names[idx] or "UNDEFINED".
fn sprinttype(idx: i32, names: &[&str]) -> BStr {
    names.get(idx as usize).copied().unwrap_or("UNDEFINED").as_bytes().to_vec()
}

/// sprintbit: non-array bitvector names, "NOBITS " when empty.
/// Each flag is followed by a trailing space.
fn sprintbit(bits: u32, names: &[&str]) -> BStr {
    let mut out = Vec::new();
    let mut any = false;
    for i in 0..32 {
        if bits & (1 << i) != 0 {
            any = true;
            let name = names.get(i as usize).copied().unwrap_or("UNDEFINED");
            out.extend_from_slice(name.as_bytes());
            out.push(b' ');
        }
    }
    if !any {
        out.extend_from_slice(b"NOBITS ");
    }
    out
}

/// sprintbitarray equivalent returning the buffer.
fn sprintbitarray(flags: &mud_data::flags::FlagSet, names: &[&str]) -> BStr {
    let mut out = BStr::new();
    crate::act::informative::sprintbitarray(&flags.0, names, &mut out);
    out
}

fn skill_percent(g: &Game, chid: CharId, skill: &[u8]) -> BStr {
    match crate::spec::find_skill_num(skill) {
        Some(n) if n > 0 => g.ch(chid).get_skill(n).to_string().into_bytes(),
        _ => b"unknown skill".to_vec(),
    }
}

/// text_processed. Returns Some(result) if the field is
/// a text field.
pub fn text_processed(g: &Game, field: &[u8], subfield: &[u8], value: &[u8]) -> Option<BStr> {
    let _ = g;
    if eq_ci(field, b"strlen") {
        Some(value.len().to_string().into_bytes())
    } else if eq_ci(field, b"toupper") {
        if value.is_empty() {
            // str is left untouched (empty from the caller's reset).
            Some(Vec::new())
        } else {
            let mut v = value.to_vec();
            v[0] = v[0].to_ascii_uppercase();
            Some(v)
        }
    } else if eq_ci(field, b"trim") {
        let is_sp = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
        let start = value.iter().position(|b| !is_sp(b));
        match start {
            None => Some(Vec::new()),
            Some(s) => {
                let e = value.iter().rposition(|b| !is_sp(b)).unwrap();
                Some(value[s..=e].to_vec())
            }
        }
    } else if eq_ci(field, b"contains") {
        Some(if str_str(value, subfield) { b"1".to_vec() } else { b"0".to_vec() })
    } else if eq_ci(field, b"car") {
        let is_sp = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
        Some(value.iter().take_while(|b| !is_sp(b)).copied().collect())
    } else if eq_ci(field, b"cdr") {
        let is_sp = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
        let mut i = 0;
        while i < value.len() && !is_sp(value[i]) {
            i += 1;
        }
        while i < value.len() && is_sp(value[i]) {
            i += 1;
        }
        Some(value[i..].to_vec())
    } else if eq_ci(field, b"charat") {
        // size_t cindex = atoi(subfield); a negative value wraps huge.
        let idx = atoi32(subfield);
        if idx < 1 || idx as usize > value.len() {
            Some(Vec::new())
        } else {
            Some(vec![value[idx as usize - 1]])
        }
    } else if eq_ci(field, b"mudcommand") {
        // Case-SENSITIVE prefix over the base table (cmd_info, no socials).
        let base = crate::interpreter::base_command_table();
        let found = base.iter().find(|e| e.command.starts_with(value));
        Some(match found {
            Some(e) => e.command.clone(),
            None => Vec::new(),
        })
    } else {
        None
    }
}

fn trgvar_in_room(g: &mut Game, vnum: i32) -> i32 {
    match g.real_room(vnum) {
        None => {
            script_log(g, "people.vnum: world[rnum] does not exist");
            -1
        }
        Some(r) => g.rooms[r as usize].people.len() as i32,
    }
}

fn uid_str(id: i64) -> BStr {
    format!("}}{}", id).into_bytes()
}

/// The command-alias tables of §8.1, indexed by attach type.
fn cmd_alias(var: &[u8], type_: i32) -> Option<&'static [u8]> {
    let t = type_ as usize;
    macro_rules! tab {
        ($name:literal, $m:literal, $o:literal, $w:literal) => {
            if eq_ci(var, $name) {
                return Some([$m as &[u8], $o, $w][t]);
            }
        };
    }
    tab!(b"door", b"mdoor ", b"odoor ", b"wdoor ");
    tab!(b"force", b"mforce ", b"oforce ", b"wforce ");
    tab!(b"load", b"mload ", b"oload ", b"wload ");
    tab!(b"purge", b"mpurge ", b"opurge ", b"wpurge ");
    tab!(b"teleport", b"mteleport ", b"oteleport ", b"wteleport ");
    tab!(b"damage", b"mdamage ", b"odamage ", b"wdamage ");
    tab!(b"send", b"msend ", b"osend ", b"wsend ");
    tab!(b"echo", b"mecho ", b"oecho ", b"wecho ");
    tab!(b"echoaround", b"mechoaround ", b"oechoaround ", b"wechoaround ");
    tab!(b"zoneecho", b"mzoneecho ", b"ozoneecho ", b"wzoneecho ");
    tab!(b"asound", b"masound ", b"oasound ", b"wasound ");
    tab!(b"at", b"mat ", b"oat ", b"wat ");
    tab!(b"transform", b"mtransform ", b"otransform ", b"wecho ");
    tab!(b"recho", b"mrecho ", b"orecho ", b"wrecho ");
    tab!(b"move", b"mecho ", b"omove ", b"wmove ");
    tab!(b"log", b"mlog ", b"olog ", b"wlog ");
    None
}

/// find_replacement. `field` empty = no field.
pub fn find_replacement(
    g: &mut Game,
    ctx: DgCtx,
    var: &[u8],
    field: &[u8],
    subfield: &[u8],
) -> BStr {
    let type_ = ctx.go.kind();
    let mut out: BStr;

    // 1. trigger locals (name only), 2. script globals (name+context).
    let mut vd: Option<TrigVar> = g
        .trig(ctx.go, ctx.iid)
        .and_then(|t| t.var_list.iter().find(|v| eq_ci(&v.name, var)).cloned());
    if vd.is_none() {
        if let Some(sc) = g.script_of(ctx.go) {
            let sc_context = sc.context;
            vd = sc
                .global_vars
                .iter()
                .find(|v| eq_ci(&v.name, var) && (v.context == 0 || v.context == sc_context))
                .cloned();
        }
    }

    if field.is_empty() {
        if let Some(vd) = vd {
            out = vd.value.clone();
        } else if eq_ci(var, b"self") {
            out = match ctx.go {
                GoId::Char(id) => uid_str(char_script_id(g, id)),
                GoId::Obj(id) => uid_str(obj_script_id(g, id)),
                GoId::Room(r) => uid_str(room_script_id(g, r)),
            };
        } else if eq_ci(var, b"global") {
            out = ROOM_ID_BASE.to_string().into_bytes();
        } else if let Some(alias) = cmd_alias(var, type_) {
            out = alias.to_vec();
        } else {
            out = Vec::new();
        }
        out.truncate(REPL_CAP);
        return out;
    }

    // Text fields on a found var's value.
    if let Some(ref v) = vd {
        if let Some(res) = text_processed(g, field, subfield, &v.value) {
            let mut res = res;
            res.truncate(REPL_CAP);
            return res;
        }
    }

    // Resolve to an entity: c / o / r.
    let mut c: Option<CharId> = None;
    let mut o: Option<ObjId> = None;
    let mut r: Option<RoomRnum> = None;

    if let Some(ref v) = vd {
        let name = v.value.clone();
        match ctx.go {
            GoId::Char(chid) => {
                if let Some(eq) = super::get_object_in_equip(g, chid, &name) {
                    o = Some(eq);
                } else if let Some(inv) = super::get_obj_in_list(g, &name, &g.ch(chid).carrying.clone()) {
                    o = Some(inv);
                } else {
                    let in_room = g.ch(chid).in_room;
                    if in_room != NOWHERE {
                        if let Some(cc) = super::get_char_in_room(g, in_room, &name) {
                            c = Some(cc);
                        }
                    }
                    if c.is_none() {
                        if in_room != NOWHERE {
                            if let Some(oo) =
                                super::get_obj_in_list(g, &name, &g.rooms[in_room as usize].contents.clone())
                            {
                                o = Some(oo);
                            }
                        }
                        if o.is_none() {
                            if let Some(cc) = super::get_char(g, &name) {
                                c = Some(cc);
                            } else if let Some(oo) = super::get_obj(g, &name) {
                                o = Some(oo);
                            } else if let Some(rr) = super::get_room(g, &name) {
                                r = Some(rr);
                            }
                        }
                    }
                }
            }
            GoId::Obj(oid) => {
                if let Some(cc) = super::get_char_by_obj(g, oid, &name) {
                    c = Some(cc);
                } else if let Some(oo) = super::get_obj_by_obj(g, oid, &name) {
                    o = Some(oo);
                } else if let Some(rr) = super::get_room(g, &name) {
                    r = Some(rr);
                }
            }
            GoId::Room(room) => {
                if let Some(cc) = super::get_char_by_room(g, room, &name) {
                    c = Some(cc);
                } else if let Some(oo) = super::get_obj_by_room(g, room, &name) {
                    o = Some(oo);
                } else if let Some(rr) = super::get_room(g, &name) {
                    r = Some(rr);
                }
            }
        }
    } else {
        // Unfound var + field: special names.
        if eq_ci(var, b"self") {
            match ctx.go {
                GoId::Char(id) => c = Some(id),
                GoId::Obj(id) => o = Some(id),
                GoId::Room(rr) => r = Some(rr),
            }
        } else if eq_ci(var, b"global") {
            // Void-room global read (§6.4).
            let Some(sc) = g.rooms[0].script.as_deref() else {
                script_log(g, "Attempt to find global var. Apparently the void has no script.");
                return Vec::new();
            };
            let found = sc.global_vars.iter().find(|v| eq_ci(&v.name, field)).cloned();
            let mut res = found.map(|v| v.value).unwrap_or_default();
            res.truncate(REPL_CAP);
            return res;
        } else if eq_ci(var, b"people") {
            let num = atoi32(field);
            let n = if num > 0 { trgvar_in_room(g, num) } else { 0 };
            return n.to_string().into_bytes();
        } else if eq_ci(var, b"happyhour") {
            // Each rate subfield is gated on
            // IS_HAPPYHOUR; anything else (including "time") falls through
            // to the raw tick counter.
            let live = crate::act::other::is_happyhour(g);
            let v = if eq_ci(field, b"qp") && live {
                g.happy.qp_rate
            } else if eq_ci(field, b"exp") && live {
                g.happy.exp_rate
            } else if eq_ci(field, b"gold") && live {
                g.happy.gold_rate
            } else {
                g.happy.ticks_left
            };
            return v.to_string().into_bytes();
        } else if eq_ci(var, b"time") {
            let ti = g.time_info;
            return if eq_ci(field, b"hour") {
                ti.hours.to_string().into_bytes()
            } else if eq_ci(field, b"day") {
                (ti.day + 1).to_string().into_bytes()
            } else if eq_ci(field, b"month") {
                (ti.month + 1).to_string().into_bytes()
            } else if eq_ci(field, b"year") {
                ti.year.to_string().into_bytes()
            } else {
                Vec::new()
            };
        } else if eq_ci(var, b"findmob") {
            if field.is_empty() || subfield.is_empty() {
                script_log(g, "findmob.vnum(mvnum) - illegal syntax");
                return b"0".to_vec();
            }
            match g.real_room(atoi32(field)) {
                None => {
                    script_log(g, &format!("findmob.vnum(ovnum): No room with vnum {}", atoi32(field)));
                    return b"0".to_vec();
                }
                Some(rr) => {
                    let mvnum = atoi32(subfield);
                    let people = g.rooms[rr as usize].people.clone();
                    let n = people.iter().filter(|&&ch| super::mob_vnum(g, ch) == mvnum).count();
                    return n.to_string().into_bytes();
                }
            }
        } else if eq_ci(var, b"findobj") {
            if field.is_empty() || subfield.is_empty() {
                script_log(g, "findobj.vnum(ovnum) - illegal syntax");
                return b"0".to_vec();
            }
            match g.real_room(atoi32(field)) {
                None => {
                    script_log(g, &format!("findobj.vnum(ovnum): No room with vnum {}", atoi32(field)));
                    return b"0".to_vec();
                }
                Some(rr) => {
                    let contents = g.rooms[rr as usize].contents.clone();
                    return item_in_list(g, subfield, &contents).to_string().into_bytes();
                }
            }
        } else if eq_ci(var, b"random") {
            if eq_ci(field, b"char") {
                let mut rndm: Option<CharId> = None;
                let mut count = 0;
                match ctx.go {
                    GoId::Char(chid) => {
                        let in_room = g.ch(chid).in_room;
                        if in_room != NOWHERE {
                            let people = g.rooms[in_room as usize].people.clone();
                            for cand in people {
                                if cand != chid
                                    && super::misc::valid_dg_target(g, cand, DG_ALLOW_GODS)
                                    && can_see(g, chid, cand)
                                {
                                    if g.rng.rand_number(0, count) == 0 {
                                        rndm = Some(cand);
                                    }
                                    count += 1;
                                }
                            }
                        }
                    }
                    GoId::Obj(oid) => {
                        let room = obj_room(g, oid);
                        if room != NOWHERE {
                            let people = g.rooms[room as usize].people.clone();
                            for cand in people {
                                if super::misc::valid_dg_target(g, cand, DG_ALLOW_GODS) {
                                    if g.rng.rand_number(0, count) == 0 {
                                        rndm = Some(cand);
                                    }
                                    count += 1;
                                }
                            }
                        }
                    }
                    GoId::Room(room) => {
                        let people = g.rooms[room as usize].people.clone();
                        for cand in people {
                            if super::misc::valid_dg_target(g, cand, DG_ALLOW_GODS) {
                                if g.rng.rand_number(0, count) == 0 {
                                    rndm = Some(cand);
                                }
                                count += 1;
                            }
                        }
                    }
                }
                return match rndm {
                    Some(ch) => uid_str(char_script_id(g, ch)),
                    None => Vec::new(),
                };
            } else if eq_ci(field, b"dir") {
                let in_room = match ctx.go {
                    GoId::Room(rr) => Some(rr),
                    GoId::Obj(oid) => {
                        let rm = obj_room(g, oid);
                        (rm != NOWHERE).then_some(rm)
                    }
                    GoId::Char(chid) => {
                        let rm = g.ch(chid).in_room;
                        (rm != NOWHERE).then_some(rm)
                    }
                };
                let Some(rr) = in_room else { return Vec::new() };
                let dir_count = crate::fight::dir_count(g) as usize;
                let doors = (0..dir_count)
                    .filter(|&i| g.world.rooms[rr as usize].dir_option[i].is_some())
                    .count();
                if doors == 0 {
                    return Vec::new();
                }
                loop {
                    let d = g.rng.rand_number(0, dir_count as i32 - 1) as usize;
                    if g.world.rooms[rr as usize].dir_option[d].is_some() {
                        return tables::DIRS[d].as_bytes().to_vec();
                    }
                }
            } else {
                let num = atoi32(field);
                let n = if num > 0 { g.rng.rand_number(1, num) } else { 0 };
                return n.to_string().into_bytes();
            }
        }
    }

    // ---- character fields ----
    if let Some(c) = c {
        // The %x.global(y)% recursion is dead: its result is clobbered by
        // the '\x1' sentinel and the field falls to the unknown path.
        let res = char_field(g, ctx, c, field, subfield);
        let mut res = match res {
            Some(v) => v,
            None => unknown_char_field(g, ctx, c, field),
        };
        res.truncate(REPL_CAP);
        return res;
    }

    // ---- object fields ----
    if let Some(o) = o {
        let res = obj_field(g, ctx, o, field, subfield);
        let mut res = match res {
            Some(v) => v,
            None => unknown_obj_field(g, ctx, o, field),
        };
        res.truncate(REPL_CAP);
        return res;
    }

    // ---- room fields ----
    if let Some(r) = r {
        let mut res = room_field(g, ctx, r, field, subfield);
        res.truncate(REPL_CAP);
        return res;
    }

    Vec::new()
}

/// Unknown char field: the entity's own globals, else log + empty.
fn unknown_char_field(g: &mut Game, ctx: DgCtx, c: CharId, field: &[u8]) -> BStr {
    if let Some(sc) = g.ch(c).script.as_deref() {
        if let Some(v) = sc.global_vars.iter().find(|v| eq_ci(&v.name, field)) {
            return v.value.clone();
        }
    }
    let msg = format!("unknown char field: '{}'", String::from_utf8_lossy(field));
    trig_log(g, ctx.go, ctx.iid, &msg);
    Vec::new()
}

fn unknown_obj_field(g: &mut Game, ctx: DgCtx, o: ObjId, field: &[u8]) -> BStr {
    if let Some(sc) = g.obj(o).script.as_deref() {
        if let Some(v) = sc.global_vars.iter().find(|v| eq_ci(&v.name, field)) {
            return v.value.clone();
        }
    }
    let (name, vnum) = trig_ident(g, ctx);
    script_log(
        g,
        &format!(
            "Trigger: {}, VNum {}, type: {}. unknown object field: '{}'",
            name,
            vnum,
            ctx.go.kind(),
            String::from_utf8_lossy(field)
        ),
    );
    Vec::new()
}

pub(super) fn trig_ident(g: &Game, ctx: DgCtx) -> (String, i32) {
    match g.trig(ctx.go, ctx.iid) {
        Some(t) => (
            String::from_utf8_lossy(&t.name).into_owned(),
            g.world.triggers[t.nr as usize].vnum as i32,
        ),
        None => ("<unknown>".into(), 0),
    }
}

/// Ability-score adjust helper: clamp real, run affect_total, return affected.
fn adjust_ability(g: &mut Game, c: CharId, which: u8, addition: i32) {
    let (is_npc, level) = {
        let ch = g.ch(c);
        (ch.is_npc(), ch.level)
    };
    let max = if is_npc || level as i32 >= LVL_GRGOD as i32 { 25 } else { 18 };
    {
        let ch = g.ch_mut(c);
        let slot = match which {
            b's' => &mut ch.real_abils.str_,
            b'i' => &mut ch.real_abils.intel,
            b'w' => &mut ch.real_abils.wis,
            b'd' => &mut ch.real_abils.dex,
            b'c' => &mut ch.real_abils.con,
            _ => &mut ch.real_abils.cha,
        };
        let mut v = *slot as i32 + addition;
        if v > max {
            v = max;
        }
        if v < 3 {
            v = 3;
        }
        *slot = v as i8;
    }
    crate::handler::affect_total(g, c);
}

/// All char fields. Returns None = unknown field.
fn char_field(g: &mut Game, ctx: DgCtx, c: CharId, field: &[u8], subfield: &[u8]) -> Option<BStr> {
    use mud_data::flags;
    let has_sub = !subfield.is_empty();

    if eq_ci(field, b"affect") {
        if has_sub {
            let spell = crate::spec::find_skill_num(subfield).unwrap_or(-1);
            Some(if spell > 0 && crate::handler::affected_by_spell(g, c, spell as i16) {
                b"1".to_vec()
            } else {
                b"0".to_vec()
            })
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"alias") {
        Some(g.ch(c).name.clone().unwrap_or_default())
    } else if eq_ci(field, b"align") {
        if has_sub {
            let a = atoi32(subfield);
            g.ch_mut(c).alignment = (-1000).max(a.min(1000));
        }
        Some(g.ch(c).alignment.to_string().into_bytes())
    } else if eq_ci(field, b"armor") {
        Some(crate::act::informative::compute_armor_class(g, c).to_string().into_bytes())
    } else if eq_ci(field, b"canbeseen") {
        Some(match ctx.go {
            GoId::Char(owner) if !can_see(g, owner, c) => b"0".to_vec(),
            _ => b"1".to_vec(),
        })
    } else if eq_ci(field, b"cha") {
        if has_sub {
            adjust_ability(g, c, b'h', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.cha.to_string().into_bytes())
    } else if eq_ci(field, b"class") {
        if has_sub {
            let mut cl = -1;
            for (i, name) in crate::act::informative::PC_CLASS_TYPES.iter().enumerate() {
                if crate::handler::is_abbrev(subfield, name) {
                    cl = i as i32;
                    break;
                }
            }
            if cl != -1 {
                g.ch_mut(c).class = cl as i8;
                Some(b"1".to_vec())
            } else {
                Some(b"0".to_vec())
            }
        } else {
            let cl = g.ch(c).class as i32;
            Some(
                crate::act::informative::PC_CLASS_TYPES
                    .get(cl as usize)
                    .copied()
                    .unwrap_or(b"UNDEFINED")
                    .to_vec(),
            )
        }
    } else if eq_ci(field, b"con") {
        if has_sub {
            adjust_ability(g, c, b'c', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.con.to_string().into_bytes())
    } else if eq_ci(field, b"damroll") {
        if has_sub {
            let ch = g.ch_mut(c);
            ch.points.damroll = 1.max(ch.points.damroll as i32 + atoi32(subfield)) as i8;
        }
        Some(g.ch(c).points.damroll.to_string().into_bytes())
    } else if eq_ci(field, b"dex") {
        if has_sub {
            adjust_ability(g, c, b'd', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.dex.to_string().into_bytes())
    } else if eq_ci(field, b"drunk") {
        if has_sub {
            let v = (-1).max(atoi32(subfield).min(24));
            if let Some(ps) = g.ch_mut(c).player_specials.as_mut() {
                ps.conditions[crate::ch::DRUNK] = v as i16;
            }
        }
        Some(get_cond(g, c, crate::ch::DRUNK).to_string().into_bytes())
    } else if eq_ci(field, b"eq") {
        if !has_sub {
            Some(Vec::new())
        } else if subfield[0] == b'*' {
            let any = g.ch(c).equipment.iter().any(|e| e.is_some());
            Some(if any { b"1".to_vec() } else { Vec::new() })
        } else {
            let pos = find_eq_pos_script(subfield);
            if pos < 0 {
                Some(Vec::new())
            } else {
                match g.ch(c).equipment[pos as usize] {
                    None => Some(Vec::new()),
                    Some(eq) => Some(uid_str(obj_script_id(g, eq))),
                }
            }
        }
    } else if eq_ci(field, b"exp") {
        if has_sub {
            let addition = atoi32(subfield).min(1000);
            crate::limits::gain_exp(g, c, addition);
        }
        Some(g.ch(c).points.exp.to_string().into_bytes())
    } else if eq_ci(field, b"fighting") {
        Some(match g.ch(c).fighting {
            Some(f) => uid_str(char_script_id(g, f)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"follower") {
        Some(match g.ch(c).followers.first().copied() {
            Some(f) => uid_str(char_script_id(g, f)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"gold") {
        if has_sub {
            crate::limits::increase_gold(g, c, atoi32(subfield));
        }
        Some(g.ch(c).points.gold.to_string().into_bytes())
    } else if eq_ci(field, b"has_item") {
        if !has_sub {
            Some(Vec::new())
        } else {
            Some(if char_has_item(g, subfield, c) { b"1" } else { b"0" }.to_vec())
        }
    } else if eq_ci(field, b"hasattached") {
        if !has_sub || !g.ch(c).is_npc() {
            Some(Vec::new())
        } else {
            let i = atoi32(subfield);
            Some(if trig_is_attached(g, GoId::Char(c), i) { b"1" } else { b"0" }.to_vec())
        }
    } else if eq_ci(field, b"heshe") {
        Some(hssh(g.ch(c).sex).to_vec())
    } else if eq_ci(field, b"himher") {
        Some(hmhr(g.ch(c).sex).to_vec())
    } else if eq_ci(field, b"hisher") {
        Some(hshr(g.ch(c).sex).to_vec())
    } else if eq_ci(field, b"hitp") {
        if has_sub {
            g.ch_mut(c).points.hit += atoi32(subfield);
            crate::fight::update_pos(g, c);
        }
        Some(g.ch(c).points.hit.to_string().into_bytes())
    } else if eq_ci(field, b"hitroll") {
        if has_sub {
            let ch = g.ch_mut(c);
            ch.points.hitroll = 1.max(ch.points.hitroll as i32 + atoi32(subfield)) as i8;
        }
        Some(g.ch(c).points.hitroll.to_string().into_bytes())
    } else if eq_ci(field, b"hunger") {
        if has_sub {
            let v = (-1).max(atoi32(subfield).min(24));
            if let Some(ps) = g.ch_mut(c).player_specials.as_mut() {
                ps.conditions[crate::ch::HUNGER] = v as i16;
            }
        }
        Some(get_cond(g, c, crate::ch::HUNGER).to_string().into_bytes())
    } else if eq_ci(field, b"id") {
        Some(char_script_id(g, c).to_string().into_bytes())
    } else if eq_ci(field, b"is_pc") {
        Some(if g.ch(c).is_npc() { b"0" } else { b"1" }.to_vec())
    } else if eq_ci(field, b"int") {
        if has_sub {
            adjust_ability(g, c, b'i', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.intel.to_string().into_bytes())
    } else if eq_ci(field, b"inventory") {
        if has_sub {
            let vnum = atoi32(subfield);
            let carrying = g.ch(c).carrying.clone();
            for oid in carrying {
                if super::obj_vnum(g, oid) == vnum {
                    return Some(uid_str(obj_script_id(g, oid)));
                }
            }
            Some(Vec::new())
        } else {
            Some(match g.ch(c).carrying.first().copied() {
                Some(o) => uid_str(obj_script_id(g, o)),
                None => Vec::new(),
            })
        }
    } else if eq_ci(field, b"is_killer") {
        if has_sub {
            if eq_ci(subfield, b"on") {
                g.ch_mut(c).act.set(flags::PLR_KILLER);
            } else if eq_ci(subfield, b"off") {
                g.ch_mut(c).act.remove(flags::PLR_KILLER);
            }
        }
        Some(if g.ch(c).plr(flags::PLR_KILLER) { b"1" } else { b"0" }.to_vec())
    } else if eq_ci(field, b"is_thief") {
        if has_sub {
            if eq_ci(subfield, b"on") {
                g.ch_mut(c).act.set(flags::PLR_THIEF);
            } else if eq_ci(subfield, b"off") {
                g.ch_mut(c).act.remove(flags::PLR_THIEF);
            }
        }
        Some(if g.ch(c).plr(flags::PLR_THIEF) { b"1" } else { b"0" }.to_vec())
    } else if eq_ci(field, b"level") {
        if has_sub {
            let lev = atoi32(subfield);
            g.ch_mut(c).level = lev.clamp(0, LVL_IMMORT as i32 - 1) as u8;
            Some(Vec::new())
        } else {
            Some(g.ch(c).level.to_string().into_bytes())
        }
    } else if eq_ci(field, b"mana") {
        if has_sub {
            g.ch_mut(c).points.mana += atoi32(subfield);
        }
        Some(g.ch(c).points.mana.to_string().into_bytes())
    } else if eq_ci(field, b"master") {
        Some(match g.ch(c).master {
            Some(m) => uid_str(char_script_id(g, m)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"maxhitp") {
        if has_sub {
            let ch = g.ch_mut(c);
            ch.points.max_hit = (ch.points.max_hit + atoi32(subfield)).max(1);
        }
        Some(g.ch(c).points.max_hit.to_string().into_bytes())
    } else if eq_ci(field, b"maxmana") {
        if has_sub {
            let ch = g.ch_mut(c);
            ch.points.max_mana = (ch.points.max_mana + atoi32(subfield)).max(1);
        }
        Some(g.ch(c).points.max_mana.to_string().into_bytes())
    } else if eq_ci(field, b"maxmove") {
        if has_sub {
            let ch = g.ch_mut(c);
            ch.points.max_move = (ch.points.max_move + atoi32(subfield)).max(1);
        }
        Some(g.ch(c).points.max_move.to_string().into_bytes())
    } else if eq_ci(field, b"move") {
        if has_sub {
            g.ch_mut(c).points.mov += atoi32(subfield);
        }
        Some(g.ch(c).points.mov.to_string().into_bytes())
    } else if eq_ci(field, b"name") {
        Some(g.ch(c).get_name().to_vec())
    } else if eq_ci(field, b"next_in_room") {
        // next_in_room: the person after c in their room's people list.
        let room = g.ch(c).in_room;
        if room == NOWHERE {
            return Some(Vec::new());
        }
        let people = &g.rooms[room as usize].people;
        let next = people.iter().position(|&p| p == c).and_then(|i| people.get(i + 1)).copied();
        Some(match next {
            Some(n) => uid_str(char_script_id(g, n)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"npcflag") {
        if has_sub {
            let buf = sprintbitarray(&g.ch(c).act, &tables::ACTION_BITS);
            Some(if str_str(&buf, subfield) { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"pos") {
        if has_sub {
            for i in POS_SLEEPING..=POS_STANDING {
                let pname = tables::POSITION_TYPES[i as usize].as_bytes();
                if subfield.len() <= pname.len()
                    && subfield.eq_ignore_ascii_case(&pname[..subfield.len()])
                {
                    g.ch_mut(c).position = i;
                    break;
                }
            }
        }
        Some(tables::POSITION_TYPES[g.ch(c).position as usize].as_bytes().to_vec())
    } else if eq_ci(field, b"prac") {
        if has_sub {
            if let Some(ps) = g.ch_mut(c).player_specials.as_mut() {
                ps.practices = 0.max(ps.practices + atoi32(subfield));
            }
        }
        let p = g.ch(c).player_specials.as_ref().map_or(0, |ps| ps.practices);
        Some(p.to_string().into_bytes())
    } else if eq_ci(field, b"pref") {
        if has_sub {
            let pref = get_flag_by_name(&tables::PREFERENCE_BITS, subfield);
            Some(if pref != -1 && g.ch(c).prf(pref as usize) { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if !g.ch(c).is_npc()
        && (eq_ci(field, b"questpoints") || eq_ci(field, b"qp") || eq_ci(field, b"qpnts"))
    {
        if has_sub {
            if let Some(ps) = g.ch_mut(c).player_specials.as_mut() {
                ps.questpoints += atoi32(subfield);
            }
        }
        Some(g.ch(c).ps().questpoints.to_string().into_bytes())
    } else if eq_ci(field, b"quest") {
        let ch = g.ch(c);
        if !ch.is_npc() {
            let q = ch.ps().current_quest;
            if q != NOTHING && g.world.quests.iter().any(|qu| qu.vnum == q) {
                return Some(q.to_string().into_bytes());
            }
        }
        Some(b"0".to_vec())
    } else if eq_ci(field, b"questdone") {
        let ch = g.ch(c);
        if !ch.is_npc() && !subfield.is_empty() {
            let qn = atoi32(subfield);
            let done = ch.ps().completed_quests.iter().any(|&v| v as i32 == qn);
            Some(if done { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"room") {
        let room = g.ch(c).in_room;
        Some(if room != NOWHERE {
            uid_str(room_script_id(g, room))
        } else {
            uid_str(ROOM_ID_BASE)
        })
    } else if eq_ci(field, b"saving_breath") {
        saving_field(g, c, SAVING_BREATH as usize, subfield)
    } else if eq_ci(field, b"saving_para") {
        saving_field(g, c, SAVING_PARA as usize, subfield)
    } else if eq_ci(field, b"saving_petri") {
        saving_field(g, c, SAVING_PETRI as usize, subfield)
    } else if eq_ci(field, b"saving_rod") {
        saving_field(g, c, SAVING_ROD as usize, subfield)
    } else if eq_ci(field, b"saving_spell") {
        saving_field(g, c, SAVING_SPELL as usize, subfield)
    } else if eq_ci(field, b"sex") {
        Some(tables::GENDERS[g.ch(c).sex as usize].as_bytes().to_vec())
    } else if eq_ci(field, b"skill") {
        Some(skill_percent(g, c, subfield))
    } else if eq_ci(field, b"skillset") {
        if !g.ch(c).is_npc() && has_sub {
            let (skillname, rest) = crate::interpreter::one_word(subfield);
            let amount = crate::interpreter::skip_spaces(rest);
            if !amount.is_empty() && c_is_number(amount) {
                if let Some(skillnum) = crate::spec::find_skill_num(&skillname) {
                    if skillnum > 0 {
                        let new_value = 0.max(100.min(atoi32(amount)));
                        g.ch_mut(c).set_skill(skillnum, new_value);
                    }
                }
            }
        }
        Some(Vec::new())
    } else if eq_ci(field, b"str") {
        if has_sub {
            adjust_ability(g, c, b's', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.str_.to_string().into_bytes())
    } else if eq_ci(field, b"stradd") {
        if g.ch(c).aff_abils.str_ >= 18 {
            if has_sub {
                let ch = g.ch_mut(c);
                let mut v = ch.real_abils.str_add as i32 + atoi32(subfield);
                v = v.clamp(0, 100);
                ch.real_abils.str_add = v as i8;
                crate::handler::affect_total(g, c);
            }
            Some(g.ch(c).aff_abils.str_add.to_string().into_bytes())
        } else {
            // No output is written and the '\x1' sentinel stays, so this
            // takes the unknown-field path.
            None
        }
    } else if eq_ci(field, b"thirst") {
        if has_sub {
            let v = (-1).max(atoi32(subfield).min(24));
            if let Some(ps) = g.ch_mut(c).player_specials.as_mut() {
                ps.conditions[crate::ch::THIRST] = v as i16;
            }
        }
        Some(get_cond(g, c, crate::ch::THIRST).to_string().into_bytes())
    } else if eq_ci(field, b"title") {
        if !g.ch(c).is_npc() && has_sub && super::misc::valid_dg_target(g, c, DG_ALLOW_GODS) {
            g.ch_mut(c).title = Some(subfield.to_vec());
        }
        let ch = g.ch(c);
        Some(if ch.is_npc() {
            Vec::new()
        } else {
            // glibc printf("%s", NULL) prints "(null)".
            ch.title.clone().unwrap_or_else(|| b"(null)".to_vec())
        })
    } else if eq_ci(field, b"varexists") {
        let mut found = false;
        if let Some(sc) = g.ch(c).script.as_deref() {
            found = sc.global_vars.iter().any(|v| eq_ci(&v.name, subfield));
        }
        Some(if found { b"1" } else { b"0" }.to_vec())
    } else if eq_ci(field, b"vnum") {
        if has_sub {
            let ch = g.ch(c);
            let res = if ch.is_npc() {
                (super::mob_vnum(g, c) == atoi32(subfield)) as i32
            } else {
                0
            };
            Some(res.to_string().into_bytes())
        } else if g.ch(c).is_npc() {
            Some(super::mob_vnum(g, c).to_string().into_bytes())
        } else {
            Some(b"-1".to_vec())
        }
    } else if eq_ci(field, b"weight") {
        Some(g.ch(c).weight.to_string().into_bytes())
    } else if eq_ci(field, b"wis") {
        if has_sub {
            adjust_ability(g, c, b'w', atoi32(subfield));
        }
        Some(g.ch(c).aff_abils.wis.to_string().into_bytes())
    } else if eq_ci(field, b"wait") {
        if has_sub {
            let addition = atoi32(subfield);
            g.ch_mut(c).wait = addition * (PULSE_VIOLENCE as i32 / 2);
        }
        Some(g.ch(c).wait.to_string().into_bytes())
    } else {
        None
    }
}

fn saving_field(g: &mut Game, c: CharId, which: usize, subfield: &[u8]) -> Option<BStr> {
    if !subfield.is_empty() {
        let ch = g.ch_mut(c);
        ch.apply_saving_throw[which] += atoi32(subfield) as i16;
    }
    Some(g.ch(c).apply_saving_throw[which].to_string().into_bytes())
}

fn get_cond(g: &Game, c: CharId, which: usize) -> i32 {
    g.ch(c).player_specials.as_ref().map_or(0, |ps| ps.conditions[which] as i32)
}

fn hssh(sex: u8) -> &'static [u8] {
    match sex {
        SEX_MALE => b"he",
        SEX_FEMALE => b"she",
        _ => b"it",
    }
}
fn hmhr(sex: u8) -> &'static [u8] {
    match sex {
        SEX_MALE => b"him",
        SEX_FEMALE => b"her",
        _ => b"it",
    }
}
fn hshr(sex: u8) -> &'static [u8] {
    match sex {
        SEX_MALE => b"his",
        SEX_FEMALE => b"her",
        _ => b"its",
    }
}

/// Object fields.
fn obj_field(g: &mut Game, _ctx: DgCtx, o: ObjId, field: &[u8], subfield: &[u8]) -> Option<BStr> {
    let has_sub = !subfield.is_empty();

    if eq_ci(field, b"affects") {
        if has_sub {
            let hit = check_flags_by_name_ar(
                &g.obj(o).perm_affects,
                mud_data::flags::NUM_AFF_FLAGS,
                subfield,
                &tables::AFFECTED_BITS,
            );
            Some(if hit { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"cost") {
        if has_sub {
            let ob = g.obj_mut(o);
            ob.cost = 1.max(atoi32(subfield) + ob.cost);
        }
        Some(g.obj(o).cost.to_string().into_bytes())
    } else if eq_ci(field, b"cost_per_day") {
        if has_sub {
            let ob = g.obj_mut(o);
            ob.cost_per_day = 1.max(atoi32(subfield) + ob.cost_per_day);
        }
        Some(g.obj(o).cost_per_day.to_string().into_bytes())
    } else if eq_ci(field, b"carried_by") {
        Some(match g.obj(o).carried_by {
            Some(c) => uid_str(char_script_id(g, c)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"contents") {
        Some(match g.obj(o).contains.first().copied() {
            Some(inner) => uid_str(obj_script_id(g, inner)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"count") {
        if g.obj(o).type_flag == mud_data::flags::ITEM_CONTAINER as i32 {
            let contains = g.obj(o).contains.clone();
            Some(item_in_list(g, subfield, &contains).to_string().into_bytes())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"extra") {
        if has_sub {
            let hit = check_flags_by_name_ar(
                &g.obj(o).extra_flags,
                mud_data::flags::NUM_ITEM_FLAGS,
                subfield,
                &tables::EXTRA_BITS,
            );
            Some(if hit { b"1" } else { b"0" }.to_vec())
        } else {
            Some(sprintbitarray(&g.obj(o).extra_flags, &tables::EXTRA_BITS))
        }
    } else if eq_ci(field, b"has_in") {
        if g.obj(o).type_flag == mud_data::flags::ITEM_CONTAINER as i32 {
            let contains = g.obj(o).contains.clone();
            Some(if item_in_list(g, subfield, &contains) != 0 { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"hasattached") {
        if !has_sub {
            Some(Vec::new())
        } else {
            let i = atoi32(subfield);
            Some(if trig_is_attached(g, GoId::Obj(o), i) { b"1" } else { b"0" }.to_vec())
        }
    } else if eq_ci(field, b"id") {
        Some(obj_script_id(g, o).to_string().into_bytes())
    } else if eq_ci(field, b"is_inroom") {
        let room = g.obj(o).in_room;
        Some(if room != NOWHERE { uid_str(room_script_id(g, room)) } else { Vec::new() })
    } else if eq_ci(field, b"is_pc") {
        Some(b"-1".to_vec())
    } else if eq_ci(field, b"name") {
        Some(obj_name(g, o).to_vec())
    } else if eq_ci(field, b"next_in_list") {
        // next_content within whatever list the object is in.
        let ob = g.obj(o);
        let list: Vec<ObjId> = if let Some(container) = ob.in_obj {
            g.obj(container).contains.clone()
        } else if let Some(c) = ob.carried_by {
            g.ch(c).carrying.clone()
        } else if ob.in_room != NOWHERE {
            g.rooms[ob.in_room as usize].contents.clone()
        } else {
            Vec::new()
        };
        let next = list.iter().position(|&x| x == o).and_then(|i| list.get(i + 1)).copied();
        Some(match next {
            Some(n) => uid_str(obj_script_id(g, n)),
            None => Vec::new(),
        })
    } else if eq_ci(field, b"oset") {
        if has_sub {
            Some(if handle_oset(g, o, subfield) { b"1" } else { b"0" }.to_vec())
        } else {
            // The sentinel stays -> unknown-field path.
            None
        }
    } else if eq_ci(field, b"room") {
        let room = obj_room(g, o);
        Some(if room != NOWHERE { uid_str(room_script_id(g, room)) } else { Vec::new() })
    } else if eq_ci(field, b"shortdesc") {
        Some(obj_short(g, o).to_vec())
    } else if eq_ci(field, b"type") {
        Some(sprinttype(g.obj(o).type_flag, &tables::ITEM_TYPES))
    } else if eq_ci(field, b"timer") {
        Some(g.obj(o).timer.to_string().into_bytes())
    } else if eq_ci(field, b"vnum") {
        if has_sub {
            Some(((super::obj_vnum(g, o) == atoi32(subfield)) as i32).to_string().into_bytes())
        } else {
            Some(super::obj_vnum(g, o).to_string().into_bytes())
        }
    } else if eq_ci(field, b"val0") {
        Some(g.obj(o).values[0].to_string().into_bytes())
    } else if eq_ci(field, b"val1") {
        Some(g.obj(o).values[1].to_string().into_bytes())
    } else if eq_ci(field, b"val2") {
        Some(g.obj(o).values[2].to_string().into_bytes())
    } else if eq_ci(field, b"val3") {
        Some(g.obj(o).values[3].to_string().into_bytes())
    } else if eq_ci(field, b"wearflag") {
        if has_sub {
            let pos = find_eq_pos_script(subfield);
            Some(if can_wear_on_pos(g, o, pos) { b"1" } else { b"0" }.to_vec())
        } else {
            Some(b"0".to_vec())
        }
    } else if eq_ci(field, b"weight") {
        if has_sub {
            let ob = g.obj_mut(o);
            ob.weight = 1.max(atoi32(subfield) + ob.weight);
        }
        Some(g.obj(o).weight.to_string().into_bytes())
    } else if eq_ci(field, b"worn_by") {
        Some(match g.obj(o).worn_by {
            Some(w) => uid_str(char_script_id(g, w)),
            None => Vec::new(),
        })
    } else {
        None
    }
}

/// handle_oset: alias/apply/longdesc/shortdesc, backed by the oset_*
/// helpers (length caps 64/64/128, apply accumulates modifiers).
fn handle_oset(g: &mut Game, o: ObjId, argument: &[u8]) -> bool {
    let (value, rest) = crate::interpreter::one_argument(argument);
    if value.is_empty() {
        return false;
    }
    let which = [&b"alias"[..], b"apply", b"longdesc", b"shortdesc"]
        .iter()
        .position(|t| crate::handler::is_abbrev(&value, t));
    match which {
        Some(0) => {
            let arg = crate::interpreter::skip_spaces(rest);
            if arg.len() > 64 {
                return false;
            }
            g.obj_mut(o).name = Some(arg.to_vec());
            true
        }
        Some(1) => oset_apply(g, o, rest),
        Some(2) => {
            let arg = crate::interpreter::skip_spaces(rest);
            if arg.len() > 128 {
                return false;
            }
            g.obj_mut(o).description = Some(arg.to_vec());
            true
        }
        Some(3) => {
            let arg = crate::interpreter::skip_spaces(rest);
            if arg.len() > 64 {
                return false;
            }
            g.obj_mut(o).short_description = Some(arg.to_vec());
            true
        }
        _ => false,
    }
}

/// Full apply-type name (as prefix of the given arg),
/// nonzero value; adds into an existing same-type slot or claims an empty
/// one; a resulting modifier of 0 frees the slot.
fn oset_apply(g: &mut Game, o: ObjId, argument: &[u8]) -> bool {
    let (arg, rest) = crate::interpreter::one_argument(argument);
    let rest = crate::interpreter::skip_spaces(rest);
    let value = atoi32(rest);
    if value == 0 {
        return false;
    }
    let mut apply = -1;
    for (i, name) in tables::APPLY_TYPES.iter().enumerate() {
        // is_abbrev(apply_types[i], arg): the TABLE NAME must prefix arg.
        if crate::handler::is_abbrev(name.as_bytes(), &arg) {
            apply = i as i32;
            break;
        }
    }
    if apply == -1 {
        return false;
    }
    let ob = g.obj_mut(o);
    let mut location = -1;
    let mut empty = -1;
    let mut prev_mod = 0;
    for i in 0..mud_data::types::MAX_OBJ_AFFECT {
        if ob.affected[i].location == apply {
            location = i as i32;
            prev_mod = ob.affected[i].modifier;
            break;
        } else if ob.affected[i].location == mud_data::flags::APPLY_NONE && empty == -1 {
            empty = i as i32;
        }
    }
    if location == -1 {
        location = empty;
    }
    if location == -1 {
        return false;
    }
    let slot = &mut ob.affected[location as usize];
    slot.modifier = prev_mod + value;
    if slot.modifier != 0 {
        slot.location = apply;
    } else {
        slot.location = mud_data::flags::APPLY_NONE;
    }
    true
}

/// Room fields.
fn room_field(g: &mut Game, ctx: DgCtx, r: RoomRnum, field: &[u8], subfield: &[u8]) -> BStr {
    let has_sub = !subfield.is_empty();
    let vnum = g.world.rooms[r as usize].vnum;

    // Room 0 (the Void): raw global-var store only.
    if vnum == 0 {
        let Some(sc) = g.rooms[0].script.as_deref() else {
            let (name, tvnum) = trig_ident(g, ctx);
            script_log(
                g,
                &format!(
                    "Trigger: {}, Vnum {}, type {}. Trying to access Global var list of void. Apparently this has not been set up!",
                    name, tvnum, ctx.go.kind()
                ),
            );
            return Vec::new();
        };
        return sc
            .global_vars
            .iter()
            .find(|v| eq_ci(&v.name, field))
            .map(|v| v.value.clone())
            .unwrap_or_default();
    }

    if eq_ci(field, b"name") {
        g.world.rooms[r as usize].name.clone().unwrap_or_default()
    } else if eq_ci(field, b"sector") {
        sprinttype(g.world.rooms[r as usize].sector_type, &tables::SECTOR_TYPES)
    } else if eq_ci(field, b"vnum") {
        if has_sub {
            ((vnum as i32 == atoi32(subfield)) as i32).to_string().into_bytes()
        } else {
            vnum.to_string().into_bytes()
        }
    } else if eq_ci(field, b"contents") {
        if has_sub {
            let contents = g.rooms[r as usize].contents.clone();
            for oid in contents {
                if super::obj_vnum(g, oid) == atoi32(subfield) {
                    return uid_str(obj_script_id(g, oid));
                }
            }
            Vec::new()
        } else {
            match g.rooms[r as usize].contents.first().copied() {
                Some(o) => uid_str(obj_script_id(g, o)),
                None => Vec::new(),
            }
        }
    } else if eq_ci(field, b"people") {
        match g.rooms[r as usize].people.first().copied() {
            Some(c) => uid_str(char_script_id(g, c)),
            None => Vec::new(),
        }
    } else if eq_ci(field, b"id") {
        room_script_id(g, r).to_string().into_bytes()
    } else if eq_ci(field, b"weather") {
        const SKY_LOOK: [&[u8]; 4] = [b"sunny", b"cloudy", b"rainy", b"lightning"];
        let indoors = g.world.rooms[r as usize].room_flags[0] & (1 << mud_data::flags::ROOM_INDOORS) != 0;
        if !indoors {
            SKY_LOOK[g.weather.sky as usize].to_vec()
        } else {
            Vec::new()
        }
    } else if eq_ci(field, b"hasattached") {
        if !has_sub {
            Vec::new()
        } else {
            let i = atoi32(subfield);
            if trig_is_attached(g, GoId::Room(r), i) { b"1" } else { b"0" }.to_vec()
        }
    } else if eq_ci(field, b"zonenumber") {
        let z = g.world.rooms[r as usize].zone;
        g.world.zones[z as usize].number.to_string().into_bytes()
    } else if eq_ci(field, b"zonename") {
        let z = g.world.rooms[r as usize].zone;
        g.world.zones[z as usize].name.clone().unwrap_or_default()
    } else if eq_ci(field, b"roomflag") {
        if has_sub {
            let flags = mud_data::flags::FlagSet(g.world.rooms[r as usize].room_flags);
            let hit = check_flags_by_name_ar(
                &flags,
                mud_data::flags::NUM_ROOM_FLAGS,
                subfield,
                &tables::ROOM_BITS,
            );
            if hit { b"1" } else { b"0" }.to_vec()
        } else {
            b"0".to_vec()
        }
    } else if let Some(dir) = dir_by_name(field) {
        match g.world.rooms[r as usize].dir_option[dir].clone() {
            None => Vec::new(),
            Some(ex) => {
                if has_sub {
                    if eq_ci(subfield, b"vnum") {
                        // An exit that leads nowhere reports -1.
                        let v = if ex.to_room != NOWHERE && (ex.to_room as usize) < g.world.rooms.len()
                        {
                            g.world.rooms[ex.to_room as usize].vnum as i32
                        } else {
                            -1
                        };
                        v.to_string().into_bytes()
                    } else if eq_ci(subfield, b"key") {
                        ex.key.to_string().into_bytes()
                    } else if eq_ci(subfield, b"bits") {
                        sprintbit(ex.exit_info as u32, &tables::EXIT_BITS)
                    } else if eq_ci(subfield, b"room") {
                        if ex.to_room != NOWHERE {
                            uid_str(room_script_id(g, ex.to_room))
                        } else {
                            Vec::new()
                        }
                    } else {
                        // Unknown exit subfield: writes nothing.
                        Vec::new()
                    }
                } else {
                    sprintbit(ex.exit_info as u32, &tables::EXIT_BITS)
                }
            }
        }
    } else {
        // Unknown room field -> room's own globals, else log + empty.
        if let Some(sc) = g.rooms[r as usize].script.as_deref() {
            if let Some(v) = sc.global_vars.iter().find(|v| eq_ci(&v.name, field)) {
                return v.value.clone();
            }
        }
        let (name, tvnum) = trig_ident(g, ctx);
        script_log(
            g,
            &format!(
                "Trigger: {}, VNum {}, type: {}. unknown room field: '{}'",
                name,
                tvnum,
                ctx.go.kind(),
                String::from_utf8_lossy(field)
            ),
        );
        Vec::new()
    }
}

fn dir_by_name(field: &[u8]) -> Option<usize> {
    for (i, d) in [&b"north"[..], b"east", b"south", b"west", b"up", b"down"].iter().enumerate() {
        if eq_ci(field, d) {
            return Some(i);
        }
    }
    None
}

/// The substitution walk over one line, including the in-place `tmpvr`
/// overwrite and the subfield accumulator that persists across variables.
pub fn var_subst(g: &mut Game, ctx: DgCtx, line: &[u8]) -> BStr {
    // The buffers here grow, so nothing needs the bound -- but a
    // line this long cannot be processed at all downstream, and a builder
    // can produce one with the string editor's /ra. Accepting it would
    // make a zone that runs here and nowhere else.
    // The same 511 already bounds a trigger line by both other routes:
    // process_input truncates typed input, and fread_string chunks a.trg
    // line before parse_trigger sees it.
    if line.len() >= MAX_INPUT_LENGTH {
        let (name, tvnum) = trig_ident(g, ctx);
        script_log(
            g,
            &format!(
                "Trigger: {}, VNum {}, type: {}. Line is {} characters, over the {} limit: '{}...'",
                name,
                tvnum,
                ctx.go.kind(),
                line.len(),
                MAX_INPUT_LENGTH - 1,
                String::from_utf8_lossy(&line[..60.min(line.len())])
            ),
        );
        return Vec::new();
    }

    if !line.contains(&b'%') {
        return line.to_vec();
    }

    // tmp: the mutable working copy with C-string semantics.
    let mut tmp = line.to_vec();
    tmp.push(0);
    let at = |t: &Vec<u8>, i: usize| -> u8 { t.get(i).copied().unwrap_or(0) };
    let read_cstr = |t: &Vec<u8>, start: usize| -> BStr {
        let mut v = Vec::new();
        let mut i = start;
        while at(t, i) != 0 {
            v.push(t[i]);
            i += 1;
        }
        v
    };

    // subfield: one buffer and a running pointer, NEVER reset between
    // %groups%.
    let mut subfield = vec![0u8; 4096];
    let mut subfield_p = 0usize;
    let sub_cstr = |s: &Vec<u8>| -> BStr {
        s.iter().take_while(|&&b| b != 0).copied().collect()
    };

    let mut buf: BStr = Vec::new();
    let mut left: i32 = 255;
    let mut p = 0usize;

    while at(&tmp, p) != 0 && left > 0 {
        while at(&tmp, p) != 0 && at(&tmp, p) != b'%' && left > 0 {
            buf.push(tmp[p]);
            p += 1;
            left -= 1;
        }

        if at(&tmp, p) == 0 {
            break;
        }
        // p at '%'
        p += 1;
        if at(&tmp, p) == b'%' && left > 0 {
            buf.push(b'%');
            p += 1;
            left -= 1;
            continue;
        }
        if at(&tmp, p) == 0 || left <= 0 {
            continue; // let the loop condition re-check and end the scan
        }

        let var_start = p;
        while at(&tmp, p) != 0 && at(&tmp, p) != b'%' && at(&tmp, p) != b'.' {
            p += 1;
        }
        let mut field_start = p;
        if at(&tmp, p) == b'.' {
            tmp[p] = 0;
            p += 1;
            let mut dots = 0;
            let mut paren_count = 0;
            field_start = p;
            loop {
                let ch = at(&tmp, p);
                if !(ch != 0 && (ch != b'%' || paren_count > 0 || dots > 0)) {
                    break;
                }
                if dots > 0 {
                    subfield[subfield_p] = 0;
                    let var_s = read_cstr(&tmp, var_start);
                    let field_s = read_cstr(&tmp, field_start);
                    let sub_s = sub_cstr(&subfield);
                    let repl = find_replacement(g, ctx, &var_s, &field_s, &sub_s);
                    if !repl.is_empty() {
                        let mut eval_line = b"eval tmpvr ".to_vec();
                        eval_line.extend_from_slice(&repl);
                        super::driver::process_eval_line(g, ctx, &eval_line);
                        // Overwrite the variable in place with "tmpvr" --
                        // six bytes including the terminator, which can
                        // run into whatever follows it.
                        for (i, &b) in b"tmpvr\0".iter().enumerate() {
                            let idx = var_start + i;
                            if idx < tmp.len() {
                                tmp[idx] = b;
                            } else {
                                tmp.push(b);
                            }
                        }
                        field_start = p;
                        dots = 0;
                        p += 1;
                        continue;
                    }
                    dots = 0;
                } else if ch == b'(' {
                    tmp[p] = 0;
                    paren_count += 1;
                } else if ch == b')' {
                    tmp[p] = 0;
                    paren_count -= 1;
                } else if paren_count > 0 {
                    if subfield_p < subfield.len() - 1 {
                        subfield[subfield_p] = ch;
                        subfield_p += 1;
                    }
                } else if ch == b'.' {
                    tmp[p] = 0;
                    dots += 1;
                }
                p += 1;
            }
        }

        // *(p++) = '\0'
        if p < tmp.len() {
            tmp[p] = 0;
        }
        p += 1;
        subfield[subfield_p] = 0;

        let sub_now = sub_cstr(&subfield);
        if !sub_now.is_empty() {
            let expanded = var_subst(g, ctx, &sub_now);
            for (i, &b) in expanded.iter().enumerate() {
                if i < subfield.len() - 1 {
                    subfield[i] = b;
                }
            }
            let end = expanded.len().min(subfield.len() - 1);
            subfield[end] = 0;
        }

        let var_s = read_cstr(&tmp, var_start);
        let field_s = read_cstr(&tmp, field_start.min(tmp.len()));
        let sub_s = sub_cstr(&subfield);
        let repl = find_replacement(g, ctx, &var_s, &field_s, &sub_s);

        let take = (left.max(0) as usize).min(repl.len());
        buf.extend_from_slice(&repl[..take]);
        left -= repl.len() as i32;
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_str_is_naive() {
        assert!(str_str(b"hello world", b"WORLD"));
        assert!(!str_str(b"hello", b""));
        // The naive scan misses overlapping matches: "aab" IS in "aaab",
        // but str_str does not find it.
        assert!(!str_str(b"aaab", b"aab"));
        assert!(str_str(b"aab", b"ab"));
    }

    #[test]
    fn text_fields() {
        let g = ();
        let _ = g;
        assert_eq!(text_processed_pure(b"strlen", b"", b"hello"), Some(b"5".to_vec()));
        assert_eq!(text_processed_pure(b"toupper", b"", b"abc"), Some(b"Abc".to_vec()));
        assert_eq!(text_processed_pure(b"trim", b"", b"  hi  "), Some(b"hi".to_vec()));
        assert_eq!(text_processed_pure(b"car", b"", b"one two"), Some(b"one".to_vec()));
        assert_eq!(text_processed_pure(b"cdr", b"", b"one  two three"), Some(b"two three".to_vec()));
        assert_eq!(text_processed_pure(b"charat", b"4", b"L337-String"), Some(b"7".to_vec()));
        assert_eq!(text_processed_pure(b"charat", b"0", b"x"), Some(b"".to_vec()));
        assert_eq!(text_processed_pure(b"charat", b"-2", b"x"), Some(b"".to_vec()));
    }

    // text_processed minus the &Game-needing mudcommand branch.
    fn text_processed_pure(field: &[u8], subfield: &[u8], value: &[u8]) -> Option<BStr> {
        assert!(!eq_ci(field, b"mudcommand"));
        let is_sp = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
        if eq_ci(field, b"strlen") {
            Some(value.len().to_string().into_bytes())
        } else if eq_ci(field, b"toupper") {
            let mut v = value.to_vec();
            if !v.is_empty() {
                v[0] = v[0].to_ascii_uppercase();
            }
            Some(v)
        } else if eq_ci(field, b"trim") {
            let start = value.iter().position(|b| !is_sp(b));
            match start {
                None => Some(Vec::new()),
                Some(s) => {
                    let e = value.iter().rposition(|b| !is_sp(b)).unwrap();
                    Some(value[s..=e].to_vec())
                }
            }
        } else if eq_ci(field, b"car") {
            Some(value.iter().take_while(|b| !is_sp(b)).copied().collect())
        } else if eq_ci(field, b"cdr") {
            let mut i = 0;
            while i < value.len() && !is_sp(&value[i]) {
                i += 1;
            }
            while i < value.len() && is_sp(&value[i]) {
                i += 1;
            }
            Some(value[i..].to_vec())
        } else if eq_ci(field, b"charat") {
            let idx = atoi32(subfield);
            if idx < 1 || idx as usize > value.len() {
                Some(Vec::new())
            } else {
                Some(vec![value[idx as usize - 1]])
            }
        } else {
            None
        }
    }
}
