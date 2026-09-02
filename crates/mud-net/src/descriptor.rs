//! Descriptor lifecycle and I/O buffering: output buffering with the exact overflow ladder, input reading and
//! line assembly (history, `^^` substitution, `--` flush), the pager, and
//! flush assembly with prompt/compact semantics.
//!
//! The game layer owns a `Descriptors` table inside its `Game` struct and
//! drives everything per pulse; nothing here knows about world state beyond
//! the opaque `CharId`.

use std::collections::VecDeque;
use std::io::{Read, Write};

use mud_data::ids::CharId;
use mud_data::types::{
    ConState, HOST_LENGTH, LARGE_BUFSIZE, MAX_INPUT_LENGTH, MAX_RAW_INPUT_LENGTH,
    MAX_STRING_LENGTH, SMALL_BUFSIZE,
};

use crate::editor::EditBuf;
use crate::protocol::{self, ProtocolState};

pub const HISTORY_SIZE: usize = 5;

/// Output-buffer counters (`buf_largecount`, `buf_overflows`, `buf_switches`,
/// and the `bufpool` free list). `show stats` prints all three, so they are
/// observable state rather than diagnostics.
#[derive(Debug, Default, Clone, Copy)]
pub struct BufStats {
    /// Large buffers ever allocated — the pool never shrinks.
    pub largecount: i32,
    /// Writes truncated because even a large buffer could not hold them.
    pub overflows: i32,
    /// Small-to-large buffer switches.
    pub switches: i32,
    /// Large buffers currently sitting in the pool, free for reuse.
    pool_free: i32,
}

/// A line editor session (d->str / d->backstr / d->mail_to).
#[derive(Debug, Default)]
pub struct EditSession {
    pub buf: EditBuf,
    /// Original text restored on abort.
    pub backstr: Option<Vec<u8>>,
    /// A mail recipient's idnum, or BOARD_MAGIC + board number for a board
    /// post.
    pub mail_to: i64,
    /// Which board slot this session writes into, -1 when it is not a
    /// board.
    pub str_slot: i32,
    /// The object whose action_description this session writes (do_write's
    /// note); the field is named rather than pointed at.
    pub note_obj: Option<mud_data::ids::ObjId>,
}

pub struct Descriptor {
    pub stream: Option<mio::net::TcpStream>,
    pub host: Vec<u8>,
    pub bad_pws: u8,
    pub idle_tics: u8,
    pub state: ConState,
    pub desc_num: u32,
    pub login_time: i64,

    // Pager: the paged text is owned here, with page-start offsets
    // alongside it (study 01 §12.3).
    pub showstr_buf: Vec<u8>,
    pub showstr_offsets: Vec<usize>,
    pub showstr_count: i32,
    pub showstr_page: i32,

    pub editing: Option<EditSession>,

    pub has_prompt: bool,
    pub inbuf: Vec<u8>,
    pub last_input: Vec<u8>,
    pub history: [Vec<u8>; HISTORY_SIZE],
    pub history_pos: usize,

    /// Buffered output (post-translation bytes). Cap LARGE_BUFSIZE-1.
    pub output: Vec<u8>,
    /// Overflow state: writes are dropped and the flush appends **OVERFLOW**.
    pub overflowed: bool,
    /// Bytes still free in the *current* buffer, small or large. This is
    /// the value `send_to_char` returns, and several listings
    /// (do_stat_room's "Chars present", do_stat_character's followers) add it
    /// to a column counter — so it is observable, not bookkeeping.
    pub bufspace: usize,
    /// Whether this descriptor has switched to a large output buffer.
    pub large_outbuf: bool,
    /// Local echo is suppressed — the client is typing a password. **F6**:
    /// such a line never reaches the snooper or the command history.
    pub echo_suppressed: bool,

    pub input: VecDeque<(Vec<u8>, bool)>,
    /// Lines this descriptor typed that a snooper should be shown, drained by
    /// the table wrapper after each read.
    pub snoop_input: Vec<Vec<u8>>,
    /// The snoop copy of the last flush, drained by the table wrapper.
    pub snoop_output: Option<Vec<u8>>,
    pub character: Option<CharId>,
    pub original: Option<CharId>,
    pub snooping: Option<usize>,
    pub snoop_by: Option<usize>,

    /// The zone a `zdelete` report named and is waiting to be confirmed for.
    /// It lives here so it dies with the connection, and so that the command
    /// that deletes carries no zone number of its own.
    pub zdelete_armed: Option<i32>,

    pub protocol: ProtocolState,
    /// Pulse at which the get_protocols event fires (ePROTOCOLS, +1.5 s).
    pub protocol_event_at: Option<u64>,
}

