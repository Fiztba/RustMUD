//! .shp parser: boot_the_shops and its helpers — read_line, read_list,
//! read_type_list, read_shop_message, add_to_shop_list, end_read_list.
//!
//! The file is a stream of fread_string chunks: "#<num>~" starts a shop,
//! "$..." ends the file, a chunk containing "v3.0" anywhere switches on the
//! v3 list format for the REST of the file, any other chunk is ignored.
//! v3 lists are -1-terminated (any negative terminates); the old format
//! reads exactly MAX_PROD/MAX_TRADE/1 lines. Producing entries are pushed
//! through real_object at load — nonexistent objects vanish — while rooms
//! and buy-types stay raw. The seven keeper messages are printf-validated
//! and become None on any violation.

use crate::lex::Reader;
use crate::model::{Shop, ShopBuyData, World};

/// The buy-type name table. The list is
/// scanned in order with a case-insensitive PREFIX match, so e.g. a line
/// "WANDS of woe" matches "WAND" and keeps "S of woe" (trimmed) as keyword.
const ITEM_TYPES: &[&[u8]] = &[
    b"UNDEFINED",
    b"LIGHT",
    b"SCROLL",
    b"WAND",
    b"STAFF",
    b"WEAPON",
    b"FURNITURE",
    b"FREE",
    b"TREASURE",
    b"ARMOR",
    b"POTION",
    b"WORN",
    b"OTHER",
    b"TRASH",
    b"FREE2",
    b"CONTAINER",
    b"NOTE",
    b"LIQ CONTAINER",
    b"KEY",
    b"FOOD",
    b"MONEY",
    b"PEN",
    b"BOAT",
    b"FOUNTAIN",
];

const MAX_PROD: usize = 5; // old-format producing list length
const MAX_TRADE: usize = 5; // old-format buy-type list length
const MAX_SHOP_OBJ: usize = 100; // soft cap on kept entries
const MAX_STRING_LENGTH: usize = 49152; // read buffer

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Produce,
    Trade,
    Room,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// An integer: leading whitespace, optional sign, then digits.
fn scan_long(s: &[u8]) -> Option<i64> {
    let mut i = 0;
    while s.get(i).copied().is_some_and(is_ws) {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut v: i64 = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        i += 1;
    }
    if i == start {
        return None;
    }
    Some(if neg { v.wrapping_neg() } else { v })
}

