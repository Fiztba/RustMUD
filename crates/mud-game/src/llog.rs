//! The login log (`lib/etc/last`) and the in-memory
//! `recent` list.
//!
//! `etc/last` is a versioned ASCII file. A fixed-size binary image
//! whose layout depends on pointer width and struct padding (304 bytes on
//! 64-bit Linux, 292 on 32-bit) and which pads with uninitialised bytes.
//! Both legacy layouts still load; new writes are versioned ASCII.
//!
//! The `recent` list is memory-only — "since last copyover/reboot" is the
//! whole retention policy.

use mud_data::ids::CharId;
use mud_data::types::*;

use crate::comm::send_to_char;
use crate::game::Game;

pub const LAST_CONNECT: i32 = 0;
pub const LAST_RECONNECT: i32 = 2;
pub const LAST_QUIT: i32 = 4;
pub const LAST_IDLEOUT: i32 = 5;
pub const LAST_DISCONNECT: i32 = 6;
pub const LAST_SHUTDOWN: i32 = 7;
pub const LAST_REBOOT: i32 = 8;
pub const LAST_CRASH: i32 = 9;

pub const MAX_LAST_ENTRIES: usize = 6000;

pub const LAST_ARRAY: [&str; 10] = [
    "Connect",
    "Enter Game",
    "Reconnect",
    "Takeover",
    "Quit",
    "Idleout",
    "Disconnect",
    "Shutdown",
    "Reboot",
    "Crash",
];

/// struct last_entry.
#[derive(Debug, Clone, Default)]
pub struct LastEntry {
    pub close_type: i32,
    pub hostname: Vec<u8>,
    pub username: Vec<u8>,
    pub time: i64,
    pub close_time: i64,
    pub idnum: i32,
    pub punique: i32,
}

/// struct recent_player.
#[derive(Debug, Clone, Default)]
pub struct RecentPlayer {
    pub vnum: i32,
    pub name: Vec<u8>,
    pub new_player: bool,
    pub copyover_player: bool,
    pub time: i64,
    pub host: Vec<u8>,
}

fn last_path(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("etc").join("last")
}

// -------------------------------------------------------------- file format

fn is_ascii_last(data: &[u8]) -> bool {
    data.starts_with(b"*") || data.starts_with(b"Last")
}

/// The legacy raw-struct layouts: 304 bytes = LP64, 292 = ILP32.
fn parse_binary_last(data: &[u8]) -> Option<Vec<LastEntry>> {
    let (rec, o_time, o_close, o_id, o_puniq, tsz) = if data.len() % 304 == 0 && !data.is_empty() {
        (304usize, 280usize, 288usize, 296usize, 300usize, 8usize)
    } else if data.len() % 292 == 0 && !data.is_empty() {
        (292, 276, 280, 284, 288, 4)
    } else {
        return None;
    };
    let cstr = |b: &[u8]| -> Vec<u8> {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        b[..end].to_vec()
    };
    let rd = |b: &[u8], o: usize, sz: usize| -> i64 {
        if sz == 8 {
            i64::from_le_bytes(b[o..o + 8].try_into().unwrap())
        } else {
            i32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as i64
        }
    };
    Some(
        data.chunks_exact(rec)
            .map(|c| LastEntry {
                close_type: rd(c, 0, 4) as i32,
                hostname: cstr(&c[4..260]),
                username: cstr(&c[260..276]),
                time: rd(c, o_time, tsz),
                close_time: rd(c, o_close, tsz),
                idnum: rd(c, o_id, 4) as i32,
                punique: rd(c, o_puniq, 4) as i32,
            })
            .collect(),
    )
}

