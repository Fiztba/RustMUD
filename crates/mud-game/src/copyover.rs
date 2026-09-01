//! Copyover — reboot the executable without dropping the players
//!
//! **F7 (mandate).** The traditional handoff is Unix-only: the listening
//! and player sockets go to a fresh `circle` through `execl`, which
//! inherits file descriptors. That path is kept here unchanged — same
//! `copyover.dat`, same `-C<fd>` argument. Windows has no fd inheritance
//! across `exec`, so it
//! gets an equivalent handoff: the parent spawns the successor, asks Winsock
//! to duplicate each socket **into that process** with `WSADuplicateSocketW`,
//! and writes the resulting `WSAPROTOCOL_INFOW` blobs into `copyover.dat`;
//! the child rebuilds every socket with `WSASocketW(..., FROM_PROTOCOL_INFO)`
//! and carries on. The players never see a disconnect on either platform.
//!
//! The two file layouts differ only in the first column of each row (a raw
//! fd versus a hex blob), so `copyover.dat` is unchanged on Unix.

use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::game::Game;

/// One saved connection.
#[derive(Debug, Clone)]
pub struct CopyoverEntry {
    /// Unix: the raw descriptor. Windows: unused (see `blob`).
    pub fd: i64,
    /// Windows: the WSAPROTOCOL_INFOW bytes for the duplicated socket.
    pub blob: Vec<u8>,
    pub pref: i64,
    pub name: BStr,
    pub host: BStr,
    /// CopyoverGet's compact protocol string ("80/24TNM...").
    pub guiopt: BStr,
}

/// What `do_copyover` leaves behind for the server binary to act on.
#[derive(Debug, Default)]
pub struct CopyoverPlan {
    pub entries: Vec<CopyoverEntry>,
    /// Descriptor indices whose sockets must survive into the successor.
    pub descs: Vec<usize>,
    pub boot_time: i64,
}

pub fn copyover_path(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("..").join("copyover.dat")
}

pub fn copyover_get(p: &mud_net::protocol::ProtocolState) -> BStr {
    use mud_net::protocol::Var;
    let mut out = format!("{}/{}", p.screen_width, p.screen_height).into_bytes();
    if p.ttype {
        out.push(b'T');
    }
    if p.naws {
        out.push(b'N');
    }
    if p.msdp {
        out.push(b'M');
    }
    if p.atcp {
        out.push(b'A');
    }
    if p.msp {
        out.push(b'S');
    }
    if p.vars[Var::MXP as usize].value_int != 0 {
        out.push(b'X');
    }
    if p.mccp {
        out.push(b'c');
    }
    if p.vars[Var::XTERM_256_COLORS as usize].value_int != 0 {
        out.push(b'C');
    }
    if p.charset {
        out.push(b'H');
    }
    if p.vars[Var::UTF_8 as usize].value_int != 0 {
        out.push(b'U');
    }
    out
}

pub fn copyover_set(p: &mut mud_net::protocol::ProtocolState, data: &[u8]) {
    use mud_net::protocol::Var;
    let (mut width, mut height) = (0i32, 0i32);
    let mut done_width = false;
    for &c in data {
        match c {
            b'T' => p.ttype = true,
            b'N' => p.naws = true,
            b'M' => p.msdp = true,
            b'A' => p.atcp = true,
            b'S' => p.msp = true,
            b'X' => {
                p.mxp = true;
                p.vars[Var::MXP as usize].value_int = 1;
            }
            b'c' => p.mccp = true,
            b'C' => p.vars[Var::XTERM_256_COLORS as usize].value_int = 1,
            b'H' => p.charset = true,
            b'U' => p.vars[Var::UTF_8 as usize].value_int = 1,
            b'/' => done_width = true,
            d if d.is_ascii_digit() => {
                // Width comes before the slash, height after it.
                if done_width {
                    height = height * 10 + (d - b'0') as i32;
                } else {
                    width = width * 10 + (d - b'0') as i32;
                }
            }
            _ => {}
        }
    }
    if width > 0 {
        p.screen_width = width;
    }
    if height > 0 {
        p.screen_height = height;
    }
    p.negotiated = true;
}