/// A decimal float: `[+-]?digits[.digits][e[+-]digits]`. An incomplete
/// exponent is not consumed, so the mantissa alone matches. Hex floats,
/// `inf` and `nan` are not part of the shop-file grammar.
fn scan_float(s: &[u8]) -> Option<f32> {
    let mut i = 0;
    while s.get(i).copied().is_some_and(is_ws) {
        i += 1;
    }
    let start = i;
    if matches!(s.get(i), Some(b'+') | Some(b'-')) {
        i += 1;
    }
    let int_at = i;
    while s.get(i).is_some_and(|b| b.is_ascii_digit()) {
        i += 1;
    }
    let int_digits = i - int_at;
    let mut frac_digits = 0;
    if s.get(i) == Some(&b'.') {
        i += 1;
        let frac_at = i;
        while s.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
        frac_digits = i - frac_at;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    let mut end = i;
    if matches!(s.get(i), Some(b'e') | Some(b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+') | Some(b'-')) {
            j += 1;
        }
        let exp_at = j;
        while s.get(j).is_some_and(|b| b.is_ascii_digit()) {
            j += 1;
        }
        if j > exp_at {
            end = j;
        }
    }
    std::str::from_utf8(&s[start..end]).ok()?.parse::<f32>().ok()
}

/// Read one line and scan it. Either failing is fatal.
fn read_line_i64(r: &mut Reader, ctx: &str) -> Result<i64, String> {
    let line = r.get_line().ok_or_else(|| format!("Error in {ctx}: unexpected EOF"))?;
    scan_long(&line).ok_or_else(|| {
        format!("Error in {ctx}, near '{}' with '%d'", String::from_utf8_lossy(&line))
    })
}

fn read_line_f32(r: &mut Reader, ctx: &str) -> Result<f32, String> {
    let line = r.get_line().ok_or_else(|| format!("Error in {ctx}: unexpected EOF"))?;
    scan_float(&line).ok_or_else(|| {
        format!("Error in {ctx}, near '{}' with '%f'", String::from_utf8_lossy(&line))
    })
}

/// Drop NOTHING/negative values, cap the list at
/// MAX_SHOP_OBJ, and for the producing list run real_object — the vnum is
/// truncated to u16 for the lookup and the entry vanishes when no such
/// object exists. The looked-up vnum is stored, which is what the writer
/// prints back.
fn add_to_shop_list(
    kept: &mut Vec<ShopBuyData>,
    kind: ListKind,
    val: i32,
    world: &mut World,
) {
    if val != 65535 && val >= 0 && kept.len() < MAX_SHOP_OBJ {
        let stored = match kind {
            ListKind::Produce => {
                let vnum = val as u16;
                world.obj_map.contains_key(&vnum).then_some(vnum as i32)
            }
            // Rooms are copied into a room_vnum (u16) array by the boot
            // loop; buy-types stay full ints.
            ListKind::Room => Some((val as u16) as i32),
            // A trade entry names an item type, but both readers fall through
            // to a bare number when the line does not match one of the names,
            // so a.shp file can put anything here — and `list_detailed_shop`
            // and sedit's trade menu then index `ITEM_TYPES` with it. Drop the
            // entry rather than keep a type nothing can be, which is what the
            // Produce arm above already does with a vnum it cannot resolve.
            ListKind::Trade if val >= mud_data::flags::NUM_ITEM_TYPES as i32 => {
                world.load_warnings.push(format!(
                    "SYSERR: Shop file lists unknown item type {}, dropping it.",
                    val
                ));
                None
            }
            ListKind::Trade => Some(val),
        };
        if let Some(v) = stored {
            kept.push(ShopBuyData { type_: v, keywords: None });
        }
    }
}

/// V3 reads ints until a negative; old format reads
/// exactly `max` lines (negative entries consumed but skipped).
fn read_list(
    r: &mut Reader,
    world: &mut World,
    new_format: bool,
    max: usize,
    kind: ListKind,
    ctx: &str,
) -> Result<Vec<ShopBuyData>, String> {
    let mut kept = Vec::new();
    if new_format {
        loop {
            let temp = read_line_i64(r, ctx)? as i32;
            if temp < 0 {
                break;
            }
            add_to_shop_list(&mut kept, kind, temp, world);
        }
    } else {
        for _ in 0..max {
            let temp = read_line_i64(r, ctx)? as i32;
            add_to_shop_list(&mut kept, kind, temp, world);
        }
    }
    Ok(kept)
}

/// Case-insensitive prefix test.
fn ci_prefix(name: &[u8], buf: &[u8]) -> bool {
    buf.len() >= name.len()
        && name.iter().zip(buf).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// The v3 buy-type list. Reads RAW physical lines (with a
/// MAX_STRING_LENGTH buffer — no blank/'*' skipping), cuts at ';' or else
/// chops the final character (the newline), matches an item-type name
/// prefix or scans a number, and keeps whatever trimmed text remains as the
/// keyword of the LAST KEPT entry — the previous entry when this line's
/// value was dropped. Terminates on a negative parsed value. A line with no
/// digits and no type-name match is an error, as is a keyword with no entry
/// to attach to.
fn read_type_list(
    r: &mut Reader,
    world: &mut World,
    new_format: bool,
    max: usize,
    ctx: &str,
) -> Result<Vec<ShopBuyData>, String> {
    if !new_format {
        return read_list(r, world, false, max, ListKind::Trade, ctx);
    }
    let mut kept: Vec<ShopBuyData> = Vec::new();
    loop {
        // EOF here would otherwise reprocess a stale buffer forever.
        let chunk = r
            .raw_gets(MAX_STRING_LENGTH)
            .ok_or_else(|| format!("unexpected end of file reading shop type list ({ctx})"))?;
        let mut buf = chunk.to_vec();
        if let Some(p) = buf.iter().position(|&b| b == b';') {
            buf.truncate(p);
        } else {
            // *(END_OF(buf) - 1) = '\0' — unconditionally chops the last
            // character (normally the '\n' the read kept).
            buf.pop();
        }

        let mut num: i32 = -1;
        let mut matched_name = false;
        if !buf.starts_with(b"-1") {
            for (tindex, name) in ITEM_TYPES.iter().enumerate() {
                if ci_prefix(name, &buf) {
                    num = tindex as i32;
                    buf.drain(..name.len());
                    matched_name = true;
                    break;
                }
            }
        }

        let mut ptr = 0usize;
        if !matched_name {
            // num stays -1 when nothing numeric was found.
            if let Some(v) = scan_long(&buf) {
                num = v as i32;
            }
            // Scan forward to the first digit; a line with none is an
            // error, handled below.
            ptr = buf
                .iter()
                .position(|b| b.is_ascii_digit())
                .ok_or_else(|| format!("shop type-list line with no digits ({ctx})"))?;
            while buf.get(ptr).is_some_and(|b| b.is_ascii_digit()) {
                ptr += 1;
            }
        }
        while buf.get(ptr).copied().is_some_and(is_ws) {
            ptr += 1;
        }
        let mut end = buf.len();
        while end > ptr && is_ws(buf[end - 1]) {
            end -= 1;
        }

        add_to_shop_list(&mut kept, ListKind::Trade, num, world);
        if ptr < end {
            match kept.last_mut() {
                Some(last) => last.keywords = Some(buf[ptr..end].to_vec()),
                None => {
                    return Err(format!(
                        "shop type-list keyword with no preceding entry ({ctx})"
                    ));
                }
            }
        }
        if num < 0 {
            break;
        }
    }
    Ok(kept)
}

/// fread_string plus printf-format validation. Each '%' looks at the NEXT
/// byte — 's' counts toward ss; 'd' counts toward ds only for messages 5/6
/// (buy/sell) and errors if no %s came first (and errors outright in
/// messages 0-4); any other byte except '%' errors, including
/// end-of-string. The second '%' of "%%" is re-examined as its own
/// specifier rather than skipped. More than one %s or %d also errors. Any
/// error => message is NULL.
fn read_shop_message(
    mnum: i32,
    r: &mut Reader,
    ctx: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(tbuf) = r.fread_string(ctx)? else {
        return Ok(None);
    };
    let mut ss = 0;
    let mut ds = 0;
    let mut err = 0;
    for cht in 0..tbuf.len() {
        if tbuf[cht] != b'%' {
            continue;
        }
        let next = tbuf.get(cht + 1).copied().unwrap_or(0);
        if next == b's' {
            ss += 1;
        } else if next == b'd' && (mnum == 5 || mnum == 6) {
            if ss == 0 {
                err += 1;
            }
            ds += 1;
        } else if next != b'%' {
            err += 1;
        }
    }
    if ss > 1 || ds > 1 {
        err += 1;
    }
    Ok(if err > 0 { None } else { Some(tbuf) })
}

/// The .shp record loop.
pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let mut r = Reader::new(data);
    let mut new_format = false;
    let mut ctx = format!("beginning of shop file {filename}");
    loop {
        let buf = r.fread_string(&ctx)?;
        // fread_string returns nothing for an empty chunk, which would then
        // be dereferenced
        // it (*buf) and crashes.
        let Some(buf) = buf else {
            return Err(format!("empty ~-chunk near {ctx}"));
        };
        if buf.first() == Some(&b'#') {
            // A failed "#%d" match leaves the vnum unset; substitute 0.
            let temp = super::trg::scan_after_hash(&buf).unwrap_or(0);
            ctx = format!("shop #{temp} in shop file {filename}");

            let producing =
                read_list(&mut r, world, new_format, MAX_PROD, ListKind::Produce, &ctx)?;
            let profit_buy = read_line_f32(&mut r, &ctx)?;
            let profit_sell = read_line_f32(&mut r, &ctx)?;
            let type_list = read_type_list(&mut r, world, new_format, MAX_TRADE, &ctx)?;

            let no_such_item1 = read_shop_message(0, &mut r, &ctx)?;
            let no_such_item2 = read_shop_message(1, &mut r, &ctx)?;
            let do_not_buy = read_shop_message(2, &mut r, &ctx)?;
            let missing_cash1 = read_shop_message(3, &mut r, &ctx)?;
            let missing_cash2 = read_shop_message(4, &mut r, &ctx)?;
            let message_buy = read_shop_message(5, &mut r, &ctx)?;
            let message_sell = read_shop_message(6, &mut r, &ctx)?;

            let temper1 = read_line_i64(&mut r, &ctx)? as i32;
            // %ld into bitvector_t, narrowed to u32 here.
            let bitvector = read_line_i64(&mut r, &ctx)? as u32;
            // %hd: the value is stored through a 16-bit field, so it wraps
            // to short range; kept as the resulting 0..=65535 vnum (
            // immediately real_mobiles it — our writer defers that
            // lookup, producing identical output).
            let keeper_vnum = ((read_line_i64(&mut r, &ctx)? as i16) as u16) as i32;
            let with_who = read_line_i64(&mut r, &ctx)? as i32;

            let in_rooms = read_list(&mut r, world, new_format, 1, ListKind::Room, &ctx)?;

            let open1 = read_line_i64(&mut r, &ctx)? as i32;
            let close1 = read_line_i64(&mut r, &ctx)? as i32;
            let open2 = read_line_i64(&mut r, &ctx)? as i32;
            let close2 = read_line_i64(&mut r, &ctx)? as i32;

            world.shops.push(Shop {
                vnum: temp as u16,
                producing: producing.into_iter().map(|b| b.type_).collect(),
                profit_buy,
                profit_sell,
                type_list,
                no_such_item1,
                no_such_item2,
                missing_cash1,
                missing_cash2,
                do_not_buy,
                message_buy,
                message_sell,
                temper1,
                bitvector,
                keeper_vnum,
                with_who,
                in_rooms: in_rooms.into_iter().map(|b| b.type_).collect(),
                open1,
                close1,
                open2,
                close2,
            });
        } else if buf.first() == Some(&b'$') {
            return Ok(());
        } else if buf.windows(4).any(|w| w == b"v3.0") {
            // VERSION3_TAG anywhere in an otherwise-ignored chunk.
            new_format = true;
        }
        // Any other chunk is a legal free-form comment: ignored.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_objs(vnums: &[u16]) -> World {
        let mut w = World::default();
        for (i, &v) in vnums.iter().enumerate() {
            w.obj_map.insert(v, i as u16);
        }
        w
    }

    const MSGS: &[u8] = b"a~\nb~\nc~\nd~\ne~\nf~\ng~\n";

    fn one_shop(lists: &[u8], world: &mut World) -> Shop {
        let mut data = b"CircleMUD v3.0 Shop File~\n#7~\n".to_vec();
        data.extend_from_slice(lists);
        data.extend_from_slice(MSGS);
        data.extend_from_slice(b"0\n6\n1234\n2\n3033\n-1\n0\n28\n0\n0\n$~\n");
        parse_file(world, &data, "t.shp").expect("parse");
        world.shops.pop().expect("one shop")
    }

    #[test]
    fn basic_v3_record() {
        let mut w = world_with_objs(&[3050, 3051]);
        let s = one_shop(b"3050\n3051\n-1\n1.15\n0.15\n2\n3\n-1\n", &mut w);
        assert_eq!(s.vnum, 7);
        assert_eq!(s.producing, vec![3050, 3051]);
        assert!((s.profit_buy - 1.15).abs() < 1e-6);
        assert!((s.profit_sell - 0.15).abs() < 1e-6);
        assert_eq!(s.type_list.len(), 2);
        assert_eq!(s.type_list[0].type_, 2);
        assert_eq!(s.no_such_item1.as_deref(), Some(&b"a"[..]));
        assert_eq!(s.temper1, 0);
        assert_eq!(s.bitvector, 6);
        assert_eq!(s.keeper_vnum, 1234);
        assert_eq!(s.with_who, 2);
        assert_eq!(s.in_rooms, vec![3033]);
        assert_eq!((s.open1, s.close1, s.open2, s.close2), (0, 28, 0, 0));
    }

    #[test]
    fn producing_drops_unresolvable_objects() {
        // 3051 is not in obj_map — real_object fails and the entry vanishes.
        let mut w = world_with_objs(&[3050]);
        let s = one_shop(b"3050\n3051\n-1\n1.00\n1.00\n-1\n", &mut w);
        assert_eq!(s.producing, vec![3050]);
    }

    #[test]
    fn type_list_names_numbers_keywords_comments() {
        let mut w = world_with_objs(&[]);
        let s = one_shop(
            b"-1\n1.00\n1.00\nWAND\nliq container ale\n5 sword ; blades only\n12stone | rock\n-1\n",
            &mut w,
        );
        let t = &s.type_list;
        assert_eq!(t.len(), 4);
        assert_eq!((t[0].type_, t[0].keywords.as_deref()), (3, None));
        // Case-insensitive prefix name match; remainder is the keyword.
        assert_eq!((t[1].type_, t[1].keywords.as_deref()), (17, Some(&b"ale"[..])));
        // ';' comment stripped, then trailing space trimmed.
        assert_eq!((t[2].type_, t[2].keywords.as_deref()), (5, Some(&b"sword"[..])));
        // Digits glued to the keyword.
        assert_eq!((t[3].type_, t[3].keywords.as_deref()), (12, Some(&b"stone | rock"[..])));
    }

    #[test]
    fn any_negative_terminates_type_list() {
        let mut w = world_with_objs(&[]);
        let s = one_shop(b"-1\n1.00\n1.00\n2\n-5\n-1\n", &mut w);
        // -5 ends the list; the leftover "-1" line has no '~', so the first
        // message's fread_string swallows it and continues into "a~".
        assert_eq!(s.type_list.len(), 1);
        assert_eq!(s.no_such_item1.as_deref(), Some(&b"-1\r\na"[..]));
    }

    #[test]
    fn message_validation_nulls_bad_formats() {
        let mut w = world_with_objs(&[]);
        let mut data = b"CircleMUD v3.0 Shop File~\n#1~\n-1\n1.00\n1.00\n-1\n".to_vec();
        data.extend_from_slice(
            b"%s ok~\ntwo %s here %s~\n%d not allowed here~\n%s fine %n bad~\nno specifier~\n%d before %s~\n%s then %d ok~\n",
        );
        data.extend_from_slice(b"0\n0\n1\n0\n-1\n0\n0\n0\n0\n$~\n");
        parse_file(&mut w, &data, "t.shp").expect("parse");
        let s = &w.shops[0];
        assert_eq!(s.no_such_item1.as_deref(), Some(&b"%s ok"[..]));
        assert_eq!(s.no_such_item2, None); // two %s
        assert_eq!(s.do_not_buy, None); // %d outside msgs 5/6
        assert_eq!(s.missing_cash1, None); // %n invalid
        assert_eq!(s.missing_cash2.as_deref(), Some(&b"no specifier"[..]));
        assert_eq!(s.message_buy, None); // %d before %s
        assert_eq!(s.message_sell.as_deref(), Some(&b"%s then %d ok"[..]));
    }

    #[test]
    fn old_format_reads_fixed_length_lists() {
        // No v3.0 tag: exactly 5 producing lines, 5 type lines, 1 room line,
        // no -1 terminators. Negative entries consume a slot but are
        // dropped.
        let mut w = world_with_objs(&[10, 11]);
        let mut data = b"#3~\n10\n11\n-1\n-1\n-1\n1.10\n0.90\n2\n3\n-1\n-1\n-1\n".to_vec();
        data.extend_from_slice(MSGS);
        data.extend_from_slice(b"0\n0\n5\n0\n3001\n0\n28\n0\n0\n$~\n");
        parse_file(&mut w, &data, "t.shp").expect("parse");
        let s = &w.shops[0];
        assert_eq!(s.producing, vec![10, 11]);
        assert_eq!(s.type_list.len(), 2);
        assert_eq!(s.in_rooms, vec![3001]);
        assert_eq!((s.open1, s.close1), (0, 28));
    }

    #[test]
    fn ignored_chunks_and_version_tag_anywhere() {
        let mut w = world_with_objs(&[]);
        // A comment chunk mentioning v3.0 flips the format switch.
        let mut data =
            b"free comment~\nconverted to v3.0 by hand~\n#9~\n-1\n1.00\n1.00\n-1\n".to_vec();
        data.extend_from_slice(MSGS);
        data.extend_from_slice(b"0\n0\n1\n0\n-1\n0\n0\n0\n0\n$~\n");
        parse_file(&mut w, &data, "t.shp").expect("parse");
        assert_eq!(w.shops.len(), 1);
        assert!(w.shops[0].in_rooms.is_empty());
    }

    #[test]
    fn keeper_hd_wraps_to_short_range() {
        let mut w = world_with_objs(&[]);
        let mut data = b"CircleMUD v3.0 Shop File~\n#1~\n-1\n1.00\n1.00\n-1\n".to_vec();
        data.extend_from_slice(MSGS);
        data.extend_from_slice(b"0\n0\n-1\n0\n-1\n0\n0\n0\n0\n$~\n");
        parse_file(&mut w, &data, "t.shp").expect("parse");
        // %hd of "-1" stored through the unsigned 16-bit keeper field.
        assert_eq!(w.shops[0].keeper_vnum, 65535);
    }

    #[test]
    fn eof_without_dollar_is_error() {
        let mut w = world_with_objs(&[]);
        assert!(parse_file(&mut w, b"CircleMUD v3.0 Shop File~\n", "t.shp").is_err());
    }
}
