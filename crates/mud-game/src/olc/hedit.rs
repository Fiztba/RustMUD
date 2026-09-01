//! The help editor, plus `helpcheck`.
//!
//! Each help *keyword* is its own row sharing one entry string, so
//! `zone_num` names a row and the editor walks back to the `duplicate == 0`
//! row before touching anything. Saving writes `lib/text/help/help.hlp` and
//! then reboots the whole help table off disk, which is what re-splits the
//! keyword line and fixes up the duplicate rows the in-memory assignment
//! left pointing at the old text.
//!
//! `HEDIT_KEYWORDS` is unreachable: the menu offers only the entry and the
//! minimum level, and nothing sets that mode.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::wizstat::HEDIT_PERMISSION;
use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::db::{add_to_save_list, remove_from_save_list, SL_HLP};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::interpreter::one_argument;
use crate::olc::{
    can_edit_zone, clear_screen, get_char_colors, str_udup, OlcData, StrTarget, CLEANUP_ALL,
    CLEANUP_STRUCTS,
};
use crate::text::HelpEntry;

/// HEDIT connectedness.
pub const HEDIT_CONFIRM_SAVESTRING: i32 = 0;
pub const HEDIT_CONFIRM_EDIT: i32 = 1;
pub const HEDIT_CONFIRM_ADD: i32 = 2;
pub const HEDIT_MAIN_MENU: i32 = 3;
pub const HEDIT_ENTRY: i32 = 4;
pub const HEDIT_MIN_LEVEL: i32 = 6;
pub const HEDIT_CONFIRM_DELETE: i32 = 7;

/// MAX_MESSAGE_LENGTH, the cap hedit hands the line editor.
const MAX_MESSAGE_LENGTH: usize = 8192;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_hedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }
    if !can_edit_zone(g, chid, HEDIT_PERMISSION) {
        send_to_char(g, chid, b"You don't have access to editing help files.\r\n");
        return;
    }
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) == Some(ConState::Hedit) {
            send_to_char(
                g,
                chid,
                b"Sorry, only one can person can edit help files at a time.\r\n",
            );
            return;
        }
    }

    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Please specify a help entry to edit.\r\n");
        return;
    }

    if crate::text::cmp_ci(b"save", &arg) == std::cmp::Ordering::Equal {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(MudlogKind::Cmp, level, true, &format!("OLC: {} saves help files.", name));
        // The delete pairs its add_to_save_list with the removal inside
        // hedit_save_to_disk. This path never added, so the removal found
        // nothing and logged "remove_from_save_list: Saved item not found."
        // on every `hedit save`. Pairing it changes nothing about what
        // reaches disk.
        add_to_save_list(g, HEDIT_PERMISSION as Idx, SL_HLP);
        if hedit_save_to_disk(g) {
            send_to_char(g, chid, b"Saving help files.\r\n");
        } else {
            send_to_char(g, chid, &crate::olc::save_failed("the help files"));
        }
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();
    olc.number = 0;
    olc.storage = Some(arg.clone());

    let found = crate::text::search_help(g, &arg, LVL_IMPL as i32);
    // The NOWHERE test comes first. Reading the row before it would index
    // the table at 65535 for every `hedit <new keyword>`, and whatever sat
    // there would send the walk hunting for a matching entry, which can
    // land the builder in an unrelated
    // help file.
    let znum = match found {
        Some(mut i) => {
            if g.help_table[i].duplicate != 0 {
                let entry = g.help_table[i].entry.clone();
                for j in 0..g.help_table.len() {
                    if g.help_table[j].duplicate == 0
                        && std::rc::Rc::ptr_eq(&g.help_table[j].entry, &entry)
                    {
                        i = j;
                        break;
                    }
                }
            }
            Some(i)
        }
        None => None,
    };
    olc.zone_num = znum.map(|i| i as i32).unwrap_or(NOWHERE as i32);
    olc.help_version = g.help_table_version;
    olc.help_key = znum.map(|i| g.help_table[i].keyword.clone());
    olc.help_text = znum.map(|i| g.help_table[i].entry.as_ref().clone());

    match znum {
        None => {
            let mut msg = b"Do you wish to add the '".to_vec();
            msg.extend_from_slice(&arg);
            msg.extend_from_slice(b"' help file? ");
            send_to_char(g, chid, &msg);
            olc.mode = HEDIT_CONFIRM_ADD;
        }
        Some(i) => {
            // No trailing space on this one, unlike every other prompt here.
            let mut msg = b"Do you wish to edit the '".to_vec();
            msg.extend_from_slice(&g.help_table[i].keyword);
            msg.extend_from_slice(b"' help file?");
            send_to_char(g, chid, &msg);
            olc.mode = HEDIT_CONFIRM_EDIT;
        }
    }

    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Hedit;
    }
    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    g.mudlog(MudlogKind::Cmp, level, true, &format!("OLC: {} starts editing help files.", name));
}

