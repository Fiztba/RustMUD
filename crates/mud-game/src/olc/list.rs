//! `rlist`/`mlist`/`olist`/`slist`/`zlist`/`tlist`/`qlist`,
//! the builder's index of everything in a zone.
//!
//! Two shapes of this file are worth naming up front:
//!
//! * Every listing but `slist`/`tlist` builds one `char buf[49152]` and pages
//! it. `len` accumulates what *would* have been written, so a listing that
//! outgrows the buffer keeps counting past the end and the overflow check
//! stops it one entry late: the last entry is truncated mid-line rather
//! than dropped. `CBuf` below implements that.
//! * `list_rooms`, `list_mobiles`, `list_objects` and `list_zones` all open
//! with an early return on an empty table -- with exactly one
//! room/mob/object/zone in
//! the table the header is composed and then thrown away, so the command
//! prints nothing at all.
//!
//! The `Q*` colour macros here are `CC*(ch, C_SPR)`, a
//! *lower* gate than `C_NRM`; the flag/type/affect tables mixed in with them
//! use `CC*(ch, C_NRM)`. The two are kept apart deliberately below.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{
    ACTION_BITS, APPLY_TYPES, ITEM_TYPES, OTRIG_TYPES, TRIG_TYPES, WTRIG_TYPES,
};
use mud_data::types::*;

use crate::act::informative::page_string;
use crate::act::other::count_color_chars;
use crate::act::BStr;
use crate::comm::{cc, send_to_char, C_NRM, C_SPR, KBGRN, KBRED, KCYN, KGRN, KNRM, KYEL};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, is_abbrev, is_name};
use crate::interpreter::{
    two_arguments, SCMD_OASIS_MLIST, SCMD_OASIS_OLIST, SCMD_OASIS_QLIST,
    SCMD_OASIS_RLIST, SCMD_OASIS_SLIST, SCMD_OASIS_TLIST, SCMD_OASIS_ZLIST,
};

const MAX_OBJ_LIST: usize = 100;

// ---------------------------------------------------------------------------
// The listing buffer
// ---------------------------------------------------------------------------

/// A fixed 48K buffer.
/// `len` counts the *untruncated* length, which is what the loops test, so it
/// can run past the buffer — one entry lands half-written before the break.
struct CBuf {
    buf: BStr,
    len: usize,
}

impl CBuf {
    fn new() -> Self {
        CBuf { buf: Vec::new(), len: 0 }
    }

    fn push(&mut self, s: &[u8]) {
        let space = MAX_STRING_LENGTH.saturating_sub(self.len);
        let take = s.len().min(space.saturating_sub(1));
        self.buf.extend_from_slice(&s[..take]);
        self.len += s.len();
    }

    /// Whether the listing has outgrown the buffer. Tested after each entry.
    fn overflowed(&self) -> bool {
        self.len > MAX_STRING_LENGTH
    }
}

/// `%-*s`: pad on the right to `width`, never truncating.
fn pad(s: &[u8], width: usize) -> BStr {
    let mut out = s.to_vec();
    while out.len() < width {
        out.push(b' ');
    }
    out
}

/// `%-45.45s`: pad *and* truncate to exactly 45.
fn pad_trunc(s: &[u8], width: usize) -> BStr {
    let mut out = s.to_vec();
    out.truncate(width);
    while out.len() < width {
        out.push(b' ');
    }
    out
}

/// The colour width fudge every listing applies: `%-*s` with
/// `count_color_chars(name) + base`, so embedded `\t` codes don't eat the
/// column.
fn colw(s: &[u8], base: usize) -> usize {
    base + count_color_chars(s)
}

/// The six `Q*` colours, resolved once for this character.
struct Q {
    nrm: BStr,
    grn: BStr,
    cyn: BStr,
    yel: BStr,
    bred: BStr,
    bgrn: BStr,
}

fn qcolors(g: &Game, chid: CharId) -> Q {
    Q {
        nrm: cc(g, chid, C_SPR, KNRM).to_vec(),
        grn: cc(g, chid, C_SPR, KGRN).to_vec(),
        cyn: cc(g, chid, C_SPR, KCYN).to_vec(),
        yel: cc(g, chid, C_SPR, KYEL).to_vec(),
        bred: cc(g, chid, C_SPR, KBRED).to_vec(),
        bgrn: cc(g, chid, C_SPR, KBGRN).to_vec(),
    }
}

fn obj_short(g: &Game, rnum: usize) -> BStr {
    g.world
        .obj_protos
        .get(rnum)
        .and_then(|o| o.short_description.clone())
        .unwrap_or_default()
}

fn obj_trig(g: &Game, rnum: usize, text: &'static [u8]) -> &'static [u8] {
    if g.world.obj_protos.get(rnum).is_some_and(|o| !o.proto_script.is_empty()) {
        text
    } else {
        b""
    }
}

fn item_type_name(t: i32) -> &'static str {
    ITEM_TYPES.get(t as usize).copied().unwrap_or("UNDEFINED")
}

// ---------------------------------------------------------------------------
// mlist level / flags
// ---------------------------------------------------------------------------

