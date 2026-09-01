// One "T <vnum>" line per attached prototype trigger.
//!
//! Shapes the format requires: extra/wear/perm flags go out as sprintascii
//! letters ("0" when empty), the third numeric line is always the 5-field
//! form, T lines precede E blocks which precede A blocks regardless of the
//! input order, extra descriptions are written in memory order (the loader
//! prepends, so a load/save cycle reverses them — deliberate),
//! A records are emitted only for slots with modifier != 0, only the action
//! description and E-block descriptions are strip_cr'd, the header block
//! gets convert_from_tabs but E blocks do NOT (their '\t' bytes are written
//! raw), and the file ends "$~\n".

use mud_data::flags::ITEM_CONTAINER;

use crate::model::{ObjProto, World};
use crate::write::{push_int, sprintascii, VnumFmt};

/// Header-block string: parse_tab ('\t' -> '@' unless doubled) exactly as
/// convert_from_tabs does over the assembled "#vnum + four strings" buffer.
fn push_header_str(out: &mut Vec<u8>, s: &[u8]) {
    let mut tmp = s.to_vec();
    let mut i = 0;
    while i < tmp.len() {
        if tmp[i] == b'\t' {
            if tmp.get(i + 1) == Some(&b'\t') {
                i += 2;
                continue;
            }
            tmp[i] = b'@';
        }
        i += 1;
    }
    out.extend_from_slice(&tmp);
    out.extend_from_slice(b"~\n");
}

/// `(s && *s) ? s: "undefined"` for the name/short/long header strings.
fn nonempty_or_undefined(s: &Option<Vec<u8>>) -> &[u8] {
    match s {
        Some(v) if !v.is_empty() => v,
        _ => b"undefined",
    }
}

fn strip_cr(s: &[u8]) -> Vec<u8> {
    s.iter().copied().filter(|&b| b != b'\r').collect()
}

/// Every existing object in the zone's vnum range,
/// ascending, then "$~\n".
pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(zone) = world.zones.get(zone_rnum as usize) else {
        return out; // C logs an invalid-zone SYSERR and writes nothing.
    };
    for vnum in zone.bot..=zone.top {
        if let Some(&rnum) = world.obj_map.get(&vnum) {
            write_object_record(&mut out, &world.obj_protos[rnum as usize], fmt);
        }
    }
    out.extend_from_slice(b"$~\n");
    out
}

