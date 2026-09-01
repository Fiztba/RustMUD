//! The line editor: the buffer-append half of `string_add` plus
//! the entire improved editor — `improved_editor_execute`,
//! `parse_edit_action`, `format_text`, `replace_str`, and the
//! `parse_at`/`parse_tab`/`smash_tilde` helpers.
//!
//! The player-visible text is kept exactly as players know it, quirks
//! included: the misspelling "occurence", the double spaces, the literal
//! backslash `\r\n` in PARSE_FORMAT's range error, and `/d` reporting
//! "0 lines deleted." when deleting past the last newline.
//!
//! A handful of sites (each noted inline) would otherwise read past a
//! terminating NUL. Each substitutes the deterministic reading "there is a
//! permanent non-matching NUL there" rather than emulating heap garbage.

use mud_data::types::{MAX_INPUT_LENGTH, MAX_STRING_LENGTH};

const PAGE_WIDTH: i32 = 80;
const FORMAT_INDENT: i32 = 1 << 0;

/// STRINGADD_* actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    Ok,
    Save,
    Abort,
    Action,
}

/// One editor session's buffer state.
#[derive(Debug, Default)]
pub struct EditBuf {
    /// The string under construction; None = empty buffer.
    pub buf: Option<Vec<u8>>,
    /// Max length, including the "\r\n\0" reserve.
    pub max_str: usize,
}

/// PARSE_* modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseCmd {
    Format,
    Replace,
    Help,
    Delete,
    Insert,
    ListNorm,
    ListNum,
    Edit,
    Toggle,
}

// ---------------------------------------------------------------------------
// Exported string helpers
// ---------------------------------------------------------------------------

/// Collapse `$$` to `$`, in place.
pub fn delete_doubledollar(s: &mut Vec<u8>) {
    // If the string has no dollar signs, return immediately.
    let Some(first) = s.iter().position(|&c| c == b'$') else {
        return;
    };
    let mut ddread = first;
    let mut ddwrite = first;
    while ddread < s.len() {
        let c = s[ddread];
        s[ddwrite] = c;
        ddwrite += 1;
        ddread += 1;
        if c == b'$' && ddread < s.len() && s[ddread] == b'$' {
            ddread += 1; // skip if we saw 2 $'s in a row
        }
    }
    s.truncate(ddwrite);
}

/// Erase line-ending tildes (`~` followed by `\r`,
/// `\n`, or end of string) by overwriting them with a space.
pub fn smash_tilde(s: &mut [u8]) {
    for p in 0..s.len() {
        if s[p] == b'~' && (p + 1 == s.len() || s[p + 1] == b'\r' || s[p + 1] == b'\n') {
            s[p] = b' ';
        }
    }
}

/// `@` becomes `\t` unless doubled (`@@` is left alone).
pub fn parse_at(s: &mut [u8]) {
    let mut p = 0;
    while p < s.len() {
        if s[p] == b'@' {
            if p + 1 >= s.len() || s[p + 1] != b'@' {
                s[p] = b'\t';
            } else {
                p += 1;
            }
        }
        p += 1;
    }
}

/// `\t` becomes `@` unless doubled (`\t\t` is left alone).
pub fn parse_tab(s: &mut [u8]) {
    let mut p = 0;
    while p < s.len() {
        if s[p] == b'\t' {
            if p + 1 >= s.len() || s[p + 1] != b'\t' {
                s[p] = b'@';
            } else {
                p += 1;
            }
        }
        p += 1;
    }
}

// ---------------------------------------------------------------------------
// Small character-class helpers
// ---------------------------------------------------------------------------

/// True for ASCII space, tab, newline, vertical tab, form feed and return.
fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// True for ASCII letters only.
fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// ASCII lowercase.
fn to_lower(c: u8) -> u8 {
    if c.is_ascii_uppercase() { c + (b'a' - b'A') } else { c }
}

/// Read byte at `p`, treating everything at/past the end as the NUL
/// terminator (see module doc for the out-of-bounds stand-in rule).
fn ch(s: &[u8], p: usize) -> u8 {
    s.get(p).copied().unwrap_or(0)
}

/// Whether `c` is one of `set`. NUL is never a member.
fn in_set(set: &[u8], c: u8) -> bool {
    c != 0 && set.contains(&c)
}

/// Index of the first `\n` at or after `from`.
fn find_nl(s: &[u8], from: usize) -> Option<usize> {
    s.get(from..)
        .and_then(|t| t.iter().position(|&c| c == b'\n'))
        .map(|r| from + r)
}

/// `strstr` returning the index of the first match.
fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Append at most `limit` bytes of `src` to `dst`.
fn append_at_most(dst: &mut Vec<u8>, src: &[u8], limit: usize) {
    let n = src.len().min(limit);
    dst.extend_from_slice(&src[..n]);
}

/// Append `src` to `dst`, keeping the result inside a `size`-byte buffer
/// that has to stay NUL-terminated.
fn append_within(dst: &mut Vec<u8>, src: &[u8], size: usize) {
    let limit = size.saturating_sub(dst.len()).saturating_sub(1);
    append_at_most(dst, src, limit);
}

/// Read a leading integer: optional whitespace, optional sign, then digits.
/// Stops at the first non-digit and never reports an error. Overflow
/// saturates at 64-bit range and is then truncated to 32 bits.
pub(crate) fn parse_int_prefix(s: &[u8]) -> i32 {
    let mut p = 0;
    while p < s.len() && is_ws(s[p]) {
        p += 1;
    }
    let mut neg = false;
    if p < s.len() && (s[p] == b'+' || s[p] == b'-') {
        neg = s[p] == b'-';
        p += 1;
    }
    let mut val: i128 = 0;
    while p < s.len() && s[p].is_ascii_digit() {
        val = val * 10 + (s[p] - b'0') as i128;
        if val > u64::MAX as i128 {
            val = u64::MAX as i128; // keep bounded; clamped below anyway
        }
        p += 1;
    }
    if neg {
        val = -val;
    }
    val.clamp(i64::MIN as i128, i64::MAX as i128) as i64 as i32
}

/// Read one decimal integer at `*p`: leading whitespace, optional sign, and
/// at least one digit.
fn scan_int(s: &[u8], p: &mut usize) -> Option<i32> {
    while *p < s.len() && is_ws(s[*p]) {
        *p += 1;
    }
    let start = *p;
    let mut q = *p;
    let mut neg = false;
    if q < s.len() && (s[q] == b'+' || s[q] == b'-') {
        neg = s[q] == b'-';
        q += 1;
    }
    let mut val: i128 = 0;
    let mut digits = 0;
    while q < s.len() && s[q].is_ascii_digit() {
        val = val * 10 + (s[q] - b'0') as i128;
        if val > u64::MAX as i128 {
            val = u64::MAX as i128;
        }
        digits += 1;
        q += 1;
    }
    if digits == 0 {
        *p = start;
        return None;
    }
    if neg {
        val = -val;
    }
    *p = q;
    Some(val.clamp(i64::MIN as i128, i64::MAX as i128) as i64 as i32)
}

/// Parse `" <low> - <high> "`. Returns (count, low, high), where count is
/// -1 if the input ran out before the first number, 0 if the first number
/// did not parse, else 1 or 2. Numbers that were not read come back as 0
/// and must be ignored.
fn parse_range(s: &[u8]) -> (i32, i32, i32) {
    let mut p = 0;
    while p < s.len() && is_ws(s[p]) {
        p += 1;
    }
    if p >= s.len() {
        return (-1, 0, 0);
    }
    let Some(v1) = scan_int(s, &mut p) else {
        return (0, 0, 0);
    };
    while p < s.len() && is_ws(s[p]) {
        p += 1;
    }
    if p >= s.len() || s[p] != b'-' {
        return (1, v1, 0);
    }
    p += 1;
    let Some(v2) = scan_int(s, &mut p) else {
        return (1, v1, 0);
    };
    (2, v1, v2)
}

/// Advance `p` over whitespace, stopping at a tab.
fn skip_spaces_idx(s: &[u8], p: &mut usize) {
    while *p < s.len() && s[*p] != b'\t' && is_ws(s[*p]) {
        *p += 1;
    }
}

/// First whitespace-delimited word (lowercased!) and the remainder after
/// skipping spaces.
fn half_chop(s: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut p = 0;
    skip_spaces_idx(s, &mut p);
    let mut arg1 = Vec::new();
    while p < s.len() && !is_ws(s[p]) {
        arg1.push(to_lower(s[p]));
        p += 1;
    }
    skip_spaces_idx(s, &mut p);
    (arg1, s[p..].to_vec())
}

/// The non-empty runs between single quotes.
fn quote_tokens(s: &[u8]) -> Vec<&[u8]> {
    s.split(|&c| c == b'\'').filter(|t| !t.is_empty()).collect()
}

/// Decimal rendering of a `%d` argument.
fn itoa(v: i32) -> Vec<u8> {
    v.to_string().into_bytes()
}

// ---------------------------------------------------------------------------
// string_add — the buffer-append half
// ---------------------------------------------------------------------------

