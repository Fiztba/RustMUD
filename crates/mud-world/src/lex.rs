//! World-file lexical primitives: get_line (READ_SIZE 256), fread_string,
//! asciiflag_conv, parse_at.
//!
//! Their *quirks* are deliberate, because world files on disk depend on
//! them: chunked reads (a >255-byte physical line comes
//! back from get_line as multiple "lines"; fread_string inserts "\r\n" at
//! 511-byte chunk boundaries), blank/comment skipping only in get_line,
//! exactly one line-final '~' stripped, "@@" left intact by parse_at, and
//! asciiflag's masked-shift behavior for letters past 'F'.

/// Cursor over a whole file's bytes with stdio-shaped reads.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    /// 1-based count of physical lines consumed; used in error messages.
    pub line_no: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0, line_no: 0 }
    }

    pub fn at_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Bytes up to and including the next `\n`, at most `n - 1` of them.
    /// None at end of input.
    fn read_raw_line(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let max = n - 1;
        let rest = &self.data[self.pos..];
        let take = match rest.iter().take(max).position(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => max.min(rest.len()),
        };
        let out = &rest[..take];
        self.pos += take;
        if out.ends_with(b"\n") {
            self.line_no += 1;
        }
        Some(out)
    }

    /// get_line: skip lines whose first byte is '*', '\n' or '\r';
    /// strip ALL trailing '\r'/'\n'. Returns None at EOF.
    ///
    /// `bufsize` is the caller's buffer, so at most `bufsize - 1` bytes come
    /// back. A line that did not fit has
    /// its tail discarded rather than left in the stream, where the next
    /// call would return the fragment as a line of its own.
    pub fn get_line_sized(&mut self, bufsize: usize) -> Option<Vec<u8>> {
        loop {
            let chunk = self.read_raw_line(bufsize)?;
            let first = chunk.first().copied();
            let fit = chunk.ends_with(b"\n");
            let mut end = chunk.len();
            while end > 0 && (chunk[end - 1] == b'\n' || chunk[end - 1] == b'\r') {
                end -= 1;
            }
            let out = chunk[..end].to_vec();
            if !fit {
                self.skip_to_eol();
            }
            if matches!(first, Some(b'*') | Some(b'\n') | Some(b'\r')) {
                continue;
            }
            return Some(out);
        }
    }

    /// Discard the remainder of a physical line the caller's buffer could not
    /// hold, counting it as consumed.
    fn skip_to_eol(&mut self) {
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            if c == b'\n' {
                self.line_no += 1;
                break;
            }
        }
    }

    /// get_line for the 39 call sites that hand it a `char[READ_SIZE]`.
    pub fn get_line(&mut self) -> Option<Vec<u8>> {
        self.get_line_sized(256)
    }

    /// Accumulate 511-byte chunks until one ends
    /// (after trailing \r\n stripping) in '~'. Non-terminal chunks get their
    /// line ending normalized to exactly "\r\n" (appended even mid-line for
    /// over-long physical lines — a deliberate quirk). parse_at applied to
    /// the whole result. Empty => None. Err on EOF.
    pub fn fread_string(&mut self, error: &str) -> Result<Option<Vec<u8>>, String> {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = self
                .read_raw_line(512)
                .ok_or_else(|| format!("fread_string: format error at or near {error}"))?;
            let mut tmp = chunk.to_vec();
            // point walks back over trailing \r\n but never past tmp[0].
            let mut point = tmp.len().saturating_sub(1);
            while point > 0 && (tmp[point] == b'\r' || tmp[point] == b'\n') {
                point -= 1;
            }
            if tmp.get(point) == Some(&b'~') {
                tmp.truncate(point);
                buf.extend_from_slice(&tmp);
                break;
            } else {
                if matches!(tmp.get(point), Some(b'\n') | Some(b'\r')) {
                    tmp.truncate(point);
                } else {
                    tmp.truncate(point + 1);
                }
                tmp.extend_from_slice(b"\r\n");
                buf.extend_from_slice(&tmp);
            }
            if buf.len() >= 49152 {
                return Err(format!("fread_string: string too large ({error})"));
            }
        }
        parse_at(&mut buf);
        Ok(if buf.is_empty() { None } else { Some(buf) })
    }

    /// One raw physical line INCLUDING its line ending,
    /// with NO blank/comment skipping and no trimming. v3 shop buy-type
    /// lists are read this way (buffer size MAX_STRING_LENGTH), so '*' and
    /// blank lines are NOT skipped inside them.
    pub fn raw_gets(&mut self, n: usize) -> Option<&'a [u8]> {
        self.read_raw_line(n)
    }
}

/// tag_argument: the first 4 bytes are the tag, then every
/// consecutive ':' or ' ' is skipped and the rest is the value.
pub fn tag_argument(line: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let n = line.len().min(4);
    let tag = line[..n].to_vec();
    let mut i = n;
    while i < line.len() && (line[i] == b':' || line[i] == b' ') {
        i += 1;
    }
    (tag, line[i..].to_vec())
}

