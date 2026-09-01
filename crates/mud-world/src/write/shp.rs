//! .shp writer: save_shops.
//!
//! Always emits the "CircleMUD v3.0 Shop File~" header, then every shop
//! whose vnum falls in the zone's bot..=top range, in ascending vnum
//! order, then "$~\n".
//!
//! Layout per shop: "#<vnum>~", producing vnums + "-1", the two profits as
//! "%1.2f", buy-types as "%d%s" (keyword glued straight after the number,
//! no space), "-1", the seven messages + temper/bitvector/keeper/with_who
//! (this block alone passes through convert_from_tabs — parse_tab turns
//! '\t' back into '@' except that a "\t\t" pair is left alone; \r is NOT
//! stripped), rooms + "-1", and the four hours. NULL messages fall back to
//! the "Ke?!" defaults; a keeper with no real mob writes -1.

use crate::model::World;
use crate::write::VnumFmt;

/// '\t' followed by anything but '\t' becomes '@';
/// a "\t\t" pair is skipped over UNCHANGED (mirror of parse_at).
fn parse_tab(s: &mut [u8]) {
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\t' {
            if s.get(i + 1) != Some(&b'\t') {
                s[i] = b'@';
            } else {
                i += 1;
            }
        }
        i += 1;
    }
}

/// "%1.2f" of a float promoted to double: fixed two decimals, rounding
/// the exact binary value half-to-even — Rust's {:.2} does the same.
fn push_profit(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(format!("{v:.2}\n").as_bytes());
}

fn push_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(v.to_string().as_bytes());
}

fn push_msg(out: &mut Vec<u8>, msg: &Option<Vec<u8>>, default: &[u8]) {
    match msg {
        Some(m) => out.extend_from_slice(m),
        None => out.extend_from_slice(default),
    }
    out.extend_from_slice(b"~\n");
}

pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let zone = &world.zones[zone_rnum as usize];
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"CircleMUD v3.0 Shop File~\n");

    for vnum in zone.bot..=zone.top {
        // real_shop: the first shop with this vnum. The shipped data is
        // sorted and unique.
        let Some(shop) = world.shops.iter().find(|s| s.vnum == vnum) else {
            continue;
        };

        out.push(b'#');
        fmt.push_vnum(&mut out, vnum as i64);
        out.extend_from_slice(b"~\n");

        // Producing list: entries are stored as looked-up vnums, walked
        // until the NOTHING sentinel.
        // An export drops what it cannot carry: 377 of
        // the shipped products come from another zone, and a shop stocking
        // other zones' goods is normal, not a mistake to mark. `in_zone`
        // is always true on a real save, so nothing is dropped there.
        for &p in &shop.producing {
            if !fmt.in_zone(p as i64) {
                continue;
            }
            fmt.push_vnum(&mut out, p as i64);
            out.push(b'\n');
        }
        out.extend_from_slice(b"-1\n");

        push_profit(&mut out, shop.profit_buy);
        push_profit(&mut out, shop.profit_sell);

        // "%d%s\n" — keyword glued directly after the type number.
        for t in &shop.type_list {
            push_i64(&mut out, t.type_ as i64);
            if let Some(k) = &t.keywords {
                out.extend_from_slice(k);
            }
            out.push(b'\n');
        }
        out.extend_from_slice(b"-1\n");

        // The message/numeric block is one buffer funnelled
        // through convert_from_tabs; nothing else in the record is.
        let mut mb: Vec<u8> = Vec::new();
        push_msg(&mut mb, &shop.no_such_item1, b"%s Ke?!");
        push_msg(&mut mb, &shop.no_such_item2, b"%s Ke?!");
        push_msg(&mut mb, &shop.do_not_buy, b"%s Ke?!");
        push_msg(&mut mb, &shop.missing_cash1, b"%s Ke?!");
        push_msg(&mut mb, &shop.missing_cash2, b"%s Ke?!");
        push_msg(&mut mb, &shop.message_buy, b"%s Ke?! %d?");
        push_msg(&mut mb, &shop.message_sell, b"%s Ke?! %d?");
        push_i64(&mut mb, shop.temper1 as i64);
        mb.push(b'\n');
        push_i64(&mut mb, shop.bitvector as i64);
        mb.push(b'\n');
        // A keeper of NOBODY writes -1; otherwise the vnum, resolved
        // the vnum to a rnum at boot; the deferred map lookup is identical.
        // The keeper is mandatory, so an export can't drop it the way it
        // drops a product — it goes out in the zone's own numbering even
        // when the mob lives elsewhere (19 shipped shops do).
        if world.mob_map.contains_key(&(shop.keeper_vnum as u16)) {
            fmt.push_zone_slot(&mut mb, shop.keeper_vnum as i64);
        } else {
            push_i64(&mut mb, -1);
        }
        mb.push(b'\n');
        push_i64(&mut mb, shop.with_who as i64);
        mb.push(b'\n');
        parse_tab(&mut mb);
        out.extend_from_slice(&mb);

        // Rooms: the walk stops at the first NOWHERE (65535) entry —
        // reachable
        // mid-list only through exotic wrapped vnums, but mirrored anyway.
        for &rv in shop.in_rooms.iter().take_while(|&&rv| rv != 65535) {
            if !fmt.in_zone(rv as i64) {
                continue; // same rule as the products
            }
            fmt.push_vnum(&mut out, rv as i64);
            out.push(b'\n');
        }
        out.extend_from_slice(b"-1\n");

        push_i64(&mut out, shop.open1 as i64);
        out.push(b'\n');
        push_i64(&mut out, shop.close1 as i64);
        out.push(b'\n');
        push_i64(&mut out, shop.open2 as i64);
        out.push(b'\n');
        push_i64(&mut out, shop.close2 as i64);
        out.push(b'\n');
    }

    out.extend_from_slice(b"$~\n");
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::{Shop, ShopBuyData, World, Zone};
    use crate::parse;

    #[test]
    fn empty_zone_writes_header_and_terminator() {
        let mut w = World::default();
        w.zones.push(Zone { number: 100, bot: 10000, top: 10099, ..Default::default() });
        assert_eq!(write_file(&w, 0), b"CircleMUD v3.0 Shop File~\n$~\n");
    }

    #[test]
    fn defaults_tabs_and_missing_keeper() {
        let mut w = World::default();
        w.zones.push(Zone { number: 0, bot: 0, top: 99, ..Default::default() });
        w.shops.push(Shop {
            vnum: 5,
            producing: vec![],
            profit_buy: 1.0,
            profit_sell: 0.15,
            type_list: vec![ShopBuyData { type_: 5, keywords: Some(b"sword".to_vec()) }],
            no_such_item1: Some(b"%s no \tRitem\tn here".to_vec()),
            no_such_item2: None,
            missing_cash1: None,
            missing_cash2: None,
            do_not_buy: None,
            message_buy: None,
            message_sell: None,
            temper1: -1,
            bitvector: 6,
            keeper_vnum: 1234, // not in mob_map => -1
            with_who: 0,
            in_rooms: vec![3033],
            open1: 0,
            close1: 28,
            open2: 0,
            close2: 0,
        });
        let got = write_file(&w, 0);
        let want: &[u8] = b"CircleMUD v3.0 Shop File~\n#5~\n-1\n1.00\n0.15\n5sword\n-1\n\
            %s no @Ritem@n here~\n%s Ke?!~\n%s Ke?!~\n%s Ke?!~\n%s Ke?!~\n\
            %s Ke?! %d?~\n%s Ke?! %d?~\n-1\n6\n-1\n0\n3033\n-1\n0\n28\n0\n0\n$~\n";
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(want)
        );
    }

    /// Products, keeper and rooms are all vnum columns — and this is the
    /// one format that drops rather than marks: out-of-zone products and
    /// rooms are dropped, the keeper is forced
    /// into the zone's numbering. Nothing here becomes ZZ.
    #[test]
    fn export_drops_out_of_zone_shop_entries_instead_of_marking_them() {
        let mut w = World::default();
        w.zones.push(Zone { number: 30, bot: 3000, top: 3099, ..Default::default() });
        w.mob_map.insert(3005, 0);
        w.mob_map.insert(182, 1); // a keeper from someone else's zone
        w.shops.push(Shop {
            vnum: 3010,
            producing: vec![3006, 1204],
            profit_buy: 1.0,
            profit_sell: 0.15,
            type_list: vec![ShopBuyData { type_: 5, keywords: None }],
            no_such_item1: None,
            no_such_item2: None,
            missing_cash1: None,
            missing_cash2: None,
            do_not_buy: None,
            message_buy: None,
            message_sell: None,
            temper1: -1,
            bitvector: 0,
            keeper_vnum: 3005,
            with_who: 0,
            in_rooms: vec![3009, 1200],
            open1: 0,
            close1: 28,
            open2: 0,
            close2: 0,
        });
        let qq = String::from_utf8(write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))).unwrap();
        assert!(!qq.contains("ZZ"), "{qq}");
        assert!(qq.contains("#QQ10~\nQQ06\n-1\n"), "{qq}"); // obj 1204 dropped
        assert!(qq.contains("\n-1\n0\nQQ05\n0\nQQ09\n-1\n"), "{qq}"); // room 1200 too

        let re = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        let re = String::from_utf8(re).unwrap();
        assert!(!re.contains("ZZ"), "{re}");
        assert!(re.contains("#40010~\n40006\n-1\n"), "{re}");
        assert!(re.contains("\n-1\n0\n40005\n0\n40009\n-1\n"), "{re}");

        // A keeper from another zone can't be dropped, so it goes out in
        // this zone's numbering — the %100 rule, and its silent
        // mistarget.
        w.shops[0].keeper_vnum = 182;
        let qq = String::from_utf8(write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))).unwrap();
        assert!(qq.contains("\n-1\n0\nQQ82\n0\n"), "{qq}");
        let re = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        assert!(String::from_utf8_lossy(&re).contains("\n-1\n0\n40082\n0\n"));
    }

    #[test]
    fn parse_tab_leaves_double_tab_alone() {
        let mut s = b"a\tb\t\tc\t".to_vec();
        parse_tab(&mut s);
        assert_eq!(s, b"a@b\t\tc@");
    }

    #[test]
    fn profit_formatting_matches_c_printf() {
        let mut out = Vec::new();
        for (v, want) in
            [(1.15f32, "1.15\n"), (0.15, "0.15\n"), (1.0, "1.00\n"), (0.125, "0.12\n")]
        {
            out.clear();
            push_profit(&mut out, v);
            assert_eq!(out, want.as_bytes());
        }
    }

    // ---- golden round-trips ----

    /// Fill a vnum map from the "#<digits>" record headers of every file in
    /// a world subdirectory — enough real_object/real_mobile coverage for
    /// shops, which produce objects across zone boundaries (e.g. 274.shp
    /// sells zone-30 food). The full boot uses the real mob/obj parsers.
    fn scan_headers(map: &mut std::collections::HashMap<u16, u16>, dir: PathBuf) {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e == "mini") {
                continue; // index files
            }
            let Ok(data) = std::fs::read(&path) else { continue };
            for line in data.split(|&b| b == b'\n') {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if let Some(rest) = line.strip_prefix(b"#") {
                    if !rest.is_empty() && rest.iter().all(u8::is_ascii_digit) {
                        let v = crate::lex::atol(rest);
                        if v < 99999 {
                            let next = map.len() as u16;
                            map.insert(v as u16, next);
                        }
                    }
                }
            }
        }
    }

}
