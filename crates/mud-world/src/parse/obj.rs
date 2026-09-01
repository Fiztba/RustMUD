//! .obj parser: the record loop, parse_object, and the object trigger
//! reader. Objects have no end-of-record marker: parse_object consumes
//! lines until the next "#"/"$" line and hands that line back to the record
//! loop, which never calls get_line again after the first record.
//!
//! Stock config assumed (bitwarning FALSE): legacy 3/4-token first numeric
//! lines take the 128-bit conversion path. The 3-token form converts
//! an *uninitialized* f3 buffer into the perm-affect flags (
//! — retval 3 never wrote f3); the deterministic port stores 0.

use mud_data::types::Idx;

use crate::lex::{asciiflag_conv, Reader};
use crate::model::{ExtraDesc, ObjAffect, ObjProto, World};
use crate::parse::mob::{asciiflag_conv_aff, lower_leading_article, parse_hash_vnum, Scanf};

/// The .obj record loop.
pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let mut r = Reader::new(data);
    let mut nr: i32 = -1;
    let mut pending: Option<Vec<u8>> = None;
    loop {
        let line = match pending.take() {
            Some(l) => l,
            None => match r.get_line() {
                Some(l) => l,
                None if nr == -1 => {
                    return Err(format!("SYSERR: obj file {filename} is empty!"));
                }
                None => {
                    return Err(format!(
                        "SYSERR: Format error in {filename} after obj #{nr}\n\
                         ...expecting a new obj, but file ended!\n\
                         (maybe the file is not terminated with '$'?)"
                    ));
                }
            },
        };
        if line.first() == Some(&b'$') {
            return Ok(());
        }
        if line.first() == Some(&b'#') {
            let last = nr;
            nr = parse_hash_vnum(&line)
                .ok_or_else(|| format!("SYSERR: Format error after obj #{last}"))?;
            if nr >= 99999 {
                return Ok(());
            }
            pending = Some(parse_object(world, &mut r, nr)?);
        } else {
            return Err(format!(
                "SYSERR: Format error in obj file {filename} near obj #{nr}\n\
                 SYSERR: ... offending line: '{}'",
                String::from_utf8_lossy(&line)
            ));
        }
    }
}

