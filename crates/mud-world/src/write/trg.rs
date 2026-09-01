//! .trg writer: the disk loop of trigedit_save.
//!
//! Per vnum in the zone's bot..=top range with a real trigger:
//! "#%d\n", "name~\n" (NULL name => "unknown trigger"),
//! "attach_type sprintascii(type) narg\n", "arglist~\n" (NULL => empty),
//! then every cmdlist line followed by '\n', or "* Empty script" when the
//! list produced no text, then "~\n". File tail "$~\n".
//!
//! Strings are written RAW: neither strip_cr nor convert_from_tabs is
//! applied, so '\t' bytes that parse_at made from '@' color codes stay
//! literal tabs on disk.

use super::{sprintascii, VnumFmt};
use crate::model::World;

pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let zone = &world.zones[zone_rnum as usize];
    let mut out: Vec<u8> = Vec::new();

    for vnum in zone.bot..=zone.top {
        let Some(&rnum) = world.trig_map.get(&vnum) else {
            continue;
        };
        let trig = &world.triggers[rnum as usize];

        out.push(b'#');
        fmt.push_vnum(&mut out, i64::from(vnum));
        out.push(b'\n');

        match &trig.name {
            Some(n) => out.extend_from_slice(n),
            None => out.extend_from_slice(b"unknown trigger"),
        }
        out.extend_from_slice(b"~\n");

        out.extend_from_slice(trig.attach_type.to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(&sprintascii(trig.trigger_type));
        out.push(b' ');
        out.extend_from_slice(trig.narg.to_string().as_bytes());
        out.push(b'\n');

        if let Some(a) = &trig.arglist {
            out.extend_from_slice(a);
        }
        out.extend_from_slice(b"~\n");

        // An export leaves the script BODY alone — vnums inside DG
        // commands are prose, not fields, and nothing here guesses at them
        // it opens the body with a note saying
        // so. Ours says the same, and names the target zone when there is
        // one, since then the vnums in the body are the only ones that
        // did not move.
        if !fmt.is_plain() {
            push_export_note(&mut out, i64::from(zone.number), fmt.new_number());
        }
        // buf is every cmd + "\n"; "* Empty script" is substituted only
        // when it stayed empty, i.e. the cmdlist had no elements.
        if trig.cmdlist.is_empty() {
            out.extend_from_slice(b"* Empty script");
        } else {
            for cmd in &trig.cmdlist {
                out.extend_from_slice(cmd);
                out.push(b'\n');
            }
        }
        out.extend_from_slice(b"~\n");
    }

    out.extend_from_slice(b"$~\n");
    out
}

/// The comment block opens every exported trigger body
/// with. `new_number` is the zone the export renumbered into, if any.
fn push_export_note(out: &mut Vec<u8>, zone_number: i64, new_number: Option<i64>) {
    out.extend_from_slice(
        b"* This trigger has been exported 'as is'. This means that vnums\n\
          * in this file are not changed, and will have to be edited by hand.\n",
    );
    match new_number {
        Some(n) => out.extend_from_slice(
            format!(
                "* This zone was number {zone_number} on The Builder Academy and has\n\
                 * been renumbered to {n}, so you should be looking for {zone_number}xx\n\
                 * and changing it to {n}xx, where xx is 00-99.\n"
            )
            .as_bytes(),
        ),
        None => out.extend_from_slice(
            format!(
                "* This zone was number {zone_number} on The Builder Academy, so you\n\
                 * should be looking for {zone_number}xx, where xx is 00-99.\n"
            )
            .as_bytes(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::model::{Trigger, World, Zone};
    use crate::parse;

    #[test]
    fn empty_zone_writes_bare_terminator() {
        let mut w = World::default();
        w.zones.push(Zone { number: 5, bot: 500, top: 599, ..Default::default() });
        assert_eq!(write_file(&w, 0), b"$~\n");
    }

    /// An exported trigger keeps its body unchanged — DG commands are
    /// text,
    /// not fields — so the body opens with the note saying so.
    #[test]
    fn export_marks_the_header_and_explains_the_untouched_body() {
        let mut w = World::default();
        w.zones.push(Zone { number: 30, bot: 3000, top: 3099, ..Default::default() });
        w.triggers.push(Trigger {
            vnum: 3017,
            name: Some(b"Guard greet".to_vec()),
            attach_type: 0,
            trigger_type: 1,
            narg: 100,
            arglist: None,
            cmdlist: vec![b"%teleport% %actor% 3005".to_vec()],
        });
        w.trig_map.insert(3017, 0);

        assert_eq!(
            String::from_utf8_lossy(&write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))),
            "#QQ17\nGuard greet~\n0 a 100\n~\n\
             * This trigger has been exported 'as is'. This means that vnums\n\
             * in this file are not changed, and will have to be edited by hand.\n\
             * This zone was number 30 on The Builder Academy, so you\n\
             * should be looking for 30xx, where xx is 00-99.\n\
             %teleport% %actor% 3005\n~\n$~\n"
        );

        let renumbered = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        let renumbered = String::from_utf8_lossy(&renumbered).into_owned();
        assert!(renumbered.starts_with("#40017\n"), "{renumbered}");
        assert!(renumbered.contains("been renumbered to 400"), "{renumbered}");
        // The body is the one place the vnums did NOT move.
        assert!(renumbered.contains("%teleport% %actor% 3005\n"), "{renumbered}");
    }

    #[test]
    fn defaults_and_empty_script() {
        let mut w = World::default();
        w.zones.push(Zone { number: 0, bot: 0, top: 99, ..Default::default() });
        w.triggers.push(Trigger {
            vnum: 3,
            name: None,
            attach_type: 0,
            trigger_type: 0,
            narg: 0,
            arglist: None,
            cmdlist: vec![],
        });
        w.trig_map.insert(3, 0);
        assert_eq!(
            write_file(&w, 0),
            b"#3\nunknown trigger~\n0 0 0\n~\n* Empty script~\n$~\n"
        );
    }

    #[test]
    fn writes_in_vnum_order_with_tabs_raw() {
        let mut w = World::default();
        w.zones.push(Zone { number: 0, bot: 0, top: 99, ..Default::default() });
        for (i, vnum) in [9u16, 4].into_iter().enumerate() {
            w.triggers.push(Trigger {
                vnum,
                name: Some(b"N".to_vec()),
                attach_type: 2,
                trigger_type: (1 << 6) | (1 << 0),
                narg: 100,
                arglist: Some(b"south".to_vec()),
                cmdlist: vec![b"say \tRhi@@\tn".to_vec()],
            });
            w.trig_map.insert(vnum, i as u16);
        }
        assert_eq!(
            write_file(&w, 0),
            &b"#4\nN~\n2 ag 100\nsouth~\nsay \tRhi@@\tn\n~\n\
               #9\nN~\n2 ag 100\nsouth~\nsay \tRhi@@\tn\n~\n$~\n"[..]
        );
    }

    // ---- golden round-trips ----

}