fn hedit_setup_new(olc: &mut OlcData) {
    olc.help = Some(Box::new(HelpEntry {
        keyword: olc.storage.clone().unwrap_or_default(),
        entry: std::rc::Rc::new(b"KEYWORDS\r\n\r\nThis help file is unfinished.\r\n".to_vec()),
        min_level: 0,
        duplicate: 0,
    }));
    olc.value = 0;
}

/// Load an existing entry into the editor. An empty body becomes
/// "undefined".
fn hedit_setup_existing(g: &Game, olc: &mut OlcData, rnum: usize) {
    let e = &g.help_table[rnum];
    olc.help = Some(Box::new(HelpEntry {
        keyword: str_udup(&e.keyword),
        entry: std::rc::Rc::new(str_udup(&e.entry)),
        duplicate: e.duplicate,
        min_level: e.min_level,
    }));
    olc.value = 0;
}

/// A write has to land on the entry's primary row: hedit_save_to_disk skips
/// duplicates, so one that lands on a duplicate never reaches the file.
fn hedit_primary_of(g: &Game, i: usize) -> usize {
    if g.help_table[i].duplicate == 0 {
        return i;
    }
    let entry = g.help_table[i].entry.clone();
    g.help_table
        .iter()
        .position(|e| e.duplicate == 0 && std::rc::Rc::ptr_eq(&e.entry, &entry))
        // A duplicate with no primary. The table is already wrong, and this at
        // least does not reach into another entry.
        .unwrap_or(i)
}

/// What [`hedit_relocate`] found.
enum Relocated {
    /// The row, canonicalized to the entry's primary.
    Row(usize),
    /// Something matched, but nothing uniquely. Refuse rather than guess.
    Ambiguous,
    /// The entry is gone; the save becomes an add.
    NotFound,
}

/// Find the row this editor opened, in a table that has been rebuilt since it
/// opened.
///
/// Not by what the builder typed: that is a word, and answering 'n' at the
/// confirm prompt walks forward to the next row that word abbreviates, so
/// after a walk it names a different entry than the one being edited.
///
/// And not by the row's keyword alone. A row's keyword is one word, and the
/// first word of a multi-keyword entry can be another entry's only keyword --
/// the shipped help file has eleven such collisions, `spells` among them. On
/// the word alone this took whichever came first, so editing the magic entry
/// through `magics` and saving after a reload destroyed the separate `spells`
/// entry, kept the builder's work out of the entry they opened, and left two
/// entries with the same keyword line in the file.
///
/// So: the pair, then the keyword alone where it names exactly one row, and
/// otherwise no answer rather than a guess. Comparing the text by content
/// rather than by handle identity is deliberate: the reload that made the
/// index stale also replaced every handle in the table.
fn hedit_relocate(g: &Game, key: Option<&[u8]>, text: Option<&[u8]>) -> Relocated {
    let mut matched = false;

    // 1. Both. The reload that changed nothing, and every reload that changed
    //    something else, land here.
    if let (Some(k), Some(t)) = (key, text) {
        if let Some(i) = g
            .help_table
            .iter()
            .position(|e| e.keyword == k && e.entry.as_slice() == t)
        {
            return Relocated::Row(hedit_primary_of(g, i));
        }
    }

    // 2. The keyword, if it names exactly one row: the text was edited.
    //
    //    There is deliberately no rule between these two matching on the text
    //    alone. boot_help puts the keyword line INTO the entry text, so text
    //    that still matches exactly is text whose keyword line is unchanged --
    //    which means the captured keyword is still one of that entry's rows,
    //    and step 1 has already answered. A rename changes the text along with
    //    the keywords and lands here or nowhere.
    if let Some(k) = key {
        let rows: Vec<usize> = g
            .help_table
            .iter()
            .enumerate()
            .filter(|(_, e)| e.keyword == k)
            .map(|(i, _)| i)
            .collect();
        if rows.len() == 1 {
            return Relocated::Row(hedit_primary_of(g, rows[0]));
        }
        if rows.len() > 1 {
            matched = true;
        }
    }

    if matched {
        Relocated::Ambiguous
    } else {
        Relocated::NotFound
    }
}