/// Returns the "#"/"$" line that ended the record.
fn parse_object(world: &mut World, r: &mut Reader, nr: i32) -> Result<Vec<u8>, String> {
    let err = format!("object #{nr}");
    let mut obj = ObjProto { vnum: nr as Idx, ..Default::default() };

    /* String data */
    obj.name = r.fread_string(&err)?;
    if obj.name.is_none() {
        return Err(format!("SYSERR: Null obj name or format error at or near {err}"));
    }
    obj.short_description = r.fread_string(&err)?;
    lower_leading_article(&mut obj.short_description);
    obj.description = r.fread_string(&err)?;
    // CAP: the room description's first byte is force-uppercased.
    if let Some(d) = &mut obj.description {
        if let Some(b) = d.first_mut() {
            *b = b.to_ascii_uppercase();
        }
    }
    obj.action_description = r.fread_string(&err)?;

    /* First numeric line: sscanf " %d %s x12" accepting 13, 4 or 3. */
    let line = r
        .get_line()
        .ok_or_else(|| format!("SYSERR: Expecting first numeric line of {err}, but file ended!"))?;
    let mut sc = Scanf::new(&line);
    let type_ = sc.int();
    let mut words: Vec<&[u8]> = Vec::new();
    if type_.is_some() {
        while words.len() < 12 {
            match sc.word() {
                Some(w) => words.push(w),
                None => break,
            }
        }
    }
    let retval = usize::from(type_.is_some()) + words.len();
    match retval {
        13 => {
            for k in 0..4 {
                obj.extra_flags[k] = asciiflag_conv(words[k]);
                obj.wear_flags[k] = asciiflag_conv(words[4 + k]);
                obj.perm_affects[k] = asciiflag_conv(words[8 + k]);
            }
        }
        3 | 4 => {
            // Legacy "type extra wear [perm]" conversion.
            obj.extra_flags[0] = asciiflag_conv(words[0]);
            obj.wear_flags[0] = asciiflag_conv(words[1]);
            obj.perm_affects[0] =
                if retval == 4 { asciiflag_conv_aff(words[2]) } else { 0 };
        }
        n => {
            return Err(format!(
                "SYSERR: Format error in first numeric line (expecting 13 args, got {n}), {err}"
            ));
        }
    }
    obj.type_flag = i32::from(type_.unwrap() as i8); // byte type_flag

    /* Second numeric line: exactly 4 values. */
    let line = r
        .get_line()
        .ok_or_else(|| format!("SYSERR: Expecting second numeric line of {err}, but file ended!"))?;
    let mut sc = Scanf::new(&line);
    let mut vals = [0i32; 4];
    let mut n = 0;
    while n < 4 {
        match sc.int() {
            Some(v) => {
                vals[n] = v;
                n += 1;
            }
            None => break,
        }
    }
    if n != 4 {
        return Err(format!(
            "SYSERR: Format error in second numeric line (expecting 4 args, got {n}), {err}"
        ));
    }
    obj.values = vals;

    /* Third numeric line: 5 values; 3 or 4 tolerated (rest default 0). */
    let line = r
        .get_line()
        .ok_or_else(|| format!("SYSERR: Expecting third numeric line of {err}, but file ended!"))?;
    let mut sc = Scanf::new(&line);
    let mut t = [0i32; 5];
    let mut n = 0;
    while n < 5 {
        match sc.int() {
            Some(v) => {
                t[n] = v;
                n += 1;
            }
            None => break,
        }
    }
    if !matches!(n, 3 | 4 | 5) {
        return Err(format!(
            "SYSERR: Format error in third numeric line (expecting 5 args, got {n}), {err}"
        ));
    }
    obj.weight = t[0];
    obj.cost = t[1];
    obj.cost_per_day = t[2];
    obj.level = t[3];
    obj.timer = t[4];

    /* Drink containers and fountains: weight must cover the liquid
     *  only when the item is takeable. */
    if (obj.type_flag == 17 || obj.type_flag == 23) // ITEM_DRINKCON / ITEM_FOUNTAIN
        && obj.weight < obj.values[1]
        && obj.wear_flags[0] & (1 << 0) != 0
    {
        obj.weight = obj.values[1] + 5;
    }

    /* Trailing E / A / T blocks until the next "#" or "$" line. */
    let err2 = format!(
        "{err}, after numeric constants\n...expecting 'E', 'A', '$', or next object number"
    );
    let mut j = 0usize;
    loop {
        let line = r
            .get_line()
            .ok_or_else(|| format!("SYSERR: Format error in {err2}"))?;
        match line.first() {
            Some(b'E') => {
                let keyword = r.fread_string(&err2)?;
                let description = r.fread_string(&err2)?;
                // Prepended, so the list is in reverse file order.
                obj.ex_descriptions.insert(0, ExtraDesc { keyword, description });
            }
            Some(b'A') => {
                if j >= 6 {
                    return Err(format!("SYSERR: Too many A fields (6 max), {err2}"));
                }
                let line = r.get_line().ok_or_else(|| {
                    format!(
                        "SYSERR: Format error in 'A' field, {err2}\n\
                         ...expecting 2 numeric constants but file ended!"
                    )
                })?;
                let mut sc = Scanf::new(&line);
                let (loc, modi) = match (sc.int(), sc.int()) {
                    (Some(l), Some(m)) => (l, m),
                    _ => {
                        return Err(format!(
                            "SYSERR: Format error in 'A' field, {err2}\n\
                             ...expecting 2 numeric arguments\n\
                             ...offending line: '{}'",
                            String::from_utf8_lossy(&line)
                        ));
                    }
                };
                // struct obj_affected_type: byte location, sbyte modifier.
                obj.affected[j] = ObjAffect {
                    location: i32::from(loc as i8),
                    modifier: i32::from(modi as i8),
                };
                j += 1;
            }
            Some(b'T') => dg_obj_trigger(&line, &mut obj),
            Some(b'$') | Some(b'#') => {
                // check_object runs here. Its checks are log-only except
                // this one: `item_types[]` is indexed by the type directly —
                // olist, stat and the two spell displays all do it — and
                // nothing between the file and those readers bounds it. So the
                // bad type is corrected rather than only reported, which is
                // also what parse_object already does for a drink container
                // lighter than its own contents.
                if obj.type_flag < 0 || obj.type_flag >= mud_data::flags::NUM_ITEM_TYPES as i32 {
                    world.load_warnings.push(format!(
                        "SYSERR: Object #{} ({}) has unknown type {}, treating it as UNDEFINED.",
                        obj.vnum,
                        String::from_utf8_lossy(obj.short_description.as_deref().unwrap_or(b"")),
                        obj.type_flag
                    ));
                    obj.type_flag = mud_data::flags::ITEM_UNDEFINED;
                }
                let rnum = world.obj_protos.len() as Idx;
                world.obj_map.insert(obj.vnum, rnum);
                world.obj_protos.push(obj);
                return Ok(line);
            }
            Some(&c) => {
                return Err(format!("SYSERR: Format error in ({}): {err2}", c as char));
            }
            None => unreachable!("get_line never yields an empty line"),
        }
    }
}

