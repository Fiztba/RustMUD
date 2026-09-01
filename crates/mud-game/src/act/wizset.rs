//! `set` — the field table, `perform_set`, and the
//! player-rename machinery it calls.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::wizstat::{ALL_PERMISSION, AEDIT_PERMISSION, HEDIT_PERMISSION};
use crate::act::{pad_right, BStr};
use crate::comm::{cc, send_to_char, C_NRM, KNRM, KYEL};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, is_abbrev};
use crate::interpreter::{half_chop, is_number, one_argument};

/// pc_class_types as &str, for sprinttype.
pub const PC_CLASS_TYPES_STR: [&str; 4] = ["Magic User", "Cleric", "Thief", "Warrior"];

const PC: u8 = 1;
const NPC: u8 = 2;
const BOTH: u8 = 3;

const MISC: u8 = 0;
const BINARY: u8 = 1;
const NUMBER: u8 = 2;

struct SetField {
    cmd: &'static [u8],
    level: u8,
    pcnpc: u8,
    type_: u8,
}

const fn f(cmd: &'static [u8], level: u8, pcnpc: u8, type_: u8) -> SetField {
    SetField { cmd, level, pcnpc, type_ }
}

/// set_fields[] — index IS the case label below.
const SET_FIELDS: [SetField; 58] = [
    f(b"ac", LVL_BUILDER, BOTH, NUMBER),
    f(b"afk", LVL_BUILDER, PC, BINARY),
    f(b"age", LVL_GOD, BOTH, NUMBER),
    f(b"align", LVL_BUILDER, BOTH, NUMBER),
    f(b"bank", LVL_BUILDER, PC, NUMBER),
    f(b"brief", LVL_GOD, PC, BINARY),
    f(b"cha", LVL_BUILDER, BOTH, NUMBER),
    f(b"class", LVL_BUILDER, BOTH, MISC),
    f(b"color", LVL_GOD, PC, BINARY),
    f(b"con", LVL_BUILDER, BOTH, NUMBER),
    f(b"damroll", LVL_BUILDER, BOTH, NUMBER),
    f(b"deleted", LVL_IMPL, PC, BINARY),
    f(b"dex", LVL_BUILDER, BOTH, NUMBER),
    f(b"drunk", LVL_BUILDER, BOTH, MISC),
    f(b"exp", LVL_GOD, BOTH, NUMBER),
    f(b"frozen", LVL_GRGOD, PC, BINARY),
    f(b"gold", LVL_BUILDER, BOTH, NUMBER),
    f(b"height", LVL_BUILDER, BOTH, NUMBER),
    f(b"hitpoints", LVL_BUILDER, BOTH, NUMBER),
    f(b"hitroll", LVL_BUILDER, BOTH, NUMBER),
    f(b"hunger", LVL_BUILDER, BOTH, MISC),
    f(b"int", LVL_BUILDER, BOTH, NUMBER),
    f(b"invis", LVL_GOD, PC, NUMBER),
    f(b"invstart", LVL_BUILDER, PC, BINARY),
    f(b"killer", LVL_GOD, PC, BINARY),
    f(b"level", LVL_GRGOD, BOTH, NUMBER),
    f(b"loadroom", LVL_BUILDER, PC, MISC),
    f(b"mana", LVL_BUILDER, BOTH, NUMBER),
    f(b"maxhit", LVL_BUILDER, BOTH, NUMBER),
    f(b"maxmana", LVL_BUILDER, BOTH, NUMBER),
    f(b"maxmove", LVL_BUILDER, BOTH, NUMBER),
    f(b"move", LVL_BUILDER, BOTH, NUMBER),
    f(b"name", LVL_IMMORT, PC, MISC),
    f(b"nodelete", LVL_GOD, PC, BINARY),
    f(b"nohassle", LVL_GOD, PC, BINARY),
    f(b"nosummon", LVL_BUILDER, PC, BINARY),
    f(b"nowizlist", LVL_GRGOD, PC, BINARY),
    f(b"olc", LVL_GRGOD, PC, MISC),
    f(b"password", LVL_GRGOD, PC, MISC),
    f(b"poofin", LVL_IMMORT, PC, MISC),
    f(b"poofout", LVL_IMMORT, PC, MISC),
    f(b"practices", LVL_GOD, PC, NUMBER),
    f(b"quest", LVL_GOD, PC, BINARY),
    f(b"room", LVL_BUILDER, BOTH, NUMBER),
    f(b"screenwidth", LVL_GOD, PC, NUMBER),
    f(b"sex", LVL_GOD, BOTH, MISC),
    f(b"showvnums", LVL_BUILDER, PC, BINARY),
    f(b"siteok", LVL_GOD, PC, BINARY),
    f(b"str", LVL_BUILDER, BOTH, NUMBER),
    f(b"stradd", LVL_BUILDER, BOTH, NUMBER),
    f(b"thief", LVL_GOD, PC, BINARY),
    f(b"thirst", LVL_BUILDER, BOTH, MISC),
    f(b"title", LVL_GOD, PC, MISC),
    f(b"variable", LVL_GRGOD, PC, MISC),
    f(b"weight", LVL_BUILDER, BOTH, NUMBER),
    f(b"wis", LVL_BUILDER, BOTH, NUMBER),
    f(b"questpoints", LVL_GOD, PC, NUMBER),
    f(b"questhistory", LVL_GOD, PC, NUMBER),
];

