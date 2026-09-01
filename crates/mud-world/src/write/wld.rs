//! .wld writer: save_rooms. Rooms are emitted in ascending vnum order
//! over the zone window; the header (name + description + numeric flag
//! line) passes through convert_from_tabs, exit general descriptions and
//! extra-description bodies get strip_cr only, and keywords are written
//! raw.

use mud_data::flags::{EX_HIDDEN, EX_ISDOOR, EX_PICKPROOF};
use mud_data::types::{MAX_STRING_LENGTH, NOTHING, NOWHERE};

use crate::model::World;
use crate::write::{push_int, VnumFmt};

/// DIR_COUNT with CONFIG_DIAGONAL_DIRS = NO:
/// only D0-D5 are ever written, whatever dir_option holds.
const DIR_COUNT: usize = 6;

/// convert_from_tabs: '\t' turns back into '@' unless followed by another
/// '\t', in which case the pair
/// is skipped unchanged — the inverse of parse_at's "@@" rule.
pub(crate) fn parse_tab(s: &mut [u8]) {
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

/// strip_cr, as a copying append.
fn push_stripped(out: &mut Vec<u8>, s: &[u8]) {
    out.extend(s.iter().copied().filter(|&b| b != b'\r'));
}

pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let zone = &world.zones[zone_rnum as usize];
    let mut out = Vec::new();

    for i in zone.bot as i32..=zone.top as i32 {
        let Some(rnum) = world.real_room(i as u16) else { continue };
        let room = &world.rooms[rnum as usize];

        // Header: vnum, name, description, then the six numeric fields.
        // The whole buffer then goes through tab conversion.
        let mut buf2 = Vec::new();
        buf2.push(b'#');
        fmt.push_vnum(&mut buf2, i64::from(room.vnum));
        buf2.push(b'\n');
        match &room.name {
            Some(n) => buf2.extend_from_slice(n),
            None => buf2.extend_from_slice(b"Untitled"),
        }
        buf2.extend_from_slice(b"~\n");
        match &room.description {
            Some(d) => push_stripped(&mut buf2, d), // strip_cr
            None => buf2.extend_from_slice(b"Empty room."),
        }
        buf2.extend_from_slice(b"~\n");
        fmt.push_zone_number(&mut buf2, i64::from(world.zones[room.zone as usize].number));
        buf2.extend_from_slice(
            format!(
                " {} {} {} {} {}\n",
                room.room_flags[0] as i32,
                room.room_flags[1] as i32,
                room.room_flags[2] as i32,
                room.room_flags[3] as i32,
                room.sector_type
            )
            .as_bytes(),
        );
        if buf2.len() >= MAX_STRING_LENGTH {
            // A room whose record does not fit is skipped whole.
            // Unreachable for anything that parsed.
            continue;
        }
        parse_tab(&mut buf2);
        out.extend_from_slice(&buf2);

        // Exits.
        for (j, ex) in room.dir_option.iter().enumerate().take(DIR_COUNT) {
            let Some(ex) = ex else { continue };
            out.extend_from_slice(format!("D{j}\n").as_bytes());
            // general description: strip_cr only, no tab conversion.
            if let Some(d) = &ex.general_description {
                push_stripped(&mut out, d);
            }
            out.extend_from_slice(b"~\n");
            // keyword: written raw (no strip_cr, no tab conversion).
            if let Some(k) = &ex.keyword {
                out.extend_from_slice(k);
            }
            out.extend_from_slice(b"~\n");
            // Door flag reverse mapping: EX_CLOSED /
            // EX_LOCKED runtime bits are ignored by design.
            let dflag = if ex.exit_info & EX_ISDOOR != 0 {
                let base = if ex.exit_info & EX_PICKPROOF != 0 { 2 } else { 1 };
                base + if ex.exit_info & EX_HIDDEN != 0 { 2 } else { 0 }
            } else {
                0
            };
            push_int(&mut out, i64::from(dflag));
            out.push(b' ');
            // The key and the target are both vnum references: export
            // marks the ones that leave the zone for reattachment.
            match ex.key {
                NOTHING => out.extend_from_slice(b"-1"),
                k => fmt.push_vnum(&mut out, i64::from(k)),
            }
            out.push(b' ');
            match ex.to_room {
                NOWHERE => out.extend_from_slice(b"-1"),
                r => fmt.push_vnum(&mut out, i64::from(world.rooms[r as usize].vnum)),
            }
            out.push(b'\n');
        }

        // Extra descriptions in stored list order.
        for xd in &room.ex_descriptions {
            out.extend_from_slice(b"E\n");
            match &xd.keyword {
                Some(k) => out.extend_from_slice(k), // raw
                // A missing keyword is written as the literal "(null)".
                None => out.extend_from_slice(b"(null)"),
            }
            out.extend_from_slice(b"~\n");
            // Parsing never stores an empty description, so the None arm
            // below is unreachable for anything read from a file.
            if let Some(d) = &xd.description {
                push_stripped(&mut out, d);
            }
            out.extend_from_slice(b"~\n");
        }

        out.extend_from_slice(b"S\n");
        // One "T <vnum>" line per attached prototype trigger.
        for &t in &room.proto_script {
            out.extend_from_slice(b"T ");
            fmt.push_vnum(&mut out, i64::from(t));
            out.push(b'\n');
        }
    }

    out.extend_from_slice(b"$~\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Exit, ExtraDesc, Room, World, Zone};

    fn tiny_world() -> World {
        let mut w = World::default();
        w.zones.push(Zone { number: 12, bot: 1200, top: 1299, ..Default::default() });
        w
    }

    fn add_room(w: &mut World, room: Room) {
        let rnum = w.rooms.len() as u16;
        w.room_map.insert(room.vnum, rnum);
        w.rooms.push(room);
    }

    #[test]
    fn fallbacks_for_missing_name_and_description() {
        let mut w = tiny_world();
        add_room(&mut w, Room { vnum: 1200, zone: 0, ..Default::default() });
        assert_eq!(
            write_file(&w, 0),
            b"#1200\nUntitled~\nEmpty room.~\n12 0 0 0 0 0\nS\n$~\n"
        );
    }

    #[test]
    fn header_converts_tabs_but_exit_strings_do_not() {
        let mut w = tiny_world();
        let mut room = Room {
            vnum: 1200,
            zone: 0,
            name: Some(b"A \tRoom".to_vec()),
            description: Some(b"Colored \tGtext\t\tkept.\r\n".to_vec()),
            ..Default::default()
        };
        room.dir_option[2] = Some(Box::new(Exit {
            general_description: Some(b"South \tYglow.\r\n".to_vec()),
            keyword: Some(b"door\tway".to_vec()),
            exit_info: 0,
            key: NOTHING,
            to_room_vnum: 0,
            to_room: NOWHERE,
        }));
        add_room(&mut w, room);
        let out = write_file(&w, 0);
        // The description's trailing "\r\n" strip_cr's to "\n", so the
        // tilde lands on its own line, as in every multi-line body.
        let expect: &[u8] = b"#1200\nA @Room~\nColored @Gtext\t\tkept.\n~\n12 0 0 0 0 0\n\
              D2\nSouth \tYglow.\n~\ndoor\tway~\n0 -1 -1\nS\n$~\n";
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(expect)
        );
    }

    #[test]
    fn door_flags_key_and_to_room_resolution() {
        let mut w = tiny_world();
        let mut room = Room { vnum: 1200, zone: 0, ..Default::default() };
        let mk = |exit_info, key, to_room| {
            Some(Box::new(Exit {
                general_description: None,
                keyword: None,
                exit_info,
                key,
                to_room_vnum: 0,
                to_room,
            }))
        };
        room.dir_option[0] = mk(EX_ISDOOR, 0, 1);
        room.dir_option[1] = mk(EX_ISDOOR | EX_PICKPROOF, NOTHING, NOWHERE);
        room.dir_option[2] = mk(EX_ISDOOR | EX_HIDDEN, 65534, NOWHERE);
        room.dir_option[3] = mk(EX_ISDOOR | EX_PICKPROOF | EX_HIDDEN, 5, NOWHERE);
        // Diagonal slots exist in memory but DIR_COUNT=6 hides 6..=9.
        room.dir_option[7] = mk(0, NOTHING, NOWHERE);
        add_room(&mut w, room);
        add_room(&mut w, Room { vnum: 1250, zone: 0, ..Default::default() });
        let out = write_file(&w, 0);
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("D0\n~\n~\n1 0 1250\n"), "{s}"); // to_room rnum 1 → vnum 1250
        assert!(s.contains("D1\n~\n~\n2 -1 -1\n"), "{s}");
        assert!(s.contains("D2\n~\n~\n3 65534 -1\n"), "{s}");
        assert!(s.contains("D3\n~\n~\n4 5 -1\n"), "{s}");
        assert!(!s.contains("D7"), "{s}");
    }

    #[test]
    fn exdesc_null_keyword_prints_glibc_marker_and_t_lines_follow_s() {
        let mut w = tiny_world();
        let room = Room {
            vnum: 1200,
            zone: 0,
            ex_descriptions: vec![ExtraDesc {
                keyword: None,
                description: Some(b"Body.\r\n".to_vec()),
            }],
            proto_script: vec![1201, 1299],
            ..Default::default()
        };
        add_room(&mut w, room);
        assert_eq!(
            write_file(&w, 0),
            b"#1200\nUntitled~\nEmpty room.~\n12 0 0 0 0 0\nE\n(null)~\nBody.\n~\nS\nT 1201\nT 1299\n$~\n"
        );
    }

    /// One room with an in-zone exit, an exit that leaves the zone, a key
    /// and an attached trigger — every vnum site the.wld writer has.
    fn export_fixture() -> World {
        let mut w = tiny_world();
        let mut room = Room { vnum: 1204, zone: 0, proto_script: vec![1210], ..Default::default() };
        let exit = |key, to_room| {
            Some(Box::new(Exit {
                general_description: None,
                keyword: None,
                exit_info: 0,
                key,
                to_room_vnum: 0,
                to_room,
            }))
        };
        room.dir_option[0] = exit(1215, 1); // rnum 1 -> vnum 1250, in zone
        room.dir_option[1] = exit(NOTHING, 2); // rnum 2 -> vnum 2010, elsewhere
        add_room(&mut w, room);
        add_room(&mut w, Room { vnum: 1250, zone: 0, ..Default::default() });
        add_room(&mut w, Room { vnum: 2010, zone: 0, ..Default::default() });
        w
    }

    #[test]
    fn qq_export_marks_the_zone_and_zzs_the_exit_that_leaves_it() {
        let w = export_fixture();
        let out = write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]));
        assert_eq!(
            String::from_utf8_lossy(&out),
            "#QQ04\nUntitled~\nEmpty room.~\nQQ 0 0 0 0 0\n\
             D0\n~\n~\n0 QQ15 QQ50\n\
             D1\n~\n~\n0 -1 ZZ10\n\
             S\nT QQ10\n\
             #QQ50\nUntitled~\nEmpty room.~\nQQ 0 0 0 0 0\nS\n$~\n"
        );
    }

    #[test]
    fn renumbering_export_slides_every_in_zone_vnum() {
        let w = export_fixture();
        let out = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 40));
        assert_eq!(
            String::from_utf8_lossy(&out),
            "#4004\nUntitled~\nEmpty room.~\n40 0 0 0 0 0\n\
             D0\n~\n~\n0 4015 4050\n\
             D1\n~\n~\n0 -1 ZZ10\n\
             S\nT 4010\n\
             #4050\nUntitled~\nEmpty room.~\n40 0 0 0 0 0\nS\n$~\n"
        );
    }

    /// The door flag mapping — 1/2 plus 2 when hidden — is what an export
    /// carries, so a hidden door stays hidden (and a pickproof hidden door
    /// stays both).
    #[test]
    fn a_hidden_door_stays_hidden_through_an_export() {
        let mut w = tiny_world();
        let mut room = Room { vnum: 1204, zone: 0, ..Default::default() };
        let door = |exit_info| {
            Some(Box::new(Exit {
                general_description: None,
                keyword: None,
                exit_info,
                key: NOTHING,
                to_room_vnum: 0,
                to_room: NOWHERE,
            }))
        };
        room.dir_option[0] = door(EX_ISDOOR);
        room.dir_option[1] = door(EX_ISDOOR | EX_HIDDEN);
        room.dir_option[2] = door(EX_ISDOOR | EX_PICKPROOF | EX_HIDDEN);
        add_room(&mut w, room);
        let out = write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]));
        let out = String::from_utf8_lossy(&out).into_owned();
        assert!(out.contains("D0\n~\n~\n1 -1 -1\n"), "{out}");
        assert!(out.contains("D1\n~\n~\n3 -1 -1\n"), "{out}");
        assert!(out.contains("D2\n~\n~\n4 -1 -1\n"), "{out}");
    }

    #[test]
    fn rooms_emit_in_vnum_order_regardless_of_load_order() {
        let mut w = tiny_world();
        add_room(&mut w, Room { vnum: 1220, zone: 0, ..Default::default() });
        add_room(&mut w, Room { vnum: 1210, zone: 0, ..Default::default() });
        let out = write_file(&w, 0);
        let s = String::from_utf8_lossy(&out);
        assert!(s.find("#1210").unwrap() < s.find("#1220").unwrap());
    }
}

