//! The Idea / Bug / Typo lists.
//!
//! The three lists share one implementation keyed by subcmd. Their files
//! (`lib/misc/{bugs,ideas,typos}`) hold SMAUG-style keyword records:
//! `Text <line>~`, `Body <block>~`, `Name`, `Notes`, `IdNum`, `Dated`,
//! `Level`, `Room`, `Flags a b c d`, `End`.
//!
//! The `ibtedit` OLC editor is stage 9; `bug edit` reports as much rather
//! than pretending.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::comm::{
    act, cc, send_editor_help, send_to_char, string_write, C_SPR, KBGRN, KBRED, KCYN, KGRN, KNRM,
    KRED, KYEL, TO_ROOM,
};
use crate::game::{Game, MudlogKind};
use crate::handler::is_abbrev;
use crate::interpreter::{
    is_number, one_argument, two_arguments, SCMD_BUG, SCMD_IDEA, SCMD_TYPO,
};

pub const MAX_IBT_LENGTH: usize = 2048;

// flag bits.
pub const IBT_RESOLVED: usize = 0;
pub const IBT_IMPORTANT: usize = 1;
pub const IBT_INPROGRESS: usize = 2;

pub const IBT_TYPES: [&str; 3] = ["Bug", "Idea", "Typo"];

/// BFRED — bright flashing red, used for the `!` marker.
const KBFRED: &[u8] = b"\x1B[1;5;31m";

/// struct ibt_data.
#[derive(Debug, Clone, Default)]
pub struct Ibt {
    pub text: Vec<u8>,
    pub body: Vec<u8>,
    pub name: Vec<u8>,
    pub notes: Vec<u8>,
    pub level: i32,
    pub room: i32,
    pub id_num: i64,
    pub flags: [u32; 4],
    pub dated: i64,
}

impl Ibt {
    pub fn flagged(&self, bit: usize) -> bool {
        self.flags[bit / 32] & (1 << (bit % 32)) != 0
    }
    fn set_flag(&mut self, bit: usize) {
        self.flags[bit / 32] |= 1 << (bit % 32);
    }
}

/// The three lists, in subcmd order (BUG 0, IDEA 1, TYPO 2).
#[derive(Debug, Default)]
pub struct IbtLists {
    pub lists: [Vec<Ibt>; 3],
}

fn ibt_path(g: &Game, mode: i32) -> Option<std::path::PathBuf> {
    let name = match mode {
        SCMD_BUG => "bugs",
        SCMD_IDEA => "ideas",
        SCMD_TYPO => "typos",
        _ => return None,
    };
    Some(g.lib_dir.join("misc").join(name))
}

/// The basename the log lines print.
fn ibt_basename(mode: i32) -> &'static str {
    match mode {
        SCMD_BUG => "bugs",
        SCMD_IDEA => "ideas",
        _ => "typos",
    }
}

fn mode_idx(mode: i32) -> Option<usize> {
    match mode {
        SCMD_BUG => Some(0),
        SCMD_IDEA => Some(1),
        SCMD_TYPO => Some(2),
        _ => None,
    }
}

// ---------------------------------------------------------- the SMAUG reader

