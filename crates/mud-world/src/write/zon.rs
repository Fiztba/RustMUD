//! .zon writer: save_zone. The header is written 10-field only when any
//! zone flag or level is set (4-field
//! otherwise); every surviving command line regenerates its "\t(comment)"
//! from the referenced entity's current text; '*'-disabled and unknown
//! commands are skipped; the file ends "S\n$\n".
//!
//! PRECONDITION: renum_zone_table has run (boot.rs) — command args are
//! rnums indexing world.mob_protos / obj_protos / rooms / triggers, and
//! every command with an unresolvable reference is already '*'. That is
//! the only state this writer ever sees.

use crate::model::World;

use super::wld::parse_tab;
use super::{push_int, sprintascii, VnumFmt};

/// One numeric argument of a reset command. Which of the four columns is a
/// vnum depends on the command letter ('s quick-reference
/// chart), and only those are rewritten by an export.
#[derive(Clone, Copy)]
enum Arg {
    N(i32),
    V(i32),
}

impl Arg {
    fn push(self, out: &mut Vec<u8>, fmt: VnumFmt) {
        match self {
            Arg::N(v) => push_int(out, i64::from(v)),
            Arg::V(v) => fmt.push_vnum(out, i64::from(v)),
        }
    }
}

/// Entity strings are passed through unchecked, so a missing one lands in
/// the file as the literal "(null)".
fn push_str_or_null(out: &mut Vec<u8>, s: Option<&[u8]>) {
    match s {
        Some(s) => out.extend_from_slice(s),
        None => out.extend_from_slice(b"(null)"),
    }
}

fn nonempty(s: &Option<Vec<u8>>) -> Option<&[u8]> {
    match s.as_deref() {
        Some(b) if !b.is_empty() => Some(b),
        _ => None,
    }
}

pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let z = &world.zones[zone_rnum as usize];
    let mut out = Vec::new();

    // flag_tot: the four int flag words are summed, wrapping.
    let mut flag_tot: i32 = 0;
    for f in z.zone_flags {
        flag_tot = flag_tot.wrapping_add(f as i32);
    }

    out.push(b'#');
    fmt.push_zone_number(&mut out, i64::from(z.number));
    out.push(b'\n');
    // builders: no tab conversion.
    match nonempty(&z.builders) {
        Some(b) => out.extend_from_slice(b),
        None => out.extend_from_slice(b"None."),
    }
    out.extend_from_slice(b"~\n");
    // name: through convert_from_tabs.
    match nonempty(&z.name) {
        Some(n) => {
            let mut nb = n.to_vec();
            parse_tab(&mut nb);
            out.extend_from_slice(&nb);
        }
        None => out.extend_from_slice(b"undefined"),
    }
    out.extend_from_slice(b"~\n");

    // The window itself moves with an export: QQ00 QQ99, or the target
    // zone's range.
    fmt.push_vnum(&mut out, i64::from(z.bot));
    out.push(b' ');
    fmt.push_vnum(&mut out, i64::from(z.top));
    out.push(b' ');
    if flag_tot == 0 && z.min_level == -1 && z.max_level == -1 {
        // "If zone flags or levels aren't set, there is no reason to save
        // them!" — 4-field header.
        out.extend_from_slice(format!("{} {}\n", z.lifespan, z.reset_mode).as_bytes());
    } else {
        // 10-field header with sprintascii'd flags.
        out.extend_from_slice(
            format!(
                "{} {} {} {} {} {} {} {}\n",
                z.lifespan,
                z.reset_mode,
                String::from_utf8_lossy(&sprintascii(z.zone_flags[0])),
                String::from_utf8_lossy(&sprintascii(z.zone_flags[1])),
                String::from_utf8_lossy(&sprintascii(z.zone_flags[2])),
                String::from_utf8_lossy(&sprintascii(z.zone_flags[3])),
                z.min_level,
                z.max_level
            )
            .as_bytes(),
        );
    }

    // Command table. Args here are post-renum rnums.
    for cmd in &z.cmds {
        let (args, comment): ([Arg; 3], Option<&[u8]>) = match cmd.command {
            b'M' => {
                let m = &world.mob_protos[cmd.arg1 as usize];
                (
                    [
                        Arg::V(m.vnum as i32),
                        Arg::N(cmd.arg2),
                        Arg::V(world.rooms[cmd.arg3 as usize].vnum as i32),
                    ],
                    m.short_descr.as_deref(),
                )
            }
            b'O' => {
                let o = &world.obj_protos[cmd.arg1 as usize];
                // An O command may legally keep arg3 == NOWHERE (limbo
                // load, file room 65535), which indexes no room. Emit -1
                // instead.
                let room = match world.rooms.get(cmd.arg3 as usize) {
                    Some(r) => Arg::V(r.vnum as i32),
                    None => Arg::N(-1),
                };
                (
                    [Arg::V(o.vnum as i32), Arg::N(cmd.arg2), room],
                    o.short_description.as_deref(),
                )
            }
            b'G' => {
                let o = &world.obj_protos[cmd.arg1 as usize];
                (
                    [Arg::V(o.vnum as i32), Arg::N(cmd.arg2), Arg::N(-1)],
                    o.short_description.as_deref(),
                )
            }
            b'E' => {
                let o = &world.obj_protos[cmd.arg1 as usize];
                (
                    [Arg::V(o.vnum as i32), Arg::N(cmd.arg2), Arg::N(cmd.arg3)],
                    o.short_description.as_deref(),
                )
            }
            b'P' => {
                let o = &world.obj_protos[cmd.arg1 as usize];
                (
                    [
                        Arg::V(o.vnum as i32),
                        Arg::N(cmd.arg2),
                        Arg::V(world.obj_protos[cmd.arg3 as usize].vnum as i32),
                    ],
                    o.short_description.as_deref(),
                )
            }
            b'D' => {
                let r = &world.rooms[cmd.arg1 as usize];
                (
                    [Arg::V(r.vnum as i32), Arg::N(cmd.arg2), Arg::N(cmd.arg3)],
                    r.name.as_deref(),
                )
            }
            b'R' => {
                let o = &world.obj_protos[cmd.arg2 as usize];
                (
                    [
                        Arg::V(world.rooms[cmd.arg1 as usize].vnum as i32),
                        Arg::V(o.vnum as i32),
                        Arg::N(-1),
                    ],
                    o.short_description.as_deref(),
                )
            }
            b'T' => {
                let t = &world.triggers[cmd.arg2 as usize];
                (
                    [
                        Arg::N(cmd.arg1),
                        Arg::V(t.vnum as i32),
                        Arg::V(world.rooms[cmd.arg3 as usize].vnum as i32),
                    ],
                    t.name.as_deref(),
                )
            }
            b'V' => (
                [
                    Arg::N(cmd.arg1),
                    Arg::N(cmd.arg2),
                    Arg::V(world.rooms[cmd.arg3 as usize].vnum as i32),
                ],
                None,
            ),
            // '*'-disabled commands are dropped; unknown letters mudlog
            // "NOT saving" and are dropped too.
            _ => continue,
        };
        out.push(cmd.command);
        out.push(b' ');
        push_int(&mut out, i64::from(cmd.if_flag));
        for a in args {
            out.push(b' ');
            a.push(&mut out, fmt);
        }
        if cmd.command != b'V' {
            // "%c %d %d %d %d \t(%s)\n" — note the space before the tab.
            out.extend_from_slice(b" \t(");
            push_str_or_null(&mut out, comment);
            out.extend_from_slice(b")\n");
        } else {
            out.push(b' ');
            push_str_or_null(&mut out, cmd.sarg1.as_deref());
            out.push(b' ');
            push_str_or_null(&mut out, cmd.sarg2.as_deref());
            out.push(b'\n');
        }
    }

    out.extend_from_slice(b"S\n$\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MobProto, ObjProto, Room, Trigger, World, Zone, ZoneCommand};

    /// A post-renum world: one zone, and entity tables the command args
    /// index directly (rnums 0..).
    fn world() -> World {
        let mut w = World::default();
        w.zones.push(Zone {
            number: 30,
            builders: Some(b"DikuMUD".to_vec()),
            name: Some(b"Northern Midgaard".to_vec()),
            bot: 3000,
            top: 3099,
            lifespan: 15,
            reset_mode: 2,
            zone_flags: [8, 0, 0, 0],
            min_level: 1,
            max_level: 33,
            ..Default::default()
        });
        w.mob_protos.push(MobProto {
            vnum: 3011,
            short_descr: Some(b"the travelling saleswoman".to_vec()),
            ..Default::default()
        });
        w.obj_protos.push(ObjProto {
            vnum: 3006,
            short_description: Some(b"the teleporter".to_vec()),
            ..Default::default()
        });
        let mut room = Room { vnum: 3000, zone: 0, ..Default::default() };
        room.name = Some(b"The Temple Square".to_vec());
        w.room_map.insert(3000, 0);
        w.rooms.push(room);
        w.triggers.push(Trigger {
            vnum: 3017,
            name: Some(b"Guard greet (room 3000)".to_vec()),
            ..Default::default()
        });
        w.trig_map.insert(3017, 0);
        w
    }

    fn cmd(command: u8, if_flag: i32, a1: i32, a2: i32, a3: i32) -> ZoneCommand {
        ZoneCommand { command, if_flag, arg1: a1, arg2: a2, arg3: a3, ..Default::default() }
    }

    #[test]
    fn ten_field_header_and_comment_regeneration() {
        let mut w = world();
        w.zones[0].cmds = vec![
            cmd(b'M', 0, 0, 1, 0),  // mob rnum 0 → 3011, room rnum 0 → 3000
            cmd(b'O', 0, 0, 99, 0), // obj rnum 0 → 3006
            cmd(b'G', 1, 0, 99, 7), // arg3 forced to -1 on write
            cmd(b'E', 1, 0, 100, 16),
            cmd(b'P', 1, 0, 2, 0), // container obj rnum 0 → 3006
            cmd(b'D', 0, 0, 3, 1),
            cmd(b'R', 0, 0, 0, 5), // arg2 is the obj rnum; arg3 junk → -1
            cmd(b'T', 1, 2, 0, 0), // trig rnum 0 → 3017
            cmd(b'*', 0, 9, 9, 9), // disabled — dropped
            cmd(b'Z', 0, 9, 9, 9), // unknown — dropped
        ];
        let expect = b"#30\n\
            DikuMUD~\n\
            Northern Midgaard~\n\
            3000 3099 15 2 d 0 0 0 1 33\n\
            M 0 3011 1 3000 \t(the travelling saleswoman)\n\
            O 0 3006 99 3000 \t(the teleporter)\n\
            G 1 3006 99 -1 \t(the teleporter)\n\
            E 1 3006 100 16 \t(the teleporter)\n\
            P 1 3006 2 3006 \t(the teleporter)\n\
            D 0 3000 3 1 \t(The Temple Square)\n\
            R 0 3000 3006 -1 \t(the teleporter)\n\
            T 1 2 3017 3000 \t(Guard greet (room 3000))\n\
            S\n$\n";
        assert_eq!(
            String::from_utf8_lossy(&write_file(&w, 0)),
            String::from_utf8_lossy(expect)
        );
    }

    /// The same command list under both export forms. Every vnum column
    /// of the quick-reference chart moves; every count, position, door
    /// direction and state stays a plain number.
    #[test]
    fn export_marks_the_vnum_columns_and_leaves_the_rest() {
        let mut w = world();
        w.zones[0].cmds = vec![
            cmd(b'M', 0, 0, 1, 0),
            cmd(b'O', 0, 0, 99, 0),
            cmd(b'G', 1, 0, 99, 7),
            cmd(b'E', 1, 0, 100, 16),
            cmd(b'P', 1, 0, 2, 0),
            cmd(b'D', 0, 0, 3, 1),
            cmd(b'R', 0, 0, 0, 5),
            cmd(b'T', 1, 2, 0, 0),
        ];
        let qq = b"#QQ\n\
            DikuMUD~\n\
            Northern Midgaard~\n\
            QQ00 QQ99 15 2 d 0 0 0 1 33\n\
            M 0 QQ11 1 QQ00 \t(the travelling saleswoman)\n\
            O 0 QQ06 99 QQ00 \t(the teleporter)\n\
            G 1 QQ06 99 -1 \t(the teleporter)\n\
            E 1 QQ06 100 16 \t(the teleporter)\n\
            P 1 QQ06 2 QQ06 \t(the teleporter)\n\
            D 0 QQ00 3 1 \t(The Temple Square)\n\
            R 0 QQ00 QQ06 -1 \t(the teleporter)\n\
            T 1 2 QQ17 QQ00 \t(Guard greet (room 3000))\n\
            S\n$\n";
        assert_eq!(
            String::from_utf8_lossy(&write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))),
            String::from_utf8_lossy(qq)
        );

        let renumbered = b"#400\n\
            DikuMUD~\n\
            Northern Midgaard~\n\
            40000 40099 15 2 d 0 0 0 1 33\n\
            M 0 40011 1 40000 \t(the travelling saleswoman)\n\
            O 0 40006 99 40000 \t(the teleporter)\n\
            G 1 40006 99 -1 \t(the teleporter)\n\
            E 1 40006 100 16 \t(the teleporter)\n\
            P 1 40006 2 40006 \t(the teleporter)\n\
            D 0 40000 3 1 \t(The Temple Square)\n\
            R 0 40000 40006 -1 \t(the teleporter)\n\
            T 1 2 40017 40000 \t(Guard greet (room 3000))\n\
            S\n$\n";
        assert_eq!(
            String::from_utf8_lossy(&write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400))),
            String::from_utf8_lossy(renumbered)
        );
    }

    /// 63 shipped reset commands live in a zone other than the one they
    /// act on. `%100`-ing such a reference would fold it into
    /// a colliding in-zone vnum, silently retargeting the command on the
    /// recipient's MUD. It is marked instead, so the file refuses to boot
    /// until someone decides what it should point at.
    #[test]
    fn a_reset_command_reaching_out_of_the_zone_is_marked_zz() {
        let mut w = world();
        w.room_map.insert(7279, 1);
        w.rooms.push(Room { vnum: 7279, zone: 0, ..Default::default() });
        w.zones[0].cmds = vec![cmd(b'M', 0, 0, 1, 1)];
        let out = write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]));
        assert!(
            String::from_utf8_lossy(&out).contains("M 0 QQ11 1 ZZ79 \t("),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn four_field_header_when_no_flags_or_levels() {
        let mut w = world();
        w.zones[0].zone_flags = [0; 4];
        w.zones[0].min_level = -1;
        w.zones[0].max_level = -1;
        let out = write_file(&w, 0);
        assert!(
            String::from_utf8_lossy(&out).contains("Northern Midgaard~\n3000 3099 15 2\nS\n$\n"),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn levels_alone_force_ten_field_header() {
        let mut w = world();
        w.zones[0].zone_flags = [0; 4];
        w.zones[0].min_level = 0; // 0 is "set" — only -1/-1 collapses
        w.zones[0].max_level = -1;
        let out = write_file(&w, 0);
        assert!(
            String::from_utf8_lossy(&out).contains("\n3000 3099 15 2 0 0 0 0 0 -1\n"),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn builders_and_name_fallbacks_and_tab_conversion() {
        let mut w = world();
        w.zones[0].builders = Some(Vec::new()); // empty ⇒ "None."
        w.zones[0].name = Some(b"Zone \tRname".to_vec()); // '\t'→'@'
        let out = write_file(&w, 0);
        let s = String::from_utf8_lossy(&out);
        assert!(s.starts_with("#30\nNone.~\nZone @Rname~\n"), "{s}");
        w.zones[0].name = None; // NULL ⇒ "undefined"
        let s2 = String::from_utf8_lossy(&write_file(&w, 0)).into_owned();
        assert!(s2.starts_with("#30\nNone.~\nundefined~\n"), "{s2}");
    }

    #[test]
    fn v_command_line_has_no_comment() {
        let mut w = world();
        let mut v = cmd(b'V', 1, 2, 0, 0);
        v.sarg1 = Some(b"loadroom".to_vec());
        v.sarg2 = Some(b"3001 exact".to_vec());
        w.zones[0].cmds = vec![v];
        let out = write_file(&w, 0);
        assert!(
            String::from_utf8_lossy(&out).contains("\nV 1 2 0 3000 loadroom 3001 exact\n"),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn missing_comment_text_prints_null_marker() {
        let mut w = world();
        w.mob_protos[0].short_descr = None;
        w.zones[0].cmds = vec![cmd(b'M', 0, 0, 1, 0)];
        let out = write_file(&w, 0);
        assert!(
            String::from_utf8_lossy(&out).contains("M 0 3011 1 3000 \t((null))\n"),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }
}
