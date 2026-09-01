//! Text screens, the help table, and lib/text loading —
//! file_to_string/load_help plus the fread_action helpers.

use std::path::Path;

pub type BStr = Vec<u8>;

const READ_SIZE: usize = 256;

/// parse_at: '@' → '\t' unless doubled ('@@' → '@').
pub fn parse_at(b: &mut Vec<u8>) {
    // parse_at: `@` becomes `\t` unless it is doubled, and a
    // doubled `@@` is stepped OVER, not collapsed -- both bytes stay. That
    // is what makes the round trip through parse_tab stable, and it is why
    // an escaped `@` reaches the player as two characters.
    mud_net::editor::parse_at(&mut b[..]);
}

/// Read a text file in chunks of at most 255 bytes. Each non-empty chunk
/// loses its final byte and gains a line ending, so a physical line longer
/// than the chunk is broken mid-line, and a file whose last line has no
/// newline loses its last byte.
pub fn file_to_string(path: &Path) -> Option<BStr> {
    let data = std::fs::read(path).ok()?;
    let mut out: BStr = Vec::with_capacity(data.len() + data.len() / 32);
    let mut i = 0usize;
    while i < data.len() {
        // Up to READ_SIZE-1 bytes, stopping after '\n'.
        let mut chunk_end = i;
        let limit = (i + READ_SIZE - 1).min(data.len());
        while chunk_end < limit {
            let c = data[chunk_end];
            chunk_end += 1;
            if c == b'\n' {
                break;
            }
        }
        let mut chunk = data[i..chunk_end].to_vec();
        i = chunk_end;
        if !chunk.is_empty() {
            chunk.pop(); // tmp[len-1] = '\0'
        }
        out.extend_from_slice(&chunk);
        out.extend_from_slice(b"\r\n");
        if out.len() + 1 > mud_data::types::MAX_STRING_LENGTH {
            return Some(Vec::new()); // C zeroes the buffer and errors
        }
    }
    Some(out)
}

/// prune_crlf: strip all trailing CR/LF.
pub fn prune_crlf(b: &mut BStr) {
    while b.last().is_some_and(|c| *c == b'\r' || *c == b'\n') {
        b.pop();
    }
}

/// The loaded text screens.
#[derive(Debug, Default)]
pub struct Texts {
    pub credits: BStr,
    pub news: BStr,
    pub motd: BStr,
    pub imotd: BStr,
    pub greetings: BStr,
    pub help_screen: BStr,
    pub ihelp_screen: BStr,
    pub info: BStr,
    pub wizlist: BStr,
    pub immlist: BStr,
    pub background: BStr,
    pub policies: BStr,
    pub handbook: BStr,
    /// File mtimes for the (news)/(motd) prompt flags.
    pub newsmod: i64,
    pub motdmod: i64,
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Texts {
    /// Load all text screens from <lib>/text (order).
    pub fn load(lib: &Path, log: &mut Vec<String>) -> Texts {
        let text = lib.join("text");
        let help_dir = text.join("help");
        let mut load_file = |p: &Path| -> BStr {
            match file_to_string(p) {
                Some(mut s) => {
                    // file_to_string applies parse_at: @-codes in
                    // text screens become live color escapes.
                    parse_at(&mut s);
                    s
                }
                None => {
                    log.push(format!("SYSERR: reading {}: No such file or directory", p.display()));
                    Vec::new()
                }
            }
        };
        let mut t = Texts {
            news: load_file(&text.join("news")),
            credits: load_file(&text.join("credits")),
            motd: load_file(&text.join("motd")),
            imotd: load_file(&text.join("imotd")),
            help_screen: load_file(&help_dir.join("help")),
            ihelp_screen: load_file(&help_dir.join("ihelp")),
            info: load_file(&text.join("info")),
            wizlist: load_file(&text.join("wizlist")),
            immlist: load_file(&text.join("immlist")),
            policies: load_file(&text.join("policies")),
            handbook: load_file(&text.join("handbook")),
            background: load_file(&text.join("background")),
            greetings: load_file(&text.join("greetings")),
            newsmod: mtime(&text.join("news")),
            motdmod: mtime(&text.join("motd")),
        };
        prune_crlf(&mut t.greetings);
        t
    }
}

/// One help entry: one row per keyword, sharing the entry text.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub keyword: BStr,
    pub entry: std::rc::Rc<BStr>,
    pub duplicate: i32,
    pub min_level: i32,
}

