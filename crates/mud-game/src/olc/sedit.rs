//! The shop editor.
//!
//! Three shapes worth naming, because they are all observable:
//!
//! * The numeric guard at the top of `sedit_parse` tests the whole answer.
//! A guard that only looks past a leading `-` accepts any non-numeric
//! answer as 0, which is worth knowing before simplifying it.
//! * The product list stores object vnums, as the room list does.
//! `SEDIT_NEW_ROOM` looks the room up to validate it and then stores
//! `atoi(arg)` — the vnum — deliberately.
//! * `sedit_rooms_menu` **rewrites** a room entry it cannot resolve to 0
//! rather than just displaying it as missing ("set to 0 to be deletable",
//! Welcor 09/04), so merely opening the long room menu edits the shop.

use std::cmp::Ordering;

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::ITEM_TYPES;
use mud_data::types::*;
use mud_world::model::{Shop, ShopBuyData};

use crate::act::wizshow::{SHOP_BITS, TRADE_LETTERS};
use crate::act::BStr;
use crate::comm::{act, send_to_char, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::{is_number, two_arguments};
use crate::olc::genshp::{
    add_shop, delete_shop, modify_shop_string, real_shop, reassign_shopkeeper, save_shops,
    ShopRtScratch,
};
use crate::olc::{
    can_edit_zone, clear_screen, genolc_checkstring, get_char_colors, send_cannot_edit, OlcData,
    CLEANUP_ALL, CLEANUP_STRUCTS,
};

/// Submodes of SEDIT connectedness. Everything above
/// `SEDIT_NUMERICAL_RESPONSE` takes a number.
pub const SEDIT_MAIN_MENU: i32 = 0;
pub const SEDIT_CONFIRM_SAVESTRING: i32 = 1;
pub const SEDIT_NOITEM1: i32 = 2;
pub const SEDIT_NOITEM2: i32 = 3;
pub const SEDIT_NOCASH1: i32 = 4;
pub const SEDIT_NOCASH2: i32 = 5;
pub const SEDIT_NOBUY: i32 = 6;
pub const SEDIT_BUY: i32 = 7;
pub const SEDIT_SELL: i32 = 8;
pub const SEDIT_PRODUCTS_MENU: i32 = 11;
pub const SEDIT_ROOMS_MENU: i32 = 12;
pub const SEDIT_NAMELIST_MENU: i32 = 13;
pub const SEDIT_NAMELIST: i32 = 14;
pub const SEDIT_COPY: i32 = 15;
/// Must stay BELOW SEDIT_NUMERICAL_RESPONSE: sedit_parse rejects any
/// non-numeric answer for a mode above it, which would eat the y/n.
pub const SEDIT_CONFIRM_DELETE: i32 = 16;
pub const SEDIT_NUMERICAL_RESPONSE: i32 = 20;
pub const SEDIT_OPEN1: i32 = 21;
pub const SEDIT_OPEN2: i32 = 22;
pub const SEDIT_CLOSE1: i32 = 23;
pub const SEDIT_CLOSE2: i32 = 24;
pub const SEDIT_KEEPER: i32 = 25;
pub const SEDIT_BUY_PROFIT: i32 = 26;
pub const SEDIT_SELL_PROFIT: i32 = 27;
pub const SEDIT_TYPE_MENU: i32 = 29;
pub const SEDIT_DELETE_TYPE: i32 = 30;
pub const SEDIT_DELETE_PRODUCT: i32 = 31;
pub const SEDIT_NEW_PRODUCT: i32 = 32;
pub const SEDIT_DELETE_ROOM: i32 = 33;
pub const SEDIT_NEW_ROOM: i32 = 34;
pub const SEDIT_SHOP_FLAGS: i32 = 35;
pub const SEDIT_NOTRADE: i32 = 36;

fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_sedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    // No building as a mob or while being forced.
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    let mut number: i32 = NOWHERE as i32;
    let mut save = false;

    if buf1.is_empty() {
        send_to_char(g, chid, b"Specify a shop VNUM to edit.\r\n");
        return;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) != Ordering::Equal {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
        save = true;
        if is_number(&buf2) {
            number = atoi(&buf2);
        } else {
            let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
            if olc_zone > 0 {
                number = match g.world.real_zone(olc_zone as Idx) {
                    None => NOWHERE as i32,
                    // The zone below is resolved with real_zone, so
                    // This has to be the zone NUMBER: a vnum here stops any
                    // argument-less save from ever resolving.
                    Some(zlok) => g.world.zones[zlok as usize].number as i32,
                };
            }
        }
        if number == NOWHERE as i32 {
            send_to_char(g, chid, b"Save which zone?\r\n");
            return;
        }
    }

    if number == NOWHERE as i32 {
        number = atoi(&buf1);
    }
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That shop VNUM can't exist.\r\n");
        return;
    }

    // Check that the shop isn't already being edited.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Sedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        let mut msg = b"That shop is currently being edited by ".to_vec();
        msg.extend_from_slice(&who);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis_sedit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();

    let znum = if save {
        g.world.real_zone(number as Idx).map(|z| z as i32)
    } else {
        crate::dg::mobcmd::real_zone_by_thing(g, number).map(|z| z as i32)
    };
    let Some(znum) = znum else {
        send_to_char(g, chid, b"Sorry, there is no zone for that number!\r\n");
        return;
    };
    olc.zone_num = znum;

    if !can_edit_zone(g, chid, znum) {
        let zvnum = g.world.zones[znum as usize].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }

    if save {
        let zvnum = g.world.zones[znum as usize].number;
        send_to_char(g, chid, format!("Saving all shops in zone {}.\r\n", zvnum).as_bytes());
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves shop info for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_shops(g, Some(znum as usize));
        return;
    }

    olc.number = number;

    match real_shop(g, number) {
        Some(real_num) => sedit_setup_existing(g, &mut olc, real_num),
        None => sedit_setup_new(&mut olc),
    }

    sedit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Sedit;
    }

    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum as usize].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!("OLC: {} starts editing zone {} allowed zone {}", name, zvnum, allowed);
    g.mudlog(MudlogKind::Cmp, level, true, &msg);
}

