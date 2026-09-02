//! The mudmail system and the postmaster spec-proc.
//!
//! `lib/etc/plrmail` is a flat ASCII log: one `### <to> <from> <time>` header
//! per message followed by a `~`-terminated body. Reading a message rewrites
//! the whole file minus the first record addressed to the reader, which is
//! why `has_mail` re-scans from the top every call.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use mud_world::lex::Reader;

use crate::comm::{act, send_to_char, TO_ROOM, TO_VICT};
use crate::game::Game;
use crate::interpreter::{cmd_is, one_argument};

pub const MIN_MAIL_LEVEL: u8 = 1;
pub const STAMP_PRICE: i32 = 150;
pub const MAX_MAIL_SIZE: usize = 8192;

/// struct mail_t.
struct MailRecord {
    recipient: i64,
    sender: i64,
    sent_time: i64,
    body: Option<Vec<u8>>,
}

fn mail_file(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("etc").join("plrmail")
}

fn mail_file_tmp(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("etc").join("plrmail_tmp")
}

/// read_mail_record. `None` at EOF *or* on a malformed header — reading
/// stops either way.
fn read_mail_record(r: &mut Reader, log: &mut Vec<String>) -> Option<MailRecord> {
    let line = r.get_line()?;
    let fields: Vec<&[u8]> = line.split(|b| *b == b' ').filter(|f| !f.is_empty()).collect();
    let parse = |b: &[u8]| -> Option<i64> { std::str::from_utf8(b).ok()?.trim().parse().ok() };
    if fields.len() < 4 || fields[0] != b"###" {
        log.push("Mail system - fatal error - malformed mail header".to_string());
        log.push(format!("Line was: {}", String::from_utf8_lossy(&line)));
        return None;
    }
    let (Some(recipient), Some(sender), Some(sent_time)) =
        (parse(fields[1]), parse(fields[2]), parse(fields[3]))
    else {
        log.push("Mail system - fatal error - malformed mail header".to_string());
        log.push(format!("Line was: {}", String::from_utf8_lossy(&line)));
        return None;
    };
    let body = r.fread_string("read mail record").ok().flatten();
    Some(MailRecord { recipient, sender, sent_time, body })
}

fn write_mail_record(out: &mut Vec<u8>, rec: &MailRecord) {
    out.extend_from_slice(
        format!("### {} {} {}\n", rec.recipient, rec.sender, rec.sent_time).as_bytes(),
    );
    out.extend_from_slice(rec.body.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"~\n");
}

fn read_all(g: &mut Game) -> Option<Vec<MailRecord>> {
    let data = std::fs::read(mail_file(g)).ok()?;
    let mut log = Vec::new();
    let mut out = Vec::new();
    {
        let mut r = Reader::new(&data);
        while let Some(rec) = read_mail_record(&mut r, &mut log) {
            out.push(rec);
        }
    }
    for line in log {
        g.log(line);
    }
    Some(out)
}

/// scan_file: the boot-time index pass. A missing file is
/// created empty and is not an error.
pub fn scan_file(g: &mut Game) -> bool {
    let path = mail_file(g);
    if !path.exists() {
        g.log("   Mail file non-existant... creating new file.".to_string());
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, b"");
        return true;
    }
    let count = read_all(g).map_or(0, |v| v.len());
    g.log(format!("   Mail file read -- {} messages.", count));
    true
}

pub fn has_mail(g: &mut Game, recipient: i64) -> bool {
    match read_all(g) {
        Some(v) => v.iter().any(|r| r.recipient == recipient),
        None => {
            g.log("read_delete: Mail file not accessible.".to_string());
            false
        }
    }
}

/// store_mail: append one record.
pub fn store_mail(g: &mut Game, to: i64, from: i64, message: Vec<u8>) {
    let mut out = Vec::new();
    write_mail_record(
        &mut out,
        &MailRecord { recipient: to, sender: from, sent_time: g.now, body: Some(message) },
    );
    let path = mail_file(g);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let appended = match std::fs::read(&path) {
        Ok(mut old) => {
            old.extend_from_slice(&out);
            old
        }
        Err(_) => out,
    };
    if std::fs::write(&path, &appended).is_err() {
        g.log("store_mail: Mail file not accessible.".to_string());
    }
}