/// one_word: next word, quoted phrases kept whole,
/// lowercased.
fn one_word(input: &[u8], pos: &mut usize) -> BStr {
    let b = input;
    let mut i = *pos;
    let mut out = Vec::new();
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < b.len() && b[i] == b'"' {
        i += 1;
        while i < b.len() && b[i] != b'"' {
            out.push(b[i].to_ascii_lowercase());
            i += 1;
        }
        if i < b.len() {
            i += 1;
        }
    } else {
        while i < b.len() && !b[i].is_ascii_whitespace() {
            out.push(b[i].to_ascii_lowercase());
            i += 1;
        }
    }
    *pos = i;
    out
}

/// Read at most 255 bytes, stopping after a newline, then drop the final
/// byte UNCONDITIONALLY.
///
/// That last chop is the whole story. For an ordinary line it removes the
/// `\n`. For a line of exactly 254 characters plus CRLF the buffer fills one
/// byte short of the `\n`, so the `\r` is chopped instead and the orphaned
/// `\n` comes back as an empty line of its own. Anything longer than that
/// loses a character at every 255-byte boundary and continues mid-line.
///
/// help.hlp has lines at exactly that length, which is how the round trip
/// through hedit caught this — the line has to survive as a blank line.
struct OneLine<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for OneLine<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        let mut end = start;
        while end < self.data.len() && end - start < 255 {
            let b = self.data[end];
            end += 1;
            if b == b'\n' {
                break;
            }
        }
        self.pos = end;
        // buf[strlen(buf) - 1] = '\0'
        Some(&self.data[start..end - 1])
    }
}

/// load_help over one .hlp file's bytes. Body lines keep their raw bytes
/// (a CRLF source renders `\r\r\n`); only parsing strips the `\r`.
pub fn load_help(data: &[u8], log: &mut Vec<String>) -> Vec<HelpEntry> {
    let mut entries = Vec::new();
    let mut lines = OneLine { data, pos: 0 };
    loop {
        let Some(key_raw) = lines.next() else { break };
        let key_line = key_raw.strip_suffix(b"\r").unwrap_or(key_raw);
        if key_line.starts_with(b"$") {
            break;
        }
        if key_line.is_empty() {
            continue;
        }
        // Entry body starts with the keyword line itself (raw bytes).
        let mut entry: BStr = Vec::new();
        entry.extend_from_slice(key_raw);
        entry.extend_from_slice(b"\r\n");
        let mut min_level = 0i32;
        loop {
            let Some(raw) = lines.next() else { break };
            let line = raw.strip_suffix(b"\r").unwrap_or(raw);
            if line.first() == Some(&b'#') {
                if line.len() > 1 {
                    min_level = crate::handler::atoi(&line[1..]);
                } else {
                    log.push("SYSERR: Help entry does not have a min level. Assuming 0.".to_string());
                    min_level = 0;
                }
                break;
            }
            if entry.len() + raw.len() + 2 < 32384 {
                entry.extend_from_slice(raw);
                entry.extend_from_slice(b"\r\n");
            }
        }
        let mut entry_at = entry;
        parse_at(&mut entry_at);
        let shared = std::rc::Rc::new(entry_at);
        let mut pos = 0usize;
        let mut dup = 0i32;
        loop {
            let kw = one_word(key_line, &mut pos);
            if kw.is_empty() {
                break;
            }
            entries.push(HelpEntry {
                keyword: kw,
                entry: shared.clone(),
                duplicate: dup,
                min_level,
            });
            dup += 1;
        }
    }
    entries
}