/// Sscanf "%s %d" on the already-read line;
/// scan failures are logged and dropped. As with mobs, pruning triggers
/// whose vnum does not exist is deferred to boot (see
/// parse::mob::dg_read_trigger).
fn dg_obj_trigger(line: &[u8], obj: &mut ObjProto) {
    let mut sc = Scanf::new(line);
    // %7s into an 8-byte buffer: the width bounds the store, not the
    // scan, so a longer first token leaves the cursor inside it and the
    // vnum conversion reads the remainder. Capping here keeps a
    // malformed T line landing where it lands everywhere else.
    let Some(_junk) = sc.word_cap(7) else { return };
    let Some(vnum) = sc.int() else { return };
    obj.proto_script.push(vnum as Idx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> World {
        let mut world = World::default();
        parse_file(&mut world, data, "test.obj").expect("parse");
        world
    }

    const HEAD: &[u8] = b"#100\nkey dull~\nA Key of dull metal~\na key lies here.~\n~\n";

    #[test]
    fn strings_flags_and_values() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"18 cdq 0 0 0 ao 0 0 0 0 0 0 0\n1 2 3 4\n5 6 7 8 9\n$~\n");
        let w = parse(&data);
        let o = &w.obj_protos[0];
        assert_eq!(o.vnum, 100);
        assert_eq!(w.real_object(100), Some(0));
        // a/an/the lowercase quirk on the short description...
        assert_eq!(o.short_description.as_deref(), Some(&b"a Key of dull metal"[..]));
        //..and CAP on the room description.
        assert_eq!(o.description.as_deref(), Some(&b"A key lies here."[..]));
        assert_eq!(o.action_description, None);
        assert_eq!(o.type_flag, 18);
        assert_eq!(o.extra_flags[0], (1 << 2) | (1 << 3) | (1 << 16)); // c|d|q
        assert_eq!(o.wear_flags[0], (1 << 0) | (1 << 14)); // a|o
        assert_eq!(o.perm_affects, [0; 4]);
        assert_eq!(o.values, [1, 2, 3, 4]);
        assert_eq!(
            (o.weight, o.cost, o.cost_per_day, o.level, o.timer),
            (5, 6, 7, 8, 9)
        );
    }

    #[test]
    fn third_line_three_and_four_field_tolerance() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"9 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n5 150 25\n$~\n");
        let w = parse(&data);
        let o = &w.obj_protos[0];
        assert_eq!((o.weight, o.cost, o.cost_per_day, o.level, o.timer), (5, 150, 25, 0, 0));

        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"9 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n5 150 25 12\n$~\n");
        let w = parse(&data);
        let o = &w.obj_protos[0];
        assert_eq!((o.level, o.timer), (12, 0));
    }

    #[test]
    fn legacy_three_and_four_token_first_line() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"9 ab n\n0 0 0 0\n1 1 0 0 0\n$~\n");
        let w = parse(&data);
        let o = &w.obj_protos[0];
        assert_eq!(o.type_flag, 9);
        assert_eq!(o.extra_flags, [0b11, 0, 0, 0]);
        assert_eq!(o.wear_flags, [1 << 13, 0, 0, 0]);
        assert_eq!(o.perm_affects, [0; 4]); // C reads garbage f3; we store 0

        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"9 ab n c\n0 0 0 0\n1 1 0 0 0\n$~\n");
        let w = parse(&data);
        // 4-token perm goes through asciiflag_conv_aff: 'c' -> bit 3.
        assert_eq!(w.obj_protos[0].perm_affects[0], 1 << 3);
    }

    #[test]
    fn drinkcon_weight_raised_only_when_takeable_and_light() {
        let body = |wear: &str, weight: &str| {
            let mut d = Vec::new();
            d.extend_from_slice(HEAD);
            d.extend_from_slice(
                format!("17 0 0 0 0 {wear} 0 0 0 0 0 0 0\n8 20 1 0\n{weight} 20 8 0 0\n$~\n")
                    .as_bytes(),
            );
            d
        };
        // weight 10 < value[1] 20, takeable -> raised to 20 + 5.
        let w = parse(&body("a", "10"));
        assert_eq!(w.obj_protos[0].weight, 25);
        // Not takeable: untouched.
        let w = parse(&body("0", "10"));
        assert_eq!(w.obj_protos[0].weight, 10);
        // Heavy enough: untouched.
        let w = parse(&body("a", "20"));
        assert_eq!(w.obj_protos[0].weight, 20);
    }

    /// The T line's first token is scanned with a width, so an over-long one
    /// is not consumed whole: the cursor stops inside it and the vnum comes
    /// off the remainder. Three outcomes, all of them the scan's and none of
    /// them a refusal of the record.
    #[test]
    fn obj_trigger_line_scans_its_first_token_with_a_width() {
        let with_t = |t: &str| {
            let mut data = Vec::new();
            data.extend_from_slice(HEAD);
            data.extend_from_slice(b"18 a 0 0 0 a 0 0 0 0 0 0 0
1 2 3 4
5 6 7 8 9
");
            data.extend_from_slice(t.as_bytes());
            data.extend_from_slice(b"$~
");
            let w = parse(&data);
            w.obj_protos[0].proto_script.clone()
        };

        // The ordinary line: one-character token, the vnum after it.
        assert_eq!(with_t("T 3017
"), vec![3017]);

        // Eight characters, so the width stops one short and the remainder
        // starts with a letter -- no digits, no vnum, the line is dropped.
        assert_eq!(with_t("TTTTTTTX 3017
"), Vec::<u16>::new());

        // The same shape with digits left over: those are what the vnum
        // conversion sees, so 99 attaches and 3017 never does.
        assert_eq!(with_t("TTTTTTT99 3017
"), vec![99]);
    }

    #[test]
    fn extra_descs_prepend_and_affects_fill_slots() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"5 0 0 0 0 an 0 0 0 0 0 0 0\n0 0 2 3\n1 1 0 0 0\n");
        data.extend_from_slice(b"A\n18 2\nE\nfirst~\nFirst text.\n~\nA\n1 -3\nE\nsecond~\nSecond.\n~\n");
        data.extend_from_slice(b"T 3014\n$~\n");
        let w = parse(&data);
        let o = &w.obj_protos[0];
        // Prepended: memory order is the reverse of file order.
        assert_eq!(o.ex_descriptions[0].keyword.as_deref(), Some(&b"second"[..]));
        assert_eq!(o.ex_descriptions[1].keyword.as_deref(), Some(&b"first"[..]));
        assert_eq!(o.ex_descriptions[1].description.as_deref(), Some(&b"First text.\r\n"[..]));
        assert_eq!(o.affected[0].location, 18);
        assert_eq!(o.affected[0].modifier, 2);
        assert_eq!(o.affected[1].location, 1);
        assert_eq!(o.affected[1].modifier, -3);
        assert_eq!(o.affected[2].location, 0);
        assert_eq!(o.proto_script, vec![3014]);
    }

    #[test]
    fn seventh_a_field_is_fatal() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"5 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n1 1 0 0 0\n");
        for _ in 0..7 {
            data.extend_from_slice(b"A\n1 1\n");
        }
        data.extend_from_slice(b"$~\n");
        let mut w = World::default();
        let e = parse_file(&mut w, &data, "t.obj").unwrap_err();
        assert!(e.contains("Too many A fields"), "{e}");
    }

    #[test]
    fn records_chain_through_returned_hash_line() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"5 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n1 1 0 0 0\n");
        data.extend_from_slice(b"#101\nball~\na ball~\nA ball is here.~\n~\n");
        data.extend_from_slice(b"13 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n1 1 0 0 0\n$~\n");
        let w = parse(&data);
        assert_eq!(w.obj_protos.len(), 2);
        assert_eq!(w.real_object(101), Some(1));
    }

    #[test]
    fn unknown_block_letter_is_fatal_and_null_name_is_fatal() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"5 0 0 0 0 a 0 0 0 0 0 0 0\n0 0 0 0\n1 1 0 0 0\nX\n$~\n");
        let mut w = World::default();
        let e = parse_file(&mut w, &data, "t.obj").unwrap_err();
        assert!(e.contains("Format error in (X)"), "{e}");

        let mut w = World::default();
        let e = parse_file(&mut w, b"#1\n~\nshort~\nlong~\n~\n", "t.obj").unwrap_err();
        assert!(e.contains("Null obj name"), "{e}");
    }

    #[test]
    fn bad_first_numeric_line_counts() {
        let mut data = Vec::new();
        data.extend_from_slice(HEAD);
        data.extend_from_slice(b"5 a b c d e\n0 0 0 0\n1 1 0 0 0\n$~\n");
        let mut w = World::default();
        let e = parse_file(&mut w, &data, "t.obj").unwrap_err();
        assert!(e.contains("expecting 13 args, got 6"), "{e}");
    }
}