/// perform_mob_flag_list. Every match is really loaded
/// into room 0 and extracted again, so the listing draws from the RNG exactly
/// as `load` would.
fn perform_mob_flag_list(g: &mut Game, chid: CharId, arg: &[u8]) {
    let mob_flag = atoi(arg);
    if mob_flag < 0 || mob_flag > flags::NUM_MOB_FLAGS as i32 {
        send_to_char(g, chid, b"Invalid flag number!\r\n");
        return;
    }
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    let mut head: BStr = b"Listing mobiles with ".to_vec();
    head.extend_from_slice(&q.yel);
    head.extend_from_slice(
        ACTION_BITS.get(mob_flag as usize).copied().unwrap_or("UNDEFINED").as_bytes(),
    );
    head.extend_from_slice(&q.nrm);
    head.extend_from_slice(b" flag set.\r\n");
    buf.push(&head);

    let ccnrm = cc(g, chid, C_NRM, KNRM).to_vec();
    let cccyn = cc(g, chid, C_NRM, KCYN).to_vec();
    let ccyel = cc(g, chid, C_NRM, KYEL).to_vec();

    let mut found = 0;
    for num in 0..g.world.mob_protos.len() {
        let bit = mob_flag as usize;
        let set = g.world.mob_protos[num].act[bit / 32] & (1 << (bit % 32)) != 0;
        if !set {
            continue;
        }
        let Some(mob) = crate::db::read_mobile(g, num as Idx) else { continue };
        crate::handler::char_to_room(g, mob, 0);
        found += 1;
        let vnum = crate::dg::mob_vnum(g, mob);
        let level = g.ch(mob).level;
        let name = g.ch(mob).get_name().to_vec();

        let mut line: BStr = ccnrm.clone();
        line.extend_from_slice(format!("{:3}. ", found).as_bytes());
        line.extend_from_slice(&cccyn);
        line.push(b'[');
        line.extend_from_slice(&ccyel);
        line.extend_from_slice(format!("{:5}", vnum).as_bytes());
        line.extend_from_slice(&cccyn);
        line.push(b']');
        line.extend_from_slice(&ccnrm);
        line.extend_from_slice(b" Level ");
        line.extend_from_slice(&ccyel);
        line.extend_from_slice(format!("{:<3}", level).as_bytes());
        line.extend_from_slice(&ccnrm);
        line.push(b' ');
        line.extend_from_slice(&name);
        line.extend_from_slice(&ccnrm);
        line.extend_from_slice(b"\r\n");
        buf.push(&line);

        crate::handler::extract_char(g, mob);
        if buf.overflowed() {
            break;
        }
    }
    if found == 0 {
        send_to_char(g, chid, b"None Found!\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

fn perform_mob_level_list(g: &mut Game, chid: CharId, arg: &[u8]) {
    let mob_level = atoi(arg);
    if !(0..=99).contains(&mob_level) {
        send_to_char(g, chid, b"Invalid mob level!\r\n");
        return;
    }
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    let mut head: BStr = b"Listing mobiles of level ".to_vec();
    head.extend_from_slice(&q.yel);
    head.extend_from_slice(format!("{}", mob_level).as_bytes());
    head.extend_from_slice(&q.nrm);
    head.extend_from_slice(b"\r\n");
    buf.push(&head);

    let ccnrm = cc(g, chid, C_NRM, KNRM).to_vec();
    let cccyn = cc(g, chid, C_NRM, KCYN).to_vec();
    let ccyel = cc(g, chid, C_NRM, KYEL).to_vec();

    let mut found = 0;
    for num in 0..g.world.mob_protos.len() {
        if g.world.mob_protos[num].level != mob_level {
            continue;
        }
        let Some(mob) = crate::db::read_mobile(g, num as Idx) else { continue };
        crate::handler::char_to_room(g, mob, 0);
        found += 1;
        let vnum = crate::dg::mob_vnum(g, mob);
        let name = g.ch(mob).get_name().to_vec();

        let mut line: BStr = ccnrm.clone();
        line.extend_from_slice(format!("{:3}. ", found).as_bytes());
        line.extend_from_slice(&cccyn);
        line.push(b'[');
        line.extend_from_slice(&ccyel);
        line.extend_from_slice(format!("{:5}", vnum).as_bytes());
        line.extend_from_slice(&cccyn);
        line.push(b']');
        line.extend_from_slice(&ccnrm);
        line.push(b' ');
        line.extend_from_slice(&name);
        line.extend_from_slice(&ccnrm);
        line.extend_from_slice(b"\r\n");
        buf.push(&line);

        crate::handler::extract_char(g, mob);
        if buf.overflowed() {
            break;
        }
    }
    if found == 0 {
        send_to_char(g, chid, b"None Found!\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

// ---------------------------------------------------------------------------
// olist type / affect / name
// ---------------------------------------------------------------------------

/// add_to_obj_list: an insertion sort that keeps the
/// hundred highest values, pushing the displaced entry down the list.
fn add_to_obj_list(lst: &mut [(i32, i32); MAX_OBJ_LIST], mut nvo: i32, mut nval: i32) {
    for slot in lst.iter_mut() {
        if nval > slot.1 {
            let tmp = *slot;
            *slot = (nvo, nval);
            nvo = tmp.0;
            nval = tmp.1;
        }
    }
}

/// perform_obj_type_list.
///
/// Reading `item_types[itemtype]` into the header before any
/// validation lets `olist type 99` index past the table. The switch already
/// has the message for this, but only reaches it once an object of that
/// type is found — which for an invalid type never happens. The guard is
/// hoisted to where its sibling `perform_obj_aff_list` puts it.
fn perform_obj_type_list(g: &mut Game, chid: CharId, arg: &[u8]) {
    let itemtype = atoi(arg);
    if itemtype < 0 || itemtype >= flags::NUM_ITEM_TYPES as i32 {
        send_to_char(g, chid, b"Not a valid item type");
        return;
    }
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();

    let mut head: BStr = b"Listing all objects of type ".to_vec();
    head.extend_from_slice(&q.yel);
    head.push(b'[');
    head.extend_from_slice(item_type_name(itemtype).as_bytes());
    head.push(b']');
    head.extend_from_slice(&q.nrm);
    head.extend_from_slice(b"\r\n");
    buf.push(&head);

    let mut found = 0;
    for num in 0..g.world.obj_protos.len() {
        if g.world.obj_protos[num].type_flag != itemtype {
            continue;
        }
        // The vnum round-trip lands back on `num`; it is kept for the
        // missing-prototype guard it carries.
        let vnum = g.world.obj_protos[num].vnum;
        let Some(r_num) = g.world.real_object(vnum) else { continue };
        let r_num = r_num as usize;
        let ov = vnum as i32;
        let values = g.world.obj_protos[num].values;
        let short = obj_short(g, r_num);
        found += 1;

        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:3}", found).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") ");
        line.extend_from_slice(&q.cyn);
        line.push(b'[');
        line.extend_from_slice(&q.yel);

        match itemtype {
            t if t == flags::ITEM_LIGHT => {
                line.extend_from_slice(format!("{:5}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                if values[2] == -1 {
                    line.extend_from_slice(&q.bred);
                    line.extend_from_slice(b" INFINITE");
                    line.extend_from_slice(&q.cyn);
                    line.push(b' ');
                    line.extend_from_slice(&short);
                    line.extend_from_slice(&q.nrm);
                } else {
                    line.extend_from_slice(&q.nrm);
                    line.extend_from_slice(format!(" ({:<3}hrs) ", values[2]).as_bytes());
                    line.extend_from_slice(&q.cyn);
                    line.extend_from_slice(&short);
                    line.extend_from_slice(&q.nrm);
                }
            }
            t if t == flags::ITEM_SCROLL || t == flags::ITEM_POTION => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            t if t == flags::ITEM_WAND || t == flags::ITEM_STAFF => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(format!(" ({}x", values[1]).as_bytes());
                line.extend_from_slice(crate::dg::misc::skill_name_b(values[3]));
                line.extend_from_slice(b") ");
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            t if t == flags::ITEM_WEAPON => {
                let avg = ((values[2] + 1) * values[1]) / 2;
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(format!(" ({} Avg Dam) ", avg).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            t if t == flags::ITEM_ARMOR => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(format!(" ({}AC) ", values[0]).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            t if t == flags::ITEM_CONTAINER => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(format!(" (Max: {}) ", values[0]).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            t if t == flags::ITEM_DRINKCON || t == flags::ITEM_FOUNTAIN => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                if values[0] != -1 {
                    line.push(b']');
                    line.extend_from_slice(&q.nrm);
                    line.extend_from_slice(format!(" (Max: {}) ", values[0]).as_bytes());
                    line.extend_from_slice(&q.cyn);
                    line.extend_from_slice(&short);
                    line.extend_from_slice(&q.nrm);
                } else {
                    line.extend_from_slice(b"] ");
                    line.extend_from_slice(&q.bred);
                    line.extend_from_slice(b"INFINITE");
                    line.extend_from_slice(&q.cyn);
                    line.push(b' ');
                    line.extend_from_slice(&short);
                    line.extend_from_slice(&q.nrm);
                }
            }
            t if t == flags::ITEM_FOOD => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.push(b']');
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(format!(" ({}hrs) ", values[0]).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(&short);
                if values[3] != 0 {
                    line.push(b' ');
                    line.extend_from_slice(&q.bgrn);
                    line.extend_from_slice(b"Poisoned!");
                    line.extend_from_slice(&q.nrm);
                } else {
                    line.extend_from_slice(&q.nrm);
                }
            }
            t if t == flags::ITEM_MONEY => {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
                line.extend_from_slice(b" (");
                line.extend_from_slice(&q.yel);
                line.extend_from_slice(format!("{} coins", values[0]).as_bytes());
                line.extend_from_slice(&q.nrm);
                line.push(b')');
            }
            t if t == flags::ITEM_TREASURE
                || t == flags::ITEM_TRASH
                || t == flags::ITEM_OTHER
                || t == flags::ITEM_WORN
                || t == flags::ITEM_NOTE
                || t == flags::ITEM_PEN
                || t == flags::ITEM_BOAT
                || t == flags::ITEM_KEY =>
            {
                line.extend_from_slice(format!("{:8}", ov).as_bytes());
                line.extend_from_slice(&q.cyn);
                line.extend_from_slice(b"] ");
                line.extend_from_slice(&short);
                line.extend_from_slice(&q.nrm);
            }
            _ => {
                send_to_char(g, chid, b"Not a valid item type");
                return;
            }
        }
        line.extend_from_slice(b"\r\n");
        // An entry that would cross the end is dropped whole here, unlike
        // the other listings, which truncate it.
        if buf.len + line.len() < MAX_STRING_LENGTH - 1 {
            buf.push(&line);
        } else {
            break;
        }
    }
    page_string(g, chid, &buf.buf);
}

/// perform_obj_aff_list.
///
/// The display loops index `obj_proto[num]` for the column width,
/// the item type and the `[TRIG]` marker, but `num` is the *finished* loop
/// counter — one past the last object, so it reads off the end of the
/// prototype table. `r_num`, the object actually being printed, is what
/// every one of those was meant to be.
fn perform_obj_aff_list(g: &mut Game, chid: CharId, arg: &[u8]) {
    let mut lst = [(NOTHING as i32, 0i32); MAX_OBJ_LIST];
    let apply = atoi(arg);
    if apply <= 0 || apply >= flags::NUM_APPLIES as i32 {
        send_to_char(g, chid, b"Not a valid affect");
        return;
    }
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    let special = apply == flags::APPLY_CLASS || apply == flags::APPLY_LEVEL;

    if special {
        for num in 0..g.world.obj_protos.len() {
            let p = &g.world.obj_protos[num];
            let matches = (apply == flags::APPLY_CLASS && p.type_flag == flags::ITEM_WEAPON)
                || (apply == flags::APPLY_LEVEL && p.type_flag == flags::ITEM_ARMOR);
            if !matches {
                continue;
            }
            let ov = p.vnum as i32;
            let v1 = if apply == flags::APPLY_CLASS {
                (p.values[2] + 1) * p.values[1] / 2
            } else {
                p.values[0]
            };
            if g.world.real_object(ov as Idx).is_some() {
                add_to_obj_list(&mut lst, ov, v1);
            }
        }
        buf.push(if apply == flags::APPLY_CLASS {
            b"Highest average damage per hit for Weapons\r\n"
        } else {
            b"Highest AC Apply for Armor\r\n"
        });
    } else {
        for num in 0..g.world.obj_protos.len() {
            for i in 0..MAX_OBJ_AFFECT {
                let aff = g.world.obj_protos[num].affected[i];
                if aff.modifier == 0 || aff.location != apply {
                    continue;
                }
                let ov = g.world.obj_protos[num].vnum as i32;
                if g.world.real_object(ov as Idx).is_some() {
                    add_to_obj_list(&mut lst, ov, aff.modifier);
                }
            }
        }
        let mut head: BStr = b"Objects with highest ".to_vec();
        head.extend_from_slice(
            APPLY_TYPES.get(apply as usize).copied().unwrap_or("UNDEFINED").as_bytes(),
        );
        head.extend_from_slice(b" affect\r\n");
        buf.push(&head);
    }

    let mut found = 0;
    for (vobj, val) in lst {
        if vobj < 0 {
            continue;
        }
        let Some(r_num) = g.world.real_object(vobj as Idx) else { continue };
        let r_num = r_num as usize;
        let short = obj_short(g, r_num);
        let type_flag = g.world.obj_protos[r_num].type_flag;
        found += 1;

        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:3}", found).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") ");
        line.extend_from_slice(&q.cyn);
        line.push(b'[');
        line.extend_from_slice(&q.yel);
        // The special-case branch prints the vnum in five columns, the
        // general one in eight.
        line.extend_from_slice(
            if special { format!("{:5}", vobj) } else { format!("{:8}", vobj) }.as_bytes(),
        );
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(format!("{:3}", val).as_bytes());
        line.push(b' ');
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad(&short, colw(&short, 42)));
        line.push(b' ');
        line.extend_from_slice(&q.yel);
        line.push(b'[');
        line.extend_from_slice(item_type_name(type_flag).as_bytes());
        line.push(b']');
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(obj_trig(g, r_num, b" [TRIG]"));
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        // Only the special-case loop breaks on overflow.
        if special && buf.overflowed() {
            break;
        }
    }
    page_string(g, chid, &buf.buf);
}

fn perform_obj_name_list(g: &mut Game, chid: CharId, arg: &[u8]) {
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    let mut head: BStr = b"Objects with the name '".to_vec();
    head.extend_from_slice(arg);
    head.extend_from_slice(b"'\r\n");
    head.extend_from_slice(
        b"Index VNum    Num   Object Name                                Object Type\r\n",
    );
    head.extend_from_slice(
        b"----- ------- ----- ------------------------------------------ ----------------\r\n",
    );
    buf.push(&head);

    let mut found = 0;
    for num in 0..g.world.obj_protos.len() {
        let name = g.world.obj_protos[num].name.clone().unwrap_or_default();
        if !is_name(arg, &name) {
            continue;
        }
        let ov = g.world.obj_protos[num].vnum as i32;
        let count = g.obj_counts[num];
        let short = obj_short(g, num);
        let type_flag = g.world.obj_protos[num].type_flag;
        found += 1;

        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:4}", found).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") ");
        line.extend_from_slice(&q.cyn);
        line.push(b'[');
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(format!("{:5}", ov).as_bytes());
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.nrm);
        line.push(b'(');
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:3}", count).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.push(b')');
        line.extend_from_slice(&q.cyn);
        line.push(b' ');
        line.extend_from_slice(&pad(&short, colw(&short, 42)));
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(b" [");
        line.extend_from_slice(item_type_name(type_flag).as_bytes());
        line.push(b']');
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(obj_trig(g, num, b" [TRIG]"));
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        if buf.overflowed() {
            break;
        }
    }
    page_string(g, chid, &buf.buf);
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_list(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    let (smin, smax, _) = two_arguments(argument);
    let mut rzone: Option<usize> = None;
    let mut vmin: i32 = NOWHERE as i32;
    let mut vmax: i32 = NOWHERE as i32;
    let mut use_name = false;

    if smin.is_empty() || smin[0] == b'.' {
        rzone = Some(g.world.rooms[g.ch(chid).in_room as usize].zone as usize);
    } else if smax.is_empty() {
        rzone = g.world.real_zone(atoi(&smin) as Idx).map(|z| z as usize);
        if matches!(rzone, None | Some(0))
            && subcmd == SCMD_OASIS_ZLIST
            && !smin[0].is_ascii_digit()
        {
            // Must be zlist, with a builder name as the argument.
            use_name = true;
        } else if rzone.is_none() {
            send_to_char(g, chid, b"Sorry, there's no zone with that number\r\n");
            return;
        }
    } else {
        vmin = atoi(&smin);
        vmax = atoi(&smax);
        if vmin > vmax {
            let msg = format!("List from {} to {} - Aren't we funny today!\r\n", vmin, vmax);
            send_to_char(g, chid, msg.as_bytes());
            return;
        }
    }

    match subcmd {
        SCMD_OASIS_MLIST => {
            let (arg, arg2, _) = two_arguments(argument);
            if is_abbrev(&arg, b"help") {
                let q = qcolors(g, chid);
                let mut out: BStr = Vec::new();
                let line = |out: &mut BStr, pre: &[u8], body: &[u8], post: &[u8]| {
                    out.extend_from_slice(pre);
                    out.extend_from_slice(&q.yel);
                    out.extend_from_slice(body);
                    out.extend_from_slice(&q.nrm);
                    out.extend_from_slice(post);
                };
                line(&mut out, b"Usage: ", b"mlist <zone>", b"        - List mobiles in a zone\r\n");
                line(
                    &mut out,
                    b"       ",
                    b"mlist <vnum> <vnum>",
                    b" - List a range of mobiles by vnum\r\n",
                );
                line(
                    &mut out,
                    b"       ",
                    b"mlist level <num>",
                    b"   - List all mobiles of a specified level\r\n",
                );
                line(
                    &mut out,
                    b"       ",
                    b"mlist flags <num>",
                    b" - List all mobiles with flag set\r\n",
                );
                line(&mut out, b"Just type ", b"mlist flags", b" to view available options.\r\n");
                send_to_char(g, chid, &out);
                return;
            } else if is_abbrev(&arg, b"level") || is_abbrev(&arg, b"flags") {
                if arg2.is_empty() {
                    mob_flag_help(g, chid);
                    return;
                }
                if is_abbrev(&arg, b"level") {
                    perform_mob_level_list(g, chid, &arg2);
                } else {
                    perform_mob_flag_list(g, chid, &arg2);
                }
            } else {
                list_mobiles(g, chid, rzone, vmin, vmax);
            }
        }
        SCMD_OASIS_OLIST => {
            let (arg, arg2, _) = two_arguments(argument);
            if is_abbrev(&arg, b"help") {
                olist_help(g, chid);
                return;
            } else if is_abbrev(&arg, b"type") || is_abbrev(&arg, b"affect") {
                if is_abbrev(&arg, b"type") {
                    if arg2.is_empty() {
                        olist_type_help(g, chid);
                        return;
                    }
                    perform_obj_type_list(g, chid, &arg2);
                } else {
                    if arg2.is_empty() {
                        olist_affect_help(g, chid);
                        return;
                    }
                    perform_obj_aff_list(g, chid, &arg2);
                }
            } else if !arg.is_empty() && !arg[0].is_ascii_digit() {
                perform_obj_name_list(g, chid, &arg);
            } else {
                list_objects(g, chid, rzone, vmin, vmax);
            }
        }
        SCMD_OASIS_RLIST => list_rooms(g, chid, rzone, vmin, vmax),
        SCMD_OASIS_TLIST => list_triggers(g, chid, rzone, vmin, vmax),
        SCMD_OASIS_SLIST => list_shops(g, chid, rzone, vmin, vmax),
        SCMD_OASIS_QLIST => crate::quest::list_quests(g, chid, rzone, vmin, vmax),
        SCMD_OASIS_ZLIST => {
            let top = g.world.zones.len().saturating_sub(1);
            let highest = g.world.zones[top].number as i32;
            if smin.is_empty() {
                list_zones(g, chid, None, 0, highest, None);
            } else if use_name {
                list_zones(g, chid, None, 0, highest, Some(&smin));
            } else {
                list_zones(g, chid, rzone, vmin, vmax, None);
            }
        }
        _ => {
            send_to_char(g, chid, b"You can't list that!\r\n");
            let msg = format!("SYSERR: do_oasis_list: Unknown list option: {}", subcmd);
            g.mudlog(MudlogKind::Brf, LVL_IMMORT, true, &msg);
        }
    }
}

/// The `mlist flags` / `mlist level` picker. Unlike
/// the usage block above it, this table paints with `CC*(ch, C_NRM)`.
fn mob_flag_help(g: &mut Game, chid: CharId) {
    send_to_char(g, chid, b"Which mobile flag or level do you want to list?\r\n");
    let ccnrm = cc(g, chid, C_NRM, KNRM).to_vec();
    let ccyel = cc(g, chid, C_NRM, KYEL).to_vec();
    for i in 0..flags::NUM_MOB_FLAGS {
        let mut out: BStr = ccnrm.clone();
        out.extend_from_slice(format!("{:2}", i).as_bytes());
        out.extend_from_slice(&ccnrm);
        out.push(b'-');
        out.extend_from_slice(&ccyel);
        out.extend_from_slice(&pad(ACTION_BITS[i].as_bytes(), 14));
        out.extend_from_slice(&ccnrm);
        send_to_char(g, chid, &out);
        if (i + 1) % 4 == 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }
    send_to_char(g, chid, b"\r\n");
    let mut out: BStr = b"Usage: ".to_vec();
    out.extend_from_slice(&ccyel);
    out.extend_from_slice(b"mlist flags <num>");
    out.extend_from_slice(&ccnrm);
    out.extend_from_slice(b"\r\n       ");
    out.extend_from_slice(&ccyel);
    out.extend_from_slice(b"mlist level <num>");
    out.extend_from_slice(&ccnrm);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        b"Displays mobs with the selected flag, or at the selected level\r\n\r\n",
    );
    send_to_char(g, chid, &out);
}

fn olist_help(g: &mut Game, chid: CharId) {
    let q = qcolors(g, chid);
    let mut out: BStr = Vec::new();
    let mut line = |pre: &[u8], body: &[u8], post: &[u8]| {
        out.extend_from_slice(pre);
        out.extend_from_slice(&q.yel);
        out.extend_from_slice(body);
        out.extend_from_slice(&q.nrm);
        out.extend_from_slice(post);
    };
    line(b"Usage: ", b"olist <zone>", b"        - List objects in a zone\r\n");
    line(b"       ", b"olist <vnum> <vnum>", b" - List a range of objects by vnum\r\n");
    line(b"       ", b"olist <name>", b"        - List all named objects with count\r\n");
    line(b"       ", b"olist type <num>", b"    - List all objects of a specified type\r\n");
    line(
        b"       ",
        b"olist affect <num>",
        format!("  - List top {} objects with affect\r\n", MAX_OBJ_LIST).as_bytes(),
    );
    line(b"Just type ", b"olist affect", b" or ");
    line(b"", b"olist type", b" to view available options\r\n");
    send_to_char(g, chid, &out);
}

fn olist_type_help(g: &mut Game, chid: CharId) {
    send_to_char(g, chid, b"Which object type do you want to list?\r\n");
    let q = qcolors(g, chid);
    for i in 1..flags::NUM_ITEM_TYPES {
        let mut out: BStr = q.nrm.clone();
        out.extend_from_slice(format!("{:2}", i).as_bytes());
        out.extend_from_slice(&q.nrm);
        out.push(b'-');
        out.extend_from_slice(&q.yel);
        out.extend_from_slice(&pad(ITEM_TYPES[i].as_bytes(), 14));
        out.extend_from_slice(&q.nrm);
        send_to_char(g, chid, &out);
        if i % 4 == 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }
    send_to_char(g, chid, b"\r\n");
    let mut out: BStr = b"Usage: ".to_vec();
    out.extend_from_slice(&q.yel);
    out.extend_from_slice(b"olist type <num>");
    out.extend_from_slice(&q.nrm);
    out.extend_from_slice(b"\r\nDisplays objects of the selected type.\r\n");
    send_to_char(g, chid, &out);
}

fn olist_affect_help(g: &mut Game, chid: CharId) {
    send_to_char(g, chid, b"Which object affect do you want to list?\r\n");
    let q = qcolors(g, chid);
    for i in 0..flags::NUM_APPLIES {
        let name = if i as i32 == flags::APPLY_CLASS {
            "Weapon Dam" // Special Case 1 - Weapon Dam
        } else if i as i32 == flags::APPLY_LEVEL {
            "AC Apply" // Special Case 2 - Armor AC Apply
        } else {
            APPLY_TYPES[i]
        };
        let mut out: BStr = q.nrm.clone();
        out.extend_from_slice(format!("{:2}-", i).as_bytes());
        out.extend_from_slice(&q.yel);
        out.extend_from_slice(&pad(name.as_bytes(), 14));
        out.extend_from_slice(&q.nrm);
        send_to_char(g, chid, &out);
        if (i + 1) % 4 == 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }
    send_to_char(g, chid, b"\r\n");
    let mut out: BStr = b"Usage: ".to_vec();
    out.extend_from_slice(&q.yel);
    out.extend_from_slice(b"olist affect <num>");
    out.extend_from_slice(&q.nrm);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "Displays top {} objects, in order, with the selected affect.\r\n",
            MAX_OBJ_LIST
        )
        .as_bytes(),
    );
    send_to_char(g, chid, &out);
}

