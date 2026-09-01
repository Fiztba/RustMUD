//! Output helpers, the act engine, and the legacy ANSI colour macros.

use mud_data::flags::{self};
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::game::Game;
use crate::handler::{can_see, can_see_obj, fname, obj_name, obj_short, pers};

pub const TO_ROOM: i32 = 1;
pub const TO_VICT: i32 = 2;
pub const TO_NOTVICT: i32 = 3;
pub const TO_CHAR: i32 = 4;
pub const TO_GMOTE: i32 = 5;
pub const TO_SLEEP: i32 = 128;
pub const DG_NO_TRIG: i32 = 256;

// Legacy ANSI (raw, bypassing protocol translation).
pub const KNRM: &[u8] = b"\x1B[0m";
pub const KRED: &[u8] = b"\x1B[0;31m";
pub const KGRN: &[u8] = b"\x1B[0;32m";
pub const KYEL: &[u8] = b"\x1B[0;33m";
pub const KBLU: &[u8] = b"\x1B[0;34m";
pub const KMAG: &[u8] = b"\x1B[0;35m";
pub const KCYN: &[u8] = b"\x1B[0;36m";
pub const KWHT: &[u8] = b"\x1B[0;37m";
pub const KBRED: &[u8] = b"\x1B[1;31m";
pub const KBGRN: &[u8] = b"\x1B[1;32m";
pub const KBYEL: &[u8] = b"\x1B[1;33m";
pub const KBWHT: &[u8] = b"\x1B[1;37m";
pub const KNUL: &[u8] = b"";

pub const C_OFF: u8 = 0;
pub const C_SPR: u8 = 1;
pub const C_NRM: u8 = 2;
pub const C_CMP: u8 = 3;

/// clr(ch, lvl).
pub fn clr(g: &Game, chid: CharId, lvl: u8) -> bool {
    g.ch(chid).color_lev() >= lvl
}

/// CC*(ch, lvl) helper.
pub fn cc(g: &Game, chid: CharId, lvl: u8, color: &'static [u8]) -> &'static [u8] {
    if clr(g, chid, lvl) { color } else { KNUL }
}

/// The color gate handed to protocol_output: clr(ch, C_CMP), true if no char.
pub fn color_allowed_for_desc(g: &Game, di: usize) -> bool {
    match g.descriptors.get(di).and_then(|d| d.character) {
        Some(chid) => match g.try_ch(chid) {
            Some(_) => clr(g, chid, C_CMP),
            None => true,
        },
        None => true,
    }
}

/// send_to_char: write to the char's descriptor if any.
pub fn send_to_char(g: &mut Game, chid: CharId, txt: &[u8]) {
    let Some(di) = g.try_ch(chid).and_then(|c| c.desc) else { return };
    write_to_desc(g, di, txt);
}

/// Same but explicitly named for callsites that pre-colored the text.
pub fn send_to_char_color(g: &mut Game, chid: CharId, txt: &[u8]) {
    send_to_char(g, chid, txt);
}

/// write_to_output on a descriptor with the char color gate.
pub fn write_to_desc(g: &mut Game, di: usize, txt: &[u8]) {
    let _ = write_to_desc_len(g, di, txt);
}

/// write_to_output's return value: the descriptor's remaining buffer space
/// (0 in the overflow state). `send_to_char` hands this straight back, and
/// the column-wrapping listings in `stat` add it to their running width —
/// which is why those lists wrap after every entry.
pub fn write_to_desc_len(g: &mut Game, di: usize, txt: &[u8]) -> usize {
    let allowed = color_allowed_for_desc(g, di);
    let (bugs, left) = g.descriptors.write_to_output(di, txt, allowed);
    for b in bugs {
        g.log(b);
    }
    left
}

/// send_to_char, returning the number of bytes buffered.
pub fn send_to_char_len(g: &mut Game, chid: CharId, txt: &[u8]) -> usize {
    let Some(di) = g.try_ch(chid).and_then(|c| c.desc) else { return 0 };
    if txt.is_empty() {
        return 0;
    }
    write_to_desc_len(g, di, txt)
}

/// write_to_descriptor: straight to the socket, bypassing the
/// output buffer. Copyover uses it so its notices land before the exec.
pub fn write_direct(g: &mut Game, di: usize, txt: &[u8]) {
    if let Some(d) = g.descriptors.get_mut(di) {
        let _ = d.write_direct(txt);
    }
}

/// Every descriptor in the playing state.
pub fn send_to_all(g: &mut Game, txt: &[u8]) {
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        write_to_desc(g, di, txt);
    }
}

