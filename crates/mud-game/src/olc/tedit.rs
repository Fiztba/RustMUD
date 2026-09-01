//! The text-file editor.
//!
//! tedit is the one OLC state that never reaches nanny: it hands the
//! descriptor straight to the line editor and does all its work in the
//! editor's cleanup. `OLC_STORAGE` carries the file name, and the field
//! index rides in `number`.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::interpreter::one_argument;
use crate::olc::{clear_screen, OlcData, CLEANUP_ALL};

/// One row of do_tedit's `fields[]`: keyword, level, cap,
/// and the file it is read from and written back to, relative to lib/.
struct Field {
    cmd: &'static str,
    level: u8,
    size: usize,
    file: &'static [&'static str],
}

const FIELDS: &[Field] = &[
    Field { cmd: "credits", level: LVL_IMPL, size: 2400, file: &["text", "credits"] },
    Field { cmd: "news", level: LVL_GRGOD, size: 8192, file: &["text", "news"] },
    Field { cmd: "motd", level: LVL_GRGOD, size: 2400, file: &["text", "motd"] },
    Field { cmd: "imotd", level: LVL_IMPL, size: 2400, file: &["text", "imotd"] },
    Field { cmd: "greetings", level: LVL_IMPL, size: 2400, file: &["text", "greetings"] },
    Field { cmd: "help", level: LVL_GRGOD, size: 2400, file: &["text", "help", "help"] },
    Field { cmd: "ihelp", level: LVL_GRGOD, size: 2400, file: &["text", "help", "ihelp"] },
    Field { cmd: "info", level: LVL_GRGOD, size: 8192, file: &["text", "info"] },
    Field { cmd: "background", level: LVL_IMPL, size: 8192, file: &["text", "background"] },
    Field { cmd: "handbook", level: LVL_IMPL, size: 8192, file: &["text", "handbook"] },
    Field { cmd: "policies", level: LVL_IMPL, size: 8192, file: &["text", "policies"] },
    Field { cmd: "wizlist", level: LVL_IMPL, size: 2400, file: &["text", "wizlist"] },
    Field { cmd: "immlist", level: LVL_GRGOD, size: 2400, file: &["text", "immlist"] },
];

fn buffer(g: &Game, idx: usize) -> &BStr {
    let t = &g.texts;
    match FIELDS[idx].cmd {
        "credits" => &t.credits,
        "news" => &t.news,
        "motd" => &t.motd,
        "imotd" => &t.imotd,
        "greetings" => &t.greetings,
        "help" => &t.help_screen,
        "ihelp" => &t.ihelp_screen,
        "info" => &t.info,
        "background" => &t.background,
        "handbook" => &t.handbook,
        "policies" => &t.policies,
        "wizlist" => &t.wizlist,
        _ => &t.immlist,
    }
}

fn set_buffer(g: &mut Game, idx: usize, v: BStr) {
    let t = &mut g.texts;
    let slot = match FIELDS[idx].cmd {
        "credits" => &mut t.credits,
        "news" => &mut t.news,
        "motd" => &mut t.motd,
        "imotd" => &mut t.imotd,
        "greetings" => &mut t.greetings,
        "help" => &mut t.help_screen,
        "ihelp" => &mut t.ihelp_screen,
        "info" => &mut t.info,
        "background" => &mut t.background,
        "handbook" => &mut t.handbook,
        "policies" => &mut t.policies,
        "wizlist" => &mut t.wizlist,
        _ => &mut t.immlist,
    };
    *slot = v;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_tedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let (field, _) = one_argument(argument);

    if field.is_empty() {
        send_to_char(g, chid, b"Files available to be edited:\r\n");
        let level = g.ch(chid).level;
        let mut i = 0;
        let mut out: BStr = Vec::new();
        for f in FIELDS {
            if level >= f.level {
                let b = f.cmd.as_bytes();
                let n = b.len().min(11);
                out.extend_from_slice(&b[..n]);
                out.extend(std::iter::repeat(b' ').take(11 - n));
                out.push(b' ');
                i += 1;
                if i % 7 == 0 {
                    out.extend_from_slice(b"\r\n");
                }
            }
        }
        if i % 7 != 0 {
            out.extend_from_slice(b"\r\n");
        }
        if i == 0 {
            out.extend_from_slice(b"None.\r\n");
        }
        send_to_char(g, chid, &out);
        return;
    }

    // strncmp(field, cmd, strlen(field)): a prefix match, case sensitive.
    let Some(l) = FIELDS.iter().position(|f| f.cmd.as_bytes().starts_with(&field[..])) else {
        send_to_char(g, chid, b"Invalid text editor option.\r\n");
        return;
    };
    if g.ch(chid).level < FIELDS[l].level {
        send_to_char(g, chid, b"You are not godly enough for that!\r\n");
        return;
    }

    clear_screen(g, di);
    send_editor_help(g, chid);
    send_to_char(g, chid, b"Edit file below:\r\n\r\n");

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_tedit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();
    olc.number = l as i32;
    let mut path = g.lib_dir.clone();
    for part in FIELDS[l].file {
        path = path.join(part);
    }
    olc.storage = Some(path.to_string_lossy().into_owned().into_bytes());

    let current = buffer(g, l).clone();
    let backstr = if current.is_empty() {
        None
    } else {
        send_to_char(g, chid, &current);
        Some(current)
    };
    let size = FIELDS[l].size;
    g.olc.insert(di, olc);
    string_write(g, chid, size, 0, backstr);

    act(g, b"$n begins editing a scroll.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Tedit;
    }
}

pub fn tedit_string_cleanup(
    g: &mut Game,
    di: usize,
    olc: Box<OlcData>,
    text: Option<BStr>,
    saved: bool,
) -> Option<Box<OlcData>> {
    let chid = g.descriptors.get(di).and_then(|d| d.character);
    // "if (!storage) terminator = STRINGADD_ABORT;"
    let storage = olc.storage.clone();
    let saved = saved && storage.is_some();

    if saved {
        let idx = olc.number as usize;
        let path = std::path::PathBuf::from(String::from_utf8_lossy(&storage.unwrap()).into_owned());
        let mut body = text.unwrap_or_default();
        body.retain(|&b| b != b'\r');
        if std::fs::write(&path, &body).is_err() {
            let msg = format!("SYSERR: Can't write file '{}'.", path.display());
            g.mudlog(MudlogKind::Cmp, LVL_IMPL, true, &msg);
        } else {
            set_buffer(g, idx, body);
            if let Some(chid) = chid {
                let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                let level = (LVL_GOD as i16).max(g.ch(chid).invis_lev()) as u8;
                let msg = format!("OLC: {} saves '{}'.", name, path.display());
                g.mudlog(MudlogKind::Cmp, level, true, &msg);
            }
            write_to_desc(g, di, b"Saved.\r\n");
            // The (news)/(motd) prompt flags key off these two mtimes.
            match FIELDS[idx].cmd {
                "news" => g.texts.newsmod = g.now,
                "motd" => g.texts.motdmod = g.now,
                _ => {}
            }
        }
    } else {
        write_to_desc(g, di, b"Edit aborted.\r\n");
        if let Some(chid) = chid {
            act(g, b"$n stops editing some scrolls.", true, Some(chid), None, None, TO_ROOM);
        }
        if let Some(t) = text {
            set_buffer(g, olc.number as usize, t);
        }
    }

    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Playing;
    }
    None
}
