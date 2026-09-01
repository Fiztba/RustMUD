//! `show` plus the two listings it borrows:
//! `print_zone` and the shop tables.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{ITEM_TYPES, ZONE_BITS};
use mud_data::types::*;

use crate::act::informative::sprintbitarray;
use crate::act::wizstat::sprinttype;
use crate::act::{pad_right, BStr};
use crate::comm::{cc, send_to_char, C_SPR, KCYN, KGRN, KNRM};
use crate::game::Game;
use crate::handler::{atoi, can_see};
use crate::interpreter::{is_number, two_arguments};
use crate::quest::sprintbit;

pub const TRADE_LETTERS: [&str; 7] =
    ["Good", "Evil", "Neutral", "Magic User", "Cleric", "Thief", "Warrior"];
pub const SHOP_BITS: [&str; 3] = ["WILL_FIGHT", "USES_BANK", "UNLIMITED_CASH"];

struct ShowField {
    cmd: &'static [u8],
    level: u8,
}

const fn s(cmd: &'static [u8], level: u8) -> ShowField {
    ShowField { cmd, level }
}

/// The `fields[]` table; index 0 is the "nothing"
/// row, whose level 0 terminates the usage listing.
const SHOW_FIELDS: [ShowField; 14] = [
    s(b"nothing", 0),
    s(b"zones", LVL_IMMORT),
    s(b"player", LVL_IMMORT),
    s(b"rent", LVL_IMMORT),
    s(b"stats", LVL_IMMORT),
    s(b"errors", LVL_IMMORT),
    s(b"death", LVL_IMMORT),
    s(b"godrooms", LVL_IMMORT),
    s(b"shops", LVL_IMMORT),
    s(b"houses", LVL_IMMORT),
    s(b"snoop", LVL_IMMORT),
    s(b"thaco", LVL_IMMORT),
    s(b"exp", LVL_IMMORT),
    s(b"colour", LVL_IMMORT),
];

/// The append-and-check every buffer-building branch of do_show uses. A row
/// that does not fit is truncated and the fragment is *kept*, then the loop
/// stops, so the last line of an over-long listing is cut mid-word. Returns
/// false when the caller should break.
fn push_capped(out: &mut BStr, row: &[u8]) -> bool {
    let room = MAX_STRING_LENGTH.saturating_sub(out.len() + 1);
    if row.len() >= room {
        out.extend_from_slice(&row[..room]);
        return false;
    }
    out.extend_from_slice(row);
    true
}

