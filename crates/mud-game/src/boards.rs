//! The bulletin boards.
//!
//! The message store is two-level on purpose: a global slot table
//! (`msg_storage`) plus a per-board index that names a slot.
//! `board_remove_msg` refuses to delete a message while a descriptor is
//! still writing into that slot, and the editor session is identified by
//! exactly that slot number.
//!
//! The on-disk board is a versioned ASCII file. A fixed-size binary image
//! of the message index would carry a live pointer (8 bytes of process
//! address) into the file and made the format depend on pointer width and
//! padding. Boards now load the legacy 64-bit and 32-bit layouts and save as
//! versioned ASCII.

use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use mud_world::lex::{tag_argument, Reader};

use crate::comm::{act, send_editor_help, send_to_char, string_write, TO_ROOM};
use crate::game::Game;
use crate::handler::{atoi, isname, obj_name};
use crate::interpreter::{cmd_is, is_number, one_argument, skip_spaces};

pub const NUM_OF_BOARDS: usize = 7;
pub const MAX_BOARD_MESSAGES: usize = 60;
pub const MAX_MESSAGE_LENGTH: usize = 4096;
pub const INDEX_SIZE: usize = (NUM_OF_BOARDS * MAX_BOARD_MESSAGES) + 5;
/// the marker `d->mail_to` carries for a board write.
pub const BOARD_MAGIC: i64 = 1048575;

/// Board appearance order (NEWEST_AT_TOP FALSE).
const NEWEST_AT_TOP: bool = false;

/// board_info[]: vnum, read, write, remove, filename.
pub struct BoardInfo {
    pub vnum: i32,
    pub read_lvl: u8,
    pub write_lvl: u8,
    pub remove_lvl: u8,
    pub filename: &'static str,
}

pub const BOARD_INFO: [BoardInfo; NUM_OF_BOARDS] = [
    BoardInfo { vnum: 3099, read_lvl: 0, write_lvl: 0, remove_lvl: LVL_GOD, filename: "board.mortal" },
    BoardInfo { vnum: 3098, read_lvl: LVL_IMMORT, write_lvl: LVL_IMMORT, remove_lvl: LVL_GRGOD, filename: "board.immortal" },
    BoardInfo { vnum: 3097, read_lvl: LVL_IMMORT, write_lvl: LVL_GRGOD, remove_lvl: LVL_IMPL, filename: "board.freeze" },
    BoardInfo { vnum: 3096, read_lvl: 0, write_lvl: 0, remove_lvl: LVL_IMMORT, filename: "board.social" },
    BoardInfo { vnum: 1226, read_lvl: 0, write_lvl: 0, remove_lvl: LVL_IMPL, filename: "board.builder" },
    BoardInfo { vnum: 1227, read_lvl: 0, write_lvl: 0, remove_lvl: LVL_IMPL, filename: "board.staff" },
    BoardInfo { vnum: 1228, read_lvl: 0, write_lvl: 0, remove_lvl: LVL_IMPL, filename: "board.advertising" },
];

/// One message index entry.
#[derive(Debug, Clone)]
pub struct MsgInfo {
    pub slot_num: i32,
    pub heading: Option<Vec<u8>>,
    pub level: u8,
}

impl Default for MsgInfo {
    fn default() -> Self {
        Self { slot_num: -1, heading: None, level: 0 }
    }
}

pub struct BoardState {
    /// msg_storage[] / msg_storage_taken[].
    pub storage: Vec<Option<Vec<u8>>>,
    pub taken: Vec<bool>,
    /// msg_index[board][msg] truncated to the live count (num_of_msgs).
    pub msgs: Vec<Vec<MsgInfo>>,
    /// board_info[].rnum, resolved by init_boards.
    pub rnum: [Idx; NUM_OF_BOARDS],
    /// gen_board's `static int loaded`.
    pub loaded: bool,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            storage: vec![None; INDEX_SIZE],
            taken: vec![false; INDEX_SIZE],
            msgs: vec![Vec::new(); NUM_OF_BOARDS],
            rnum: [NOTHING; NUM_OF_BOARDS],
            loaded: false,
        }
    }
}

fn board_path(g: &Game, board: usize) -> std::path::PathBuf {
    g.lib_dir.join("etc").join(BOARD_INFO[board].filename)
}

fn find_slot(g: &mut Game) -> Option<usize> {
    for i in 0..INDEX_SIZE {
        if !g.boards.taken[i] {
            g.boards.taken[i] = true;
            return Some(i);
        }
    }
    None
}