fn set_or_remove(g: &mut Game, vict: CharId, plr: bool, bit: usize, on: bool, off: bool) {
    if on {
        if plr {
            g.ch_mut(vict).act.set(bit);
        } else {
            g.ch_mut(vict).ps_mut().pref.set(bit);
        }
    } else if off {
        if plr {
            g.ch_mut(vict).act.remove(bit);
        } else {
            g.ch_mut(vict).ps_mut().pref.remove(bit);
        }
    }
}

fn perform_set(g: &mut Game, chid: CharId, vict: CharId, mode: usize, val_arg: &[u8]) -> bool {
    let field = &SET_FIELDS[mode];

    if g.ch(chid).level != LVL_IMPL
        && !g.ch(vict).is_npc()
        && g.ch(chid).level <= g.ch(vict).level
        && vict != chid
    {
        send_to_char(g, chid, b"Maybe that's not such a great idea...\r\n");
        return false;
    }
    if g.ch(chid).level < field.level {
        send_to_char(g, chid, b"You are not godly enough for that!\r\n");
        return false;
    }
    let is_npc = g.ch(vict).is_npc();
    if is_npc && field.pcnpc & NPC == 0 {
        send_to_char(g, chid, b"You can't do that to a beast!\r\n");
        return false;
    } else if !is_npc && field.pcnpc & PC == 0 {
        send_to_char(g, chid, b"That can only be done to a beast!\r\n");
        return false;
    }

    let (mut on, mut off) = (false, false);
    let mut value = 0i32;
    if field.type_ == BINARY {
        if val_arg == b"on" || val_arg == b"yes" {
            on = true;
        } else if val_arg == b"off" || val_arg == b"no" {
            off = true;
        }
        if !(on || off) {
            send_to_char(g, chid, b"Value must be 'on' or 'off'.\r\n");
            return false;
        }
    } else if field.type_ == NUMBER {
        value = atoi(val_arg);
    }

    // RANGE(lo, hi).
    macro_rules! range {
        ($lo:expr, $hi:expr) => {{
            value = value.max($lo).min($hi);
            value
        }};
    }
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();

    match mode {
        0 => {
            g.ch_mut(vict).points.armor = range!(-100, 100);
            crate::handler::affect_total(g, vict);
        }
        1 => set_or_remove(g, vict, false, flags::PRF_AFK, on, off),
        2 => {
            if value < 2 || value > 200 {
                send_to_char(g, chid, b"Ages 2 to 200 accepted.\r\n");
                return false;
            }
            let birth = g.now - ((value - 17) as i64 * SECS_PER_MUD_YEAR as i64);
            g.ch_mut(vict).time.birth = birth;
        }
        3 => {
            g.ch_mut(vict).alignment = range!(-1000, 1000);
            crate::handler::affect_total(g, vict);
        }
        4 => g.ch_mut(vict).points.bank_gold = range!(0, 100_000_000),
        5 => set_or_remove(g, vict, false, flags::PRF_BRIEF, on, off),
        6 | 9 | 12 | 21 | 48 | 55 => {
            if is_npc || g.ch(vict).level >= LVL_GRGOD {
                range!(3, 25);
            } else {
                range!(3, 18);
            }
            let a = &mut g.ch_mut(vict).real_abils;
            match mode {
                6 => a.cha = value as i8,
                9 => a.con = value as i8,
                12 => a.dex = value as i8,
                21 => a.intel = value as i8,
                55 => a.wis = value as i8,
                _ => {
                    a.str_ = value as i8;
                    a.str_add = 0;
                }
            }
            crate::handler::affect_total(g, vict);
        }
        7 => {
            let i = parse_class(val_arg.first().copied().unwrap_or(0));
            if i == CLASS_UNDEFINED {
                send_to_char(g, chid, b"That is not a class.\r\n");
                return false;
            }
            g.ch_mut(vict).class = i;
        }
        8 => {
            set_or_remove(g, vict, false, flags::PRF_COLOR_1, on, off);
            set_or_remove(g, vict, false, flags::PRF_COLOR_2, on, off);
        }
        10 => {
            g.ch_mut(vict).points.damroll = range!(-20, 20) as i8;
            crate::handler::affect_total(g, vict);
        }
        11 => set_or_remove(g, vict, true, flags::PLR_DELETED, on, off),
        13 | 20 | 51 => {
            let (idx, label): (usize, &[u8]) = match mode {
                13 => (crate::ch::DRUNK, b"drunkenness"),
                20 => (crate::ch::HUNGER, b"hunger"),
                _ => (crate::ch::THIRST, b"thirst"),
            };
            if val_arg.eq_ignore_ascii_case(b"off") {
                g.ch_mut(vict).ps_mut().conditions[idx] = -1;
                let mut out = vname.clone().into_bytes();
                out.extend_from_slice(b"'s ");
                out.extend_from_slice(label);
                out.extend_from_slice(b" is now off.\r\n");
                send_to_char(g, chid, &out);
            } else if is_number(val_arg) {
                value = atoi(val_arg);
                range!(0, 24);
                g.ch_mut(vict).ps_mut().conditions[idx] = value as i16;
                let mut out = vname.clone().into_bytes();
                out.extend_from_slice(b"'s ");
                out.extend_from_slice(label);
                out.extend_from_slice(format!(" set to {}.\r\n", value).as_bytes());
                send_to_char(g, chid, &out);
            } else {
                send_to_char(g, chid, b"Must be 'off' or a value from 0 to 24.\r\n");
                return false;
            }
        }
        14 => g.ch_mut(vict).points.exp = range!(0, 50_000_000),
        15 => {
            if chid == vict && on {
                send_to_char(g, chid, b"Better not -- could be a long winter!\r\n");
                return false;
            }
            set_or_remove(g, vict, true, flags::PLR_FROZEN, on, off);
        }
        16 => g.ch_mut(vict).points.gold = range!(0, 100_000_000),
        17 => {
            g.ch_mut(vict).height = value.clamp(0, 255) as u8;
            crate::handler::affect_total(g, vict);
        }
        18 => {
            let max = g.ch(vict).points.max_hit;
            g.ch_mut(vict).points.hit = range!(-9, max);
            crate::handler::affect_total(g, vict);
        }
        19 => {
            g.ch_mut(vict).points.hitroll = range!(-20, 20) as i8;
            crate::handler::affect_total(g, vict);
        }
        22 => {
            if g.ch(chid).level < LVL_IMPL && chid != vict {
                send_to_char(g, chid, b"You aren't godly enough for that!\r\n");
                return false;
            }
            let lvl = g.ch(vict).level as i32;
            g.ch_mut(vict).ps_mut().invis_level = range!(0, lvl) as i16;
        }
        23 => set_or_remove(g, vict, true, flags::PLR_INVSTART, on, off),
        24 => set_or_remove(g, vict, true, flags::PLR_KILLER, on, off),
        25 => {
            if (!is_npc && value > g.ch(chid).level as i32) || value > LVL_IMPL as i32 {
                send_to_char(g, chid, b"You can't do that.\r\n");
                return false;
            }
            range!(1, LVL_IMPL as i32);
            g.ch_mut(vict).level = value as u8;
        }
        26 => {
            if val_arg.eq_ignore_ascii_case(b"off") {
                g.ch_mut(vict).act.remove(flags::PLR_LOADROOM);
            } else if is_number(val_arg) {
                let rvnum = atoi(val_arg);
                if g.real_room(rvnum).is_some() {
                    g.ch_mut(vict).act.set(flags::PLR_LOADROOM);
                    g.ch_mut(vict).ps_mut().load_room = rvnum as Idx;
                    send_to_char(
                        g,
                        chid,
                        format!("{} will enter at room #{}.\r\n", vname, rvnum).as_bytes(),
                    );
                } else {
                    send_to_char(g, chid, b"That room does not exist!\r\n");
                    return false;
                }
            } else {
                send_to_char(g, chid, b"Must be 'off' or a room's virtual number.\r\n");
                return false;
            }
        }
        27 => {
            let max = g.ch(vict).points.max_mana;
            g.ch_mut(vict).points.mana = range!(0, max);
            crate::handler::affect_total(g, vict);
        }
        28 => {
            g.ch_mut(vict).points.max_hit = range!(1, 5000);
            crate::handler::affect_total(g, vict);
        }
        29 => {
            g.ch_mut(vict).points.max_mana = range!(1, 5000);
            crate::handler::affect_total(g, vict);
        }
        30 => {
            g.ch_mut(vict).points.max_move = range!(1, 5000);
            crate::handler::affect_total(g, vict);
        }
        31 => {
            let max = g.ch(vict).points.max_move;
            g.ch_mut(vict).points.mov = range!(0, max);
            crate::handler::affect_total(g, vict);
        }
        32 => {
            if chid != vict && g.ch(chid).level < LVL_IMPL {
                send_to_char(g, chid, b"Only Imps can change the name of other players.\r\n");
                return false;
            }
            if !change_player_name(g, chid, vict, val_arg) {
                send_to_char(g, chid, b"Name has not been changed!\r\n");
                return false;
            }
            // The wizlist and the immlist are both built from player
            // names, so a rename leaves whichever list the player is on
            // showing the old name until something unrelated regenerates it.
            // Called unguarded on purpose: autowiz already decides who
            // belongs on each list -- level, and the NOWIZLIST flag -- and a
            // second copy of that rule here would be free to drift from it,
            // which is how the export writers ended up wrong.
            crate::limits::run_autowiz(g);
        }
        33 => set_or_remove(g, vict, true, flags::PLR_NODELETE, on, off),
        34 => {
            if g.ch(chid).level < LVL_GOD && chid != vict {
                send_to_char(g, chid, b"You aren't godly enough for that!\r\n");
                return false;
            }
            set_or_remove(g, vict, false, flags::PRF_NOHASSLE, on, off);
        }
        35 => {
            set_or_remove(g, vict, false, flags::PRF_SUMMONABLE, on, off);
            send_to_char(
                g,
                chid,
                format!("Nosummon {} for {}.\r\n", if !on { "ON" } else { "OFF" }, vname).as_bytes(),
            );
        }
        36 => set_or_remove(g, vict, true, flags::PLR_NOWIZLIST, on, off),
        37 => {
            let zone = if is_abbrev(val_arg, b"socials")
                || is_abbrev(val_arg, b"actions")
                || is_abbrev(val_arg, b"aedit")
            {
                AEDIT_PERMISSION
            } else if is_abbrev(val_arg, b"hedit") || is_abbrev(val_arg, b"help") {
                HEDIT_PERMISSION
            } else if val_arg.first() == Some(&b'*') || is_abbrev(val_arg, b"all") {
                ALL_PERMISSION
            } else if is_abbrev(val_arg, b"off") {
                NOWHERE as i32
            } else if !is_number(val_arg) {
                send_to_char(
                    g,
                    chid,
                    b"Value must be a zone number, 'aedit', 'hedit', 'off' or 'all'.\r\n",
                );
                return false;
            } else {
                atoi(val_arg)
            };
            g.ch_mut(vict).ps_mut().olc_zone = zone;
        }
        38 => {
            if g.ch(vict).level >= LVL_GRGOD {
                send_to_char(g, chid, b"You cannot change that.\r\n");
                return false;
            }
            let name = g.ch(vict).name.clone().unwrap_or_default();
            let hash = mud_data::crypt::crypt(val_arg, &name).map(|h| h.to_vec()).unwrap_or_default();
            let mut hash = hash;
            hash.truncate(MAX_PWD_LENGTH);
            g.ch_mut(vict).passwd = hash;
            let mut out = b"Password changed to '".to_vec();
            out.extend_from_slice(val_arg);
            out.extend_from_slice(b"'.\r\n");
            send_to_char(g, chid, &out);
        }
        39 | 40 => {
            if vict == chid || g.ch(chid).level == LVL_IMPL {
                let mut v = crate::interpreter::skip_spaces(val_arg).to_vec();
                crate::text::parse_at(&mut v);
                let value = if v.is_empty() { None } else { Some(v) };
                if mode == 39 {
                    g.ch_mut(vict).ps_mut().poofin = value;
                } else {
                    g.ch_mut(vict).ps_mut().poofout = value;
                }
            }
        }
        41 => g.ch_mut(vict).ps_mut().practices = range!(0, 100),
        42 => set_or_remove(g, vict, false, flags::PRF_QUEST, on, off),
        43 => {
            let Some(rnum) = g.real_room(value) else {
                send_to_char(g, chid, b"No room exists with that number.\r\n");
                return false;
            };
            if g.ch(vict).in_room != NOWHERE {
                crate::handler::char_from_room(g, vict);
            }
            crate::handler::char_to_room(g, vict, rnum);
        }
        44 => g.ch_mut(vict).ps_mut().screen_width = range!(40, 200),
        45 => {
            let Some(i) = crate::act::informative::search_block(val_arg, &mud_data::tables::GENDERS)
            else {
                send_to_char(g, chid, b"Must be 'male', 'female', or 'neutral'.\r\n");
                return false;
            };
            g.ch_mut(vict).sex = i as u8;
        }
        46 => set_or_remove(g, vict, false, flags::PRF_SHOWVNUMS, on, off),
        47 => set_or_remove(g, vict, true, flags::PLR_SITEOK, on, off),
        49 => {
            g.ch_mut(vict).real_abils.str_add = range!(0, 100) as i8;
            if value > 0 {
                g.ch_mut(vict).real_abils.str_ = 18;
            }
            crate::handler::affect_total(g, vict);
        }
        50 => set_or_remove(g, vict, true, flags::PLR_THIEF, on, off),
        52 => {
            crate::login::set_title(g, vict, Some(val_arg.to_vec()));
            let title = g.ch(vict).title.clone().unwrap_or_default();
            let mut out = vname.clone().into_bytes();
            out.extend_from_slice(b"'s title is now: ");
            out.extend_from_slice(&title);
            out.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &out);
        }
        53 => return crate::dg::commands::perform_set_dg_var(g, chid, vict, val_arg),
        54 => {
            g.ch_mut(vict).weight = value.clamp(0, 255) as u8;
            crate::handler::affect_total(g, vict);
        }
        56 => g.ch_mut(vict).ps_mut().questpoints = range!(0, 100_000_000),
        57 => {
            let qvnum = atoi(val_arg);
            if crate::quest::real_quest(g, qvnum).is_none() {
                send_to_char(g, chid, b"That quest doesn't exist.\r\n");
                return false;
            }
            if crate::quest::is_complete(g, vict, qvnum) {
                crate::quest::remove_completed_quest(g, vict, qvnum);
                send_to_char(
                    g,
                    chid,
                    format!("Quest {} removed from history for player {}.\r\n", qvnum, vname)
                        .as_bytes(),
                );
            } else {
                crate::quest::add_completed_quest_pub(g, vict, qvnum);
                send_to_char(
                    g,
                    chid,
                    format!("Quest {} added to history for player {}.\r\n", qvnum, vname).as_bytes(),
                );
            }
        }
        _ => {
            send_to_char(g, chid, b"Can't set that!\r\n");
            return false;
        }
    }

    if field.type_ == BINARY {
        let mut out = field.cmd.to_vec();
        out.extend_from_slice(if on { b" ON for " } else { b" OFF for " });
        out.extend_from_slice(vname.as_bytes());
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
    } else if field.type_ == NUMBER {
        let mut out = vname.clone().into_bytes();
        out.extend_from_slice(b"'s ");
        out.extend_from_slice(field.cmd);
        out.extend_from_slice(format!(" set to {}.\r\n", value).as_bytes());
        send_to_char(g, chid, &out);
    } else {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    }
    true
}