/// The fread_* family, over a byte slice: `fread_word`, `fread_line`
/// (trailing `~` stripped), `fread_clean_string` (`~`-terminated block with
/// `\r\n` joins) and `fread_number`.
struct IbtReader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> IbtReader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, p: 0 }
    }
    fn eof(&self) -> bool {
        self.p >= self.d.len()
    }
    fn skip_space(&mut self) {
        while self.p < self.d.len() && self.d[self.p].is_ascii_whitespace() {
            self.p += 1;
        }
    }
    /// fread_word: quote-delimited or whitespace-delimited.
    fn word(&mut self) -> Vec<u8> {
        self.skip_space();
        if self.eof() {
            return Vec::new();
        }
        let quote = matches!(self.d[self.p], b'\'' | b'"');
        let end_ch = if quote {
            let c = self.d[self.p];
            self.p += 1;
            Some(c)
        } else {
            None
        };
        let start = self.p;
        while self.p < self.d.len() {
            let c = self.d[self.p];
            let done = match end_ch {
                Some(q) => c == q,
                None => c.is_ascii_whitespace(),
            };
            if done {
                let out = self.d[start..self.p].to_vec();
                // Whitespace terminators are ungetc'd; quotes are consumed.
                if end_ch.is_some() {
                    self.p += 1;
                }
                return out;
            }
            self.p += 1;
        }
        self.d[start..].to_vec()
    }
    /// fread_line: to end of line, trailing `~` dropped.
    fn line(&mut self) -> Vec<u8> {
        self.skip_space();
        let start = self.p;
        while self.p < self.d.len() && self.d[self.p] != b'\n' && self.d[self.p] != b'\r' {
            self.p += 1;
        }
        let mut out = self.d[start..self.p].to_vec();
        while self.p < self.d.len() && (self.d[self.p] == b'\n' || self.d[self.p] == b'\r') {
            self.p += 1;
        }
        if out.last() == Some(&b'~') {
            out.pop();
        }
        out
    }
    /// fread_number, including its `|` continuation.
    fn number(&mut self) -> i32 {
        self.skip_space();
        let mut sign = false;
        if self.p < self.d.len() && (self.d[self.p] == b'+' || self.d[self.p] == b'-') {
            sign = self.d[self.p] == b'-';
            self.p += 1;
        }
        let mut n: i32 = 0;
        while self.p < self.d.len() && self.d[self.p].is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((self.d[self.p] - b'0') as i32);
            self.p += 1;
        }
        if sign {
            n = -n;
        }
        if self.p < self.d.len() && self.d[self.p] == b'|' {
            self.p += 1;
            n += self.number();
        } else if self.p < self.d.len() && self.d[self.p] == b' ' {
            self.p += 1;
        }
        n
    }
    /// fread_clean_string.
    fn clean_string(&mut self) -> Vec<u8> {
        self.skip_space();
        let mut buf = Vec::new();
        while self.p < self.d.len() {
            let start = self.p;
            while self.p < self.d.len() && self.d[self.p] != b'\n' {
                self.p += 1;
            }
            let mut chunk = self.d[start..self.p].to_vec();
            if self.p < self.d.len() {
                self.p += 1; // the '\n'
            }
            while chunk.last() == Some(&b'\r') {
                chunk.pop();
            }
            if chunk.last() == Some(&b'~') {
                chunk.pop();
                buf.extend_from_slice(&chunk);
                break;
            }
            buf.extend_from_slice(&chunk);
            buf.extend_from_slice(b"\r\n");
        }
        mud_world::lex::parse_at(&mut buf);
        buf
    }
    fn to_eol(&mut self) {
        while self.p < self.d.len() && self.d[self.p] != b'\n' && self.d[self.p] != b'\r' {
            self.p += 1;
        }
        while self.p < self.d.len() && (self.d[self.p] == b'\n' || self.d[self.p] == b'\r') {
            self.p += 1;
        }
    }
    /// fread_flags: up to four space-separated ints.
    fn flags(&mut self) -> [u32; 4] {
        let mut out = [0u32; 4];
        let start = self.p;
        while self.p < self.d.len() && self.d[self.p] != b'\n' && self.d[self.p] != b'\r' {
            self.p += 1;
        }
        let line = &self.d[start..self.p];
        while self.p < self.d.len() && (self.d[self.p] == b'\n' || self.d[self.p] == b'\r') {
            self.p += 1;
        }
        for (i, tok) in line.split(|b| *b == b' ').filter(|t| !t.is_empty()).take(4).enumerate() {
            out[i] = crate::handler::atoi(tok) as u32;
        }
        out
    }
}