fn parse_ascii_last(data: &[u8]) -> Vec<LastEntry> {
    let mut out = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line[0] == b'*' {
            continue;
        }
        // Last: <close_type> <time> <close_time> <idnum> <punique> <user> <host>
        let mut parts = line.splitn(8, |b| *b == b' ');
        if parts.next() != Some(b"Last:") {
            continue;
        }
        let mut num = || -> i64 {
            parts.next().map(|t| crate::handler::atoi(t) as i64).unwrap_or(0)
        };
        let close_type = num() as i32;
        let time = num();
        let close_time = num();
        let idnum = num() as i32;
        let punique = num() as i32;
        let username = parts.next().unwrap_or(b"").to_vec();
        let hostname = parts.next().unwrap_or(b"").to_vec();
        out.push(LastEntry {
            close_type,
            hostname,
            username,
            time,
            close_time,
            idnum,
            punique,
        });
    }
    out
}

fn write_last_file(g: &mut Game, entries: &[LastEntry]) {
    let mut out = b"* tbaMUD login log (ASCII v1)\n".to_vec();
    for e in entries {
        out.extend_from_slice(
            format!(
                "Last: {} {} {} {} {} ",
                e.close_type, e.time, e.close_time, e.idnum, e.punique
            )
            .as_bytes(),
        );
        // The name never contains a space; the host is the rest of the line.
        out.extend_from_slice(if e.username.is_empty() { b"-" } else { &e.username });
        out.push(b' ');
        out.extend_from_slice(if e.hostname.is_empty() { b"-" } else { &e.hostname });
        out.push(b'\n');
    }
    let path = last_path(g);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, &out) {
        g.log(format!("Error trying to open new last file: {}", e));
    }
}

fn read_last_file(g: &mut Game) -> Option<Vec<LastEntry>> {
    let data = std::fs::read(last_path(g)).ok()?;
    if data.is_empty() {
        return Some(Vec::new());
    }
    if is_ascii_last(&data) {
        Some(parse_ascii_last(&data))
    } else {
        match parse_binary_last(&data) {
            Some(v) => {
                g.log(format!("   Converting legacy binary last file ({} records) to ASCII.", v.len()));
                Some(v)
            }
            None => {
                g.log("clean_llog_entries: read error or unexpected end of file.".to_string());
                None
            }
        }
    }
}

/// clean_llog_entries: trim to the newest
/// MAX_LAST_ENTRIES records.
pub fn clean_llog_entries(g: &mut Game) {
    let Some(entries) = read_last_file(g) else { return };
    if entries.len() < MAX_LAST_ENTRIES {
        // Nothing to rewrite in this case — except a legacy binary file,
        // which still has to be converted, so write when the format
        // changed.
        let was_ascii =
            std::fs::read(last_path(g)).map(|d| d.is_empty() || is_ascii_last(&d)).unwrap_or(true);
        if !was_ascii {
            write_last_file(g, &entries);
        }
        return;
    }
    let keep = entries[entries.len() - MAX_LAST_ENTRIES..].to_vec();
    write_last_file(g, &keep);
}

/// find_llog_entry + mod_llog_entry + add_llog_entry
/// as one pass: update the newest matching (idnum, punique) row if there is
/// one, else append. A quit/idleout/reboot/shutdown close type is inviolate.
pub fn add_llog_entry(g: &mut Game, chid: CharId, type_: i32) {
    let (punique, idnum, name, host) = {
        let ch = g.ch(chid);
        (
            ch.punique,
            ch.idnum as i32,
            ch.get_name().to_vec(),
            ch.player_specials.as_ref().and_then(|p| p.host.clone()).unwrap_or_default(),
        )
    };
    // A name entered with a bad password never gets a pref assigned.
    if punique <= 0 {
        return;
    }
    let mut entries = read_last_file(g).unwrap_or_default();

    let found = entries.iter().rposition(|e| e.idnum == idnum && e.punique == punique);
    match found {
        Some(i) => {
            let e = &mut entries[i];
            if e.close_type != LAST_QUIT
                && e.close_type != LAST_IDLEOUT
                && e.close_type != LAST_REBOOT
                && e.close_type != LAST_SHUTDOWN
            {
                e.close_type = type_;
            }
            e.close_time = g.now;
        }
        None => {
            let mut username = name;
            username.truncate(15);
            let mut hostname = host;
            hostname.truncate(127);
            entries.push(LastEntry {
                close_type: type_,
                hostname,
                username,
                time: g.now,
                close_time: 0,
                idnum,
                punique,
            });
        }
    }
    write_last_file(g, &entries);
}

