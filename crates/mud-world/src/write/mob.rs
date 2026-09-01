//! .mob writer: save_mobiles, write_mobile_record, write_mobile_espec,
// One "T <vnum>" line per attached prototype trigger.
//! the T lines.
//!
//! Shapes the format requires: act/aff flags are written numerically as
//! signed ints (never letters), the type letter is always 'E', THAC0 goes
//! back out as `20 - hitroll` and AC as `armor / 10`, only the long and
//! detailed descriptions are strip_cr'd (alias and short desc keep any CRs),
//! the whole header block gets convert_from_tabs ('\t' -> '@' except "\t\t"
//! pairs), and the file ends "$\n" — no tilde, unlike every other world
//! format.

use crate::model::{MobProto, World};
use crate::write::{push_int, VnumFmt};

const UNDEFINED: &[u8] = b"An undefined string.\n";

/// NULL or empty becomes "An undefined string.\n". The substitution
/// happens at write time rather than mutating the prototype, which
/// produces the same bytes on every save.
fn checked(s: &Option<Vec<u8>>) -> &[u8] {
    match s {
        Some(v) if !v.is_empty() => v,
        _ => UNDEFINED,
    }
}

/// Append one header string: optional strip_cr, then parse_tab ('\t' not
/// followed by '\t' becomes '@'; "\t\t" survives and is skipped as a pair),
/// then the "~\n" terminator. Equivalent to convert_from_tabs over the
/// assembled block, since the "~\n" separators cannot be affected.
fn push_header_str(out: &mut Vec<u8>, s: &[u8], strip_cr: bool) {
    let mut tmp: Vec<u8> = if strip_cr {
        s.iter().copied().filter(|&b| b != b'\r').collect()
    } else {
        s.to_vec()
    };
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

/// "%d" of a stored int flag word (the model keeps the u32 bit pattern).
fn flag_int(v: u32) -> i64 {
    i64::from(v as i32)
}

/// save_mobiles: every existing mob in the zone's vnum range, in
/// ascending vnum order, then the "$\n" terminator.
pub fn write_file(world: &World, zone_rnum: u16) -> Vec<u8> {
    write_file_fmt(world, zone_rnum, VnumFmt::Plain)
}

pub fn write_file_fmt(world: &World, zone_rnum: u16, fmt: VnumFmt) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(zone) = world.zones.get(zone_rnum as usize) else {
        return out; // C logs an invalid-zone SYSERR and writes nothing.
    };
    for vnum in zone.bot..=zone.top {
        if let Some(&rnum) = world.mob_map.get(&vnum) {
            write_mobile_record(&mut out, &world.mob_protos[rnum as usize], fmt);
        }
    }
    out.extend_from_slice(b"$\n");
    out
}

fn write_mobile_record(out: &mut Vec<u8>, mob: &MobProto, fmt: VnumFmt) {
    out.push(b'#');
    fmt.push_vnum(out, i64::from(mob.vnum));
    out.push(b'\n');
    push_header_str(out, checked(&mob.keywords), false);
    push_header_str(out, checked(&mob.short_descr), false);
    push_header_str(out, checked(&mob.long_descr), true);
    push_header_str(out, checked(&mob.ddescription), true);

    // "%d %d %d %d %d %d %d %d %d E\n"
    for k in 0..4 {
        push_int(out, flag_int(mob.act[k]));
        out.push(b' ');
    }
    for k in 0..4 {
        push_int(out, flag_int(mob.affected_by[k]));
        out.push(b' ');
    }
    push_int(out, i64::from(mob.alignment));
    out.extend_from_slice(b" E\n");

    // "%d %d %d %dd%d+%d %dd%d+%d\n"
    push_int(out, i64::from(mob.level));
    out.push(b' ');
    push_int(out, i64::from(20 - mob.hitroll));
    out.push(b' ');
    push_int(out, i64::from(mob.armor / 10));
    out.push(b' ');
    push_int(out, i64::from(mob.hit));
    out.push(b'd');
    push_int(out, i64::from(mob.mana));
    out.push(b'+');
    push_int(out, i64::from(mob.mov));
    out.push(b' ');
    push_int(out, i64::from(mob.damnodice));
    out.push(b'd');
    push_int(out, i64::from(mob.damsizedice));
    out.push(b'+');
    push_int(out, i64::from(mob.damroll));
    out.push(b'\n');

    // "%d %d\n%d %d %d\n"
    push_int(out, i64::from(mob.gold));
    out.push(b' ');
    push_int(out, i64::from(mob.exp));
    out.push(b'\n');
    push_int(out, i64::from(mob.position));
    out.push(b' ');
    push_int(out, i64::from(mob.default_pos));
    out.push(b' ');
    push_int(out, i64::from(mob.sex));
    out.push(b'\n');

    write_mobile_espec(out, mob);

    // One "T <vnum>" line per attached prototype trigger.
    for &t in &mob.proto_script {
        out.extend_from_slice(b"T ");
        fmt.push_vnum(out, i64::from(t));
        out.push(b'\n');
    }
}