/// Feed one input line to the editor. Returns the resulting action and the
/// messages to send to the descriptor (in order).
///
/// This is `string_add` minus the game-side cleanup: on `Abort` the caller
/// owns the d->backstr restore, and on `Save`/`Abort` the caller runs the
/// per-state cleanup table and detaches the session.
pub fn editor_add_line(
    eb: &mut EditBuf,
    line: &[u8],
    improved: bool,
    is_trigedit: bool,
) -> (EditorAction, Vec<Vec<u8>>, Option<Vec<u8>>) {
    let mut msgs: Vec<Vec<u8>> = Vec::new();
    let mut paged: Option<Vec<u8>> = None;
    let mut str_buf: Vec<u8> = line.to_vec();

    delete_doubledollar(&mut str_buf);
    smash_tilde(&mut str_buf);

    // Terminal string: '\t' by itself.
    let mut action = if str_buf == b"\t" {
        str_buf.clear();
        EditorAction::Save
    } else if improved {
        let a = improved_editor_execute(eb, &mut str_buf, is_trigedit, &mut msgs, &mut paged);
        if a == EditorAction::Action {
            return (EditorAction::Action, msgs, paged);
        }
        a
    } else {
        // !CONFIG_IMPROVED_EDITOR: '/' lines are plain text.
        EditorAction::Ok
    };

    if action != EditorAction::Ok {
        // Do nothing.
    } else if eb.buf.is_none() {
        if str_buf.len() + 3 > eb.max_str {
            // \r\n\0
            msgs.push(b"String too long - Truncated.\r\n".to_vec());
            str_buf.truncate(eb.max_str.saturating_sub(3));
            str_buf.extend_from_slice(b"\r\n");
            eb.buf = Some(str_buf);
            if !improved {
                action = EditorAction::Save;
            }
        } else {
            eb.buf = Some(str_buf);
        }
    } else {
        let cur = eb.buf.as_mut().expect("checked above");
        if str_buf.len() + cur.len() + 3 > eb.max_str {
            // \r\n\0
            msgs.push(b"String too long.  Last line skipped.\r\n".to_vec());
            if !improved {
                action = EditorAction::Save;
            } else if action == EditorAction::Ok {
                action = EditorAction::Action; // No appending \r\n\0, but still let them save.
            }
        } else {
            cur.extend_from_slice(&str_buf);
        }
    }

    // Common cleanup code.
    match action {
        EditorAction::Abort => {
            // Game-side: free *d->str, restore d->backstr. The caller owns
            // the backstr, so the buffer is left as-is here.
        }
        EditorAction::Save => {
            if let Some(b) = &mut eb.buf {
                if b.is_empty() {
                    *b = b"Nothing.\r\n".to_vec();
                }
            }
        }
        EditorAction::Action | EditorAction::Ok => {}
    }

    if action == EditorAction::Save || action == EditorAction::Abort {
        // Game-side cleanup table + PLR flag clears happen in the caller.
    } else if action != EditorAction::Action {
        // 3 = \r\n\0
        if let Some(cur) = eb.buf.as_mut() {
            if cur.len() + 3 <= eb.max_str {
                cur.extend_from_slice(b"\r\n");
            }
        }
    }

    (action, msgs, paged)
}

// ---------------------------------------------------------------------------
// improved_editor_execute
// ---------------------------------------------------------------------------

fn improved_editor_execute(
    eb: &mut EditBuf,
    str_buf: &mut Vec<u8>,
    is_trigedit: bool,
    msgs: &mut Vec<Vec<u8>>,
    paged: &mut Option<Vec<u8>>,
) -> EditorAction {
    if str_buf.first() != Some(&b'/') {
        return EditorAction::Ok;
    }

    let mut actions: Vec<u8> = str_buf.get(2..).unwrap_or(&[]).to_vec();
    actions.truncate(MAX_INPUT_LENGTH - 1);
    let cmd = str_buf.get(1).copied().unwrap_or(0);
    str_buf.clear(); // *str = '\0'

    match cmd {
        b'a' => return EditorAction::Abort,
        b'c' => {
            if eb.buf.is_some() {
                eb.buf = None;
                msgs.push(b"Current buffer cleared.\r\n".to_vec());
            } else {
                msgs.push(b"Current buffer empty.\r\n".to_vec());
            }
        }
        b'd' => parse_edit_action(ParseCmd::Delete, &actions, eb, is_trigedit, msgs, paged),
        b'e' => parse_edit_action(ParseCmd::Edit, &actions, eb, is_trigedit, msgs, paged),
        b'f' => {
            if eb.buf.is_some() {
                parse_edit_action(ParseCmd::Format, &actions, eb, is_trigedit, msgs, paged);
            } else {
                msgs.push(b"Current buffer empty.\r\n".to_vec());
            }
        }
        b'i' => {
            if eb.buf.is_some() {
                parse_edit_action(ParseCmd::Insert, &actions, eb, is_trigedit, msgs, paged);
            } else {
                msgs.push(b"Current buffer empty.\r\n".to_vec());
            }
        }
        b'h' => parse_edit_action(ParseCmd::Help, &actions, eb, is_trigedit, msgs, paged),
        b'l' => {
            if eb.buf.is_some() {
                parse_edit_action(ParseCmd::ListNorm, &actions, eb, is_trigedit, msgs, paged);
            } else {
                msgs.push(b"Current buffer empty.\r\n".to_vec());
            }
        }
        b'n' => {
            if eb.buf.is_some() {
                parse_edit_action(ParseCmd::ListNum, &actions, eb, is_trigedit, msgs, paged);
            } else {
                msgs.push(b"Current buffer empty.\r\n".to_vec());
            }
        }
        b'r' => parse_edit_action(ParseCmd::Replace, &actions, eb, is_trigedit, msgs, paged),
        b's' => return EditorAction::Save,
        b't' => parse_edit_action(ParseCmd::Toggle, &actions, eb, is_trigedit, msgs, paged),
        _ => msgs.push(b"Invalid option.\r\n".to_vec()),
    }
    EditorAction::Action
}

// ---------------------------------------------------------------------------
// parse_edit_action
// ---------------------------------------------------------------------------

const HELP_TEXT: &[u8] = b"Editor command formats: /<letter>\r\n\r\n\
/a         -  aborts editor\r\n\
/c         -  clears buffer\r\n\
/d#        -  deletes a line #\r\n\
/e# <text> -  changes the line at # with <text>\r\n\
/f         -  formats text\r\n\
/fi        -  indented formatting of text\r\n\
/h         -  list text editor commands\r\n\
/i# <text> -  inserts <text> before line #\r\n\
/l         -  lists buffer\r\n\
/n         -  lists buffer with line numbers\r\n\
/r 'a' 'b' -  replace 1st occurence of text <a> in buffer with text <b>\r\n\
/ra 'a' 'b'-  replace all occurences of text <a> within buffer with text <b>\r\n\
\x20             usage: /r[a] 'pattern' 'replacement'\r\n\
/t         -  toggles '@' and tabs\r\n\
/s         -  saves text\r\n\
\r\n\
A line number or range narrows /d, /f, /fi, /l and /n to those lines:\r\n\
\x20             usage: /fi 5  (line 5)   /fi 5-9  (lines 5 through 9)\r\n";

/// READ_SIZE — the `char line[]` format_script builds each
/// output line in.
const READ_SIZE: usize = 256;

/// Case-insensitive prefix test. A line shorter than the keyword can never
/// match.
fn strn_starts(t: &[u8], word: &[u8]) -> bool {
    t.len() >= word.len() && t[..word.len()].iter().zip(word).all(|(a, b)| to_lower(*a) == to_lower(*b))
}