/// search_help: binary search over the
/// keyword-sorted table, backed up to the first match, then walked forward
/// past entries above the caller's level.
///
/// The search is bounded by the last index. Seeding it with the entry
/// count instead lets a query that sorts after every keyword probe one
/// past the end of the table.
pub fn search_help(g: &crate::game::Game, argument: &[u8], level: i32) -> Option<usize> {
    let table = &g.help_table;
    if table.is_empty() {
        return None;
    }
    let minlen = argument.len();
    let key = |i: usize| -> std::cmp::Ordering {
        let kw = &table[i].keyword;
        let n = minlen.min(kw.len());
        // Case-insensitive over at most `minlen` bytes; a keyword shorter
        // than the query cannot match.
        for k in 0..n {
            let (a, b) = (argument[k].to_ascii_lowercase(), kw[k].to_ascii_lowercase());
            if a != b {
                return a.cmp(&b);
            }
        }
        if minlen > kw.len() {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    };
    let (mut bot, mut top) = (0i64, table.len() as i64 - 1);
    while bot <= top {
        let start_mid = (bot + top) / 2;
        let mut mid = start_mid;
        match key(mid as usize) {
            std::cmp::Ordering::Equal => {
                while mid > 0 && key(mid as usize - 1) == std::cmp::Ordering::Equal {
                    mid -= 1;
                }
                while level < table[mid as usize].min_level && mid < start_mid {
                    mid += 1;
                }
                if key(mid as usize) != std::cmp::Ordering::Equal
                    || level < table[mid as usize].min_level
                {
                    break;
                }
                return Some(mid as usize);
            }
            std::cmp::Ordering::Greater => bot = mid + 1,
            std::cmp::Ordering::Less => top = mid - 1,
        }
    }
    None
}

/// Boot the help table from <lib>/text/help/index, then sort by keyword
/// case-insensitively.
pub fn boot_help(lib: &Path, mini: bool, log: &mut Vec<String>) -> Vec<HelpEntry> {
    let dir = lib.join("text").join("help");
    let index = dir.join(if mini { "index.mini" } else { "index" });
    let Ok(data) = std::fs::read(&index) else {
        log.push(format!("SYSERR: opening help index file: {}", index.display()));
        return Vec::new();
    };
    let mut entries = Vec::new();
    for line in data.split(|c| *c == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.starts_with(b"$") || line.is_empty() {
            break;
        }
        let path = dir.join(String::from_utf8_lossy(line).as_ref());
        match std::fs::read(&path) {
            Ok(file) => entries.extend(load_help(&file, log)),
            Err(e) => log.push(format!("SYSERR: {}: {}", path.display(), e)),
        }
    }
    entries.sort_by(|a, b| cmp_ci(&a.keyword, &b.keyword));
    entries
}

/// Case-insensitive byte ordering.
pub fn cmp_ci(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    let la: Vec<u8> = a.to_ascii_lowercase();
    let lb: Vec<u8> = b.to_ascii_lowercase();
    la.cmp(&lb)
}

// ---------------------------------------------------------------------------
// strfrmt / strpaste — the automap's column formatter
// ---------------------------------------------------------------------------

/// is_ws_not_tab: whitespace except '\t' (the color escape).
fn is_ws_not_tab(c: u8) -> bool {
    c != b'\t' && c.is_ascii_whitespace()
}

/// strfrmt: word-wrap `str` into a `w`-wide, `h`-tall
/// column. `\` starts a new line, `\t`-codes and `` ` ``/`$`/`#` pairs cost
/// the right number of printable columns, and the last non-MXP color code is
/// re-emitted after each wrap.
pub fn strfrmt(str_: &[u8], w: i32, h: i32, _justify: bool, hpad: bool, vpad: bool) -> Vec<u8> {
    let mut ret: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    let mut llen: i32 = 0;
    let mut lcount: i32 = 0;
    let mut last_color = b'n';
    let mut new_line_started = false;

    let s = str_;
    let mut sp = 0usize;
    while sp < s.len() {
        while sp < s.len() && is_ws_not_tab(s[sp]) {
            sp += 1;
        }
        let mut wp = sp;
        let mut wlen: i32 = 0;
        while sp < s.len() {
            if is_ws_not_tab(s[sp]) {
                break;
            }
            if s[sp] == b'\\' && sp + 1 < s.len() && s[sp + 1] == b'\\' {
                if sp != wp {
                    break; // finish the current word first
                }
                sp += 2;
                while sp < s.len() && is_ws_not_tab(s[sp]) {
                    sp += 1;
                }
                wp = sp;
                if hpad {
                    while llen < w {
                        line.push(b' ');
                        llen += 1;
                    }
                }
                line.extend_from_slice(b"\r\n");
                ret.extend_from_slice(&line);
                llen = 0;
                lcount += 1;
                line.clear();
            } else if matches!(s[sp], b'`' | b'$' | b'#') {
                if sp + 1 < s.len() && s[sp + 1] == s[sp] {
                    wlen += 1;
                }
                sp += 2;
            } else if s[sp] == b'\t' && sp + 1 < s.len() {
                let mxp_end = match s[sp + 1] {
                    b'[' => Some(b']'),
                    b'<' => Some(b'>'),
                    _ => None,
                };
                if mxp_end.is_none() {
                    last_color = s[sp + 1];
                }
                sp += 2;
                if let Some(end) = mxp_end {
                    while sp < s.len() && s[sp] != end {
                        sp += 1;
                    }
                }
            } else {
                wlen += 1;
                sp += 1;
            }
        }

        if llen + wlen + if line.is_empty() { 0 } else { 1 } > w {
            if hpad {
                while llen < w {
                    line.push(b' ');
                    llen += 1;
                }
            }
            line.extend_from_slice(b"\tn\r\n");
            ret.extend_from_slice(&line);
            llen = 0;
            lcount += 1;
            line.clear();
            if last_color != b'n' {
                line.push(b'\t');
                line.push(last_color);
                new_line_started = true;
            }
        }
        if !line.is_empty() && !new_line_started {
            line.push(b' ');
            llen += 1;
        }
        new_line_started = false;
        llen += wlen;
        line.extend_from_slice(&s[wp..sp]);
    }

    if !line.is_empty() {
        if hpad {
            while llen < w {
                line.push(b' ');
                llen += 1;
            }
        }
        line.extend_from_slice(b"\r\n");
        ret.extend_from_slice(&line);
        lcount += 1;
    }
    if vpad {
        while lcount < h {
            if hpad {
                ret.extend(std::iter::repeat(b' ').take(w.max(0) as usize));
            }
            ret.extend_from_slice(b"\r\n");
            lcount += 1;
        }
    }
    ret
}

/// strpaste: join two multi-line strings side by side.
pub fn strpaste(str1: &[u8], str2: &[u8], joiner: &[u8]) -> Vec<u8> {
    let isnewl = |c: u8| c == b'\n' || c == b'\r';
    let mut out = Vec::new();
    let (mut p1, mut p2) = (0usize, 0usize);
    while p1 < str1.len() || p2 < str2.len() {
        while p1 < str1.len() && !isnewl(str1[p1]) {
            out.push(str1[p1]);
            p1 += 1;
        }
        if p1 < str1.len() {
            if p1 + 1 < str1.len() && str1[p1 + 1] != str1[p1] && isnewl(str1[p1 + 1]) {
                p1 += 1;
            }
            p1 += 1;
        }
        out.extend_from_slice(joiner);
        while p2 < str2.len() && !isnewl(str2[p2]) {
            out.push(str2[p2]);
            p2 += 1;
        }
        if p2 < str2.len() {
            if p2 + 1 < str2.len() && str2[p2 + 1] != str2[p2] && isnewl(str2[p2 + 1]) {
                p2 += 1;
            }
            p2 += 1;
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

// ---------------------------------------------------------------------------
// autowiz — the self-updating wizlists
// ---------------------------------------------------------------------------

use mud_data::types::{LVL_GOD, LVL_GRGOD, LVL_IMMORT, LVL_IMPL};

pub fn reboot_wizlists(g: &mut crate::game::Game) {
    let text = g.lib_dir.join("text");
    for (path, is_wiz) in [(text.join("wizlist"), true), (text.join("immlist"), false)] {
        let mut s = file_to_string(&path).unwrap_or_default();
        parse_at(&mut s);
        if is_wiz {
            g.texts.wizlist = s;
        } else {
            g.texts.immlist = s;
        }
    }
}

const IMM_LMARG: &[u8] = b"   ";
const IMM_NSIZE: usize = 16;
const LINE_LEN: usize = 64;

/// autowiz's `level_params[]` (util/); `levels` is built by
/// prepending, so the write order is highest tier first.
fn level_params() -> Vec<(u8, &'static str)> {
    vec![
        (LVL_IMPL, "Implementors"),
        (LVL_GRGOD, "Greater Gods"),
        (LVL_GOD, "Gods"),
        (LVL_IMMORT, "Immortals"),
    ]
}

/// write_wizlist (util/). `\n` line endings, exactly as the
/// helper binary writes them — file_to_string re-CRLFs on load.
fn write_wizlist(buckets: &[(u8, &'static str, Vec<Vec<u8>>)], minlev: u8, maxlev: u8) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(
b"*******************************************************************************\n\
*          The following people have reached immortality on RustMUD.          *\n\
*******************************************************************************\n\n",
    );
    for (level, level_name, names) in buckets {
        if *level < minlev || *level > maxlev {
            continue;
        }
        let i = 39 - (level_name.len() >> 1);
        out.extend(std::iter::repeat(b' ').take(i));
        out.extend_from_slice(level_name.as_bytes());
        out.push(b'\n');
        out.extend(std::iter::repeat(b' ').take(i));
        out.extend(std::iter::repeat(b'~').take(level_name.len()));
        out.push(b'\n');

        // COL_LEVEL == LVL_IMMORT: only the Immortals bucket is columnar.
        let columnar = *level <= LVL_IMMORT;
        let mut buf: Vec<u8> = Vec::new();
        for name in names {
            buf.extend_from_slice(name);
            if buf.len() > LINE_LEN {
                if columnar {
                    out.extend_from_slice(IMM_LMARG);
                } else {
                    let i = 40usize.saturating_sub(buf.len() >> 1);
                    out.extend(std::iter::repeat(b' ').take(i));
                }
                out.extend_from_slice(&buf);
                out.push(b'\n');
                buf.clear();
            } else if columnar {
                for _ in 0..IMM_NSIZE.saturating_sub(name.len()) {
                    buf.push(b' ');
                }
            } else {
                buf.extend_from_slice(b"   ");
            }
        }
        if !buf.is_empty() {
            if columnar {
                out.extend_from_slice(IMM_LMARG);
                out.extend_from_slice(&buf);
                out.push(b'\n');
            } else {
                let i = 40usize.saturating_sub(buf.len() >> 1);
                out.extend(std::iter::repeat(b' ').take(i));
                out.extend_from_slice(&buf);
                out.push(b'\n');
            }
        }
        out.push(b'\n');
    }
    out
}

/// The `bin/autowiz` body: read the player index, bucket by level, sort each
/// bucket, and write the two lists.
pub fn write_wizlists(g: &mut crate::game::Game) {
    use crate::game::{PINDEX_DELETED, PINDEX_NOWIZLIST};

    let mut buckets: Vec<(u8, &'static str, Vec<Vec<u8>>)> =
        level_params().into_iter().map(|(l, n)| (l, n, Vec::new())).collect();

    for p in &g.player_table {
        if p.level < LVL_IMMORT as i32
            || p.flags & PINDEX_NOWIZLIST != 0
            || p.flags & PINDEX_DELETED != 0
        {
            continue;
        }
        // add_name: rejects any name with a non-alpha byte.
        if p.name.is_empty() || !p.name.iter().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let mut name = p.name.clone();
        name[0] = name[0].to_ascii_uppercase();
        // The first bucket (walking IMPL→IMMORT) whose level is <= theirs.
        let Some(slot) = buckets.iter_mut().find(|(l, _, _)| *l as i32 <= p.level) else {
            continue;
        };
        slot.2.push(name);
    }
    for b in buckets.iter_mut() {
        b.2.sort();
    }

    let text = g.lib_dir.join("text");
    let wizlevel = g.config.min_wizlist_lev.clamp(0, 255) as u8;
    let wiz = write_wizlist(&buckets, wizlevel, LVL_IMPL);
    let imm = write_wizlist(&buckets, LVL_IMMORT, wizlevel.saturating_sub(1));
    for (path, body) in [(text.join("wizlist"), wiz), (text.join("immlist"), imm)] {
        if let Err(e) = std::fs::write(&path, &body) {
            g.log(format!("SYSERR: autowiz: {}: {}", path.display(), e));
        }
    }
}

#[cfg(test)]
mod wizlist_banner_tests {
    use super::write_wizlist;

    /// The banner is a fixed 79-column box. The MUD's name sits inside it, so
    /// renaming the game re-centres the middle line -- and a name one byte
    /// longer silently pushes the closing `*` out of true. Nothing else
    /// checks this line.
    #[test]
    fn banner_box_is_79_columns() {
        let out = write_wizlist(&[], 0, 255);
        let banner: Vec<&[u8]> = out.split(|&b| b == b'\n').take(3).collect();
        assert_eq!(banner.len(), 3);
        for (i, line) in banner.iter().enumerate() {
            assert_eq!(line.len(), 79, "banner line {} is {} cols: {:?}",
                       i + 1, line.len(), String::from_utf8_lossy(line));
        }
        assert!(banner[0].iter().all(|&b| b == b'*'));
        assert!(banner[2].iter().all(|&b| b == b'*'));
        assert_eq!(banner[1].first(), Some(&b'*'));
        assert_eq!(banner[1].last(), Some(&b'*'));
    }

    /// The name in the banner is the one this server brands itself with.
    #[test]
    fn banner_names_this_mud() {
        let out = write_wizlist(&[], 0, 255);
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("reached immortality on RustMUD."), "{:?}", &s[..120.min(s.len())]);
    }
}

#[cfg(test)]
mod parse_at_tests {
    use super::parse_at;

    /// The rose social carries `@R@@@G}` -- a red `@` escape between two
    /// colour codes. Collapsing the escape at load cost the file its green
    /// code on the next save, which is how the socials round-trip caught it.
    #[test]
    fn doubled_at_survives_the_round_trip() {
        let mut b = b"@R@@@G}---".to_vec();
        parse_at(&mut b);
        assert_eq!(b, b"\tR@@\tG}---".to_vec());
        mud_net::editor::parse_tab(&mut b[..]);
        assert_eq!(b, b"@R@@@G}---".to_vec());
    }

    #[test]
    fn lone_at_becomes_a_tab_and_length_never_changes() {
        let mut b = b"a@Rb".to_vec();
        parse_at(&mut b);
        assert_eq!(b, b"a\tRb".to_vec());
    }
}

#[cfg(test)]
mod get_one_line_tests {
    use super::{load_help, OneLine};

    fn lines(data: &[u8]) -> Vec<Vec<u8>> {
        OneLine { data, pos: 0 }.map(|l| l.to_vec()).collect()
    }

    #[test]
    fn ordinary_line_loses_only_its_newline() {
        assert_eq!(lines(b"abc\r\ndef\r\n"), vec![b"abc\r".to_vec(), b"def\r".to_vec()]);
    }

    /// 254 characters plus CRLF is 256 bytes: the read takes 255, stopping
    /// one byte short of the '\n', and the chop removes the '\r'. The '\n'
    /// is then a line by itself, which reads back as blank.
    #[test]
    fn two_hundred_fifty_four_plus_crlf_yields_a_blank_line() {
        let mut d = vec![b'x'; 254];
        d.extend_from_slice(b"\r\n");
        assert_eq!(lines(&d), vec![vec![b'x'; 254], Vec::new()]);
    }

    /// Past 255 bytes a character is lost at the boundary.
    #[test]
    fn a_longer_line_loses_a_character_at_the_boundary() {
        let mut d = vec![b'y'; 300];
        d.extend_from_slice(b"\r\n");
        let got = lines(&d);
        assert_eq!(got[0].len(), 254);
        assert_eq!(got[1], vec![b'y'; 300 - 255].into_iter().chain(*b"\r").collect::<Vec<u8>>());
    }

    #[test]
    fn an_entry_keeps_the_blank_the_chop_creates() {
        let mut d = vec![b'K'; 254];
        d.extend_from_slice(b"\r\nbody\r\n#0\r\n$~\r\n");
        let mut log = Vec::new();
        let e = load_help(&d, &mut log);
        assert_eq!(e.len(), 1);
        let text = String::from_utf8_lossy(e[0].entry.as_ref()).into_owned();
        assert!(text.contains("\r\n\r\nbody"), "{:?}", text);
    }
}
