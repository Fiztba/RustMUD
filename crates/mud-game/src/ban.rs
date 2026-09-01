//! The site ban list (`lib/etc/badsites`) and the three ban tiers.
//!
//! The list is a plain-text table of four whitespace-separated fields,
//! so it round-trips as-is. It is held newest-first and written in reverse,
//! which puts the file back on disk **oldest-first**.

use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::send_to_char;
use crate::game::{Game, MudlogKind};
use crate::interpreter::{one_argument, two_arguments};

pub const BAN_NOT: i32 = 0;
pub const BAN_NEW: i32 = 1;
pub const BAN_SELECT: i32 = 2;
pub const BAN_ALL: i32 = 3;

pub const BAN_TYPES: [&str; 5] = ["no", "new", "select", "all", "ERROR"];

/// One `ban_list_element`.
#[derive(Debug, Clone)]
pub struct BanEntry {
    pub site: BStr,
    pub type_: i32,
    pub date: i64,
    pub name: BStr,
}

fn ban_path(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("etc").join("badsites")
}

/// load_banned. Records are prepended, so the in-memory order is the
/// reverse of the file's.
pub fn load_banned(lib: &std::path::Path, log: &mut Vec<String>) -> Vec<BanEntry> {
    let path = lib.join("etc").join("badsites");
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log.push(format!("   Ban file '{}' doesn't exist.", path.display()));
            return Vec::new();
        }
        Err(e) => {
            log.push(format!("SYSERR: Unable to open banfile '{}': {}", path.display(), e));
            return Vec::new();
        }
    };
    // The four fields are whitespace-delimited, and a record may span lines.
    let mut fields = data.split(|c: &u8| c.is_ascii_whitespace()).filter(|f| !f.is_empty());
    let mut out: Vec<BanEntry> = Vec::new();
    loop {
        let (Some(ty), Some(site), Some(date), Some(name)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            break;
        };
        let mut site = site.to_vec();
        site.truncate(BANNED_SITE_LENGTH);
        let mut name = name.to_vec();
        name.truncate(MAX_NAME_LENGTH);
        // Type stays at 0 when the tag matches nothing.
        let type_ = (BAN_NOT..=BAN_ALL)
            .find(|&i| BAN_TYPES[i as usize].as_bytes() == ty)
            .unwrap_or(0);
        out.insert(0, BanEntry { site, type_, date: crate::handler::atoi(date) as i64, name });
    }
    out
}

