//! .mob parser: the record loop, parse_mobile, parse_simple_mob,
//! parse_enhanced_mob, parse_espec/interpret_espec, and the MOB_TRIGGER
//! reader.
//!
//! Numeric lines are scanned with a cursor (`Scanf`) rather than
//! `super::scan_ints`, because the count of fields converted stops at
//! trailing
//! junk *inside* a token ("12abc 5" scans one %d, not two) and the mob
//! grammar leans on exact return counts (10 vs 4 on the flags line, 9 on
//! the dice line). Stock config is assumed: bitwarning = FALSE, so legacy
//! 4-token flag lines take the 128-bit conversion path.

use mud_data::types::Idx;

use crate::lex::{asciiflag_conv, atol, Reader};
use crate::model::{MobProto, World};

/// Like asciiflag_conv, but letters map one bit HIGHER ('a' -> bit 1, ...,
/// 'F' -> bit 32 & 31 = bit 0 through the `1 << n` int mask). Used only by
/// the legacy conversion paths.
pub(crate) fn asciiflag_conv_aff(token: &[u8]) -> u32 {
    let mut flags: u32 = 0;
    let mut is_num = !token.is_empty();
    for (i, &c) in token.iter().enumerate() {
        if c.is_ascii_lowercase() {
            flags |= 1u32 << ((c - b'a' + 1) & 31);
        } else if c.is_ascii_uppercase() {
            flags |= 1u32 << ((26 + (c - b'A' + 1)) & 31);
        }
        if !c.is_ascii_digit() && (c != b'-' || i != 0) {
            is_num = false;
        }
    }
    if is_num {
        flags = atol(token) as u32;
    }
    flags
}