/// The row this editor is on, or `None` if it is not a row any more.
///
/// olc.zone_num is the index hedit_setup_existing read to fill the editor, and
/// nothing moves it afterwards -- the confirm prompt's 'n' walk happens before
/// setup. So once the table is known not to have been rebuilt, that index
/// still names the entry on the builder's screen, and there is nothing left
/// for a re-resolution to add.
fn hedit_find_row(g: &Game, olc: &OlcData) -> Option<usize> {
    if olc.help_version != g.help_table_version {
        return None;
    }
    if olc.zone_num == NOWHERE as i32 || olc.zone_num < 0 {
        return None;
    }
    let i = olc.zone_num as usize;
    if i >= g.help_table.len() {
        return None;
    }
    Some(i)
}

/// Remove a help entry, and every row that shares its text.
///
/// One entry is several rows -- one per keyword -- all holding the same text.
/// Removing only the row the builder opened would leave the others pointing at
/// an entry that is no longer written out, so `help <other keyword>` would go
/// on finding a heading with nothing under it.
fn hedit_delete_entry(g: &mut Game, rnum: usize) -> bool {
    if rnum >= g.help_table.len() {
        return false;
    }
    let text = g.help_table[rnum].entry.clone();

    // Never leave the table empty. hedit_save_to_disk writes help.hlp and
    // boots the table straight back off it; on a file with no entries that is
    // "boot error - 0 records counted", which would take the server down
    // mid-command and fail every boot after it until somebody edited the file
    // by hand.
    let keep = g
        .help_table
        .iter()
        .filter(|e| !std::rc::Rc::ptr_eq(&e.entry, &text))
        .count();
    if keep == 0 {
        return false;
    }

    let before = g.help_table.len();
    g.help_table.retain(|e| !std::rc::Rc::ptr_eq(&e.entry, &text));
    // The table changed shape, so anyone holding an index into it is holding a
    // stale one. Nothing outlives this command today; the counter's whole value
    // is that it is bumped without needing to know that.
    g.help_table_version += 1;
    before != g.help_table.len()
}

/// What a save did. `Refused` means nothing was written and nothing was
/// discarded, and the caller says why.
pub enum Saved {
    Ok,
    WriteFailed,
    Refused,
}

fn hedit_save_internally(g: &mut Game, olc: &mut OlcData) -> Saved {
    // An index into a table that has been rebuilt since the editor opened
    // names whatever now sits in that slot, so writing through it overwrites
    // an entry the builder never asked for. Only a help reload can do that
    // while hedit is open, since hedit refuses a second editor.
    //
    // Take the row again rather than refusing outright: this is the last thing
    // that runs before the editor is torn down, so a flat refusal would throw
    // the builder's work away to protect somebody else's. Treating it as new
    // is not an option either -- a reload that changes nothing still bumps the
    // counter, and appending then puts a second entry in help.hlp under the
    // same keyword, which search_help resolves to the older of the two.
    //
    // Only where the row genuinely cannot be identified is the save refused,
    // and then the version is deliberately left stale: the builder goes back
    // to the editor, and their next attempt has to come through here again
    // rather than sail past a check that has already been satisfied.
    if olc.help_version != g.help_table_version {
        match hedit_relocate(g, olc.help_key.as_deref(), olc.help_text.as_deref()) {
            Relocated::Ambiguous => return Saved::Refused,
            Relocated::NotFound => olc.zone_num = NOWHERE as i32,
            Relocated::Row(i) => olc.zone_num = i as i32,
        }
        olc.help_version = g.help_table_version;
    }

    // The write always lands on the entry's primary row, stale index or not.
    // hedit_save_to_disk skips duplicates, so a builder who reached one with
    // 'n' at the confirm prompt had their min_level change written nowhere at
    // all -- only the entry text survived, and only because the pass below
    // carries it to the primary by hand.
    if olc.zone_num != NOWHERE as i32
        && olc.zone_num >= 0
        && (olc.zone_num as usize) < g.help_table.len()
    {
        olc.zone_num = hedit_primary_of(g, olc.zone_num as usize) as i32;
    }
    if let Some(h) = olc.help.as_mut() {
        h.duplicate = 0;
    }

    let help = olc.help.as_ref().unwrap().as_ref().clone();
    if olc.zone_num == NOWHERE as i32 {
        g.help_table.push(help);
        g.help_table_version += 1;
    } else {
        // The row's twins share its text -- that is how one entry answers to
        // several keywords -- and they hold their own handle on it, so
        // replacing this row alone leaves them showing the old version. That
        // matters even though the table is rebooted from disk immediately
        // afterwards: hedit_save_to_disk writes the PRIMARY row of each
        // entry, so a builder who reached a duplicate row with 'n' would
        // otherwise have their edit written nowhere at all.
        let rnum = olc.zone_num as usize;
        let old = g.help_table[rnum].entry.clone();
        let new = help.entry.clone();
        g.help_table[rnum] = help;
        for i in 0..g.help_table.len() {
            if i != rnum && std::rc::Rc::ptr_eq(&g.help_table[i].entry, &old) {
                g.help_table[i].entry = new.clone();
            }
        }
    }
    add_to_save_list(g, HEDIT_PERMISSION as Idx, SL_HLP);
    if hedit_save_to_disk(g) {
        Saved::Ok
    } else {
        Saved::WriteFailed
    }
}