impl Descriptor {
    /// init_descriptor.
    pub fn new(stream: Option<mio::net::TcpStream>, host: &[u8], desc_num: u32, login_time: i64, negotiate: bool) -> Self {
        let mut host = host.to_vec();
        host.truncate(HOST_LENGTH);
        Self {
            stream,
            host,
            bad_pws: 0,
            idle_tics: 0,
            state: if negotiate { ConState::GetProtocol } else { ConState::GetName },
            desc_num,
            login_time,
            showstr_buf: Vec::new(),
            showstr_offsets: Vec::new(),
            showstr_count: 0,
            showstr_page: 0,
            editing: None,
            has_prompt: true, // "prompt is part of greetings"
            inbuf: Vec::new(),
            last_input: Vec::new(),
            history: Default::default(),
            history_pos: 0,
            output: Vec::new(),
            overflowed: false,
            bufspace: SMALL_BUFSIZE - 1,
            large_outbuf: false,
            echo_suppressed: false,
            input: VecDeque::new(),
            snoop_input: Vec::new(),
            snoop_output: None,
            character: None,
            original: None,
            snooping: None,
            snoop_by: None,
            zdelete_armed: None,
            protocol: ProtocolState::new(),
            protocol_event_at: None,
        }
    }

    /// In play: the playing state, or any OLC editor state.
    pub fn is_playing(&self) -> bool {
        matches!(self.state, ConState::Playing) || self.in_olc()
    }

    pub fn in_olc(&self) -> bool {
        (self.state as u8) >= ConState::Oedit as u8 && (self.state as u8) <= ConState::Msgedit as u8
    }

    /// write_to_output → vwrite_to_output. `color_allowed`
    /// is the caller's clr(ch, C_CMP) gate (true when no character).
    /// Returns the bug-log lines for mudlog plus the buffer space left
    /// afterwards (0 in the overflow state).
    pub fn write_to_output(
        &mut self,
        txt: &[u8],
        color_allowed: bool,
        stats: &mut BufStats,
    ) -> (Vec<String>, usize) {
        let mut bugs = Vec::new();
        // Already in overflow state: drop new text.
        if self.bufspace == 0 {
            return (bugs, 0);
        }
        let translated = protocol::protocol_output(&mut self.protocol, txt, color_allowed, &mut bugs);
        if self.protocol.write_oob > 0 {
            self.protocol.write_oob -= 1;
        }
        let Some(bytes) = translated else {
            return (bugs, self.bufspace); // discarded whole
        };
        (bugs, self.append_output(&bytes, stats))
    }

    /// The buffer half of vwrite_to_output, shared with
    /// the protocol pump. Returns the new bufspace.
    fn append_output(&mut self, bytes: &[u8], stats: &mut BufStats) -> usize {
        let mut size = bytes.len();
        let mut bytes = bytes;
        // Too big even for a large buffer: truncate into the overflow state.
        if size + self.output.len() + 1 > LARGE_BUFSIZE {
            size = (LARGE_BUFSIZE - self.output.len()).saturating_sub(1);
            bytes = &bytes[..size];
            stats.overflows += 1;
        }
        if self.bufspace > size {
            self.output.extend_from_slice(bytes);
            self.bufspace -= size;
            self.overflowed = self.bufspace == 0;
            return self.bufspace;
        }
        // Switch to a large buffer, taking one from the pool if there is one.
        stats.switches += 1;
        if !self.large_outbuf {
            self.large_outbuf = true;
            if stats.pool_free > 0 {
                stats.pool_free -= 1;
            } else {
                stats.largecount += 1;
            }
        }
        self.output.extend_from_slice(bytes);
        self.bufspace = (LARGE_BUFSIZE - 1).saturating_sub(self.output.len());
        self.overflowed = self.bufspace == 0;
        self.bufspace
    }

    /// Release the large buffer back to the pool and reset to a small one
    fn reset_buffer(&mut self, stats: &mut BufStats) {
        if self.large_outbuf {
            self.large_outbuf = false;
            stats.pool_free += 1;
        }
        self.bufspace = (SMALL_BUFSIZE - 1).saturating_sub(self.output.len());
    }