/// Real-file golden round-trip: parse the shipped zones + rooms (with
/// trigger vnums registered so post-'S' T lines survive, matching boot
/// order), renumber exits the way boot does, write every zone back, and
/// byte-compare against the golden tree.
#[cfg(test)]
mod golden_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use mud_data::types::NOWHERE;

    use crate::lex::Reader;
    use crate::model::{Trigger, World};
    use crate::parse;

    /// index_boot: entries until a '$' line.
    fn read_index(dir: &Path) -> Vec<String> {
        let data = fs::read(dir.join("index")).expect("index file");
        let mut r = Reader::new(&data);
        let mut out = Vec::new();
        while let Some(line) = r.get_line_sized(49152) {
            if line.starts_with(b"$") {
                break;
            }
            out.push(String::from_utf8_lossy(&line).into_owned());
        }
        out
    }

    /// Just enough of parse_trigger to register each trigger's vnum (and
    /// name) so real_trigger resolves during the .wld parse — triggers
    /// load before rooms in boot_world.
    fn register_triggers(world: &mut World, data: &[u8]) {
        let mut r = Reader::new(data);
        while let Some(line) = r.get_line_sized(49152) {
            match line.first() {
                Some(b'$') => break,
                Some(b'#') => {
                    let vnum = crate::lex::atol(&line[1..]);
                    if vnum >= 99999 {
                        break;
                    }
                    let name = r.fread_string("trig name").expect("trig name");
                    r.get_line_sized(49152); // attach_type / flags / narg line
                    r.fread_string("trig arg").expect("trig arglist");
                    r.fread_string("trig body").expect("trig body");
                    let rnum = world.triggers.len() as u16;
                    world.triggers.push(Trigger {
                        vnum: vnum as u16,
                        name,
                        ..Default::default()
                    });
                    world.trig_map.entry(vnum as u16).or_insert(rnum);
                }
                _ => panic!("bad trg line: {:?}", String::from_utf8_lossy(&line)),
            }
        }
    }

    fn assert_bytes_eq(ours: &[u8], golden: &[u8], what: &str) {
        if ours == golden {
            return;
        }
        let n = ours.iter().zip(golden.iter()).take_while(|(a, b)| a == b).count();
        let ctx = |b: &[u8]| {
            String::from_utf8_lossy(&b[n.saturating_sub(40)..(n + 60).min(b.len())])
                .into_owned()
        };
        panic!(
            "{what}: first differs at byte {n} (ours {} bytes, golden {} bytes)\n\
             ours:   {:?}\ngolden: {:?}",
            ours.len(),
            golden.len(),
            ctx(ours),
            ctx(golden)
        );
    }

}