/// The whitespace bytes this grammar recognises.
pub(crate) fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// A cursor over one line, covering what the world loaders need: a token
/// (skip whitespace, take a non-empty non-whitespace run), an integer
/// (skip whitespace, optional sign, at least one digit, wrapping at 32
/// bits), a single byte (NO whitespace skip) and literal bytes (must match at
/// the cursor, no skip). Format-string whitespace is `skip_ws`.
pub(crate) struct Scanf<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Scanf<'a> {
    pub(crate) fn new(s: &'a [u8]) -> Self {
        Scanf { s, pos: 0 }
    }

    pub(crate) fn skip_ws(&mut self) {
        while self.pos < self.s.len() && is_ws(self.s[self.pos]) {
            self.pos += 1;
        }
    }

    /// %s
    pub(crate) fn word(&mut self) -> Option<&'a [u8]> {
        self.word_cap(usize::MAX)
    }

    /// %Ns — at most `cap` bytes of the token; the rest stays unread.
    pub(crate) fn word_cap(&mut self, cap: usize) -> Option<&'a [u8]> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len()
            && self.pos - start < cap
            && !is_ws(self.s[self.pos])
        {
            self.pos += 1;
        }
        if self.pos > start { Some(&self.s[start..self.pos]) } else { None }
    }

    /// %d
    pub(crate) fn int(&mut self) -> Option<i32> {
        self.skip_ws();
        let mut p = self.pos;
        let neg = match self.s.get(p) {
            Some(b'-') => {
                p += 1;
                true
            }
            Some(b'+') => {
                p += 1;
                false
            }
            _ => false,
        };
        let digits_at = p;
        let mut v: i64 = 0;
        while let Some(&c) = self.s.get(p) {
            if !c.is_ascii_digit() {
                break;
            }
            v = v.wrapping_mul(10).wrapping_add(i64::from(c - b'0'));
            p += 1;
        }
        if p == digits_at {
            return None;
        }
        self.pos = p;
        Some((if neg { -v } else { v }) as i32)
    }

    /// %c
    pub(crate) fn chr(&mut self) -> Option<u8> {
        let b = *self.s.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// A literal non-whitespace byte in the format string.
    pub(crate) fn lit(&mut self, b: u8) -> bool {
        if self.s.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

/// The vnum on a `#N` record header. `line[0]` is already `'#'`.
pub(crate) fn parse_hash_vnum(line: &[u8]) -> Option<i32> {
    let mut sc = Scanf::new(line);
    if !sc.lit(b'#') {
        return None;
    }
    sc.int()
}

/// if the first word (fname = leading
/// alphabetic run) of a short description is "a"/"an"/"the"
/// (case-insensitive), its first byte is forced lowercase.
pub(crate) fn lower_leading_article(s: &mut Option<Vec<u8>>) {
    if let Some(s) = s {
        if s.is_empty() {
            return;
        }
        let n = s.iter().take_while(|b| b.is_ascii_alphabetic()).count();
        let word = &s[..n];
        if word.eq_ignore_ascii_case(b"a")
            || word.eq_ignore_ascii_case(b"an")
            || word.eq_ignore_ascii_case(b"the")
        {
            s[0] = s[0].to_ascii_lowercase();
        }
    }
}

/// "#vnum" records until a line starting '$' (or a vnum >= 99999, which
/// ends the file the same way).
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
                    return Err(format!("SYSERR: mob file {filename} is empty!"));
                }
                None => {
                    return Err(format!(
                        "SYSERR: Format error in {filename} after mob #{nr}\n\
                         ...expecting a new mob, but file ended!\n\
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
                .ok_or_else(|| format!("SYSERR: Format error after mob #{last}"))?;
            if nr >= 99999 {
                return Ok(());
            }
            pending = parse_mobile(world, &mut r, nr)?;
        } else {
            return Err(format!(
                "SYSERR: Format error in mob file {filename} near mob #{nr}\n\
                 SYSERR: ... offending line: '{}'",
                String::from_utf8_lossy(&line)
            ));
        }
    }
}

/// Returns the first line after the record's T lines (the next record's
/// "#"/"$" line) for the record loop to reprocess, or None at EOF. The
/// get_line lookahead is equivalent for every well-formed file (it also
/// skips comment lines, which fread_letter would leave for get_line anyway).
fn parse_mobile(world: &mut World, r: &mut Reader, nr: i32) -> Result<Option<Vec<u8>>, String> {
    let err = format!("mob vnum {nr}");
    let mut mob = MobProto { vnum: nr as Idx, ..Default::default() };

    /* String data */
    mob.keywords = r.fread_string(&err)?;
    mob.short_descr = r.fread_string(&err)?;
    lower_leading_article(&mut mob.short_descr);
    mob.long_descr = r.fread_string(&err)?;
    mob.ddescription = r.fread_string(&err)?;

    /* Numeric data: sscanf "%s %s %s %s %s %s %s %s %d %c". */
    let line = r.get_line().ok_or_else(|| {
        format!(
            "SYSERR: Format error after string section of mob #{nr}\n\
             ...expecting line of form '# # # {{S | E}}', but file ended!"
        )
    })?;
    // R1: same guard as parse_room — these eight fields are bounded to
    // WORLD_FLAG_FIELD, and a line carrying a longer token is refused,
    // because %127s would otherwise hand the remainder to the next
    // conversion and shift every field along it.
    if crate::parse::wld::line_has_overlong_field(&line) {
        return Err(format!(
            "SYSERR: Mob #{nr} has a field longer than {} characters.",
            crate::parse::wld::WORLD_FLAG_FIELD
        ));
    }
    let mut sc = Scanf::new(&line);
    let mut words: Vec<&[u8]> = Vec::new();
    while words.len() < 8 {
        match sc.word() {
            Some(w) => words.push(w),
            None => break,
        }
    }
    let mut align: Option<i32> = None;
    let mut letter: Option<u8> = None;
    if words.len() == 8 {
        align = sc.int();
        if align.is_some() {
            sc.skip_ws(); // the format-string space before %c
            letter = sc.chr();
        }
    }
    let retval = words.len() + usize::from(align.is_some()) + usize::from(letter.is_some());

    let letter = if retval == 10 {
        for k in 0..4 {
            mob.act[k] = asciiflag_conv(words[k]);
            mob.affected_by[k] = asciiflag_conv(words[4 + k]);
        }
        mob.alignment = align.unwrap();
        letter.unwrap()
    } else if retval == 4 {
        // Legacy "act aff align letter" line: converted to 128 bits at
        // load.
        mob.act[0] = asciiflag_conv(words[0]);
        mob.affected_by[0] = asciiflag_conv_aff(words[1]);
        mob.alignment = atol(words[2]) as i32;
        // MOB_AGGRESSIVE(5) beats MOB_AGGR_GOOD(9)/_NEUTRAL(10)/_EVIL(8).
        if mob.act[0] & (1 << 5) != 0 {
            mob.act[0] &= !((1 << 9) | (1 << 10) | (1 << 8));
        }
        words[3][0]
    } else {
        return Err(format!(
            "SYSERR: Format error after string section of mob #{nr}\n \
             ...expecting line of form '# # # {{S | E}}'"
        ));
    };

    // MOB_ISNPC(3) force-set; reserved MOB_NOTDEADYET(19) force-cleared.
    mob.act[0] |= 1 << 3;
    mob.act[0] &= !(1 << 19);

    // CHARM(22), POISON(12) and SLEEP(15) say something is being done
    // to a mob rather than something it is, and nothing puts them back once
    // they are gone, so a mob file that sets one leaves the mob that way for
    // the rest of its life. medit strips all three on save; this is the same
    // check for the files medit has never touched. It used to sit in the
    // legacy conversion branch above, which reaches only pre-128-bit mob
    // files -- none of a modern world's. Say which mob is at fault rather
    // than fixing it quietly: the file still has the bit, and only a builder
    // can take it out for good.
    let illegal = (1 << 22) | (1 << 12) | (1 << 15);
    if mob.affected_by[0] & illegal != 0 {
        world.load_warnings.push(format!(
            "SYSERR: Mob #{nr} has illegal affection bits set:{}{}{} -- removing them.",
            if mob.affected_by[0] & (1 << 22) != 0 { " CHARM" } else { "" },
            if mob.affected_by[0] & (1 << 12) != 0 { " POISON" } else { "" },
            if mob.affected_by[0] & (1 << 15) != 0 { " SLEEP" } else { "" },
        ));
        mob.affected_by[0] &= !illegal;
    }

    match letter.to_ascii_uppercase() {
        b'S' => parse_simple_mob(r, &mut mob, nr)?,
        b'E' => parse_enhanced_mob(r, &mut mob, nr)?,
        _ => {
            return Err(format!(
                "SYSERR: Unsupported mob type '{}' in mob #{nr}",
                letter as char
            ));
        }
    }

    /* DG triggers -- script info follows the mob's S/E section. */
    let pending = loop {
        match r.get_line() {
            Some(line) if line.first() == Some(&b'T') => dg_read_trigger(&line, &mut mob),
            other => break other,
        }
    };

    let rnum = world.mob_protos.len() as Idx;
    world.mob_map.insert(mob.vnum, rnum);
    world.mob_protos.push(mob);
    Ok(pending)
}

/// Read a `T` line: a flag word of at most seven bytes, then a vnum. A
/// failure to scan is logged and dropped, never fatal. Each format is
/// parsed standalone here, so pruning
/// triggers whose vnum does not exist is left to boot.
fn dg_read_trigger(line: &[u8], mob: &mut MobProto) {
    let mut sc = Scanf::new(line);
    let Some(_junk) = sc.word_cap(7) else { return };
    let Some(vnum) = sc.int() else { return };
    mob.proto_script.push(vnum as Idx);
}

/// parse_simple_mob's three get_line'd numeric lines. Stores narrow to the
/// on-disk widths (level/position/sex/dice are bytes, hp fields and armor
/// are sh_ints); model.rs holds the post-narrowing value in i32.
fn parse_simple_mob(r: &mut Reader, mob: &mut MobProto, nr: i32) -> Result<(), String> {
    // The six abilities default to 11 and saves to 0; the model's None
    // espec fields already mean those defaults (the writer omits them).
    let line = r
        .get_line()
        .ok_or_else(|| format!("SYSERR: Format error in mob #{nr}, file ended after S flag!"))?;
    let t = scan_mob_dice(&line).ok_or_else(|| {
        format!(
            "SYSERR: Format error in mob #{nr}, first line after S flag\n\
             ...expecting line of form '# # # #d#+# #d#+#'"
        )
    })?;
    mob.level = i32::from(t[0] as i8);
    mob.hitroll = i32::from(20i32.wrapping_sub(t[1]) as i8);
    mob.armor = i32::from(t[2].wrapping_mul(10) as i16);
    // max_hit = 0 flags that hit/mana/mov hold the XdY+Z hp dice.
    mob.hit = i32::from(t[3] as i16);
    mob.mana = i32::from(t[4] as i16);
    mob.mov = i32::from(t[5] as i16);
    mob.damnodice = i32::from(t[6] as i8);
    mob.damsizedice = i32::from(t[7] as i8);
    mob.damroll = i32::from(t[8] as i8);

    let line = r.get_line().ok_or_else(|| {
        format!(
            "SYSERR: Format error in mob #{nr}, second line after S flag\n\
             ...expecting line of form '# #', but file ended!"
        )
    })?;
    let mut sc = Scanf::new(&line);
    let (gold, exp) = match (sc.int(), sc.int()) {
        (Some(g), Some(x)) => (g, x),
        _ => {
            return Err(format!(
                "SYSERR: Format error in mob #{nr}, second line after S flag\n\
                 ...expecting line of form '# #'"
            ));
        }
    };
    mob.gold = gold;
    mob.exp = exp;

    let line = r.get_line().ok_or_else(|| {
        format!(
            "SYSERR: Format error in last line of mob #{nr}\n\
             ...expecting line of form '# # #', but file ended!"
        )
    })?;
    let mut sc = Scanf::new(&line);
    let (pos, dpos, sex) = match (sc.int(), sc.int(), sc.int()) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => {
            return Err(format!(
                "SYSERR: Format error in last line of mob #{nr}\n\
                 ...expecting line of form '# # #'"
            ));
        }
    };
    mob.position = i32::from(pos as i8);
    mob.default_pos = i32::from(dpos as i8);
    mob.sex = i32::from(sex as i8);
    // Class 0, weight 200 and height 198 are also defaults (not modeled).
    Ok(())
}