    /// Drain protocol-layer negotiation bytes into the output buffer,
    /// bypassing translation (they are raw telnet), which is where protocol
    /// data flows through write_to_output but contains no `\t` codes.
    pub fn pump_protocol_out(&mut self, stats: &mut BufStats) {
        if self.protocol.out.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.protocol.out);
        if self.bufspace == 0 {
            return;
        }
        self.append_output(&bytes, stats);
        if self.protocol.write_oob > 0 {
            self.protocol.write_oob -= 1;
        }
    }

    /// write_to_descriptor: direct, unbuffered,
    /// untranslated. Returns Err on hard socket error.
    pub fn write_direct(&mut self, data: &[u8]) -> Result<usize, ()> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(data.len()); // testing without a socket
        };
        let mut total = 0;
        while total < data.len() {
            match stream.write(&data[total..]) {
                Ok(0) => return Err(()), // "write() returned 0???"
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(total),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(()),
            }
        }
        Ok(total)
    }

    /// echo_off/echo_on — ride the normal output buffer.
    pub fn echo_off(&mut self, stats: &mut BufStats) {
        self.echo_suppressed = true;
        let _ = self.write_to_output(crate::telnet::ECHO_OFF, true, stats);
    }

    pub fn echo_on(&mut self, stats: &mut BufStats) {
        self.echo_suppressed = false;
        let _ = self.write_to_output(crate::telnet::ECHO_ON, true, stats);
    }

    /// process_output. Assembles the flush payload and
    /// writes it. `compact` = PRF_COMPACT; `add_crlf_and_prompt` = playing
    /// non-NPC descriptor; `prompt` = make_prompt(d) result.
    /// Returns Err() when the socket died.
    pub fn process_output(
        &mut self,
        compact: bool,
        playing_pc: bool,
        prompt: &[u8],
        stats: &mut BufStats,
    ) -> Result<(), ()> {
        let oob = self.protocol.write_oob != 0;
        let mut payload: Vec<u8> = Vec::with_capacity(self.output.len() + prompt.len() + 32);
        payload.extend_from_slice(b"\r\n");
        payload.extend_from_slice(&self.output);
        if self.overflowed {
            payload.extend_from_slice(b"**OVERFLOW**\r\n");
        }
        if playing_pc && !compact && !oob {
            payload.extend_from_slice(b"\r\n");
        }
        if !oob {
            payload.extend_from_slice(prompt);
        }

        let send_from = if self.has_prompt && !oob {
            self.has_prompt = false;
            0 // include the leading \r\n (interrupting an existing prompt)
        } else {
            2
        };

        let written = self.write_direct(&payload[send_from..])?;
        // Snoop copy: "% " + the buffer + "%%". The text is padded to
        // `result` columns with %*s, but `result` never exceeds the buffer
        // length in practice, so the padding never fires.
        if written > 0 {
            let mut copy = b"% ".to_vec();
            copy.extend_from_slice(&self.output);
            copy.extend_from_slice(b"%%");
            self.snoop_output = Some(copy);
        }
        let content_len = self.output.len();
        // Bytes of `output` content actually sent (leading \r\n excluded).
        let content_sent = written.saturating_sub(if send_from == 0 { 2 } else { 0 }).min(content_len);
        if content_sent >= content_len {
            // Full content flush; any unsent prompt/overflow tail is re-saved
            // as fresh content.
            let sent_total = send_from + written;
            let tail = if sent_total < payload.len() { payload[sent_total..].to_vec() } else { Vec::new() };
            self.output = tail;
            self.overflowed = false;
            self.reset_buffer(stats);
        } else {
            // Partial content write: shift the remainder down.
            self.output.drain(..content_sent);
            self.bufspace += content_sent;
        }
        Ok(())
    }

    /// process_input: drain the socket, strip protocol,
    /// split lines. Returns Err() to close (EOF/overflow/error), Ok(bugs).
    /// The error carries its own log lines: a connection that closes has
    /// something worth saying about why, and the host it came from is the
    /// only identifying thing a scanner that never sent a name leaves behind.
    pub fn process_input(&mut self, stats: &mut BufStats) -> Result<Vec<String>, Vec<String>> {
        let mut bugs = Vec::new();
        self.snoop_input.clear();
        // Read phase.
        loop {
            if self.inbuf.len() >= MAX_RAW_INPUT_LENGTH - 1 {
                bugs.push(format!(
                    "WARNING: process_input: about to close connection: input overflow [{}]",
                    String::from_utf8_lossy(&self.host)
                ));
                return Err(bugs);
            }
            let Some(stream) = self.stream.as_mut() else { break };
            let mut buf = [0u8; MAX_RAW_INPUT_LENGTH];
            match stream.read(&mut buf) {
                Ok(0) => {
                    bugs.push(format!(
                        "WARNING: EOF on socket read (connection broken by peer) [{}]",
                        String::from_utf8_lossy(&self.host)
                    ));
                    return Err(bugs);
                }
                Ok(n) => {
                    self.protocol.write_oob = 0;
                    let empty = self.output.is_empty();
                    let r = protocol::protocol_input(&mut self.protocol, &buf[..n], empty);
                    bugs.extend(r.bugs);
                    if r.fatal {
                        return Err(bugs);
                    }
                    self.inbuf.extend_from_slice(&r.in_band);
                    // Everything ProtocolInput generates is staged in
                    // protocol.out, and it has to be drained here: the only
                    // other drain runs for playing descriptors, which
                    // stranded the whole negotiate_full burst, IAC WILL MSDP
                    // included, until the player entered the game.
                    self.pump_protocol_out(stats);
                    // Keep reading until would-block.
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Err(bugs),
                Err(_) => return Err(bugs),
            }
        }
        if self.split_lines(&mut bugs, stats).is_err() {
            return Err(bugs);
        }
        Ok(bugs)
    }

    /// Feed bytes directly with a throwaway BufStats (tests only).
    pub fn feed_input_test(&mut self, data: &[u8]) -> Result<Vec<String>, ()> {
        let mut stats = BufStats::default();
        self.feed_input(data, &mut stats)
    }

    /// Feed bytes directly, without a socket.
    pub fn feed_input(&mut self, data: &[u8], stats: &mut BufStats) -> Result<Vec<String>, ()> {
        let mut bugs = Vec::new();
        let empty = self.output.is_empty();
        let r = protocol::protocol_input(&mut self.protocol, data, empty);
        bugs.extend(r.bugs);
        if r.fatal {
            return Err(());
        }
        self.inbuf.extend_from_slice(&r.in_band);
        // Same drain as process_input.
        self.pump_protocol_out(stats);
        self.split_lines(&mut bugs, stats)?;
        Ok(bugs)
    }

    fn split_lines(&mut self, _bugs: &mut [String], stats: &mut BufStats) -> Result<(), ()> {
        // Line assembly.
        loop {
            let Some(nl_pos) = self.inbuf.iter().position(|c| *c == b'\r' || *c == b'\n') else {
                break;
            };
            let mut tmp: Vec<u8> = Vec::with_capacity(MAX_INPUT_LENGTH);
            let mut space_left = (MAX_INPUT_LENGTH - 1) as i32;
            let mut truncated_at: Option<usize> = None;
            let mut idx = 0usize;
            while idx < nl_pos {
                if space_left <= 1 {
                    truncated_at = Some(idx);
                    break;
                }
                let c = self.inbuf[idx];
                if c == 0x08 || c == 0x7F {
                    // Backspace/delete undoing doubled '$'.
                    if let Some(last) = tmp.pop() {
                        if last == b'$' {
                            tmp.pop();
                            space_left += 2;
                        } else {
                            space_left += 1;
                        }
                    }
                } else if c.is_ascii() && (0x20..0x7F).contains(&c) {
                    tmp.push(c);
                    if c == b'$' {
                        tmp.push(b'$');
                        space_left -= 2;
                    } else {
                        space_left -= 1;
                    }
                }
                idx += 1;
            }
            // Notify whenever input remained.
            if truncated_at.is_some() {
                let mut msg = b"Line too long.  Truncated to:\r\n".to_vec();
                msg.extend_from_slice(&tmp);
                msg.extend_from_slice(b"\r\n");
                if self.write_direct(&msg).is_err() {
                    return Err(());
                }
            }

            // "% <line>" to the snooper. F6: never while the
            // client's echo is off, so passwords stay off the snoop stream
            // and out of the history below.
            if !self.echo_suppressed {
                let mut line = b"% ".to_vec();
                line.extend_from_slice(&tmp);
                line.extend_from_slice(b"\r\n");
                self.snoop_input.push(line);
            }

            let mut failed_subst = false;
            if tmp.first() == Some(&b'!') && tmp.len() == 1 {
                tmp = self.last_input.clone();
            } else if tmp.first() == Some(&b'!') {
                // History recall by abbreviation.
                let mut cmdline: &[u8] = &tmp[1..];
                while cmdline.first().is_some_and(|c| c.is_ascii_whitespace() && *c != b'\t') {
                    cmdline = &cmdline[1..];
                }
                let starting_pos = self.history_pos;
                let mut cnt = if self.history_pos == 0 { HISTORY_SIZE - 1 } else { self.history_pos - 1 };
                while cnt != starting_pos {
                    if !self.history[cnt].is_empty() && is_abbrev(cmdline, &self.history[cnt]) {
                        tmp = self.history[cnt].clone();
                        self.last_input = tmp.clone();
                        let mut echo = tmp.clone();
                        echo.extend_from_slice(b"\r\n");
                        let _ = self.write_to_output(&echo, true, stats);
                        break;
                    }
                    if cnt == 0 {
                        cnt = HISTORY_SIZE;
                    }
                    cnt -= 1;
                }
            } else if tmp.first() == Some(&b'^') {
                match perform_subst(&self.last_input, &tmp) {
                    Ok(newline) => {
                        tmp = newline;
                        self.last_input = tmp.clone();
                    }
                    Err(()) => {
                        let _ = self.write_to_output(b"Invalid substitution.\r\n", true, stats);
                        failed_subst = true;
                    }
                }
            } else if self.echo_suppressed {
                // An echo-off line is not remembered at all.
            } else {
                self.last_input = tmp.clone();
                self.history[self.history_pos] = tmp.clone();
                self.history_pos = (self.history_pos + 1) % HISTORY_SIZE;
            }

            if tmp == b"--" {
                let _ = self.write_to_output(b"All queued commands cancelled.\r\n", true, stats);
                self.flush_queues();
                failed_subst = true;
            }

            if !failed_subst {
                self.input.push_back((tmp, false));
            }

            // Consume the newline run; the unread tail of an over-long line is
            // discarded (the read point jumps past the newline).
            let mut after = nl_pos;
            while after < self.inbuf.len() && (self.inbuf[after] == b'\r' || self.inbuf[after] == b'\n') {
                after += 1;
            }
            self.inbuf.drain(..after);
        }
        Ok(())
    }

    /// flush_queues (flavor): clear pending input.
    pub fn flush_queues(&mut self) {
        self.input.clear();
    }

    // ---- Pager ----

    /// The paged text is always copied into the descriptor.
    pub fn page_string(&mut self, text: &[u8], page_length: i32, screen_width: i32, compact: bool, color_allowed: bool, stats: &mut BufStats) {
        if text.is_empty() {
            return;
        }
        self.showstr_buf = text.to_vec();
        self.showstr_offsets = paginate(&self.showstr_buf, page_length, screen_width, compact);
        self.showstr_count = self.showstr_offsets.len() as i32;
        self.showstr_page = 0;
        self.show_string(b"", page_length, screen_width, compact, color_allowed, stats);
    }

    pub fn show_string(
        &mut self,
        input: &[u8],
        _page_length: i32,
        _screen_width: i32,
        _compact: bool,
        color_allowed: bool,
        stats: &mut BufStats,
    ) {
        let mut word: Vec<u8> = Vec::new();
        let mut it = input.iter().copied().skip_while(|c| c.is_ascii_whitespace() && *c != b'\t');
        for c in it.by_ref() {
            if c.is_ascii_whitespace() {
                break;
            }
            word.push(c.to_ascii_lowercase());
        }

        match word.first() {
            Some(b'q') => {
                self.pager_clear();
                return;
            }
            Some(b'r') => self.showstr_page = (self.showstr_page - 1).max(0),
            Some(b'b') => self.showstr_page = (self.showstr_page - 2).max(0),
            Some(c) if c.is_ascii_digit() => {
                let n = atoi_bytes(&word);
                self.showstr_page = (n - 1).clamp(0, self.showstr_count - 1);
            }
            Some(_) => {
                let _ = self.write_to_output(
                    b"Valid commands while paging are RETURN, Q, R, B, or a numeric value.\r\n",
                    color_allowed,
                    stats,
                );
                return;
            }
            None => {}
        }

        if self.showstr_page + 1 >= self.showstr_count {
            let start = self.showstr_offsets[self.showstr_page as usize];
            let mut out = self.showstr_buf[start..].to_vec();
            out.extend_from_slice(b"\tn");
            let _ = self.write_to_output(&out, color_allowed, stats);
            self.pager_clear();
        } else {
            let start = self.showstr_offsets[self.showstr_page as usize];
            let end = self.showstr_offsets[self.showstr_page as usize + 1];
            let mut diff = end - start;
            if diff > MAX_STRING_LENGTH - 3 {
                diff = MAX_STRING_LENGTH - 3;
            }
            let mut buffer = self.showstr_buf[start..start + diff].to_vec();
            // Normalize the tail to exactly one \r\n.
            let n = buffer.len();
            if n >= 2 && buffer[n - 2] == b'\r' && buffer[n - 1] == b'\n' {
                // Fine as is.
            } else if n >= 2 && buffer[n - 2] == b'\n' && buffer[n - 1] == b'\r' {
                buffer.truncate(n - 2);
                buffer.extend_from_slice(b"\r\n");
            } else if n >= 1 && (buffer[n - 1] == b'\r' || buffer[n - 1] == b'\n') {
                buffer.truncate(n - 1);
                buffer.extend_from_slice(b"\r\n");
            } else {
                buffer.extend_from_slice(b"\r\n");
            }
            let _ = self.write_to_output(&buffer, color_allowed, stats);
            self.showstr_page += 1;
        }
    }

    pub fn paging(&self) -> bool {
        self.showstr_count != 0
    }

    fn pager_clear(&mut self) {
        self.showstr_buf.clear();
        self.showstr_offsets.clear();
        self.showstr_count = 0;
        self.showstr_page = 0;
    }
}