/// do_copyover. Everything up to the `execl`: the
/// per-player save, the "remain seated" notice, and the plan the binary needs
/// to hand the sockets over.
pub fn do_copyover(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    // copyover.dat is opened first: bail out if it is not writable. The probe
    // file is then removed, because the real one is not written until the
    // handoff itself, and a successor that finds an empty copyover.dat in the
    // meantime would recover nothing from it.
    let path = copyover_path(g);
    if let Err(_e) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path)
    {
        crate::comm::send_to_char(g, chid, b"Copyover file not writeable, aborted.\r\n");
        return;
    }
    let _ = std::fs::remove_file(&path);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let notice = format!("\r\n *** COPYOVER by {} - please remain seated!\r\n", name);

    let mut plan = CopyoverPlan { boot_time: g.boot_time, ..Default::default() };

    for di in g.descriptors.order.clone() {
        let Some(d) = g.descriptors.get(di) else { continue };
        let och = d.character;
        let state = d.state;
        let original = d.original;

        // A switched immortal is put back in their own body first.
        // Returning out of do_copyover here would abort the whole copyover
        // the moment anyone is switched, so un-switch and carry on.
        if och.is_some() && original.is_some() {
            if let Some(o) = och {
                crate::act::wizard::return_to_char(g, o);
            }
        }

        let Some(d) = g.descriptors.get(di) else { continue };
        let och = d.character;
        let host = d.host.clone();
        let guiopt = copyover_get(&d.protocol);
        #[cfg(unix)]
        let fd = {
            use std::os::unix::io::AsRawFd;
            d.stream.as_ref().map(|s| s.as_raw_fd() as i64).unwrap_or(-1)
        };
        #[cfg(not(unix))]
        let fd = -1i64;

        let playing = och.is_some() && (state as u8) <= (ConState::Playing as u8);
        if !playing {
            // Drop those still logging in.
            crate::comm::write_direct(
                g,
                di,
                b"\r\nSorry, we are rebooting. Come back in a few minutes.\r\n",
            );
            crate::run::close_socket(g, di);
            continue;
        }

        let och = och.unwrap();
        if g.try_ch(och).is_none() {
            continue;
        }
        let pname = g.ch(och).name.clone().unwrap_or_default();
        let pref = g.ch(och).punique as i64;
        plan.entries.push(CopyoverEntry {
            fd,
            blob: Vec::new(),
            pref,
            name: pname,
            host,
            guiopt,
        });
        plan.descs.push(di);

        // Save the character exactly where they stand.
        let room = g.ch(och).in_room;
        let vnum = if room == NOWHERE { NOWHERE as i32 } else { g.world.rooms[room as usize].vnum as i32 };
        g.ch_mut(och).ps_mut().load_room = vnum as Idx;
        crate::objsave::crash_rentsave(g, och, 0);
        crate::players_glue::save_char(g, och);
        crate::comm::write_direct(g, di, notice.as_bytes());
    }

    g.copyover = Some(plan);
}

