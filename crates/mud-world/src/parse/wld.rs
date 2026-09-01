//! The .wld loader: the record loop, parse_room, and setup_dir — including
//! the zone-window scan (fatal outside every zone), the ignored in-file zone
//! number, door-flag 0-4 mapping, key -1/65535 and to_room 0/-1 sentinels,
//! prepended E-blocks, and the post-'S' trigger lines, which are dropped
//! when the vnum does not resolve.

use mud_data::flags::{EX_HIDDEN, EX_ISDOOR, EX_PICKPROOF};
use mud_data::types::{NOTHING, NOWHERE};

use crate::lex::{asciiflag_conv, atol, Reader};
use crate::model::{Exit, ExtraDesc, Room, World};

use super::zon::{is_ws, Scan};

/// `diagonal_dirs = NO`, and the reference lib/ ships no
/// etc/config to override it — so DIR_COUNT is 6 and setup_dir refuses
/// diagonal blocks without reading them.
pub(crate) const CONFIG_DIAGONAL_DIRS: bool = false;

/// `WORLD_FLAG_FIELD`: the longest single flag field a world file may
/// carry, and the width the scanf conversions are bounded to.
pub(crate) const WORLD_FLAG_FIELD: usize = 127;

/// R1: a scanf field width bounds the store, not the scan. Handed a token
/// longer than the width, `%127s` copies 127 characters and leaves the read
/// position *inside* the token, so the next conversion collects the
/// remainder as though it were the next field — every field after it shifts
/// along, and the conversion count still reaches its total. Bounding the
/// buffers stops the overflow but not the shift, so such a line is refused
/// outright. Checked per whitespace-separated token.
pub(crate) fn line_has_overlong_field(line: &[u8]) -> bool {
    line.split(|b| is_ws(*b))
        .any(|tok| tok.len() > WORLD_FLAG_FIELD)
}

pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let mut r = Reader::new(data);
    let mut nr: i64 = -1;
    // parse_room hands back the first non-'T' line after a room's 'S' (
    // peeks a char with fread_letter+ungetc; we re-process a whole line).
    let mut pending: Option<Vec<u8>> = None;

    loop {
        let line = match pending.take() {
            Some(l) => l,
            None => match r.get_line() {
                Some(l) => l,
                None => {
                    return Err(if nr == -1 {
                        format!("SYSERR: world file {filename} is empty!")
                    } else {
                        format!(
                            "SYSERR: Format error in {filename} after world #{nr}\n\
                             ...expecting a new world, but file ended!\n\
                             (maybe the file is not terminated with '$'?)"
                        )
                    });
                }
            },
        };
        match line.first() {
            Some(b'$') => return Ok(()),
            Some(b'#') => {
                let last = nr;
                nr = match Scan::new(&line[1..]).int() {
                    Some(v) => v,
                    None => {
                        return Err(format!("SYSERR: Format error after world #{last}"));
                    }
                };
                if nr >= 99999 {
                    return Ok(()); // acts like '$'
                }
                pending = parse_room(world, &mut r, nr as i32)?;
            }
            _ => {
                return Err(format!(
                    "SYSERR: Format error in world file {filename} near world #{nr}\n\
                     SYSERR: ... offending line: '{}'",
                    String::from_utf8_lossy(&line)
                ));
            }
        }
    }
}