fn sedit_setup_new(olc: &mut OlcData) {
    let shop = Shop {
        close1: 28,
        profit_buy: 1.0,
        profit_sell: 1.0,
        with_who: 0,
        no_such_item1: Some(b"%s Sorry, I don't stock that item.".to_vec()),
        no_such_item2: Some(b"%s You don't seem to have that.".to_vec()),
        missing_cash1: Some(b"%s I can't afford that!".to_vec()),
        missing_cash2: Some(b"%s You are too poor!".to_vec()),
        do_not_buy: Some(b"%s I don't trade in such items.".to_vec()),
        message_buy: Some(b"%s That'll be %d coins, thanks.".to_vec()),
        message_sell: Some(b"%s I'll give you %d coins for that.".to_vec()),
        ..Default::default()
    };
    let rt = ShopRtScratch::new_shop();
    olc.shop_keeper = rt.keeper;
    olc.shop_bank = rt.bank;
    olc.shop_sort = rt.sort;
    olc.shop_func = rt.func;
    olc.shop = Some(Box::new(shop));
}

pub fn sedit_setup_existing(g: &Game, olc: &mut OlcData, rshop_num: usize) {
    let mut shop = g.world.shops[rshop_num].clone();
    // Every message is defaulted, so an absent one reads "undefined".
    for slot in [
        &mut shop.no_such_item1,
        &mut shop.no_such_item2,
        &mut shop.missing_cash1,
        &mut shop.missing_cash2,
        &mut shop.do_not_buy,
        &mut shop.message_buy,
        &mut shop.message_sell,
    ] {
        *slot = Some(crate::olc::str_udup(slot.as_deref().unwrap_or(b"")));
    }
    let rt = ShopRtScratch::from_rt(&g.shops_rt[rshop_num]);
    olc.shop_keeper = rt.keeper;
    olc.shop_bank = rt.bank;
    olc.shop_sort = rt.sort;
    olc.shop_func = rt.func;
    olc.shop = Some(Box::new(shop));
}