/// format_script: the `/f` reformatter the string editor
/// runs for trigedit. It re-indents by block structure and refuses the whole
/// job on an unmatched `end`/`done`/`else`/`break`.
///
/// Three details are load-bearing:
///
/// * keyword matching is case-insensitive, so `IF `, `End` and `BREAK` all
///   count;
/// * runs of line endings collapse, so blank lines vanish from the result;
/// * the length check uses the line's *untruncated* length, and counts two
///   bytes per indent level whether or not the 256-byte line buffer had
///   room for them.
fn format_script(eb: &mut EditBuf, msgs: &mut Vec<Vec<u8>>) -> bool {
    let Some(src) = eb.buf.clone() else { return false };
    if src.is_empty() {
        return false;
    }

    let mut nsc: Vec<u8> = Vec::new();
    let mut len: usize = 0;
    let mut indent: i32 = 0;
    let mut indent_next = false;
    let mut line_num = 0;
    let mut block_stack: Vec<u8> = Vec::new();
    let mut switch_indent: Vec<i32> = Vec::new();
    let mut case_indent: i32 = 0;
    let mut in_switch: i32 = 0;

    for tok in src.split(|c| *c == b'\n' || *c == b'\r') {
        if tok.is_empty() {
            continue; // empty tokens are skipped
        }
        line_num += 1;
        let t = {
            let mut p = 0;
            while p < tok.len() && is_ws(tok[p]) {
                p += 1;
            }
            &tok[p..]
        };

        if strn_starts(t, b"switch ") {
            indent_next = true;
            block_stack.push(b's');
            switch_indent.push(indent);
            in_switch += 1;
        } else if strn_starts(t, b"case") || strn_starts(t, b"default") {
            if in_switch > 0 {
                indent = switch_indent.last().copied().unwrap_or(0) + 1;
                indent_next = true;
                case_indent = indent;
            }
        } else if strn_starts(t, b"if ") || strn_starts(t, b"while ") {
            indent_next = true;
            block_stack.push(b'l');
        } else if strn_starts(t, b"end") || strn_starts(t, b"done") {
            if block_stack.is_empty() {
                msgs.push(
                    format!("Unmatched 'end' or 'done' (line {})!\r\n", line_num).into_bytes(),
                );
                return false;
            }
            if block_stack.last() == Some(&b's') {
                indent = switch_indent.last().copied().unwrap_or(0);
                switch_indent.pop();
                case_indent = 0;
                in_switch -= 1;
            } else {
                indent -= 1;
            }
            block_stack.pop();
            indent_next = false;
        } else if strn_starts(t, b"else") {
            if block_stack.last() != Some(&b'l') {
                msgs.push(format!("Unmatched 'else' (line {})!\r\n", line_num).into_bytes());
                return false;
            }
            indent -= 1;
            indent_next = true;
        } else if strn_starts(t, b"break") {
            if !matches!(block_stack.last(), Some(b's') | Some(b'l')) {
                msgs.push(
                    format!("Break not in case or loop (line {})!\r\n", line_num).into_bytes(),
                );
                return false;
            }
            // `case_indent` is only ever set by a `case`, so a `break` in a
            // plain `while` indents to 1 and flattens everything after it.
            // Wrong, and kept deliberately -- see the ledger. The runtime
            // accepts this construct (`break` is `cl = find_done(cl)`, which
            // matches a `while`'s `done` too), so the script is valid and
            // only its formatting is mangled.
            indent = case_indent + 1;
            indent_next = false;
        }

        let levels = indent.max(0) as usize;
        let nlen = levels * 2; // counted whether or not the write truncates
        let mut line: Vec<u8> = Vec::new();
        for _ in 0..levels {
            if line.len() + 2 <= READ_SIZE - 1 {
                line.extend_from_slice(b"  ");
            }
        }
        // Past 128 indent levels the text would run off the 256-byte
        // buffer. Clamp to the buffer instead.
        let off = nlen.min(line.len());
        line.truncate(off);
        let mut tail = t.to_vec();
        tail.extend_from_slice(b"\r\n");
        let llen = tail.len(); // the untruncated length
        let room = READ_SIZE.saturating_sub(off).saturating_sub(1);
        line.extend_from_slice(&tail[..llen.min(room)]);

        if llen + nlen + len > eb.max_str - 1 {
            msgs.push(b"String too long, formatting aborted\r\n".to_vec());
            return false;
        }
        len += nlen + llen;
        nsc.extend_from_slice(&line);

        if indent_next {
            indent += 1;
            indent_next = false;
        }
    }

    if !block_stack.is_empty() {
        msgs.push(b"Unmatched block statements ignored.\r\n".to_vec());
    }

    eb.buf = Some(nsc);
    true
}