/// Returns the pending line for the caller's loop.
fn parse_room(
    world: &mut World,
    r: &mut Reader,
    virtual_nr: i32,
) -> Result<Option<Vec<u8>>, String> {
    let buf2 = format!("room #{virtual_nr}");

    // The zone cursor only ever advances, so its value at entry is always
    // the previous room's zone.
    let mut zone = world.rooms.last().map(|rm| rm.zone as usize).unwrap_or(0);
    if world.zones.is_empty() {
        // An empty zone table would be indexed here; boot order
        // forbids it.
        return Err(format!("SYSERR: Room {virtual_nr} is outside of any zone."));
    }
    if virtual_nr < world.zones[zone].bot as i32 {
        return Err(format!(
            "SYSERR: Room #{} is below zone {} (bot={}, top={}).",
            virtual_nr,
            world.zones[zone].number,
            world.zones[zone].bot,
            world.zones[zone].top
        ));
    }
    while virtual_nr > world.zones[zone].top as i32 {
        zone += 1;
        if zone >= world.zones.len() {
            return Err(format!("SYSERR: Room {virtual_nr} is outside of any zone."));
        }
    }
    // No bot re-check after advancing: a room in the gap between two zones
    // attaches to the next zone, exactly as does.

    let mut room = Room {
        vnum: virtual_nr as u16, // int → ush_int store truncates
        zone: zone as u16,
        ..Default::default()
    };
    room.name = r.fread_string(&buf2)?;
    room.description = r.fread_string(&buf2)?;

    let line = r.get_line().ok_or_else(|| {
        format!("SYSERR: Expecting roomflags/sector type of room #{virtual_nr} but file ended!")
    })?;
    if line_has_overlong_field(&line) {
        return Err(format!(
            "SYSERR: Room #{virtual_nr} has a field longer than {WORLD_FLAG_FIELD} characters."
        ));
    }
    // Six fields; count them the same way.
    let mut sc = Scan::new(&line);
    let mut retval = 0;
    let t0 = sc.int();
    if t0.is_some() {
        retval = 1;
        // The leading zone number is read and ignored.
    }
    let mut flags_tok: [Option<Vec<u8>>; 4] = [None, None, None, None];
    if retval == 1 {
        for slot in flags_tok.iter_mut() {
            match sc.word(usize::MAX) {
                Some(w) => {
                    *slot = Some(w);
                    retval += 1;
                }
                None => break,
            }
        }
    }
    let mut t2 = None;
    if retval == 5 {
        t2 = sc.int();
        if t2.is_some() {
            retval = 6;
        }
    }
    if retval == 3 {
        // Legacy 3-field "zone flags sector" line. Stock config has
        // bitwarning=FALSE, so the value is converted rather than aborting
        // (the save-list/`converting` bookkeeping that
        // rewrites files at boot end is out of scope here).
        room.room_flags[0] = asciiflag_conv(flags_tok[0].as_deref().unwrap_or(b""));
        room.room_flags[1] = 0;
        room.room_flags[2] = 0;
        room.room_flags[3] = 0;
        room.sector_type = atol(flags_tok[1].as_deref().unwrap_or(b"")) as i32;
    } else if retval == 6 {
        for (i, tok) in flags_tok.iter().enumerate() {
            room.room_flags[i] = asciiflag_conv(tok.as_deref().unwrap_or(b""));
        }
        // Sanity check is `> NUM_ROOM_SECTORS` (10): sector 10 passes, 11+
        // clamps to SECT_INSIDE, negatives pass (off-by-one).
        let mut sect = t2.unwrap() as i32;
        if sect > 10 {
            sect = 0;
        }
        room.sector_type = sect;
    } else {
        return Err(format!(
            "SYSERR: Format error in roomflags/sector type of room #{virtual_nr}"
        ));
    }

    let err_des = format!("SYSERR: Format error in room #{virtual_nr} (expecting D/E/S)");
    loop {
        let line = r.get_line().ok_or_else(|| err_des.clone())?;
        match line.first() {
            Some(b'D') => {
                // dir = atoi(line + 1).
                setup_dir(r, &mut room, atol(&line[1..]) as i32)?;
            }
            Some(b'E') => {
                let mut ed = ExtraDesc {
                    keyword: r.fread_string(&buf2)?,
                    description: r.fread_string(&buf2)?,
                };
                // ensure_newline_terminated (call at 1367).
                if let Some(d) = &mut ed.description
                    && !d.is_empty()
                    && d.last() != Some(&b'\n')
                {
                    d.extend_from_slice(b"\r\n");
                }
                // Prepended: the Vec is head-first, and writers/readers
                // walk it forward.
                room.ex_descriptions.insert(0, ed);
            }
            Some(b'S') => {
                // DG triggers come after the room's S.
                // Reading whole get_line lines here means a '*'-comment
                // line between S and a T line is silently skipped, since
                // Reader hides comment lines.
                let mut pending = None;
                while let Some(tline) = r.get_line() {
                    let first =
                        tline.iter().copied().find(|&b| !is_ws(b));
                    if first == Some(b'T') {
                        dg_read_trigger(world, &mut room, &tline);
                    } else {
                        pending = Some(tline);
                        break;
                    }
                }
                let rnum = world.rooms.len() as u16;
                // On a duplicate vnum the map keeps the first room,
                // deterministically. The shipped world has none.
                world.room_map.entry(room.vnum).or_insert(rnum);
                world.rooms.push(room);
                return Ok(pending);
            }
            _ => return Err(err_des),
        }
    }
}