/// parse_class, by first letter.
fn parse_class(c: u8) -> i8 {
    match c.to_ascii_lowercase() {
        b'm' => CLASS_MAGIC_USER,
        b'c' => CLASS_CLERIC,
        b'w' => CLASS_WARRIOR,
        b't' => CLASS_THIEF,
        _ => CLASS_UNDEFINED,
    }
}

fn show_set_help(g: &mut Game, chid: CharId) {
    const SET_LEVELS: [&str; 4] = ["Imm", "God", "GrGod", "IMP"];
    const SET_TARGETS: [&str; 3] = ["PC", "NPC", "BOTH"];
    const SET_TYPES: [&str; 3] = ["MISC", "BINARY", "NUMBER"];
    let cyn = cc(g, chid, C_NRM, crate::comm::KCYN).to_vec();
    let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
    let level = g.ch(chid).level;

    let mut buf = cyn;
    buf.extend_from_slice(b"Command             Lvl    Who?  Type");
    buf.extend_from_slice(&nrm);
    buf.extend_from_slice(b"\r\n");
    for fld in SET_FIELDS.iter() {
        if fld.level > level {
            continue;
        }
        buf.extend_from_slice(&pad_right(fld.cmd, 20));
        let lvl_name = SET_LEVELS
            .get((fld.level as i32 - LVL_IMMORT as i32).max(0) as usize)
            .copied()
            .unwrap_or("");
        buf.extend_from_slice(&pad_right(lvl_name.as_bytes(), 5));
        buf.extend_from_slice(b"  ");
        buf.extend_from_slice(&pad_right(
            SET_TARGETS[(fld.pcnpc - 1) as usize].as_bytes(),
            4,
        ));
        buf.extend_from_slice(b"  ");
        buf.extend_from_slice(&pad_right(SET_TYPES[fld.type_ as usize].as_bytes(), 6));
        buf.extend_from_slice(b"\r\n");
    }
    crate::act::informative::page_string(g, chid, &buf);
}

