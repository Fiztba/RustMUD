//! Rebuilding `lib/plrfiles/index` from the player files themselves.
//!
//! The index is read at boot and never reconstructed, so losing it means
//! every `.plr` file survives and nobody can log in. This lives as a
//! subcommand rather than a standalone tool, so it shares the pfile parser
//! the server already uses instead of a second copy that can drift from
//! it.
//!
//! **This is not a transliteration of that tool, because that tool writes a
//! malformed index.** The index carries five fields:
//!
//! ```text
//! Each line is: id, name, level, flags, last-login.
//! Read back in the same order, stopping at the first field that fails.
//! ```
//!
//! while `rebuildAsciiIndex.c` writes six — `"%ld %s %d %d 0 %ld"`, with an
//! `adminlevel` inserted after the level. Parsed back by the server that
//! lands `adminlevel` in the flags field (through `asciiflag_conv`) and the
//! literal `0` in `last`, so a rebuilt index gives every player garbage
//! flags and a zeroed last-login. On the one day you need it.
//!
//! We write what the reader reads.
//!
//! ## What cannot be recovered
//!
//! The four `PINDEX_*` bits — DELETED, NODELETE, SELFDELETE, NOWIZLIST —
//! exist only in the index. Nothing in a `.plr` file records them, so a
//! rebuild necessarily resets them to zero, and that is worth stating
//! rather than hiding:
//!
//! * **NODELETE** protection is lost; a protected character becomes
//! deletable again.
//! * **DELETED / SELFDELETE** are lost, so a character who was flagged for
//! deletion but whose files have not yet been swept by `remove_player`
//! comes back as an ordinary player.
//! * **NOWIZLIST** is lost, so an immortal excluded from the wizlist
//! reappears on it at the next `autowiz` run.
//!
//! Recovering those means editing the rebuilt index by hand. The report
//! this returns names every entry so that is possible.

use std::io;
use std::path::Path;

use crate::players::{load_char, save_index, IndexEntry};

/// What a rebuild found, for the caller to print.
#[derive(Debug, Default)]
pub struct Report {
    /// Entries written, in the order they were written.
    pub entries: Vec<IndexEntry>,
    /// `.plr` files that could not be parsed at all, with the reason.
    pub unreadable: Vec<(String, String)>,
    /// Files parsed but missing an `Id:` line — these cannot be indexed,
    /// because the id is what the rest of the game keys on.
    pub no_id: Vec<String>,
}

/// Rebuild the index by walking `lib/plrfiles` for `*.plr` files.
///
/// Nothing is written until every file has been read, so a parse failure
/// part-way through leaves the existing index untouched.
pub fn rebuild_player_index(lib: &Path) -> io::Result<Report> {
    let root = lib.join("plrfiles");
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }

    let mut report = Report::default();
    let mut names: Vec<Vec<u8>> = Vec::new();
    collect_plr_names(&root, &mut names, &mut report)?;

    // readdir order is neither sorted nor stable across filesystems. Sort
    // by id so a rebuild is reproducible and two runs can be diffed against
    // each other.
    for name in names {
        match load_char(lib, &name) {
            Some((pf, errors)) => {
                let shown = String::from_utf8_lossy(&name).into_owned();
                if pf.idnum == 0 {
                    report.no_id.push(shown);
                    continue;
                }
                for e in errors {
                    report.unreadable.push((shown.clone(), e));
                }
                report.entries.push(IndexEntry {
                    // The index stores names lowercase (create_entry's
                    // invariant), and the filename is
                    // already lowercase because get_filename lowered it.
                    name,
                    id: pf.idnum,
                    // Nothing is clamped here; the level in the file
                    // is what the index carries.
                    level: pf.level,
                    // Not recoverable from a pfile -- see the module note.
                    flags: 0,
                    last: pf.last_logon,
                });
            }
            None => report.unreadable.push((
                String::from_utf8_lossy(&name).into_owned(),
                "could not be read or parsed".to_string(),
            )),
        }
    }

    report.entries.sort_by_key(|e| e.id);
    save_index(lib, &report.entries)?;
    Ok(report)
}

/// Every `<name>.plr` under `dir`, recursively. A flat scan would report a
/// directory it cannot open and carries on; so does this.
fn collect_plr_names(dir: &Path, out: &mut Vec<Vec<u8>>, report: &mut Report) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            report.unreadable.push((dir.display().to_string(), e.to_string()));
            return Ok(());
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_plr_names(&path, out, report)?;
            continue;
        }
        // parsename: the name is everything before the FIRST '.', and the
        // extension must be exactly ".plr".
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else { continue };
        let Some((stem, ext)) = fname.split_once('.') else { continue };
        if ext != "plr" || stem.is_empty() {
            continue;
        }
        out.push(stem.as_bytes().to_vec());
    }
    Ok(())
}