// ------------------------------------------------------------- recent list

/// AddRecentPlayer. The row is prepended and then handed
/// `get_max_recent` *without* the +1 just used, so the two newest rows
/// share a vnum. Deliberate.
pub fn add_recent_player(g: &mut Game, name: &[u8], host: &[u8], newplr: bool, cpyplr: bool) -> bool {
    if name.is_empty() {
        return false;
    }
    let max = g.recent_list.iter().map(|r| r.vnum).max().unwrap_or(0);
    g.recent_list.insert(
        0,
        RecentPlayer {
            vnum: max + 1,
            name: name.to_vec(),
            new_player: newplr,
            copyover_player: cpyplr,
            time: g.now,
            host: host.to_vec(),
        },
    );
    let max_vnum = g.recent_list.iter().map(|r| r.vnum).max().unwrap_or(0);
    g.recent_list[0].vnum = max_vnum;
    true
}

pub fn do_recent(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::comm::{cc, send_to_char, C_SPR, KCYN, KNRM, KRED, KYEL};
    let (arg, _) = crate::interpreter::one_argument(argument);
    let limit = if arg.is_empty() { 0 } else { crate::handler::atoi(&arg) };
    let level = g.ch(chid).level;
    let high = level >= LVL_GRGOD;

    if high {
        send_to_char(
            g,
            chid,
            b" ID | DATE/TIME                | HOST IP                          | Player Name\r\n",
        );
    } else {
        send_to_char(g, chid, b" ID | DATE/TIME                | Player Name\r\n");
    }

    let q = |g: &Game, c: &'static [u8]| cc(g, chid, C_SPR, c).to_vec();
    let (nrm, red, yel, cyn) = (q(g, KNRM), q(g, KRED), q(g, KYEL), q(g, KCYN));

    let mut hits = 0;
    let mut count = 0;
    for r in g.recent_list.clone() {
        hits += 1;
        if limit != 0 && count >= limit {
            // Drop the cursor here and stop counting further rows.
            break;
        }
        let mut timestr = crate::act::wizard::ctime_like(r.time, g.tz_offset_secs);
        // "%a %b %d %H:%M:%S %Y" — ctime order with the year last, then a
        // %-24.24s clip.
        timestr = reorder_ctime(&timestr);
        timestr.truncate(24);

        let mut line = format!("{:3} | {:<24} | ", r.vnum, timestr).into_bytes();
        if high {
            let loc = r.host == b"localhost";
            if loc {
                line.extend_from_slice(&red);
            }
            let mut h = r.host.clone();
            let pad = 32usize.saturating_sub(h.len());
            h.extend(std::iter::repeat(b' ').take(pad));
            line.extend_from_slice(&h);
            line.extend_from_slice(&nrm);
            line.extend_from_slice(b" | ");
        }
        line.extend_from_slice(&r.name);
        if r.new_player {
            line.push(b' ');
            line.extend_from_slice(&yel);
            line.extend_from_slice(b"(New Player)");
            line.extend_from_slice(&nrm);
        } else if r.copyover_player {
            line.push(b' ');
            line.extend_from_slice(&cyn);
            line.extend_from_slice(b"(Copyover)");
            line.extend_from_slice(&nrm);
        }
        line.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &line);
        count += 1;
    }

    let now = crate::act::wizard::ctime_like(g.now, g.tz_offset_secs);
    let m = format!(
        "Current Server Time: {}\r\nShowing {} players since last copyover/reboot\r\n",
        now, hits
    );
    send_to_char(g, chid, m.as_bytes());
}