/// hedit_save_to_disk: every non-duplicate row's entry,
/// then `#<min level>`, then the `$~` terminator — and a full reboot of the
/// help table off disk afterwards.
pub fn hedit_save_to_disk(g: &mut Game) -> bool {
    let mut out: BStr = Vec::new();
    for e in &g.help_table {
        if e.duplicate != 0 {
            continue;
        }
        let mut buf: BStr = if e.entry.is_empty() {
            b"Empty\r\n".to_vec()
        } else {
            e.entry.as_ref().clone()
        };
        buf.retain(|&b| b != b'\r');
        mud_net::editor::parse_tab(&mut buf);
        out.extend_from_slice(&buf);
        out.extend_from_slice(format!("#{}\n", e.min_level).as_bytes());
    }
    out.extend_from_slice(b"$~\n");

    let path = g.lib_dir.join("text").join("help").join("help.hlp");
    if std::fs::write(&path, &out).is_err() {
        g.log("SYSERR: Could not write help index file".to_string());
        return false;
    }
    remove_from_save_list(g, HEDIT_PERMISSION as Idx, SL_HLP);

    // Reboot the help files.
    let mut log = Vec::new();
    g.help_table = crate::text::boot_help(&g.lib_dir, g.mini_mud, &mut log);
    g.help_table_version += 1;
    for line in log {
        g.log(line);
    }
    true
}