/// find_board: the room's contents first, then an
/// immortal's inventory.
fn find_board(g: &Game, chid: CharId) -> Option<usize> {
    let room = g.ch(chid).in_room;
    if room != NOWHERE {
        for &oid in &g.rooms[room as usize].contents {
            let rnum = g.obj(oid).item_number;
            for i in 0..NUM_OF_BOARDS {
                if g.boards.rnum[i] == rnum && rnum != NOTHING {
                    return Some(i);
                }
            }
        }
    }
    if g.ch(chid).level >= LVL_IMMORT {
        for &oid in &g.ch(chid).carrying {
            let rnum = g.obj(oid).item_number;
            for i in 0..NUM_OF_BOARDS {
                if g.boards.rnum[i] == rnum && rnum != NOTHING {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// init_boards. A missing board vnum is not fatal: F5's policy applies —
/// log and carry on with that board inert.
fn init_boards(g: &mut Game) {
    g.boards.storage = vec![None; INDEX_SIZE];
    g.boards.taken = vec![false; INDEX_SIZE];
    for i in 0..NUM_OF_BOARDS {
        let rnum = g.world.real_object(BOARD_INFO[i].vnum as Idx).unwrap_or(NOTHING);
        g.boards.rnum[i] = rnum;
        if rnum == NOTHING {
            g.log(format!(
                "SYSERR: Fatal board error: board vnum {} does not exist!",
                BOARD_INFO[i].vnum
            ));
        }
        g.boards.msgs[i].clear();
        board_load_board(g, i);
    }
}

// ---------------------------------------------------------------- file I/O

/// The legacy fixed-size binary record
/// layouts. 32 bytes = LP64 (int, pad, char*, int, int, int, pad);
/// 20 bytes = ILP32.
fn parse_binary_board(data: &[u8]) -> Option<Vec<(Vec<u8>, u8, Option<Vec<u8>>)>> {
    if data.len() < 4 {
        return None;
    }
    let count = i32::from_le_bytes(data[0..4].try_into().ok()?);
    if count < 1 || count as usize > MAX_BOARD_MESSAGES {
        return None;
    }
    // Try each candidate record size; the right one consumes the file.
    for &(rec, o_level, o_hlen, o_mlen) in &[(32usize, 16usize, 20usize, 24usize), (20, 8, 12, 16)] {
        let mut pos = 4usize;
        let mut out = Vec::new();
        let mut ok = true;
        for _ in 0..count {
            if pos + rec > data.len() {
                ok = false;
                break;
            }
            let r = &data[pos..pos + rec];
            let rd = |o: usize| i32::from_le_bytes(r[o..o + 4].try_into().unwrap());
            let (level, hlen, mlen) = (rd(o_level), rd(o_hlen), rd(o_mlen));
            pos += rec;
            if hlen <= 0 || pos + hlen as usize > data.len() || mlen < 0 {
                ok = false;
                break;
            }
            let mut heading = data[pos..pos + hlen as usize].to_vec();
            pos += hlen as usize;
            if heading.last() == Some(&0) {
                heading.pop();
            }
            let body = if mlen > 0 {
                if pos + mlen as usize > data.len() {
                    ok = false;
                    break;
                }
                let mut b = data[pos..pos + mlen as usize].to_vec();
                pos += mlen as usize;
                if b.last() == Some(&0) {
                    b.pop();
                }
                Some(b)
            } else {
                None
            };
            out.push((heading, level.clamp(0, LVL_IMPL as i32) as u8, body));
        }
        if ok && out.len() == count as usize {
            return Some(out);
        }
    }
    None
}

fn parse_ascii_board(data: &[u8]) -> Vec<(Vec<u8>, u8, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut r = Reader::new(data);
    let mut heading: Option<Vec<u8>> = None;
    let mut level: u8 = 0;
    while let Some(line) = r.get_line() {
        if line.starts_with(b"$~") {
            break;
        }
        let (tag, value) = tag_argument(&line);
        match tag.as_slice() {
            b"Head" => {
                heading = Some(value);
                level = 0;
            }
            b"Levl" => level = atoi(&value).clamp(0, LVL_IMPL as i32) as u8,
            b"Body" => {
                let body = r.fread_string("board body").ok().flatten();
                if let Some(h) = heading.take() {
                    out.push((h, level, body));
                }
            }
            _ => {}
        }
    }
    out
}

pub fn board_load_board(g: &mut Game, board: usize) {
    let path = board_path(g, board);
    let Ok(data) = std::fs::read(&path) else { return };
    if data.is_empty() {
        return;
    }
    let is_ascii = data.starts_with(b"*") || data.starts_with(b"Head");
    let records = if is_ascii {
        parse_ascii_board(&data)
    } else {
        match parse_binary_board(&data) {
            Some(v) => {
                g.log(format!(
                    "   Converting legacy binary board {} ({} messages) to ASCII.",
                    board,
                    v.len()
                ));
                v
            }
            None => {
                g.log(format!("SYSERR: Board file {} corrupt.  Resetting.", board));
                board_reset_board(g, board);
                return;
            }
        }
    };

    for (heading, level, body) in records {
        if g.boards.msgs[board].len() >= MAX_BOARD_MESSAGES {
            break;
        }
        let Some(slot) = find_slot(g) else {
            g.log(format!("SYSERR: Out of slots booting board {}!  Resetting...", board));
            board_reset_board(g, board);
            return;
        };
        g.boards.storage[slot] = body;
        g.boards.msgs[board].push(MsgInfo { slot_num: slot as i32, heading: Some(heading), level });
    }
    if !is_ascii {
        board_save_board(g, board);
    }
}

/// board_save_board, D3-style: versioned ASCII, and an
/// empty board still deletes its file.
pub fn board_save_board(g: &mut Game, board: usize) {
    let path = board_path(g, board);
    if g.boards.msgs[board].is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let mut out = b"* tbaMUD board file (ASCII v1)\n".to_vec();
    for i in 0..g.boards.msgs[board].len() {
        let m = &g.boards.msgs[board][i];
        let slot = m.slot_num;
        let heading = m.heading.clone().unwrap_or_default();
        let level = m.level;
        out.extend_from_slice(b"Head: ");
        out.extend_from_slice(&heading);
        out.push(b'\n');
        out.extend_from_slice(format!("Levl: {}\n", level).as_bytes());
        out.extend_from_slice(b"Body:\n");
        if slot >= 0 && (slot as usize) < INDEX_SIZE {
            if let Some(text) = &g.boards.storage[slot as usize] {
                out.extend_from_slice(text);
            }
        }
        out.extend_from_slice(b"~\n");
    }
    out.extend_from_slice(b"$~\n");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, &out) {
        g.log(format!("SYSERR: Error writing board: {}", e));
    }
}

pub fn board_clear_board(g: &mut Game, board: usize) {
    for m in std::mem::take(&mut g.boards.msgs[board]) {
        if m.slot_num < 0 {
            continue;
        }
        let slot = m.slot_num as usize;
        if slot < INDEX_SIZE {
            g.boards.storage[slot] = None;
            g.boards.taken[slot] = false;
        }
    }
}

fn board_reset_board(g: &mut Game, board: usize) {
    board_clear_board(g, board);
    let _ = std::fs::remove_file(board_path(g, board));
}

pub fn board_clear_all(g: &mut Game) {
    for i in 0..NUM_OF_BOARDS {
        board_clear_board(g, i);
    }
}

// ---------------------------------------------------------------- the proc

/// gen_board. Boards initialize on first use, gated by the `loaded`
/// flag.
pub fn gen_board(g: &mut Game, chid: CharId, board_obj: ObjId, cmd: usize, arg: &[u8]) -> bool {
    if !g.boards.loaded {
        init_boards(g);
        g.boards.loaded = true;
    }
    if g.ch(chid).desc.is_none() {
        return false;
    }

    let is_write = cmd_is(g, cmd, b"write");
    let is_look = cmd_is(g, cmd, b"look");
    let is_examine = cmd_is(g, cmd, b"examine");
    let is_read = cmd_is(g, cmd, b"read");
    let is_remove = cmd_is(g, cmd, b"remove");
    if !(is_write || is_look || is_examine || is_read || is_remove) {
        return false;
    }

    let Some(board) = find_board(g, chid) else {
        g.log("SYSERR:  degenerate board!  (what the hell...)".to_string());
        return false;
    };

    if is_write {
        board_write_message(g, board, chid, arg)
    } else if is_look || is_examine {
        board_show_board(g, board, chid, arg, board_obj)
    } else if is_read {
        board_display_msg(g, board, chid, arg, board_obj)
    } else {
        board_remove_msg(g, board, chid, arg)
    }
}

fn board_write_message(g: &mut Game, board: usize, chid: CharId, arg: &[u8]) -> bool {
    if g.ch(chid).level < BOARD_INFO[board].write_lvl {
        send_to_char(g, chid, b"You are not holy enough to write on this board.\r\n");
        return true;
    }
    if g.boards.msgs[board].len() >= MAX_BOARD_MESSAGES {
        send_to_char(g, chid, b"The board is full.\r\n");
        return true;
    }
    let Some(slot) = find_slot(g) else {
        send_to_char(g, chid, b"The board is malfunctioning - sorry.\r\n");
        g.log("SYSERR: Board: failed to find empty slot on write.".to_string());
        return true;
    };

    let mut arg = skip_spaces(arg).to_vec();
    mud_net::editor::delete_doubledollar(&mut arg);
    // JE: truncate the headline at 80 characters.
    arg.truncate(80);

    if arg.is_empty() {
        send_to_char(g, chid, b"We must have a headline!\r\n");
        return true;
    }

    let tmstr = crate::act::wizard::strftime_date(g.now, g.tz_offset_secs);
    let mut paren = b"(".to_vec();
    paren.extend_from_slice(g.ch(chid).get_name());
    paren.push(b')');
    let mut heading = tmstr.into_bytes();
    heading.push(b' ');
    // "%-12s" over the parenthesized name.
    heading.extend_from_slice(&paren);
    for _ in paren.len()..12 {
        heading.push(b' ');
    }
    heading.extend_from_slice(b" :: ");
    heading.extend_from_slice(&arg);

    let level = g.ch(chid).level;
    g.boards.msgs[board].push(MsgInfo { slot_num: slot as i32, heading: Some(heading), level });

    send_to_char(g, chid, b"Write your message.\r\n");
    send_editor_help(g, chid);
    act(g, b"$n starts to write a message.", true, Some(chid), None, None, TO_ROOM);

    string_write(g, chid, MAX_MESSAGE_LENGTH, BOARD_MAGIC + board as i64, None);
    if let Some(di) = g.ch(chid).desc {
        if let Some(d) = g.descriptors.get_mut(di) {
            if let Some(s) = d.editing.as_mut() {
                s.str_slot = slot as i32;
            }
        }
    }
    true
}

fn board_show_board(
    g: &mut Game,
    board: usize,
    chid: CharId,
    arg: &[u8],
    board_obj: ObjId,
) -> bool {
    if g.ch(chid).desc.is_none() {
        return false;
    }
    let (tmp, _) = one_argument(arg);
    if tmp.is_empty() || !isname(&tmp, obj_name(g, board_obj)) {
        return false;
    }
    if g.ch(chid).level < BOARD_INFO[board].read_lvl {
        send_to_char(g, chid, b"You try but fail to understand the holy words.\r\n");
        return true;
    }
    act(g, b"$n studies the board.", true, Some(chid), None, None, TO_ROOM);

    let count = g.boards.msgs[board].len();
    if count == 0 {
        send_to_char(
            g,
            chid,
            b"This is a bulletin board.  Usage: READ/REMOVE <messg #>, WRITE <header>.\r\nThe board is empty.\r\n",
        );
        return true;
    }

    let mut buf = format!(
        "This is a bulletin board.  Usage: READ/REMOVE <messg #>, WRITE <header>.\r\nYou will need to look at the board to save your message.\r\nThere are {} messages on the board.\r\n",
        count
    )
    .into_bytes();
    let order: Vec<usize> =
        if NEWEST_AT_TOP { (0..count).rev().collect() } else { (0..count).collect() };
    for i in order {
        let Some(h) = g.boards.msgs[board][i].heading.clone() else {
            g.log(format!("SYSERR: Board {} is fubar'd.", board));
            send_to_char(g, chid, b"Sorry, the board isn't working.\r\n");
            return true;
        };
        let n = if NEWEST_AT_TOP { count - i } else { i + 1 };
        buf.extend_from_slice(format!("{:<2} : ", n).as_bytes());
        buf.extend_from_slice(&h);
        buf.extend_from_slice(b"\r\n");
    }
    crate::act::informative::page_string(g, chid, &buf);
    true
}

fn board_display_msg(
    g: &mut Game,
    board: usize,
    chid: CharId,
    arg: &[u8],
    board_obj: ObjId,
) -> bool {
    let (number, _) = one_argument(arg);
    if number.is_empty() {
        return false;
    }
    if isname(&number, obj_name(g, board_obj)) {
        // so "read board" works
        return board_show_board(g, board, chid, arg, board_obj);
    }
    if !is_number(&number) {
        return false; // read 2.mail, look 2.sword
    }
    let msg = atoi(&number);
    if msg == 0 {
        return false;
    }
    if g.ch(chid).level < BOARD_INFO[board].read_lvl {
        send_to_char(g, chid, b"You try but fail to understand the holy words.\r\n");
        return true;
    }
    let count = g.boards.msgs[board].len();
    if count == 0 {
        send_to_char(g, chid, b"The board is empty!\r\n");
        return true;
    }
    if msg < 1 || msg as usize > count {
        send_to_char(g, chid, b"That message exists only in your imagination.\r\n");
        return true;
    }
    let ind = if NEWEST_AT_TOP { count - msg as usize } else { msg as usize - 1 };
    let slot = g.boards.msgs[board][ind].slot_num;
    if slot < 0 || slot as usize >= INDEX_SIZE {
        send_to_char(g, chid, b"Sorry, the board is not working.\r\n");
        let room = g.ch(chid).in_room;
        let vnum = if room == NOWHERE { NOWHERE as u16 } else { g.world.rooms[room as usize].vnum };
        g.log(format!("SYSERR: Board is screwed up. (Room #{})", vnum));
        return true;
    }
    let Some(heading) = g.boards.msgs[board][ind].heading.clone() else {
        send_to_char(g, chid, b"That message appears to be screwed up.\r\n");
        return true;
    };
    let Some(text) = g.boards.storage[slot as usize].clone() else {
        send_to_char(g, chid, b"That message seems to be empty.\r\n");
        return true;
    };

    let mut buf = format!("Message {} : ", msg).into_bytes();
    buf.extend_from_slice(&heading);
    buf.extend_from_slice(b"\r\n\r\n");
    buf.extend_from_slice(&text);
    buf.extend_from_slice(b"\r\n");
    crate::act::informative::page_string(g, chid, &buf);
    true
}

fn board_remove_msg(g: &mut Game, board: usize, chid: CharId, arg: &[u8]) -> bool {
    let (number, _) = one_argument(arg);
    if number.is_empty() || !is_number(&number) {
        return false;
    }
    let msg = atoi(&number);
    if msg == 0 {
        return false;
    }
    let count = g.boards.msgs[board].len();
    if count == 0 {
        send_to_char(g, chid, b"The board is empty!\r\n");
        return true;
    }
    if msg < 1 || msg as usize > count {
        send_to_char(g, chid, b"That message exists only in your imagination.\r\n");
        return true;
    }
    let ind = if NEWEST_AT_TOP { count - msg as usize } else { msg as usize - 1 };
    let Some(heading) = g.boards.msgs[board][ind].heading.clone() else {
        send_to_char(g, chid, b"That message appears to be screwed up.\r\n");
        return true;
    };

    let mut paren = b"(".to_vec();
    paren.extend_from_slice(g.ch(chid).get_name());
    paren.push(b')');
    let mine = heading.windows(paren.len()).any(|w| w == &paren[..]);
    if g.ch(chid).level < BOARD_INFO[board].remove_lvl && !mine {
        send_to_char(g, chid, b"You are not holy enough to remove other people's messages.\r\n");
        return true;
    }
    if g.ch(chid).level < g.boards.msgs[board][ind].level {
        send_to_char(g, chid, b"You can't remove a message holier than yourself.\r\n");
        return true;
    }
    let slot = g.boards.msgs[board][ind].slot_num;
    if slot < 0 || slot as usize >= INDEX_SIZE {
        send_to_char(g, chid, b"That message is majorly screwed up.\r\n");
        let room = g.ch(chid).in_room;
        let vnum = if room == NOWHERE { NOWHERE as u16 } else { g.world.rooms[room as usize].vnum };
        g.log(format!("SYSERR: The board is seriously screwed up. (Room #{})", vnum));
        return true;
    }
    // Someone still writing into that slot?
    let busy = g.descriptors.indices().into_iter().any(|di| {
        g.descriptors.get(di).is_some_and(|d| {
            d.state == ConState::Playing
                && d.editing.as_ref().is_some_and(|s| s.str_slot == slot)
        })
    });
    if busy {
        send_to_char(
            g,
            chid,
            b"At least wait until the author is finished before removing it!\r\n",
        );
        return true;
    }

    g.boards.storage[slot as usize] = None;
    g.boards.taken[slot as usize] = false;
    g.boards.msgs[board].remove(ind);

    send_to_char(g, chid, b"Message removed.\r\n");
    let m = format!("$n just removed message {}.", msg);
    act(g, m.as_bytes(), false, Some(chid), None, None, TO_ROOM);
    board_save_board(g, board);
    true
}

/// The editor's board half of playing_string_cleanup.
pub fn board_finish_write(g: &mut Game, chid: CharId, slot: i32, board: usize, text: Option<Vec<u8>>) {
    if slot >= 0 && (slot as usize) < INDEX_SIZE {
        g.boards.storage[slot as usize] = text;
    }
    if board < NUM_OF_BOARDS {
        board_save_board(g, board);
    }
    let _ = chid;
}