fn print_zone_to_buf(g: &Game, out: &mut BStr, zone: usize, listall: bool) {
    let z = &g.world.zones[zone];
    let nrm = crate::comm::KNRM;
    if !listall {
        let name = z.name.as_deref().unwrap_or(b"");
        let width = crate::act::other::count_color_chars(name) + 30;
        out.extend_from_slice(format!("{:3} ", z.number).as_bytes());
        out.extend_from_slice(&pad_right(name, width));
        out.extend_from_slice(nrm);
        out.extend_from_slice(b" By: ");
        out.extend_from_slice(&crate::act::pad_right_trunc(
            z.builders.as_deref().unwrap_or(b""),
            10,
        ));
        out.extend_from_slice(nrm);
        out.extend_from_slice(format!(" Range: {:5}-{:5}\r\n", z.bot, z.top).as_bytes());
        return;
    }

    let mut flags_buf = Vec::new();
    sprintbitarray(&z.zone_flags, &ZONE_BITS, &mut flags_buf);
    let reset = match z.reset_mode {
        0 => &b"Never reset"[..],
        1 => b"Reset when no players are in zone",
        _ => b"Normal reset",
    };
    out.extend_from_slice(format!("{:3} ", z.number).as_bytes());
    out.extend_from_slice(&crate::act::pad_right_trunc(z.name.as_deref().unwrap_or(b""), 30));
    out.extend_from_slice(nrm);
    out.extend_from_slice(b" By: ");
    out.extend_from_slice(&crate::act::pad_right_trunc(z.builders.as_deref().unwrap_or(b""), 10));
    out.extend_from_slice(nrm);
    out.extend_from_slice(format!(" Age: {:3}; Reset: {:3} (", z.age_of(g, zone), z.lifespan).as_bytes());
    out.extend_from_slice(reset);
    out.extend_from_slice(format!("); Range: {:5}-{:5}\r\n", z.bot, z.top).as_bytes());

    let (bot, top) = (z.bot as i32, z.top as i32);
    let j = g
        .world
        .rooms
        .iter()
        .filter(|r| (r.vnum as i32) >= bot && (r.vnum as i32) <= top)
        .count();
    let k = g
        .world
        .obj_protos
        .iter()
        .filter(|p| (p.vnum as i32) >= bot && (p.vnum as i32) <= top)
        .count();
    let l = g
        .world
        .mob_protos
        .iter()
        .filter(|p| (p.vnum as i32) >= bot && (p.vnum as i32) <= top)
        .count();
    let m = g.world.shops.iter().filter(|sh| (sh.vnum as i32) >= bot && (sh.vnum as i32) <= top).count();
    let n = g
        .world
        .triggers
        .iter()
        .filter(|t| (t.vnum as i32) >= bot && (t.vnum as i32) <= top)
        .count();
    let o = crate::quest::count_quests(g, bot, top);

    out.extend_from_slice(b"       Zone stats:\r\n       ---------------\r\n         Flags:    ");
    out.extend_from_slice(&flags_buf);
    out.extend_from_slice(
        format!(
            "\r\n         Min Lev:  {:2}\r\n         Max Lev:  {:2}\r\n         Rooms:    {:2}\r\n         Objects:  {:2}\r\n         Mobiles:  {:2}\r\n         Shops:    {:2}\r\n         Triggers: {:2}\r\n         Quests:   {:2}\r\n",
            z.min_level, z.max_level, j, k, l, m, n, o
        )
        .as_bytes(),
    );
}

trait ZoneAge {
    fn age_of(&self, g: &Game, rnum: usize) -> i32;
}
impl ZoneAge for mud_world::model::Zone {
    fn age_of(&self, g: &Game, rnum: usize) -> i32 {
        g.zones_rt[rnum].age
    }
}