fn parse_edit_action(
    command: ParseCmd,
    string: &[u8],
    eb: &mut EditBuf,
    is_trigedit: bool,
    msgs: &mut Vec<Vec<u8>>,
    // The two listings go to the pager, not to the descriptor. Every
    // other message this produces -- including the listings' own
    // range rejections, which return before reaching the pager -- is
    // written straight out.
    paged: &mut Option<Vec<u8>>,
) {
    match command {
        ParseCmd::Help => msgs.push(HELP_TEXT.to_vec()),

        ParseCmd::Toggle => {
            let Some(buf) = eb.buf.as_mut() else {
                msgs.push(b"No string.\r\n".to_vec());
                return;
            };
            let mut has_at = false;
            let mut c = 0usize;
            while c < buf.len() {
                if buf[c] == b'@' {
                    c += 1;
                    if ch(buf, c) != b'@' {
                        has_at = true;
                        break;
                    }
                }
                c += 1;
            }
            if has_at {
                parse_at(buf);
                msgs.push(b"Toggling (at) into (tab) Characters...\r\n".to_vec());
            } else {
                parse_tab(buf);
                msgs.push(b"Toggling (tab) into (at) Characters...\r\n".to_vec());
            }
        }

        ParseCmd::Format => {
            if is_trigedit {
                let formatted = format_script(eb, msgs);
                let msg: &[u8] = if formatted {
                    b"Script formatted.\r\n"
                } else {
                    b"Script not formatted.\r\n"
                };
                msgs.push(msg.to_vec());
                return;
            }
            let mut indent = false;
            let mut flags = 0i32;
            let mut j = 0usize;
            while is_alpha(ch(string, j)) && j < 2 {
                let cj = string[j];
                j += 1;
                if cj == b'i' && !indent {
                    indent = true;
                    flags += FORMAT_INDENT;
                }
            }
            let scan_from: &[u8] = if indent { string.get(1..).unwrap_or(&[]) } else { string };
            let (line_low, line_high) = match parse_range(scan_from) {
                (-1, ..) | (0, ..) => (1, 999999),
                (1, low, _) => (low, low),
                (_, low, high) => {
                    if high < low {
                        // This message carries a literal backslash-r
                        // backslash-n, not a CRLF.
                        msgs.push(b"That range is invalid.\\r\\n".to_vec());
                        return;
                    }
                    (low, high)
                }
            };
            let line_low = line_low.max(1); // in case line_low is negative or zero

            format_text(eb, flags, line_low, line_high, msgs);
            let msg: &[u8] = if indent {
                b"Text formatted with indent.\r\n"
            } else {
                b"Text formatted without indent.\r\n"
            };
            msgs.push(msg.to_vec());
        }

        ParseCmd::Replace => {
            let mut rep_all = false;
            let mut j = 0usize;
            while is_alpha(ch(string, j)) && j < 2 {
                let cj = string[j];
                j += 1;
                if cj == b'a' {
                    rep_all = true;
                }
            }
            let toks = quote_tokens(string);
            if toks.is_empty() {
                msgs.push(b"Invalid format.\r\n".to_vec());
                return;
            }
            let Some(&s) = toks.get(1) else {
                msgs.push(b"Target string must be enclosed in single quotes.\r\n".to_vec());
                return;
            };
            if toks.get(2).is_none() {
                msgs.push(b"No replacement string.\r\n".to_vec());
                return;
            }
            let Some(&t) = toks.get(3) else {
                msgs.push(b"Replacement string must be enclosed in single quotes.\r\n".to_vec());
                return;
            };
            // wb's fix for empty buffer replacement crashing
            let Some(buf) = eb.buf.as_mut() else {
                return;
            };
            // unsigned int total_len = (strlen(t) - strlen(s)) + strlen(*d->str),
            // computed in size_t then truncated to 32 bits.
            let total_len = (t.len() as u64)
                .wrapping_sub(s.len() as u64)
                .wrapping_add(buf.len() as u64) as u32;
            if total_len as u64 <= eb.max_str as u64 {
                let replaced = replace_str(buf, s, t, rep_all, eb.max_str as u32);
                if replaced > 0 {
                    let mut m = b"Replaced ".to_vec();
                    m.extend_from_slice(&itoa(replaced));
                    m.extend_from_slice(b" occurence");
                    m.extend_from_slice(if replaced != 1 { b"s " } else { b" " });
                    m.extend_from_slice(b"of '");
                    m.extend_from_slice(s);
                    m.extend_from_slice(b"' with '");
                    m.extend_from_slice(t);
                    m.extend_from_slice(b"'.\r\n");
                    msgs.push(m);
                } else if replaced == 0 {
                    let mut m = b"String '".to_vec();
                    m.extend_from_slice(s);
                    m.extend_from_slice(b"' not found.\r\n");
                    msgs.push(m);
                } else {
                    msgs.push(
                        b"ERROR: Replacement string causes buffer overflow, aborted replace.\r\n"
                            .to_vec(),
                    );
                }
            } else {
                msgs.push(b"Not enough space left in buffer.\r\n".to_vec());
            }
        }

        ParseCmd::Delete => {
            let (line_low, line_high) = match parse_range(string) {
                // Count -1 leaves the range unset; treat it as count 0.
                (-1, ..) | (0, ..) => {
                    msgs.push(b"You must specify a line number or range to delete.\r\n".to_vec());
                    return;
                }
                (1, low, _) => (low, low),
                (_, low, high) => {
                    if high < low {
                        msgs.push(b"That range is invalid.\r\n".to_vec());
                        return;
                    }
                    (low, high)
                }
            };

            let mut i: i32 = 1;
            let mut total_len: i32 = 1;
            let Some(buf) = eb.buf.as_mut() else {
                msgs.push(b"Buffer is empty.\r\n".to_vec());
                return;
            };
            if line_low > 0 {
                let mut s: Option<usize> = Some(0);
                while let Some(p) = s {
                    if i >= line_low {
                        break;
                    }
                    s = find_nl(buf, p).map(|q| {
                        i += 1;
                        q + 1
                    });
                }
                let Some(start) = s.filter(|_| i >= line_low) else {
                    msgs.push(b"Line(s) out of range; not deleting.\r\n".to_vec());
                    return;
                };
                let t = start;
                let mut s: Option<usize> = Some(start);
                while let Some(p) = s {
                    if i >= line_high {
                        break;
                    }
                    s = find_nl(buf, p).map(|q| {
                        i += 1;
                        total_len += 1;
                        q + 1
                    });
                }
                let tail_from = s.and_then(|p| find_nl(buf, p));
                if let Some(q) = tail_from {
                    // while (*(++s)) *(t++) = *s; — shift the tail after
                    // line_high's '\n' down to t, then terminate.
                    let tail: Vec<u8> = buf[q + 1..].to_vec();
                    buf.truncate(t);
                    buf.extend_from_slice(&tail);
                } else {
                    total_len -= 1;
                    buf.truncate(t);
                }
                // RECREATE(*d->str, char, strlen + 3): allocation-only, no-op here.

                let mut m = itoa(total_len);
                m.extend_from_slice(b" line");
                m.extend_from_slice(if total_len != 1 { b"s " } else { b" " });
                m.extend_from_slice(b"deleted.\r\n");
                msgs.push(m);
            } else {
                msgs.push(b"Invalid, line numbers to delete must be higher than 0.\r\n".to_vec());
            }
        }

        ParseCmd::ListNorm => {
            let (line_low, line_high) = if !string.is_empty() {
                match parse_range(string) {
                    // Count -1 leaves the range unset; use the case-0
                    // default.
                    (-1, ..) | (0, ..) => (1, 999999),
                    (1, low, _) => (low, low),
                    (_, low, high) => (low, high),
                }
            } else {
                (1, 999999)
            };

            if line_low < 1 {
                msgs.push(b"Line numbers must be greater than 0.\r\n".to_vec());
                return;
            } else if line_high < line_low {
                msgs.push(b"That range is invalid.\r\n".to_vec());
                return;
            }
            let mut out: Vec<u8> = Vec::new();
            if line_high < 999999 || line_low > 1 {
                let mut header = b"Current buffer range [".to_vec();
                header.extend_from_slice(&itoa(line_low));
                header.extend_from_slice(b" - ");
                header.extend_from_slice(&itoa(line_high));
                header.extend_from_slice(b"]:\r\n");
                append_at_most(&mut out, &header, MAX_STRING_LENGTH - 1);
            }
            let mut i: i32 = 1;
            let mut total_len: i32 = 0;
            // s = *d->str: a NULL buffer skips the walk and fails the range
            // check below (unreachable via /l, which requires a buffer).
            let Some(buf) = eb.buf.as_deref() else {
                msgs.push(b"Line(s) out of range; no buffer listing.\r\n".to_vec());
                return;
            };
            let mut s: Option<usize> = Some(0);
            while let Some(p) = s {
                if i >= line_low {
                    break;
                }
                s = find_nl(buf, p).map(|q| {
                    i += 1;
                    q + 1
                });
            }
            let Some(start) = s.filter(|_| i >= line_low) else {
                msgs.push(b"Line(s) out of range; no buffer listing.\r\n".to_vec());
                return;
            };
            let t = start;
            let mut s: Option<usize> = Some(start);
            while let Some(p) = s {
                if i > line_high {
                    break;
                }
                s = find_nl(buf, p).map(|q| {
                    i += 1;
                    total_len += 1;
                    q + 1
                });
            }
            match s {
                Some(end) => append_within(&mut out, &buf[t..end], MAX_STRING_LENGTH),
                None => append_within(&mut out, &buf[t..], MAX_STRING_LENGTH),
            }
            // This is kind of annoying...but some people like it.
            let mut count_line = b"\r\n".to_vec();
            count_line.extend_from_slice(&itoa(total_len));
            count_line.extend_from_slice(b" line");
            count_line.extend_from_slice(if total_len != 1 { b"s " } else { b" " });
            count_line.extend_from_slice(b"shown.\r\n");
            append_within(&mut out, &count_line, MAX_STRING_LENGTH);
            // page_string(d, buf, TRUE)
            if !out.is_empty() {
                *paged = Some(out);
            }
        }

        ParseCmd::ListNum => {
            let (line_low, line_high) = if !string.is_empty() {
                match parse_range(string) {
                    (-1, ..) | (0, ..) => (1, 999999),
                    (1, low, _) => (low, low),
                    (_, low, high) => (low, high),
                }
            } else {
                (1, 999999)
            };

            if line_low < 1 {
                msgs.push(b"Line numbers must be greater than 0.\r\n".to_vec());
                return;
            }
            if line_high < line_low {
                msgs.push(b"That range is invalid.\r\n".to_vec());
                return;
            }
            let mut out: Vec<u8> = Vec::new();
            let mut i: i32 = 1;
            // As in ListNorm: a NULL buffer fails the post-walk range check.
            let Some(buf) = eb.buf.as_deref() else {
                msgs.push(b"Line(s) out of range; no buffer listing.\r\n".to_vec());
                return;
            };
            let mut s: Option<usize> = Some(0);
            while let Some(p) = s {
                if i >= line_low {
                    break;
                }
                s = find_nl(buf, p).map(|q| {
                    i += 1;
                    q + 1
                });
            }
            let Some(start) = s.filter(|_| i >= line_low) else {
                msgs.push(b"Line(s) out of range; no buffer listing.\r\n".to_vec());
                return;
            };
            let mut t = start;
            let mut s: Option<usize> = Some(start);
            while let Some(p) = s {
                if i > line_high {
                    break;
                }
                match find_nl(buf, p) {
                    Some(q) => {
                        i += 1;
                        let after = q + 1;
                        let numbered = format!("{:4}: ", i - 1).into_bytes();
                        append_within(&mut out, &numbered, MAX_STRING_LENGTH);
                        append_within(&mut out, &buf[t..after], MAX_STRING_LENGTH);
                        t = after;
                        s = Some(after);
                    }
                    None => s = None,
                }
            }
            match s {
                Some(end) => append_within(&mut out, &buf[t..end], MAX_STRING_LENGTH),
                None => append_within(&mut out, &buf[t..], MAX_STRING_LENGTH),
            }
            // page_string(d, buf, TRUE): silently drops an empty string.
            if !out.is_empty() {
                *paged = Some(out);
            }
        }

        ParseCmd::Insert => {
            let (arg1, mut insert_text) = half_chop(string);
            if arg1.is_empty() {
                msgs.push(b"You must specify a line number before which to insert text.\r\n".to_vec());
                return;
            }
            let line_low = parse_int_prefix(&arg1);
            append_within(&mut insert_text, b"\r\n", MAX_STRING_LENGTH - 1);

            let mut i: i32 = 1;
            let max_str = eb.max_str;
            let Some(buf) = eb.buf.as_mut() else {
                msgs.push(b"Buffer is empty, nowhere to insert.\r\n".to_vec());
                return;
            };
            if line_low > 0 {
                let mut s: Option<usize> = Some(0);
                while let Some(p) = s {
                    if i >= line_low {
                        break;
                    }
                    s = find_nl(buf, p).map(|q| {
                        i += 1;
                        q + 1
                    });
                }
                let Some(pos) = s.filter(|_| i >= line_low) else {
                    msgs.push(b"Line number out of range; insert aborted.\r\n".to_vec());
                    return;
                };
                // strlen(*d->str) [prefix, *s nulled] + strlen(buf2) +
                // strlen(s + 1) + 3 > d->max_str. strlen(s+1) reads past the
                // terminator when s is at the end (UB); stand-in: 0.
                let suffix_after_first = buf.len().saturating_sub(pos).saturating_sub(1);
                if pos + insert_text.len() + suffix_after_first + 3 > max_str {
                    msgs.push(
                        b"Insert text pushes buffer over maximum size, insert aborted.\r\n"
                            .to_vec(),
                    );
                    return;
                }
                let mut new_buf: Vec<u8> = Vec::new();
                if pos > 0 {
                    append_within(&mut new_buf, &buf[..pos], MAX_STRING_LENGTH);
                }
                append_within(&mut new_buf, &insert_text, MAX_STRING_LENGTH);
                if pos < buf.len() {
                    append_within(&mut new_buf, &buf[pos..], MAX_STRING_LENGTH);
                }
                *buf = new_buf;
                msgs.push(b"Line inserted.\r\n".to_vec());
            } else {
                msgs.push(b"Line number must be higher than 0.\r\n".to_vec());
            }
        }

        ParseCmd::Edit => {
            let (arg1, mut new_line) = half_chop(string);
            if arg1.is_empty() {
                msgs.push(b"You must specify a line number at which to change text.\r\n".to_vec());
                return;
            }
            let line_low = parse_int_prefix(&arg1);
            append_within(&mut new_line, b"\r\n", MAX_STRING_LENGTH - 1);

            let mut i: i32 = 1;
            let max_str = eb.max_str;
            let Some(buf) = eb.buf.as_mut() else {
                msgs.push(b"Buffer is empty, nothing to change.\r\n".to_vec());
                return;
            };
            if line_low > 0 {
                // Loop through the text counting \n characters until we get
                // to the line.
                let mut s: Option<usize> = Some(0);
                while let Some(p) = s {
                    if i >= line_low {
                        break;
                    }
                    s = find_nl(buf, p).map(|q| {
                        i += 1;
                        q + 1
                    });
                }
                let Some(pos) = s.filter(|_| i >= line_low) else {
                    msgs.push(b"Line number out of range; change aborted.\r\n".to_vec());
                    return;
                };
                let mut new_buf: Vec<u8> = Vec::new();
                if pos != 0 {
                    append_within(&mut new_buf, &buf[..pos], MAX_STRING_LENGTH);
                }
                append_within(&mut new_buf, &new_line, MAX_STRING_LENGTH);
                if let Some(q) = find_nl(buf, pos) {
                    append_within(&mut new_buf, &buf[q + 1..], MAX_STRING_LENGTH);
                }
                if new_buf.len() > max_str {
                    msgs.push(
                        b"Change causes new length to exceed buffer maximum size, aborted.\r\n"
                            .to_vec(),
                    );
                    return;
                }
                *buf = new_buf;
                msgs.push(b"Line changed.\r\n".to_vec());
            } else {
                msgs.push(b"Line number must be higher than 0.\r\n".to_vec());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// replace_str
// ---------------------------------------------------------------------------

/// Returns the occurrence count, 0 for "not found" (including the mid-loop
/// overflow abort, which also reports 0 after truncating the string at the
/// failing match — see below), or -1 when the up-front size check
/// fails.
fn replace_str(
    string: &mut Vec<u8>,
    pattern: &[u8],
    replacement: &[u8],
    rep_all: bool,
    max_size: u32,
) -> i32 {
    // (strlen(*string) - strlen(pattern)) + strlen(replacement) > max_size,
    // in size_t arithmetic (wraps for pattern longer than the string).
    let check = (string.len() as u64)
        .wrapping_sub(pattern.len() as u64)
        .wrapping_add(replacement.len() as u64);
    if check > max_size as u64 {
        return -1;
    }

    let cap = max_size as usize;
    let mut replace_buffer: Vec<u8> = Vec::new();
    let mut i: i32 = 0;

    if rep_all {
        let mut jetsam = 0usize;
        let mut flow = 0usize;
        while let Some(rel) = find_sub(&string[flow..], pattern) {
            let m = flow + rel;
            i += 1;
            // temp = *flow; *flow = '\0';
            let seg_len = m - jetsam;
            if replace_buffer.len() + seg_len + replacement.len() > cap {
                i = -1;
                // Break without restoring temp: the source string is left
                // truncated at this match.
                string.truncate(m);
                break;
            }
            let seg: Vec<u8> = string[jetsam..m].to_vec();
            append_within(&mut replace_buffer, &seg, cap);
            append_within(&mut replace_buffer, replacement, cap);
            // *flow = temp; flow += strlen(pattern); jetsam = flow;
            flow = m + pattern.len();
            jetsam = flow;
            if pattern.is_empty() {
                break; // an empty pattern would never advance; tokens are non-empty
            }
        }
        let tail: Vec<u8> = string.get(jetsam..).unwrap_or(&[]).to_vec();
        append_within(&mut replace_buffer, &tail, cap);
    } else if let Some(m) = find_sub(string, pattern) {
        i += 1;
        // Copy everything before the match, clamped to the buffer.
        let n = m.min(cap.saturating_sub(1));
        replace_buffer.extend_from_slice(&string[..n]);
        append_within(&mut replace_buffer, replacement, cap);
        let rest: Vec<u8> = string[m + pattern.len()..].to_vec();
        append_within(&mut replace_buffer, &rest, cap);
    }

    if i <= 0 {
        return 0;
    }
    *string = replace_buffer;
    i
}

// ---------------------------------------------------------------------------
// format_text + text helpers
// ---------------------------------------------------------------------------

fn count_color_chars(s: &[u8]) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let len = s.len();
    let mut num = 0;
    let mut i = 0usize;
    while i < len {
        while ch(s, i) == b'\t' {
            if ch(s, i + 1) == b'\t' {
                num += 1;
            } else {
                num += 2;
            }
            i += 2;
        }
        i += 1;
    }
    num
}

/// Uppercase the first letter after any leading `\t`-color or
/// ANSI CSI codes. Applied to an owned copy.
fn capitalise(word: &mut [u8]) {
    let mut p = 0usize;
    loop {
        if word.get(p) == Some(&b'\t') && p + 1 < word.len() {
            p += 2;
        } else if word.get(p) == Some(&0x1B) && word.get(p + 1) == Some(&b'[') {
            p += 2;
            while p < word.len() && !is_alpha(word[p]) {
                p += 1;
            }
            if p < word.len() {
                p += 1;
            }
        } else {
            break;
        }
    }
    if p < word.len() && word[p].is_ascii_lowercase() {
        word[p] -= b'a' - b'A';
    }
}

/// The first line: skip leading `\n`s, then take the run up to the next one
/// or to the end. None if nothing remains.
fn first_line(s: &[u8]) -> Option<&[u8]> {
    let start = s.iter().position(|&c| c != b'\n')?;
    let end = s[start..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(s.len(), |r| start + r);
    Some(&s[start..end])
}

fn format_text(eb: &mut EditBuf, mode: i32, low: i32, high: i32, msgs: &mut Vec<Vec<u8>>) -> i32 {
    let mut line_chars: i32;
    let mut cap_next = true;
    let mut cap_next_next = false;
    let mut color_chars: i32 = 0;
    let mut pass_line = false;
    let mut formatted: Vec<u8> = Vec::new();

    // Fix memory overrun. (A SYSERR also goes to the mud log here.)
    if eb.max_str > MAX_STRING_LENGTH {
        return 0;
    }
    let maxlen = eb.max_str;
    let Some(orig) = eb.buf.as_ref() else {
        return 0;
    };
    let orig: Vec<u8> = orig.clone();
    let len = orig.len();

    // Copy lines 1..low-1 through unchanged.
    let mut fpos = 0usize;
    let mut i: i32 = 0;
    while i < low - 1 {
        let window_end = len.min(fpos + (MAX_STRING_LENGTH - 1));
        let Some(tok) = first_line(&orig[fpos..window_end]) else {
            msgs.push(b"There aren't that many lines!\r\n".to_vec());
            return 0;
        };
        let mut piece = tok.to_vec();
        piece.push(b'\n');
        append_within(&mut formatted, &piece, MAX_STRING_LENGTH);
        match find_nl(&orig, fpos) {
            Some(q) => fpos = q + 1,
            None => {
                // No newline left to advance past — treat as "not enough
                // lines".
                msgs.push(b"There aren't that many lines!\r\n".to_vec());
                return 0;
            }
        }
        i += 1;
    }

    if mode & FORMAT_INDENT != 0 {
        append_within(&mut formatted, b"   ", MAX_STRING_LENGTH);
        line_chars = 3;
    } else {
        line_chars = 0;
    }

    while ch(&orig, fpos) != 0 && i < high {
        // Skip leading whitespace, counting lines against `high`.
        while ch(&orig, fpos) != 0 && in_set(b"\n\r\x0c\x0b ", ch(&orig, fpos)) {
            if ch(&orig, fpos) == b'\n' && !pass_line {
                let old = i;
                i += 1;
                if old >= high {
                    pass_line = true;
                    break;
                }
            }
            fpos += 1;
        }

        let mut word_range: (usize, usize) = (fpos, fpos);
        if ch(&orig, fpos) != 0 {
            let start = fpos;
            // Scan the word, counting protocol color chars.
            while ch(&orig, fpos) != 0 && !in_set(b"\n\r\x0c\x0b .?!", ch(&orig, fpos)) {
                if ch(&orig, fpos) == b'\t' {
                    let nxt = ch(&orig, fpos + 1);
                    if nxt == b'\t' {
                        color_chars += 1;
                    } else if nxt == b'[' {
                        color_chars += 7;
                    } else {
                        color_chars += 2;
                    }
                    fpos += 1;
                }
                fpos += 1;
            }

            if cap_next_next {
                cap_next_next = false;
                cap_next = true;
            }

            // If we stopped on a sentence delimiter, move off it. (For a
            // string ending mid-word, stop at the end.)
            while in_set(b".!?", ch(&orig, fpos)) {
                cap_next_next = true;
                fpos += 1;
            }

            let wend;
            if in_set(b"\n\r", ch(&orig, fpos)) {
                wend = fpos; // *flow = '\0'
                fpos += 1;
                if ch(&orig, fpos) == b'\n' {
                    let old = i;
                    i += 1;
                    if old >= high {
                        pass_line = true;
                    }
                }
                while ch(&orig, fpos) != 0 && in_set(b"\n\r", ch(&orig, fpos)) && !pass_line {
                    fpos += 1;
                    if ch(&orig, fpos) == b'\n' {
                        let old = i;
                        i += 1;
                        if old >= high {
                            pass_line = true;
                        }
                    }
                }
                // temp = *flow (restored below as a no-op).
            } else {
                wend = fpos; // temp = *flow; *flow = '\0'
            }
            let word = &orig[start..wend.min(len)];
            word_range = (start, wend.min(len));

            // line_chars + strlen(start) + 1 - color_chars > PAGE_WIDTH,
            // where strlen promotes the arithmetic to size_t: a negative
            // value wraps huge and triggers the wrap.
            let width = line_chars as i64 + word.len() as i64 + 1 - color_chars as i64;
            if width as u64 > PAGE_WIDTH as u64 {
                append_within(&mut formatted, b"\r\n", MAX_STRING_LENGTH);
                line_chars = 0;
                color_chars = count_color_chars(word);
            }

            if !cap_next {
                if line_chars > 0 {
                    append_within(&mut formatted, b" ", MAX_STRING_LENGTH);
                    line_chars += 1;
                }
                line_chars += word.len() as i32;
                append_within(&mut formatted, word, MAX_STRING_LENGTH);
            } else {
                cap_next = false;
                let mut capped = word.to_vec();
                capitalise(&mut capped);
                line_chars += capped.len() as i32;
                append_within(&mut formatted, &capped, MAX_STRING_LENGTH);
            }
            // *flow = temp
        }

        if cap_next_next && ch(&orig, fpos) != 0 {
            // All-int arithmetic here (no size_t promotion).
            if line_chars + 3 - color_chars > PAGE_WIDTH {
                append_within(&mut formatted, b"\r\n", MAX_STRING_LENGTH);
                line_chars = 0;
                color_chars = count_color_chars(&orig[word_range.0..word_range.1]);
            } else if ch(&orig, fpos) == b'"' || ch(&orig, fpos) == b'\'' {
                let quote = [ch(&orig, fpos), b' ', b' '];
                append_within(&mut formatted, &quote, MAX_STRING_LENGTH);
                fpos += 1;
                line_chars += 1;
            } else {
                append_within(&mut formatted, b"  ", MAX_STRING_LENGTH);
                line_chars += 2;
            }
        }
    }

    if ch(&orig, fpos) != 0 {
        append_within(&mut formatted, b"\r\n", MAX_STRING_LENGTH);
    }
    append_within(&mut formatted, &orig[fpos.min(len)..], MAX_STRING_LENGTH);
    if ch(&orig, fpos) == 0 {
        append_within(&mut formatted, b"\r\n", MAX_STRING_LENGTH);
    }

    // int len = MIN(maxlen, strlen(formatted) + 1); copy len - 1 chars.
    let out_len = maxlen.min(formatted.len() + 1);
    formatted.truncate(out_len.saturating_sub(1));
    eb.buf = Some(formatted);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eb_with(lines: &[&[u8]], max_str: usize) -> EditBuf {
        let mut eb = EditBuf { buf: None, max_str };
        for l in lines {
            let (a, m, _) = editor_add_line(&mut eb, l, true, false);
            assert_eq!(a, EditorAction::Ok);
            assert!(m.is_empty());
        }
        eb
    }

    /// The third element is the buffer listing, which the caller pages
    /// rather than writing straight out. Only `/l` and `/n` produce one.
    fn add(eb: &mut EditBuf, line: &[u8]) -> (EditorAction, Vec<Vec<u8>>, Option<Vec<u8>>) {
        editor_add_line(eb, line, true, false)
    }

    fn three_lines() -> EditBuf {
        eb_with(&[b"one", b"two", b"three"], 2048)
    }

    fn buf(eb: &EditBuf) -> &[u8] {
        eb.buf.as_deref().expect("buffer should exist")
    }

    // -- exported helpers ---------------------------------------------------

    #[test]
    fn delete_doubledollar_collapses_pairs() {
        let mut s = b"a$$b".to_vec();
        delete_doubledollar(&mut s);
        assert_eq!(s, b"a$b");
        let mut s = b"$$".to_vec();
        delete_doubledollar(&mut s);
        assert_eq!(s, b"$");
        let mut s = b"a$$$$b".to_vec();
        delete_doubledollar(&mut s);
        assert_eq!(s, b"a$$b");
        let mut s = b"no dollars".to_vec();
        delete_doubledollar(&mut s);
        assert_eq!(s, b"no dollars");
        let mut s = b"$".to_vec();
        delete_doubledollar(&mut s);
        assert_eq!(s, b"$");
    }

    #[test]
    fn smash_tilde_erases_line_ending_tildes() {
        let mut s = b"abc~\r\ndef~x~".to_vec();
        smash_tilde(&mut s);
        assert_eq!(s, b"abc \r\ndef~x ");
        let mut s = b"~\nmid~dle".to_vec();
        smash_tilde(&mut s);
        assert_eq!(s, b" \nmid~dle");
    }

    #[test]
    fn parse_at_and_parse_tab_respect_doubling() {
        let mut s = b"@@a@b@".to_vec();
        parse_at(&mut s);
        assert_eq!(s, b"@@a\tb\t");
        let mut s = b"\t\ta\tb\t".to_vec();
        parse_tab(&mut s);
        assert_eq!(s, b"\t\ta@b@");
    }

    // -- string_add append + terminator flow ------------------------------

    #[test]
    fn append_lines_and_terminate() {
        let mut eb = EditBuf { buf: None, max_str: 2048 };
        let (a, m, _) = add(&mut eb, b"Hello there.");
        assert_eq!((a, m.len()), (EditorAction::Ok, 0));
        assert_eq!(buf(&eb), b"Hello there.\r\n");
        let (a, _, _) = add(&mut eb, b"");
        assert_eq!(a, EditorAction::Ok);
        assert_eq!(buf(&eb), b"Hello there.\r\n\r\n");
        let (a, m, _) = add(&mut eb, b"\t");
        assert_eq!((a, m.len()), (EditorAction::Save, 0));
        assert_eq!(buf(&eb), b"Hello there.\r\n\r\n");
    }

    #[test]
    fn save_with_empty_buffer_becomes_nothing() {
        // Allocated-but-empty string is replaced by "Nothing.\r\n"...
        let mut eb = EditBuf { buf: Some(Vec::new()), max_str: 2048 };
        let (a, _, _) = add(&mut eb, b"\t");
        assert_eq!(a, EditorAction::Save);
        assert_eq!(buf(&eb), b"Nothing.\r\n");
        //..but a NULL buffer stays NULL.
        let mut eb = EditBuf { buf: None, max_str: 2048 };
        let (a, _, _) = add(&mut eb, b"\t");
        assert_eq!(a, EditorAction::Save);
        assert!(eb.buf.is_none());
    }

    #[test]
    fn first_line_too_long_truncates() {
        let mut eb = EditBuf { buf: None, max_str: 10 };
        let (a, m, _) = add(&mut eb, b"abcdefghijklmno");
        assert_eq!(a, EditorAction::Ok);
        assert_eq!(m, vec![b"String too long - Truncated.\r\n".to_vec()]);
        // Truncated at max_str - 3 then "\r\n"; no further \r\n fits.
        assert_eq!(buf(&eb), b"abcdefg\r\n");
        // Non-improved editor auto-saves instead.
        let mut eb = EditBuf { buf: None, max_str: 10 };
        let (a, m, _) = editor_add_line(&mut eb, b"abcdefghijklmno", false, false);
        assert_eq!(a, EditorAction::Save);
        assert_eq!(m, vec![b"String too long - Truncated.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"abcdefg\r\n");
    }

    #[test]
    fn overflow_append_skips_line_and_demotes_action() {
        let mut eb = EditBuf { buf: Some(b"abc\r\n".to_vec()), max_str: 10 };
        let (a, m, _) = add(&mut eb, b"defgh");
        // Improved editor: OK demoted to ACTION so the player can still save.
        assert_eq!(a, EditorAction::Action);
        assert_eq!(m, vec![b"String too long.  Last line skipped.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"abc\r\n");
        // Non-improved editor auto-saves.
        let mut eb = EditBuf { buf: Some(b"abc\r\n".to_vec()), max_str: 10 };
        let (a, m, _) = editor_add_line(&mut eb, b"defgh", false, false);
        assert_eq!(a, EditorAction::Save);
        assert_eq!(m, vec![b"String too long.  Last line skipped.\r\n".to_vec()]);
        // Exact-fit boundary: strlen + strlen + 3 == max_str appends fine.
        let mut eb = EditBuf { buf: Some(b"abc\r\n".to_vec()), max_str: 10 };
        let (a, m, _) = add(&mut eb, b"de");
        assert_eq!((a, m.len()), (EditorAction::Ok, 0));
        assert_eq!(buf(&eb), b"abc\r\nde\r\n");
    }

    #[test]
    fn non_improved_treats_slash_lines_as_text() {
        let mut eb = EditBuf { buf: None, max_str: 2048 };
        let (a, m, _) = editor_add_line(&mut eb, b"/s", false, false);
        assert_eq!((a, m.len()), (EditorAction::Ok, 0));
        assert_eq!(buf(&eb), b"/s\r\n");
    }

    // -- simple commands -----------------------------------------------------

    #[test]
    fn abort_save_clear_and_invalid() {
        let mut eb = three_lines();
        let (a, m, _) = add(&mut eb, b"/a");
        assert_eq!((a, m.len()), (EditorAction::Abort, 0));
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n"); // caller restores backstr

        let (a, m, _) = add(&mut eb, b"/s");
        assert_eq!((a, m.len()), (EditorAction::Save, 0));
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");

        let (a, m, _) = add(&mut eb, b"/c");
        assert_eq!(a, EditorAction::Action);
        assert_eq!(m, vec![b"Current buffer cleared.\r\n".to_vec()]);
        assert!(eb.buf.is_none());
        let (_, m, _) = add(&mut eb, b"/c");
        assert_eq!(m, vec![b"Current buffer empty.\r\n".to_vec()]);

        let (a, m, _) = add(&mut eb, b"/x");
        assert_eq!(a, EditorAction::Action);
        assert_eq!(m, vec![b"Invalid option.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/");
        assert_eq!(m, vec![b"Invalid option.\r\n".to_vec()]);

        // /f /i /l /n require a buffer.
        let (_, m, _) = add(&mut eb, b"/f");
        assert_eq!(m, vec![b"Current buffer empty.\r\n".to_vec()]);
        // /t reports differently (no gate in improved_editor_execute).
        let (_, m, _) = add(&mut eb, b"/t");
        assert_eq!(m, vec![b"No string.\r\n".to_vec()]);
    }

    // -- /d ------------------------------------------------------------------

    #[test]
    fn delete_single_line() {
        let mut eb = three_lines();
        let (a, m, _) = add(&mut eb, b"/d 2");
        assert_eq!(a, EditorAction::Action);
        assert_eq!(m, vec![b"1 line deleted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\nthree\r\n");
    }

    #[test]
    fn delete_range_and_errors() {
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/d 1 - 2");
        assert_eq!(m, vec![b"2 lines deleted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"three\r\n");

        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/d 2-3"); // a range with no spaces is accepted
        assert_eq!(m, vec![b"2 lines deleted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\n");

        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/d");
        assert_eq!(m, vec![b"You must specify a line number or range to delete.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/d x");
        assert_eq!(m, vec![b"You must specify a line number or range to delete.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/d 3 - 1");
        assert_eq!(m, vec![b"That range is invalid.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/d 0");
        assert_eq!(m, vec![b"Invalid, line numbers to delete must be higher than 0.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/d 5");
        assert_eq!(m, vec![b"Line(s) out of range; not deleting.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn delete_past_last_newline_reports_zero_lines() {
        // A quirk: /d N where N == lines + 1 lands on the terminator and
        // reports "0 lines deleted." without changing anything.
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/d 4");
        assert_eq!(m, vec![b"0 lines deleted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");
    }

    // -- /e and /i -----------------------------------------------------------

    #[test]
    fn edit_line() {
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/e 2 TWO");
        assert_eq!(m, vec![b"Line changed.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\nTWO\r\nthree\r\n");

        // Number may abut the command letter.
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/e1 first");
        assert_eq!(m, vec![b"Line changed.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"first\r\ntwo\r\nthree\r\n");

        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/e");
        assert_eq!(m, vec![b"You must specify a line number at which to change text.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/e 9 x");
        assert_eq!(m, vec![b"Line number out of range; change aborted.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/e 0 x");
        assert_eq!(m, vec![b"Line number must be higher than 0.\r\n".to_vec()]);

        let mut eb = EditBuf { buf: Some(b"one\r\n".to_vec()), max_str: 8 };
        let (_, m, _) = add(&mut eb, b"/e 1 abcdefgh");
        assert_eq!(m, vec![b"Change causes new length to exceed buffer maximum size, aborted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\n");
    }

    #[test]
    fn insert_line() {
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/i 2 middle");
        assert_eq!(m, vec![b"Line inserted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\nmiddle\r\ntwo\r\nthree\r\n");

        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/i 1 first");
        assert_eq!(m, vec![b"Line inserted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"first\r\none\r\ntwo\r\nthree\r\n");

        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/i");
        assert_eq!(m, vec![b"You must specify a line number before which to insert text.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/i 9 x");
        assert_eq!(m, vec![b"Line number out of range; insert aborted.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/i 0 x");
        assert_eq!(m, vec![b"Line number must be higher than 0.\r\n".to_vec()]);

        let mut eb = EditBuf { buf: Some(b"one\r\n".to_vec()), max_str: 12 };
        let (_, m, _) = add(&mut eb, b"/i 1 abcdefgh");
        assert_eq!(m, vec![b"Insert text pushes buffer over maximum size, insert aborted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\n");
    }

    // -- /l and /n -----------------------------------------------------------

    #[test]
    fn list_normal() {
        let mut eb = three_lines();
        let (a, m, paged) = add(&mut eb, b"/l");
        assert_eq!(a, EditorAction::Action);
        assert!(m.is_empty(), "a listing goes to the pager, not to msgs");
        assert_eq!(paged, Some(b"one\r\ntwo\r\nthree\r\n\r\n3 lines shown.\r\n".to_vec()));

        let (_, _, paged) = add(&mut eb, b"/l 2");
        assert_eq!(paged, Some(b"Current buffer range [2 - 2]:\r\ntwo\r\n\r\n1 line shown.\r\n".to_vec()));

        // Landing exactly on the terminator lists zero lines...
        let (_, _, paged) = add(&mut eb, b"/l 4");
        assert_eq!(paged, Some(b"Current buffer range [4 - 4]:\r\n\r\n0 lines shown.\r\n".to_vec()));
        //..but one past it is out of range.
        let (_, m, _) = add(&mut eb, b"/l 5");
        assert_eq!(m, vec![b"Line(s) out of range; no buffer listing.\r\n".to_vec()]);

        let (_, m, _) = add(&mut eb, b"/l 0");
        assert_eq!(m, vec![b"Line numbers must be greater than 0.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/l 3 - 2");
        assert_eq!(m, vec![b"That range is invalid.\r\n".to_vec()]);
    }

    #[test]
    fn list_numbered() {
        let mut eb = three_lines();
        let (_, _, paged) = add(&mut eb, b"/n");
        assert_eq!(paged, Some(b"   1: one\r\n   2: two\r\n   3: three\r\n".to_vec()));

        let (_, _, paged) = add(&mut eb, b"/n 2 - 3");
        assert_eq!(paged, Some(b"   2: two\r\n   3: three\r\n".to_vec()));

        // An unterminated trailing line is listed without a number.
        let mut eb = EditBuf { buf: Some(b"one\r\ntwo".to_vec()), max_str: 2048 };
        let (_, _, paged) = add(&mut eb, b"/n");
        assert_eq!(paged, Some(b"   1: one\r\ntwo".to_vec()));
    }

    // -- /r and /ra ----------------------------------------------------------

    #[test]
    fn replace_first_occurrence() {
        let mut eb = three_lines();
        let (a, m, _) = add(&mut eb, b"/r 'two' 'TWO'");
        assert_eq!(a, EditorAction::Action);
        // Exact, including the "occurence" misspelling and the space
        // carried by the %s before "of".
        assert_eq!(m, vec![b"Replaced 1 occurence of 'two' with 'TWO'.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\nTWO\r\nthree\r\n");
    }

    #[test]
    fn replace_all_occurrences() {
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/ra 'e' 'E'");
        assert_eq!(m, vec![b"Replaced 3 occurences of 'e' with 'E'.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"onE\r\ntwo\r\nthrEE\r\n");
    }

    #[test]
    fn replace_errors() {
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/r 'zzz' 'x'");
        assert_eq!(m, vec![b"String 'zzz' not found.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/r");
        assert_eq!(m, vec![b"Invalid format.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/r foo");
        assert_eq!(m, vec![b"Target string must be enclosed in single quotes.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/r 'a'");
        assert_eq!(m, vec![b"No replacement string.\r\n".to_vec()]);
        let (_, m, _) = add(&mut eb, b"/r 'a' b");
        assert_eq!(m, vec![b"Replacement string must be enclosed in single quotes.\r\n".to_vec()]);

        let mut eb = EditBuf { buf: Some(b"one\r\ntwo\r\nthree\r\n".to_vec()), max_str: 20 };
        let (_, m, _) = add(&mut eb, b"/r 'e' 'EEEEEEEE'");
        assert_eq!(m, vec![b"Not enough space left in buffer.\r\n".to_vec()]);
    }

    #[test]
    fn replace_all_overflow_truncates_and_reports_not_found() {
        // A quirk: the mid-loop overflow abort in replace_str breaks with
        // the scratch '\0' still written, truncating the buffer at the match,
        // and returns 0 — so the player sees "not found".
        let mut eb = EditBuf { buf: Some(b"aaaa\r\n".to_vec()), max_str: 12 };
        let (_, m, _) = add(&mut eb, b"/ra 'a' 'bbbb'");
        assert_eq!(m, vec![b"String 'a' not found.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"aaa");
    }

    // -- /t ------------------------------------------------------------------

    #[test]
    fn toggle_at_and_tab() {
        let mut eb = EditBuf { buf: Some(b"say @Rhello@@world\r\n".to_vec()), max_str: 2048 };
        let (_, m, _) = add(&mut eb, b"/t");
        assert_eq!(m, vec![b"Toggling (at) into (tab) Characters...\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"say \tRhello@@world\r\n");
        let (_, m, _) = add(&mut eb, b"/t");
        assert_eq!(m, vec![b"Toggling (tab) into (at) Characters...\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"say @Rhello@@world\r\n");
    }

    // -- /h ------------------------------------------------------------------

    #[test]
    fn help_text_is_byte_exact() {
        let mut eb = three_lines();
        let (a, m, _) = add(&mut eb, b"/h");
        assert_eq!(a, EditorAction::Action);
        let expected: &[u8] = b"Editor command formats: /<letter>\r\n\r\n\
/a         -  aborts editor\r\n\
/c         -  clears buffer\r\n\
/d#        -  deletes a line #\r\n\
/e# <text> -  changes the line at # with <text>\r\n\
/f         -  formats text\r\n\
/fi        -  indented formatting of text\r\n\
/h         -  list text editor commands\r\n\
/i# <text> -  inserts <text> before line #\r\n\
/l         -  lists buffer\r\n\
/n         -  lists buffer with line numbers\r\n\
/r 'a' 'b' -  replace 1st occurence of text <a> in buffer with text <b>\r\n\
/ra 'a' 'b'-  replace all occurences of text <a> within buffer with text <b>\r\n";
        let mut full = expected.to_vec();
        full.extend_from_slice(b"              usage: /r[a] 'pattern' 'replacement'\r\n");
        full.extend_from_slice(b"/t         -  toggles '@' and tabs\r\n");
        full.extend_from_slice(b"/s         -  saves text\r\n");
        full.extend_from_slice(b"\r\n");
        full.extend_from_slice(
            b"A line number or range narrows /d, /f, /fi, /l and /n to those lines:\r\n",
        );
        full.extend_from_slice(
            b"              usage: /fi 5  (line 5)   /fi 5-9  (lines 5 through 9)\r\n",
        );
        assert_eq!(m, vec![full]);
        // The usage line keeps its 14-space indent.
        let msg = &m[0];
        let text = std::str::from_utf8(msg).unwrap();
        assert!(text.contains("\r\n              usage: /r[a] 'pattern' 'replacement'\r\n"));
    }

    // -- /f ------------------------------------------------------------------

    #[test]
    fn format_simple_paragraph() {
        let mut eb = eb_with(&[b"this is a test.", b"second sentence here."], 2048);
        let (a, m, _) = add(&mut eb, b"/f");
        assert_eq!(a, EditorAction::Action);
        assert_eq!(m, vec![b"Text formatted without indent.\r\n".to_vec()]);
        // Capitalized sentence starts, two spaces after the ender, one line.
        assert_eq!(buf(&eb), b"This is a test.  Second sentence here.\r\n");
    }

    #[test]
    fn format_with_indent() {
        let mut eb = eb_with(&[b"this is a test.", b"second sentence here."], 2048);
        let (_, m, _) = add(&mut eb, b"/fi");
        assert_eq!(m, vec![b"Text formatted with indent.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"   This is a test.  Second sentence here.\r\n");
    }

    #[test]
    fn format_wraps_at_page_width() {
        // Twelve 9-char words: 8 fit on the first 79-char line, 4 spill.
        let mut line = b"abcdefghi".to_vec();
        for _ in 0..11 {
            line.extend_from_slice(b" abcdefghi");
        }
        let mut eb = EditBuf { buf: None, max_str: 2048 };
        let (a, _, _) = add(&mut eb, &line);
        assert_eq!(a, EditorAction::Ok);
        let (_, m, _) = add(&mut eb, b"/f");
        assert_eq!(m, vec![b"Text formatted without indent.\r\n".to_vec()]);
        let mut expected = b"Abcdefghi".to_vec();
        for _ in 0..7 {
            expected.extend_from_slice(b" abcdefghi");
        }
        expected.extend_from_slice(b"\r\nabcdefghi");
        for _ in 0..3 {
            expected.extend_from_slice(b" abcdefghi");
        }
        expected.extend_from_slice(b"\r\n");
        assert_eq!(buf(&eb), &expected[..]);
    }

    #[test]
    fn format_range_leaves_tail_verbatim() {
        // /f 1 - 1 formats only line 1, then appends the sentence spacing,
        // "\r\n", and the untouched remainder.
        let mut eb = eb_with(&[b"one.", b"two."], 2048);
        let (_, m, _) = add(&mut eb, b"/f 1 - 1");
        assert_eq!(m, vec![b"Text formatted without indent.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"One.  \r\ntwo.\r\n");
    }

    #[test]
    fn format_invalid_range_uses_literal_backslash_message() {
        // This message carries a literal backslash-r backslash-n, not a
        // CRLF — that is what the player receives.
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/f 2 - 1");
        assert_eq!(m, vec![b"That range is invalid.\\r\\n".to_vec()]);
        assert!(m[0].ends_with(b"invalid.\x5cr\x5cn"));
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn format_too_few_lines_still_reports_formatted() {
        // format_text's return value is ignored by PARSE_FORMAT: both
        // messages are sent.
        let mut eb = three_lines();
        let (_, m, _) = add(&mut eb, b"/f 99");
        assert_eq!(
            m,
            vec![
                b"There aren't that many lines!\r\n".to_vec(),
                b"Text formatted without indent.\r\n".to_vec(),
            ]
        );
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn format_trigedit_runs_format_script() {
        let mut eb = three_lines();
        let (a, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(a, EditorAction::Action);
        // Plain lines: no block structure, so they come back unchanged.
        assert_eq!(m, vec![b"Script formatted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn format_script_indents_blocks() {
        let mut eb = EditBuf {
            buf: Some(
                b"if %actor.level% > 10\r\nsay hi\r\nelse\r\nsay bye\r\nend\r\n%echo% done\r\n"
                    .to_vec(),
            ),
            max_str: 16384,
        };
        let (_, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(m, vec![b"Script formatted.\r\n".to_vec()]);
        assert_eq!(
            buf(&eb),
            b"if %actor.level% > 10\r\n  say hi\r\nelse\r\n  say bye\r\nend\r\n%echo% done\r\n"
        );
    }

    #[test]
    fn format_script_indents_switch_cases() {
        let mut eb = EditBuf {
            buf: Some(
                b"switch %actor.class%\r\ncase mage\r\nsay spell\r\nbreak\r\ndefault\r\nsay hit\r\nbreak\r\ndone\r\n"
                    .to_vec(),
            ),
            max_str: 16384,
        };
        let (_, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(m, vec![b"Script formatted.\r\n".to_vec()]);
        assert_eq!(
            buf(&eb),
            b"switch %actor.class%\r\n  case mage\r\n    say spell\r\n    break\r\n  default\r\n    say hit\r\n    break\r\ndone\r\n"
        );
    }

    #[test]
    fn format_script_rejects_unmatched_end() {
        let mut eb = EditBuf { buf: Some(b"say hi\r\nend\r\n".to_vec()), max_str: 16384 };
        let (_, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(
            m,
            vec![
                b"Unmatched 'end' or 'done' (line 2)!\r\n".to_vec(),
                b"Script not formatted.\r\n".to_vec(),
            ]
        );
        // The buffer is left exactly as it was.
        assert_eq!(buf(&eb), b"say hi\r\nend\r\n");
    }

    #[test]
    fn format_script_keyword_match_is_case_insensitive() {
        // Keyword matching is case-insensitive, so IF/End still count.
        let mut eb = EditBuf {
            buf: Some(b"IF 1\r\nsay hi\r\nEnd\r\n".to_vec()),
            max_str: 16384,
        };
        let (_, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(m, vec![b"Script formatted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"IF 1\r\n  say hi\r\nEnd\r\n");
    }

    #[test]
    fn format_script_drops_blank_lines() {
        // Runs of line endings collapse, so blank lines never survive.
        let mut eb = EditBuf {
            buf: Some(b"say one\r\n\r\n\r\nsay two\r\n".to_vec()),
            max_str: 16384,
        };
        let (_, m, _) = editor_add_line(&mut eb, b"/f", true, true);
        assert_eq!(m, vec![b"Script formatted.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"say one\r\nsay two\r\n");
    }

    #[test]
    fn format_unterminated_text() {
        // Text without a trailing newline formats cleanly.
        let mut eb = EditBuf { buf: Some(b"hello".to_vec()), max_str: 2048 };
        let (_, m, _) = add(&mut eb, b"/f");
        assert_eq!(m, vec![b"Text formatted without indent.\r\n".to_vec()]);
        assert_eq!(buf(&eb), b"Hello\r\n");
    }
}