/// read_ibt: one record, `End`-terminated.
fn read_ibt(r: &mut IbtReader, log: &mut Vec<String>) -> Option<Ibt> {
    r.skip_space();
    if r.eof() {
        return None;
    }
    let mut ibt = Ibt { id_num: NOBODY as i64, ..Default::default() };
    let mut id_num: Option<Vec<u8>> = None;
    let mut dated: Option<Vec<u8>> = None;

    loop {
        if r.eof() {
            return None;
        }
        let word = r.word();
        let mut matched = true;
        match word.first().map(|c| c.to_ascii_uppercase()) {
            Some(b'B') if word.eq_ignore_ascii_case(b"Body") => ibt.body = r.clean_string(),
            Some(b'D') if word.eq_ignore_ascii_case(b"Dated") => dated = Some(r.line()),
            Some(b'E') if word.eq_ignore_ascii_case(b"End") => {
                if let Some(v) = id_num {
                    ibt.id_num = crate::handler::atoi(&v) as i64;
                }
                if let Some(v) = dated {
                    ibt.dated = std::str::from_utf8(&v)
                        .ok()
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                }
                return Some(ibt);
            }
            Some(b'F') if word.eq_ignore_ascii_case(b"Flags") => ibt.flags = r.flags(),
            Some(b'I') if word.eq_ignore_ascii_case(b"IdNum") => id_num = Some(r.line()),
            Some(b'L') if word.eq_ignore_ascii_case(b"Level") => ibt.level = r.number(),
            Some(b'N') if word.eq_ignore_ascii_case(b"Name") => ibt.name = r.line(),
            Some(b'N') if word.eq_ignore_ascii_case(b"Notes") => ibt.notes = r.clean_string(),
            Some(b'R') if word.eq_ignore_ascii_case(b"Room") => ibt.room = r.number(),
            Some(b'T') if word.eq_ignore_ascii_case(b"Text") => ibt.text = r.line(),
            Some(b'*') => r.to_eol(),
            _ => {
                log.push(format!(
                    "SYSERR: Invalid keyword ({}) in IBT file",
                    String::from_utf8_lossy(&word)
                ));
                matched = false;
            }
        }
        if !matched {
            r.to_eol();
        }
    }
}

pub fn load_ibt_file(g: &mut Game, mode: i32) {
    let Some(idx) = mode_idx(mode) else {
        g.log(format!("SYSERR: Invalid mode ({}) in load_ibt_file", mode));
        return;
    };
    g.ibt.lists[idx].clear();
    let Some(path) = ibt_path(g, mode) else { return };
    let Ok(data) = std::fs::read(&path) else {
        g.log(format!("No File: misc/{}", ibt_basename(mode)));
        return;
    };
    let mut log = Vec::new();
    let mut out = Vec::new();
    {
        let mut r = IbtReader::new(&data);
        while let Some(ibt) = read_ibt(&mut r, &mut log) {
            out.push(ibt);
        }
    }
    for line in log {
        g.log(line);
    }
    g.ibt.lists[idx] = out;
}

pub fn save_ibt_file(g: &mut Game, mode: i32) {
    let Some(idx) = mode_idx(mode) else {
        g.log(format!("SYSERR: Invalid mode ({}) in save_ibt_file", mode));
        return;
    };
    let Some(path) = ibt_path(g, mode) else { return };
    let mut out = Vec::new();
    for ibt in &g.ibt.lists[idx] {
        if !ibt.text.is_empty() {
            out.extend_from_slice(b"Text      ");
            out.extend_from_slice(&ibt.text);
            out.extend_from_slice(b"~\n");
        }
        if !ibt.body.is_empty() {
            out.extend_from_slice(b"Body      ");
            out.extend_from_slice(&ibt.body);
            out.extend_from_slice(b"~\n");
        }
        if !ibt.name.is_empty() {
            out.extend_from_slice(b"Name      ");
            out.extend_from_slice(&ibt.name);
            out.extend_from_slice(b"~\n");
        }
        if !ibt.notes.is_empty() {
            out.extend_from_slice(b"Notes     ");
            out.extend_from_slice(&ibt.notes);
            out.extend_from_slice(b"~\n");
        }
        if ibt.id_num != NOBODY as i64 {
            out.extend_from_slice(format!("IdNum     {}\n", ibt.id_num).as_bytes());
        }
        if ibt.dated != 0 {
            out.extend_from_slice(format!("Dated     {}\n", ibt.dated).as_bytes());
        }
        out.extend_from_slice(format!("Level     {}\n", ibt.level).as_bytes());
        out.extend_from_slice(format!("Room      {}\n", ibt.room).as_bytes());
        out.extend_from_slice(
            format!(
                "Flags     {} {} {} {}\n",
                ibt.flags[0], ibt.flags[1], ibt.flags[2], ibt.flags[3]
            )
            .as_bytes(),
        );
        out.extend_from_slice(b"End\n");
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, &out).is_err() {
        g.log("SYSERR: Unable to open IBT file for writing in save_ibt_file".to_string());
        g.log(format!("        IBT File: misc/{}", ibt_basename(mode)));
    }
}

