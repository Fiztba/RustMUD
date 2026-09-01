//! Importing a pre-3.x CircleMUD binary player file.
//!
//! The traditional converter for these files does not read a *format*; it
//! does
//!
//! ```c
//! Records are fixed-size images of the on-disk player struct, read back to
//! ```
//!
//! — a raw dump of a C struct. What is on disk therefore depends on the
//! word size, alignment and byte order of the machine that WROTE it, and
//! that dependency is normally hidden behind editing the struct and
//! recompiling. Anyone reaching for this tool is doing so years later,
//! under stress, without the original build.
//!
//! So this states its assumption instead of burying it. The default is
//! stock CircleMUD 3.0 on a 32-bit host, where `long`, `time_t` and a
//! pointer are all four bytes — which is what the machines that wrote these
//! files were. The offsets below are not hand arithmetic: they were taken
//! from a program compiled with ILP32-width typedefs, so the compiler
//! computed the padding.
//!
//! ```text
//! struct char_file_u 5176 bytes
//! name 0 (21) char_specials_saved 4248
//! description 21 (4096) player_specials_saved 4276
//! title 4117 (81) abilities 4576
//! sex/chclass/level 4198/9/4200 points 4584
//! hometown 4202 affected[32] 4616
//! birth 4204 last_logon 5128
//! played 4208 host 5132 (41)
//! weight/height 4212/4213
//! pwd 4214 (31)
//! ```
//!
//! A file that is not an exact multiple of 5176 bytes did not come from
//! this layout, and the import refuses rather than decoding whatever is
//! there into somebody's level and gold. Each record is checked again on
//! the way past: the name must be NUL-terminated printable ASCII, because
//! that is the first field and a wrong layout shows up there immediately.
//!
//! Output goes through the ordinary `save_char` writer, so an imported
//! character is written exactly as the server would have written it.

use std::io;
use std::path::Path;

use crate::players::{get_filename, save_char, FileKind, PfAffect, PlayerFile};

/// The size of `struct char_file_u` under the assumed layout.
pub const RECORD_SIZE: usize = 5176;

const OFF_NAME: usize = 0;
const LEN_NAME: usize = 21;
const OFF_DESCRIPTION: usize = 21;
const LEN_DESCRIPTION: usize = 4096;
const OFF_TITLE: usize = 4117;
const LEN_TITLE: usize = 81;
const OFF_SEX: usize = 4198;
const OFF_CLASS: usize = 4199;
const OFF_LEVEL: usize = 4200;
const OFF_BIRTH: usize = 4204;
const OFF_PLAYED: usize = 4208;
const OFF_WEIGHT: usize = 4212;
const OFF_HEIGHT: usize = 4213;
const OFF_PWD: usize = 4214;
const LEN_PWD: usize = 31;
const OFF_CSDS: usize = 4248;
const OFF_PSDS: usize = 4276;
const OFF_ABILITIES: usize = 4576;
const OFF_POINTS: usize = 4584;
const OFF_AFFECTED: usize = 4616;
const AFFECT_SIZE: usize = 16;
const MAX_AFFECT: usize = 32;
const OFF_LAST_LOGON: usize = 5128;
const OFF_HOST: usize = 5132;
const LEN_HOST: usize = 41;

/// Byte order of the machine that wrote the file. A 32-bit big-endian host
/// (SPARC, a PowerPC Mac) lays the struct out identically and differs only
/// in integer byte order, so this is the only axis that needs a switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

pub struct Report {
    pub imported: Vec<Vec<u8>>,
    pub skipped: Vec<(usize, String)>,
    /// Files that already existed and were left alone.
    pub existing: Vec<Vec<u8>>,
}

struct Cursor<'a> {
    b: &'a [u8],
    e: Endian,
}