fn hedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    let c = g.olc_colors;
    let h = olc.help.as_ref().unwrap();
    let mut out: BStr = Vec::new();
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"-- Help file editor\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"1");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Entry       :\r\n");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(h.entry.as_ref());
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"2");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Min Level   : ");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(h.min_level.to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"X");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Delete this help entry\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"Q");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Quit\r\nEnter choice : ");
    write_to_desc(g, di, &out);
    olc.mode = HEDIT_MAIN_MENU;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn hedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    match olc.mode {
        HEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    // The save can decline, so it happens before anything is
                    // announced or torn down. The old order logged the edit and
                    // said it had reached disk before the write was attempted.
                    let outcome = hedit_save_internally(g, &mut olc);
                    if matches!(outcome, Saved::Refused) {
                        write_to_desc(
                            g,
                            di,
                            b"The help files were reloaded while you were editing, and more \
                              than one entry now answers to what you opened. Writing to the \
                              wrong one would destroy an entry you never touched, so nothing \
                              has been saved. Your work is still here.\r\n",
                        );
                        hedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let kw = String::from_utf8_lossy(&olc.help.as_ref().unwrap().keyword)
                            .into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        // passing (TRUE, level, CMP) to a mudlog that
                        // takes (type, level, file) sends this out at BRF.
                        g.mudlog(
                            MudlogKind::Cmp,
                            level,
                            true,
                            &format!("OLC: {} edits help for {}.", name, kw),
                        );
                    }
                    if matches!(outcome, Saved::Ok) {
                        write_to_desc(g, di, b"Help saved to disk.\r\n");
                    } else {
                        write_to_desc(g, di, &crate::olc::save_failed("the help files"));
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_STRUCTS);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to save your changes? : \r\n",
                    );
                }
            }
            return Some(olc);
        }

        HEDIT_CONFIRM_EDIT => {
            // Above the match, not inside one arm of it. All three arms read
            // the same row -- 'y' to fill the editor, 'n' to walk
            // to the next match, and the reprompt to name the entry -- and the
            // index they share was taken before a reload could move it. The
            // reprompt is the one a builder is most likely to reach, since a
            // bare RETURN lands there.
            //
            // Refusing costs nothing here: nothing has been typed yet. That is
            // why this says so and stops, where the save -- which runs after
            // the work is done -- goes looking for the row instead.
            if olc.help_version != g.help_table_version {
                write_to_desc(
                    g,
                    di,
                    b"The help files were reloaded while you were deciding, so that is not \
                      necessarily the entry you asked for any more. Nothing has been \
                      changed; run hedit again.\r\n",
                );
                crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                return None;
            }
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let rnum = olc.zone_num as usize;
                    olc.help_key = Some(g.help_table[rnum].keyword.clone());
                    olc.help_text = Some(g.help_table[rnum].entry.as_ref().clone());
                    hedit_setup_existing(g, &mut olc, rnum);
                    hedit_disp_menu(g, di, &mut olc);
                }
                Some(b'q') | Some(b'Q') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    // A walk that gives up on the FIRST non-match never
                    // reaches a second entry whose keyword the argument
                    // also abbreviates, and offers to add instead. Hence
                    // the else clause here,
                    // which aedit's otherwise identical loop lacks.
                    let storage = olc.storage.clone().unwrap_or_default();
                    let mut znum = olc.zone_num as usize + 1;
                    while znum < g.help_table.len()
                        && !crate::handler::is_abbrev(&storage, &g.help_table[znum].keyword)
                    {
                        znum += 1;
                    }
                    if znum >= g.help_table.len() {
                        olc.zone_num = NOWHERE as i32;
                        let mut msg = b"Do you wish to add the '".to_vec();
                        msg.extend_from_slice(&storage);
                        msg.extend_from_slice(b"' help file? ");
                        write_to_desc(g, di, &msg);
                        olc.mode = HEDIT_CONFIRM_ADD;
                    } else {
                        olc.zone_num = znum as i32;
                        let mut msg = b"Do you wish to edit the '".to_vec();
                        msg.extend_from_slice(&g.help_table[znum].keyword);
                        msg.extend_from_slice(b"' help file? ");
                        write_to_desc(g, di, &msg);
                        olc.mode = HEDIT_CONFIRM_EDIT;
                    }
                }
                _ => {
                    let mut msg = b"Invalid choice!\r\nDo you wish to edit the '".to_vec();
                    msg.extend_from_slice(&g.help_table[olc.zone_num as usize].keyword);
                    msg.extend_from_slice(b"' help file? ");
                    write_to_desc(g, di, &msg);
                }
            }
            return Some(olc);
        }

        HEDIT_CONFIRM_ADD => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    hedit_setup_new(&mut olc);
                    hedit_disp_menu(g, di, &mut olc);
                }
                Some(b'n') | Some(b'N') | Some(b'q') | Some(b'Q') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    let mut msg = b"Invalid choice!\r\nDo you wish to add the '".to_vec();
                    msg.extend_from_slice(&olc.storage.clone().unwrap_or_default());
                    msg.extend_from_slice(b"' help file? ");
                    write_to_desc(g, di, &msg);
                }
            }
            return Some(olc);
        }

        HEDIT_CONFIRM_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let row = hedit_find_row(g, &olc);
                    if row.is_some_and(|r| hedit_delete_entry(g, r)) {
                        // hedit_save_to_disk ends by removing this from the
                        // save list, so it has to be on it -- the ordinary save
                        // adds it immediately before saving for exactly this
                        // reason. Without the add, every deletion logs
                        // "remove_from_save_list: Saved item not found."
                        add_to_save_list(g, HEDIT_PERMISSION as Idx, SL_HLP);
                        if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                            let kw = String::from_utf8_lossy(
                                &olc.help.as_ref().unwrap().keyword,
                            )
                            .into_owned();
                            let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                            g.mudlog(
                                MudlogKind::Cmp,
                                level,
                                true,
                                &format!("OLC: {} deletes help entry '{}'", name, kw),
                            );
                        }
                        write_to_desc(g, di, b"Help entry deleted.\r\n");
                        // Rewrites help.hlp from the table and reboots it,
                        // which is how every other hedit change reaches disk.
                        hedit_save_to_disk(g);
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    // Nothing was removed, so nothing is thrown away either --
                    // cleaning up here would discard the builder's unsaved work
                    // on top of refusing the delete.
                    if hedit_find_row(g, &olc).is_some() {
                        // Found, so the refusal came from the last-entry guard.
                        // Saying it was reloaded would be false twice over,
                        // with the entry still on screen underneath.
                        write_to_desc(
                            g,
                            di,
                            b"That is the last help entry left. The MUD cannot boot from a \
                              help file with none, so it will not be deleted.\r\n",
                        );
                    } else {
                        write_to_desc(
                            g,
                            di,
                            b"That entry is no longer in the help table. It may have been \
                              reloaded while you were editing it. Nothing was deleted.\r\n",
                        );
                    }
                    hedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'n') | Some(b'N') => {
                    hedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDelete this help entry, and every keyword that \
                          reaches it? : ",
                    );
                    return Some(olc);
                }
            }
        }

        HEDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'x') | Some(b'X') => {
                    if hedit_find_row(g, &olc).is_none() {
                        write_to_desc(
                            g,
                            di,
                            b"That entry is not in the help table -- either it was never saved, \
                              or the table was reloaded while you were editing. Quit without \
                              saving.\r\n",
                        );
                        hedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    write_to_desc(
                        g,
                        di,
                        b"Delete this help entry, and every keyword that reaches it? : ",
                    );
                    olc.mode = HEDIT_CONFIRM_DELETE;
                    return Some(olc);
                }
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = HEDIT_CONFIRM_SAVESTRING;
                    } else {
                        write_to_desc(g, di, b"No changes made.\r\n");
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                }
                Some(b'1') => {
                    olc.mode = HEDIT_ENTRY;
                    clear_screen(g, di);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter help entry: (/s saves /h for help)\r\n");
                    let old = olc.help.as_ref().unwrap().entry.as_ref().clone();
                    let old = if old.is_empty() { None } else { Some(old) };
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, MAX_MESSAGE_LENGTH, 0, old);
                    }
                    olc.str_target = Some(StrTarget::HelpEntry);
                    olc.value = 1;
                }
                Some(b'2') => {
                    write_to_desc(g, di, b"Enter min level : ");
                    olc.mode = HEDIT_MIN_LEVEL;
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    hedit_disp_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        HEDIT_MIN_LEVEL => {
            let number = atoi(arg);
            if !(0..=LVL_IMPL as i32).contains(&number) {
                write_to_desc(g, di, b"That is not a valid choice!\r\nEnter min level:-\r\n] ");
                return Some(olc);
            }
            olc.help.as_mut().unwrap().min_level = number;
        }

        _ => {
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: Reached default case in parse_hedit",
            );
        }
    }

    olc.value = 1;
    hedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