/// clean_ibt_list: drop body-less records (an aborted
/// write leaves one behind).
pub fn clean_ibt_list(g: &mut Game, mode: i32) {
    let Some(idx) = mode_idx(mode) else { return };
    g.ibt.lists[idx].retain(|i| !i.body.is_empty());
}

/// The editor's IBT half of playing_string_cleanup: the
/// newest record of that list is the one being written.
pub fn ibt_finish_write(g: &mut Game, mode: i32, body: Option<Vec<u8>>) {
    let Some(idx) = mode_idx(mode) else { return };
    if let Some(last) = g.ibt.lists[idx].last_mut() {
        last.body = body.unwrap_or_default();
    }
}

/// is_ibt_logger: idnum first, then name.
fn is_ibt_logger(g: &Game, ibt: &Ibt, chid: CharId) -> bool {
    let ch = g.ch(chid);
    if ch.is_npc() {
        return false;
    }
    if ibt.id_num != NOBODY as i64 && ibt.id_num == ch.idnum {
        return true;
    }
    ibt.name == ch.get_name()
}

/// TANA: "an" before a vowel, else "a".
fn tana(s: &[u8]) -> &'static [u8] {
    match s.first() {
        Some(c) if b"aeiouyAEIOUY".contains(c) => b"an",
        _ => b"a",
    }
}