/// write_ban_list — tail-first recursion = oldest-first file.
fn write_ban_list(g: &mut Game) {
    let mut out = Vec::new();
    for node in g.ban_list.iter().rev() {
        out.extend_from_slice(BAN_TYPES[node.type_ as usize].as_bytes());
        out.push(b' ');
        out.extend_from_slice(&node.site);
        out.push(b' ');
        out.extend_from_slice(node.date.to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(&node.name);
        out.push(b'\n');
    }
    let path = ban_path(g);
    if let Err(e) = std::fs::write(&path, &out) {
        g.log(format!("SYSERR: Unable to open '{}' for writing: {}", path.display(), e));
    }
}

/// isbanned: substring match, highest tier wins.
pub fn isbanned(g: &Game, hostname: &[u8]) -> i32 {
    if hostname.is_empty() {
        return 0;
    }
    let host = hostname.to_ascii_lowercase();
    let mut i = 0;
    for node in &g.ban_list {
        // strstr(hostname, "") is non-NULL: an empty site matches everything.
        let hit = node.site.is_empty()
            || (host.len() >= node.site.len()
                && host.windows(node.site.len()).any(|w| w == &node.site[..]));
        if hit {
            i = i.max(node.type_);
        }
    }
    i
}

fn ban_row(site: &[u8], ty: &[u8], on: &[u8], by: &[u8]) -> BStr {
    // "%-25.25s %-8.8s %-15.15s %-16.16s\r\n"
    let mut out = crate::act::pad_right_trunc(site, 25);
    out.extend_from_slice(b"  ");
    out.extend_from_slice(&crate::act::pad_right_trunc(ty, 8));
    out.extend_from_slice(b"  ");
    out.extend_from_slice(&crate::act::pad_right_trunc(on, 15));
    out.extend_from_slice(b"  ");
    out.extend_from_slice(&crate::act::pad_right_trunc(by, 16));
    out.extend_from_slice(b"\r\n");
    out
}

pub fn do_ban(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if argument.is_empty() {
        if g.ban_list.is_empty() {
            send_to_char(g, chid, b"No sites are banned.\r\n");
            return;
        }
        let hdr = ban_row(b"Banned Site Name", b"Ban Type", b"Banned On", b"Banned By");
        send_to_char(g, chid, &hdr);
        let dashes = b"---------------------------------";
        let row = ban_row(dashes, dashes, dashes, dashes);
        send_to_char(g, chid, &row);
        let rows: Vec<BStr> = g
            .ban_list
            .iter()
            .map(|n| {
                let timestr = if n.date != 0 {
                    crate::act::wizard::strftime_date(n.date, g.tz_offset_secs).into_bytes()
                } else {
                    b"Unknown".to_vec()
                };
                ban_row(&n.site, BAN_TYPES[n.type_ as usize].as_bytes(), &timestr, &n.name)
            })
            .collect();
        for r in rows {
            send_to_char(g, chid, &r);
        }
        return;
    }

    let (flag, site, _) = two_arguments(argument);
    if site.is_empty() || flag.is_empty() {
        send_to_char(g, chid, b"Usage: ban {all | select | new} site_name\r\n");
        return;
    }
    let flag_l = flag.to_ascii_lowercase();
    if !matches!(&flag_l[..], b"select" | b"all" | b"new") {
        send_to_char(g, chid, b"Flag must be ALL, SELECT, or NEW.\r\n");
        return;
    }
    if g.ban_list.iter().any(|n| n.site.eq_ignore_ascii_case(&site)) {
        send_to_char(
            g,
            chid,
            b"That site has already been banned -- unban it to change the ban type.\r\n",
        );
        return;
    }

    let mut ban_site = site.clone();
    ban_site.truncate(BANNED_SITE_LENGTH);
    ban_site.make_ascii_lowercase();
    let mut name = g.ch(chid).get_name().to_vec();
    name.truncate(MAX_NAME_LENGTH);
    // Scans BAN_NEW..=BAN_ALL; the flag is already known to be one of them.
    let type_ = (BAN_NEW..=BAN_ALL)
        .find(|&i| BAN_TYPES[i as usize].as_bytes() == &flag_l[..])
        .unwrap_or(0);
    let date = g.now;
    g.ban_list.insert(0, BanEntry { site: ban_site, type_, date, name });

    let who = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Nrm,
        (LVL_GOD as i16).max(invis) as u8,
        true,
        &format!(
            "{} has banned {} for {} players.",
            who,
            String::from_utf8_lossy(&site),
            BAN_TYPES[type_ as usize]
        ),
    );
    send_to_char(g, chid, b"Site banned.\r\n");
    write_ban_list(g);
}

pub fn do_unban(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (site, _) = one_argument(argument);
    if site.is_empty() {
        send_to_char(g, chid, b"A site to unban might help.\r\n");
        return;
    }
    let Some(pos) = g.ban_list.iter().position(|n| n.site.eq_ignore_ascii_case(&site)) else {
        send_to_char(g, chid, b"That site is not currently banned.\r\n");
        return;
    };
    let node = g.ban_list.remove(pos);
    send_to_char(g, chid, b"Site unbanned.\r\n");
    let who = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Nrm,
        (LVL_GOD as i16).max(invis) as u8,
        true,
        &format!(
            "{} removed the {}-player ban on {}.",
            who,
            BAN_TYPES[node.type_ as usize],
            String::from_utf8_lossy(&node.site)
        ),
    );
    write_ban_list(g);
}