fn setup_dir(r: &mut Reader, room: &mut Room, dir: i32) -> Result<(), String> {
    // Names the room actually being read. Deriving it from the room
    // table instead would answer NOWHERE, since the table has not reached
    // this room yet, and report "room #65536" for every room but rnum 0.
    let buf2 = format!("room #{}, direction D{}", room.vnum, dir);

    if !CONFIG_DIAGONAL_DIRS && (6..=9).contains(&dir) {
        // Logs "Warning: Diagonal direction disabled" and returns WITHOUT
        // reading the block's strings — the following lines then fall into
        // the D/E/S dispatcher.
        return Ok(());
    }
    if !(0..10).contains(&dir) {
        // Out of range for dir_option — refuse the line.
        return Err(format!("SYSERR: Format error, {buf2}"));
    }

    let general_description = r.fread_string(&buf2)?;
    let keyword = r.fread_string(&buf2)?;

    let line = r
        .get_line()
        .ok_or_else(|| format!("SYSERR: Format error, {buf2}"))?;
    let mut sc = Scan::new(&line);
    let (Some(t0), Some(t1), Some(t2)) = (sc.int(), sc.int(), sc.int()) else {
        return Err(format!("SYSERR: Format error, {buf2}"));
    };

    // Door-flag mapping; anything else means no door.
    let exit_info = match t0 {
        1 => EX_ISDOOR,
        2 => EX_ISDOOR | EX_PICKPROOF,
        3 => EX_ISDOOR | EX_HIDDEN,
        4 => EX_ISDOOR | EX_PICKPROOF | EX_HIDDEN,
        _ => 0,
    };

    // A second D-block for the same direction overwrites the first (
    // CREATEs over the old pointer, leaking it —).
    room.dir_option[dir as usize] = Some(Box::new(Exit {
        general_description,
        keyword,
        exit_info,
        // key -1 or 65535 ⇒ NOTHING; other values truncate through
        // obj_vnum (unsigned short), key 0 is real.
        key: if t1 == -1 || t1 == 65535 { NOTHING } else { t1 as u16 },
        // to_room 0 or -1 ⇒ NOWHERE. The raw vnum is kept separately and
        // boot's renum pass resolves it.
        to_room_vnum: t2 as i32,
        to_room: NOWHERE,
    }));
    Ok(())
}