/// "Sat Aug 22 20:05:00 2026" -> "Sat Aug 22 20:05:00 2026" is already the
/// %a %b %d %H:%M:%S %Y order; only the day padding differs (%d vs %e).
fn reorder_ctime(c: &str) -> String {
    let p: Vec<&str> = c.split_whitespace().collect();
    if p.len() < 5 {
        return c.to_string();
    }
    format!("{} {} {:02} {} {}", p[0], p[1], p[2].parse::<i32>().unwrap_or(0), p[3], p[4])
}

// ---------------------------------------------------------------------------
// do_last
// ---------------------------------------------------------------------------

/// list_llog_entries — `last *`, IMPL only.
///
/// **F2 (mandated fix).** An unconditional `break` at the end of the loop
/// body would print exactly one row however many records the file holds,
/// while the loop, its header and its ferror check all plainly intend the
/// whole log. There is no such break here.
fn list_llog_entries(g: &mut Game, chid: CharId) {
    let entries = read_last_file(g);
    if entries.is_none() {
        g.log("llist_log_entries: could not open last log file.".to_string());
        send_to_char(g, chid, b"Error! - no last log");
    }
    send_to_char(g, chid, b"Last log\r\n");
    let Some(entries) = entries else { return };
    let rows: Vec<Vec<u8>> = entries
        .iter()
        .map(|e| {
            let timestr = strftime_last_full(e.time, g.tz_offset_secs);
            let mut out = crate::act::pad_left(&e.username, 10);
            out.extend_from_slice(format!("    {}    ", e.punique).as_bytes());
            out.extend_from_slice(
                LAST_ARRAY.get(e.close_type as usize).copied().unwrap_or("").as_bytes(),
            );
            out.extend_from_slice(b"    ");
            out.extend_from_slice(timestr.as_bytes());
            out.extend_from_slice(b"\r\n");
            out
        })
        .collect();
    for r in rows {
        send_to_char(g, chid, &r);
    }
}

/// strftime "%a %b %d %Y %H:%M:%S".
fn strftime_last_full(unix: i64, tz: i64) -> String {
    let c = crate::act::wizard::ctime_like(unix, tz);
    let p: Vec<&str> = c.split_whitespace().collect();
    format!("{} {} {:02} {} {}", p[0], p[1], p[2].parse::<i32>().unwrap_or(0), p[4], p[3])
}

/// strftime "%a %b %d %Y %H:%M".
fn strftime_last_hm(unix: i64, tz: i64) -> String {
    let full = strftime_last_full(unix, tz);
    full[..full.len() - 3].to_string()
}

fn is_in_game(g: &Game, idnum: i32) -> Option<CharId> {
    for &di in &g.descriptors.order {
        let Some(d) = g.descriptors.get(di) else { continue };
        let Some(c) = d.character else { continue };
        if g.try_ch(c).is_some_and(|ch| ch.idnum == idnum as i64) {
            return Some(c);
        }
    }
    None
}