/// do_ibt — `bug`, `idea` and `typo`.
pub fn do_ibt(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let Some(idx) = mode_idx(subcmd) else {
        g.log(format!("Invalid subcmd ({}) in do_ibt", subcmd));
        return;
    };
    let (arg, arg_text) = one_argument(argument);
    let (_, arg2, _) = two_arguments(argument);
    let cmd_name = g.commands.get(cmd).map(|c| c.command.clone()).unwrap_or_default();
    let type_name = IBT_TYPES[idx].as_bytes().to_vec();
    let level = g.ch(chid).level;

    let q = |g: &Game, c: &'static [u8]| cc(g, chid, C_SPR, c).to_vec();

    if arg.is_empty() {
        let (yel, nrm) = (q(g, KYEL), q(g, KNRM));
        let mut usage = Vec::new();
        let line = |label: &[u8]| {
            let mut l = b"       ".to_vec();
            l.extend_from_slice(&yel);
            l.extend_from_slice(&cmd_name);
            l.extend_from_slice(label);
            l.extend_from_slice(&nrm);
            l.extend_from_slice(b"\r\n");
            l
        };
        let mut first = line(b" submit <header>");
        first.splice(0..7, b"Usage: ".iter().copied());
        usage.extend_from_slice(&first);
        usage.extend_from_slice(&line(b" list"));
        usage.extend_from_slice(&line(b" show <num>"));
        if level >= LVL_GRGOD {
            usage.extend_from_slice(&line(b" remove <num>"));
            usage.extend_from_slice(&line(b" edit <num>"));
            usage.extend_from_slice(&line(b" resolve <num>"));
        }
        send_to_char(g, chid, &usage);
        if level < LVL_IMMORT {
            let mut m = b"Note: Only ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b"s logged by you will be listed or shown.\r\n");
            send_to_char(g, chid, &m);
        }
        return;
    }

    if is_abbrev(&arg, b"show") {
        if !is_number(&arg2) {
            let mut m = b"Show which ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b"?\r\n");
            send_to_char(g, chid, &m);
            return;
        }
        let ano = crate::handler::atoi(&arg2);
        let Some(ibt) = get_by_num(g, idx, ano) else {
            let mut m = b"That ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b" doesn't exist.\r\n");
            send_to_char(g, chid, &m);
            return;
        };
        if level < LVL_IMMORT && !is_ibt_logger(g, &ibt, chid) {
            let mut m = b"Sorry but you may only view ".to_vec();
            m.extend_from_slice(&type_name);
            // "\n\r" here, not "\r\n" — deliberate.
            m.extend_from_slice(b"s you have posted yourself.\n\r");
            send_to_char(g, chid, &m);
            return;
        }
        let (cyn, yel, nrm) = (q(g, KCYN), q(g, KYEL), q(g, KNRM));
        let mut out = cyn.clone();
        out.extend_from_slice(&type_name);
        out.extend_from_slice(b" by ");
        out.extend_from_slice(&yel);
        out.extend_from_slice(&ibt.name);
        out.extend_from_slice(b"\r\n");
        let timestr = if ibt.dated != 0 {
            crate::act::wizard::ctime_like(ibt.dated, g.tz_offset_secs)
        } else {
            "Unknown".to_string()
        };
        out.extend_from_slice(&cyn);
        out.extend_from_slice(b"Submitted: ");
        out.extend_from_slice(&yel);
        out.extend_from_slice(timestr.as_bytes());
        out.extend_from_slice(b"\r\n");
        if level >= LVL_IMMORT {
            out.extend_from_slice(&cyn);
            out.extend_from_slice(b"Level: ");
            out.extend_from_slice(&yel);
            out.extend_from_slice(format!("{}\r\n", ibt.level).as_bytes());
            out.extend_from_slice(&cyn);
            out.extend_from_slice(b"Room : ");
            out.extend_from_slice(&yel);
            out.extend_from_slice(format!("{}\r\n", ibt.room).as_bytes());
        }
        out.extend_from_slice(&cyn);
        out.extend_from_slice(b"Title: ");
        out.extend_from_slice(&yel);
        out.extend_from_slice(&ibt.text);
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&type_name);
        out.extend_from_slice(b" Details");
        out.extend_from_slice(&yel);
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&ibt.body);
        out.extend_from_slice(b"\r\n");
        if !ibt.notes.is_empty() {
            out.extend_from_slice(&cyn);
            out.extend_from_slice(&type_name);
            out.extend_from_slice(b" Notes");
            out.extend_from_slice(&yel);
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(&ibt.notes);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&type_name);
        out.extend_from_slice(b" Status: ");
        let resolved = ibt.flagged(IBT_RESOLVED);
        out.extend_from_slice(&q(g, if resolved { KBGRN } else { KBRED }));
        out.extend_from_slice(if resolved { &b"Resolved"[..] } else { &b"Unresolved"[..] });
        if ibt.flagged(IBT_INPROGRESS) {
            out.extend_from_slice(b" (In Progress)");
        }
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);
        return;
    }

    if is_abbrev(&arg, b"list") {
        list_ibt(g, chid, idx, &cmd_name, &type_name);
        return;
    }

    if is_abbrev(&arg, b"submit") {
        // `arg_text` is one_argument's raw remainder, so the headline
        // keeps the space that separated it from "submit", and a
        // whitespace-only tail counts as a heading (710).
        let heading = arg_text;
        if heading.is_empty() {
            send_to_char(g, chid, b"You need to add a heading!\r\n");
            return;
        }
        let plr_flag = match subcmd {
            SCMD_IDEA => flags::PLR_IDEA,
            SCMD_BUG => flags::PLR_BUG,
            _ => flags::PLR_TYPO,
        };
        g.ch_mut(chid).act.set(plr_flag);

        let mut m = b"Write your ".to_vec();
        m.extend_from_slice(&cmd_name);
        m.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &m);
        send_editor_help(g, chid);

        let mut msg = b"$n starts to give ".to_vec();
        msg.extend_from_slice(tana(&cmd_name));
        msg.push(b' ');
        msg.extend_from_slice(&cmd_name);
        msg.push(b'.');
        act(g, &msg, true, Some(chid), None, None, TO_ROOM);

        // string_write BEFORE the record is filled in: the editor writes
        // into the record that is linked afterwards.
        string_write(g, chid, MAX_IBT_LENGTH, 0, None);

        let room = g.ch(chid).in_room;
        let ibt = Ibt {
            room: if room == NOWHERE { NOWHERE as i32 } else { g.world.rooms[room as usize].vnum as i32 },
            level: g.ch(chid).level as i32,
            text: heading.to_vec(),
            name: g.ch(chid).get_name().to_vec(),
            id_num: g.ch(chid).idnum,
            dated: g.now,
            ..Default::default()
        };
        g.ibt.lists[idx].push(ibt);

        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(
            MudlogKind::Nrm,
            lvl,
            false,
            &format!(
                "{} has posted {} {}!",
                name,
                String::from_utf8_lossy(tana(&cmd_name)),
                String::from_utf8_lossy(&cmd_name)
            ),
        );
        return;
    }

    if is_abbrev(&arg, b"resolve") {
        if level < LVL_GRGOD {
            what_scold(g, chid, &type_name, level, &cmd_name);
            return;
        }
        if !is_number(&arg2) {
            let mut m = b"Resolve which ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b"?\r\n");
            send_to_char(g, chid, &m);
            return;
        }
        let ano = crate::handler::atoi(&arg2);
        let Some(pos) = index_of(g, idx, ano) else {
            let mut m = type_name.clone();
            m.extend_from_slice(b" not found\r\n");
            send_to_char(g, chid, &m);
            return;
        };
        if g.ibt.lists[idx][pos].flagged(IBT_RESOLVED) {
            let mut m = b"That ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b" has already been resolved!\r\n");
            send_to_char(g, chid, &m);
            return;
        }
        let mut m = type_name.clone();
        m.extend_from_slice(format!(" {} resolved!\r\n", ano).as_bytes());
        send_to_char(g, chid, &m);
        g.ibt.lists[idx][pos].set_flag(IBT_RESOLVED);
        if g.config.ibt_autosave {
            save_ibt_file(g, subcmd);
        }
        return;
    }

    if is_abbrev(&arg, b"remove") {
        if level < LVL_GRGOD {
            what_scold(g, chid, &type_name, level, &cmd_name);
            return;
        }
        if !is_number(&arg2) {
            let mut m = b"Remove which ".to_vec();
            m.extend_from_slice(&cmd_name);
            m.extend_from_slice(b"?\r\n");
            send_to_char(g, chid, &m);
            return;
        }
        let ano = crate::handler::atoi(&arg2);
        let (cyn, nrm) = (q(g, KCYN), q(g, KNRM));
        match index_of(g, idx, ano) {
            Some(pos) => {
                g.ibt.lists[idx].remove(pos);
                let mut m = cyn;
                m.extend_from_slice(&type_name);
                m.extend_from_slice(format!(" Number {} removed.", ano).as_bytes());
                m.extend_from_slice(&nrm);
                m.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &m);
                if g.config.ibt_autosave {
                    save_ibt_file(g, subcmd);
                }
            }
            None => {
                let mut m = type_name.clone();
                m.extend_from_slice(b" not found\r\n");
                send_to_char(g, chid, &m);
            }
        }
        return;
    }

    if is_abbrev(&arg, b"save") {
        if level < LVL_GRGOD {
            what_scold(g, chid, &type_name, level, &cmd_name);
            return;
        }
        save_ibt_file(g, subcmd);
        let mut m = type_name.clone();
        m.extend_from_slice(b" list saved.\r\n");
        send_to_char(g, chid, &m);
        return;
    }

    if is_abbrev(&arg, b"edit") {
        if level < LVL_GRGOD {
            what_scold(g, chid, &type_name, level, &cmd_name);
            return;
        }
        // do_oasis_ibtedit — the OLC editor lands with the rest of OLC.
        send_to_char(g, chid, b"The IBT editor is not available yet.\r\n");
        return;
    }

    what_scold(g, chid, &type_name, level, &cmd_name);
}