impl<'a> Cursor<'a> {
    fn i8(&self, at: usize) -> i32 {
        self.b[at] as i8 as i32
    }
    fn u8(&self, at: usize) -> i32 {
        self.b[at] as i32
    }
    fn i16(&self, at: usize) -> i32 {
        let raw = [self.b[at], self.b[at + 1]];
        match self.e {
            Endian::Little => i16::from_le_bytes(raw) as i32,
            Endian::Big => i16::from_be_bytes(raw) as i32,
        }
    }
    fn u16(&self, at: usize) -> i32 {
        let raw = [self.b[at], self.b[at + 1]];
        match self.e {
            Endian::Little => u16::from_le_bytes(raw) as i32,
            Endian::Big => u16::from_be_bytes(raw) as i32,
        }
    }
    fn i32(&self, at: usize) -> i32 {
        let raw = [self.b[at], self.b[at + 1], self.b[at + 2], self.b[at + 3]];
        match self.e {
            Endian::Little => i32::from_le_bytes(raw),
            Endian::Big => i32::from_be_bytes(raw),
        }
    }
    /// A NUL-terminated string in a fixed-width field.
    fn cstr(&self, at: usize, cap: usize) -> Vec<u8> {
        let field = &self.b[at..at + cap];
        let end = field.iter().position(|&c| c == 0).unwrap_or(cap);
        field[..end].to_vec()
    }
}

/// Convert one record. `Err` means the record does not look like this
/// layout at all, which is worth stopping for rather than importing.
fn parse_record(rec: &[u8], e: Endian) -> Result<PlayerFile, String> {
    let c = Cursor { b: rec, e };

    let name = c.cstr(OFF_NAME, LEN_NAME);
    if name.is_empty() {
        return Err("empty name".into());
    }
    if !name.iter().all(|&b| b.is_ascii_graphic()) {
        return Err(format!(
            "name field is not printable ASCII ({:?}) -- the file is probably \
             not the assumed layout",
            String::from_utf8_lossy(&name)
        ));
    }

    let mut pf = PlayerFile { name: Some(name), ..Default::default() };

    pf.passwd = c.cstr(OFF_PWD, LEN_PWD);
    let title = c.cstr(OFF_TITLE, LEN_TITLE);
    pf.title = if title.is_empty() { None } else { Some(title) };
    let desc = c.cstr(OFF_DESCRIPTION, LEN_DESCRIPTION);
    pf.description = if desc.is_empty() { None } else { Some(desc) };
    let host = c.cstr(OFF_HOST, LEN_HOST);
    pf.host = if host.is_empty() { None } else { Some(host) };

    pf.sex = c.i8(OFF_SEX);
    pf.class = c.i8(OFF_CLASS);
    pf.level = c.i8(OFF_LEVEL);
    pf.birth = c.i32(OFF_BIRTH) as i64;
    pf.played = c.i32(OFF_PLAYED);
    pf.weight = c.u8(OFF_WEIGHT);
    pf.height = c.u8(OFF_HEIGHT);
    pf.last_logon = c.i32(OFF_LAST_LOGON) as i64;

    // char_special_data_saved
    pf.alignment = c.i32(OFF_CSDS);
    pf.idnum = c.i32(OFF_CSDS + 4) as i64;
    // The old bitvectors are a single 32-bit long; the modern ones are four
    // words, so the old value is the low word and the rest stay clear.
    pf.plr_flags[0] = c.i32(OFF_CSDS + 8) as u32;
    pf.aff_flags[0] = c.i32(OFF_CSDS + 12) as u32;
    for i in 0..5 {
        pf.saving_throws[i] = c.i16(OFF_CSDS + 16 + i * 2);
    }

    // player_special_data_saved
    for skill in 1..=200usize {
        let v = c.i8(OFF_PSDS + skill);
        if v != 0 {
            pf.skills.push((skill as i32, v));
        }
    }
    pf.wimpy = c.i32(OFF_PSDS + 208);
    pf.freeze_level = c.i8(OFF_PSDS + 212);
    pf.invis_level = c.i16(OFF_PSDS + 214);
    pf.load_room = c.u16(OFF_PSDS + 216);
    pf.prf_flags[0] = c.i32(OFF_PSDS + 220) as u32;
    pf.bad_pws = c.u8(OFF_PSDS + 224);
    // conditions[] is GET_COND order: DRUNK, HUNGER, THIRST.
    pf.drunk = c.i8(OFF_PSDS + 225);
    pf.hunger = c.i8(OFF_PSDS + 226);
    pf.thirst = c.i8(OFF_PSDS + 227);
    pf.practices = c.i32(OFF_PSDS + 236);

    // abilities
    pf.str_ = c.i8(OFF_ABILITIES);
    pf.str_add = c.i8(OFF_ABILITIES + 1);
    pf.intel = c.i8(OFF_ABILITIES + 2);
    pf.wis = c.i8(OFF_ABILITIES + 3);
    pf.dex = c.i8(OFF_ABILITIES + 4);
    pf.con = c.i8(OFF_ABILITIES + 5);
    pf.cha = c.i8(OFF_ABILITIES + 6);

    // points
    pf.mana = c.i16(OFF_POINTS);
    pf.max_mana = c.i16(OFF_POINTS + 2);
    pf.hit = c.i16(OFF_POINTS + 4);
    pf.max_hit = c.i16(OFF_POINTS + 6);
    pf.mov = c.i16(OFF_POINTS + 8);
    pf.max_move = c.i16(OFF_POINTS + 10);
    pf.ac = c.i16(OFF_POINTS + 12);
    pf.gold = c.i32(OFF_POINTS + 16);
    pf.bank = c.i32(OFF_POINTS + 20);
    pf.exp = c.i32(OFF_POINTS + 24);
    pf.hitroll = c.i8(OFF_POINTS + 28);
    pf.damroll = c.i8(OFF_POINTS + 29);

    // affected[MAX_AFFECT] -- `next` is a pointer from a dead process and
    // is ignored; a zero spell type means an unused slot.
    for i in 0..MAX_AFFECT {
        let at = OFF_AFFECTED + i * AFFECT_SIZE;
        let spell = c.i16(at);
        if spell == 0 {
            continue;
        }
        let mut bitvector = [0u32; 4];
        bitvector[0] = c.i32(at + 8) as u32;
        pf.affects.push(PfAffect {
            spell,
            duration: c.i16(at + 2),
            modifier: c.i8(at + 4),
            location: c.i8(at + 5),
            bitvector,
        });
    }

    Ok(pf)
}