pub fn do_last(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::interpreter::half_chop;

    let mut name: Vec<u8> = Vec::new();
    let mut num = 0i32;
    if !argument.is_empty() {
        let (mut arg, mut rest) = half_chop(argument);
        while !arg.is_empty() {
            if (arg.first() == Some(&b'*') || arg == b"all") && g.ch(chid).level == LVL_IMPL {
                list_llog_entries(g, chid);
                return;
            }
            if arg[0].is_ascii_digit() {
                num = crate::handler::atoi(&arg).max(0);
            } else {
                name = arg.clone();
            }
            let (a, r) = half_chop(&rest);
            arg = a;
            rest = r;
        }
    }

    if !name.is_empty() && num == 0 {
        // `last <name>`: read the pfile, not the login log.
        let Some(vict) = crate::players_glue::load_char_offline(g, &name) else {
            send_to_char(g, chid, b"There is no such player.\r\n");
            return;
        };
        let timestr = {
            // "%a %b %d %H:%M:%S %Y"
            let logon = g.ch(vict).time.logon;
            let c = crate::act::wizard::ctime_like(logon, g.tz_offset_secs);
            let p: Vec<&str> = c.split_whitespace().collect();
            format!("{} {} {:02} {} {}", p[0], p[1], p[2].parse::<i32>().unwrap_or(0), p[3], p[4])
        };
        let host = g
            .ch(vict)
            .player_specials
            .as_ref()
            .and_then(|ps| ps.host.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| b"(NOHOST)".to_vec());
        let class = crate::act::informative::class_abbr(g.ch(vict).class);
        let mut out =
            format!("[{:5}] [{:2} ", g.ch(vict).idnum, g.ch(vict).level).into_bytes();
        out.extend_from_slice(class);
        out.extend_from_slice(b"] ");
        out.extend_from_slice(&crate::act::pad_right(g.ch(vict).get_name(), 12));
        out.extend_from_slice(b" : ");
        out.extend_from_slice(&crate::act::pad_right(&host, 18));
        out.extend_from_slice(b" : ");
        out.extend_from_slice(&crate::act::pad_right(timestr.as_bytes(), 24));
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);
        crate::players_glue::free_offline_char(g, vict);
        return;
    }

    if num <= 0 || num >= 100 {
        num = 10;
    }
    let Some(entries) = read_last_file(g) else {
        send_to_char(g, chid, b"No entries found.\r\n");
        return;
    };
    if entries.is_empty() {
        send_to_char(g, chid, b"No entries found.\r\n");
        return;
    }

    send_to_char(g, chid, b"Last log\r\n");
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut left = num;
    for e in entries.iter().rev() {
        if left <= 0 {
            break;
        }
        if !name.is_empty() && !name.eq_ignore_ascii_case(&e.username) {
            continue;
        }
        let timestr = strftime_last_hm(e.time, g.tz_offset_secs);
        // '%10.10s %20.20s %20.21s - ' — every field is RIGHT-justified,
        // since none carries a '-' flag. It read as left-justified here
        // until stage9-pager first paged `last` to the end.
        let mut out = crate::act::pad_left_trunc(&e.username, 10, 10);
        out.push(b' ');
        out.extend_from_slice(&crate::act::pad_left_trunc(&e.hostname, 20, 20));
        out.push(b' ');
        out.extend_from_slice(&crate::act::pad_left_trunc(timestr.as_bytes(), 20, 21));
        out.extend_from_slice(b" - ");
        let still = is_in_game(g, e.idnum)
            .and_then(|c| g.try_ch(c))
            .is_some_and(|c| c.punique == e.punique);
        if still {
            out.extend_from_slice(b"Still Playing  ");
        } else {
            let delta = e.close_time - e.time;
            let to = {
                let c = crate::act::wizard::ctime_like(e.close_time, g.tz_offset_secs);
                c.split_whitespace().nth(3).unwrap_or("").to_string()
            };
            let mut to = to.into_bytes();
            to.truncate(5);
            // gmtime(delta) formatted %H:%M — the elapsed-time hack.
            let d = delta.max(0);
            let deltastr = format!("{:02}:{:02}", (d / 3600) % 24, (d / 60) % 60);
            out.extend_from_slice(&crate::act::pad_right_trunc(&to, 5));
            out.extend_from_slice(b" (");
            out.extend_from_slice(&crate::act::pad_right_trunc(deltastr.as_bytes(), 5));
            out.extend_from_slice(b") ");
            out.extend_from_slice(
                LAST_ARRAY.get(e.close_type as usize).copied().unwrap_or("").as_bytes(),
            );
        }
        out.extend_from_slice(b"\r\n");
        rows.push(out);
        left -= 1;
    }
    for r in rows {
        send_to_char(g, chid, &r);
    }
}