/// read_delete: pull the first message for `recipient`,
/// rewriting the file without it, and render it as the note's text.
pub fn read_delete(g: &mut Game, recipient: i64) -> Vec<u8> {
    let Some(all) = read_all(g) else {
        g.log("read_delete: Mail file not accessible.".to_string());
        return b"Mail system malfunction - please report this".to_vec();
    };

    let mut keep: Option<MailRecord> = None;
    let mut rest = Vec::new();
    for rec in all {
        if keep.is_none() && rec.recipient == recipient {
            keep = Some(rec);
            continue;
        }
        rest.push(rec);
    }

    let buf = match &keep {
        None => b"Mail system error - please report".to_vec(),
        Some(rec) => {
            let timestr = crate::act::wizard::ctime_like(rec.sent_time, g.tz_offset_secs);
            let from = crate::players_glue::get_name_by_id(g, rec.sender);
            let to = crate::players_glue::get_name_by_id(g, rec.recipient);
            let mut buf = b" * * * * tbaMUD Mail System * * * *\r\nDate: ".to_vec();
            buf.extend_from_slice(timestr.as_bytes());
            buf.extend_from_slice(b"\r\nTo  : ");
            buf.extend_from_slice(to.as_deref().unwrap_or(b"Unknown"));
            buf.extend_from_slice(b"\r\nFrom: ");
            buf.extend_from_slice(from.as_deref().unwrap_or(b"Unknown"));
            buf.extend_from_slice(b"\r\n\r\n");
            buf.extend_from_slice(rec.body.as_deref().unwrap_or(b"No message"));
            buf
        }
    };

    let mut out = Vec::new();
    for rec in &rest {
        write_mail_record(&mut out, rec);
    }
    // Survivors go to plrmail_tmp, then plrmail is removed and the temp
    // renamed over it. The temp file is left behind only on failure.
    let tmp = mail_file_tmp(g);
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::remove_file(mail_file(g));
        let _ = std::fs::rename(&tmp, mail_file(g));
    } else {
        g.log("read_delete: new Mail file not accessible.".to_string());
    }
    buf
}

pub fn notify_if_playing(g: &mut Game, from: CharId, recipient_id: i64) {
    let name = g.ch(from).get_name().to_vec();
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        // IS_PLAYING: the notice reaches someone sitting in an OLC editor too.
        if !d.is_playing() {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.idnum != recipient_id {
            continue;
        }
        if has_mail(g, recipient_id) {
            let mut msg = b"You have new mudmail from ".to_vec();
            msg.extend_from_slice(&name);
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
        }
    }
}

fn mail_recip_ok(g: &Game, name: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    g.player_table
        .iter()
        .any(|p| p.name == lower && p.flags & crate::game::PINDEX_DELETED == 0)
}

pub fn postmaster(g: &mut Game, chid: CharId, mailman: CharId, cmd: usize, arg: &[u8]) -> bool {
    if g.ch(chid).desc.is_none() || g.ch(chid).is_npc() {
        return false;
    }
    if !(cmd_is(g, cmd, b"mail") || cmd_is(g, cmd, b"check") || cmd_is(g, cmd, b"receive")) {
        return false;
    }
    if g.no_mail {
        send_to_char(g, chid, b"Sorry, the mail system is having technical difficulties.\r\n");
        return false;
    }

    if cmd_is(g, cmd, b"mail") {
        postmaster_send_mail(g, chid, mailman, arg);
        true
    } else if cmd_is(g, cmd, b"check") {
        postmaster_check_mail(g, chid, mailman);
        true
    } else if cmd_is(g, cmd, b"receive") {
        postmaster_receive_mail(g, chid, mailman);
        true
    } else {
        false
    }
}