/// The shared "<Type> what?" refusal and its level-dependent usage tail.
fn what_scold(g: &mut Game, chid: CharId, type_name: &[u8], level: u8, cmd_name: &[u8]) {
    let mut m = type_name.to_vec();
    m.extend_from_slice(b" what?\r\n");
    send_to_char(g, chid, &m);
    if level < LVL_GRGOD {
        let mut u = b"Usage: ".to_vec();
        u.extend_from_slice(type_name);
        u.extend_from_slice(b" submit <text>\r\n");
        send_to_char(g, chid, &u);
    } else {
        let mut u = b"Usage:  ".to_vec();
        u.extend_from_slice(cmd_name);
        u.extend_from_slice(b" (submit/list/show/remove/resolve)\r\n");
        send_to_char(g, chid, &u);
    }
}

fn index_of(g: &Game, idx: usize, num: i32) -> Option<usize> {
    if num < 1 || num as usize > g.ibt.lists[idx].len() {
        None
    } else {
        Some(num as usize - 1)
    }
}

fn get_by_num(g: &Game, idx: usize, num: i32) -> Option<Ibt> {
    index_of(g, idx, num).map(|p| g.ibt.lists[idx][p].clone())
}

/// The `list` branch of do_ibt.
fn list_ibt(g: &mut Game, chid: CharId, idx: usize, cmd_name: &[u8], type_name: &[u8]) {
    let level = g.ch(chid).level;
    let q = |g: &Game, c: &'static [u8]| cc(g, chid, C_SPR, c).to_vec();
    let (cyn, grn, yel, red, nrm) = (q(g, KCYN), q(g, KGRN), q(g, KYEL), q(g, KRED), q(g, KNRM));
    let (bgrn, bred, bfred) = (q(g, KBGRN), q(g, KBRED), q(g, KBFRED));

    if g.ibt.lists[idx].is_empty() {
        let mut m = b"No ".to_vec();
        m.extend_from_slice(cmd_name);
        m.extend_from_slice(b"s have been reported!\r\n");
        crate::act::informative::page_string(g, chid, &m);
        return;
    }

    let mut buf = Vec::new();
    if level < LVL_IMMORT {
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" No ");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b"|");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" Description\r\n");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b" ---|-------------------------------------------------");
        buf.extend_from_slice(&nrm);
        buf.extend_from_slice(b"\r\n");
    } else {
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" No ");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b"|");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b"Name        ");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b"|");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b"Room  ");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b"|");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b"Level");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(b"|");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" Description\r\n");
        buf.extend_from_slice(&grn);
        buf.extend_from_slice(
            b" ---|------------|------|-----|-------------------------------------------------",
        );
        buf.extend_from_slice(&nrm);
        buf.extend_from_slice(b"\r\n");
    }

    let (mut i, mut num_res, mut num_unres) = (0i32, 0i32, 0i32);
    for pos in 0..g.ibt.lists[idx].len() {
        i += 1;
        let ibt = g.ibt.lists[idx][pos].clone();
        if level < LVL_IMMORT && !is_ibt_logger(g, &ibt, chid) {
            continue;
        }
        let imp: Vec<u8> = if ibt.flagged(IBT_IMPORTANT) {
            let mut v = bfred.clone();
            v.push(b'!');
            v.extend_from_slice(&nrm);
            v
        } else {
            b" ".to_vec()
        };
        let color = if ibt.flagged(IBT_RESOLVED) {
            num_res += 1;
            grn.clone()
        } else if ibt.flagged(IBT_INPROGRESS) {
            num_unres += 1;
            yel.clone()
        } else {
            num_unres += 1;
            red.clone()
        };

        buf.extend_from_slice(&imp);
        buf.extend_from_slice(&color);
        buf.extend_from_slice(format!("{:3}", i).as_bytes());
        if level < LVL_IMMORT {
            // The resolved (green) mortal row omits the separator colour
            // reset the other two carry — a format-string asymmetry.
            if !ibt.flagged(IBT_RESOLVED) {
                buf.extend_from_slice(&grn);
                buf.extend_from_slice(b"|");
                buf.extend_from_slice(&color);
            } else {
                buf.extend_from_slice(b"|");
            }
            buf.extend_from_slice(&ibt.text);
        } else {
            buf.extend_from_slice(&grn);
            buf.extend_from_slice(b"|");
            buf.extend_from_slice(&color);
            let mut n = ibt.name.clone();
            n.truncate(12);
            let pad = 12usize.saturating_sub(n.len());
            buf.extend_from_slice(&n);
            buf.extend(std::iter::repeat(b' ').take(pad));
            buf.extend_from_slice(&grn);
            buf.extend_from_slice(b"|");
            buf.extend_from_slice(&color);
            buf.extend_from_slice(format!("{:6}", ibt.room).as_bytes());
            buf.extend_from_slice(&grn);
            buf.extend_from_slice(b"|");
            buf.extend_from_slice(&color);
            buf.extend_from_slice(format!("{:5}", ibt.level).as_bytes());
            buf.extend_from_slice(&grn);
            buf.extend_from_slice(b"|");
            buf.extend_from_slice(&color);
            buf.extend_from_slice(&ibt.text);
        }
        buf.extend_from_slice(&nrm);
        buf.extend_from_slice(b"\r\n");
    }

    if num_res + num_unres > 0 {
        // "\n\r" before the summary, not "\r\n" — deliberate.
        buf.extend_from_slice(b"\n\r");
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(format!("{} ", i).as_bytes());
        buf.extend_from_slice(cmd_name);
        buf.extend_from_slice(b"s in file. ");
        buf.extend_from_slice(&bgrn);
        buf.extend_from_slice(format!("{}", num_res).as_bytes());
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" resolved, ");
        buf.extend_from_slice(&bred);
        buf.extend_from_slice(format!("{}", num_unres).as_bytes());
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b" unresolved");
        buf.extend_from_slice(&nrm);
        buf.extend_from_slice(b"\r\n");
        // "<Type>s in RED are unresolved <cmd>s." — the word is the colour,
        // rendered *in* that colour.
        for (c, colour, status) in [
            (&red, &b"RED"[..], &b"unresolved"[..]),
            (&yel, &b"YELLOW"[..], &b"in-progress"[..]),
            (&grn, &b"GREEN"[..], &b"resolved"[..]),
        ] {
            buf.extend_from_slice(&cyn);
            buf.extend_from_slice(type_name);
            buf.extend_from_slice(b"s in ");
            buf.extend_from_slice(c);
            buf.extend_from_slice(colour);
            buf.extend_from_slice(&cyn);
            buf.extend_from_slice(b" are ");
            buf.extend_from_slice(status);
            buf.push(b' ');
            buf.extend_from_slice(cmd_name);
            buf.extend_from_slice(b"s.\r\n");
        }
    } else {
        buf.extend_from_slice(b"No ");
        buf.extend_from_slice(cmd_name);
        buf.extend_from_slice(b"s have been found that were reported by you!\r\n");
    }
    if level >= LVL_GRGOD {
        buf.extend_from_slice(&cyn);
        buf.extend_from_slice(b"You may use ");
        buf.extend_from_slice(cmd_name);
        buf.extend_from_slice(b" remove, resolve or edit to change the list..");
        buf.extend_from_slice(&nrm);
        buf.extend_from_slice(b"\r\n");
    }
    buf.extend_from_slice(&cyn);
    buf.extend_from_slice(b"You may use ");
    buf.extend_from_slice(&yel);
    buf.extend_from_slice(cmd_name);
    buf.extend_from_slice(b" show <number>");
    buf.extend_from_slice(&cyn);
    buf.extend_from_slice(b" to see more indepth about the ");
    buf.extend_from_slice(cmd_name);
    buf.extend_from_slice(b".");
    buf.extend_from_slice(&nrm);
    buf.extend_from_slice(b"\r\n");

    crate::act::informative::page_string(g, chid, &buf);
}