fn sedit_save_internally(g: &mut Game, di: usize, olc: &mut OlcData) {
    // Read before add_shop overwrites the record: the mobile this shop used to
    // keep, and the spec proc that mobile had before it was made a keeper.
    let (oldkeeper, oldfunc) = match real_shop(g, olc.number) {
        Some(r) => (g.shops_rt[r].keeper, g.shops_rt[r].func),
        None => (NOBODY, None),
    };

    let mut shop = olc.shop.as_ref().unwrap().as_ref().clone();
    shop.vnum = olc.number as Idx;
    // The file field the writer prints; the runtime rnum lives in ShopRt.
    shop.keeper_vnum = if olc.shop_keeper == NOBODY {
        -1
    } else {
        g.world.mob_protos[olc.shop_keeper as usize].vnum as i32
    };
    let rt = ShopRtScratch {
        keeper: olc.shop_keeper,
        bank: olc.shop_bank,
        sort: olc.shop_sort,
        func: olc.shop_func,
    };
    add_shop(g, &shop, &rt);

    let released = reassign_shopkeeper(g, olc.number, oldkeeper, oldfunc);

    // A released keeper that lives in another zone has its mobile file queued
    // rather than written -- see reassign_shopkeeper. Queued is silent, so say
    // which zone is now waiting on a save; until it happens a reboot brings the
    // stale flag back.
    if released != NOBODY {
        let kvnum = g.world.mob_protos[released as usize].vnum as i32;
        let kz = crate::dg::mobcmd::real_zone_by_thing(g, kvnum);
        let shopzone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
        if let Some(kz) = kz {
            let number = g.world.zones[kz].number;
            if Some(kz) != shopzone && crate::db::in_save_list(g, number, crate::db::SL_MOB) {
                let msg = format!(
                    "The old keeper (mobile {}) lives in zone {}; that zone's \
                     mobile file still needs saving.\r\n",
                    kvnum, number
                );
                write_to_desc(g, di, msg.as_bytes());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn sedit_products_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    write_to_desc(g, di, b"##     VNUM     Product\r\n");

    let products = olc.shop.as_ref().unwrap().producing.clone();
    for (i, vnum) in products.iter().enumerate() {
        let short = g
            .world
            .real_object(*vnum as Idx)
            .and_then(|r| g.world.obj_protos[r as usize].short_description.clone())
            .unwrap_or_default();
        let c = g.olc_colors;
        let mut line: BStr = format!("{:2} - [", i).into_bytes();
        line.extend_from_slice(c.cyn());
        line.extend_from_slice(format!("{:5}", vnum).as_bytes());
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"] - ");
        line.extend_from_slice(c.yel());
        line.extend_from_slice(&short);
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"\r\n");
        write_to_desc(g, di, &line);
    }
    let c = g.olc_colors;
    let out = format!(
        "\r\n{}A{}) Add a new product.\r\n{}D{}) Delete a product.\r\n{}Q{}) Quit\r\nEnter choice : ",
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s()
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = SEDIT_PRODUCTS_MENU;
}

fn sedit_compact_rooms_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);

    let rooms = olc.shop.as_ref().unwrap().in_rooms.clone();
    for (i, vnum) in rooms.iter().enumerate() {
        match g.real_room(*vnum) {
            Some(rnum) => {
                let name = g.world.rooms[rnum as usize].name.clone().unwrap_or_default();
                let mut line: BStr = format!("{:2} - [@\t{:5}\tn] - \ty", i, vnum).into_bytes();
                line.extend_from_slice(&name);
                line.extend_from_slice(b"\tn\r\n");
                write_to_desc(g, di, &line);
            }
            None => {
                let line = format!("{:2} - [\tR!Removed Room!\tn]\r\n", i);
                write_to_desc(g, di, line.as_bytes());
            }
        }
    }
    let c = g.olc_colors;
    let out = format!(
        "\r\n{}A{}) Add a new room.\r\n{}D{}) Delete a room.\r\n{}L{}) Long display.\r\n\
         {}Q{}) Quit\r\nEnter choice : ",
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s()
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = SEDIT_ROOMS_MENU;
}

/// sedit_rooms_menu. An unresolvable room is *rewritten*
/// to 0 here, not merely flagged — opening this menu edits the shop.
fn sedit_rooms_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    write_to_desc(g, di, b"##     VNUM     Room\r\n\r\n");

    let count = olc.shop.as_ref().unwrap().in_rooms.len();
    for i in 0..count {
        let vnum = olc.shop.as_ref().unwrap().in_rooms[i];
        let rnum = match g.real_room(vnum) {
            Some(r) => r,
            None => {
                olc.shop.as_mut().unwrap().in_rooms[i] = 0;
                0
            }
        };
        let vnum = olc.shop.as_ref().unwrap().in_rooms[i];
        let name = g.world.rooms[rnum as usize].name.clone().unwrap_or_default();
        let c = g.olc_colors;
        let mut line: BStr = format!("{:2} - [", i).into_bytes();
        line.extend_from_slice(c.cyn());
        line.extend_from_slice(format!("{:5}", vnum).as_bytes());
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"] - ");
        line.extend_from_slice(c.yel());
        line.extend_from_slice(&name);
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"\r\n");
        write_to_desc(g, di, &line);
    }
    let c = g.olc_colors;
    let out = format!(
        "\r\n{}A{}) Add a new room.\r\n{}D{}) Delete a room.\r\n{}C{}) Compact Display.\r\n\
         {}Q{}) Quit\r\nEnter choice : ",
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s()
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = SEDIT_ROOMS_MENU;
}

fn sedit_namelist_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    write_to_desc(g, di, b"##              Type   Namelist\r\n\r\n");

    let list = olc.shop.as_ref().unwrap().type_list.clone();
    for (i, entry) in list.iter().enumerate() {
        let tname = ITEM_TYPES.get(entry.type_ as usize).copied().unwrap_or("UNDEFINED");
        let c = g.olc_colors;
        let mut line: BStr = format!("{:2} - ", i).into_bytes();
        line.extend_from_slice(c.cyn());
        line.extend_from_slice(format!("{:>15}", tname).as_bytes());
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b" - ");
        line.extend_from_slice(c.yel());
        line.extend_from_slice(entry.keywords.as_deref().unwrap_or(b"<None>"));
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"\r\n");
        write_to_desc(g, di, &line);
    }
    let c = g.olc_colors;
    let out = format!(
        "\r\n{}A{}) Add a new entry.\r\n{}D{}) Delete an entry.\r\n{}Q{}) Quit\r\nEnter choice : ",
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s(),
        c.grn_s(),
        c.nrm_s()
    );
    write_to_desc(g, di, out.as_bytes());
    olc.mode = SEDIT_NAMELIST_MENU;
}

