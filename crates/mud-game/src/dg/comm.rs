//! Sub_write (the ~ | ^ & * ` token codes), send_to_zone, and
//! send_to_range (used by *recho).

use mud_data::ids::CharId;
use mud_data::types::*;

use crate::game::Game;
use crate::handler::{can_see, can_see_obj, obj_short, pers};

use super::{get_char_in_room, get_obj_in_room};

pub type BStr = Vec<u8>;

pub const TO_ROOM_T: i32 = 1;
pub const TO_CHAR_T: i32 = 4;

/// any_one_name: like any_one_arg but stops at punctuation
/// (except '#' and '-'); lowercases.
pub fn any_one_name(argument: &[u8]) -> (BStr, &[u8]) {
    let is_sp = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let mut i = 0;
    while i < argument.len() && is_sp(argument[i]) {
        i += 1;
    }
    let mut out = Vec::new();
    while i < argument.len() {
        let c = argument[i];
        if is_sp(c) {
            break;
        }
        if c.is_ascii_punctuation() && c != b'#' && c != b'-' {
            break;
        }
        out.push(c.to_ascii_lowercase());
        i += 1;
    }
    (out, &argument[i..])
}

enum Tok {
    Char(Option<CharId>, u8),
    Obj(Option<mud_data::ids::ObjId>),
}

/// SENDOK as sub_write uses it: to_sleeping is hardcoded 1, so sleepers get
/// the text; descriptor-less NPCs with an Act trigger count as receivers.
fn sub_sendok(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    (ch.desc.is_some() || g.script_check(super::GoId::Char(chid), super::MTRIG_ACT))
        && !ch.plr(mud_data::flags::PLR_WRITING)
}

/// sub_write: render token codes anchored at `ch`, deliver
/// per `targets` (TO_CHAR to ch and/or TO_ROOM around ch).
pub fn sub_write(g: &mut Game, arg: &[u8], chid: CharId, _find_invis: bool, targets: i32) {
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return;
    }

    // Tokenize: literal runs + entity lookups (resolved once, at the anchor).
    let mut literals: Vec<BStr> = Vec::new();
    let mut toks: Vec<Tok> = Vec::new();
    let mut cur: BStr = Vec::new();
    let mut p = 0usize;
    while p < arg.len() {
        match arg[p] {
            c @ (b'~' | b'|' | b'^' | b'&' | b'*') => {
                literals.push(std::mem::take(&mut cur));
                let (name, rest) = any_one_name(&arg[p + 1..]);
                p = arg.len() - rest.len();
                // find_invis is TRUE from every engine call site.
                let target = get_char_in_room(g, room, &name);
                toks.push(Tok::Char(target, c));
            }
            b'`' => {
                literals.push(std::mem::take(&mut cur));
                let (name, rest) = any_one_name(&arg[p + 1..]);
                p = arg.len() - rest.len();
                let target = get_obj_in_room(g, room, &name);
                toks.push(Tok::Obj(target));
            }
            b'\\' => {
                p += 1;
                if p < arg.len() {
                    cur.push(arg[p]);
                    p += 1;
                }
            }
            c => {
                cur.push(c);
                p += 1;
            }
        }
    }
    literals.push(cur);

    if targets & TO_CHAR_T != 0 && sub_sendok(g, chid) {
        let msg = render(g, chid, &literals, &toks);
        deliver(g, chid, &msg);
    }
    if targets & TO_ROOM_T != 0 {
        let people = g.rooms[room as usize].people.clone();
        for to in people {
            if to == chid {
                continue;
            }
            if g.try_ch(to).is_none() || !sub_sendok(g, to) {
                continue;
            }
            let msg = render(g, to, &literals, &toks);
            deliver(g, to, &msg);
        }
    }
}

fn render(g: &mut Game, to: CharId, literals: &[BStr], toks: &[Tok]) -> BStr {
    let mut sb: BStr = Vec::new();
    for (i, tok) in toks.iter().enumerate() {
        sb.extend_from_slice(&literals[i]);
        match tok {
            Tok::Char(target, code) => match code {
                b'~' => match target {
                    None => sb.extend_from_slice(b"someone"),
                    Some(t) if *t == to => sb.extend_from_slice(b"you"),
                    Some(t) => sb.extend_from_slice(&pers(g, to, *t)),
                },
                b'|' => match target {
                    None => sb.extend_from_slice(b"someone's"),
                    Some(t) if *t == to => sb.extend_from_slice(b"your"),
                    Some(t) => {
                        sb.extend_from_slice(&pers(g, to, *t));
                        sb.extend_from_slice(b"'s");
                    }
                },
                b'^' => match target {
                    Some(t) if can_see(g, to, *t) => {
                        if *t == to {
                            sb.extend_from_slice(b"your");
                        } else {
                            sb.extend_from_slice(hshr(g.ch(*t).sex));
                        }
                    }
                    _ => sb.extend_from_slice(b"its"),
                },
                b'&' => match target {
                    Some(t) if can_see(g, to, *t) => {
                        if *t == to {
                            sb.extend_from_slice(b"you");
                        } else {
                            sb.extend_from_slice(hssh(g.ch(*t).sex));
                        }
                    }
                    _ => sb.extend_from_slice(b"it"),
                },
                b'*' => match target {
                    Some(t) if can_see(g, to, *t) => {
                        if *t == to {
                            sb.extend_from_slice(b"you");
                        } else {
                            sb.extend_from_slice(hmhr(g.ch(*t).sex));
                        }
                    }
                    _ => sb.extend_from_slice(b"it"),
                },
                _ => {}
            },
            Tok::Obj(target) => match target {
                Some(o) if g.try_obj(*o).is_some() && can_see_obj(g, to, *o) => {
                    sb.extend_from_slice(obj_short(g, *o));
                }
                Some(_) => sb.extend_from_slice(b"something"),
                None => sb.extend_from_slice(b"something"),
            },
        }
    }
    sb.extend_from_slice(literals.last().map(|l| l.as_slice()).unwrap_or(b""));
    // "\n\r" here, reversed — a wire fingerprint.
    sb.extend_from_slice(b"\n\r");
    sb
}

fn deliver(g: &mut Game, to: CharId, msg: &[u8]) {
    crate::comm::send_to_char(g, to, msg);
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

/// send_to_zone: every playing, awake descriptor in the zone.
pub fn send_to_zone(g: &mut Game, msg: &[u8], zone: usize) {
    if msg.is_empty() {
        return;
    }
    let order = g.descriptors.order.clone();
    for di in order {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if !ch.awake() || ch.in_room == NOWHERE {
            continue;
        }
        if g.world.rooms[ch.in_room as usize].zone as usize != zone {
            continue;
        }
        crate::comm::write_to_desc(g, di, msg);
    }
}

/// Rooms with vnum in [start,finish]. Note the bound,
/// which skips the highest room — deliberate.
pub fn send_to_range(g: &mut Game, start: i32, finish: i32, msg: &[u8]) {
    if start > finish {
        g.log("send_to_range passed start room value greater then finish.".to_string());
        return;
    }
    // The last room in the table is included.
    for j in 0..g.world.rooms.len() {
        let vnum = g.world.rooms[j].vnum as i32;
        if vnum >= start && vnum <= finish {
            let people = g.rooms[j].people.clone();
            for chid in people {
                let Some(di) = g.try_ch(chid).and_then(|c| c.desc) else { continue };
                crate::comm::write_to_desc(g, di, msg);
            }
        }
    }
}