/// '@' followed by anything but '@' becomes '\t';
/// "@@" is skipped over UNCHANGED (both bytes remain).
pub fn parse_at(s: &mut [u8]) {
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'@' {
            if s.get(i + 1) != Some(&b'@') {
                s[i] = b'\t';
            } else {
                i += 1;
            }
        }
        i += 1;
    }
}

/// Letters accumulate bits (a-z -> 0-25, A-Z ->
/// 26-51 via `1 << n` on int, which x86-64 masks to n & 31 — so 'F'
/// is bit 31 and letters beyond wrap); an all-numeric token (optional
/// leading '-') REPLACES everything via atol.
pub fn asciiflag_conv(token: &[u8]) -> u32 {
    let mut flags: u32 = 0;
    let mut is_num = !token.is_empty();
    for (i, &c) in token.iter().enumerate() {
        if c.is_ascii_lowercase() {
            flags |= 1u32 << ((c - b'a') & 31);
        } else if c.is_ascii_uppercase() {
            flags |= 1u32 << ((26 + (c - b'A')) & 31);
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

/// `atol` semantics (enough of them): optional sign, digits, stop at the
/// first non-digit, overflow wrapping to 32 bits.
pub fn atol(s: &[u8]) -> i64 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        i += 1;
    }
    if neg { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_line_skips_comments_and_blanks() {
        let data = b"* comment\n\n\r\nreal line\r\nnext\n";
        let mut r = Reader::new(data);
        assert_eq!(r.get_line().unwrap(), b"real line");
        assert_eq!(r.get_line().unwrap(), b"next");
        assert!(r.get_line().is_none());
    }

    #[test]
    fn get_line_truncates_at_bufsize_and_drops_the_tail() {
        // An over-long line is cut to bufsize-1 and its remainder is
        // discarded, so the NEXT call returns the next real line rather than
        // a fragment of this one.
        let mut data = vec![b'x'; 300];
        data.push(b'\n');
        data.extend_from_slice(b"next\n");
        let mut r = Reader::new(&data);
        assert_eq!(r.get_line().unwrap().len(), 255);
        assert_eq!(r.get_line().unwrap(), b"next");
        assert!(r.get_line().is_none());
    }

    #[test]
    fn get_line_sized_honours_the_callers_buffer() {
        // Alias replacements are read with a MAX_INPUT_LENGTH + 1 buffer,
        // so a 300-byte replacement comes back whole.
        let mut data = vec![b'x'; 300];
        data.push(b'\n');
        data.extend_from_slice(b"next\n");
        let mut r = Reader::new(&data);
        assert_eq!(r.get_line_sized(513).unwrap().len(), 300);
        assert_eq!(r.get_line_sized(513).unwrap(), b"next");
    }

    #[test]
    fn fread_string_basic_and_tilde_rules() {
        let mut r = Reader::new(b"line one\nline two~\n#next\n");
        let s = r.fread_string("test").unwrap().unwrap();
        assert_eq!(s, b"line one\r\nline two");
        // Mid-line tildes are literal; only line-final terminates.
        let mut r = Reader::new(b"a~b\nend~\n");
        assert_eq!(r.fread_string("t").unwrap().unwrap(), b"a~b\r\nend");
        // Exactly one trailing tilde stripped.
        let mut r = Reader::new(b"text~~\n");
        assert_eq!(r.fread_string("t").unwrap().unwrap(), b"text~");
        // Empty string => None.
        let mut r = Reader::new(b"~\n");
        assert_eq!(r.fread_string("t").unwrap(), None);
    }

    #[test]
    fn fread_string_crlf_normalization() {
        let mut r = Reader::new(b"one\r\ntwo\r\nend~\r\n");
        assert_eq!(r.fread_string("t").unwrap().unwrap(), b"one\r\ntwo\r\nend");
    }

    #[test]
    fn parse_at_rules() {
        let mut s = b"a@rb".to_vec();
        parse_at(&mut s);
        assert_eq!(s, b"a\trb");
        let mut s = b"a@@b".to_vec();
        parse_at(&mut s);
        assert_eq!(s, b"a@@b");
        let mut s = b"end@".to_vec();
        parse_at(&mut s);
        assert_eq!(s, b"end\t");
    }

    #[test]
    fn asciiflag_letters_numbers_and_replacement() {
        assert_eq!(asciiflag_conv(b"abc"), 0b111);
        assert_eq!(asciiflag_conv(b"A"), 1 << 26);
        assert_eq!(asciiflag_conv(b"F"), 1 << 31);
        assert_eq!(asciiflag_conv(b"128"), 128);
        assert_eq!(asciiflag_conv(b"-1"), u32::MAX);
        // Mixed letters+digits: digits break is_num, letters still OR'd.
        assert_eq!(asciiflag_conv(b"a1"), 1);
        assert_eq!(asciiflag_conv(b"0"), 0);
    }
}