fn sedit_shop_flags_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let mut count = 0;
    for (i, name) in SHOP_BITS.iter().enumerate() {
        count += 1;
        let c = g.olc_colors;
        let mut n = name.as_bytes().to_vec();
        n.truncate(20);
        while n.len() < 20 {
            n.push(b' ');
        }
        let mut line: BStr = format!("{}{:2}{}) ", c.grn_s(), i + 1, c.nrm_s()).into_bytes();
        line.extend_from_slice(&n);
        line.extend_from_slice(b"   ");
        if count % 2 == 0 {
            line.extend_from_slice(b"\r\n");
        }
        write_to_desc(g, di, &line);
    }
    let bits =
        crate::quest::sprintbit(olc.shop.as_ref().unwrap().bitvector as i64, &SHOP_BITS);
    let c = g.olc_colors;
    let mut out: BStr = b"\r\nCurrent Shop Flags : ".to_vec();
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter choice : ");
    write_to_desc(g, di, &out);
    olc.mode = SEDIT_SHOP_FLAGS;
}

fn sedit_no_trade_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let mut count = 0;
    for (i, name) in TRADE_LETTERS.iter().enumerate() {
        count += 1;
        let c = g.olc_colors;
        let mut n = name.as_bytes().to_vec();
        n.truncate(20);
        while n.len() < 20 {
            n.push(b' ');
        }
        let mut line: BStr = format!("{}{:2}{}) ", c.grn_s(), i + 1, c.nrm_s()).into_bytes();
        line.extend_from_slice(&n);
        line.extend_from_slice(b"   ");
        if count % 2 == 0 {
            line.extend_from_slice(b"\r\n");
        }
        write_to_desc(g, di, &line);
    }
    let bits =
        crate::quest::sprintbit(olc.shop.as_ref().unwrap().with_who as i64, &TRADE_LETTERS);
    let c = g.olc_colors;
    let mut out: BStr = b"\r\nCurrently won't trade with: ".to_vec();
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter choice : ");
    write_to_desc(g, di, &out);
    olc.mode = SEDIT_NOTRADE;
}

fn sedit_types_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let mut count = 0;
    for i in 0..flags::NUM_ITEM_TYPES {
        count += 1;
        let c = g.olc_colors;
        let mut n = ITEM_TYPES[i].as_bytes().to_vec();
        while n.len() < 20 {
            n.push(b' ');
        }
        let mut line: BStr = format!("{}{:2}{}) {}", c.grn_s(), i, c.nrm_s(), c.cyn_s()).into_bytes();
        line.extend_from_slice(&n);
        line.extend_from_slice(c.nrm());
        line.extend_from_slice(b"  ");
        if count % 3 == 0 {
            line.extend_from_slice(b"\r\n");
        }
        write_to_desc(g, di, &line);
    }
    let c = g.olc_colors;
    let mut out: BStr = c.nrm().to_vec();
    out.extend_from_slice(b"Enter choice : ");
    write_to_desc(g, di, &out);
    olc.mode = SEDIT_TYPE_MENU;
}