/// Nine numbers: `level hitroll ac NdN+N NdN+N`. All nine must convert.
fn scan_mob_dice(line: &[u8]) -> Option<[i32; 9]> {
    let mut sc = Scanf::new(line);
    let mut t = [0i32; 9];
    t[0] = sc.int()?;
    t[1] = sc.int()?;
    t[2] = sc.int()?;
    t[3] = sc.int()?;
    if !sc.lit(b'd') {
        return None;
    }
    t[4] = sc.int()?;
    if !sc.lit(b'+') {
        return None;
    }
    t[5] = sc.int()?;
    t[6] = sc.int()?;
    if !sc.lit(b'd') {
        return None;
    }
    t[7] = sc.int()?;
    if !sc.lit(b'+') {
        return None;
    }
    t[8] = sc.int()?;
    Some(t)
}

/// parse_enhanced_mob: the simple body, then espec lines until a line that
/// is exactly "E". A '#' line first is a fatal unterminated-section error.
fn parse_enhanced_mob(r: &mut Reader, mob: &mut MobProto, nr: i32) -> Result<(), String> {
    parse_simple_mob(r, mob, nr)?;
    while let Some(line) = r.get_line() {
        if line == b"E" {
            return Ok(());
        }
        if line.first() == Some(&b'#') {
            return Err(format!("SYSERR: Unterminated E section in mob #{nr}"));
        }
        parse_espec(&line, mob);
    }
    Err(format!("SYSERR: Unexpected end of file reached after mob #{nr}"))
}

