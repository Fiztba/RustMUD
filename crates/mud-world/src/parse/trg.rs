//! .trg parser: the record loop and parse_trigger.
//!
//! Record grammar per trigger: name (fread_string), one get_line of
//! "attach_type flags [narg]" -- narg is 0 when only two fields are
//! present -- then arglist (fread_string), then the WHOLE script body as
//! one fread_string, split on line endings. Empty tokens vanish,
//! so blank lines inside a body are dropped. fread_string already ran
//! parse_at, so '@' color codes are '\t' in the stored lines.

use crate::lex::{Reader, asciiflag_conv};
use crate::model::{Trigger, World};

/// The whitespace bytes the world-file grammars recognise.
pub(crate) fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// An integer at `*i`: whitespace, optional sign, then digits. None when no
/// digit follows. The value wraps through a 32-bit store.
pub(crate) fn scan_int(s: &[u8], i: &mut usize) -> Option<i32> {
    while s.get(*i).copied().is_some_and(is_ws) {
        *i += 1;
    }
    let mut j = *i;
    let neg = match s.get(j) {
        Some(b'-') => {
            j += 1;
            true
        }
        Some(b'+') => {
            j += 1;
            false
        }
        _ => false,
    };
    let digits_at = j;
    let mut v: i64 = 0;
    while let Some(&c) = s.get(j) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        j += 1;
    }
    if j == digits_at {
        return None;
    }
    *i = j;
    Some(if neg { v.wrapping_neg() as i32 } else { v as i32 })
}

/// One token: skip whitespace, then take the non-whitespace run.
pub(crate) fn scan_word(s: &[u8], i: &mut usize) -> Option<Vec<u8>> {
    while s.get(*i).copied().is_some_and(is_ws) {
        *i += 1;
    }
    let start = *i;
    while s.get(*i).copied().is_some_and(|b| !is_ws(b)) {
        *i += 1;
    }
    if *i == start { None } else { Some(s[start..*i].to_vec()) }
}

/// The vnum on a `#N` record header: a literal `'#'`, then an integer,
/// which may be preceded by whitespace and a sign.
pub(crate) fn scan_after_hash(line: &[u8]) -> Option<i32> {
    let mut i = 0;
    if line.first() != Some(&b'#') {
        return None;
    }
    i += 1;
    scan_int(line, &mut i)
}

/// The .trg record loop. `$` ends the file, `#vnum` starts
/// a record, `#99999`+ acts as EOF, anything else is a fatal format error.
pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let mut r = Reader::new(data);
    let mut nr: i32 = -1;
    loop {
        let line = match r.get_line() {
            Some(l) => l,
            None => {
                return Err(if nr == -1 {
                    format!("trg file {filename} is empty!")
                } else {
                    format!(
                        "Format error in {filename} after trg #{nr}: \
                         expecting a new trg, but file ended! \
                         (maybe the file is not terminated with '$'?)"
                    )
                });
            }
        };
        if line.first() == Some(&b'$') {
            return Ok(());
        }
        if line.first() == Some(&b'#') {
            let last = nr;
            nr = match scan_after_hash(&line) {
                Some(v) => v,
                None => return Err(format!("Format error after trg #{last}")),
            };
            if nr >= 99999 {
                return Ok(());
            }
            parse_trigger(world, &mut r, nr)?;
        } else {
            return Err(format!(
                "Format error in trg file {filename} near trg #{nr}: \
                 offending line: '{}'",
                String::from_utf8_lossy(&line)
            ));
        }
    }
}