pub fn hedit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    if olc.str_target.take() == Some(StrTarget::HelpEntry) {
        olc.help.as_mut().unwrap().entry = std::rc::Rc::new(text.unwrap_or_default());
    }
    if olc.mode == HEDIT_ENTRY {
        hedit_disp_menu(g, di, &mut olc);
    }
    Some(olc)
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_helpcheck(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    let mut buf: BStr = Vec::new();
    let mut count = 0usize;
    for i in 1..g.commands.len() {
        // Socials are do_action and are skipped; minimum_level >= 0 always
        // holds for the table as it stands.
        if g.commands[i].social.is_some() {
            continue;
        }
        let cmd = g.commands[i].command.clone();
        if crate::text::search_help(g, &cmd, LVL_IMPL as i32).is_some() {
            continue;
        }
        count += 1;
        let n = cmd.len().min(20);
        buf.extend_from_slice(&cmd[..n]);
        buf.extend(std::iter::repeat(b' ').take(20 - n));
        if count % 3 == 0 {
            buf.extend_from_slice(b"\r\n");
        }
    }
    if count % 3 != 0 {
        buf.extend_from_slice(b"\r\n");
    }
    if g.ch(chid).desc.is_none() {
        return;
    }
    if buf.is_empty() {
        send_to_char(g, chid, b"All commands have help entries.\r\n");
    } else {
        send_to_char(g, chid, b"Commands without help entries:\r\n");
        crate::act::informative::page_string(g, chid, &buf);
    }
}
