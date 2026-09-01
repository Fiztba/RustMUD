//! The .zon loader: the two-pass command count with its
//! "<letter><space>" / bare-"S" rules, the
//! 10-field vs 4-field header, the missing-builders auto-repair (with its
//! stale-`tmp` line-number quirk), first-'~' truncation for builders and
//! name, and the per-letter command grammars including T and V lines.
//! Args are stored raw (file vnums); boot's renum_zone_table converts.

use crate::lex::{asciiflag_conv, atol, parse_at, Reader};
use crate::model::{World, Zone, ZoneCommand};

/// A cursor over one line. Each read resumes exactly where the last one
/// stopped, which can be mid-token after a partial numeric match -- the
/// token-based `scan_ints` helper cannot express that.
pub(crate) struct Scan<'a> {
    s: &'a [u8],
    pos: usize,
}

pub(crate) fn is_ws(b: u8) -> bool {
    // The whitespace bytes this grammar recognises.
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

impl<'a> Scan<'a> {
    pub(crate) fn new(s: &'a [u8]) -> Self {
        Scan { s, pos: 0 }
    }

    pub(crate) fn skip_ws(&mut self) {
        while self.pos < self.s.len() && is_ws(self.s[self.pos]) {
            self.pos += 1;
        }
    }

    /// %d / %hd: skip whitespace, optional sign, one-or-more digits (else
    /// matching failure). Overflow wraps through a 32-bit store.
    pub(crate) fn int(&mut self) -> Option<i64> {
        self.skip_ws();
        let start = self.pos;
        let mut i = self.pos;
        if matches!(self.s.get(i), Some(b'-') | Some(b'+')) {
            i += 1;
        }
        let digits = i;
        while i < self.s.len() && self.s[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            self.pos = start;
            return None;
        }
        let v = atol(&self.s[start..i]);
        self.pos = i;
        Some(v)
    }

    /// %s / %Ns: skip whitespace, then up to `max` non-whitespace bytes;
    /// fails only when zero bytes match (end of input).
    pub(crate) fn word(&mut self, max: usize) -> Option<Vec<u8>> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len()
            && self.pos - start < max
            && !is_ws(self.s[self.pos])
        {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        Some(self.s[start..self.pos].to_vec())
    }

    /// %N[^\f\n\r\t\v]: no whitespace skip of its own (callers issue the
    /// format-string space via skip_ws); space itself is a member.
    pub(crate) fn scanset_line(&mut self, max: usize) -> Option<Vec<u8>> {
        let start = self.pos;
        while self.pos < self.s.len()
            && self.pos - start < max
            && !matches!(self.s[self.pos], b'\x0c' | b'\n' | b'\r' | b'\t' | b'\x0b')
        {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        Some(self.s[start..self.pos].to_vec())
    }
}

/// get_line consumes one or more physical lines (skipped blanks and
/// comments included) and load_zones accumulates that count into line_num.
/// Reader::line_no counts consumed physical lines, so the delta matches.
fn get_line_track(r: &mut Reader, line_num: &mut usize) -> Option<Vec<u8>> {
    let before = r.line_no;
    let line = r.get_line();
    *line_num += r.line_no - before;
    line
}

pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let zname = filename;

    // Command-count pre-pass: skip the first 3 content
    // lines, then count lines that are "<one of MOPGERDTV><space>" or are
    // exactly "S". "$", "S ", indented or tab-separated commands do NOT
    // count — the strict cmd_no+1 check below preserves those fatals.
    let mut pre = Reader::new(data);
    for _ in 0..3 {
        pre.get_line();
    }
    let mut num_of_cmds = 0usize;
    while let Some(buf) = pre.get_line() {
        let counted = (buf.len() >= 2 && buf[1] == b' ' && b"MOPGERDTV".contains(&buf[0]))
            || buf.as_slice() == b"S";
        if counted {
            num_of_cmds += 1;
        }
    }
    if num_of_cmds == 0 {
        return Err(format!("SYSERR: {zname} is empty!"));
    }

    // rewind(fl); line_num restarts at 0.
    let mut r = Reader::new(data);
    let mut line_num = 0usize;
    let mut z = Zone::default();

    // "#%hd": literal '#' first, %hd wraps through short.
    let buf = get_line_track(&mut r, &mut line_num)
        .ok_or_else(|| format!("SYSERR: Format error in {zname}, line {line_num}"))?;
    if buf.first() != Some(&b'#') {
        return Err(format!("SYSERR: Format error in {zname}, line {line_num}"));
    }
    z.number = match Scan::new(&buf[1..]).int() {
        Some(v) => v as i16 as u16,
        None => return Err(format!("SYSERR: Format error in {zname}, line {line_num}")),
    };

    // Builders: truncate at the FIRST '~', unlike fread_string. Empty
    // stays an empty (non-None) string.
    let mut buf = get_line_track(&mut r, &mut line_num)
        .ok_or_else(|| format!("SYSERR: Format error in {zname} - premature end of file"))?;
    if let Some(p) = buf.iter().position(|&b| b == b'~') {
        buf.truncate(p);
    }
    z.builders = Some(buf);

    // Name: same truncation + parse_at.
    let mut buf = get_line_track(&mut r, &mut line_num)
        .ok_or_else(|| format!("SYSERR: Format error in {zname} - premature end of file"))?;
    if let Some(p) = buf.iter().position(|&b| b == b'~') {
        buf.truncate(p);
    }
    parse_at(&mut buf);
    z.name = Some(buf);

    // Numeric line: 10 fields, else 4, else assume the builders line was
    // missing and re-scan the stored name.
    let buf = get_line_track(&mut r, &mut line_num)
        .ok_or_else(|| format!("SYSERR: Format error in {zname} - premature end of file"))?;
    let mut zone_fix = false;
    if let Some((bot, top, life, reset, flags, min, max)) = scan_header10(&buf) {
        z.bot = bot;
        z.top = top;
        z.lifespan = life;
        z.reset_mode = reset;
        for (i, f) in flags.iter().enumerate() {
            z.zone_flags[i] = asciiflag_conv(f);
        }
        z.min_level = min;
        z.max_level = max;
    } else {
        if let Some((bot, top, life, reset)) = scan_header4(&buf) {
            z.bot = bot;
            z.top = top;
            z.lifespan = life;
            z.reset_mode = reset;
        } else {
            // Logs "SYSERR: Format error in numeric constant line of %s,
            // attempting to fix." then re-scans Z.name.
            let name = z.name.as_deref().unwrap_or(b"");
            match scan_header4(name) {
                Some((bot, top, life, reset)) => {
                    z.bot = bot;
                    z.top = top;
                    z.lifespan = life;
                    z.reset_mode = reset;
                    z.name = z.builders.take();
                    z.builders = Some(b"None.".to_vec());
                    zone_fix = true;
                }
                None => {
                    return Err("SYSERR: Could not fix previous error, aborting game."
                        .to_string());
                }
            }
        }
        // Both 4-field paths leave flags 0 and default the levels.
        z.min_level = -1;
        z.max_level = -1;
    }
    if z.bot > z.top {
        return Err(format!(
            "SYSERR: Zone {} bottom ({}) > top ({}).",
            z.number, z.bot, z.top
        ));
    }

    // Reset command table.
    let mut held: Option<Vec<u8>> = if zone_fix { Some(buf) } else { None };
    let mut cmd_no = 0usize;
    loop {
        let buf = match held.take() {
            // zone_fix skips the read, then does line_num += tmp where tmp
            // is still 3 from the pre-pass skip loop (2194) — a
            // preserved off-by-three in the repaired command's line number.
            Some(b) => {
                line_num += 3;
                b
            }
            None => match get_line_track(&mut r, &mut line_num) {
                Some(b) => b,
                None => {
                    return Err(format!(
                        "SYSERR: Format error in {zname} - premature end of file"
                    ));
                }
            },
        };

        // skip_spaces, then the command letter is the next byte.
        let mut p = 0;
        while p < buf.len() && is_ws(buf[p]) {
            p += 1;
        }
        let command = buf.get(p).copied().unwrap_or(0);
        if command == b'*' {
            continue;
        }
        if command == b'S' || command == b'$' {
            break; // stored as 'S' in C; our Vec simply ends
        }
        let rest = &buf[(p + 1).min(buf.len())..];
        let mut sc = Scan::new(rest);
        let mut cmd = ZoneCommand { command, ..Default::default() };
        let mut tmp_if: i64 = 0;
        let mut error = false;

        // A command outside "MOGEPDTV" takes the 3-argument branch. An
        // all-blank line yields a NUL command, which counts as a member
        // here, so it takes the 4-argument branch and fails there.
        let in_4arg_set = command == 0 || b"MOGEPDTV".contains(&command);
        if !in_4arg_set {
            // 3-arg command (R and unknown letters): if_flag arg1 arg2.
            match (sc.int(), sc.int(), sc.int()) {
                (Some(a), Some(b), Some(c)) => {
                    tmp_if = a;
                    cmd.arg1 = b as i32;
                    cmd.arg2 = c as i32;
                }
                _ => error = true,
            }
        } else if command == b'V' {
            // " %d %d %d %d %79s %79[^\f\n\r\t\v]".
            match (sc.int(), sc.int(), sc.int(), sc.int()) {
                (Some(a), Some(b), Some(c), Some(d)) => {
                    tmp_if = a;
                    cmd.arg1 = b as i32;
                    cmd.arg2 = c as i32;
                    cmd.arg3 = d as i32;
                    let t1 = sc.word(79);
                    sc.skip_ws(); // the format-string space before the scanset
                    let t2 = sc.scanset_line(79);
                    match (t1, t2) {
                        (Some(s1), Some(s2)) => {
                            cmd.sarg1 = Some(s1);
                            cmd.sarg2 = Some(s2);
                        }
                        _ => error = true,
                    }
                }
                _ => error = true,
            }
        } else {
            // 4-arg command (M O G E P D T): if_flag arg1 arg2 arg3.
            match (sc.int(), sc.int(), sc.int(), sc.int()) {
                (Some(a), Some(b), Some(c), Some(d)) => {
                    tmp_if = a;
                    cmd.arg1 = b as i32;
                    cmd.arg2 = c as i32;
                    cmd.arg3 = d as i32;
                }
                _ => error = true,
            }
        }

        // ZCMD.if_flag is a `bool` stored as signed char: truncate.
        cmd.if_flag = (tmp_if as i8) as i32;

        if error {
            return Err(format!(
                "SYSERR: Format error in {zname}, line {line_num}: '{}'",
                String::from_utf8_lossy(&buf)
            ));
        }
        cmd.line = line_num;
        z.cmds.push(cmd);
        cmd_no += 1;
    }

    // the pre-pass count includes the terminating bare "S".
    if num_of_cmds != cmd_no + 1 {
        return Err(format!(
            "SYSERR: Zone command count mismatch for {zname}. Estimated: {num_of_cmds}, Actual: {}",
            cmd_no + 1
        ));
    }

    world.zones.push(z);
    Ok(())
}

/// " %hd %hd %d %d %s %s %s %s %d %d" — all 10 or None.
#[allow(clippy::type_complexity)]
fn scan_header10(
    line: &[u8],
) -> Option<(u16, u16, i32, i32, [Vec<u8>; 4], i32, i32)> {
    let mut sc = Scan::new(line);
    let bot = sc.int()?;
    let top = sc.int()?;
    let life = sc.int()?;
    let reset = sc.int()?;
    let f1 = sc.word(usize::MAX)?;
    let f2 = sc.word(usize::MAX)?;
    let f3 = sc.word(usize::MAX)?;
    let f4 = sc.word(usize::MAX)?;
    let min = sc.int()?;
    let max = sc.int()?;
    Some((
        bot as i16 as u16,
        top as i16 as u16,
        life as i32,
        reset as i32,
        [f1, f2, f3, f4],
        min as i32,
        max as i32,
    ))
}

/// " %hd %hd %d %d " (2154).
fn scan_header4(line: &[u8]) -> Option<(u16, u16, i32, i32)> {
    let mut sc = Scan::new(line);
    let bot = sc.int()?;
    let top = sc.int()?;
    let life = sc.int()?;
    let reset = sc.int()?;
    Some((bot as i16 as u16, top as i16 as u16, life as i32, reset as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> Result<World, String> {
        let mut w = World::default();
        parse_file(&mut w, data, "test.zon")?;
        Ok(w)
    }

    #[test]
    fn ten_field_header_with_letter_flags() {
        let w = parse(
            b"#30\nDikuMUD~\nNorthern Midgaard~\n3000 3099 15 2 d 0 0 0 1 33\n\
              M 0 3011 1 3000 \t(the saleswoman)\nS\n$\n",
        )
        .unwrap();
        let z = &w.zones[0];
        assert_eq!(z.number, 30);
        assert_eq!((z.bot, z.top, z.lifespan, z.reset_mode), (3000, 3099, 15, 2));
        assert_eq!(z.zone_flags, [8, 0, 0, 0]); // 'd' = bit 3
        assert_eq!((z.min_level, z.max_level), (1, 33));
        assert_eq!(z.builders.as_deref(), Some(&b"DikuMUD"[..]));
        assert_eq!(z.name.as_deref(), Some(&b"Northern Midgaard"[..]));
        let c = &z.cmds[0];
        assert_eq!(
            (c.command, c.if_flag, c.arg1, c.arg2, c.arg3),
            (b'M', 0, 3011, 1, 3000)
        );
        assert_eq!(c.line, 5);
    }

    #[test]
    fn four_field_header_defaults_levels_and_flags() {
        let w = parse(b"#5\nBob~\nTest~\n500 599 30 2\nM 0 1 1 500\nS\n$\n").unwrap();
        let z = &w.zones[0];
        assert_eq!(z.zone_flags, [0, 0, 0, 0]);
        assert_eq!((z.min_level, z.max_level), (-1, -1));
    }

    #[test]
    fn missing_builders_line_auto_repair() {
        // No builders line: the name slot got the numbers. The fix-up
        // builders="None.", shifts name, and reuses the held command line —
        // whose recorded line number carries the stale-tmp +3.
        let w = parse(b"#5\nThe Zone~\n500 599 30 2\nM 0 1 1 500\nS\n$\n").unwrap();
        let z = &w.zones[0];
        assert_eq!(z.builders.as_deref(), Some(&b"None."[..]));
        assert_eq!(z.name.as_deref(), Some(&b"The Zone"[..]));
        assert_eq!((z.bot, z.top, z.lifespan, z.reset_mode), (500, 599, 30, 2));
        assert_eq!((z.min_level, z.max_level), (-1, -1));
        assert_eq!(z.cmds.len(), 1);
        assert_eq!(z.cmds[0].command, b'M');
        assert_eq!(z.cmds[0].line, 7); // 4 real reads + stale tmp 3
    }

    #[test]
    fn unfixable_header_is_fatal() {
        let e = parse(b"#5\nBob~\nThe Zone~\nnot numbers\nM 0 1 1 500\nS\n$\n")
            .unwrap_err();
        assert_eq!(e, "SYSERR: Could not fix previous error, aborting game.");
    }

    #[test]
    fn bot_above_top_is_fatal() {
        let e = parse(b"#5\nBob~\nT~\n600 599 30 2\nS\n$\n").unwrap_err();
        assert_eq!(e, "SYSERR: Zone 5 bottom (600) > top (599).");
    }

    #[test]
    fn r_command_reads_three_args_and_ignores_trailer() {
        let w = parse(b"#5\nB~\nT~\n1 2 3 4\nR 0 3000 3006 -1 \t(junk)\nS\n$\n").unwrap();
        let c = &w.zones[0].cmds[0];
        // arg3 is never read for 'R', so it stays zero.
        assert_eq!((c.command, c.if_flag, c.arg1, c.arg2, c.arg3), (b'R', 0, 3000, 3006, 0));
    }

    #[test]
    fn unknown_letter_takes_three_arg_branch() {
        let w = parse(b"#5\nB~\nT~\n1 2 3 4\nZ 1 10 20 30\nS\n$\n");
        // 'Z' is outside "MOGEPDTV", so it parses via the 3-argument
        // branch -- but it is not in the pre-pass set either, so the file
        // dies on the command-count check instead.
        let e = w.unwrap_err();
        assert!(e.starts_with("SYSERR: Zone command count mismatch"), "{e}");
    }

    #[test]
    fn v_command_strings() {
        let w = parse(
            b"#5\nB~\nT~\n1 2 3 4\nV 1 2 0 500 loadroom 500 and more\nS\n$\n",
        )
        .unwrap();
        let c = &w.zones[0].cmds[0];
        assert_eq!((c.command, c.if_flag, c.arg1, c.arg2, c.arg3), (b'V', 1, 2, 0, 500));
        assert_eq!(c.sarg1.as_deref(), Some(&b"loadroom"[..]));
        // sarg2 is the rest of the line, spaces included.
        assert_eq!(c.sarg2.as_deref(), Some(&b"500 and more"[..]));
    }

    #[test]
    fn v_sarg2_stops_at_tab() {
        let w = parse(b"#5\nB~\nT~\n1 2 3 4\nV 0 0 0 500 name value\tignored\nS\n$\n")
            .unwrap();
        let c = &w.zones[0].cmds[0];
        assert_eq!(c.sarg2.as_deref(), Some(&b"value"[..]));
    }

    #[test]
    fn v_missing_value_is_fatal() {
        let e = parse(b"#5\nB~\nT~\n1 2 3 4\nV 0 0 0 500 nameonly\nS\n$\n").unwrap_err();
        assert!(e.starts_with("SYSERR: Format error in test.zon, line 5"), "{e}");
    }

    #[test]
    fn precount_rejects_trailing_space_s_and_tabbed_commands() {
        // "S " does not pre-count but still terminates → mismatch (quirk 12).
        let e = parse(b"#5\nB~\nT~\n1 2 3 4\nM 0 1 1 500\nS \n$\n").unwrap_err();
        assert!(e.starts_with("SYSERR: Zone command count mismatch"), "{e}");
        // Tab after the letter neither pre-counts nor changes parsing.
        let e = parse(b"#5\nB~\nT~\n1 2 3 4\nM\t0 1 1 500\nS\n$\n").unwrap_err();
        assert!(e.starts_with("SYSERR: Zone command count mismatch"), "{e}");
    }

    #[test]
    fn dollar_alone_terminates_but_fails_count_check() {
        // '$' ends the table like S but is never
        // pre-counted, so a file with no bare "S" line dies on the check.
        let e = parse(b"#5\nB~\nT~\n1 2 3 4\nM 0 1 1 500\n$\n").unwrap_err();
        assert!(e.starts_with("SYSERR: Zone command count mismatch"), "{e}");
    }

    #[test]
    fn star_comments_skipped_at_both_levels() {
        // Full-line '*' vanishes in get_line; indented '*' is skipped by the
        // parse loop. Neither consumes a command slot.
        let w = parse(
            b"#5\nB~\nT~\n1 2 3 4\n* comment\nM 0 1 1 500\n  * indented\nS\n$\n",
        )
        .unwrap();
        assert_eq!(w.zones[0].cmds.len(), 1);
        // Comment lines still advance the line counter for later commands.
        assert_eq!(w.zones[0].cmds[0].line, 6);
    }

    #[test]
    fn builders_and_name_truncate_at_first_tilde() {
        let w = parse(b"#5\nBob~junk after~\nName~more~\n1 2 3 4\nS\n$\n").unwrap();
        let z = &w.zones[0];
        assert_eq!(z.builders.as_deref(), Some(&b"Bob"[..]));
        assert_eq!(z.name.as_deref(), Some(&b"Name"[..]));
    }

    #[test]
    fn if_flag_truncates_through_signed_char() {
        // if_flag is stored through a char-typed bool: 300 → 44.
        let w = parse(b"#5\nB~\nT~\n1 2 3 4\nM 300 1 1 500\nS\n$\n").unwrap();
        assert_eq!(w.zones[0].cmds[0].if_flag, 44);
    }

    #[test]
    fn empty_zone_file_is_fatal() {
        assert_eq!(parse(b"#5\nB~\nT~\n1 2 3 4\n").unwrap_err(), "SYSERR: test.zon is empty!");
    }

    #[test]
    fn name_gets_parse_at() {
        let w = parse(b"#5\nB@r~\nT@rname~\n1 2 3 4\nS\n$\n").unwrap();
        let z = &w.zones[0];
        assert_eq!(z.builders.as_deref(), Some(&b"B@r"[..])); // builders: no parse_at
        assert_eq!(z.name.as_deref(), Some(&b"T\trname"[..]));
    }
}