pub fn print_zone(g: &mut Game, chid: CharId, vnum: i32) {
    let Some(rnum) = g.world.zones.iter().position(|z| z.number as i32 == vnum) else {
        send_to_char(
            g,
            chid,
            format!("Zone #{} does not exist in the database.\r\n", vnum).as_bytes(),
        );
        return;
    };
    let (grn, cyn, nrm) = (
        cc(g, chid, C_SPR, KGRN).to_vec(),
        cc(g, chid, C_SPR, KCYN).to_vec(),
        cc(g, chid, C_SPR, KNRM).to_vec(),
    );
    let z = g.world.zones[rnum].clone();
    let (bot, top) = (z.bot as i32, z.top as i32);
    let size_rooms = g.world.rooms.iter().filter(|r| r.zone as usize == rnum).count();
    let size_objects =
        g.world.obj_protos.iter().filter(|p| (p.vnum as i32) >= bot && (p.vnum as i32) <= top).count();
    let size_mobiles =
        g.world.mob_protos.iter().filter(|p| (p.vnum as i32) >= bot && (p.vnum as i32) <= top).count();
    let size_shops =
        g.world.shops.iter().filter(|sh| (sh.vnum as i32) >= bot && (sh.vnum as i32) <= top).count();
    let size_trigs = g
        .world
        .triggers
        .iter()
        .filter(|t| (t.vnum as i32) >= bot && (t.vnum as i32) <= top)
        .count();
    let size_quests = crate::quest::count_quests(g, bot, top);
    let mut flags_buf = Vec::new();
    sprintbitarray(&z.zone_flags, &ZONE_BITS, &mut flags_buf);
    let reset: &[u8] = match z.reset_mode {
        0 => b"Never reset",
        1 => b"Reset when no players are in zone.",
        _ => b"Normal reset.",
    };

    let mut out: BStr = Vec::new();
    let row = |label: &[u8], value: &[u8], out: &mut BStr| {
        out.extend_from_slice(&grn);
        out.extend_from_slice(label);
        out.extend_from_slice(&cyn);
        out.extend_from_slice(value);
        out.extend_from_slice(b"\r\n");
    };
    row(b"Virtual Number = ", z.number.to_string().as_bytes(), &mut out);
    row(b"Name of zone   = ", z.name.as_deref().unwrap_or(b""), &mut out);
    row(b"Builders       = ", z.builders.as_deref().unwrap_or(b""), &mut out);
    row(b"Lifespan       = ", z.lifespan.to_string().as_bytes(), &mut out);
    row(b"Age            = ", g.zones_rt[rnum].age.to_string().as_bytes(), &mut out);
    row(b"Bottom of Zone = ", z.bot.to_string().as_bytes(), &mut out);
    row(b"Top of Zone    = ", z.top.to_string().as_bytes(), &mut out);
    row(b"Reset Mode     = ", reset, &mut out);
    row(b"Zone Flags     = ", &flags_buf, &mut out);
    row(b"Min Level      = ", z.min_level.to_string().as_bytes(), &mut out);
    row(b"Max Level      = ", z.max_level.to_string().as_bytes(), &mut out);
    out.extend_from_slice(&grn);
    out.extend_from_slice(b"Size\r\n");
    row(b"   Rooms       = ", size_rooms.to_string().as_bytes(), &mut out);
    row(b"   Objects     = ", size_objects.to_string().as_bytes(), &mut out);
    row(b"   Mobiles     = ", size_mobiles.to_string().as_bytes(), &mut out);
    row(b"   Shops       = ", size_shops.to_string().as_bytes(), &mut out);
    row(b"   Triggers    = ", size_trigs.to_string().as_bytes(), &mut out);
    // The final row carries the reset back to normal.
    out.extend_from_slice(&grn);
    out.extend_from_slice(b"   Quests      = ");
    out.extend_from_slice(&cyn);
    out.extend_from_slice(size_quests.to_string().as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
}

/// customer_string, compact form.
fn customer_string(g: &Game, shop_nr: usize) -> BStr {
    let with_who = g.world.shops[shop_nr].with_who;
    let mut out = Vec::new();
    for (sindex, letter) in TRADE_LETTERS.iter().enumerate() {
        if with_who & (1 << sindex) != 0 {
            out.push(b'_');
        } else {
            out.push(letter.as_bytes()[0]);
        }
    }
    out
}

fn list_all_shops(g: &mut Game, chid: CharId) {
    const HEADER: &[u8] = b" ##   Virtual   Where    Keeper    Buy   Sell   Customers\r\n\
---------------------------------------------------------\r\n";
    let page_length = if g.ch(chid).is_npc() { 22 } else { g.ch(chid).ps().page_length };
    let per_page = (page_length - 2).max(1) as usize;
    let mut buf: BStr = Vec::new();
    for shop_nr in 0..g.world.shops.len() {
        if shop_nr % per_page == 0 {
            buf.extend_from_slice(HEADER);
        }
        let keeper = g.shops_rt[shop_nr].keeper;
        let buf1: BStr = if keeper == NOBODY {
            b"<NONE>".to_vec()
        } else {
            format!("{:6}", g.world.mob_protos[keeper as usize].vnum).into_bytes()
        };
        let sh = &g.world.shops[shop_nr];
        let where_ = sh.in_rooms.first().copied().unwrap_or(NOWHERE as i32);
        buf.extend_from_slice(
            format!("{:3}   {:6}   {:6}    ", shop_nr + 1, sh.vnum, where_).as_bytes(),
        );
        buf.extend_from_slice(&buf1);
        buf.extend_from_slice(
            format!("   {:3.2}   {:3.2}    ", sh.profit_sell, sh.profit_buy).as_bytes(),
        );
        buf.extend_from_slice(&customer_string(g, shop_nr));
        buf.extend_from_slice(b"\r\n");
    }
    crate::act::informative::page_string(g, chid, &buf);
}

/// The word-wrapping column helper the detailed shop listing uses.
fn wrap_push(out: &mut BStr, column: &mut usize, chunk: &[u8]) {
    if chunk.len() + *column >= 78 && *column >= 20 {
        out.extend_from_slice(b"\r\n            ");
        *column = 12;
    }
    out.extend_from_slice(chunk);
    *column += chunk.len();
}

fn list_detailed_shop(g: &mut Game, chid: CharId, shop_nr: usize) {
    let sh = g.world.shops[shop_nr].clone();
    let mut out =
        format!("Vnum:       [{:5}], Rnum: [{:5}]\r\n", sh.vnum, shop_nr + 1).into_bytes();

    out.extend_from_slice(b"Rooms:      ");
    let mut column = 12;
    for (sindex, &rv) in sh.in_rooms.iter().enumerate() {
        if sindex > 0 {
            out.extend_from_slice(b", ");
            column += 2;
        }
        let chunk = match g.real_room(rv) {
            Some(temp) => {
                let mut c = g.world.rooms[temp as usize].name.clone().unwrap_or_default();
                c.extend_from_slice(format!(" (#{})", g.world.rooms[temp as usize].vnum).as_bytes());
                c
            }
            None => format!("<UNKNOWN> (#{})", rv).into_bytes(),
        };
        wrap_push(&mut out, &mut column, &chunk);
    }
    if sh.in_rooms.is_empty() {
        // The "Rooms:      " label went out before the loop, so the
        // empty case adds only the word.
        out.extend_from_slice(b"None!");
    }

    out.extend_from_slice(b"\r\nShopkeeper: ");
    let keeper = g.shops_rt[shop_nr].keeper;
    if keeper != NOBODY {
        let p = &g.world.mob_protos[keeper as usize];
        out.extend_from_slice(p.short_descr.as_deref().unwrap_or(b""));
        out.extend_from_slice(
            format!(
                " (#{}), Special Function: {}\r\n",
                p.vnum,
                if g.shops_rt[shop_nr].func.is_some() { "YES" } else { "NO" }
            )
            .as_bytes(),
        );
        if let Some(k) = get_char_num(g, keeper) {
            let gold = g.ch(k).points.gold;
            let bank = g.shops_rt[shop_nr].bank;
            out.extend_from_slice(
                format!(
                    "Coins:      [{:9}], Bank: [{:9}] (Total: {})\r\n",
                    gold,
                    bank,
                    gold + bank
                )
                .as_bytes(),
            );
        }
    } else {
        out.extend_from_slice(b"<NONE>\r\n");
    }

    out.extend_from_slice(b"Customers:  ");
    let mut column = 12;
    let mut found = false;
    for (sindex, letter) in TRADE_LETTERS.iter().enumerate() {
        if sh.with_who & (1 << sindex) == 0 {
            // Ask whether anything was printed, not how far the scan
            // has got -- excluding the first group used to open with ", ".
            if found {
                out.extend_from_slice(b", ");
                column += 2;
            }
            wrap_push(&mut out, &mut column, letter.as_bytes());
            found = true;
        }
    }
    out.extend_from_slice(if found { &b""[..] } else { b"Nobody!" });
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"Produces:   ");
    let mut column = 12;
    let producing = g.shops_rt[shop_nr].producing.clone();
    for (sindex, &rnum) in producing.iter().enumerate() {
        if sindex > 0 {
            out.extend_from_slice(b", ");
            column += 2;
        }
        let p = &g.world.obj_protos[rnum as usize];
        let mut chunk = p.short_description.clone().unwrap_or_default();
        chunk.extend_from_slice(format!(" (#{})", p.vnum).as_bytes());
        wrap_push(&mut out, &mut column, &chunk);
    }
    if producing.is_empty() {
        // B93, as above.
        out.extend_from_slice(b"Nothing!");
    }

    out.extend_from_slice(b"\r\nBuys:       ");
    let mut column = 12;
    for (sindex, bd) in sh.type_list.iter().enumerate() {
        if sindex > 0 {
            out.extend_from_slice(b", ");
            column += 2;
        }
        let mut chunk =
            ITEM_TYPES.get(bd.type_ as usize).copied().unwrap_or("UNDEFINED").as_bytes().to_vec();
        chunk.extend_from_slice(format!(" (#{}) [", bd.type_).as_bytes());
        chunk.extend_from_slice(bd.keywords.as_deref().unwrap_or(b"all"));
        chunk.push(b']');
        wrap_push(&mut out, &mut column, &chunk);
    }
    if sh.type_list.is_empty() {
        // B93, as above.
        out.extend_from_slice(b"Nothing!");
    }

    out.extend_from_slice(
        format!(
            "\r\nBuy at:     [{:4.2}], Sell at: [{:4.2}], Open: [{}-{}, {}-{}]\r\n",
            sh.profit_sell, sh.profit_buy, sh.open1, sh.close1, sh.open2, sh.close2
        )
        .as_bytes(),
    );
    let bits = sprintbit(sh.bitvector as i64, &SHOP_BITS);
    out.extend_from_slice(b"Bits:       ");
    out.extend_from_slice(&bits);
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
}