/// Serialize copyover.dat.
pub fn write_copyover_file(g: &Game, plan: &CopyoverPlan, listener_blob: &[u8]) -> std::io::Result<()> {
    let mut out = format!("{}\n", plan.boot_time).into_bytes();
    if !listener_blob.is_empty() {
        out.extend_from_slice(b"M ");
        out.extend_from_slice(hex(listener_blob).as_bytes());
        out.push(b'\n');
    }
    for e in &plan.entries {
        if e.blob.is_empty() {
            out.extend_from_slice(e.fd.to_string().as_bytes());
        } else {
            out.push(b'X');
            out.extend_from_slice(hex(&e.blob).as_bytes());
        }
        out.extend_from_slice(format!(" {} ", e.pref).as_bytes());
        out.extend_from_slice(&e.name);
        out.push(b' ');
        out.extend_from_slice(if e.host.is_empty() { b"-" } else { &e.host });
        out.push(b' ');
        out.extend_from_slice(if e.guiopt.is_empty() { b"-" } else { &e.guiopt });
        out.push(b'\n');
    }
    out.extend_from_slice(b"-1\n");

    // A successor that cannot inherit the sockets in place is already running
    // by the time this is written and reads the file the moment it boots.
    // Writing beside it and renaming into place means the successor sees the
    // whole handoff or no handoff, never half of one.
    let path = copyover_path(g);
    let tmp = path.with_extension("dat.tmp");
    std::fs::write(&tmp, &out)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// The parsed file, for the recovering process.
#[derive(Debug, Default)]
pub struct CopyoverFile {
    pub boot_time: i64,
    pub listener_blob: Vec<u8>,
    pub entries: Vec<CopyoverEntry>,
}

pub fn read_copyover_file(path: &std::path::Path) -> Option<CopyoverFile> {
    let data = std::fs::read(path).ok()?;
    let mut out = CopyoverFile::default();
    let mut lines = data.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    out.boot_time = crate::handler::atoi(lines.next().unwrap_or(b"")) as i64;
    // The trailing "-1" is the only thing separating a finished file from one
    // still being written, so its absence means "not yet", not "empty".
    let mut complete = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line == b"-1" {
            complete = true;
            break;
        }
        if let Some(rest) = line.strip_prefix(b"M ") {
            out.listener_blob = unhex(rest);
            continue;
        }
        let mut f = line.split(|&c| c == b' ').filter(|x| !x.is_empty());
        let (Some(first), Some(pref), Some(name)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let host = f.next().unwrap_or(b"-");
        let guiopt = f.next().unwrap_or(b"-");
        let (fd, blob) = if let Some(h) = first.strip_prefix(b"X") {
            (-1, unhex(h))
        } else {
            (crate::handler::atoi(first) as i64, Vec::new())
        };
        out.entries.push(CopyoverEntry {
            fd,
            blob,
            pref: crate::handler::atoi(pref) as i64,
            name: name.to_vec(),
            host: if host == b"-" { Vec::new() } else { host.to_vec() },
            guiopt: if guiopt == b"-" { Vec::new() } else { guiopt.to_vec() },
        });
    }
    if !complete {
        return None;
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn unhex(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i + 1 < s.len() {
        let hi = (s[i] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (s[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
        out.push(hi << 4 | lo);
        i += 2;
    }
    out
}

// ---------------------------------------------------------------------------
// Platform socket handoff
// ---------------------------------------------------------------------------

/// The handoff primitives live in `mud-sys`, the workspace's single crate
/// with `unsafe`: every OS interface for giving a live
/// socket to a successor process is raw FFI.
pub use mud_sys as plat;

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str, body: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("rustmud-copyover-{}-{}", std::process::id(), name));
        std::fs::write(&p, body).expect("write scratch handoff file");
        p
    }

    #[test]
    fn handoff_file_round_trips() {
        let p = scratch(
            "full",
            b"1756600000\nM 4142\nX6465 7 Fizban 1.2.3.4 80/24TNM\n12 9 Nauzhror - -\n-1\n",
        );
        let cf = read_copyover_file(&p).expect("a complete file parses");
        assert_eq!(cf.boot_time, 1756600000);
        assert_eq!(cf.listener_blob, vec![0x41, 0x42]);
        assert_eq!(cf.entries.len(), 2);

        assert_eq!(cf.entries[0].blob, vec![0x64, 0x65]);
        assert_eq!(cf.entries[0].pref, 7);
        assert_eq!(cf.entries[0].name, b"Fizban");
        assert_eq!(cf.entries[0].host, b"1.2.3.4");
        assert_eq!(cf.entries[0].guiopt, b"80/24TNM");

        // The descriptor form, and the "-" that stands in for an empty field.
        assert_eq!(cf.entries[1].fd, 12);
        assert!(cf.entries[1].blob.is_empty());
        assert!(cf.entries[1].host.is_empty());
        assert!(cf.entries[1].guiopt.is_empty());

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_file_without_its_terminator_is_not_a_handoff() {
        // What a successor sees when it beats the predecessor to the write:
        // the writability probe, or a partial line, is not something to
        // recover from.
        for (name, body) in [
            ("empty", &b""[..]),
            ("header", &b"1756600000\n"[..]),
            ("cut", &b"1756600000\nX6465 7 Fizban 1.2.3.4 80/24TNM\n"[..]),
        ] {
            let p = scratch(name, body);
            assert!(read_copyover_file(&p).is_none(), "{} should not parse", name);
            let _ = std::fs::remove_file(&p);
        }
    }

    #[test]
    fn protocol_options_round_trip() {
        let mut p = mud_net::protocol::ProtocolState::default();
        p.screen_width = 100;
        p.screen_height = 40;
        p.msdp = true;
        p.naws = true;
        let saved = copyover_get(&p);

        let mut q = mud_net::protocol::ProtocolState::default();
        copyover_set(&mut q, &saved);
        assert_eq!(q.screen_width, 100);
        assert_eq!(q.screen_height, 40);
        assert!(q.msdp);
        assert!(q.naws);
        assert!(q.negotiated);
    }
}