/// send_to_outdoor: playing, awake, outside.
pub fn send_to_outdoor(g: &mut Game, txt: &[u8]) {
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if !ch.awake() || !outside(g, chid) {
            continue;
        }
        write_to_desc(g, di, txt);
    }
}

/// OUTSIDE(ch) = !ROOM_INDOORS.
pub fn outside(g: &Game, chid: CharId) -> bool {
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return false;
    }
    g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_INDOORS) == 0
}

pub fn send_to_room(g: &mut Game, room: RoomRnum, txt: &[u8]) {
    let people = g.rooms[room as usize].people.clone();
    for chid in people {
        let Some(di) = g.try_ch(chid).and_then(|c| c.desc) else { continue };
        write_to_desc(g, di, txt);
    }
}

/// HMHR/HSHR/HSSH pronoun tables.
pub fn hmhr(sex: u8) -> &'static [u8] {
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

pub fn hssh(sex: u8) -> &'static [u8] {
    match sex {
        SEX_MALE => b"he",
        SEX_FEMALE => b"she",
        _ => b"it",
    }
}

/// SANA(obj): "an"/"a" by first letter of the keyword list.
fn sana(g: &Game, oid: ObjId) -> &'static [u8] {
    let name = obj_name(g, oid);
    if name.first().is_some_and(|c| b"aeiouAEIOU".contains(c)) {
        b"an"
    } else {
        b"a"
    }
}

/// CAP: uppercase the first letter after any color/ANSI prefix.
pub fn cap(txt: &mut [u8]) {
    let mut p = 0usize;
    loop {
        if p + 1 < txt.len() && txt[p] == b'\t' {
            p += 2;
        } else if p + 1 < txt.len() && txt[p] == 0x1B && txt[p + 1] == b'[' {
            p += 2;
            while p < txt.len() && !txt[p].is_ascii_alphabetic() {
                p += 1;
            }
            if p < txt.len() {
                p += 1;
            }
        } else {
            break;
        }
    }
    if p < txt.len() {
        txt[p] = txt[p].to_ascii_uppercase();
    }
}

/// Either party of `$T`/`$t` raw-text arguments.
#[derive(Clone, Copy)]
pub enum ActArg<'a> {
    None,
    Char(CharId),
    Obj(ObjId),
    Text(&'a [u8]),
}