/// get_char_num: the first live mob with this prototype rnum.
fn get_char_num(g: &Game, rnum: Idx) -> Option<CharId> {
    g.character_list
        .iter()
        .copied()
        .find(|&c| g.try_ch(c).is_some_and(|ch| ch.mob_rnum == rnum))
}

fn show_shops(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        list_all_shops(g, chid);
        return;
    }
    let shop_nr: i64 = if arg == b"." {
        let room_vnum = g.world.rooms[g.ch(chid).in_room as usize].vnum as i32;
        match (0..g.world.shops.len()).find(|&i| crate::shop::ok_shop_room(g, i, room_vnum)) {
            Some(i) => i as i64,
            None => {
                send_to_char(g, chid, b"This isn't a shop!\r\n");
                return;
            }
        }
    } else if is_number(arg) {
        match g.world.shops.iter().position(|sh| sh.vnum as i32 == atoi(arg)) {
            Some(i) => i as i64,
            None => -1,
        }
    } else {
        -1
    };
    if shop_nr < 0 || shop_nr as usize >= g.world.shops.len() {
        send_to_char(g, chid, b"Illegal shop number.\r\n");
        return;
    }
    list_detailed_shop(g, chid, shop_nr as usize);
}

pub fn do_show(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    let level = g.ch(chid).level;

    if argument.is_empty() {
        send_to_char(g, chid, b"Show options:\r\n");
        let mut out: BStr = Vec::new();
        let mut j = 0;
        for fld in SHOW_FIELDS.iter().skip(1) {
            if fld.level == 0 {
                break;
            }
            if fld.level <= level {
                out.extend_from_slice(&pad_right(fld.cmd, 15));
                j += 1;
                if j % 5 == 0 {
                    out.extend_from_slice(b"\r\n");
                }
            }
        }
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);
        return;
    }

    let (field, value, arg) = two_arguments(&argument);
    let arg = arg.to_vec();
    // "colour" is the only field spelt one way; "show color" fell through to
    // "Sorry, I don't understand that". Both spellings are accepted here
    // (a cosmetic fix); the option listing still shows one.
    let field = if field == b"color" { b"colour".to_vec() } else { field };
    let l = SHOW_FIELDS.iter().position(|f| f.cmd.starts_with(&field[..])).unwrap_or(SHOW_FIELDS.len());
    let flevel = SHOW_FIELDS.get(l).map(|f| f.level).unwrap_or(0);
    if level < flevel {
        send_to_char(g, chid, b"You are not godly enough for that!\r\n");
        return;
    }
    let self_ = value == b".";
    let _ = arg;

    match l {
        1 => {
            let mut buf: BStr = Vec::new();
            let mut builder = 0;
            if self_ {
                let zone = g.world.rooms[g.ch(chid).in_room as usize].zone as usize;
                print_zone_to_buf(g, &mut buf, zone, true);
            } else if !value.is_empty() && is_number(&value) {
                match g.world.zones.iter().position(|z| z.number as i32 == atoi(&value)) {
                    Some(zrn) => print_zone_to_buf(g, &mut buf, zrn, true),
                    None => {
                        send_to_char(g, chid, b"That is not a valid zone.\r\n");
                        return;
                    }
                }
            } else {
                if !value.is_empty() {
                    builder = 1;
                }
                for zrn in 0..g.world.zones.len() {
                    if !value.is_empty() {
                        let builders = g.world.zones[zrn].builders.clone().unwrap_or_default();
                        let hit = builders
                            .split(|&c| c == b' ')
                            .filter(|w| !w.is_empty())
                            .any(|w| w.eq_ignore_ascii_case(&value));
                        if !hit {
                            continue;
                        }
                        if builder == 1 {
                            builder = 2;
                        }
                    }
                    let mut row = Vec::new();
                    print_zone_to_buf(g, &mut row, zrn, false);
                    if !push_capped(&mut buf, &row) {
                        break;
                    }
                }
            }
            let mut capped = value.clone();
            if let Some(c) = capped.first_mut() {
                *c = c.to_ascii_uppercase();
            }
            if builder == 1 {
                let mut out = capped.clone();
                out.extend_from_slice(b" has not built any zones here.\r\n");
                send_to_char(g, chid, &out);
            } else if builder == 2 {
                let mut out = b"The following zones have been built by: ".to_vec();
                out.extend_from_slice(&capped);
                out.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &out);
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        2 => {
            if value.is_empty() {
                send_to_char(g, chid, b"A name would help.\r\n");
                return;
            }
            let Some(vict) = crate::players_glue::load_char_offline(g, &value) else {
                send_to_char(g, chid, b"There is no such player.\r\n");
                return;
            };
            let tz = g.tz_offset_secs;
            // A second strftime of "%H:%H:%S" would print the hour where
            // the minutes belong; corrected here.
            let buf1 = strftime_full(g.ch(vict).time.birth, tz);
            let buf2 = strftime_full(g.ch(vict).time.logon, tz);
            let sex = mud_data::tables::GENDERS
                .get(g.ch(vict).sex as usize)
                .copied()
                .unwrap_or("");
            let mut out = b"Player: ".to_vec();
            out.extend_from_slice(&pad_right(g.ch(vict).get_name(), 12));
            out.extend_from_slice(format!(" ({}) [{:2} ", sex, g.ch(vict).level).as_bytes());
            out.extend_from_slice(crate::act::informative::class_abbr(g.ch(vict).class));
            out.extend_from_slice(b"]\r\n");
            let p = g.ch(vict).points;
            out.extend_from_slice(
                format!(
                    "Gold: {:<8}  Bal: {:<8} Exp: {:<8}  Align: {:<5}  Lessons: {:<3}\r\n",
                    p.gold,
                    p.bank_gold,
                    p.exp,
                    g.ch(vict).alignment,
                    g.ch(vict).ps().practices
                )
                .as_bytes(),
            );
            out.extend_from_slice(b"Started: ");
            out.extend_from_slice(&crate::act::pad_right_trunc(buf1.as_bytes(), 25));
            out.extend_from_slice(b"  Last: ");
            out.extend_from_slice(&crate::act::pad_right_trunc(buf2.as_bytes(), 25));
            out.extend_from_slice(b"\r\n");
            let played = g.ch(vict).time.played;
            out.extend_from_slice(
                format!("Played: {}h {}m\r\n", played / 3600, played / 60 % 60).as_bytes(),
            );
            send_to_char(g, chid, &out);
            crate::players_glue::free_offline_char(g, vict);
        }
        3 => {
            if value.is_empty() {
                send_to_char(g, chid, b"A name would help.\r\n");
                return;
            }
            crate::objsave::crash_listrent(g, chid, &value);
        }
        4 => {
            let (mut i, mut j, mut k, mut con) = (0, 0, 0, 0);
            for vict in g.character_list.clone() {
                if g.try_ch(vict).is_none() {
                    continue;
                }
                if g.ch(vict).is_npc() {
                    j += 1;
                } else if can_see(g, chid, vict) {
                    i += 1;
                    if g.ch(vict).desc.is_some() {
                        con += 1;
                    }
                }
            }
            for _ in &g.object_list {
                k += 1;
            }
            let out = format!(
                "Current stats:\r\n\
                 \x20 {:5} players in game  {:5} connected\r\n\
                 \x20 {:5} registered\r\n\
                 \x20 {:5} mobiles          {:5} prototypes\r\n\
                 \x20 {:5} objects          {:5} prototypes\r\n\
                 \x20 {:5} rooms            {:5} zones\r\n\
                 \x20 {:5} triggers         {:5} shops\r\n\
                 \x20 {:5} large bufs       {:5} autoquests\r\n\
                 \x20 {:5} buf switches     {:5} overflows\r\n\
                 \x20 {:5} lists\r\n",
                i,
                con,
                g.player_table.len(),
                j,
                g.world.mob_protos.len(),
                k,
                g.world.obj_protos.len(),
                g.world.rooms.len(),
                g.world.zones.len(),
                g.world.triggers.len(),
                g.world.shops.len(),
                g.descriptors.bufstats.largecount,
                g.world.quests.len(),
                g.descriptors.bufstats.switches,
                g.descriptors.bufstats.overflows,
                g.list_count()
            );
            send_to_char(g, chid, out.as_bytes());
        }
        5 => {
            let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
            let mut buf = b"Errant Rooms\r\n------------\r\n".to_vec();
            let mut k = 0;
            'rooms: for i in 0..g.world.rooms.len() {
                for j in 0..crate::fight::dir_count(g) {
                    let Some(ex) = g.world.rooms[i].dir_option[j].as_deref() else { continue };
                    let (to_room, has_gen) = (ex.to_room, ex.general_description.is_some());
                    if to_room == 0 {
                        k += 1;
                        if !push_capped(&mut buf, &errant_row(g, k, i, j, b"void   ", &qnrm)) {
                            break 'rooms;
                        }
                    }
                    if to_room == NOWHERE && !has_gen {
                        k += 1;
                        if !push_capped(&mut buf, &errant_row(g, k, i, j, b"Nowhere", &qnrm)) {
                            break 'rooms;
                        }
                    }
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        6 | 7 => {
            let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
            let (bit, mut buf) = if l == 6 {
                (flags::ROOM_DEATH, b"Death Traps\r\n-----------\r\n".to_vec())
            } else {
                (flags::ROOM_GODROOM, b"Godrooms\r\n--------------------------\r\n".to_vec())
            };
            let mut j = 0;
            for i in 0..g.world.rooms.len() {
                if g.world.rooms[i].room_flags[bit / 32] & (1 << (bit % 32)) == 0 {
                    continue;
                }
                j += 1;
                let mut row = format!("{:2}: [{:5}] ", j, g.world.rooms[i].vnum).into_bytes();
                row.extend_from_slice(g.world.rooms[i].name.as_deref().unwrap_or(b""));
                row.extend_from_slice(&qnrm);
                row.extend_from_slice(b"\r\n");
                if !push_capped(&mut buf, &row) {
                    break;
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        8 => show_shops(g, chid, &value),
        9 => crate::house::hcontrol_list_houses(g, chid, &value),
        10 => {
            let mut i = 0;
            send_to_char(g, chid, b"People currently snooping:\r\n--------------------------\r\n");
            let qnrm = cc(g, chid, C_SPR, KNRM).to_vec();
            let mut out: BStr = Vec::new();
            for di in g.descriptors.order.clone() {
                let Some(d) = g.descriptors.get(di) else { continue };
                let (Some(sd), Some(who)) = (d.snooping, d.character) else { continue };
                if d.state != ConState::Playing || g.try_ch(who).is_none() {
                    continue;
                }
                if g.ch(chid).level < g.ch(who).level {
                    continue;
                }
                if !can_see(g, chid, who) || g.ch(who).in_room == NOWHERE {
                    continue;
                }
                let Some(target) = g.descriptors.get(sd).and_then(|x| x.character) else { continue };
                if g.try_ch(target).is_none() {
                    continue;
                }
                i += 1;
                out.extend_from_slice(&pad_right(g.ch(target).get_name(), 10));
                out.extend_from_slice(&qnrm);
                out.extend_from_slice(b" - snooped by ");
                out.extend_from_slice(g.ch(who).get_name());
                out.extend_from_slice(&qnrm);
                out.extend_from_slice(b".\r\n");
            }
            send_to_char(g, chid, &out);
            if i == 0 {
                send_to_char(g, chid, b"No one is currently snooping.\r\n");
            }
        }
        11 => {
            let mut buf = b"LvL - Mu Cl Th Wa\r\n----------------\r\n".to_vec();
            for j in 1..LVL_IMMORT as i32 {
                let row = format!(
                    "{:<3} - {:<2} {:<2} {:<2} {:<2}\r\n",
                    j,
                    mud_data::tables::thaco(CLASS_MAGIC_USER as i32, j),
                    mud_data::tables::thaco(CLASS_CLERIC as i32, j),
                    mud_data::tables::thaco(CLASS_THIEF as i32, j),
                    mud_data::tables::thaco(CLASS_WARRIOR as i32, j)
                );
                if !push_capped(&mut buf, row.as_bytes()) {
                    break;
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        12 => {
            let mut buf =
                b"LvL - Mu     Cl     Th     Wa\r\n--------------------------\r\n".to_vec();
            let le = mud_data::tables::level_exp;
            for i in 1..LVL_IMMORT as i32 {
                let row = format!(
                    "{:<3} - {:<6} {:<6} {:<6} {:<6}\r\n",
                    i,
                    le(CLASS_MAGIC_USER as i32, i) - le(CLASS_MAGIC_USER as i32, i - 1),
                    le(CLASS_CLERIC as i32, i) - le(CLASS_CLERIC as i32, i - 1),
                    le(CLASS_THIEF as i32, i) - le(CLASS_THIEF as i32, i - 1),
                    le(CLASS_WARRIOR as i32, i) - le(CLASS_WARRIOR as i32, i - 1)
                );
                if !push_capped(&mut buf, row.as_bytes()) {
                    break;
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        13 => {
            let mut buf = b"Colours\r\n--------------------------\r\n".to_vec();
            let mut k = 0;
            'colours: for r in 0..6 {
                for gg in 0..6 {
                    for b in 0..6 {
                        let colour = format!("F{}{}{}", r, gg, b);
                        // ColourRGB emits the \t[F###] form; the protocol
                        // layer resolves it per-descriptor.
                        k += 1;
                        let mut row = format!("\t[{}]{}", colour, colour).into_bytes();
                        row.extend_from_slice(if k % 6 == 0 { &b"\tn\r\n"[..] } else { b"    " });
                        if !push_capped(&mut buf, &row) {
                            break 'colours;
                        }
                    }
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
        }
        _ => send_to_char(g, chid, b"Sorry, I don't understand that.\r\n"),
    }
}

fn errant_row(g: &Game, k: i32, i: usize, j: usize, tag: &[u8], qnrm: &[u8]) -> BStr {
    let name = g.world.rooms[i].name.as_deref().unwrap_or(b"");
    let width = crate::act::other::count_color_chars(name) + 40;
    let mut row = format!("{:2}: (", k).into_bytes();
    row.extend_from_slice(tag);
    row.extend_from_slice(format!(") [{:5}] ", g.world.rooms[i].vnum).as_bytes());
    row.extend_from_slice(&pad_right(name, width));
    row.extend_from_slice(qnrm);
    row.extend_from_slice(b" (");
    row.extend_from_slice(mud_data::tables::DIRS[j].as_bytes());
    row.extend_from_slice(b")\r\n");
    row
}

/// strftime "%a %b %d %H:%M:%S %Y".
fn strftime_full(unix: i64, tz: i64) -> String {
    let c = crate::act::wizard::ctime_like(unix, tz);
    let p: Vec<&str> = c.split_whitespace().collect();
    format!("{} {} {:02} {} {}", p[0], p[1], p[2].parse::<i32>().unwrap_or(0), p[3], p[4])
}

#[allow(unused)]
fn _unused(x: &[u8]) -> BStr {
    sprinttype(0, &["a"]);
    x.to_vec()
}