fn parse_trigger(world: &mut World, r: &mut Reader, nr: i32) -> Result<(), String> {
    let errors = format!("trig vnum {nr}");

    let name = r.fread_string(&errors)?;

    // One line of "attach_type flags [narg]". A third field sets narg;
    // fields that are absent stay 0.
    let line = r
        .get_line()
        .ok_or_else(|| format!("format error: missing attach-type line ({errors})"))?;
    let mut i = 0;
    let attach_raw = scan_int(&line, &mut i);
    let flags_tok = if attach_raw.is_some() { scan_word(&line, &mut i) } else { None };
    let narg_tok = if flags_tok.is_some() { scan_int(&line, &mut i) } else { None };
    // attach_type is stored through a signed char.
    let attach_type = (attach_raw.unwrap_or(0) as i8) as i32;
    let trigger_type = flags_tok.as_deref().map_or(0, asciiflag_conv);
    // k == 3 exactly means all three matched.
    let narg = narg_tok.unwrap_or(0);

    let arglist = r.fread_string(&errors)?;

    // The whole body is ONE tilde string, then walked with
    // Split on line endings; empty tokens are skipped, so blank lines vanish.
    // An absent body, or one containing only line separators, is an error.
    let body = r.fread_string(&errors)?;
    let cmdlist: Vec<Vec<u8>> = body
        .as_deref()
        .unwrap_or_default()
        .split(|&b| b == b'\n' || b == b'\r')
        .filter(|t| !t.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    if cmdlist.is_empty() {
        return Err(format!("empty script body crashes C's strtok ({errors})"));
    }

    // The stored vnum is a u16, so a larger value truncates.
    let vnum = nr as u16;
    let rnum = world.triggers.len() as u16;
    world.triggers.push(Trigger {
        vnum,
        name,
        attach_type,
        trigger_type,
        narg,
        arglist,
        cmdlist,
    });
    world.trig_map.insert(vnum, rnum);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> World {
        let mut w = World::default();
        parse_file(&mut w, data, "test.trg").expect("parse");
        w
    }

    #[test]
    fn basic_record() {
        let w = parse(b"#1\nGreet Trig~\n0 g 100\n~\nsay hi\nend\n~\n$~\n");
        assert_eq!(w.triggers.len(), 1);
        let t = &w.triggers[0];
        assert_eq!(t.vnum, 1);
        assert_eq!(t.name.as_deref(), Some(&b"Greet Trig"[..]));
        assert_eq!(t.attach_type, 0);
        assert_eq!(t.trigger_type, 1 << 6); // 'g'
        assert_eq!(t.narg, 100);
        assert_eq!(t.arglist, None);
        assert_eq!(t.cmdlist, vec![b"say hi".to_vec(), b"end".to_vec()]);
        assert_eq!(w.trig_map.get(&1), Some(&0));
    }

    #[test]
    fn blank_lines_in_body_vanish_via_strtok() {
        let w = parse(b"#5\nT~\n2 q 100\n~\nline one\n\n\r\n\nline two\n~\n$~\n");
        assert_eq!(w.triggers[0].cmdlist, vec![b"line one".to_vec(), b"line two".to_vec()]);
    }

    #[test]
    fn missing_narg_defaults_to_zero() {
        let w = parse(b"#2\nT~\n1 ab\n~\n* body\n~\n$~\n");
        let t = &w.triggers[0];
        assert_eq!(t.attach_type, 1);
        assert_eq!(t.trigger_type, 0b11);
        assert_eq!(t.narg, 0);
    }

    #[test]
    fn numeric_flags_and_at_conversion() {
        // '@' becomes '\t' via parse_at inside fread_string; "@@" is kept.
        let w = parse(b"#3\nName @Red@@~\n0 128 5\narg~\nsay @n done@@\n~\n$~\n");
        let t = &w.triggers[0];
        assert_eq!(t.name.as_deref(), Some(&b"Name \tRed@@"[..]));
        assert_eq!(t.trigger_type, 128);
        assert_eq!(t.arglist.as_deref(), Some(&b"arg"[..]));
        assert_eq!(t.cmdlist, vec![b"say \tn done@@".to_vec()]);
    }

    #[test]
    fn vnum_99999_acts_as_eof_and_dollar_ends() {
        let mut w = World::default();
        parse_file(&mut w, b"#1\nA~\n0 g 100\n~\n* x\n~\n#99999\njunk", "t.trg").unwrap();
        assert_eq!(w.triggers.len(), 1);
    }

    #[test]
    fn empty_body_is_error() {
        let mut w = World::default();
        assert!(parse_file(&mut w, b"#1\nA~\n0 g 100\n~\n~\n$~\n", "t.trg").is_err());
    }

    #[test]
    fn format_errors_are_fatal() {
        let mut w = World::default();
        assert!(parse_file(&mut w, b"", "t.trg").is_err()); // empty file
        assert!(parse_file(&mut w, b"junk\n", "t.trg").is_err()); // no # or $
        assert!(parse_file(&mut w, b"#nodigits\n", "t.trg").is_err()); // not a number
        // EOF after a record without '$' terminator is fatal too.
        assert!(parse_file(&mut w, b"#1\nA~\n0 g 100\n~\n* x\n~\n", "t.trg").is_err());
    }

    #[test]
    fn crlf_input_and_hash_with_space() {
        // Whitespace after the '#' is skipped.
        let w = parse(b"# 7\r\nN~\r\n0 g 100\r\n~\r\n* b\r\n~\r\n$~\r\n");
        assert_eq!(w.triggers[0].vnum, 7);
    }
}