/// next_page/count_pages/paginate_string folded into offset computation.
/// Returns the byte offset of each page start.
fn paginate(text: &[u8], page_length: i32, screen_width: i32, compact: bool) -> Vec<usize> {
    let mut offsets = vec![0usize];
    let pw = if (40..=250).contains(&screen_width) { screen_width } else { 80 };
    let page_len = page_length - if compact { 1 } else { 2 };
    let mut col = 1i32;
    let mut line = 1i32;
    let mut i = 0usize;
    while i < text.len() {
        if line > page_len {
            offsets.push(i);
            col = 1;
            line = 1;
            // Scanning for the next page restarts at this char.
            continue;
        }
        let c = text[i];
        if c == 0x1B {
            // Skip to 'm' or max 9 chars.
            let mut count = 0;
            while i < text.len() && text[i] != b'm' && count < 9 {
                i += 1;
                count += 1;
            }
            i += 1; // for(;;str++) advances past the 'm' (or stop char)
            continue;
        }
        if c == b'\t' {
            if text.get(i + 1) != Some(&b'\t') {
                i += 1; // skip the code char too
            }
            i += 1;
            continue;
        }
        if c == b'\r' {
            col = 1;
        } else if c == b'\n' {
            line += 1;
        } else {
            col += 1;
            if col - 1 > pw {
                col = 1;
                line += 1;
            }
        }
        i += 1;
    }
    offsets
}