pub fn do_set(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (mut name, mut buf) = half_chop(argument);
    let (mut is_file, mut is_player) = (false, false);

    if name == b"file" {
        is_file = true;
        let (n, b) = half_chop(&buf);
        name = n;
        buf = b;
    } else if name.eq_ignore_ascii_case(b"help") {
        show_set_help(g, chid);
        return;
    } else if name.eq_ignore_ascii_case(b"player") {
        is_player = true;
        let (n, b) = half_chop(&buf);
        name = n;
        buf = b;
    } else if name.eq_ignore_ascii_case(b"mob") {
        let (n, b) = half_chop(&buf);
        name = n;
        buf = b;
    }
    let (field, buf) = half_chop(&buf);

    if name.is_empty() || field.is_empty() {
        send_to_char(g, chid, b"Usage: set <victim> <field> <value>\r\n");
        let yel = cc(g, chid, C_NRM, KYEL).to_vec();
        let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
        let mut out = b"       ".to_vec();
        out.extend_from_slice(&yel);
        out.extend_from_slice(b"set help");
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b" will display valid fields\r\n");
        send_to_char(g, chid, &out);
        return;
    }

    let mut cbuf: Option<CharId> = None;
    let vict = if !is_file {
        let found = if is_player {
            crate::handler::get_player_vis(g, chid, &name, false)
        } else {
            crate::handler::get_char_world_vis(g, chid, &name, None)
        };
        match found {
            Some(v) => v,
            None => {
                if is_player {
                    send_to_char(g, chid, b"There is no such player.\r\n");
                } else {
                    send_to_char(g, chid, b"There is no such creature.\r\n");
                }
                return;
            }
        }
    } else {
        match crate::players_glue::load_char_offline(g, &name) {
            Some(v) => {
                if g.ch(v).level > g.ch(chid).level {
                    crate::players_glue::free_offline_char(g, v);
                    send_to_char(g, chid, b"Sorry, you can't do that.\r\n");
                    return;
                }
                cbuf = Some(v);
                v
            }
            None => {
                send_to_char(g, chid, b"There is no such player.\r\n");
                return;
            }
        }
    };

    let mode = SET_FIELDS.iter().position(|f| f.cmd.starts_with(&field[..]));
    let retval = match mode {
        None => {
            send_to_char(g, chid, b"Can't set that!\r\n");
            false
        }
        Some(m) => perform_set(g, chid, vict, m, &buf),
    };

    if retval {
        if !is_file && !g.ch(vict).is_npc() {
            crate::players_glue::save_char(g, vict);
        }
        if is_file {
            // GET_PFILEPOS(cbuf) = player_i — the scratch
            // character is only writable once it is told which slot it is.
            let name = g.ch(vict).name.clone().unwrap_or_default().to_ascii_lowercase();
            if let Some(i) = g.player_table.iter().position(|p| p.name == name) {
                g.ch_mut(vict).pfilepos = i as i32;
            }
            crate::players_glue::save_char(g, vict);
            send_to_char(g, chid, b"Saved in file.\r\n");
        }
    }
    if let Some(c) = cbuf {
        crate::players_glue::free_offline_char(g, c);
    }
}