/// perform_act: expand $-codes for one receiver.
pub(crate) fn perform_act(
    g: &mut Game,
    orig: &[u8],
    ch: Option<CharId>,
    obj: Option<ObjId>,
    vict_obj: ActArg,
    to: CharId,
) -> Vec<u8> {
    // dg_act_check is set by act, so direct perform_act callers
    // (say_spell) see the value the last act call left behind.
    let dg_act_check = g.dg_act_check;
    let mut out: Vec<u8> = Vec::with_capacity(orig.len() + 16);
    let mut uppercase_next = false;
    let mut idx = 0usize;
    // DG victim/target/arg tracking: dg_victim defaults
    // to the receiver when they ARE the vict_obj.
    let mut dg_victim: Option<CharId> = match vict_obj {
        ActArg::Char(v) if v == to => Some(v),
        _ => None,
    };
    let mut dg_target: Option<ObjId> = None;
    let mut dg_arg: Option<Vec<u8>> = None;

    macro_rules! push_str {
        ($bytes:expr) => {{
            for &b in $bytes.iter() {
                if uppercase_next && !b.is_ascii_whitespace() {
                    out.push(b.to_ascii_uppercase());
                    uppercase_next = false;
                } else {
                    out.push(b);
                }
            }
        }};
    }

    while idx < orig.len() {
        let c = orig[idx];
        if c == b'$' {
            idx += 1;
            let code = orig.get(idx).copied().unwrap_or(0);
            let expansion: Vec<u8> = match code {
                b'n' => ch.map(|c| pers(g, to, c)).unwrap_or_default(),
                b'N' => match vict_obj {
                    ActArg::Char(v) => {
                        dg_victim = Some(v);
                        pers(g, to, v)
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b'm' => ch.map(|c| hmhr(g.ch(c).sex).to_vec()).unwrap_or_default(),
                b'M' => match vict_obj {
                    ActArg::Char(v) => {
                        dg_victim = Some(v);
                        hmhr(g.ch(v).sex).to_vec()
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b's' => ch.map(|c| hshr(g.ch(c).sex).to_vec()).unwrap_or_default(),
                b'S' => match vict_obj {
                    ActArg::Char(v) => {
                        dg_victim = Some(v);
                        hshr(g.ch(v).sex).to_vec()
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b'e' => ch.map(|c| hssh(g.ch(c).sex).to_vec()).unwrap_or_default(),
                b'E' => match vict_obj {
                    ActArg::Char(v) => {
                        dg_victim = Some(v);
                        hssh(g.ch(v).sex).to_vec()
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b'o' => match obj {
                    Some(o) => objn(g, o, to),
                    None => b"<NULL>".to_vec(),
                },
                b'O' => match vict_obj {
                    ActArg::Obj(o) => {
                        dg_target = Some(o);
                        objn(g, o, to)
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b'p' => match obj {
                    Some(o) => objs_vis(g, o, to),
                    None => b"<NULL>".to_vec(),
                },
                b'P' => match vict_obj {
                    ActArg::Obj(o) => {
                        dg_target = Some(o);
                        objs_vis(g, o, to)
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b'a' => match obj {
                    Some(o) => sana(g, o).to_vec(),
                    None => b"<NULL>".to_vec(),
                },
                b'A' => match vict_obj {
                    ActArg::Obj(o) => sana(g, o).to_vec(),
                    _ => b"<NULL>".to_vec(),
                },
                b'T' => match vict_obj {
                    ActArg::Text(t) => {
                        dg_arg = Some(t.to_vec());
                        t.to_vec()
                    }
                    _ => b"<NULL>".to_vec(),
                },
                b't' => match vict_obj {
                    // $t reads `obj` as text; used by DG only.
                    _ => b"<NULL>".to_vec(),
                },
                b'F' => match vict_obj {
                    ActArg::Text(t) => fname(t),
                    _ => b"<NULL>".to_vec(),
                },
                b'u' => {
                    // Uppercase previous word.
                    let mut j = out.len();
                    while j > 0 && !out[j - 1].is_ascii_whitespace() {
                        j -= 1;
                    }
                    if j != out.len() {
                        out[j] = out[j].to_ascii_uppercase();
                    }
                    Vec::new()
                }
                b'U' => {
                    uppercase_next = true;
                    Vec::new()
                }
                b'$' => b"$".to_vec(),
                _ => {
                    g.log(format!("SYSERR: Illegal $-code to act(): {}", code as char));
                    g.log(format!("SYSERR: {}", String::from_utf8_lossy(&orig[idx..])));
                    Vec::new()
                }
            };
            push_str!(&expansion);
            idx += 1;
        } else {
            if uppercase_next && !c.is_ascii_whitespace() {
                out.push(c.to_ascii_uppercase());
                uppercase_next = false;
            } else {
                out.push(c);
            }
            idx += 1;
        }
    }
    out.extend_from_slice(b"\r\n");

    // CAP happens inside the desc-write, so descriptor-less receivers
    // (act-trigger mobs) get the UNcapitalized text.
    if let Some(di) = g.ch(to).desc {
        cap(&mut out);
        write_to_desc(g, di, &out.clone());
    }

    // act triggers: nag the mobs.
    if g.ch(to).is_npc() && dg_act_check && ch != Some(to) {
        crate::dg::triggers::act_mtrigger(
            g,
            to,
            &out.clone(),
            ch,
            dg_victim,
            obj,
            dg_target,
            dg_arg.as_deref(),
        );
    }
    out
}

fn objn(g: &Game, oid: ObjId, to: CharId) -> Vec<u8> {
    if can_see_obj(g, to, oid) {
        fname(obj_name(g, oid))
    } else {
        b"something".to_vec()
    }
}

pub fn objs_vis(g: &Game, oid: ObjId, to: CharId) -> Vec<u8> {
    if can_see_obj(g, to, oid) {
        obj_short(g, oid).to_vec()
    } else {
        b"something".to_vec()
    }
}

/// SENDOK: desc OR an Act-trigger script, awake gate,
/// not writing. Dead-pending chars still receive.
fn sendok(g: &Game, chid: CharId, to_sleeping: bool) -> bool {
    let ch = g.ch(chid);
    (ch.desc.is_some() || g.script_check(crate::dg::GoId::Char(chid), crate::dg::MTRIG_ACT))
        && (to_sleeping || ch.awake())
        && !ch.plr(flags::PLR_WRITING)
}

/// act. Returns the last rendered message (DG uses it).
pub fn act(
    g: &mut Game,
    s: &[u8],
    hide_invisible: bool,
    ch: Option<CharId>,
    obj: Option<ObjId>,
    vict_obj: Option<CharId>,
    type_: i32,
) -> Option<Vec<u8>> {
    let varg = match vict_obj {
        Some(v) => ActArg::Char(v),
        None => ActArg::None,
    };
    act_full(g, s, hide_invisible, ch, obj, varg, type_)
}

pub fn act_full(
    g: &mut Game,
    s: &[u8],
    hide_invisible: bool,
    ch: Option<CharId>,
    obj: Option<ObjId>,
    vict_obj: ActArg,
    mut type_: i32,
) -> Option<Vec<u8>> {
    if s.is_empty() {
        return None;
    }
    let to_sleeping = type_ & TO_SLEEP != 0;
    if to_sleeping {
        type_ &= !TO_SLEEP;
    }
    g.dg_act_check = type_ & DG_NO_TRIG == 0;
    if type_ & DG_NO_TRIG != 0 {
        type_ &= !DG_NO_TRIG;
    }

    if type_ == TO_CHAR {
        if let Some(chid) = ch {
            if sendok(g, chid, to_sleeping) {
                return Some(perform_act(g, s, ch, obj, vict_obj, chid));
            }
        }
        return None;
    }
    if type_ == TO_VICT {
        if let ActArg::Char(to) = vict_obj {
            if sendok(g, to, to_sleeping) {
                return Some(perform_act(g, s, ch, obj, vict_obj, to));
            }
        }
        return None;
    }
    if type_ == TO_GMOTE {
        let mut last = None;
        for di in g.descriptors.indices() {
            let Some(d) = g.descriptors.get(di) else { continue };
            if d.state != ConState::Playing {
                continue;
            }
            let Some(to) = d.character else { continue };
            let Some(toc) = g.try_ch(to) else { continue };
            if toc.prf(flags::PRF_NOGOSS) || toc.plr(flags::PLR_WRITING) {
                continue;
            }
            let room = toc.in_room;
            if room != NOWHERE && g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_SOUNDPROOF) != 0 {
                continue;
            }
            let mut buf = Vec::with_capacity(s.len() + 16);
            buf.extend_from_slice(cc(g, to, C_NRM, KYEL));
            buf.extend_from_slice(s);
            buf.extend_from_slice(cc(g, to, C_NRM, KNRM));
            last = Some(perform_act(g, &buf, ch, obj, vict_obj, to));
        }
        return last;
    }

    // TO_ROOM / TO_NOTVICT.
    let room = match ch.map(|c| g.ch(c).in_room) {
        Some(r) if r != NOWHERE => r,
        _ => match obj.map(|o| g.obj(o).in_room) {
            Some(r) if r != NOWHERE => r,
            _ => {
                g.log("SYSERR: no valid target to act()!".to_string());
                return None;
            }
        },
    };
    let people = g.rooms[room as usize].people.clone();
    let mut last = None;
    for to in people {
        if !sendok(g, to, to_sleeping) || Some(to) == ch {
            continue;
        }
        if hide_invisible {
            if let Some(chid) = ch {
                if !can_see(g, to, chid) {
                    continue;
                }
            }
        }
        if type_ != TO_ROOM {
            if let ActArg::Char(v) = vict_obj {
                if to == v {
                    continue;
                }
            }
        }
        last = Some(perform_act(g, s, ch, obj, vict_obj, to));
    }
    last
}

/// send_to_group: every PC member with a playing
/// descriptor except `exclude` gets a per-recipient-colored `[Group] `
/// prefix and the (caller-formatted) body. Members in list order.
pub fn send_to_group(g: &mut Game, exclude: Option<CharId>, gid: u64, body: &[u8]) {
    let members = match g.group(gid) {
        Some(gr) => gr.members.clone(),
        None => return,
    };
    for tch in members {
        if Some(tch) == exclude {
            continue;
        }
        let Some(c) = g.try_ch(tch) else { continue };
        if c.is_npc() {
            continue;
        }
        let Some(di) = c.desc else { continue };
        if g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
            continue;
        }
        let mut buf = Vec::with_capacity(body.len() + 24);
        buf.extend_from_slice(cc(g, tch, C_NRM, KGRN));
        buf.push(b'[');
        buf.extend_from_slice(cc(g, tch, C_NRM, KBGRN));
        buf.extend_from_slice(b"Group");
        buf.extend_from_slice(cc(g, tch, C_NRM, KGRN));
        buf.push(b']');
        buf.extend_from_slice(cc(g, tch, C_NRM, KNRM));
        buf.push(b' ');
        buf.extend_from_slice(body);
        write_to_desc(g, di, &buf);
    }
}

/// game_info: `\tcInfo: \ty<msg>\tn\r\n` to all playing.
pub fn game_info(g: &mut Game, msg: &[u8]) {
    let mut buf = Vec::with_capacity(msg.len() + 20);
    buf.extend_from_slice(b"\tcInfo: \ty");
    buf.extend_from_slice(msg);
    buf.extend_from_slice(b"\tn\r\n");
    send_to_all(g, &buf);
}

/// string_write: hand a descriptor to the line editor.
/// `mail_to` doubles as the board marker (`BOARD_MAGIC + board`).
/// `backstr` is the text restored on abort.
///
/// `string_add` APPENDS to whatever the edited field already holds, so
/// editing a mob description or a quest message starts with the old text in
/// the buffer and the length check counts it. Every caller passes `backstr`
/// as a copy of that same field, and the buffer is seeded from it. Abort
/// restores `backstr`, which is left untouched.
pub fn string_write(
    g: &mut Game,
    chid: CharId,
    max_str: usize,
    mail_to: i64,
    backstr: Option<Vec<u8>>,
) {
    if !g.ch(chid).is_npc() {
        g.ch_mut(chid).act.set(flags::PLR_WRITING);
    }
    let Some(di) = g.ch(chid).desc else { return };
    let Some(d) = g.descriptors.get_mut(di) else { return };
    d.editing = Some(mud_net::descriptor::EditSession {
        buf: mud_net::editor::EditBuf { buf: backstr.clone(), max_str },
        backstr,
        mail_to,
        str_slot: -1,
        note_obj: None,
    });
}

pub fn send_editor_help(g: &mut Game, chid: CharId) {
    send_to_char(g, chid, b"Instructions: /s to save, /h for more options.\r\n");
}