fn sedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);

    let shop = olc.shop.as_ref().unwrap().as_ref().clone();
    let notrade = crate::quest::sprintbit(shop.with_who as i64, &TRADE_LETTERS);
    let flags_str = crate::quest::sprintbit(shop.bitvector as i64, &SHOP_BITS);
    let (keeper_vnum, keeper_name): (i32, BStr) = if olc.shop_keeper == NOBODY {
        (-1, b"None".to_vec())
    } else {
        let p = &g.world.mob_protos[olc.shop_keeper as usize];
        (p.vnum as i32, p.short_descr.clone().unwrap_or_default())
    };

    let c = g.olc_colors;
    let (nrm, grn, cyn, yel) = (c.nrm_s(), c.grn_s(), c.cyn_s(), c.yel_s());
    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!("-- Shop Number : [{}{}{}]\r\n", cyn, olc.number, nrm).as_bytes(),
    );
    out.extend_from_slice(
        format!("{}0{}) Keeper      : [{}{}{}] {}", grn, nrm, cyn, keeper_vnum, nrm, yel)
            .as_bytes(),
    );
    out.extend_from_slice(&keeper_name);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}1{}) Open 1      : {}{:4}{}          {}2{}) Close 1     : {}{:4}\r\n",
            grn, nrm, cyn, shop.open1, nrm, grn, nrm, cyn, shop.close1
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{}3{}) Open 2      : {}{:4}{}          {}4{}) Close 2     : {}{:4}\r\n",
            grn, nrm, cyn, shop.open2, nrm, grn, nrm, cyn, shop.close2
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{}5{}) Sell rate   : {}{:.2}{}          {}6{}) Buy rate    : {}{:.2}\r\n",
            grn, nrm, cyn, shop.profit_buy, nrm, grn, nrm, cyn, shop.profit_sell
        )
        .as_bytes(),
    );
    for (key, label, text) in [
        (&b"7"[..], &b"Keeper no item "[..], shop.no_such_item1.as_deref()),
        (b"8", b"Player no item ", shop.no_such_item2.as_deref()),
        (b"9", b"Keeper no cash ", shop.missing_cash1.as_deref()),
        (b"A", b"Player no cash ", shop.missing_cash2.as_deref()),
        (b"B", b"Keeper no buy  ", shop.do_not_buy.as_deref()),
        (b"C", b"Buy success    ", shop.message_buy.as_deref()),
        (b"D", b"Sell success   ", shop.message_sell.as_deref()),
    ] {
        out.extend_from_slice(grn.as_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(nrm.as_bytes());
        out.extend_from_slice(b") ");
        out.extend_from_slice(label);
        out.extend_from_slice(b": ");
        out.extend_from_slice(yel.as_bytes());
        out.extend_from_slice(text.unwrap_or(b"(null)"));
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("{}E{}) No Trade With  : {}", grn, nrm, cyn).as_bytes());
    out.extend_from_slice(&notrade);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}F{}) Shop flags     : {}", grn, nrm, cyn).as_bytes());
    out.extend_from_slice(&flags_str);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}R{}) Rooms Menu\r\n{}P{}) Products Menu\r\n{}T{}) Accept Types Menu\r\n\
             {}W{}) Copy Shop\r\n{}X{}) Delete Shop\r\n{}Q{}) Quit\r\nEnter Choice : ",
            grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm, grn, nrm
        )
        .as_bytes(),
    );
    write_to_desc(g, di, &out);
    olc.mode = SEDIT_MAIN_MENU;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn sedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    // The whole answer is tested, not just the byte after a leading `-`:
    // that weaker test lets every non-numeric answer through as 0.
    if olc.mode > SEDIT_NUMERICAL_RESPONSE {
        let numeric = arg.first().is_some_and(|c| c.is_ascii_digit())
            || (arg.first() == Some(&b'-') && arg.get(1).is_some_and(|c| c.is_ascii_digit()));
        if arg.is_empty() || !numeric {
            write_to_desc(g, di, b"Field must be numerical, try again : ");
            return Some(olc);
        }
    }

    match olc.mode {
        SEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    sedit_save_internally(g, di, &mut olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg = format!("OLC: {} edits shop {}", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    if g.config.auto_save_olc {
                        let zone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if save_shops(g, zone) {
                            write_to_desc(g, di, b"Shop saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the shop"));
                        }
                    } else {
                        write_to_desc(g, di, b"Shop saved to memory.\r\n");
                    }
                    // CLEANUP_STRUCTS, not CLEANUP_ALL: the shop's strings
                    // are now owned by the table.
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_STRUCTS);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to save your changes? : ",
                    );
                }
            }
            return Some(olc);
        }

        SEDIT_CONFIRM_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    // Resolve by VNUM, never the stored index: another builder
                    // saving a shop renumbers this one underneath the editor.
                    let drnum = real_shop(g, olc.number);

                    // A keeper that stops being one loses MOB_SPEC, and the mob
                    // file is written so that sticks. Anyone holding that mobile
                    // in medit took their copy before the change and would save
                    // the flag straight back -- to disk, and across the reboot --
                    // putting back the SYSERR the clearing exists to stop. So the
                    // two do not happen at once. sedit already refuses a shop
                    // another descriptor holds; this is the same refusal, one
                    // record along.
                    if let Some(r) = drnum {
                        let keeper = g.shops_rt[r].keeper;
                        if keeper != NOBODY {
                            let kvnum = g.world.mob_protos[keeper as usize].vnum as i32;
                            let holder = g.descriptors.order.clone().into_iter().find(|&dsc| {
                                dsc != di
                                    && g.descriptors.get(dsc).map(|x| x.state)
                                        == Some(ConState::Medit)
                                    && g.olc.get(&dsc).map(|o| o.number) == Some(kvnum)
                            });
                            if let Some(dsc) = holder {
                                let who = g
                                    .descriptors
                                    .get(dsc)
                                    .and_then(|x| x.character)
                                    .map(|c| String::from_utf8_lossy(g.ch(c).get_name()).into_owned())
                                    .unwrap_or_else(|| "someone".to_string());
                                let msg = format!(
                                    "This shop's keeper (mobile {}) is currently being \
                                     edited by {}.\r\n",
                                    kvnum, who
                                );
                                write_to_desc(g, di, msg.as_bytes());
                                sedit_disp_menu(g, di, &mut olc);
                                return Some(olc);
                            }
                        }
                    }

                    // Read before the delete: afterwards drnum names a different
                    // shop, or none. Only used to tell the builder about a keeper
                    // whose mob file is in another zone.
                    let (keepvnum, kzone) = match drnum {
                        Some(r) if g.shops_rt[r].keeper != NOBODY => {
                            let kv = g.world.mob_protos[g.shops_rt[r].keeper as usize].vnum as i32;
                            (kv, crate::dg::mobcmd::real_zone_by_thing(g, kv))
                        }
                        _ => (-1, None),
                    };

                    if drnum.is_some_and(|r| delete_shop(g, r)) {
                        if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                            let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                            g.mudlog(
                                MudlogKind::Cmp,
                                level,
                                true,
                                &format!("OLC: {} deletes shop {}", name, olc.number),
                            );
                        }
                        write_to_desc(g, di, b"Shop deleted.\r\n");
                        // delete_shop marks the zone; whether that reaches disk
                        // now is the same question every other editor answers
                        // here, and qedit's delete says so explicitly rather than
                        // leaving the builder to guess.
                        if g.config.auto_save_olc {
                            let z = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                            crate::olc::genshp::save_shops(g, z);
                            write_to_desc(g, di, b"Shop file saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, b"Shop file saved to memory.\r\n");
                        }

                        // A keeper that stopped being one loses MOB_SPEC, and
                        // that is only written out here when it lives in the zone
                        // this delete already writes. Anywhere else it is queued
                        // -- correct, since reaching across to write a zone the
                        // builder may not own would also flush somebody else's
                        // pending work there. But the queue is silent, and "saved
                        // to disk" reads like the whole job is done.
                        let shopzone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if let Some(kz) = kzone {
                            let number = g.world.zones[kz].number;
                            if Some(kz) != shopzone
                                && crate::db::in_save_list(g, number, crate::db::SL_MOB)
                            {
                                let msg = format!(
                                    "The keeper (mobile {}) lives in zone {}; that zone's \
                                     mobile file still needs saving.\r\n",
                                    keepvnum, number
                                );
                                write_to_desc(g, di, msg.as_bytes());
                            }
                        }
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    // Nothing went, so nothing is discarded either.
                    write_to_desc(g, di, b"Could not delete that shop.\r\n");
                    sedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'n') | Some(b'N') => {
                    sedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\nDelete this shop? : ");
                    return Some(olc);
                }
            }
        }

        SEDIT_MAIN_MENU => {
            // `i` marks what kind of prompt follows: 1 numeric, -1 text,
            // 0 straight back to the menu.
            let mut i = 0;
            match arg.first().copied() {
                Some(b'x') | Some(b'X') => {
                    if real_shop(g, olc.number).is_none() {
                        write_to_desc(
                            g,
                            di,
                            b"That shop has never been saved -- quit without saving instead.\r\n",
                        );
                        sedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    write_to_desc(g, di, b"Are you sure you want to delete this shop? ");
                    olc.mode = SEDIT_CONFIRM_DELETE;
                    return Some(olc);
                }
                Some(b'w') | Some(b'W') => {
                    write_to_desc(g, di, b"Copy what shop? ");
                    olc.mode = SEDIT_COPY;
                    return Some(olc);
                }
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = SEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    return Some(olc);
                }
                Some(b'0') => {
                    olc.mode = SEDIT_KEEPER;
                    write_to_desc(g, di, b"Enter vnum number of shop keeper : ");
                    return Some(olc);
                }
                Some(b'1') => {
                    olc.mode = SEDIT_OPEN1;
                    i += 1;
                }
                Some(b'2') => {
                    olc.mode = SEDIT_CLOSE1;
                    i += 1;
                }
                Some(b'3') => {
                    olc.mode = SEDIT_OPEN2;
                    i += 1;
                }
                Some(b'4') => {
                    olc.mode = SEDIT_CLOSE2;
                    i += 1;
                }
                Some(b'5') => {
                    olc.mode = SEDIT_BUY_PROFIT;
                    i += 1;
                }
                Some(b'6') => {
                    olc.mode = SEDIT_SELL_PROFIT;
                    i += 1;
                }
                Some(b'7') => {
                    olc.mode = SEDIT_NOITEM1;
                    i -= 1;
                }
                Some(b'8') => {
                    olc.mode = SEDIT_NOITEM2;
                    i -= 1;
                }
                Some(b'9') => {
                    olc.mode = SEDIT_NOCASH1;
                    i -= 1;
                }
                Some(b'a') | Some(b'A') => {
                    olc.mode = SEDIT_NOCASH2;
                    i -= 1;
                }
                Some(b'b') | Some(b'B') => {
                    olc.mode = SEDIT_NOBUY;
                    i -= 1;
                }
                Some(b'c') | Some(b'C') => {
                    olc.mode = SEDIT_BUY;
                    i -= 1;
                }
                Some(b'd') | Some(b'D') => {
                    olc.mode = SEDIT_SELL;
                    i -= 1;
                }
                Some(b'e') | Some(b'E') => {
                    sedit_no_trade_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'f') | Some(b'F') => {
                    sedit_shop_flags_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'r') | Some(b'R') => {
                    sedit_rooms_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'p') | Some(b'P') => {
                    sedit_products_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b't') | Some(b'T') => {
                    sedit_namelist_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    sedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
            // `if (i == 0) break;` drops to the dirty-flag + menu at the
            // bottom. No case above leaves i at 0 without returning, so it is
            // unreachable — kept so the shape matches.
            match i {
                0 => {}
                1 => {
                    write_to_desc(g, di, b"\r\nEnter new value : ");
                    return Some(olc);
                }
                -1 => {
                    write_to_desc(g, di, b"\r\nEnter new text :\r\n] ");
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Oops...\r\n");
                    return Some(olc);
                }
            }
        }

        SEDIT_NAMELIST_MENU => match arg.first().copied() {
            Some(b'a') | Some(b'A') => {
                sedit_types_menu(g, di, &mut olc);
                return Some(olc);
            }
            Some(b'd') | Some(b'D') => {
                write_to_desc(g, di, b"\r\nDelete which entry? : ");
                olc.mode = SEDIT_DELETE_TYPE;
                return Some(olc);
            }
            // 'q' and anything else fall out to the main menu below.
            _ => {}
        },

        SEDIT_PRODUCTS_MENU => match arg.first().copied() {
            Some(b'a') | Some(b'A') => {
                write_to_desc(g, di, b"\r\nEnter new product vnum number : ");
                olc.mode = SEDIT_NEW_PRODUCT;
                return Some(olc);
            }
            Some(b'd') | Some(b'D') => {
                write_to_desc(g, di, b"\r\nDelete which product? : ");
                olc.mode = SEDIT_DELETE_PRODUCT;
                return Some(olc);
            }
            _ => {}
        },

        SEDIT_ROOMS_MENU => match arg.first().copied() {
            Some(b'a') | Some(b'A') => {
                write_to_desc(g, di, b"\r\nEnter new room vnum number : ");
                olc.mode = SEDIT_NEW_ROOM;
                return Some(olc);
            }
            Some(b'c') | Some(b'C') => {
                sedit_compact_rooms_menu(g, di, &mut olc);
                return Some(olc);
            }
            Some(b'l') | Some(b'L') => {
                sedit_rooms_menu(g, di, &mut olc);
                return Some(olc);
            }
            Some(b'd') | Some(b'D') => {
                write_to_desc(g, di, b"\r\nDelete which room? : ");
                olc.mode = SEDIT_DELETE_ROOM;
                return Some(olc);
            }
            _ => {}
        },

        // String edits: every keeper message goes through modify_shop_string
        // so it keeps the "%s" the keeper's name is substituted into.
        SEDIT_NOITEM1 | SEDIT_NOITEM2 | SEDIT_NOCASH1 | SEDIT_NOCASH2 | SEDIT_NOBUY
        | SEDIT_BUY | SEDIT_SELL => {
            let mut text = arg.to_vec();
            if genolc_checkstring(&mut text) {
                let new = modify_shop_string(&text);
                let shop = olc.shop.as_mut().unwrap();
                let mode = olc.mode;
                let slot = match mode {
                    SEDIT_NOITEM1 => &mut shop.no_such_item1,
                    SEDIT_NOITEM2 => &mut shop.no_such_item2,
                    SEDIT_NOCASH1 => &mut shop.missing_cash1,
                    SEDIT_NOCASH2 => &mut shop.missing_cash2,
                    SEDIT_NOBUY => &mut shop.do_not_buy,
                    SEDIT_BUY => &mut shop.message_buy,
                    _ => &mut shop.message_sell,
                };
                *slot = Some(new);
            }
        }

        SEDIT_NAMELIST => {
            let mut text = arg.to_vec();
            if genolc_checkstring(&mut text) {
                let type_ = olc.value;
                olc.shop
                    .as_mut()
                    .unwrap()
                    .type_list
                    .push(ShopBuyData { type_, keywords: Some(text) });
            }
            sedit_namelist_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_KEEPER => {
            let mut i = atoi(arg);
            if i != -1 {
                match g.world.real_mobile(i as Idx) {
                    Some(r) => i = r as i32,
                    None => {
                        write_to_desc(g, di, b"That mobile does not exist, try again : ");
                        return Some(olc);
                    }
                }
            }
            // The working copy, and nothing else. Installing the proc here
            // put a mobile to work the moment the vnum was typed -- before the
            // builder had saved, and whether or not they went on to. Quitting
            // without saving left it a shopkeeper for a shop that still names
            // somebody else, and answering the prompt twice left the first
            // mobile answering `list` with "Sorry, but you cannot do that
            // here!" for the rest of the reboot. Both mobiles are put right by
            // reassign_shopkeeper when the shop is actually saved.
            olc.shop_keeper = if i == -1 { NOBODY } else { i as Idx };
        }

        SEDIT_OPEN1 => olc.shop.as_mut().unwrap().open1 = limit(atoi(arg), 0, 28),
        SEDIT_OPEN2 => olc.shop.as_mut().unwrap().open2 = limit(atoi(arg), 0, 28),
        SEDIT_CLOSE1 => olc.shop.as_mut().unwrap().close1 = limit(atoi(arg), 0, 28),
        SEDIT_CLOSE2 => olc.shop.as_mut().unwrap().close2 = limit(atoi(arg), 0, 28),

        SEDIT_BUY_PROFIT => {
            // Nothing parseable leaves the field untouched.
            if let Some(v) = scan_f32(arg) {
                olc.shop.as_mut().unwrap().profit_buy = v;
            }
        }
        SEDIT_SELL_PROFIT => {
            if let Some(v) = scan_f32(arg) {
                olc.shop.as_mut().unwrap().profit_sell = v;
            }
        }

        SEDIT_TYPE_MENU => {
            olc.value = limit(atoi(arg), 0, flags::NUM_ITEM_TYPES as i32 - 1);
            write_to_desc(g, di, b"Enter namelist (return for none) :-\r\n] ");
            olc.mode = SEDIT_NAMELIST;
            return Some(olc);
        }

        SEDIT_DELETE_TYPE => {
            let n = atoi(arg);
            let list = &mut olc.shop.as_mut().unwrap().type_list;
            if n >= 0 && (n as usize) < list.len() {
                list.remove(n as usize);
            }
            sedit_namelist_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_NEW_PRODUCT => {
            let mut i = atoi(arg);
            if i != -1 {
                match g.world.real_object(i as Idx) {
                    Some(r) => i = r as i32,
                    None => {
                        write_to_desc(g, di, b"That object does not exist, try again : ");
                        return Some(olc);
                    }
                }
            }
            // Guarding `i > 0` on the *rnum* means object rnum 0 —
            // vnum 1, "a pair of wings" in the shipped world — can never be
            // added to any shop, and the builder gets no error, just the
            // menu again. Confirmed live: the input was taken and left the
            // list unchanged. The room case below already uses `>= 0`.
            if i >= 0 {
                let vnum = g.world.obj_protos[i as usize].vnum as i32;
                olc.shop.as_mut().unwrap().producing.push(vnum);
            }
            sedit_products_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_DELETE_PRODUCT => {
            let n = atoi(arg);
            let list = &mut olc.shop.as_mut().unwrap().producing;
            if n >= 0 && (n as usize) < list.len() {
                list.remove(n as usize);
            }
            sedit_products_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_NEW_ROOM => {
            let raw = atoi(arg);
            let mut i = raw;
            if i != -1 {
                match g.real_room(i) {
                    Some(r) => i = r as i32,
                    None => {
                        write_to_desc(g, di, b"That room does not exist, try again : ");
                        return Some(olc);
                    }
                }
            }
            // Validated by rnum, stored by vnum — deliberate.
            if i >= 0 {
                olc.shop.as_mut().unwrap().in_rooms.push(raw);
            }
            sedit_rooms_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_DELETE_ROOM => {
            let n = atoi(arg);
            let list = &mut olc.shop.as_mut().unwrap().in_rooms;
            if n >= 0 && (n as usize) < list.len() {
                list.remove(n as usize);
            }
            sedit_rooms_menu(g, di, &mut olc);
            return Some(olc);
        }

        SEDIT_SHOP_FLAGS => {
            let i = limit(atoi(arg), 0, SHOP_BITS.len() as i32);
            if i > 0 {
                olc.shop.as_mut().unwrap().bitvector ^= 1u32 << (i - 1);
                sedit_shop_flags_menu(g, di, &mut olc);
                return Some(olc);
            }
        }

        SEDIT_NOTRADE => {
            let i = limit(atoi(arg), 0, TRADE_LETTERS.len() as i32);
            if i > 0 {
                olc.shop.as_mut().unwrap().with_who ^= 1i32 << (i - 1);
                sedit_no_trade_menu(g, di, &mut olc);
                return Some(olc);
            }
        }

        SEDIT_COPY => match real_shop(g, atoi(arg)) {
            Some(i) => sedit_setup_existing(g, &mut olc, i),
            None => write_to_desc(g, di, b"That shop does not exist.\r\n"),
        },

        _ => {
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: sedit_parse(): Reached default case!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
            return None;
        }
    }

    // Anything reaching here changed something.
    olc.value = 1;
    sedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

/// A float: leading whitespace, optional sign, digits and an optional
/// fractional part. Returns None when nothing parses.
fn scan_f32(arg: &[u8]) -> Option<f32> {
    let mut p = 0;
    while p < arg.len() && arg[p].is_ascii_whitespace() {
        p += 1;
    }
    let start = p;
    if matches!(arg.get(p), Some(b'+') | Some(b'-')) {
        p += 1;
    }
    let digits = p;
    while p < arg.len() && arg[p].is_ascii_digit() {
        p += 1;
    }
    if p < arg.len() && arg[p] == b'.' {
        p += 1;
        while p < arg.len() && arg[p].is_ascii_digit() {
            p += 1;
        }
    }
    if p == digits {
        return None;
    }
    std::str::from_utf8(&arg[start..p]).ok()?.parse::<f32>().ok()
}