pub fn change_player_name(g: &mut Game, chid: CharId, vict: CharId, new_name: &[u8]) -> bool {
    if new_name.len() < 2
        || new_name.len() > MAX_NAME_LENGTH
        || !crate::login::valid_name(g, new_name)
        || crate::interpreter::fill_word(&new_name.to_ascii_lowercase())
        || crate::interpreter::reserved_word(&new_name.to_ascii_lowercase())
    {
        send_to_char(g, chid, b"Invalid new name.\r\n");
        return false;
    }
    if crate::handler::get_player_vis(g, chid, new_name, false).is_some() {
        send_to_char(g, chid, b"Sorry, the new name already exists.\r\n");
        return false;
    }
    if let Some(tmp) = crate::players_glue::load_char_offline(g, new_name) {
        crate::players_glue::free_offline_char(g, tmp);
        send_to_char(g, chid, b"Sorry, the new name already exists.\r\n");
        return false;
    }

    let idnum = g.ch(vict).idnum;
    let Some(i) = g.player_table.iter().position(|p| p.id == idnum) else {
        send_to_char(g, chid, b"Your target was not found in the player index.\r\n");
        let name = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
        g.log(format!(
            "SYSERR: Player {}, with ID {}, could not be found in the player index.",
            name, idnum
        ));
        return false;
    };

    let old_name = g.ch(vict).get_name().to_vec();
    use mud_world::players::{get_filename, FileKind};
    if get_filename(FileKind::Plr, &old_name).is_none() {
        send_to_char(g, chid, b"Unable to ascertain player's old pfile name.\r\n");
        return false;
    }
    if get_filename(FileKind::Plr, new_name).is_none() {
        send_to_char(g, chid, b"Unable to ascertain player's new pfile name.\r\n");
        return false;
    }

    g.player_table[i].name = new_name.to_ascii_lowercase();
    let mut capped = new_name.to_vec();
    if let Some(c) = capped.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    g.ch_mut(vict).name = Some(capped);

    // Building an `mv` command string for the pfile and never running it
    // leaves the pfile rewritten under the new name by the
    // save_char that perform_set does next, and the old one is orphaned —
    // along with the player's object, text and variable files, which the
    // renamed character can then never load. Renaming is the evident intent,
    // so do it for all four.
    for kind in [FileKind::Plr, FileKind::Objs, FileKind::Text, FileKind::Vars] {
        let (Some(from), Some(to)) =
            (get_filename(kind, &old_name), get_filename(kind, new_name))
        else {
            continue;
        };
        let from = g.lib_dir.join(from);
        if from.exists() {
            let _ = std::fs::rename(&from, g.lib_dir.join(to));
        }
    }

    crate::players_glue::save_player_index(g);
    let gname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    g.mudlog(
        MudlogKind::Brf,
        LVL_IMMORT,
        true,
        &format!(
            "(GC) {} changed the name of {} to {}",
            gname,
            String::from_utf8_lossy(&old_name),
            String::from_utf8_lossy(new_name)
        ),
    );

    if g.ch(vict).desc.is_some() {
        let yel = cc(g, vict, C_NRM, KYEL).to_vec();
        let nrm = cc(g, vict, C_NRM, KNRM).to_vec();
        let mut out = b"Your login name has changed from ".to_vec();
        out.extend_from_slice(&yel);
        out.extend_from_slice(&old_name);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b" to ");
        out.extend_from_slice(&yel);
        out.extend_from_slice(new_name);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, vict, &out);
    }
    true
}

#[allow(unused)]
fn _unused(a: &[u8]) -> BStr {
    one_argument(a).0
}