/// Convert every record in `src` and write the results under
/// `lib/plrfiles`. An existing `.plr` is never overwritten.
///
/// Nothing is written until every record has parsed, so a file that turns
/// out to be the wrong layout leaves the player directory untouched.
pub fn import_binary_pfiles(
    lib: &Path,
    src: &Path,
    endian: Endian,
    dry_run: bool,
) -> io::Result<Report> {
    let data = std::fs::read(src)?;
    if data.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file is empty"));
    }
    if data.len() % RECORD_SIZE != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} is {} bytes, which is not a multiple of {} -- it was not \
                 written by the assumed layout (stock CircleMUD 3.0, 32-bit). \
                 Refusing rather than decoding it into player data.",
                src.display(),
                data.len(),
                RECORD_SIZE
            ),
        ));
    }

    let mut report = Report { imported: Vec::new(), skipped: Vec::new(), existing: Vec::new() };
    let mut parsed: Vec<PlayerFile> = Vec::new();

    for (i, rec) in data.chunks_exact(RECORD_SIZE).enumerate() {
        match parse_record(rec, endian) {
            Ok(pf) => parsed.push(pf),
            Err(why) => report.skipped.push((i, why)),
        }
    }

    // A layout mismatch shows up as most records failing, not one. Say so
    // plainly instead of writing the handful that happened to look valid.
    let total = data.len() / RECORD_SIZE;
    if !report.skipped.is_empty() && report.skipped.len() * 2 >= total {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} of {} records did not parse; the file is probably not the \
                 assumed layout. Nothing was written.",
                report.skipped.len(),
                total
            ),
        ));
    }

    for pf in &parsed {
        let name = pf.name.clone().unwrap_or_default();
        let Some(rel) = get_filename(FileKind::Plr, &name) else {
            report.skipped.push((0, format!("no filename for {:?}", String::from_utf8_lossy(&name))));
            continue;
        };
        let path = lib.join(rel);
        if path.exists() {
            report.existing.push(name);
            continue;
        }
        if !dry_run {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, save_char(pf))?;
        }
        report.imported.push(name);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one record with known values at the verified offsets.
    fn record() -> Vec<u8> {
        let mut r = vec![0u8; RECORD_SIZE];
        let put = |r: &mut Vec<u8>, at: usize, s: &[u8]| r[at..at + s.len()].copy_from_slice(s);
        let put32 = |r: &mut Vec<u8>, at: usize, v: i32| {
            r[at..at + 4].copy_from_slice(&v.to_le_bytes())
        };
        let put16 = |r: &mut Vec<u8>, at: usize, v: i16| {
            r[at..at + 2].copy_from_slice(&v.to_le_bytes())
        };

        put(&mut r, OFF_NAME, b"Grognard\0");
        put(&mut r, OFF_PWD, b"abScrambled\0");
        put(&mut r, OFF_TITLE, b"the Weary\0");
        put(&mut r, OFF_DESCRIPTION, b"A tired old warrior.\0");
        put(&mut r, OFF_HOST, b"10.0.0.7\0");
        r[OFF_SEX] = 1;
        r[OFF_CLASS] = 2;
        r[OFF_LEVEL] = 27;
        put32(&mut r, OFF_BIRTH, 900_000_000);
        put32(&mut r, OFF_PLAYED, 123_456);
        r[OFF_WEIGHT] = 180;
        r[OFF_HEIGHT] = 72;
        put32(&mut r, OFF_LAST_LOGON, 1_000_000_000);

        put32(&mut r, OFF_CSDS, -350); // alignment
        put32(&mut r, OFF_CSDS + 4, 42); // idnum
        put32(&mut r, OFF_CSDS + 8, 0x0000_0005); // act/plr flags
        put32(&mut r, OFF_CSDS + 12, 0x0000_0010); // affected_by
        for i in 0..5 {
            put16(&mut r, OFF_CSDS + 16 + i * 2, -(i as i16) - 1);
        }

        r[OFF_PSDS + 11] = 75; // skill 11 at 75%
        r[OFF_PSDS + 200] = 99; // the last skill
        put32(&mut r, OFF_PSDS + 208, 30); // wimpy
        r[OFF_PSDS + 212] = 34; // freeze_level
        put16(&mut r, OFF_PSDS + 214, 3); // invis_level
        put16(&mut r, OFF_PSDS + 216, 3001); // load_room
        put32(&mut r, OFF_PSDS + 220, 0x0000_0021); // pref
        r[OFF_PSDS + 224] = 2; // bad_pws
        r[OFF_PSDS + 225] = 5; // drunk
        r[OFF_PSDS + 226] = 15; // hunger
        r[OFF_PSDS + 227] = 20; // thirst
        put32(&mut r, OFF_PSDS + 236, 7); // practices

        for (i, v) in [11i8, 0, 13, 14, 15, 16, 17].iter().enumerate() {
            r[OFF_ABILITIES + i] = *v as u8;
        }

        put16(&mut r, OFF_POINTS, 40); // mana
        put16(&mut r, OFF_POINTS + 2, 100); // max_mana
        put16(&mut r, OFF_POINTS + 4, 55); // hit
        put16(&mut r, OFF_POINTS + 6, 120); // max_hit
        put16(&mut r, OFF_POINTS + 8, 60); // move
        put16(&mut r, OFF_POINTS + 10, 82); // max_move
        put16(&mut r, OFF_POINTS + 12, -30); // armor
        put32(&mut r, OFF_POINTS + 16, 1234); // gold
        put32(&mut r, OFF_POINTS + 20, 5678); // bank
        put32(&mut r, OFF_POINTS + 24, 987_654); // exp
        r[OFF_POINTS + 28] = 6i8 as u8; // hitroll
        r[OFF_POINTS + 29] = 7i8 as u8; // damroll

        // one affect in slot 3
        let a = OFF_AFFECTED + 3 * AFFECT_SIZE;
        put16(&mut r, a, 201); // spell
        put16(&mut r, a + 2, 12); // duration
        r[a + 4] = 2i8 as u8; // modifier
        r[a + 5] = 1; // location
        put32(&mut r, a + 8, 0x0000_0040); // bitvector
        put32(&mut r, a + 12, 0xDEAD_BEEFu32 as i32); // next: a dead pointer

        r
    }

    #[test]
    fn every_field_lands_where_the_compiler_said() {
        let pf = parse_record(&record(), Endian::Little).expect("parses");
        assert_eq!(pf.name.as_deref(), Some(&b"Grognard"[..]));
        assert_eq!(pf.passwd, b"abScrambled");
        assert_eq!(pf.title.as_deref(), Some(&b"the Weary"[..]));
        assert_eq!(pf.description.as_deref(), Some(&b"A tired old warrior."[..]));
        assert_eq!(pf.host.as_deref(), Some(&b"10.0.0.7"[..]));
        assert_eq!((pf.sex, pf.class, pf.level), (1, 2, 27));
        assert_eq!(pf.birth, 900_000_000);
        assert_eq!(pf.played, 123_456);
        assert_eq!((pf.weight, pf.height), (180, 72));
        assert_eq!(pf.last_logon, 1_000_000_000);

        assert_eq!(pf.alignment, -350);
        assert_eq!(pf.idnum, 42);
        assert_eq!(pf.plr_flags, [5, 0, 0, 0]);
        assert_eq!(pf.aff_flags, [16, 0, 0, 0]);
        assert_eq!(pf.saving_throws, [-1, -2, -3, -4, -5]);

        assert_eq!(pf.skills, vec![(11, 75), (200, 99)]);
        assert_eq!(pf.wimpy, 30);
        assert_eq!(pf.freeze_level, 34);
        assert_eq!(pf.invis_level, 3);
        assert_eq!(pf.load_room, 3001);
        assert_eq!(pf.prf_flags, [0x21, 0, 0, 0]);
        assert_eq!(pf.bad_pws, 2);
        assert_eq!((pf.drunk, pf.hunger, pf.thirst), (5, 15, 20));
        assert_eq!(pf.practices, 7);

        assert_eq!((pf.str_, pf.str_add, pf.intel), (11, 0, 13));
        assert_eq!((pf.wis, pf.dex, pf.con, pf.cha), (14, 15, 16, 17));

        assert_eq!((pf.mana, pf.max_mana), (40, 100));
        assert_eq!((pf.hit, pf.max_hit), (55, 120));
        assert_eq!((pf.mov, pf.max_move), (60, 82));
        assert_eq!(pf.ac, -30);
        assert_eq!((pf.gold, pf.bank, pf.exp), (1234, 5678, 987_654));
        assert_eq!((pf.hitroll, pf.damroll), (6, 7));

        assert_eq!(pf.affects.len(), 1, "only the populated slot is kept");
        let a = &pf.affects[0];
        assert_eq!((a.spell, a.duration, a.modifier, a.location), (201, 12, 2, 1));
        assert_eq!(a.bitvector, [0x40, 0, 0, 0]);
    }

    #[test]
    fn big_endian_reads_the_same_values() {
        // Same record, integers byte-swapped: the layout is identical on a
        // 32-bit big-endian host, only the byte order differs.
        let le = record();
        let mut be = le.clone();
        for at in [OFF_BIRTH, OFF_PLAYED, OFF_LAST_LOGON, OFF_CSDS + 4, OFF_POINTS + 16] {
            let v = i32::from_le_bytes([le[at], le[at + 1], le[at + 2], le[at + 3]]);
            be[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }
        let pf = parse_record(&be, Endian::Big).expect("parses");
        assert_eq!(pf.birth, 900_000_000);
        assert_eq!(pf.played, 123_456);
        assert_eq!(pf.idnum, 42);
        assert_eq!(pf.gold, 1234);
    }

    #[test]
    fn refuses_a_record_whose_name_is_not_text() {
        let mut r = record();
        r[OFF_NAME..OFF_NAME + 4].copy_from_slice(&[0x01, 0xFF, 0x7F, 0x02]);
        let err = parse_record(&r, Endian::Little).unwrap_err();
        assert!(err.contains("not printable"), "unexpected: {err}");
    }

    #[test]
    fn an_unwritten_slot_is_not_an_affect() {
        let mut r = record();
        // clear the one affect; nothing should be produced
        let a = OFF_AFFECTED + 3 * AFFECT_SIZE;
        r[a..a + AFFECT_SIZE].fill(0);
        let pf = parse_record(&r, Endian::Little).expect("parses");
        assert!(pf.affects.is_empty());
    }
}