fn postmaster_send_mail(g: &mut Game, chid: CharId, mailman: CharId, arg: &[u8]) {
    if g.ch(chid).level < MIN_MAIL_LEVEL {
        let msg =
            format!("$n tells you, 'Sorry, you have to be level {} to send mail!'", MIN_MAIL_LEVEL);
        act(g, msg.as_bytes(), false, Some(mailman), None, Some(chid), TO_VICT);
        return;
    }
    let (buf, _) = one_argument(arg);
    if buf.is_empty() {
        act(
            g,
            b"$n tells you, 'You need to specify an addressee!'",
            false,
            Some(mailman),
            None,
            Some(chid),
            TO_VICT,
        );
        return;
    }
    if g.ch(chid).points.gold < STAMP_PRICE && g.ch(chid).level < LVL_IMMORT {
        let msg = format!(
            "$n tells you, 'A stamp costs {} coin{}.'\r\n$n tells you, '...which I see you can't afford.'",
            STAMP_PRICE,
            if STAMP_PRICE == 1 { "" } else { "s" }
        );
        act(g, msg.as_bytes(), false, Some(mailman), None, Some(chid), TO_VICT);
        return;
    }
    let recipient = crate::players_glue::get_id_by_name(g, &buf);
    let ok = recipient.is_some_and(|id| id >= 0) && mail_recip_ok(g, &buf);
    if !ok {
        act(
            g,
            b"$n tells you, 'No one by that name is registered here!'",
            false,
            Some(mailman),
            None,
            Some(chid),
            TO_VICT,
        );
        return;
    }
    let recipient = recipient.unwrap_or(-1);

    act(g, b"$n starts to write some mail.", true, Some(chid), None, None, TO_ROOM);

    if g.ch(chid).level < LVL_IMMORT {
        let msg = format!("$n tells you, 'I'll take {} coins for the stamp.'", STAMP_PRICE);
        act(g, msg.as_bytes(), false, Some(mailman), None, Some(chid), TO_VICT);
        crate::limits::decrease_gold(g, chid, STAMP_PRICE);
    }

    act(
        g,
        b"$n tells you, 'Write your message. (/s saves /h for help).'",
        false,
        Some(mailman),
        None,
        Some(chid),
        TO_VICT,
    );

    g.ch_mut(chid).act.set(flags::PLR_MAILING);
    crate::comm::string_write(g, chid, MAX_MAIL_SIZE, recipient, None);
}

fn postmaster_check_mail(g: &mut Game, chid: CharId, mailman: CharId) {
    let id = g.ch(chid).idnum;
    let msg: &[u8] = if has_mail(g, id) {
        b"$n tells you, 'You have mail waiting.'"
    } else {
        b"$n tells you, 'Sorry, you don't have any mail waiting.'"
    };
    act(g, msg, false, Some(mailman), None, Some(chid), TO_VICT);
}

/// postmaster_receive_mail. Every waiting letter becomes a
/// fresh ITEM_NOTE. The object carries `item_number = 1` rather than
/// NOTHING, so the note is saved and reloaded as prototype rnum 1 with
/// every string overridden. Deliberate.
fn postmaster_receive_mail(g: &mut Game, chid: CharId, mailman: CharId) {
    let id = g.ch(chid).idnum;
    if !has_mail(g, id) {
        act(
            g,
            b"$n tells you, 'Sorry, you don't have any mail waiting.'",
            false,
            Some(mailman),
            None,
            Some(chid),
            TO_VICT,
        );
        return;
    }
    while has_mail(g, id) {
        let text = read_delete(g, id);
        let mut obj = crate::obj::create_obj();
        obj.item_number = 1;
        obj.name = Some(b"mail paper letter".to_vec());
        obj.short_description = Some(b"a piece of mail".to_vec());
        obj.description = Some(b"Someone has left a piece of mail here.".to_vec());
        obj.type_flag = flags::ITEM_NOTE;
        obj.wear_flags = mud_data::flags::FlagSet::default();
        obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
        obj.weight = 1;
        obj.cost = 30;
        obj.cost_per_day = 10;
        obj.action_description = Some(text);

        let oid = g.objs.insert(obj);
        g.object_list.push_front(oid);
        crate::handler::obj_to_char(g, oid, chid);

        act(g, b"$n gives you a piece of mail.", false, Some(mailman), None, Some(chid), TO_VICT);
        act(g, b"$N gives $n a piece of mail.", false, Some(chid), None, Some(mailman), TO_ROOM);
    }
}