fn write_object_record(out: &mut Vec<u8>, obj: &ObjProto, fmt: VnumFmt) {
    // Header: "#%d\n%s~\n%s~\n%s~\n%s~\n" through convert_from_tabs; only
    // the action description is strip_cr'd, and it may be empty (bare ~).
    out.push(b'#');
    fmt.push_vnum(out, i64::from(obj.vnum));
    out.push(b'\n');
    push_header_str(out, nonempty_or_undefined(&obj.name));
    push_header_str(out, nonempty_or_undefined(&obj.short_description));
    push_header_str(out, nonempty_or_undefined(&obj.description));
    let action = match &obj.action_description {
        Some(s) => strip_cr(s),
        None => Vec::new(),
    };
    push_header_str(out, &action);

    // "%d %s x12\n" — type then extra/wear/perm flag words as letters.
    push_int(out, i64::from(obj.type_flag));
    for flags in [&obj.extra_flags, &obj.wear_flags, &obj.perm_affects] {
        for &word in flags.iter() {
            out.push(b' ');
            out.extend_from_slice(&sprintascii(word));
        }
    }
    out.push(b'\n');

    // "%d %d %d %d\n". A container's third value is its key's vnum
    // (val 2 == -1 for "no key"), the one obj value an export rewrites.
    let key_slot = (obj.type_flag == ITEM_CONTAINER).then_some(2);
    for (i, &v) in obj.values.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        if key_slot == Some(i) && v != -1 {
            fmt.push_vnum(out, i64::from(v));
        } else {
            push_int(out, i64::from(v));
        }
    }
    out.push(b'\n');

    // "%d %d %d %d %d\n"
    push_int(out, i64::from(obj.weight));
    out.push(b' ');
    push_int(out, i64::from(obj.cost));
    out.push(b' ');
    push_int(out, i64::from(obj.cost_per_day));
    out.push(b' ');
    push_int(out, i64::from(obj.level));
    out.push(b' ');
    push_int(out, i64::from(obj.timer));
    out.push(b'\n');

    // One "T <vnum>" line per attached prototype trigger.
    for &t in &obj.proto_script {
        out.extend_from_slice(b"T ");
        fmt.push_vnum(out, i64::from(t));
        out.push(b'\n');
    }

    // Extra descriptions, in list order; corrupt entries (empty keyword or
    // description) are skipped with a mudlog. No tab conversion here —
    // These are written outside the tab conversion.
    for ex in &obj.ex_descriptions {
        let (Some(keyword), Some(description)) = (&ex.keyword, &ex.description) else {
            continue;
        };
        if keyword.is_empty() || description.is_empty() {
            continue;
        }
        out.extend_from_slice(b"E\n");
        out.extend_from_slice(keyword);
        out.extend_from_slice(b"~\n");
        out.extend_from_slice(&strip_cr(description));
        out.extend_from_slice(b"~\n");
    }

    // Affects: only slots whose modifier is non-zero.
    for aff in &obj.affected {
        if aff.modifier != 0 {
            out.extend_from_slice(b"A\n");
            push_int(out, i64::from(aff.location));
            out.push(b' ');
            push_int(out, i64::from(aff.modifier));
            out.push(b'\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExtraDesc, ObjAffect, Zone};
    use crate::parse;
    use std::path::PathBuf;

    fn zone(number: u16, bot: u16, top: u16) -> Zone {
        Zone { number, bot, top, ..Default::default() }
    }

    /// An object's vnum sites are its header, its T lines and — for a
    /// container only — value 2, which holds the vnum of its key.
    #[test]
    fn export_marks_the_header_the_triggers_and_a_container_key() {
        let mut w = World::default();
        w.zones.push(zone(30, 3000, 3099));
        let proto = |vnum, type_flag| ObjProto {
            vnum,
            type_flag,
            values: [10, 0, 3015, 0],
            proto_script: vec![3017],
            weight: 5,
            cost: 100,
            cost_per_day: 10,
            level: 3,
            timer: 42,
            ..Default::default()
        };
        w.obj_protos.push(proto(3010, ITEM_CONTAINER));
        w.obj_map.insert(3010, 0);
        w.obj_protos.push(proto(3011, ITEM_CONTAINER));
        w.obj_protos[1].values[2] = -1; // "no key"
        w.obj_map.insert(3011, 1);
        // A weapon's value 2 is a dice size, not a vnum.
        w.obj_protos.push(proto(3012, mud_data::flags::ITEM_WEAPON));
        w.obj_map.insert(3012, 2);

        let qq = String::from_utf8(write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))).unwrap();
        assert!(qq.contains("#QQ10\n"), "{qq}");
        assert!(qq.contains("\n10 0 QQ15 0\n"), "{qq}");
        assert!(qq.contains("\n10 0 -1 0\n"), "{qq}");
        assert!(qq.contains("\n10 0 3015 0\n"), "{qq}");
        assert!(qq.contains("\nT QQ17\n"), "{qq}");
        // An export emits the full five-field line, timer included.
        assert_eq!(qq.matches("\n5 100 10 3 42\n").count(), 3, "{qq}");

        let re = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        let re = String::from_utf8(re).unwrap();
        assert!(re.contains("#40010\n"), "{re}");
        assert!(re.contains("\n10 0 40015 0\n"), "{re}");
        assert!(re.contains("\nT 40017\n"), "{re}");
    }

    #[test]
    fn record_block_order_and_flag_letters() {
        let mut world = World::default();
        world.zones.push(zone(1, 42, 42));
        let obj = ObjProto {
            vnum: 42,
            name: Some(b"sword long".to_vec()),
            short_description: Some(b"a long sword".to_vec()),
            description: Some(b"A long sword is here.".to_vec()),
            action_description: None,
            type_flag: 5,
            extra_flags: [(1 << 2) | (1 << 3) | (1 << 16), 0, 0, 0],
            wear_flags: [(1 << 0) | (1 << 13), 0, 0, 0],
            values: [0, 2, 3, 11],
            weight: 5,
            cost: 100,
            cost_per_day: 50,
            // A load/save cycle keeps memory order: entry 0 writes first.
            ex_descriptions: vec![
                ExtraDesc {
                    keyword: Some(b"second".to_vec()),
                    description: Some(b"Second text.\r\n".to_vec()),
                },
                ExtraDesc { keyword: Some(b"skipme".to_vec()), description: None },
                ExtraDesc {
                    keyword: Some(b"first".to_vec()),
                    description: Some(b"First text.\r\n".to_vec()),
                },
            ],
            affected: [
                ObjAffect { location: 18, modifier: 2 },
                ObjAffect { location: 7, modifier: 0 }, // dropped: modifier 0
                ObjAffect { location: 1, modifier: -1 },
                ObjAffect::default(),
                ObjAffect::default(),
                ObjAffect::default(),
            ],
            proto_script: vec![3014, 3015],
            ..Default::default()
        };
        world.obj_map.insert(42, 0);
        world.obj_protos.push(obj);
        let out = write_file(&world, 0);
        let expect = b"#42\n\
            sword long~\n\
            a long sword~\n\
            A long sword is here.~\n\
            ~\n\
            5 cdq 0 0 0 an 0 0 0 0 0 0 0\n\
            0 2 3 11\n\
            5 100 50 0 0\n\
            T 3014\n\
            T 3015\n\
            E\n\
            second~\n\
            Second text.\n~\n\
            E\n\
            first~\n\
            First text.\n~\n\
            A\n\
            18 2\n\
            A\n\
            1 -1\n\
            $~\n";
        assert_eq!(out, expect.as_slice());
    }

    #[test]
    fn empty_strings_fall_back_and_tabs_stay_raw_in_e_blocks() {
        let mut world = World::default();
        world.zones.push(zone(1, 7, 7));
        let obj = ObjProto {
            vnum: 7,
            name: None,
            short_description: Some(Vec::new()),
            description: Some(b"\tgA sign.\tn".to_vec()),
            ex_descriptions: vec![ExtraDesc {
                keyword: Some(b"sign".to_vec()),
                description: Some(b"Reads \tRstop\tn.\r\n".to_vec()),
            }],
            ..Default::default()
        };
        world.obj_map.insert(7, 0);
        world.obj_protos.push(obj);
        let out = write_file(&world, 0);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("#7\nundefined~\nundefined~\n@gA sign.@n~\n~\n"), "{text}");
        // E-block bytes skip convert_from_tabs: the tabs are written raw.
        assert!(text.contains("E\nsign~\nReads \tRstop\tn.\n~\n"), "{text}");
    }
}