/// write_mobile_espec: only non-default values, in emission order (Str
/// before StrAdd before Dex — not the parse-table order), then "E\n".
fn write_mobile_espec(out: &mut Vec<u8>, mob: &MobProto) {
    let mut espec = |key: &[u8], v: i32| {
        out.extend_from_slice(key);
        out.extend_from_slice(b": ");
        push_int(out, i64::from(v));
        out.push(b'\n');
    };
    let emit = [
        (&b"BareHandAttack"[..], mob.bare_hand_attack.unwrap_or(0), 0),
        (&b"Str"[..], mob.str_.unwrap_or(11), 11),
        (&b"StrAdd"[..], mob.str_add.unwrap_or(0), 0),
        (&b"Dex"[..], mob.dex.unwrap_or(11), 11),
        (&b"Int"[..], mob.intel.unwrap_or(11), 11),
        (&b"Wis"[..], mob.wis.unwrap_or(11), 11),
        (&b"Con"[..], mob.con.unwrap_or(11), 11),
        (&b"Cha"[..], mob.cha.unwrap_or(11), 11),
        (&b"SavingPara"[..], mob.saving_para.unwrap_or(0), 0),
        (&b"SavingRod"[..], mob.saving_rod.unwrap_or(0), 0),
        (&b"SavingPetri"[..], mob.saving_petri.unwrap_or(0), 0),
        (&b"SavingBreath"[..], mob.saving_breath.unwrap_or(0), 0),
        (&b"SavingSpell"[..], mob.saving_spell.unwrap_or(0), 0),
    ];
    for (key, value, default) in emit {
        if value != default {
            espec(key, value);
        }
    }
    out.extend_from_slice(b"E\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Zone;
    use crate::parse;
    use std::path::PathBuf;

    fn zone(number: u16, bot: u16, top: u16) -> Zone {
        Zone { number, bot, top, ..Default::default() }
    }

    /// A mobile's only vnum sites are its header and its T lines.
    #[test]
    fn export_marks_the_header_and_the_trigger_lines() {
        let mut w = World::default();
        w.zones.push(zone(30, 3000, 3099));
        w.mob_protos.push(MobProto {
            vnum: 3005,
            proto_script: vec![3017, 1204],
            ..Default::default()
        });
        w.mob_map.insert(3005, 0);

        let qq = String::from_utf8(write_file_fmt(&w, 0, VnumFmt::qq(&w.zones[0]))).unwrap();
        assert!(qq.starts_with("#QQ05\n"), "{qq}");
        assert!(qq.ends_with("\nT QQ17\nT ZZ04\n$\n"), "{qq}");

        let re = write_file_fmt(&w, 0, VnumFmt::renumber(&w.zones[0], 400));
        let re = String::from_utf8(re).unwrap();
        assert!(re.starts_with("#40005\n"), "{re}");
        assert!(re.ends_with("\nT 40017\nT ZZ04\n$\n"), "{re}");
    }

    #[test]
    fn undefined_strings_and_espec_defaults() {
        let mut world = World::default();
        world.zones.push(zone(1, 100, 100));
        let mob = MobProto {
            vnum: 100,
            str_: Some(11),          // explicit default: omitted like C's != 11
            dex: Some(18),
            saving_spell: Some(2),
            bare_hand_attack: Some(12),
            proto_script: vec![95],
            ..Default::default()
        };
        world.mob_map.insert(100, 0);
        world.mob_protos.push(mob);
        let out = write_file(&world, 0);
        let expect = b"#100\n\
            An undefined string.\n~\n\
            An undefined string.\n~\n\
            An undefined string.\n~\n\
            An undefined string.\n~\n\
            0 0 0 0 0 0 0 0 0 E\n\
            0 20 0 0d0+0 0d0+0\n\
            0 0\n\
            0 0 0\n\
            BareHandAttack: 12\n\
            Dex: 18\n\
            SavingSpell: 2\n\
            E\n\
            T 95\n\
            $\n";
        assert_eq!(out, expect.as_slice());
    }

    #[test]
    fn flags_print_as_signed_ints_and_tabs_become_at_signs() {
        let mut world = World::default();
        world.zones.push(zone(1, 5, 5));
        let mob = MobProto {
            vnum: 5,
            keywords: Some(b"guide".to_vec()),
            short_descr: Some(b"the \tRguide\tn".to_vec()),
            long_descr: Some(b"A guide.\r\n".to_vec()),
            ddescription: Some(b"Tabby \t\t literal.\r\n".to_vec()),
            act: [0x8000_0000, 0, 0, 0],
            affected_by: [u32::MAX, 0, 0, 0],
            ..Default::default()
        };
        world.mob_map.insert(5, 0);
        world.mob_protos.push(mob);
        let out = write_file(&world, 0);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("the @Rguide@n~\n"), "{text}");
        assert!(text.contains("A guide.\n~\n"), "{text}");
        // "\t\t" survives parse_tab untouched.
        assert!(text.contains("Tabby \t\t literal.\n~\n"), "{text}");
        assert!(text.contains("-2147483648 0 0 0 -1 0 0 0 0 E\n"), "{text}");
    }
}