/// Split at the FIRST `:`. The keyword keeps any spaces before it, so a
/// line reading " Str:" matches nothing; the value starts after the
/// whitespace that follows. No
/// colon means value=None and the keyword cannot match.
fn parse_espec(line: &[u8], mob: &mut MobProto) {
    let (keyword, value) = match line.iter().position(|&b| b == b':') {
        Some(i) => {
            let mut v = &line[i + 1..];
            while let Some((&c, rest)) = v.split_first() {
                if !is_ws(c) {
                    break;
                }
                v = rest;
            }
            (&line[..i], Some(v))
        }
        None => (&line[..], None),
    };
    interpret_espec(keyword, value, mob);
}

/// interpret_espec: atoi the value, clamp per keyword, store. Unknown
/// keywords are logged and ignored, never fatal.
fn interpret_espec(keyword: &[u8], value: Option<&[u8]>, mob: &mut MobProto) {
    let Some(value) = value else { return };
    let num = atol(value) as i32;
    let kw = |k: &[u8]| keyword.eq_ignore_ascii_case(k);
    if kw(b"BareHandAttack") {
        mob.bare_hand_attack = Some(num.clamp(0, 14)); // NUM_ATTACK_TYPES - 1
    } else if kw(b"Str") {
        mob.str_ = Some(num.clamp(3, 25));
    } else if kw(b"StrAdd") {
        mob.str_add = Some(num.clamp(0, 100));
    } else if kw(b"Int") {
        mob.intel = Some(num.clamp(3, 25));
    } else if kw(b"Wis") {
        mob.wis = Some(num.clamp(3, 25));
    } else if kw(b"Dex") {
        mob.dex = Some(num.clamp(3, 25));
    } else if kw(b"Con") {
        mob.con = Some(num.clamp(3, 25));
    } else if kw(b"Cha") {
        mob.cha = Some(num.clamp(3, 25));
    } else if kw(b"SavingPara") {
        mob.saving_para = Some(num.clamp(0, 100));
    } else if kw(b"SavingRod") {
        mob.saving_rod = Some(num.clamp(0, 100));
    } else if kw(b"SavingPetri") {
        mob.saving_petri = Some(num.clamp(0, 100));
    } else if kw(b"SavingBreath") {
        mob.saving_breath = Some(num.clamp(0, 100));
    } else if kw(b"SavingSpell") {
        mob.saving_spell = Some(num.clamp(0, 100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> World {
        let mut world = World::default();
        parse_file(&mut world, data, "test.mob").expect("parse");
        world
    }

    const SIMPLE_TAIL: &[u8] = b"10 5 -3 2d8+40 3d4+7\n100 2000\n8 4 2\n";

    #[test]
    fn simple_mob_transforms_and_termination() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#7\nguard~\nthe guard~\nA guard.\n~\nBig.\n~\n");
        data.extend_from_slice(b"2 0 0 0 8 0 0 0 -250 S\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$~\n");
        let w = parse(&data);
        assert_eq!(w.mob_protos.len(), 1);
        let m = &w.mob_protos[0];
        assert_eq!(m.vnum, 7);
        assert_eq!(w.real_mobile(7), Some(0));
        assert_eq!(m.act[0], 2 | (1 << 3)); // MOB_ISNPC force-set
        assert_eq!(m.affected_by[0], 8);
        assert_eq!(m.alignment, -250);
        assert_eq!(m.level, 10);
        assert_eq!(m.hitroll, 20 - 5);
        assert_eq!(m.armor, -30);
        assert_eq!((m.hit, m.mana, m.mov), (2, 8, 40));
        assert_eq!((m.damnodice, m.damsizedice, m.damroll), (3, 4, 7));
        assert_eq!((m.gold, m.exp), (100, 2000));
        assert_eq!((m.position, m.default_pos, m.sex), (8, 4, 2));
        assert!(m.str_.is_none() && m.saving_para.is_none());
    }

    #[test]
    fn short_descr_article_lowercased() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\nwiz~\nThe Wizard~\nHere.\n~\nDesc.\n~\n");
        data.extend_from_slice(b"0 0 0 0 0 0 0 0 0 S\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$\n");
        let w = parse(&data);
        assert_eq!(w.mob_protos[0].short_descr.as_deref(), Some(&b"the Wizard"[..]));
        // Non-article first words are untouched.
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\nwiz~\nTheo the wizard~\nHere.\n~\nDesc.\n~\n");
        data.extend_from_slice(b"0 0 0 0 0 0 0 0 0 S\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$\n");
        let w = parse(&data);
        assert_eq!(w.mob_protos[0].short_descr.as_deref(), Some(&b"Theo the wizard"[..]));
    }

    #[test]
    fn enhanced_mob_especs_clamped_and_unknowns_ignored() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#5\nm~\nm~\nm\n~\nm\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(
            b"BareHandAttack: 99\nStr: 200\nStrAdd: -4\nsavingpara: 55\nCha:\n\
              NoColonKeyword\nBogus: 9\nE\n$\n",
        );
        let w = parse(&data);
        let m = &w.mob_protos[0];
        assert_eq!(m.bare_hand_attack, Some(14)); // clamp 0..NUM_ATTACK_TYPES-1
        assert_eq!(m.str_, Some(25)); // clamp 3..25
        assert_eq!(m.str_add, Some(0)); // clamp 0..100
        assert_eq!(m.saving_para, Some(55)); // keyword match is case-insensitive
        assert_eq!(m.cha, Some(3)); // empty value -> atoi 0 -> clamped to 3
        assert!(m.intel.is_none() && m.wis.is_none());
    }

    #[test]
    fn espec_keyword_with_space_before_colon_is_unrecognized() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#5\nm~\nm~\nm\n~\nm\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"Str : 20\nE\n$\n");
        let w = parse(&data);
        assert!(w.mob_protos[0].str_.is_none());
    }

    #[test]
    fn legacy_four_token_flags_line_converts() {
        // act "fj" = AGGRESSIVE(5)|AGGR_GOOD(9): conflict strips AGGR_GOOD.
        // aff "cl": conv_aff maps to bits 3 and 12; POISON(12) is stripped.
        let mut data = Vec::new();
        data.extend_from_slice(b"#9\nm~\nm~\nm\n~\nm\n~\nfj cl 750 S\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$\n");
        let w = parse(&data);
        let m = &w.mob_protos[0];
        assert_eq!(m.act[0], (1 << 5) | (1 << 3));
        assert_eq!(m.affected_by[0], 1 << 3);
        assert_eq!(m.alignment, 750);
        assert_eq!(m.act[1..], [0, 0, 0]);
    }

    #[test]
    fn trigger_lines_attach_and_chain_to_next_record() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\na~\na~\na\n~\na\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"E\nT 95\nT 300\n");
        data.extend_from_slice(b"#2\nb~\nb~\nb\n~\nb\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"E\n$\n");
        let w = parse(&data);
        assert_eq!(w.mob_protos[0].proto_script, vec![95, 300]);
        assert!(w.mob_protos[1].proto_script.is_empty());
        assert_eq!(w.real_mobile(2), Some(1));
    }

    #[test]
    fn vnum_99999_and_dollar_both_terminate() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\na~\na~\na\n~\na\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"E\n#99999\njunk that would not parse\n");
        let w = parse(&data);
        assert_eq!(w.mob_protos.len(), 1);
    }

    #[test]
    fn unterminated_e_section_is_fatal() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#1\na~\na~\na\n~\na\n~\n0 0 0 0 0 0 0 0 0 E\n");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"#2\n");
        let mut w = World::default();
        let e = parse_file(&mut w, &data, "t.mob").unwrap_err();
        assert!(e.contains("Unterminated E section"), "{e}");
    }

    #[test]
    fn unsupported_type_letter_is_fatal() {
        let data = b"#1\na~\na~\na\n~\na\n~\n0 0 0 0 0 0 0 0 0 Q\n";
        let mut w = World::default();
        let e = parse_file(&mut w, data, "t.mob").unwrap_err();
        assert!(e.contains("Unsupported mob type 'Q'"), "{e}");
    }

    #[test]
    fn crlf_input_parses_identically() {
        let lf: Vec<u8> = {
            let mut d = Vec::new();
            d.extend_from_slice(b"#1\na b~\nthe a~\nRoom line.\n~\nLook desc.\n~\n");
            d.extend_from_slice(b"516106 0 0 0 2128 0 0 0 1000 E\n");
            d.extend_from_slice(b"34 9 -10 6d6+340 5d5+5\n340 115600\n8 8 2\n");
            d.extend_from_slice(b"BareHandAttack: 12\nE\nT 95\n$\n");
            d
        };
        let crlf: Vec<u8> = lf
            .iter()
            .flat_map(|&b| if b == b'\n' { vec![b'\r', b'\n'] } else { vec![b] })
            .collect();
        let a = parse(&lf);
        let b = parse(&crlf);
        assert_eq!(a.mob_protos[0].long_descr, b.mob_protos[0].long_descr);
        assert_eq!(a.mob_protos[0].hitroll, 20 - 9);
        assert_eq!(a.mob_protos[0].armor, -100);
        assert_eq!(b.mob_protos[0].bare_hand_attack, Some(12));
        assert_eq!(b.mob_protos[0].proto_script, vec![95]);
    }

    #[test]
    fn asciiflag_conv_aff_shift() {
        assert_eq!(asciiflag_conv_aff(b"a"), 1 << 1);
        assert_eq!(asciiflag_conv_aff(b"z"), 1 << 26);
        assert_eq!(asciiflag_conv_aff(b"F"), 1 << 0); // 26+5+1 = 32, masked
        assert_eq!(asciiflag_conv_aff(b"12"), 12);
    }

    #[test]
    fn scanf_matches_c_sscanf_count_semantics() {
        // "900E" — %d then %c with no separating space still converts both.
        let mut sc = Scanf::new(b"900E");
        assert_eq!(sc.int(), Some(900));
        sc.skip_ws();
        assert_eq!(sc.chr(), Some(b'E'));
        // Trailing junk inside a token stops the NEXT conversion.
        let mut sc = Scanf::new(b"12abc 5");
        assert_eq!(sc.int(), Some(12));
        assert_eq!(sc.int(), None);
        // Dice literals require exact adjacency.
        assert!(scan_mob_dice(b"1 2 3 4d5+6 7d8+9").is_some());
        assert!(scan_mob_dice(b"1 2 3 4 d5+6 7d8+9").is_none());
        assert_eq!(scan_mob_dice(b"1 2 3 4d5+6 7d8+9").unwrap(), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    // ------------------------------------------------------------------ B96

    /// CHARM, POISON and SLEEP are conditions rather than properties, and
    /// nothing in the game puts them back once they are gone. The strip used
    /// to sit in the legacy conversion branch, so a mob in the 128-bit form
    /// -- which is to say every mob in a modern world -- kept them for life.
    #[test]
    fn illegal_affect_bits_are_stripped_from_every_prototype() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#7
guard~
the guard~
A guard.
~
Big.
~
");
        // CHARM(22) | POISON(12) | SLEEP(15) == 4231168.
        data.extend_from_slice(b"2 0 0 0 4231168 0 0 0 -250 S
");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$~
");
        let w = parse(&data);

        assert_eq!(w.mob_protos[0].affected_by[0], 0, "all three bits are gone");
        assert_eq!(
            w.load_warnings,
            vec![concat!(
                "SYSERR: Mob #7 has illegal affection bits set:",
                " CHARM POISON SLEEP -- removing them."
            )
            .to_string()]
        );
    }

    /// The message names what was actually set, nothing more.
    #[test]
    fn the_illegal_affect_message_names_only_the_bits_present() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#9
mob~
the mob~
A mob.
~
Big.
~
");
        data.extend_from_slice(b"0 0 0 0 32768 0 0 0 0 S
"); // SLEEP alone
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$~
");
        let w = parse(&data);

        assert_eq!(w.mob_protos[0].affected_by[0], 0);
        assert_eq!(
            w.load_warnings,
            vec!["SYSERR: Mob #9 has illegal affection bits set: SLEEP -- removing them."
                .to_string()]
        );
    }

    /// An unrelated affect is left alone, and says nothing.
    #[test]
    fn a_legal_affect_bit_survives_the_check() {
        let mut data = Vec::new();
        data.extend_from_slice(b"#11
mob~
the mob~
A mob.
~
Big.
~
");
        data.extend_from_slice(b"0 0 0 0 8 0 0 0 0 S
");
        data.extend_from_slice(SIMPLE_TAIL);
        data.extend_from_slice(b"$~
");
        let w = parse(&data);

        assert_eq!(w.mob_protos[0].affected_by[0], 8, "an unrelated flag is kept");
        assert!(w.load_warnings.is_empty(), "nothing logged: {:?}", w.load_warnings);
    }
}