/// (WLD branch of dg_read_trigger): "%7s %d"; a
/// malformed line or an unknown trigger vnum logs a SYSERR and is dropped
/// — the vnum is only appended to proto_script when real_trigger hits.
fn dg_read_trigger(world: &World, room: &mut Room, line: &[u8]) {
    let mut sc = Scan::new(line);
    let junk = sc.word(7);
    let vnum = sc.int();
    let (Some(_), Some(vnum)) = (junk, vnum) else {
        return; // count != 2
    };
    let vnum = vnum as u16; // int → trig_vnum truncates
    if world.real_trigger(vnum).is_none() {
        return; // "Trigger vnum #%d asked for but non-existant!" — dropped
    }
    room.proto_script.push(vnum);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Trigger;

    /// One zone covering 0..=99 plus one covering 3000..=3099.
    fn world_with_zones() -> World {
        let mut w = World::default();
        for (number, bot, top) in [(0u16, 0u16, 99u16), (30, 3000, 3099)] {
            w.zones.push(crate::model::Zone {
                number,
                bot,
                top,
                ..Default::default()
            });
        }
        w
    }

    fn parse_into(w: &mut World, data: &[u8]) -> Result<(), String> {
        parse_file(w, data, "test.wld")
    }

    #[test]
    fn basic_room_and_exit() {
        let mut w = world_with_zones();
        parse_into(
            &mut w,
            b"#3001\nThe Temple~\n   Description line one.\nLine two.\n~\n\
              30 156 0 0 0 2\nD0\nYou see north.\n~\ngate~\n1 3010 3054\nS\n$~\n",
        )
        .unwrap();
        let room = &w.rooms[0];
        assert_eq!(room.vnum, 3001);
        assert_eq!(room.zone, 1);
        assert_eq!(room.room_flags, [156, 0, 0, 0]);
        assert_eq!(room.sector_type, 2);
        assert_eq!(room.name.as_deref(), Some(&b"The Temple"[..]));
        assert_eq!(
            room.description.as_deref(),
            Some(&b"   Description line one.\r\nLine two.\r\n"[..])
        );
        let ex = room.dir_option[0].as_ref().unwrap();
        assert_eq!(ex.general_description.as_deref(), Some(&b"You see north.\r\n"[..]));
        assert_eq!(ex.keyword.as_deref(), Some(&b"gate"[..]));
        assert_eq!(ex.exit_info, EX_ISDOOR);
        assert_eq!(ex.key, 3010);
        assert_eq!(ex.to_room_vnum, 3054);
        assert_eq!(ex.to_room, NOWHERE);
        assert_eq!(w.real_room(3001), Some(0));
    }

    #[test]
    fn door_flag_mapping_and_sentinels() {
        let mut w = world_with_zones();
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\n~\n~\n0 0 0 0 0 0\n");
        for (d, line) in [
            (0, &b"0 -1 3054"[..]),   // plain exit, no key
            (1, b"2 65535 0"),        // pickproof, key sentinel, to_room 0
            (2, b"3 0 -1"),           // hidden, key 0 is real, to_room -1
            (3, b"4 70000 70000"),    // all bits, key wraps through u16
            (4, b"7 5 5"),            // out-of-range flag → 0
        ] {
            data.extend_from_slice(format!("D{d}\n~\n~\n").as_bytes());
            data.extend_from_slice(line);
            data.push(b'\n');
        }
        data.extend_from_slice(b"S\n$~\n");
        parse_into(&mut w, &data).unwrap();
        let room = &w.rooms[0];
        let ex = |d: usize| room.dir_option[d].as_ref().unwrap();
        assert_eq!((ex(0).exit_info, ex(0).key, ex(0).to_room_vnum), (0, NOTHING, 3054));
        assert_eq!(
            (ex(1).exit_info, ex(1).key, ex(1).to_room_vnum),
            (EX_ISDOOR | EX_PICKPROOF, NOTHING, 0)
        );
        assert_eq!(
            (ex(2).exit_info, ex(2).key, ex(2).to_room_vnum),
            (EX_ISDOOR | EX_HIDDEN, 0, -1)
        );
        assert_eq!(
            (ex(3).exit_info, ex(3).key, ex(3).to_room_vnum),
            (EX_ISDOOR | EX_PICKPROOF | EX_HIDDEN, 4464, 70000)
        );
        assert_eq!((ex(4).exit_info, ex(4).key), (0, 5));
    }

    #[test]
    fn sector_sanity_off_by_one() {
        let mut w = world_with_zones();
        parse_into(
            &mut w,
            b"#1\n~\n~\n0 0 0 0 0 10\nS\n#2\n~\n~\n0 0 0 0 0 11\nS\n#3\n~\n~\n0 0 0 0 0 -3\nS\n$~\n",
        )
        .unwrap();
        assert_eq!(w.rooms[0].sector_type, 10); // == NUM_ROOM_SECTORS passes
        assert_eq!(w.rooms[1].sector_type, 0); // > clamps to SECT_INSIDE
        assert_eq!(w.rooms[2].sector_type, -3); // negatives pass
    }

    #[test]
    fn legacy_three_field_line_converts() {
        let mut w = world_with_zones();
        parse_into(&mut w, b"#1\n~\n~\n0 abc 3\nS\n$~\n").unwrap();
        assert_eq!(w.rooms[0].room_flags, [0b111, 0, 0, 0]);
        assert_eq!(w.rooms[0].sector_type, 3);
    }

    #[test]
    fn letter_flags_on_128bit_line() {
        let mut w = world_with_zones();
        parse_into(&mut w, b"#1\n~\n~\n99 ad 0 b 0 1\nS\n$~\n").unwrap();
        // The leading zone field (99) is ignored; membership is windowed.
        assert_eq!(w.rooms[0].zone, 0);
        assert_eq!(w.rooms[0].room_flags, [0b1001, 0, 2, 0]);
    }

    #[test]
    fn extra_descriptions_prepend_and_newline_terminate() {
        let mut w = world_with_zones();
        parse_into(
            &mut w,
            b"#1\n~\n~\n0 0 0 0 0 0\nE\nfirst~\nSame-line tilde.~\nE\nsecond~\nOwn line.\n~\nS\n$~\n",
        )
        .unwrap();
        let room = &w.rooms[0];
        // Reverse file order (list head first).
        assert_eq!(room.ex_descriptions[0].keyword.as_deref(), Some(&b"second"[..]));
        assert_eq!(room.ex_descriptions[1].keyword.as_deref(), Some(&b"first"[..]));
        // Tilde on the text line ⇒ no \n ⇒ ensure_newline appends \r\n.
        assert_eq!(
            room.ex_descriptions[1].description.as_deref(),
            Some(&b"Same-line tilde.\r\n"[..])
        );
        assert_eq!(
            room.ex_descriptions[0].description.as_deref(),
            Some(&b"Own line.\r\n"[..])
        );
    }

    #[test]
    fn trigger_lines_after_s() {
        let mut w = world_with_zones();
        w.triggers.push(Trigger { vnum: 3017, ..Default::default() });
        w.trig_map.insert(3017, 0);
        parse_into(
            &mut w,
            b"#1\n~\n~\n0 0 0 0 0 0\nS\nT 3017\nT 9999\nTx 3017\n#2\n~\n~\n0 0 0 0 0 0\nS\n$~\n",
        )
        .unwrap();
        // 3017 resolves; 9999 doesn't (dropped); "Tx 3017" scans junk="Tx"
        // vnum=3017 → resolves and attaches (the %7s word is just junk).
        assert_eq!(w.rooms[0].proto_script, vec![3017, 3017]);
        assert_eq!(w.rooms.len(), 2); // the '#2' line was handed back intact
    }

    #[test]
    fn t_line_without_vnum_is_dropped() {
        let mut w = world_with_zones();
        parse_into(&mut w, b"#1\n~\n~\n0 0 0 0 0 0\nS\nTrash\n#2\n~\n~\n0 0 0 0 0 0\nS\n$~\n")
            .unwrap();
        assert!(w.rooms[0].proto_script.is_empty());
        assert_eq!(w.rooms.len(), 2);
    }

    #[test]
    fn room_below_zone_is_fatal() {
        let mut w = world_with_zones();
        // Advance the cursor into zone 30, then present an earlier vnum.
        let e = parse_into(
            &mut w,
            b"#3001\n~\n~\n0 0 0 0 0 0\nS\n#150\n~\n~\n0 0 0 0 0 0\nS\n$~\n",
        )
        .unwrap_err();
        assert_eq!(e, "SYSERR: Room #150 is below zone 30 (bot=3000, top=3099).");
    }

    #[test]
    fn room_outside_all_zones_is_fatal() {
        let mut w = world_with_zones();
        let e = parse_into(&mut w, b"#5000\n~\n~\n0 0 0 0 0 0\nS\n$~\n").unwrap_err();
        assert_eq!(e, "SYSERR: Room 5000 is outside of any zone.");
    }

    #[test]
    fn gap_room_attaches_to_next_zone() {
        // No bot re-check after the advance loop: a room between the two
        // windows lands in the following zone.
        let mut w = world_with_zones();
        parse_into(&mut w, b"#200\n~\n~\n0 0 0 0 0 0\nS\n$~\n").unwrap();
        assert_eq!(w.rooms[0].zone, 1);
    }

    #[test]
    fn vnum_99999_terminates_like_dollar() {
        let mut w = world_with_zones();
        parse_into(&mut w, b"#1\n~\n~\n0 0 0 0 0 0\nS\n#99999\nnot a room\n").unwrap();
        assert_eq!(w.rooms.len(), 1);
    }

    #[test]
    fn diagonal_dir_block_desyncs_parser() {
        // With diagonals disabled setup_dir consumes nothing, so the
        // block's text falls into the D/E/S dispatcher and dies there.
        let mut w = world_with_zones();
        let e = parse_into(
            &mut w,
            b"#1\n~\n~\n0 0 0 0 0 0\nD6\nA tunnel.~\n~\n0 -1 5\nS\n$~\n",
        )
        .unwrap_err();
        assert_eq!(e, "SYSERR: Format error in room #1 (expecting D/E/S)");
    }

    #[test]
    fn duplicate_vnum_keeps_both_rooms_first_in_map() {
        let mut w = world_with_zones();
        parse_into(
            &mut w,
            b"#7\nfirst~\n~\n0 0 0 0 0 0\nS\n#7\nsecond~\n~\n0 0 0 0 0 0\nS\n$~\n",
        )
        .unwrap();
        assert_eq!(w.rooms.len(), 2);
        assert_eq!(w.real_room(7), Some(0));
    }

    #[test]
    fn empty_file_and_missing_terminator_errors() {
        let mut w = world_with_zones();
        assert_eq!(
            parse_into(&mut w, b"").unwrap_err(),
            "SYSERR: world file test.wld is empty!"
        );
        let mut w = world_with_zones();
        let e = parse_into(&mut w, b"#1\n~\n~\n0 0 0 0 0 0\nS\n").unwrap_err();
        assert!(e.contains("expecting a new world, but file ended!"), "{e}");
    }
}