/// is_abbrev: arg1 non-empty, case-insensitive prefix.
pub fn is_abbrev(arg1: &[u8], arg2: &[u8]) -> bool {
    if arg1.is_empty() {
        return false;
    }
    if arg1.len() > arg2.len() {
        return false;
    }
    arg1.eq_ignore_ascii_case(&arg2[..arg1.len()])
}

fn atoi_bytes(b: &[u8]) -> i32 {
    let mut n: i64 = 0;
    let mut it = b.iter();
    let mut neg = false;
    let mut first = true;
    for &c in it.by_ref() {
        if first && (c == b'-' || c == b'+') {
            neg = c == b'-';
            first = false;
            continue;
        }
        first = false;
        if c.is_ascii_digit() {
            n = n * 10 + (c - b'0') as i64;
            if n > i32::MAX as i64 {
                n = i32::MAX as i64;
            }
        } else {
            break;
        }
    }
    (if neg { -n } else { n }) as i32
}

/// perform_subst: ^old^new over last_input.
fn perform_subst(orig: &[u8], subst: &[u8]) -> Result<Vec<u8>, ()> {
    let body = &subst[1..];
    let Some(second_caret) = body.iter().position(|c| *c == b'^') else {
        return Err(());
    };
    let first = &body[..second_caret];
    let second = &body[second_caret + 1..];
    let Some(strpos) = find_sub(orig, first) else {
        return Err(());
    };
    let mut newsub = Vec::with_capacity(MAX_INPUT_LENGTH + 5);
    newsub.extend_from_slice(&orig[..strpos]);
    newsub.extend_from_slice(second);
    if strpos + first.len() < orig.len() {
        newsub.extend_from_slice(&orig[strpos + first.len()..]);
    }
    newsub.truncate(MAX_INPUT_LENGTH - 1);
    Ok(newsub)
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The descriptor table: slab storage plus newest-first iteration order
/// — new descriptors are prepended, and iteration order is observable.
#[derive(Default)]
pub struct Descriptors {
    slots: Vec<Option<Descriptor>>,
    /// Live indices, newest first.
    pub order: Vec<usize>,
    last_desc: u32,
    /// Output-buffer counters, shared by every descriptor.
    pub bufstats: BufStats,
}

impl Descriptors {
    /// Split borrow so a descriptor and the shared buffer stats can be held
    /// mutably at once.
    pub fn get_mut_with_stats(&mut self, idx: usize) -> Option<(&mut Descriptor, &mut BufStats)> {
        let d = self.slots.get_mut(idx)?.as_mut()?;
        Some((d, &mut self.bufstats))
    }

    /// Returns the mudlog lines and the remaining buffer space.
    pub fn write_to_output(
        &mut self,
        idx: usize,
        txt: &[u8],
        color_allowed: bool,
    ) -> (Vec<String>, usize) {
        match self.get_mut_with_stats(idx) {
            Some((d, stats)) => d.write_to_output(txt, color_allowed, stats),
            None => (Vec::new(), 0),
        }
    }

    pub fn pump_protocol_out(&mut self, idx: usize) {
        if let Some((d, stats)) = self.get_mut_with_stats(idx) {
            d.pump_protocol_out(stats);
        }
    }

    pub fn process_output(
        &mut self,
        idx: usize,
        compact: bool,
        playing_pc: bool,
        prompt: &[u8],
    ) -> Result<(), ()> {
        let r = match self.get_mut_with_stats(idx) {
            Some((d, stats)) => d.process_output(compact, playing_pc, prompt, stats),
            None => return Ok(()),
        };
        self.flush_snoop(idx);
        r
    }

    pub fn process_input(&mut self, idx: usize) -> Result<Vec<String>, Vec<String>> {
        let r = match self.get_mut_with_stats(idx) {
            Some((d, stats)) => d.process_input(stats),
            None => return Ok(Vec::new()),
        };
        self.flush_snoop(idx);
        r
    }

    /// Ship whatever `idx` produced to whoever is snooping it.
    fn flush_snoop(&mut self, idx: usize) {
        let Some(d) = self.slots.get_mut(idx).and_then(|s| s.as_mut()) else { return };
        let by = d.snoop_by;
        let lines = std::mem::take(&mut d.snoop_input);
        let out = d.snoop_output.take();
        let Some(by) = by else { return };
        for l in lines {
            self.write_to_output(by, &l, true);
        }
        if let Some(o) = out {
            self.write_to_output(by, &o, true);
        }
    }

    pub fn echo_off(&mut self, idx: usize) {
        if let Some((d, stats)) = self.get_mut_with_stats(idx) {
            d.echo_off(stats);
        }
    }

    pub fn echo_on(&mut self, idx: usize) {
        if let Some((d, stats)) = self.get_mut_with_stats(idx) {
            d.echo_on(stats);
        }
    }

    pub fn show_string(
        &mut self,
        idx: usize,
        input: &[u8],
        page_length: i32,
        screen_width: i32,
        compact: bool,
        color_allowed: bool,
    ) {
        if let Some((d, stats)) = self.get_mut_with_stats(idx) {
            d.show_string(input, page_length, screen_width, compact, color_allowed, stats);
        }
    }

    pub fn page_string(
        &mut self,
        idx: usize,
        text: &[u8],
        page_length: i32,
        screen_width: i32,
        compact: bool,
        color_allowed: bool,
    ) {
        if let Some((d, stats)) = self.get_mut_with_stats(idx) {
            d.page_string(text, page_length, screen_width, compact, color_allowed, stats);
        }
    }
}

impl Descriptors {
    pub fn insert(&mut self, mut d: Descriptor) -> usize {
        self.last_desc += 1;
        if self.last_desc == 1000 {
            self.last_desc = 1;
        }
        d.desc_num = self.last_desc;
        let idx = match self.slots.iter().position(|s| s.is_none()) {
            Some(i) => {
                self.slots[i] = Some(d);
                i
            }
            None => {
                self.slots.push(Some(d));
                self.slots.len() - 1
            }
        };
        self.order.insert(0, idx);
        idx
    }

    pub fn remove(&mut self, idx: usize) -> Option<Descriptor> {
        self.order.retain(|i| *i != idx);
        self.slots.get_mut(idx).and_then(|s| s.take())
    }

    pub fn get(&self, idx: usize) -> Option<&Descriptor> {
        self.slots.get(idx).and_then(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Descriptor> {
        self.slots.get_mut(idx).and_then(|s| s.as_mut())
    }

    /// Snapshot of live indices in list order (newest first) for iteration
    /// while mutating.
    pub fn indices(&self) -> Vec<usize> {
        self.order.clone()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> Descriptor {
        Descriptor::new(None, b"localhost", 1, 0, false)
    }

    #[test]
    fn line_splitting_queues_and_doubles_dollars() {
        let mut d = desc();
        d.feed_input_test(b"say hi $5\r\nlook\r\n").unwrap();
        assert_eq!(d.input.pop_front().unwrap().0, b"say hi $$5");
        assert_eq!(d.input.pop_front().unwrap().0, b"look");
    }

    #[test]
    fn crlf_run_is_one_blank_command() {
        let mut d = desc();
        d.feed_input_test(b"\r\n\r\n").unwrap();
        assert_eq!(d.input.len(), 1);
        assert_eq!(d.input.pop_front().unwrap().0, b"");
    }

    #[test]
    fn bang_recalls_last_input() {
        let mut d = desc();
        d.feed_input_test(b"north\r\n!\r\n").unwrap();
        assert_eq!(d.input.pop_front().unwrap().0, b"north");
        assert_eq!(d.input.pop_front().unwrap().0, b"north");
    }

    #[test]
    fn history_prefix_recall_echoes() {
        let mut d = desc();
        d.feed_input_test(b"cast armor\r\nlook\r\n!c\r\n").unwrap();
        let lines: Vec<_> = d.input.drain(..).map(|x| x.0).collect();
        assert_eq!(lines, vec![b"cast armor".to_vec(), b"look".to_vec(), b"cast armor".to_vec()]);
        assert!(d.output.windows(12).any(|w| w == b"cast armor\r\n"));
    }

    #[test]
    fn caret_substitution() {
        let mut d = desc();
        d.feed_input_test(b"say hello world\r\n^hello^goodbye\r\n").unwrap();
        let lines: Vec<_> = d.input.drain(..).map(|x| x.0).collect();
        assert_eq!(lines[1], b"say goodbye world".to_vec());
    }

    #[test]
    fn bad_substitution_discards_line() {
        let mut d = desc();
        d.feed_input_test(b"say hi\r\n^nope\r\n").unwrap();
        assert_eq!(d.input.len(), 1);
        assert!(d.output.windows(23).any(|w| w == b"Invalid substitution.\r\n"));
    }

    #[test]
    fn dash_dash_flushes_queue() {
        let mut d = desc();
        d.feed_input_test(b"look\r\nnorth\r\n--\r\n").unwrap();
        assert!(d.input.is_empty());
        assert!(d.output.windows(32).any(|w| w == b"All queued commands cancelled.\r\n"));
    }

    #[test]
    fn backspace_edits_and_undoes_dollar() {
        let mut d = desc();
        d.feed_input_test(b"ab$\x08c\r\n").unwrap();
        assert_eq!(d.input.pop_front().unwrap().0, b"abc");
    }

    #[test]
    fn output_overflow_ladder() {
        let mut d = desc();
        let mut st = BufStats::default();
        let big = vec![b'x'; LARGE_BUFSIZE];
        d.write_to_output(&big[..10000], true, &mut st);
        d.write_to_output(&big[..10000], true, &mut st);
        d.write_to_output(&big[..10000], true, &mut st);
        assert!(d.overflowed);
        assert_eq!(d.output.len(), LARGE_BUFSIZE - 1);
        // Further writes dropped.
        d.write_to_output(b"more", true, &mut st);
        assert_eq!(d.output.len(), LARGE_BUFSIZE - 1);
    }

    #[test]
    fn pager_pages_and_quits() {
        let mut d = desc();
        let mut st = BufStats::default();
        let mut text = Vec::new();
        for i in 0..60 {
            text.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
        d.page_string(&text, 22, 80, false, true, &mut st);
        assert!(d.paging());
        assert!(d.showstr_count >= 3);
        assert!(d.output.starts_with(b"line 0\r\n"));
        d.output.clear();
        d.show_string(b"q", 22, 80, false, true, &mut st);
        assert!(!d.paging());
    }

    /// Answering IAC WILL TTYPE must put the whole negotiation burst where
    /// the client will actually receive it. Staging it in protocol.out
    /// without draining here meant the only drains ran for playing
    /// descriptors, so a client saw IAC WILL MSDP only after entering the
    /// game, and any MSDP it had already asked for went nowhere.
    #[test]
    fn negotiation_reply_reaches_output_during_input() {
        use crate::telnet::*;
        let mut d = desc();
        d.protocol.negotiated = false;
        d.feed_input_test(&[IAC, WILL, TELOPT_TTYPE]).unwrap();
        assert!(d.protocol.out.is_empty(), "negotiation left staged in protocol.out");
        let burst = &[IAC, WILL, TELOPT_MSDP][..];
        assert!(
            d.output.windows(burst.len()).any(|w| w == burst),
            "IAC WILL MSDP did not reach the output buffer: {:?}",
            d.output
        );
    }

    /// The same drain covers what MSDP itself generates: DO MSDP is answered
    /// with SERVER_ID inside the read that carried it.
    #[test]
    fn server_id_reaches_output_during_input() {
        use crate::telnet::*;
        let mut d = desc();
        d.feed_input_test(&[IAC, DO, TELOPT_MSDP]).unwrap();
        assert!(d.protocol.msdp);
        let want = &[IAC, SB, TELOPT_MSDP, MSDP_VAR][..];
        assert!(
            d.output.windows(want.len()).any(|w| w == want),
            "SERVER_ID did not reach the output buffer: {:?}",
            d.output
        );
    }

    #[test]
    fn pager_last_page_appends_color_stop() {
        let mut d = desc();
        let mut st = BufStats::default();
        d.page_string(b"only line\r\n", 22, 80, false, true, &mut st);
        // Single page: sent immediately with \tn (stripped by protocol since
        // \tn renders ESC[0;00m unconditionally).
        assert!(d.output.ends_with(b"\x1B[0;00m"));
        assert!(!d.paging());
    }
}