// ---------------------------------------------------------------------------
// The listings themselves
// ---------------------------------------------------------------------------

/// The `[bottom, top]` every listing works over: a zone's bounds, or the
/// explicit vnum range.
fn bounds(g: &Game, rnum: Option<usize>, vmin: i32, vmax: i32) -> (i32, i32) {
    match rnum {
        Some(z) => (g.world.zones[z].bot as i32, g.world.zones[z].top as i32),
        None => (vmin, vmax),
    }
}

fn list_rooms(g: &mut Game, chid: CharId, rnum: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = bounds(g, rnum, vmin, vmax);
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    buf.push(
        b"Index VNum    Room Name                                    Exits\r\n\
          ----- ------- -------------------------------------------- -----\r\n",
    );
    // A one-room world prints nothing at all.
    if g.world.rooms.len() <= 1 {
        return;
    }

    let dirs = crate::fight::dir_count(g);
    let mut counter = 0;
    for i in 0..g.world.rooms.len() {
        let vnum = g.world.rooms[i].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let name = g.world.rooms[i].name.clone().unwrap_or_default();
        let trig: &[u8] =
            if g.world.rooms[i].proto_script.is_empty() { b"" } else { b"[TRIG] " };

        let mut line: BStr = format!("{:4}) [", counter).into_bytes();
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:<5}", vnum).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad(&name, colw(&name, 44)));
        line.extend_from_slice(&q.nrm);
        line.push(b' ');
        line.extend_from_slice(trig);

        let zone = g.world.rooms[i].zone;
        for j in 0..dirs {
            let Some(ex) = g.world.rooms[i].dir_option[j].as_deref() else { continue };
            if ex.to_room == NOWHERE {
                continue;
            }
            let dest = ex.to_room as usize;
            if g.world.rooms[dest].zone == zone {
                continue;
            }
            line.push(b'(');
            line.extend_from_slice(&q.yel);
            line.extend_from_slice(format!("{}", g.world.rooms[dest].vnum).as_bytes());
            line.extend_from_slice(&q.nrm);
            line.push(b')');
        }
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        if buf.overflowed() {
            break;
        }
    }

    if counter == 0 {
        send_to_char(g, chid, b"No rooms found for zone/range specified.\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

fn list_mobiles(g: &mut Game, chid: CharId, rnum: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = bounds(g, rnum, vmin, vmax);
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    buf.push(
        b"Index VNum    Mobile Name                                  Level\r\n\
          ----- ------- -------------------------------------------- -----\r\n",
    );
    if g.world.mob_protos.len() <= 1 {
        return;
    }

    let mut counter = 0;
    for i in 0..g.world.mob_protos.len() {
        let vnum = g.world.mob_protos[i].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let short = g.world.mob_protos[i].short_descr.clone().unwrap_or_default();
        let level = g.world.mob_protos[i].level;
        let trig: &[u8] =
            if g.world.mob_protos[i].proto_script.is_empty() { b"" } else { b" [TRIG]" };

        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:4}", counter).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") [");
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:<5}", vnum).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad(&short, colw(&short, 44)));
        line.push(b' ');
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(format!("[{:4}]", level).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(trig);
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        if buf.overflowed() {
            break;
        }
    }

    if counter == 0 {
        send_to_char(g, chid, b"None found.\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

fn list_objects(g: &mut Game, chid: CharId, rnum: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = bounds(g, rnum, vmin, vmax);
    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    buf.push(
        b"Index VNum    Object Name                                  Object Type\r\n\
          ----- ------- -------------------------------------------- ----------------\r\n",
    );
    if g.world.obj_protos.len() <= 1 {
        return;
    }

    let mut counter = 0;
    for i in 0..g.world.obj_protos.len() {
        let vnum = g.world.obj_protos[i].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let short = obj_short(g, i);
        let type_flag = g.world.obj_protos[i].type_flag;

        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:4}", counter).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") [");
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:<5}", vnum).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad(&short, colw(&short, 44)));
        line.push(b' ');
        line.extend_from_slice(&q.yel);
        line.push(b'[');
        line.extend_from_slice(item_type_name(type_flag).as_bytes());
        line.push(b']');
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(obj_trig(g, i, b" [TRIG]"));
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        if buf.overflowed() {
            break;
        }
    }

    if counter == 0 {
        send_to_char(g, chid, b"None found.\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

/// list_shops. Written straight to the descriptor,
/// with no pager.
fn list_shops(g: &mut Game, chid: CharId, rnum: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = bounds(g, rnum, vmin, vmax);
    let q = qcolors(g, chid);
    send_to_char(
        g,
        chid,
        b"Index VNum    RNum    Shop Room(s)\r\n\
          ----- ------- ------- -----------------------------------------\r\n",
    );

    let mut counter = 0;
    for i in 0..g.world.shops.len() {
        let vnum = g.world.shops[i].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let mut line: BStr = q.grn.clone();
        line.extend_from_slice(format!("{:4}", counter).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b") [");
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:<5}", vnum).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] [");
        line.extend_from_slice(&q.grn);
        // The +1 is strange but fits the rest of the shop code.
        line.extend_from_slice(format!("{:<5}", i + 1).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.push(b']');

        let rooms = g.world.shops[i].in_rooms.clone();
        for (j, room) in rooms.iter().enumerate() {
            if j > 0 && j % 6 == 0 {
                line.extend_from_slice(b"\r\n                      ");
            } else {
                line.push(b' ');
            }
            line.extend_from_slice(&q.cyn);
            line.push(b'[');
            line.extend_from_slice(&q.yel);
            line.extend_from_slice(format!("{:<5}", room).as_bytes());
            line.extend_from_slice(&q.cyn);
            line.push(b']');
            line.extend_from_slice(&q.nrm);
        }
        if rooms.is_empty() {
            line.push(b' ');
            line.extend_from_slice(&q.cyn);
            line.extend_from_slice(b"None.");
            line.extend_from_slice(&q.nrm);
        }
        line.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &line);
    }

    if counter == 0 {
        send_to_char(g, chid, b"None found.\r\n");
    }
}

fn list_zones(
    g: &mut Game,
    chid: CharId,
    rnum: Option<usize>,
    vmin: i32,
    vmax: i32,
    name: Option<&[u8]>,
) {
    let mut bottom = vmin;
    let mut top = vmax;

    if let Some(z) = rnum {
        // Only one parameter was supplied - just list that zone.
        let vnum = g.world.zones[z].number as i32;
        crate::act::wizshow::print_zone(g, chid, vnum);
        return;
    }
    let use_name = name.is_some_and(|n| !n.is_empty());
    if use_name {
        let last = g.world.zones.len().saturating_sub(1);
        if bottom == 0 {
            bottom = g.world.zones[0].number as i32;
        }
        if top == 0 {
            top = g.world.zones[last].number as i32;
        }
    }

    let q = qcolors(g, chid);
    let mut buf = CBuf::new();
    buf.push(
        b"VNum  Zone Name                      Builder(s)\r\n\
          ----- ------------------------------ --------------------------------------\r\n",
    );
    if g.world.zones.len() <= 1 {
        return;
    }

    let mut counter = 0;
    for i in 0..g.world.zones.len() {
        let number = g.world.zones[i].number as i32;
        if number < bottom || number > top {
            continue;
        }
        let builders = g.world.zones[i].builders.clone();
        if use_name && !is_name(name.unwrap(), builders.as_deref().unwrap_or(b"")) {
            continue;
        }
        counter += 1;
        let zname = g.world.zones[i].name.clone().unwrap_or_default();

        let mut line: BStr = b"[".to_vec();
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:3}", number).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad(&zname, colw(&zname, 30)));
        line.push(b' ');
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(builders.as_deref().unwrap_or(b"None."));
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"\r\n");
        buf.push(&line);
        if buf.overflowed() {
            break;
        }
    }

    if counter == 0 {
        send_to_char(g, chid, b"  None found within those parameters.\r\n");
    } else {
        page_string(g, chid, &buf.buf);
    }
}

/// list_triggers. Written straight to the descriptor rather than paged.
fn list_triggers(g: &mut Game, chid: CharId, rnum: Option<usize>, vmin: i32, vmax: i32) {
    let (bottom, top) = bounds(g, rnum, vmin, vmax);
    let q = qcolors(g, chid);
    send_to_char(
        g,
        chid,
        b"Index VNum    Trigger Name                                  Type\r\n\
          ----- ------- --------------------------------------------- ---------\r\n",
    );

    let mut counter = 0;
    for i in 0..g.world.triggers.len() {
        let vnum = g.world.triggers[i].vnum as i32;
        if vnum < bottom || vnum > top {
            continue;
        }
        counter += 1;
        let name = g.world.triggers[i].name.clone().unwrap_or_default();
        let attach = g.world.triggers[i].attach_type;
        let ttype = g.world.triggers[i].trigger_type as i64;

        let mut line: BStr = format!("{:4}) [", counter).into_bytes();
        line.extend_from_slice(&q.grn);
        line.extend_from_slice(format!("{:5}", vnum).as_bytes());
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"] ");
        line.extend_from_slice(&q.cyn);
        line.extend_from_slice(&pad_trunc(&name, 45));
        line.extend_from_slice(&q.nrm);
        line.push(b' ');

        let (label, bits) = if attach == crate::dg::OBJ_TRIGGER {
            (&b"obj "[..], crate::quest::sprintbit(ttype, &OTRIG_TYPES))
        } else if attach == crate::dg::WLD_TRIGGER {
            (&b"wld "[..], crate::quest::sprintbit(ttype, &WTRIG_TYPES))
        } else {
            (&b"mob "[..], crate::quest::sprintbit(ttype, &TRIG_TYPES))
        };
        line.extend_from_slice(label);
        line.extend_from_slice(&q.yel);
        line.extend_from_slice(&bits);
        line.extend_from_slice(&q.nrm);
        line.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &line);
    }

    if counter == 0 {
        let msg = match rnum {
            None => format!("No triggers found from {} to {}\r\n", vmin, vmax),
            Some(z) => {
                format!("No triggers found for zone #{}\r\n", g.world.zones[z].number)
            }
        };
        send_to_char(g, chid, msg.as_bytes());
    }
}
