//! .qst writer: save_quests.
//!
//! Per vnum in the zone's bot..=top range with a real quest (linear
//! first-match): one record --
//! "#<vnum>", the five strings (name falls back to "Untitled" and is NOT
//! strip_cr'd; desc/info/done/quit fall back to "undefined" and ARE),
//! "type qm flags target prev next prereq" with NOBODY/NOTHING written as
//! -1, "value[0..4] returnmob quantity" (returnmob's 65535 becomes -1),
//! "gold exp obj_reward" with obj_reward RAW (an unset reward stays 65535,
//! as it is on disk), and "S". The whole record passes through
//! convert_from_tabs (parse_tab: '\t'->'@' except "\t\t" pairs). Records
//! of MAX_STRING_LENGTH or more are skipped. File tail "$~\n".

use super::{sprintascii, VnumFmt};
use crate::model::World;

const MAX_STRING_LENGTH: usize = 49152;

/// See write::shp for the mirror-of-parse_at quirk.
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

fn push_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(v.to_string().as_bytes());
}

/// A string field: fallback for NULL, optional strip_cr, then "~\n".
fn push_str(out: &mut Vec<u8>, s: &Option<Vec<u8>>, default: &[u8], strip_cr: bool) {
    match s {
        Some(s) if strip_cr => out.extend(s.iter().copied().filter(|&b| b != b'\r')),
        Some(s) => out.extend_from_slice(s),
        None => out.extend_from_slice(default),
    }
    out.extend_from_slice(b"~\n");
}

/// The `x == NOTHING ? -1: x` dance (NOTHING/NOBODY = 65535).
fn none_as_minus_one(v: i32) -> i64 {
    if v == 65535 { -1 } else { v as i64 }
}

/// A quest field holding a mob/obj/room/quest vnum, written -1 when unset.
/// The .qst format has no established export convention, so the marking
/// rule here is this writer's own.
fn push_vnum_field(out: &mut Vec<u8>, fmt: VnumFmt, v: i32) {
    match none_as_minus_one(v) {
        -1 => push_i64(out, -1),
        v => fmt.push_vnum(out, v),
    }
}

pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let zone = &world.zones[zone_rnum as usize];
    let mut out: Vec<u8> = Vec::new();

    for vnum in zone.bot..=zone.top {
        let Some(q) = world.quests.iter().find(|q| q.vnum == vnum) else {
            continue;
        };

        let mut rec: Vec<u8> = Vec::new();
        rec.push(b'#');
        fmt.push_vnum(&mut rec, q.vnum as i64);
        rec.push(b'\n');
        push_str(&mut rec, &q.name, b"Untitled", false);
        push_str(&mut rec, &q.desc, b"undefined", true);
        push_str(&mut rec, &q.info, b"undefined", true);
        push_str(&mut rec, &q.done, b"undefined", true);
        push_str(&mut rec, &q.quit, b"undefined", true);

        push_i64(&mut rec, q.type_ as i64);
        rec.push(b' ');
        push_vnum_field(&mut rec, fmt, q.qm_vnum);
        rec.push(b' ');
        rec.extend_from_slice(&sprintascii(q.flags));
        rec.push(b' ');
        // The target is a mob, object or room vnum depending on the quest
        // type — a vnum either way.
        push_vnum_field(&mut rec, fmt, q.target);
        rec.push(b' ');
        push_vnum_field(&mut rec, fmt, q.prev_quest);
        rec.push(b' ');
        push_vnum_field(&mut rec, fmt, q.next_quest);
        rec.push(b' ');
        push_vnum_field(&mut rec, fmt, q.prereq);
        rec.push(b'\n');

        push_i64(&mut rec, q.value as i64);
        rec.push(b' ');
        push_i64(&mut rec, q.penalty as i64);
        rec.push(b' ');
        push_i64(&mut rec, q.min_level as i64);
        rec.push(b' ');
        push_i64(&mut rec, q.max_level as i64);
        rec.push(b' ');
        push_i64(&mut rec, q.time as i64);
        rec.push(b' ');
        // QST_RETURNMOB == NOBODY ? -1:... (a -1 already in the int slot
        // also prints -1); quantity is raw.
        push_vnum_field(&mut rec, fmt, q.obj_in);
        rec.push(b' ');
        push_i64(&mut rec, q.obj_out as i64);
        rec.push(b'\n');

        push_i64(&mut rec, q.gold_reward as i64);
        rec.push(b' ');
        push_i64(&mut rec, q.exp_reward as i64);
        rec.push(b' ');
        // obj_reward is written RAW — NOTHING appears as 65535 on disk,
        // which push_vnum passes through rather than marking ZZ35.
        fmt.push_vnum(&mut rec, q.obj_reward as i64);
        rec.push(b'\n');
        rec.extend_from_slice(b"S\n");

        // if (n < MAX_STRING_LENGTH) write; else skip with a SYSERR.
        if rec.len() < MAX_STRING_LENGTH {
            parse_tab(&mut rec);
            out.extend_from_slice(&rec);
        }
    }

    out.extend_from_slice(b"$~\n");
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::{Quest, World, Zone};
    use crate::parse;

    #[test]
    fn empty_zone_writes_bare_terminator() {
        let mut w = World::default();
        w.zones.push(Zone { number: 3, bot: 300, top: 399, ..Default::default() });
        assert_eq!(write_file(&w, 0), b"$~\n");
    }

    #[test]
    fn defaults_conversions_and_tabs() {
        let mut w = World::default();
        w.zones.push(Zone { number: 0, bot: 0, top: 99, ..Default::default() });
        w.quests.push(Quest {
            vnum: 7,
            qm_vnum: 65535, // NOBODY => -1
            flags: 1,
            type_: 3,
            name: None,                                // => "Untitled"
            desc: Some(b"multi\r\nline\r\n".to_vec()), // strip_cr
            info: Some(b"see \tRred\tn".to_vec()),     // tabs => '@'
            done: None,
            quit: None,
            value: 10,
            penalty: 0,
            min_level: 1,
            max_level: 34,
            target: 65535,  // => -1
            prereq: 4,
            obj_in: -1,     // int slot: already -1
            obj_out: 2,
            time: 60,
            gold_reward: 5,
            exp_reward: 0,
            obj_reward: 65535, // written RAW
            prev_quest: 65535,
            next_quest: 200,
        });
        let want: &[u8] = b"#7\nUntitled~\nmulti\nline\n~\nsee @Rred@n~\nundefined~\nundefined~\n\
            3 -1 a -1 -1 200 4\n10 0 1 34 60 -1 2\n5 0 65535\nS\n$~\n";
        assert_eq!(
            String::from_utf8_lossy(&write_file(&w, 0)),
            String::from_utf8_lossy(want)
        );
    }

    // ---- golden round-trip ----

    /// The .qst format has no established export convention: the help text
    /// lists it among a zone's files, but the seven `export_save_*` do not
    /// include it. Its vnum columns are the questmaster, the target, the
    /// prev/next chain, the prerequisite object, the return mob and the
    /// object reward; value, levels, time, quantity and gold are not.
    #[test]
    fn export_marks_the_quest_vnum_columns() {
        let mut w = World::default();
        w.zones.push(Zone { number: 1, bot: 100, top: 199, ..Default::default() });
        w.quests.push(Quest {
            vnum: 104,
            qm_vnum: 179,
            flags: 1,
            type_: 3,
            name: Some(b"A Quest".to_vec()),
            desc: None,
            info: None,
            done: None,
            quit: None,
            value: 10,
            penalty: 0,
            min_level: 1,
            max_level: 34,
            target: 3001, // a mob in someone else's zone
            prereq: 122,
            obj_in: 145,
            obj_out: 2,
            time: 60,
            gold_reward: 500,
            exp_reward: 0,
            obj_reward: 65535, // unset: stays raw, never ZZ35
            prev_quest: 65535,
            next_quest: 105,
        });
        let qq = write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]));
        let qq = String::from_utf8_lossy(&qq).into_owned();
        assert!(qq.starts_with("#QQ04\n"), "{qq}");
        assert!(qq.contains("\n3 QQ79 a ZZ01 -1 QQ05 QQ22\n"), "{qq}");
        assert!(qq.contains("\n10 0 1 34 60 QQ45 2\n500 0 65535\nS\n"), "{qq}");

        let re = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        let re = String::from_utf8_lossy(&re).into_owned();
        assert!(re.starts_with("#40004\n"), "{re}");
        assert!(re.contains("\n3 40079 a ZZ01 -1 40005 40022\n"), "{re}");
        assert!(re.contains("\n10 0 1 34 60 40045 2\n500 0 65535\nS\n"), "{re}");
    }

}
