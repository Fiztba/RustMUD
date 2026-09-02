//! The immortal command suite: movement (goto/at/trans/
//! teleport), the world tools (load/purge/vnum/zreset/zpurge/zlock), player
//! administration (advance/restore/wizutil/snoop/switch/force/dc), the
//! channels (echo/send/gecho/wiznet), and the server controls (wizlock,
//! shutdown, copyover, autowiz). `stat`, `set` and `show` live in the
//! sibling `wizstat`/`wizset`/`wizshow` modules.

use mud_data::flags::{self};
use mud_data::tables;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::{self, act, cc, send_to_char, C_NRM, C_SPR, KCYN, KGRN, KNRM, KYEL};
use crate::game::{Game, MudlogKind};
use crate::handler::{
    atoi, char_from_room, char_to_room, get_char_world_vis, get_number, get_obj_vis_counted,
    is_abbrev,
};
use crate::dg::mobcmd::real_zone_by_thing;
use crate::interpreter::{half_chop, is_number, one_argument, two_arguments};

pub fn find_target_room(g: &mut Game, chid: CharId, rawroomstr: &[u8]) -> Option<RoomRnum> {
    let (roomstr, _) = one_argument(rawroomstr);
    if roomstr.is_empty() {
        send_to_char(g, chid, b"You must supply a room number or name.\r\n");
        return None;
    }
    let location: Option<RoomRnum> = if roomstr[0].is_ascii_digit() && !roomstr.contains(&b'.') {
        match g.real_room(crate::handler::atoi(&roomstr)) {
            Some(r) => Some(r),
            None => {
                send_to_char(g, chid, b"No room exists with that number.\r\n");
                return None;
            }
        }
    } else {
        let (num, name) = get_number(&roomstr);
        let mut count = num;
        if let Some(target) = get_char_world_vis(g, chid, &name, Some(num)) {
            let room = g.ch(target).in_room;
            if room == NOWHERE {
                send_to_char(g, chid, b"That character is currently lost.\r\n");
                return None;
            }
            Some(room)
        } else if let Some(oid) = get_obj_vis_counted(g, chid, &name, &mut count) {
            let o = g.obj(oid);
            let mut location = NOWHERE;
            if o.in_room != NOWHERE {
                location = o.in_room;
            } else if let Some(c) = o.carried_by.and_then(|c| g.try_ch(c)) {
                if c.in_room != NOWHERE {
                    location = c.in_room;
                }
            } else if let Some(c) = o.worn_by.and_then(|c| g.try_ch(c)) {
                if c.in_room != NOWHERE {
                    location = c.in_room;
                }
            }
            if location == NOWHERE {
                send_to_char(g, chid, b"That object is currently not in a room.\r\n");
                return None;
            }
            Some(location)
        } else {
            send_to_char(g, chid, b"Nothing exists by that name.\r\n");
            return None;
        }
    };
    let location = location?;
    if g.ch(chid).level >= LVL_GRGOD {
        return Some(location);
    }
    let room_flag =
        |bit: usize| g.world.rooms[location as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0;
    if room_flag(flags::ROOM_GODROOM) {
        send_to_char(g, chid, b"You are not godly enough to use that room!\r\n");
    } else if room_flag(flags::ROOM_PRIVATE) && g.rooms[location as usize].people.len() > 1 {
        send_to_char(g, chid, b"There's a private conversation going on in that room.\r\n");
    } else if room_flag(flags::ROOM_HOUSE)
        && !crate::house::house_can_enter(g, chid, g.world.rooms[location as usize].vnum as i32)
    {
        send_to_char(g, chid, b"That's private property -- no trespassing!\r\n");
    } else {
        return Some(location);
    }
    None
}

pub fn do_goto(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(location) = find_target_room(g, chid, argument) else { return };
    let zone = g.world.rooms[location as usize].zone as usize;
    let zone_noimmort = g.world.zones[zone].zone_flags[flags::ZONE_NOIMMORT / 32]
        & (1 << (flags::ZONE_NOIMMORT % 32))
        != 0;
    let level = g.ch(chid).level;
    if zone_noimmort && level >= LVL_IMMORT && level < LVL_GRGOD {
        send_to_char(g, chid, b"Sorry, that zone is off-limits for immortals!\r\n");
        return;
    }
    let poofout = g.ch(chid).ps().poofout.clone();
    let mut msg = b"$n ".to_vec();
    msg.extend_from_slice(poofout.as_deref().unwrap_or(b"disappears in a puff of smoke."));
    act(g, &msg, true, Some(chid), None, None, comm::TO_ROOM);

    char_from_room(g, chid);
    char_to_room(g, chid, location);

    let poofin = g.ch(chid).ps().poofin.clone();
    let mut msg = b"$n ".to_vec();
    msg.extend_from_slice(poofin.as_deref().unwrap_or(b"appears with an ear-splitting bang."));
    act(g, &msg, true, Some(chid), None, None, comm::TO_ROOM);

    crate::act::informative::look_at_room(g, chid, false);
    let room = g.ch(chid).in_room;
    crate::dg::triggers::enter_wtrigger(g, room, chid, -1);
}

pub fn do_at(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, command) = half_chop(argument);
    if buf.is_empty() {
        send_to_char(g, chid, b"You must supply a room number or a name.\r\n");
        return;
    }
    if command.is_empty() {
        send_to_char(g, chid, b"What do you want to do there?\r\n");
        return;
    }
    let Some(location) = find_target_room(g, chid, &buf) else { return };
    let original_loc = g.ch(chid).in_room;
    char_from_room(g, chid);
    char_to_room(g, chid, location);
    crate::interpreter::command_interpreter(g, chid, &command);
    // Check the char is still there.
    if g.try_ch(chid).is_some() && g.ch(chid).in_room == location {
        char_from_room(g, chid);
        char_to_room(g, chid, original_loc);
    }
}

/// do_date / uptime.
pub fn do_date(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_DATE;
    let mytime = if subcmd == SCMD_DATE { g.now } else { g.boot_time };
    let timestr = ctime_like(mytime, g.tz_offset_secs);
    if subcmd == SCMD_DATE {
        let mut msg = b"Current machine time: ".to_vec();
        msg.extend_from_slice(timestr.as_bytes());
        msg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &msg);
    } else {
        let up = g.now - g.boot_time;
        let d = up / 86400;
        let h = (up / 3600) % 24;
        let m = (up / 60) % 60;
        let msg = format!(
            "Up since {}: {} day{}, {}:{:02}\r\n",
            timestr,
            d,
            if d == 1 { "" } else { "s" },
            h,
            m
        );
        send_to_char(g, chid, msg.as_bytes());
    }
}

/// strftime("%a %b %d %Y") — the hcontrol/house build-date stamp.
pub fn strftime_date(unix: i64, tz_offset_secs: i64) -> String {
    let c = ctime_like(unix, tz_offset_secs);
    // "Sat Aug 22 20:05:00 2026" -> "Sat Aug 22 2026", day zero-padded (%d).
    let p: Vec<&str> = c.split_whitespace().collect();
    format!("{} {} {:02} {}", p[0], p[1], p[2].parse::<i32>().unwrap_or(0), p[4])
}

/// strftime("%c")-alike: "Sat Aug 22 20:05:00 2026".
pub fn ctime_like(unix: i64, tz_offset_secs: i64) -> String {
    let local = unix + tz_offset_secs;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    let wd = (days + 4).rem_euclid(7); // 1970-01-01 = Thursday
    const WDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}",
        WDAYS[wd as usize],
        MONTHS[(mth - 1) as usize],
        d,
        h,
        m,
        s,
        y
    )
}

pub fn do_shutdown(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_SHUTDOWN;
    if subcmd != SCMD_SHUTDOWN {
        send_to_char(g, chid, b"If you want to shut something down, say so!\r\n");
        return;
    }
    let (arg, _) = one_argument(argument);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();

    if arg.is_empty() {
        g.log(format!("(GC) Shutdown by {}.", name));
        comm::send_to_all(g, b"Shutting down.\r\n");
        g.circle_shutdown = true;
    } else if arg == b"reboot" {
        g.log(format!("(GC) Reboot by {}.", name));
        comm::send_to_all(g, b"Rebooting.. come back in a few minutes.\r\n");
        touch_file(g, "../.fastboot");
        g.circle_shutdown = true;
        g.circle_reboot = 2;
    } else if arg == b"die" {
        g.log(format!("(GC) Shutdown by {}.", name));
        comm::send_to_all(g, b"Shutting down for maintenance.\r\n");
        touch_file(g, "../.killscript");
        g.circle_shutdown = true;
    } else if arg == b"now" {
        g.log(format!("(GC) Shutdown NOW by {}.", name));
        comm::send_to_all(g, b"Rebooting.. come back in a minute or two.\r\n");
        g.circle_shutdown = true;
        g.circle_reboot = 2;
    } else if arg == b"pause" {
        g.log(format!("(GC) Shutdown by {}.", name));
        comm::send_to_all(g, b"Shutting down for maintenance.\r\n");
        touch_file(g, "../pause");
        g.circle_shutdown = true;
    } else {
        send_to_char(g, chid, b"Unknown shutdown option.\r\n");
    }
}

fn touch_file(g: &mut Game, rel: &str) {
    let path = g.lib_dir.join(rel);
    if let Err(e) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        g.log(format!("SYSERR: {}: {}", path.display(), e));
    }
}


pub fn do_send(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, buf) = half_chop(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Send what to who?\r\n");
        return;
    }
    let Some(vict) = get_char_world_vis(g, chid, &arg, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    let mut line = buf.clone();
    line.extend_from_slice(b"\r\n");
    send_to_char(g, vict, &line);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Cmp,
        (LVL_GOD as i16).max(invis) as u8,
        true,
        &format!("(GC) {} sent {}: {}", name, vname, String::from_utf8_lossy(&buf)),
    );
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        send_to_char(g, chid, b"Sent.\r\n");
    } else {
        let mut out = b"You send '".to_vec();
        out.extend_from_slice(&buf);
        out.extend_from_slice(b"' to ");
        out.extend_from_slice(vname.as_bytes());
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
    }
}

/// The transfer body shared by `trans <who>` and `trans all`.
fn perform_transfer(g: &mut Game, chid: CharId, victim: CharId) {
    act(g, b"$n disappears in a mushroom cloud.", false, Some(victim), None, None, comm::TO_ROOM);
    char_from_room(g, victim);
    let dest = g.ch(chid).in_room;
    char_to_room(g, victim, dest);
    act(g, b"$n arrives from a puff of smoke.", false, Some(victim), None, None, comm::TO_ROOM);
    act(g, b"$n has transferred you!", false, Some(chid), None, Some(victim), comm::TO_VICT);
    crate::act::informative::look_at_room(g, victim, false);
    let room = g.ch(victim).in_room;
    crate::dg::triggers::enter_wtrigger(g, room, victim, -1);
}

pub fn do_trans(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    if buf.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to transfer?\r\n");
        return;
    }
    if buf != b"all" {
        let Some(victim) = get_char_world_vis(g, chid, &buf, None) else {
            let msg = g.config.noperson.clone();
            send_to_char(g, chid, &msg);
            return;
        };
        if victim == chid {
            send_to_char(g, chid, b"That doesn't make much sense, does it?\r\n");
            return;
        }
        if g.ch(chid).level < g.ch(victim).level && !g.ch(victim).is_npc() {
            send_to_char(g, chid, b"Go transfer someone your own size.\r\n");
            return;
        }
        perform_transfer(g, chid, victim);
        return;
    }
    // Trans all.
    if g.ch(chid).level < LVL_GRGOD {
        send_to_char(g, chid, b"I think not.\r\n");
        return;
    }
    for di in g.descriptors.order.clone() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(victim) = d.character else { continue };
        if victim == chid || g.try_ch(victim).is_none() {
            continue;
        }
        if g.ch(victim).level >= g.ch(chid).level {
            continue;
        }
        perform_transfer(g, chid, victim);
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
}

pub fn do_teleport(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, buf2, _) = two_arguments(argument);
    if buf.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to teleport?\r\n");
        return;
    }
    let Some(victim) = get_char_world_vis(g, chid, &buf, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if victim == chid {
        send_to_char(g, chid, b"Use 'goto' to teleport yourself.\r\n");
        return;
    }
    if g.ch(victim).level >= g.ch(chid).level {
        send_to_char(g, chid, b"Maybe you shouldn't do that.\r\n");
        return;
    }
    if buf2.is_empty() {
        send_to_char(g, chid, b"Where do you wish to send this person?\r\n");
        return;
    }
    let Some(target) = find_target_room(g, chid, &buf2) else { return };
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
    act(g, b"$n disappears in a puff of smoke.", false, Some(victim), None, None, comm::TO_ROOM);
    char_from_room(g, victim);
    char_to_room(g, victim, target);
    act(g, b"$n arrives from a puff of smoke.", false, Some(victim), None, None, comm::TO_ROOM);
    act(g, b"$n has teleported you!", false, Some(chid), None, Some(victim), comm::TO_VICT);
    crate::act::informative::look_at_room(g, victim, false);
    let room = g.ch(victim).in_room;
    crate::dg::triggers::enter_wtrigger(g, room, victim, -1);
}

/// "%3d. [%5d] %-40s %s\r\n" — the vnum_* listing row.
fn vnum_row(found: i32, vnum: i32, name: &[u8], trig: bool) -> BStr {
    let mut out = format!("{:3}. [{:5}] ", found, vnum).into_bytes();
    out.extend_from_slice(&crate::act::pad_right(name, 40));
    out.push(b' ');
    if trig {
        out.extend_from_slice(b"[TRIG]");
    }
    out.extend_from_slice(b"\r\n");
    out
}

fn vnum_mobile(g: &mut Game, searchname: &[u8], chid: CharId) -> i32 {
    let mut found = 0;
    let rows: Vec<BStr> = (0..g.world.mob_protos.len())
        .filter_map(|nr| {
            let p = &g.world.mob_protos[nr];
            if !crate::handler::isname(searchname, p.keywords.as_deref().unwrap_or(b"")) {
                return None;
            }
            found += 1;
            Some(vnum_row(
                found,
                p.vnum as i32,
                p.short_descr.as_deref().unwrap_or(b""),
                !p.proto_script.is_empty(),
            ))
        })
        .collect();
    for r in rows {
        send_to_char(g, chid, &r);
    }
    found
}

fn vnum_object(g: &mut Game, searchname: &[u8], chid: CharId) -> i32 {
    let mut found = 0;
    let rows: Vec<BStr> = (0..g.world.obj_protos.len())
        .filter_map(|nr| {
            let p = &g.world.obj_protos[nr];
            if !crate::handler::isname(searchname, p.name.as_deref().unwrap_or(b"")) {
                return None;
            }
            found += 1;
            Some(vnum_row(
                found,
                p.vnum as i32,
                p.short_description.as_deref().unwrap_or(b""),
                !p.proto_script.is_empty(),
            ))
        })
        .collect();
    for r in rows {
        send_to_char(g, chid, &r);
    }
    found
}

fn vnum_room(g: &mut Game, searchname: &[u8], chid: CharId) -> i32 {
    let mut found = 0;
    let rows: Vec<BStr> = (0..g.world.rooms.len())
        .filter_map(|nr| {
            let r = &g.world.rooms[nr];
            if !crate::handler::isname(searchname, r.name.as_deref().unwrap_or(b"")) {
                return None;
            }
            found += 1;
            Some(vnum_row(
                found,
                r.vnum as i32,
                r.name.as_deref().unwrap_or(b""),
                !r.proto_script.is_empty(),
            ))
        })
        .collect();
    for r in rows {
        send_to_char(g, chid, &r);
    }
    found
}

/// List every trigger whose name matches, one per line, with its vnum.
fn vnum_trig(g: &mut Game, searchname: &[u8], chid: CharId) -> i32 {
    let mut found = 0;
    let top = g.world.triggers.len();
    let rows: Vec<BStr> = (0..top)
        .filter_map(|nr| {
            let t = &g.world.triggers[nr];
            if !crate::handler::isname(searchname, t.name.as_deref().unwrap_or(b"")) {
                return None;
            }
            found += 1;
            let mut out = format!("{:3}. [{:5}] ", found, t.vnum).into_bytes();
            out.extend_from_slice(&crate::act::pad_right(t.name.as_deref().unwrap_or(b""), 40));
            out.extend_from_slice(b"\r\n");
            Some(out)
        })
        .collect();
    for r in rows {
        send_to_char(g, chid, &r);
    }
    found
}

pub fn do_vnum(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    const USAGE: &[u8] = b"Usage: vnum { obj | mob | room | trig } <name>\r\n";
    let (buf, buf2) = half_chop(argument);
    if buf.is_empty() || buf2.is_empty() {
        send_to_char(g, chid, USAGE);
        return;
    }
    let mut good_arg = false;
    if is_abbrev(&buf, b"mob") {
        good_arg = true;
        if vnum_mobile(g, &buf2, chid) == 0 {
            send_to_char(g, chid, b"No mobiles by that name.\r\n");
        }
    }
    if is_abbrev(&buf, b"obj") {
        good_arg = true;
        if vnum_object(g, &buf2, chid) == 0 {
            send_to_char(g, chid, b"No objects by that name.\r\n");
        }
    }
    if is_abbrev(&buf, b"room") {
        good_arg = true;
        if vnum_room(g, &buf2, chid) == 0 {
            send_to_char(g, chid, b"No rooms by that name.\r\n");
        }
    }
    if is_abbrev(&buf, b"trig") {
        good_arg = true;
        if vnum_trig(g, &buf2, chid) == 0 {
            send_to_char(g, chid, b"No triggers by that name.\r\n");
        }
    }
    if !good_arg {
        send_to_char(g, chid, USAGE);
    }
}

/// GET_ROOM_ZONE as an Option, for can_edit_zone.
pub fn zone_of(g: &Game, room: RoomRnum) -> Option<usize> {
    if room == NOWHERE {
        None
    } else {
        Some(g.world.rooms[room as usize].zone as usize)
    }
}

pub fn do_load(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, buf2, rest) = two_arguments(argument);
    let (buf3, _) = one_argument(rest);

    if buf.is_empty() || buf2.is_empty() || !buf2[0].is_ascii_digit() {
        send_to_char(g, chid, b"Usage: load < obj | mob > <vnum> <number>\r\n");
        return;
    }
    if !is_number(&buf2) {
        send_to_char(g, chid, b"That is not a number.\r\n");
        return;
    }
    let n = {
        let v = atoi(&buf3);
        if v > 0 && v <= 100 {
            v
        } else {
            1
        }
    };

    let zone = zone_of(g, g.ch(chid).in_room);
    let may_build =
        g.ch(chid).level >= LVL_GRGOD || crate::dg::commands::can_edit_zone(g, chid, zone);

    if is_abbrev(&buf, b"mob") {
        if !may_build {
            send_to_char(g, chid, b"Sorry, you can't load mobs here.\r\n");
            return;
        }
        let Some(r_num) = g.world.real_mobile(atoi(&buf2) as Idx) else {
            send_to_char(g, chid, b"There is no monster with that number.\r\n");
            return;
        };
        for _ in 0..n {
            let Some(mob) = crate::db::read_mobile(g, r_num) else { continue };
            let room = g.ch(chid).in_room;
            char_to_room(g, mob, room);
            act(
                g,
                b"$n makes a quaint, magical gesture with one hand.",
                true,
                Some(chid),
                None,
                None,
                comm::TO_ROOM,
            );
            act(g, b"$n has created $N!", false, Some(chid), None, Some(mob), comm::TO_ROOM);
            act(g, b"You create $N.", false, Some(chid), None, Some(mob), comm::TO_CHAR);
            crate::dg::triggers::load_mtrigger(g, mob);
        }
    } else if is_abbrev(&buf, b"obj") {
        if !may_build {
            send_to_char(g, chid, b"Sorry, you can't load objects here.\r\n");
            return;
        }
        let Some(r_num) = g.world.real_object(atoi(&buf2) as Idx) else {
            send_to_char(g, chid, b"There is no object with that number.\r\n");
            return;
        };
        for _ in 0..n {
            let Some(obj) = crate::db::read_object(g, r_num) else { continue };
            if g.config.load_into_inventory {
                crate::handler::obj_to_char(g, obj, chid);
            } else {
                let room = g.ch(chid).in_room;
                crate::handler::obj_to_room(g, obj, room);
            }
            act(
                g,
                b"$n makes a strange magical gesture.",
                true,
                Some(chid),
                None,
                None,
                comm::TO_ROOM,
            );
            act(g, b"$n has created $p!", false, Some(chid), Some(obj), None, comm::TO_ROOM);
            act(g, b"You create $p.", false, Some(chid), Some(obj), None, comm::TO_CHAR);
            crate::dg::triggers::load_otrigger(g, obj);
        }
    } else {
        send_to_char(g, chid, b"That'll have to be either 'obj' or 'mob'.\r\n");
    }
}

pub fn purge_room(g: &mut Game, room: RoomRnum) -> bool {
    if room == NOWHERE || room as usize >= g.world.rooms.len() {
        return false;
    }
    for vict in g.rooms[room as usize].people.clone() {
        if g.try_ch(vict).is_none() || !g.ch(vict).is_npc() {
            continue;
        }
        while let Some(&oid) = g.ch(vict).carrying.first() {
            crate::handler::extract_obj(g, oid);
        }
        for j in 0..NUM_WEARS {
            if let Some(oid) = g.ch(vict).equipment[j] {
                crate::handler::extract_obj(g, oid);
            }
        }
        crate::handler::extract_char(g, vict);
    }
    while let Some(&oid) = g.rooms[room as usize].contents.first() {
        crate::handler::extract_obj(g, oid);
    }
    true
}

pub fn do_purge(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);

    let zone = zone_of(g, g.ch(chid).in_room);
    if g.ch(chid).level < LVL_GRGOD && !crate::dg::commands::can_edit_zone(g, chid, zone) {
        send_to_char(g, chid, b"Sorry, you can't purge anything here.\r\n");
        return;
    }

    if !buf.is_empty() {
        let (number, name) = get_number(&buf);
        let mut count = number;
        if let Some(vict) = crate::handler::get_char_room_vis_counted(g, chid, &name, &mut count) {
            if !g.ch(vict).is_npc() && g.ch(chid).level <= g.ch(vict).level {
                let mut out = b"You can't purge ".to_vec();
                out.extend_from_slice(g.ch(vict).get_name());
                out.extend_from_slice(b"!\r\n");
                send_to_char(g, chid, &out);
                return;
            }
            act(g, b"$n disintegrates $N.", false, Some(chid), None, Some(vict), comm::TO_NOTVICT);
            if !g.ch(vict).is_npc() {
                let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
                let invis = g.ch(chid).invis_lev();
                g.mudlog(
                    MudlogKind::Brf,
                    (LVL_GOD as i16).max(invis) as u8,
                    true,
                    &format!("(GC) {} has purged {}.", name, vname),
                );
                if let Some(di) = g.ch(vict).desc {
                    if let Some(d) = g.descriptors.get_mut(di) {
                        d.state = ConState::Close;
                        d.character = None;
                    }
                    g.ch_mut(vict).desc = None;
                }
            }
            crate::handler::extract_char(g, vict);
        } else {
            let room = g.ch(chid).in_room;
            let contents = g.rooms[room as usize].contents.clone();
            let mut count2 = number;
            if let Some(obj) =
                crate::handler::get_obj_in_list_vis_counted(g, chid, &name, &mut count2, &contents)
            {
                act(g, b"$n destroys $p.", false, Some(chid), Some(obj), None, comm::TO_ROOM);
                crate::handler::extract_obj(g, obj);
            } else {
                send_to_char(g, chid, b"Nothing here by that name.\r\n");
                return;
            }
        }
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        act(
            g,
            b"$n gestures... You are surrounded by scorching flames!",
            false,
            Some(chid),
            None,
            None,
            comm::TO_ROOM,
        );
        let room = g.ch(chid).in_room;
        comm::send_to_room(g, room, b"The world seems a little cleaner.\r\n");
        purge_room(g, room);
    }
}

pub fn do_advance(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (name, level, _) = two_arguments(argument);
    if name.is_empty() {
        send_to_char(g, chid, b"Advance who?\r\n");
        return;
    }
    let Some(victim) = get_char_world_vis(g, chid, &name, None) else {
        send_to_char(g, chid, b"That player is not here.\r\n");
        return;
    };
    if g.ch(chid).level <= g.ch(victim).level {
        send_to_char(g, chid, b"Maybe that's not such a great idea.\r\n");
        return;
    }
    if g.ch(victim).is_npc() {
        send_to_char(g, chid, b"NO!  Not on NPC's.\r\n");
        return;
    }
    let newlevel = atoi(&level);
    if level.is_empty() || newlevel <= 0 {
        send_to_char(g, chid, b"That's not a level!\r\n");
        return;
    }
    if newlevel > LVL_IMPL as i32 {
        send_to_char(g, chid, format!("{} is the highest possible level.\r\n", LVL_IMPL).as_bytes());
        return;
    }
    if newlevel > g.ch(chid).level as i32 {
        send_to_char(g, chid, b"Yeah, right.\r\n");
        return;
    }
    if newlevel == g.ch(victim).level as i32 {
        act(g, b"$E is already at that level.", false, Some(chid), None, Some(victim), comm::TO_CHAR);
        return;
    }
    let oldlevel = g.ch(victim).level as i32;
    if newlevel < oldlevel {
        crate::login::do_start(g, victim);
        g.ch_mut(victim).level = newlevel as u8;
        send_to_char(
            g,
            victim,
            b"You are momentarily enveloped by darkness!\r\nYou feel somewhat diminished.\r\n",
        );
    } else {
        act(
            g,
            b"$n makes some strange gestures. A strange feeling comes upon you,\r\n\
Like a giant hand, light comes down from above, grabbing your body,\r\n\
that begins to pulse with colored lights from inside.\r\n\r\n\
Your head seems to be filled with demons from another plane as\r\n\
your body dissolves to the elements of time and space itself.\r\n\
Suddenly a silent explosion of light snaps you back to reality.\r\n\r\n\
You feel slightly different.",
            false,
            Some(chid),
            None,
            Some(victim),
            comm::TO_VICT,
        );
    }

    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);

    let gname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(victim).get_name()).into_owned();
    if newlevel < oldlevel {
        g.log(format!("(GC) {} demoted {} from level {} to {}.", gname, vname, oldlevel, newlevel));
    } else {
        g.log(format!("(GC) {} has advanced {} to level {} (from {})", gname, vname, newlevel, oldlevel));
    }

    if oldlevel >= LVL_IMMORT as i32 && newlevel < LVL_IMMORT as i32 {
        for bit in [
            flags::PRF_LOG1,
            flags::PRF_LOG2,
            flags::PRF_NOHASSLE,
            flags::PRF_HOLYLIGHT,
            flags::PRF_SHOWVNUMS,
        ] {
            g.ch_mut(victim).ps_mut().pref.remove(bit);
        }
        if !g.ch(victim).plr(flags::PLR_NOWIZLIST) {
            crate::limits::run_autowiz(g);
        }
    } else if oldlevel < LVL_IMMORT as i32 && newlevel >= LVL_IMMORT as i32 {
        for bit in [
            flags::PRF_LOG2,
            flags::PRF_HOLYLIGHT,
            flags::PRF_SHOWVNUMS,
            flags::PRF_AUTOEXIT,
        ] {
            g.ch_mut(victim).ps_mut().pref.set(bit);
        }
        for i in 1..=MAX_SKILLS as i32 {
            g.ch_mut(victim).set_skill(i, 100);
        }
        let ps = g.ch_mut(victim).ps_mut();
        ps.olc_zone = NOWHERE as i32;
        ps.conditions = [-1, -1, -1];
    }

    let class = g.ch(victim).class as i32;
    let gain = mud_data::tables::level_exp(class, newlevel) - g.ch(victim).points.exp;
    crate::limits::gain_exp_regardless(g, victim, gain);
    crate::players_glue::save_char(g, victim);
}

pub fn do_restore(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, _) = one_argument(argument);
    if buf.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to restore?\r\n");
        return;
    }
    if is_abbrev(&buf, b"all") {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} restored all", name),
        );
        for di in g.descriptors.order.clone() {
            let Some(d) = g.descriptors.get(di) else { continue };
            if !d.is_playing() {
                continue;
            }
            let Some(vict) = d.character else { continue };
            if g.try_ch(vict).is_none() || g.ch(vict).level >= LVL_IMMORT {
                continue;
            }
            let ch = g.ch_mut(vict);
            ch.points.hit = ch.points.max_hit;
            ch.points.mana = ch.points.max_mana;
            ch.points.mov = ch.points.max_move;
            crate::fight::update_pos(g, vict);
            let mut out = g.ch(vict).get_name().to_vec();
            out.extend_from_slice(b" has been fully healed.\r\n");
            send_to_char(g, chid, &out);
            act(
                g,
                b"You have been fully healed by $N!",
                false,
                Some(vict),
                None,
                Some(chid),
                comm::TO_CHAR,
            );
        }
        return;
    }
    let Some(vict) = get_char_world_vis(g, chid, &buf, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if !g.ch(vict).is_npc() && chid != vict && g.ch(vict).level >= g.ch(chid).level {
        act(g, b"$E doesn't need your help.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
        return;
    }
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Nrm,
        (LVL_GOD as i16).max(invis) as u8,
        true,
        &format!("(GC) {} restored {}", name, vname),
    );
    {
        let ch = g.ch_mut(vict);
        ch.points.hit = ch.points.max_hit;
        ch.points.mana = ch.points.max_mana;
        ch.points.mov = ch.points.max_move;
    }
    if !g.ch(vict).is_npc() && g.ch(chid).level >= LVL_GRGOD {
        if g.ch(vict).level >= LVL_IMMORT {
            for i in 1..=MAX_SKILLS as i32 {
                g.ch_mut(vict).set_skill(i, 100);
            }
        }
        if g.ch(vict).level >= LVL_GRGOD {
            let a = &mut g.ch_mut(vict).real_abils;
            a.str_add = 100;
            a.intel = 25;
            a.wis = 25;
            a.dex = 25;
            a.str_ = 25;
            a.con = 25;
            a.cha = 25;
        }
    }
    crate::fight::update_pos(g, vict);
    crate::handler::affect_total(g, vict);
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
    act(g, b"You have been fully healed by $N!", false, Some(vict), None, Some(chid), comm::TO_CHAR);
}

pub fn perform_immort_vis(g: &mut Game, chid: CharId) {
    if g.ch(chid).invis_lev() == 0
        && !g.ch(chid).aff(flags::AFF_HIDE)
        && !g.ch(chid).aff(flags::AFF_INVISIBLE)
    {
        send_to_char(g, chid, b"You are already fully visible.\r\n");
        return;
    }
    g.ch_mut(chid).ps_mut().invis_level = 0;
    crate::act::other::appear(g, chid);
    send_to_char(g, chid, b"You are now fully visible.\r\n");
}

fn perform_immort_invis(g: &mut Game, chid: CharId, level: i16) {
    let room = g.ch(chid).in_room;
    let cur = g.ch(chid).invis_lev();
    for tch in g.rooms[room as usize].people.clone() {
        if tch == chid || g.try_ch(tch).is_none() || g.ch(tch).is_npc() {
            continue;
        }
        let tlev = g.ch(tch).level as i16;
        if tlev >= cur && tlev < level {
            act(
                g,
                b"You blink and suddenly realize that $n is gone.",
                false,
                Some(chid),
                None,
                Some(tch),
                comm::TO_VICT,
            );
        }
        if tlev < cur && tlev >= level {
            act(
                g,
                b"You suddenly realize that $n is standing beside you.",
                false,
                Some(chid),
                None,
                Some(tch),
                comm::TO_VICT,
            );
        }
    }
    g.ch_mut(chid).ps_mut().invis_level = level;
    send_to_char(g, chid, format!("Your invisibility level is {}.\r\n", level).as_bytes());
}

pub fn do_invis(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"You can't do that!\r\n");
        return;
    }
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        if g.ch(chid).invis_lev() > 0 {
            perform_immort_vis(g, chid);
        } else {
            let lev = g.ch(chid).level as i16;
            perform_immort_invis(g, chid, lev);
        }
        return;
    }
    let level = atoi(&arg);
    if level > g.ch(chid).level as i32 {
        send_to_char(g, chid, b"You can't go invisible above your own level.\r\n");
    } else if level < 1 {
        perform_immort_vis(g, chid);
    } else {
        perform_immort_invis(g, chid, level as i16);
    }
}

pub fn do_gecho(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let mut argument = crate::interpreter::skip_spaces(argument).to_vec();
    crate::interpreter::delete_doubledollar(&mut argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"That must be a mistake...\r\n");
        return;
    }
    let mut line = argument.clone();
    line.extend_from_slice(b"\r\n");
    for di in g.descriptors.order.clone() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if !d.is_playing() {
            continue;
        }
        let Some(pt) = d.character else { continue };
        if pt == chid || g.try_ch(pt).is_none() {
            continue;
        }
        send_to_char(g, pt, &line);
    }
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Cmp,
        (LVL_BUILDER as i16).max(invis) as u8,
        true,
        &format!("(GC) {} gechoed: {}", name, String::from_utf8_lossy(&argument)),
    );
    if g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        send_to_char(g, chid, &line);
    }
}

pub fn do_dc(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let num_to_dc = atoi(&arg);
    if num_to_dc == 0 {
        send_to_char(g, chid, b"Usage: DC <user number> (type USERS for a list)\r\n");
        return;
    }
    let target = g
        .descriptors
        .order
        .iter()
        .copied()
        .find(|&di| g.descriptors.get(di).is_some_and(|d| d.desc_num as i32 == num_to_dc));
    let Some(di) = target else {
        send_to_char(g, chid, b"No such connection.\r\n");
        return;
    };
    let victim = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(v) = victim {
        if g.try_ch(v).is_some() && g.ch(v).level >= g.ch(chid).level {
            if !crate::handler::can_see(g, chid, v) {
                send_to_char(g, chid, b"No such connection.\r\n");
            } else {
                send_to_char(g, chid, b"Umm.. maybe that's not such a good idea...\r\n");
            }
            return;
        }
    }
    let state = g.descriptors.get(di).map(|d| d.state).unwrap_or(ConState::Close);
    if state == ConState::Disconnect || state == ConState::Close {
        act(g, b"$E's already being disconnected.", false, Some(chid), None, victim, comm::TO_CHAR);
        return;
    }
    let new_state = if state == ConState::Playing { ConState::Disconnect } else { ConState::Close };
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = new_state;
    }
    send_to_char(g, chid, format!("Connection #{} closed.\r\n", num_to_dc).as_bytes());
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    g.log(format!("(GC) Connection closed by {}.", name));
}

pub fn do_wizlock(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let when: &[u8] = if !arg.is_empty() {
        let value = atoi(&arg);
        if value < 0 || value > g.ch(chid).level as i32 {
            send_to_char(g, chid, b"Invalid wizlock value.\r\n");
            return;
        }
        g.circle_restrict = value as u8;
        b"now"
    } else {
        b"currently"
    };
    match g.circle_restrict {
        0 => {
            let mut out = b"The game is ".to_vec();
            out.extend_from_slice(when);
            out.extend_from_slice(b" completely open.\r\n");
            send_to_char(g, chid, &out);
        }
        1 => {
            let mut out = b"The game is ".to_vec();
            out.extend_from_slice(when);
            out.extend_from_slice(b" closed to new players.\r\n");
            send_to_char(g, chid, &out);
        }
        n => {
            let mut out = format!("Only level {} and above may enter the game ", n).into_bytes();
            out.extend_from_slice(when);
            out.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &out);
        }
    }
}

pub fn do_force(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, to_force) = half_chop(argument);
    let mut buf1 = b"$n has forced you to '".to_vec();
    buf1.extend_from_slice(&to_force);
    buf1.extend_from_slice(b"'.");

    if arg.is_empty() || to_force.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to force do what?\r\n");
        return;
    }
    let level = g.ch(chid).level;
    if level < LVL_GRGOD || (arg != b"all" && arg != b"room") {
        let Some(vict) = get_char_world_vis(g, chid, &arg, None) else {
            let msg = g.config.noperson.clone();
            send_to_char(g, chid, &msg);
            return;
        };
        if !g.ch(vict).is_npc() && level < LVL_GOD {
            send_to_char(g, chid, b"You cannot force players.\r\n");
            return;
        }
        if !g.ch(vict).is_npc() && level <= g.ch(vict).level {
            send_to_char(g, chid, b"No, no, no!\r\n");
            return;
        }
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
        act(g, &buf1, true, Some(chid), None, Some(vict), comm::TO_VICT);
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Cmp,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} forced {} to {}", name, vname, String::from_utf8_lossy(&to_force)),
        );
        crate::interpreter::command_interpreter(g, vict, &to_force);
    } else if arg == b"room" {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let room = g.ch(chid).in_room;
        let vnum = g.world.rooms[room as usize].vnum;
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} forced room {} to {}", name, vnum, String::from_utf8_lossy(&to_force)),
        );
        for vict in g.rooms[room as usize].people.clone() {
            if g.try_ch(vict).is_none() {
                continue;
            }
            if !g.ch(vict).is_npc() && g.ch(vict).level >= level {
                continue;
            }
            act(g, &buf1, true, Some(chid), None, Some(vict), comm::TO_VICT);
            crate::interpreter::command_interpreter(g, vict, &to_force);
        }
    } else {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} forced all to {}", name, String::from_utf8_lossy(&to_force)),
        );
        for di in g.descriptors.order.clone() {
            let Some(d) = g.descriptors.get(di) else { continue };
            if d.state != ConState::Playing {
                continue;
            }
            let Some(vict) = d.character else { continue };
            if g.try_ch(vict).is_none() {
                continue;
            }
            if !g.ch(vict).is_npc() && g.ch(vict).level >= level {
                continue;
            }
            act(g, &buf1, true, Some(chid), None, Some(vict), comm::TO_VICT);
            crate::interpreter::command_interpreter(g, vict, &to_force);
        }
    }
}

pub fn do_wiznet(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let mut argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        send_to_char(g, chid, b"Usage: wiznet [ #<level> ] [<text> | *<emotetext> | @ ]\r\n");
        return;
    }
    let mut emote = false;
    let mut level = LVL_IMMORT as i32;
    match argument[0] {
        c @ (b'*' | b'#') => {
            emote = c == b'*';
            let (buf1, _) = one_argument(&argument[1..]);
            if is_number(&buf1) {
                let (num, rest) = half_chop(&argument[1..]);
                level = atoi(&num).max(LVL_IMMORT as i32);
                argument = rest;
                if level > g.ch(chid).level as i32 {
                    send_to_char(g, chid, b"You can't wizline above your own level.\r\n");
                    return;
                }
            } else if emote {
                argument.remove(0);
            }
        }
        b'@' => {
            send_to_char(g, chid, b"God channel status:\r\n");
            let mut rows: Vec<BStr> = Vec::new();
            for di in g.descriptors.order.clone() {
                let Some(d) = g.descriptors.get(di) else { continue };
                if d.state != ConState::Playing {
                    continue;
                }
                let Some(t) = d.character else { continue };
                if g.try_ch(t).is_none() || g.ch(t).level < LVL_IMMORT {
                    continue;
                }
                if !crate::handler::can_see(g, chid, t) {
                    continue;
                }
                let mut out = b"  ".to_vec();
                out.extend_from_slice(&crate::act::pad_right(g.ch(t).get_name(), MAX_NAME_LENGTH));
                if g.ch(t).plr(flags::PLR_WRITING) {
                    out.extend_from_slice(b" (Writing)");
                }
                if g.ch(t).plr(flags::PLR_MAILING) {
                    out.extend_from_slice(b" (Writing mail)");
                }
                if g.ch(t).prf(flags::PRF_NOWIZ) {
                    out.extend_from_slice(b" (Offline)");
                }
                out.extend_from_slice(b"\r\n");
                rows.push(out);
            }
            for r in rows {
                send_to_char(g, chid, &r);
            }
            return;
        }
        b'\\' => {
            argument.remove(0);
        }
        _ => {}
    }
    if g.ch(chid).prf(flags::PRF_NOWIZ) {
        send_to_char(g, chid, b"You are offline!\r\n");
        return;
    }
    let argument = crate::interpreter::skip_spaces(&argument).to_vec();
    if argument.is_empty() {
        send_to_char(g, chid, b"Don't bother the gods like that!\r\n");
        return;
    }
    let name = g.ch(chid).get_name().to_vec();
    let arrow: &[u8] = if emote { b"<--- " } else { b"" };
    let (mut buf1, mut buf2) = (Vec::new(), Vec::new());
    if level > LVL_IMMORT as i32 {
        buf1.extend_from_slice(b"\tc");
        buf1.extend_from_slice(&name);
        buf1.extend_from_slice(format!(": <{}> ", level).as_bytes());
        buf1.extend_from_slice(arrow);
        buf1.extend_from_slice(&argument);
        buf1.extend_from_slice(b"\tn\r\n");
        buf2.extend_from_slice(format!("\tcSomeone: <{}> ", level).as_bytes());
        buf2.extend_from_slice(arrow);
        buf2.extend_from_slice(&argument);
        buf2.extend_from_slice(b"\tn\r\n");
    } else {
        buf1.extend_from_slice(b"\tc");
        buf1.extend_from_slice(&name);
        buf1.extend_from_slice(b": ");
        buf1.extend_from_slice(arrow);
        buf1.extend_from_slice(&argument);
        buf1.extend_from_slice(b"\tn\r\n");
        buf2.extend_from_slice(b"\tcSomeone: ");
        buf2.extend_from_slice(arrow);
        buf2.extend_from_slice(&argument);
        buf2.extend_from_slice(b"\tn\r\n");
    }

    let self_desc = g.ch(chid).desc;
    for di in g.descriptors.order.clone() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if !d.is_playing() {
            continue;
        }
        let Some(t) = d.character else { continue };
        if g.try_ch(t).is_none() || g.ch(t).level < level as u8 {
            continue;
        }
        if g.ch(t).prf(flags::PRF_NOWIZ) {
            continue;
        }
        if Some(di) == self_desc && g.ch(t).prf(flags::PRF_NOREPEAT) {
            continue;
        }
        let mut body = if crate::handler::can_see(g, t, chid) { buf1.clone() } else { buf2.clone() };
        crate::text::parse_at(&mut body);
        let cyn = cc(g, t, C_NRM, KCYN).to_vec();
        let nrm = cc(g, t, C_NRM, KNRM).to_vec();
        let mut out = cyn;
        out.extend_from_slice(&body);
        out.extend_from_slice(&nrm);
        send_to_char(g, t, &out);
        crate::act::informative::add_history(g, t, &body, crate::act::informative::HIST_WIZNET);
    }

    if g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    }
}

pub fn do_zreset(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();

    if arg.first() == Some(&b'*') {
        if g.ch(chid).level < LVL_GOD {
            send_to_char(g, chid, b"You do not have permission to reset the entire world.\r\n");
            return;
        }
        for zr in 0..g.world.zones.len() {
            crate::db::reset_zone(g, zr);
        }
        send_to_char(g, chid, b"Reset world.\r\n");
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} reset entire world.", name),
        );
        return;
    }

    // An unknown zone resolves to nothing and takes the same path as a
    // denied one, so the two are indistinguishable to the builder.
    let i: Option<usize> = if arg.first() == Some(&b'.') || arg.is_empty() {
        Some(g.world.rooms[g.ch(chid).in_room as usize].zone as usize)
    } else {
        let j = atoi(&arg);
        g.world.zones.iter().position(|z| z.number as i32 == j)
    };

    let allowed = i.is_some_and(|zr| {
        crate::dg::commands::can_edit_zone(g, chid, Some(zr)) || g.ch(chid).level > LVL_IMMORT
    });
    if let (Some(zr), true) = (i, allowed) {
        crate::db::reset_zone(g, zr);
        let (num, zname) = {
            let z = &g.world.zones[zr];
            (z.number, z.name.clone().unwrap_or_default())
        };
        let mut out = format!("Reset zone #{}: ", num).into_bytes();
        out.extend_from_slice(&zname);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} reset zone {} ({})", name, num, String::from_utf8_lossy(&zname)),
        );
    } else {
        let olc = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
        send_to_char(
            g,
            chid,
            format!("You do not have permission to reset this zone. Try {}.\r\n", olc).as_bytes(),
        );
    }
}

/// do_wizutil — reroll/pardon/notitle/mute/freeze/
/// thaw/unaffect.
pub fn do_wizutil(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::{
        SCMD_FREEZE, SCMD_MUTE, SCMD_NOTITLE, SCMD_PARDON, SCMD_REROLL, SCMD_THAW, SCMD_UNAFFECT,
    };
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Yes, but for whom?!?\r\n");
        return;
    }
    let Some(vict) = get_char_world_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"There is no such player.\r\n");
        return;
    };
    if g.ch(vict).is_npc() {
        send_to_char(g, chid, b"You can't do that to a mob!\r\n");
        return;
    }
    if g.ch(vict).level >= g.ch(chid).level && vict != chid {
        send_to_char(g, chid, b"Hmmm...you'd better not.\r\n");
        return;
    }
    let gname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();

    match subcmd {
        SCMD_REROLL => {
            send_to_char(g, chid, b"Rerolled...\r\n");
            crate::login::roll_real_abils(g, vict);
            g.log(format!("(GC) {} has rerolled {}.", gname, vname));
            let a = g.ch(vict).aff_abils;
            send_to_char(
                g,
                chid,
                format!(
                    "New stats: Str {}/{}, Int {}, Wis {}, Dex {}, Con {}, Cha {}\r\n",
                    a.str_, a.str_add, a.intel, a.wis, a.dex, a.con, a.cha
                )
                .as_bytes(),
            );
        }
        SCMD_PARDON => {
            if !g.ch(vict).plr(flags::PLR_THIEF) && !g.ch(vict).plr(flags::PLR_KILLER) {
                send_to_char(g, chid, b"Your victim is not flagged.\r\n");
                return;
            }
            g.ch_mut(vict).act.remove(flags::PLR_THIEF);
            g.ch_mut(vict).act.remove(flags::PLR_KILLER);
            send_to_char(g, chid, b"Pardoned.\r\n");
            send_to_char(g, vict, b"You have been pardoned by the Gods!\r\n");
            g.mudlog(
                MudlogKind::Brf,
                (LVL_GOD as i16).max(invis) as u8,
                true,
                &format!("(GC) {} pardoned by {}", vname, gname),
            );
        }
        SCMD_NOTITLE => {
            let result = plr_tog_chk(g, vict, flags::PLR_NOTITLE);
            let msg = format!("(GC) Notitle {} for {} by {}.", onoff(result), vname, gname);
            g.mudlog(MudlogKind::Nrm, (LVL_GOD as i16).max(invis) as u8, true, &msg);
            send_to_char(g, chid, format!("{}\r\n", msg).as_bytes());
        }
        SCMD_MUTE => {
            let result = plr_tog_chk(g, vict, flags::PLR_NOSHOUT);
            let msg = format!("(GC) Mute {} for {} by {}.", onoff(result), vname, gname);
            g.mudlog(MudlogKind::Brf, (LVL_GOD as i16).max(invis) as u8, true, &msg);
            send_to_char(g, chid, format!("{}\r\n", msg).as_bytes());
        }
        SCMD_FREEZE => {
            if chid == vict {
                send_to_char(g, chid, b"Oh, yeah, THAT'S real smart...\r\n");
                return;
            }
            if g.ch(vict).plr(flags::PLR_FROZEN) {
                send_to_char(g, chid, b"Your victim is already pretty cold.\r\n");
                return;
            }
            g.ch_mut(vict).act.set(flags::PLR_FROZEN);
            let lev = g.ch(chid).level as i8;
            g.ch_mut(vict).ps_mut().freeze_level = lev;
            send_to_char(
                g,
                vict,
                b"A bitter wind suddenly rises and drains every erg of heat from your body!\r\nYou feel frozen!\r\n",
            );
            send_to_char(g, chid, b"Frozen.\r\n");
            act(
                g,
                b"A sudden cold wind conjured from nowhere freezes $n!",
                false,
                Some(vict),
                None,
                None,
                comm::TO_ROOM,
            );
            g.mudlog(
                MudlogKind::Brf,
                (LVL_GOD as i16).max(invis) as u8,
                true,
                &format!("(GC) {} frozen by {}.", vname, gname),
            );
        }
        SCMD_THAW => {
            if !g.ch(vict).plr(flags::PLR_FROZEN) {
                send_to_char(
                    g,
                    chid,
                    b"Sorry, your victim is not morbidly encased in ice at the moment.\r\n",
                );
                return;
            }
            let flev = g.ch(vict).ps().freeze_level;
            if flev > g.ch(chid).level as i8 {
                let hmhr = comm::hmhr(g.ch(vict).sex);
                let mut out =
                    format!("Sorry, a level {} God froze {}... you can't unfreeze ", flev, vname)
                        .into_bytes();
                out.extend_from_slice(hmhr);
                out.extend_from_slice(b".\r\n");
                send_to_char(g, chid, &out);
                return;
            }
            g.mudlog(
                MudlogKind::Brf,
                (LVL_GOD as i16).max(invis) as u8,
                true,
                &format!("(GC) {} un-frozen by {}.", vname, gname),
            );
            g.ch_mut(vict).act.remove(flags::PLR_FROZEN);
            send_to_char(
                g,
                vict,
                b"A fireball suddenly explodes in front of you, melting the ice!\r\nYou feel thawed.\r\n",
            );
            send_to_char(g, chid, b"Thawed.\r\n");
            act(
                g,
                b"A sudden fireball conjured from nowhere thaws $n!",
                false,
                Some(vict),
                None,
                None,
                comm::TO_ROOM,
            );
        }
        SCMD_UNAFFECT => {
            let has_bits = !g.ch(vict).affected_by.is_empty();
            if !g.ch(vict).affected.is_empty() || has_bits {
                while !g.ch(vict).affected.is_empty() {
                    crate::handler::affect_remove(g, vict, 0);
                }
                g.ch_mut(vict).affected_by = mud_data::flags::FlagSet::EMPTY;
                crate::handler::affect_total(g, vict);
                send_to_char(
                    g,
                    vict,
                    b"There is a brief flash of light!\r\nYou feel slightly different.\r\n",
                );
                send_to_char(g, chid, b"All spells removed.\r\n");
            } else {
                send_to_char(g, chid, b"Your victim does not have any affections!\r\n");
                return;
            }
        }
        other => {
            g.log(format!("SYSERR: Unknown subcmd {} passed to do_wizutil (act.wizard.c)", other));
        }
    }
    crate::players_glue::save_char(g, vict);
}

/// PLR_TOG_CHK: toggle and report the NEW value.
fn plr_tog_chk(g: &mut Game, chid: CharId, bit: usize) -> bool {
    let now = !g.ch(chid).act.is_set(bit);
    if now {
        g.ch_mut(chid).act.set(bit);
    } else {
        g.ch_mut(chid).act.remove(bit);
    }
    now
}

fn onoff(b: bool) -> &'static str {
    if b {
        "ON"
    } else {
        "OFF"
    }
}

// ---------------------------------------------------------------------------
// Snooping and switching
// ---------------------------------------------------------------------------

/// snoop_check: a level change may invalidate an
/// active snoop in either direction.
pub fn snoop_check(g: &mut Game, chid: CharId) {
    let Some(di) = g.try_ch(chid).and_then(|c| c.desc) else { return };
    let level = g.ch(chid).level;

    let snooping = g.descriptors.get(di).and_then(|d| d.snooping);
    if let Some(sd) = snooping {
        let victim_level =
            g.descriptors.get(sd).and_then(|d| d.character).and_then(|c| g.try_ch(c)).map(|c| c.level);
        if victim_level.is_some_and(|l| l >= level) {
            if let Some(d) = g.descriptors.get_mut(sd) {
                d.snoop_by = None;
            }
            if let Some(d) = g.descriptors.get_mut(di) {
                d.snooping = None;
            }
        }
    }

    let snoop_by = g.descriptors.get(di).and_then(|d| d.snoop_by);
    if let Some(bd) = snoop_by {
        let by_level =
            g.descriptors.get(bd).and_then(|d| d.character).and_then(|c| g.try_ch(c)).map(|c| c.level);
        if by_level.is_some_and(|l| level >= l) {
            if let Some(d) = g.descriptors.get_mut(bd) {
                d.snooping = None;
            }
            if let Some(d) = g.descriptors.get_mut(di) {
                d.snoop_by = None;
            }
        }
    }
}

fn stop_snooping(g: &mut Game, chid: CharId) {
    let di = g.ch(chid).desc.unwrap();
    let Some(sd) = g.descriptors.get(di).and_then(|d| d.snooping) else {
        send_to_char(g, chid, b"You aren't snooping anyone.\r\n");
        return;
    };
    send_to_char(g, chid, b"You stop snooping.\r\n");
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let lvl = g.ch(chid).level;
    g.mudlog(MudlogKind::Brf, lvl, true, &format!("(GC) {} stops snooping", name));
    if let Some(d) = g.descriptors.get_mut(sd) {
        d.snoop_by = None;
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.snooping = None;
    }
}

pub fn do_snoop(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        stop_snooping(g, chid);
        return;
    }
    let Some(victim) = get_char_world_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"No such person around.\r\n");
        return;
    };
    let Some(vd) = g.ch(victim).desc else {
        send_to_char(g, chid, b"There's no link.. nothing to snoop.\r\n");
        return;
    };
    if victim == chid {
        stop_snooping(g, chid);
        return;
    }
    if g.descriptors.get(vd).and_then(|d| d.snoop_by).is_some() {
        send_to_char(g, chid, b"Busy already. \r\n");
        return;
    }
    if g.descriptors.get(vd).and_then(|d| d.snooping) == Some(di) {
        send_to_char(g, chid, b"Don't be stupid.\r\n");
        return;
    }
    let tch = g.descriptors.get(vd).and_then(|d| d.original).unwrap_or(victim);
    if g.ch(tch).level >= g.ch(chid).level {
        send_to_char(g, chid, b"You can't.\r\n");
        return;
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(victim).get_name()).into_owned();
    let lvl = g.ch(chid).level;
    g.mudlog(MudlogKind::Brf, lvl, true, &format!("(GC) {} snoops {}", name, vname));

    if let Some(prev) = g.descriptors.get(di).and_then(|d| d.snooping) {
        if let Some(d) = g.descriptors.get_mut(prev) {
            d.snoop_by = None;
        }
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.snooping = Some(vd);
    }
    if let Some(d) = g.descriptors.get_mut(vd) {
        d.snoop_by = Some(di);
    }
}

pub fn do_switch(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let Some(di) = g.ch(chid).desc else { return };
    if g.descriptors.get(di).and_then(|d| d.original).is_some() {
        send_to_char(g, chid, b"You're already switched.\r\n");
        return;
    }
    if arg.is_empty() {
        send_to_char(g, chid, b"Switch with who?\r\n");
        return;
    }
    let Some(victim) = get_char_world_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"No such character.\r\n");
        return;
    };
    if chid == victim {
        send_to_char(g, chid, b"Hee hee... we are jolly funny today, eh?\r\n");
        return;
    }
    if g.ch(victim).desc.is_some() {
        send_to_char(g, chid, b"You can't do that, the body is already in use!\r\n");
        return;
    }
    let level = g.ch(chid).level;
    if level < LVL_IMPL && !g.ch(victim).is_npc() {
        send_to_char(g, chid, b"You are not holy enough to use their body.\r\n");
        return;
    }
    let vroom = g.ch(victim).in_room;
    if level < LVL_GRGOD && crate::handler::room_flagged(g, vroom, flags::ROOM_GODROOM) {
        send_to_char(g, chid, b"You are not godly enough to use that room!\r\n");
        return;
    }
    if level < LVL_GRGOD
        && crate::handler::room_flagged(g, vroom, flags::ROOM_HOUSE)
        && !crate::house::house_can_enter(g, chid, g.world.rooms[vroom as usize].vnum as i32)
    {
        send_to_char(g, chid, b"That's private property -- no trespassing!\r\n");
        return;
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(victim).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Cmp,
        (LVL_GOD as i16).max(invis) as u8,
        true,
        &format!("(GC) {} Switched into: {}", name, vname),
    );
    if let Some(d) = g.descriptors.get_mut(di) {
        d.character = Some(victim);
        d.original = Some(chid);
    }
    g.ch_mut(victim).desc = Some(di);
    g.ch_mut(chid).desc = None;
}

pub fn return_to_char(g: &mut Game, chid: CharId) {
    let Some(di) = g.ch(chid).desc else { return };
    let Some(orig) = g.descriptors.get(di).and_then(|d| d.original) else { return };
    // Someone switched into our real body gets disconnected.
    if let Some(od) = g.try_ch(orig).and_then(|c| c.desc) {
        if let Some(d) = g.descriptors.get_mut(od) {
            d.character = None;
            d.state = ConState::Disconnect;
        }
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.character = Some(orig);
        d.original = None;
    }
    g.ch_mut(orig).desc = Some(di);
    g.ch_mut(chid).desc = None;
}

/// do_cheat: idnum 1 restores itself to IMPL.
fn do_cheat(g: &mut Game, chid: CharId) {
    if g.ch(chid).idnum != 1 {
        send_to_char(g, chid, b"You do not have access to this command.\r\n");
        return;
    }
    g.ch_mut(chid).level = LVL_IMPL;
    send_to_char(g, chid, b"Your level has been restored, for now!\r\n");
    crate::players_glue::save_char(g, chid);
}

pub fn do_return(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    let switched = g.ch(chid).desc.and_then(|di| g.descriptors.get(di)).and_then(|d| d.original);
    if !g.ch(chid).is_npc() && switched.is_none() {
        let level = g.ch(chid).level;
        do_cheat(g, chid);
        let newlevel = g.ch(chid).level;
        if !g.ch(chid).plr(flags::PLR_NOWIZLIST) && level != newlevel {
            crate::limits::run_autowiz(g);
        }
    }
    if switched.is_some() {
        send_to_char(g, chid, b"You return to your original body.\r\n");
        return_to_char(g, chid);
    }
}

// ---------------------------------------------------------------------------
// World tools
// ---------------------------------------------------------------------------

pub fn do_links(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(crate::interpreter::skip_spaces(argument));
    let (zrnum, zvnum) = if !is_number(&arg) {
        let zr = g.world.rooms[g.ch(chid).in_room as usize].zone as usize;
        (Some(zr), g.world.zones[zr].number as i32)
    } else {
        let zv = atoi(&arg);
        (g.world.zones.iter().position(|z| z.number as i32 == zv), zv)
    };
    let Some(zrnum) = zrnum else {
        send_to_char(g, chid, b"No zone was found with that number.\r\n");
        return;
    };
    let (first, last) = (g.world.zones[zrnum].bot as i32, g.world.zones[zrnum].top as i32);

    send_to_char(g, chid, format!("Zone {} is linked to the following zones:\r\n", zvnum).as_bytes());
    let mut rows: Vec<BStr> = Vec::new();
    for nr in 0..g.world.rooms.len() {
        let vnum = g.world.rooms[nr].vnum as i32;
        if vnum > last {
            break;
        }
        if vnum < first {
            continue;
        }
        for j in 0..crate::fight::dir_count(g) {
            let Some(exit) = g.world.rooms[nr].dir_option[j].as_deref() else { continue };
            let to_room = exit.to_room;
            if to_room == NOWHERE {
                continue;
            }
            let tzone = g.world.rooms[to_room as usize].zone as usize;
            if tzone == zrnum {
                continue;
            }
            let mut out = format!("{:3} ", g.world.zones[tzone].number).into_bytes();
            out.extend_from_slice(&crate::act::pad_right(
                g.world.zones[tzone].name.as_deref().unwrap_or(b""),
                30,
            ));
            out.extend_from_slice(format!(" at {:5} (", vnum).as_bytes());
            out.extend_from_slice(&crate::act::pad_right(
                mud_data::tables::DIRS[j].as_bytes(),
                5,
            ));
            out.extend_from_slice(
                format!(") ---> {:5}\r\n", g.world.rooms[to_room as usize].vnum).as_bytes(),
            );
            rows.push(out);
        }
    }
    for r in rows {
        send_to_char(g, chid, &r);
    }
}

fn mob_checkload(g: &mut Game, chid: CharId, mvnum: i32) {
    let Some(mrnum) = g.world.real_mobile(mvnum as Idx) else {
        send_to_char(g, chid, b"That mob does not exist.\r\n");
        return;
    };
    let mut out = b"Checking load info for the mob ".to_vec();
    out.extend_from_slice(g.world.mob_protos[mrnum as usize].short_descr.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"...\r\n");
    send_to_char(g, chid, &out);

    let mut rows: Vec<BStr> = Vec::new();
    for zone in 0..g.world.zones.len() {
        for zc in &g.world.zones[zone].cmds {
            if zc.command != b'M' {
                continue;
            }
            if zc.arg1 == mrnum as i32 {
                let room = &g.world.rooms[zc.arg3 as usize];
                let mut r = format!("  [{:5}] ", room.vnum).into_bytes();
                r.extend_from_slice(room.name.as_deref().unwrap_or(b""));
                r.extend_from_slice(format!(" ({} MAX)\r\n", zc.arg2).as_bytes());
                rows.push(r);
            }
        }
    }
    for r in rows {
        send_to_char(g, chid, &r);
    }
}

fn obj_checkload(g: &mut Game, chid: CharId, ovnum: i32) {
    let Some(ornum) = g.world.real_object(ovnum as Idx) else {
        send_to_char(g, chid, b"That object does not exist.\r\n");
        return;
    };
    let mut out = b"Checking load info for the obj ".to_vec();
    out.extend_from_slice(
        g.world.obj_protos[ornum as usize].short_description.as_deref().unwrap_or(b""),
    );
    out.extend_from_slice(b"...\r\n");
    send_to_char(g, chid, &out);

    let ornum = ornum as i32;
    let mut rows: Vec<BStr> = Vec::new();
    for zone in 0..g.world.zones.len() {
        let (mut lastroom_v, mut lastroom_r, mut lastmob_r) = (0i32, 0usize, 0usize);
        for zc in &g.world.zones[zone].cmds {
            let room_name = |r: usize| g.world.rooms[r].name.clone().unwrap_or_default();
            match zc.command {
                b'M' => {
                    lastroom_v = g.world.rooms[zc.arg3 as usize].vnum as i32;
                    lastroom_r = zc.arg3 as usize;
                    lastmob_r = zc.arg1 as usize;
                }
                b'O' => {
                    lastroom_v = g.world.rooms[zc.arg3 as usize].vnum as i32;
                    lastroom_r = zc.arg3 as usize;
                    if zc.arg1 == ornum {
                        let mut r = format!("  [{:5}] ", lastroom_v).into_bytes();
                        r.extend_from_slice(&room_name(lastroom_r));
                        r.extend_from_slice(format!(" ({} Max)\r\n", zc.arg2).as_bytes());
                        rows.push(r);
                    }
                }
                b'P' => {
                    if zc.arg1 == ornum {
                        let mut r = format!("  [{:5}] ", lastroom_v).into_bytes();
                        r.extend_from_slice(&room_name(lastroom_r));
                        r.extend_from_slice(
                            format!(" (Put in another object [{} Max])\r\n", zc.arg2).as_bytes(),
                        );
                        rows.push(r);
                    }
                }
                b'G' | b'E' => {
                    if zc.arg1 == ornum {
                        let verb: &[u8] =
                            if zc.command == b'G' { b" (Given to " } else { b" (Equipped to " };
                        let mut r = format!("  [{:5}] ", lastroom_v).into_bytes();
                        r.extend_from_slice(&room_name(lastroom_r));
                        r.extend_from_slice(verb);
                        r.extend_from_slice(
                            g.world.mob_protos[lastmob_r].short_descr.as_deref().unwrap_or(b""),
                        );
                        r.extend_from_slice(
                            format!(
                                " [{}][{} Max])\r\n",
                                g.world.mob_protos[lastmob_r].vnum, zc.arg2
                            )
                            .as_bytes(),
                        );
                        rows.push(r);
                    }
                }
                b'R' => {
                    lastroom_v = g.world.rooms[zc.arg1 as usize].vnum as i32;
                    lastroom_r = zc.arg1 as usize;
                    if zc.arg2 == ornum {
                        let mut r = format!("  [{:5}] ", lastroom_v).into_bytes();
                        r.extend_from_slice(&room_name(lastroom_r));
                        r.extend_from_slice(b" (Removed from room)\r\n");
                        rows.push(r);
                    }
                }
                _ => {}
            }
        }
    }
    for r in rows {
        send_to_char(g, chid, &r);
    }
}

fn trg_checkload(g: &mut Game, chid: CharId, tvnum: i32) {
    let Some(trnum) = g.world.real_trigger(tvnum as Idx) else {
        send_to_char(g, chid, b"That trigger does not exist.\r\n");
        return;
    };
    let kind: &[u8] = match g.world.triggers[trnum as usize].attach_type {
        crate::dg::MOB_TRIGGER => b"mobile",
        crate::dg::OBJ_TRIGGER => b"object",
        _ => b"room",
    };
    let mut out = b"Checking load info for the ".to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(b" trigger '");
    out.extend_from_slice(g.world.triggers[trnum as usize].name.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"':\r\n");
    send_to_char(g, chid, &out);

    let trnum_i = trnum as i32;
    let mut found = false;
    let mut rows: Vec<BStr> = Vec::new();
    for zone in 0..g.world.zones.len() {
        let (mut lastroom_v, mut lastroom_r) = (0i32, 0usize);
        let (mut lastmob_r, mut lastobj_r) = (0usize, 0usize);
        for zc in &g.world.zones[zone].cmds {
            match zc.command {
                b'M' => {
                    lastroom_v = g.world.rooms[zc.arg3 as usize].vnum as i32;
                    lastroom_r = zc.arg3 as usize;
                    lastmob_r = zc.arg1 as usize;
                }
                b'O' => {
                    lastroom_v = g.world.rooms[zc.arg3 as usize].vnum as i32;
                    lastroom_r = zc.arg3 as usize;
                    lastobj_r = zc.arg1 as usize;
                }
                b'P' | b'G' | b'E' => lastobj_r = zc.arg1 as usize,
                // No `break`, so 'R' falls through into 'T'.
                b'R' | b'T' => {
                    if zc.command == b'R' {
                        lastroom_v = 0;
                        lastroom_r = 0;
                        lastobj_r = 0;
                        lastmob_r = 0;
                    }
                    if zc.arg2 != trnum_i {
                        continue;
                    }
                    if zc.arg1 == crate::dg::MOB_TRIGGER {
                        let mut r = format!("mob [{:5}] ", g.world.mob_protos[lastmob_r].vnum)
                            .into_bytes();
                        r.extend_from_slice(&crate::act::pad_right(
                            g.world.mob_protos[lastmob_r].short_descr.as_deref().unwrap_or(b""),
                            60,
                        ));
                        r.extend_from_slice(format!(" (zedit room {:5})\r\n", lastroom_v).as_bytes());
                        rows.push(r);
                        found = true;
                    } else if zc.arg1 == crate::dg::OBJ_TRIGGER {
                        let mut r = format!("obj [{:5}] ", g.world.obj_protos[lastobj_r].vnum)
                            .into_bytes();
                        r.extend_from_slice(&crate::act::pad_right(
                            g.world.obj_protos[lastobj_r]
                                .short_description
                                .as_deref()
                                .unwrap_or(b""),
                            60,
                        ));
                        r.extend_from_slice(format!("  (zedit room {})\r\n", lastroom_v).as_bytes());
                        rows.push(r);
                        found = true;
                    } else if zc.arg1 == crate::dg::WLD_TRIGGER {
                        let mut r = format!("room [{:5}] ", lastroom_v).into_bytes();
                        r.extend_from_slice(&crate::act::pad_right(
                            g.world.rooms[lastroom_r].name.as_deref().unwrap_or(b""),
                            60,
                        ));
                        r.extend_from_slice(b" (zedit)\r\n");
                        rows.push(r);
                        found = true;
                    }
                }
                _ => {}
            }
        }
    }
    // The prototype attach lists (`T` lines inside.mob/.obj/.wld records).
    for i in 0..g.world.mob_protos.len() {
        for &tv in &g.world.mob_protos[i].proto_script {
            if tv as i32 == tvnum {
                let mut r = format!("mob [{:5}] ", g.world.mob_protos[i].vnum).into_bytes();
                r.extend_from_slice(g.world.mob_protos[i].short_descr.as_deref().unwrap_or(b""));
                r.extend_from_slice(b"\r\n");
                rows.push(r);
                found = true;
            }
        }
    }
    for j in 0..g.world.obj_protos.len() {
        for &tv in &g.world.obj_protos[j].proto_script {
            if tv as i32 == tvnum {
                let mut r = format!("obj [{:5}] ", g.world.obj_protos[j].vnum).into_bytes();
                r.extend_from_slice(
                    g.world.obj_protos[j].short_description.as_deref().unwrap_or(b""),
                );
                r.extend_from_slice(b"\r\n");
                rows.push(r);
                found = true;
            }
        }
    }
    for k in 0..g.world.rooms.len() {
        for &tv in &g.world.rooms[k].proto_script {
            if tv as i32 == tvnum {
                let mut r = format!("room[{:5}] ", g.world.rooms[k].vnum).into_bytes();
                r.extend_from_slice(g.world.rooms[k].name.as_deref().unwrap_or(b""));
                r.extend_from_slice(b"\r\n");
                rows.push(r);
                found = true;
            }
        }
    }
    for r in rows {
        send_to_char(g, chid, &r);
    }
    if !found {
        send_to_char(g, chid, b"This trigger is not attached to anything.\r\n");
    }
}

/* ------------------------------------------------------------------ zcheck */

/* The limits act.wizard.c declares above do_zcheck. Four of them are
 * expressions over the mob being checked, so they are functions here. */
const MAX_MOB_DAM_ALLOWED: f64 = 500.0;
const MAX_DAM_ALLOWED: f64 = 50.0;
const MAX_AFFECTS_ALLOWED: i32 = 3;
const MAX_OBJ_GOLD_ALLOWED: i32 = 1_000_000;
const ZC_MAX_OBJ_WEIGHT: i32 = 1_000_000;
const ZC_MAX_OBJ_COST: i32 = 2_000_000;
const MAX_APPLY_HITROLL_TOTAL: i32 = 5;
const MAX_APPLY_DAMROLL_TOTAL: i32 = 5;
const MIN_ROOM_DESC_LENGTH: usize = 80;
const MAX_COLOUMN_WIDTH: usize = 80;
const NUM_ATTACK_TYPES: i32 = 15;

/// Zones a player should never be able to walk into.
const OFFLIMIT_ZONES: [Idx; 4] = [0, 12, 13, 14];

/// `zarmor`: the AC ceiling for each body part, and the noun the message
/// uses for it. `TOTAL_WEAR_CHECKS` is `NUM_ITEM_WEARS - 2`, which is
/// exactly this table's length -- TAKE and WIELD are the two left out.
const ZARMOR: [(usize, i32, &[u8]); flags::NUM_ITEM_WEARS - 2] = [
    (flags::ITEM_WEAR_FINGER, 10, b"Ring"),
    (flags::ITEM_WEAR_NECK, 10, b"Necklace"),
    (flags::ITEM_WEAR_BODY, 10, b"Body armor"),
    (flags::ITEM_WEAR_HEAD, 10, b"Head gear"),
    (flags::ITEM_WEAR_LEGS, 10, b"Legwear"),
    (flags::ITEM_WEAR_FEET, 10, b"Footwear"),
    (flags::ITEM_WEAR_HANDS, 10, b"Glove"),
    (flags::ITEM_WEAR_ARMS, 10, b"Armwear"),
    (flags::ITEM_WEAR_SHIELD, 10, b"Shield"),
    (flags::ITEM_WEAR_ABOUT, 10, b"Cloak"),
    (flags::ITEM_WEAR_WAIST, 10, b"Belt"),
    (flags::ITEM_WEAR_WRIST, 10, b"Wristwear"),
    (flags::ITEM_WEAR_HOLD, 10, b"Held item"),
];

/// `zaffs`, indexed by apply location and so kept in the order of the
/// `APPLY_*` constants. A `max` of -99 means no range is set and the apply
/// is skipped; `min == max` means the apply is not allowed at all, whatever
/// its value. Hitroll and damroll carry -99 because they are totalled
/// separately below.
const ZAFFS: [(i32, i32, &[u8]); flags::NUM_APPLIES] = [
    (0, -99, b"unused0"),
    (-5, 3, b"strength"),
    (-5, 3, b"dexterity"),
    (-5, 3, b"intelligence"),
    (-5, 3, b"wisdom"),
    (-5, 3, b"constitution"),
    (-5, 3, b"charisma"),
    (0, 0, b"class"),
    (0, 0, b"level"),
    (-10, 10, b"age"),
    (-50, 50, b"character weight"),
    (-50, 50, b"character height"),
    (-50, 50, b"mana"),
    (-50, 50, b"hit points"),
    (-50, 50, b"movement"),
    (0, 0, b"gold"),
    (0, 0, b"experience"),
    (-10, 10, b"magical AC"),
    (0, -99, b"hitroll"),
    (0, -99, b"damroll"),
    (-2, 2, b"saving throw (paralysis)"),
    (-2, 2, b"saving throw (rod)"),
    (-2, 2, b"saving throw (death)"),
    (-2, 2, b"saving throw (breath)"),
    (-2, 2, b"saving throw (spell)"),
];

/// `%-30s`: pad on the right, never truncate.
fn zc_pad30(s: &[u8]) -> BStr {
    let mut out = s.to_vec();
    while out.len() < 30 {
        out.push(b' ');
    }
    out
}

/// `strncmp(s, "   ", 3)` -- true when the text is NOT indented, which is
/// also true for anything shorter than three characters.
fn unindented(s: &[u8]) -> bool {
    !s.starts_with(b"   ")
}

/// zcheck: walk one zone's mob, object and room prototypes and report every
/// limit they break.
///
/// One thing carried over deliberately: `is_number` decides which branch
/// the argument takes, so anything that is not a number -- "." included,
/// and "frobozz" too -- means the zone the caller is standing in.
///
/// All four scans cover their whole table, including the last entry: the
/// unlinked-rooms verdict is wrong if even one room is left out.
pub fn do_zcheck(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);

    let zrnum = if !is_number(&arg) {
        Some(g.world.rooms[g.ch(chid).in_room as usize].zone as usize)
    } else {
        g.world.real_zone(atoi(&arg) as Idx).map(|z| z as usize)
    };
    let Some(zrnum) = zrnum else {
        send_to_char(g, chid, b"Check what zone ?\r\n");
        return;
    };
    let zone_number = g.world.zones[zrnum].number;
    send_to_char(g, chid, format!("Checking zone {}!\r\n", zone_number).as_bytes());

    let mut buf: BStr = Vec::new();
    let mut found;

    /* ---- mobs ---- */
    send_to_char(g, chid, b"Checking Mobs for limits...\r\n");
    let top_of_mobt = g.world.mob_protos.len();
    for i in 0..top_of_mobt {
        if real_zone_by_thing(g, g.world.mob_protos[i].vnum as i32) != Some(zrnum) {
            continue;
        }
        found = false;
        let m = &g.world.mob_protos[i];
        let level = m.level;
        let max_damroll = (level / 5).max(1);
        let max_hitroll = (level / 3).max(1);
        let max_gold = level * 3000;
        let max_exp = level * level * 120;

        if m.keywords.as_deref() == Some(b"mob unfinished".as_ref()) {
            found = true;
            buf.extend_from_slice(b"- Alias hasn't been set.\r\n");
        }
        if m.short_descr.as_deref() == Some(b"the unfinished mob".as_ref()) {
            found = true;
            buf.extend_from_slice(b"- Short description hasn't been set.\r\n");
        }
        if m.long_descr.as_deref().unwrap_or(b"").starts_with(b"An unfinished mob stands here.") {
            found = true;
            buf.extend_from_slice(b"- Long description hasn't been set.\r\n");
        }
        let desc = m.ddescription.as_deref().unwrap_or(b"");
        if !desc.is_empty() {
            if desc.starts_with(b"It looks unfinished.") {
                found = true;
                buf.extend_from_slice(b"- Description hasn't been set.\r\n");
            } else if unindented(desc) {
                found = true;
                buf.extend_from_slice(b"- Description hasn't been formatted. (/fi)\r\n");
            }
        }
        if level > LVL_IMPL as i32 {
            found = true;
            buf.extend_from_slice(
                format!("- Is level {} (limit: 1-{})\r\n", level, LVL_IMPL).as_bytes(),
            );
        }
        if m.damroll > max_damroll {
            found = true;
            buf.extend_from_slice(
                format!("- Damroll of {} is too high (limit: {})\r\n", m.damroll, max_damroll)
                    .as_bytes(),
            );
        }
        if m.hitroll > max_hitroll {
            found = true;
            buf.extend_from_slice(
                format!("- Hitroll of {} is too high (limit: {})\r\n", m.hitroll, max_hitroll)
                    .as_bytes(),
            );
        }
        /* Average damage per round, damroll included. */
        let avg_dam = (m.damsizedice as f64 / 2.0) * m.damnodice as f64 + m.damroll as f64;
        if avg_dam > MAX_MOB_DAM_ALLOWED {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- average damage of {:4.1} is too high (limit: {})\r\n",
                    avg_dam, MAX_MOB_DAM_ALLOWED as i32
                )
                .as_bytes(),
            );
        }
        if m.damsizedice == 1 && m.damnodice == 1 && level == 0 {
            found = true;
            let (yel, nrm) = (cc(g, chid, C_NRM, KYEL).to_vec(), cc(g, chid, C_NRM, KNRM).to_vec());
            buf.extend_from_slice(b"- Needs to be fixed - ");
            buf.extend_from_slice(&yel);
            buf.extend_from_slice(b"Autogenerate!");
            buf.extend_from_slice(&nrm);
            buf.extend_from_slice(b"\r\n");
        }
        let m = &g.world.mob_protos[i];
        let act = flags::FlagSet(m.act);
        let aff = flags::FlagSet(m.affected_by);
        if act.is_set(flags::MOB_AGGRESSIVE)
            && (act.is_set(flags::MOB_AGGR_GOOD)
                || act.is_set(flags::MOB_AGGR_EVIL)
                || act.is_set(flags::MOB_AGGR_NEUTRAL))
        {
            found = true;
            buf.extend_from_slice(b"- Both aggresive and agressive to align.\r\n");
        }
        if m.gold > max_gold {
            found = true;
            buf.extend_from_slice(
                format!("- Set to {} Gold (limit : {}).\r\n", m.gold, max_gold).as_bytes(),
            );
        }
        if m.exp > max_exp {
            found = true;
            buf.extend_from_slice(
                format!("- Has {} experience (limit: {})\r\n", m.exp, max_exp).as_bytes(),
            );
        }
        if aff.is_set(flags::AFF_CHARM) || aff.is_set(flags::AFF_POISON) {
            found = true;
            let charm: &[u8] = if aff.is_set(flags::AFF_CHARM) { b"CHARM" } else { b"" };
            let poison: &[u8] = if aff.is_set(flags::AFF_POISON) { b"POISON" } else { b"" };
            buf.extend_from_slice(b"- Has illegal affection bits set (");
            buf.extend_from_slice(charm);
            buf.push(b' ');
            buf.extend_from_slice(poison);
            buf.extend_from_slice(b")\r\n");
        }
        if !act.is_set(flags::MOB_SENTINEL) && !act.is_set(flags::MOB_STAY_ZONE) {
            found = true;
            buf.extend_from_slice(b"- Neither SENTINEL nor STAY_ZONE bits set.\r\n");
        }
        if act.is_set(flags::MOB_SPEC) {
            found = true;
            buf.extend_from_slice(b"- SPEC flag needs to be removed.\r\n");
        }

        if found {
            let (vnum, name) =
                (g.world.mob_protos[i].vnum, g.world.mob_protos[i].short_descr.clone());
            let cyn = cc(g, chid, C_NRM, KCYN).to_vec();
            let yel = cc(g, chid, C_NRM, KYEL).to_vec();
            let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
            let mut head: BStr = Vec::new();
            head.extend_from_slice(&cyn);
            head.extend_from_slice(format!("[{:5}]", vnum).as_bytes());
            head.extend_from_slice(&yel);
            head.push(b' ');
            head.extend_from_slice(&zc_pad30(name.as_deref().unwrap_or(b"<None>")));
            head.extend_from_slice(b": ");
            head.extend_from_slice(&nrm);
            head.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &head);
            let out = std::mem::take(&mut buf);
            send_to_char(g, chid, &out);
        }
        buf.clear();
    }

    /* ---- objects ---- */
    send_to_char(g, chid, b"\r\nChecking Objects for limits...\r\n");
    let top_of_objt = g.world.obj_protos.len();
    for i in 0..top_of_objt {
        if real_zone_by_thing(g, g.world.obj_protos[i].vnum as i32) != Some(zrnum) {
            continue;
        }
        found = false;
        let o = &g.world.obj_protos[i];
        let wear = flags::FlagSet(o.wear_flags);

        match o.type_flag {
            flags::ITEM_MONEY => {
                if o.values[0] > MAX_OBJ_GOLD_ALLOWED {
                    found = true;
                    buf.extend_from_slice(
                        format!(
                            "- Is worth {} (money limit {} coins).\r\n",
                            o.values[0], MAX_OBJ_GOLD_ALLOWED
                        )
                        .as_bytes(),
                    );
                }
            }
            flags::ITEM_WEAPON => {
                if o.values[3] >= NUM_ATTACK_TYPES {
                    found = true;
                    buf.extend_from_slice(
                        format!("- has out of range attack type {}.\r\n", o.values[3]).as_bytes(),
                    );
                }
                let avg = ((o.values[2] + 1) as f64 / 2.0) * o.values[1] as f64;
                if avg > MAX_DAM_ALLOWED {
                    found = true;
                    buf.extend_from_slice(
                        format!(
                            "- Damroll is {:2.1} (limit {})\r\n",
                            avg, MAX_DAM_ALLOWED as i32
                        )
                        .as_bytes(),
                    );
                }
            }
            flags::ITEM_ARMOR => {
                let ac = o.values[0];
                for (bit, allowed, message) in ZARMOR {
                    if wear.is_set(bit) && ac > allowed {
                        found = true;
                        buf.extend_from_slice(format!("- Has AC {} (", ac).as_bytes());
                        buf.extend_from_slice(message);
                        buf.extend_from_slice(format!(" limit is {})\r\n", allowed).as_bytes());
                    }
                }
            }
            _ => {}
        }

        if !wear.is_set(flags::ITEM_WEAR_TAKE) {
            if o.cost != 0
                || (o.weight != 0 && o.type_flag != flags::ITEM_FOUNTAIN)
                || o.cost_per_day != 0
            {
                found = true;
                buf.extend_from_slice(
                    format!(
                        "- is NO_TAKE, but has cost ({}) weight ({}) or rent ({}) set.\r\n",
                        o.cost, o.weight, o.cost_per_day
                    )
                    .as_bytes(),
                );
            }
        } else {
            /* B99: the C writes `cost == 0 && (found=1) && type != ITEM_TRASH`,
             * raising the flag before it looks at the type -- so a zero-cost
             * piece of trash printed a header with nothing under it. The type
             * is tested first here, and in the C. */
            if o.type_flag != flags::ITEM_TRASH && o.cost == 0 {
                found = true;
                buf.extend_from_slice(b"- has 0 cost (min. 1).\r\n");
            }
            if o.weight == 0 {
                found = true;
                buf.extend_from_slice(b"- has 0 weight (min. 1).\r\n");
            }
            if o.weight > ZC_MAX_OBJ_WEIGHT {
                found = true;
                buf.extend_from_slice(
                    format!(
                        "  Weight is too high: {} (limit  {}).\r\n",
                        o.weight, ZC_MAX_OBJ_WEIGHT
                    )
                    .as_bytes(),
                );
            }
            if o.cost > ZC_MAX_OBJ_COST {
                found = true;
                buf.extend_from_slice(
                    format!("- has {} cost (max {}).\r\n", o.cost, ZC_MAX_OBJ_COST).as_bytes(),
                );
            }
        }

        if o.level > LVL_IMMORT as i32 - 1 {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- has min level set to {} (max {}).\r\n",
                    o.level,
                    LVL_IMMORT as i32 - 1
                )
                .as_bytes(),
            );
        }

        let has_action = o.action_description.as_deref().is_some_and(|d| !d.is_empty());
        if has_action
            && o.type_flag != flags::ITEM_STAFF
            && o.type_flag != flags::ITEM_WAND
            && o.type_flag != flags::ITEM_SCROLL
            && o.type_flag != flags::ITEM_NOTE
        {
            found = true;
            buf.extend_from_slice(
                b"- has action_description set, but is inappropriate type.\r\n",
            );
        }

        let affs = o.affected.iter().filter(|a| a.modifier != 0).count() as i32;
        if affs > MAX_AFFECTS_ALLOWED {
            found = true;
            buf.extend_from_slice(
                format!("- has {} affects (limit {}).\r\n", affs, MAX_AFFECTS_ALLOWED).as_bytes(),
            );
        }

        for a in &o.affected {
            /* The C indexes zaffs by the location with no bound; every
             * location the loaders produce is inside the table, and a
             * location outside it has no defined behaviour to copy. */
            let Some(&(min, max, message)) = ZAFFS.get(a.location as usize) else { continue };
            if max != -99 && (a.modifier > max || a.modifier < min || min == max) {
                found = true;
                buf.extend_from_slice(b"- apply to ");
                buf.extend_from_slice(message);
                buf.extend_from_slice(
                    format!(" is {} (limit {} - {}).\r\n", a.modifier, min, max).as_bytes(),
                );
            }
        }

        /* +hit and +dam are totalled, because of +hit_n_dam. */
        let mut tohit = 0;
        let mut todam = 0;
        for a in &o.affected {
            if a.location == flags::APPLY_HITROLL {
                tohit += a.modifier;
            }
            if a.location == flags::APPLY_DAMROLL {
                todam += a.modifier;
            }
        }
        if todam.abs() > MAX_APPLY_DAMROLL_TOTAL {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- total damroll {} out of range (limit +/-{}.\r\n",
                    todam, MAX_APPLY_DAMROLL_TOTAL
                )
                .as_bytes(),
            );
        }
        if tohit.abs() > MAX_APPLY_HITROLL_TOTAL {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- total hitroll {} out of range (limit +/-{}).\r\n",
                    tohit, MAX_APPLY_HITROLL_TOTAL
                )
                .as_bytes(),
            );
        }

        if o.ex_descriptions.iter().any(|e| unindented(e.description.as_deref().unwrap_or(b""))) {
            found = true;
            buf.extend_from_slice(b"- has unformatted extra description\r\n");
        }

        if found {
            let (vnum, name) =
                (g.world.obj_protos[i].vnum, g.world.obj_protos[i].short_description.clone());
            let mut head: BStr = Vec::new();
            head.extend_from_slice(format!("[{:5}] ", vnum).as_bytes());
            head.extend_from_slice(&zc_pad30(name.as_deref().unwrap_or(b"")));
            head.extend_from_slice(b": \r\n");
            send_to_char(g, chid, &head);
            let out = std::mem::take(&mut buf);
            send_to_char(g, chid, &out);
        }
        buf.clear();
    }

    /* ---- rooms ---- */
    send_to_char(g, chid, b"\r\nChecking Rooms for limits...\r\n");
    let top_of_world = g.world.rooms.len();
    found = false;
    for i in 0..top_of_world {
        if g.world.rooms[i].zone as usize != zrnum {
            continue;
        }
        let dir_count = crate::fight::dir_count(g);
        for j in 0..dir_count {
            let Some(exroom) = g.world.rooms[i].dir_option[j].as_deref().map(|e| e.to_room) else {
                continue;
            };
            if exroom == NOWHERE {
                continue;
            }
            if g.world.rooms[exroom as usize].zone as usize == zrnum {
                continue;
            }
            for off in OFFLIMIT_ZONES {
                if g.world.real_zone(off).map(|z| z as usize)
                    == Some(g.world.rooms[exroom as usize].zone as usize)
                {
                    found = true;
                    buf.extend_from_slice(
                        format!(
                            "- Exit {} cannot connect to {} (zone off limits).\r\n",
                            tables::DIRS[j],
                            g.world.rooms[exroom as usize].vnum
                        )
                        .as_bytes(),
                    );
                }
            }
        }

        let r = &g.world.rooms[i];
        let rf = flags::FlagSet(r.room_flags);
        /* B99: the C is the one check in the command that writes without
         * raising `found`, and the room loop clears the buffer inside
         * `if (found)` where the two loops above clear it unconditionally --
         * so the line was swallowed, and then blamed on whichever later room
         * in the zone reported next. Both halves fixed, here and in the C. */
        if rf.is_set(flags::ROOM_ATRIUM)
            || rf.is_set(flags::ROOM_HOUSE)
            || rf.is_set(flags::ROOM_HOUSE_CRASH)
            || rf.is_set(flags::ROOM_OLC)
            || rf.is_set(flags::ROOM_BFS_MARK)
        {
            found = true;
            let part = |set: bool, s: &'static [u8]| -> &'static [u8] { if set { s } else { b"" } };
            buf.extend_from_slice(b"- Has illegal affection bits set (");
            buf.extend_from_slice(part(rf.is_set(flags::ROOM_ATRIUM), b"ATRIUM"));
            buf.push(b' ');
            buf.extend_from_slice(part(rf.is_set(flags::ROOM_HOUSE), b"HOUSE"));
            buf.push(b' ');
            buf.extend_from_slice(part(rf.is_set(flags::ROOM_HOUSE_CRASH), b"HCRSH"));
            buf.push(b' ');
            buf.extend_from_slice(part(rf.is_set(flags::ROOM_OLC), b"OLC"));
            buf.push(b' ');
            buf.extend_from_slice(part(rf.is_set(flags::ROOM_BFS_MARK), b"*"));
            buf.extend_from_slice(b")\r\n");
        }

        let desc = r.description.as_deref().unwrap_or(b"");
        if MIN_ROOM_DESC_LENGTH != 0 && desc.len() < MIN_ROOM_DESC_LENGTH {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- Room description is too short. ({:04} of min. {} characters).\r\n",
                    desc.len(),
                    MIN_ROOM_DESC_LENGTH
                )
                .as_bytes(),
            );
        }
        if unindented(desc) {
            found = true;
            buf.extend_from_slice(
                b"- Room description not formatted with indent (/fi in the editor).\r\n",
            );
        }
        /* strcspn: how much text there is before the first \r or \n. */
        let first_line = desc.iter().position(|c| *c == b'\r' || *c == b'\n').unwrap_or(desc.len());
        if first_line > MAX_COLOUMN_WIDTH {
            found = true;
            buf.extend_from_slice(
                format!(
                    "- Room description not wrapped at {} chars (/fi in the editor).\r\n",
                    MAX_COLOUMN_WIDTH
                )
                .as_bytes(),
            );
        }
        if r.ex_descriptions.iter().any(|e| unindented(e.description.as_deref().unwrap_or(b""))) {
            found = true;
            buf.extend_from_slice(b"- has unformatted extra description\r\n");
        }

        if found {
            let (vnum, name) = (g.world.rooms[i].vnum, g.world.rooms[i].name.clone());
            let mut head: BStr = Vec::new();
            head.extend_from_slice(format!("[{:5}] ", vnum).as_bytes());
            head.extend_from_slice(&zc_pad30(name.as_deref().unwrap_or(b"An unnamed room")));
            head.extend_from_slice(b": \r\n");
            send_to_char(g, chid, &head);
            let out = std::mem::take(&mut buf);
            send_to_char(g, chid, &out);
        }
        buf.clear();
        found = false;
    }

    /* How much of the zone is not linked to anything. */
    let mut rooms_in_zone = 0;
    let mut unlinked = 0;
    for i in 0..top_of_world {
        if g.world.rooms[i].zone as usize != zrnum {
            continue;
        }
        rooms_in_zone += 1;
        let dir_count = crate::fight::dir_count(g);
        if (0..dir_count).all(|j| g.world.rooms[i].dir_option[j].is_none()) {
            unlinked += 1;
        }
    }
    if unlinked * 3 > rooms_in_zone {
        send_to_char(g, chid, b"More than 1/3 of the rooms are not linked.\r\n");
    }
}

pub fn do_checkloadstatus(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf1, buf2, _) = two_arguments(argument);
    if buf1.is_empty() || buf2.is_empty() || !buf2[0].is_ascii_digit() {
        send_to_char(g, chid, b"Checkload <M | O | T> <vnum>\r\n");
        return;
    }
    match buf1[0].to_ascii_lowercase() {
        b'm' => mob_checkload(g, chid, atoi(&buf2)),
        b'o' => obj_checkload(g, chid, atoi(&buf2)),
        b't' => trg_checkload(g, chid, atoi(&buf2)),
        _ => {}
    }
}

pub fn do_peace(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    act(
        g,
        b"As $n makes a strange arcane gesture, a golden light descends\r\nfrom the heavens stopping all the fighting.\r\n",
        false,
        Some(chid),
        None,
        None,
        comm::TO_ROOM,
    );
    let room = g.ch(chid).in_room;
    comm::send_to_room(g, room, b"Everything is quite peaceful now.\r\n");
    for vict in g.rooms[room as usize].people.clone() {
        if g.try_ch(vict).is_none() {
            continue;
        }
        if g.ch(vict).fighting.is_some() {
            crate::fight::stop_fighting(g, vict);
        }
        if g.ch(vict).is_npc() {
            crate::mobact::clear_memory(g, vict);
        }
    }
}

/// The two fixed refusals, kept out of the body so the flow reads.
const ZDELETE_SYNTAX: &[u8] = b"Syntax: zdelete <zone vnum>   what deleting it would cost\r\n        zdelete confirm      delete the zone that report named\r\n\r\nIt always takes both. Nothing is deleted by a command that has not first shown you what the deletion does, and the second command names no zone, so there is no number left to mistype.\r\n";
const ZDELETE_NOTHING_ARMED: &[u8] = b"You have not been shown a zone to delete. Run zdelete <zone vnum> first, and read what it says.\r\n";
const ZDELETE_CONFIRM_BARE: &[u8] = b"zdelete confirm takes nothing after it -- the zone is the one the report named, so that there is no number here to get wrong.\r\n";

/// Has this zone been through zdelete?
///
/// The test is absence from the zone index, not the presence of a .deleted
/// file: that is the invariant zdelete establishes, and the one thing nothing
/// else undoes by accident. save_zone rewrites a .zon whenever a zone is
/// saved or unlocked, so a file-pair test disarms itself the first time that
/// happens; only create_world_index puts a line back. A zone still in the
/// table but missing from the index is one deleted since the boot.
fn zone_was_deleted(g: &Game, zvnum: i32) -> bool {
    let idx = g.lib_dir.join("world").join("zon").join("index");
    let Ok(data) = std::fs::read(&idx) else {
        // Say so rather than quietly answering "not deleted": an unreadable
        // zone index dooms the next boot anyway, but it should be heard from
        // here and not from the reboot.
        return false;
    };
    // Compare the number, not the text: every other reader of these files
    // takes a token or scans a number, so padding is invisible to them -- and
    // the recovery zdelete prints has the operator editing them by hand.
    for line in data.split(|&b| b == b'\n') {
        let mut line = line.to_vec();
        while matches!(line.last(), Some(b'\r') | Some(b' ')) {
            line.pop();
        }
        if line.first() == Some(&b'$') {
            break;
        }
        if !line.is_empty() && atoi(&line) == zvnum {
            return false;
        }
    }
    true
}

/// Who, if anyone, has an editor open on this zone.
fn zdelete_editor_open(g: &Game, zrnum: usize) -> Option<Vec<u8>> {
    for (&di, olc) in g.olc.iter() {
        if olc.zone_num != zrnum as i32 {
            continue;
        }
        let name = g
            .descriptors
            .get(di)
            .and_then(|d| d.character)
            .and_then(|c| g.try_ch(c))
            .map(|c| c.get_name().to_vec())
            .unwrap_or_else(|| b"Someone".to_vec());
        return Some(name);
    }
    None
}

/// Everything the deletion costs, printed before anything is touched. This is
/// the only place any of it is ever said: after the reboot the exits are
/// simply gone, and the SYSERRs name vnums rather than the zone that took
/// them with it.
fn zdelete_report(g: &mut Game, chid: CharId, zrnum: usize, zvnum: i32, n: [usize; 6]) {
    let s = |v: usize| if v == 1 { "" } else { "s" };
    let zname = String::from_utf8_lossy(
        &g.world.zones[zrnum].name.clone().unwrap_or_default(),
    )
    .into_owned();
    let mut out = format!("Zone {}: {}\r\n", zvnum, zname).into_bytes();
    out.extend_from_slice(
        format!(
            "  {} room{}, {} mobile{}, {} object{}, {} trigger{}, {} shop{}, {} quest{}\r\n",
            n[0], s(n[0]), n[1], s(n[1]), n[2], s(n[2]),
            n[3], s(n[3]), n[4], s(n[4]), n[5], s(n[5])
        )
        .as_bytes(),
    );

    // Exits leading in. After the reboot they resolve nowhere: they stop being
    // listed, cannot be walked, and nothing says why.
    let mut inbound = 0usize;
    let mut listed = 0usize;
    for i in 0..g.world.rooms.len() {
        if g.world.rooms[i].zone as usize == zrnum {
            continue;
        }
        for door in 0..NUM_OF_DIRS {
            let Some(ex) = g.world.rooms[i].dir_option[door].as_ref() else {
                continue;
            };
            let to = ex.to_room_vnum;
            if to < 0 || g.real_room(to).is_none() {
                continue;
            }
            let tr = g.real_room(to).unwrap();
            if g.world.rooms[tr as usize].zone as usize != zrnum {
                continue;
            }
            if inbound == 0 {
                out.extend_from_slice(
                    b"\r\nExits leading into it, which will become dead ends with no message:\r\n",
                );
            }
            inbound += 1;
            if listed < 20 {
                listed += 1;
                let from_zone = g.world.zones[g.world.rooms[i].zone as usize].number;
                out.extend_from_slice(
                    format!(
                        "  zone {:<4} room {:<6} {:<5} -> {}\r\n",
                        from_zone,
                        g.world.rooms[i].vnum,
                        mud_data::tables::DIRS[door],
                        to
                    )
                    .as_bytes(),
                );
            }
        }
    }
    if inbound > listed {
        out.extend_from_slice(format!("  ... and {} more.\r\n", inbound - listed).as_bytes());
    }
    if inbound == 0 {
        out.extend_from_slice(b"\r\nNo exits lead into it.\r\n");
    }

    // Triggers are the second way out of the zone, and a noisier one: a
    // prototype elsewhere that attaches one logs a SYSERR on every boot.
    let mut attached = 0usize;
    let mut owners = 0usize;
    let count = |g: &Game, scripts: &[Idx], own: &mut usize, att: &mut usize| {
        let mut hit = false;
        for &t in scripts {
            if real_zone_by_thing(g, t as i32) == Some(zrnum) {
                *att += 1;
                if !hit {
                    hit = true;
                    *own += 1;
                }
            }
        }
    };
    for i in 0..g.world.rooms.len() {
        if g.world.rooms[i].zone as usize == zrnum {
            continue;
        }
        let sc = g.world.rooms[i].proto_script.clone();
        count(g, &sc, &mut owners, &mut attached);
    }
    for i in 0..g.world.mob_protos.len() {
        if real_zone_by_thing(g, g.world.mob_protos[i].vnum as i32) == Some(zrnum) {
            continue;
        }
        let sc = g.world.mob_protos[i].proto_script.clone();
        count(g, &sc, &mut owners, &mut attached);
    }
    for i in 0..g.world.obj_protos.len() {
        if real_zone_by_thing(g, g.world.obj_protos[i].vnum as i32) == Some(zrnum) {
            continue;
        }
        let sc = g.world.obj_protos[i].proto_script.clone();
        count(g, &sc, &mut owners, &mut attached);
    }
    if attached > 0 {
        out.extend_from_slice(
            format!(
                "{} trigger attachment{} on {} thing{} outside the zone name a trigger inside it; each attachment logs a SYSERR at every boot.\r\n",
                attached, s(attached), owners, s(owners)
            )
            .as_bytes(),
        );
    }

    // Reset commands live in the zone file of the zone that runs them, so
    // another zone loading this one's mobs or objects keeps trying.
    let mut resets = 0usize;
    for z in 0..g.world.zones.len() {
        if z == zrnum {
            continue;
        }
        for c in 0..g.world.zones[z].cmds.len() {
            if zdelete_cmd_touches(g, zrnum, z, c) {
                resets += 1;
            }
        }
    }
    if resets > 0 {
        out.extend_from_slice(
            format!(
                "{} reset command{} in other zones load{} something from this one; each is a SYSERR at every boot.\r\n",
                resets, s(resets), if resets == 1 { "s" } else { "" }
            )
            .as_bytes(),
        );
    }

    // The counts above are prototypes. The instances are what players own.
    if n[2] > 0 {
        let carried = g
            .object_list
            .iter()
            .filter(|&&oid| {
                g.try_obj(oid).is_some_and(|o| {
                    let r = o.item_number as usize;
                    r < g.world.obj_protos.len()
                        && real_zone_by_thing(g, g.world.obj_protos[r].vnum as i32) == Some(zrnum)
                })
            })
            .count();
        out.extend_from_slice(
            format!(
                "\r\nEvery object of this zone is destroyed by the reboot -- {} of them exist right now, and so is every copy in a player file, rent file or house. The player is not told; the item is dropped as it loads.\r\n",
                carried
            )
            .as_bytes(),
        );
    }

    // character_list, not the descriptors: a linkless body is still standing
    // in the zone and is the one player who cannot be told anything at all.
    let players = g
        .character_list
        .iter()
        .filter(|&&id| {
            g.try_ch(id).is_some_and(|c| {
                !c.is_npc()
                    && c.in_room != NOWHERE
                    && g.world.rooms[c.in_room as usize].zone as usize == zrnum
            })
        })
        .count();
    if players > 0 {
        out.extend_from_slice(
            format!(
                "{} player{} standing in it; they will be moved to a start room by the reboot.\r\n",
                players,
                if players == 1 { " is" } else { "s are" }
            )
            .as_bytes(),
        );
    }

    out.extend_from_slice(
        b"\r\nThe zone's files are set aside, not erased, and nothing in memory moves: it is gone at the next reboot.\r\nTo go through with it, the next command is just:  zdelete confirm\r\n",
    );
    send_to_char(g, chid, &out);
}

/// Does one of this reset command's arguments name something the zone owns?
/// Such a command lives in another zone's file, so it survives the deletion
/// and logs a zone-file SYSERR on every boot afterwards.
fn zdelete_cmd_touches(g: &Game, zrnum: usize, z: usize, c: usize) -> bool {
    let cmd = &g.world.zones[z].cmds[c];
    // renum converted these to rnums at boot, so each is an index; a command
    // whose lookup failed holds a sentinel, hence the bounds tests.
    let mob = |r: i32| {
        r >= 0
            && (r as usize) < g.world.mob_protos.len()
            && real_zone_by_thing(g, g.world.mob_protos[r as usize].vnum as i32) == Some(zrnum)
    };
    let obj = |r: i32| {
        r >= 0
            && (r as usize) < g.world.obj_protos.len()
            && real_zone_by_thing(g, g.world.obj_protos[r as usize].vnum as i32) == Some(zrnum)
    };
    let room = |r: i32| {
        r >= 0
            && (r as usize) < g.world.rooms.len()
            && g.world.rooms[r as usize].zone as usize == zrnum
    };
    let trg = |r: i32| {
        r >= 0
            && (r as usize) < g.world.triggers.len()
            && real_zone_by_thing(g, g.world.triggers[r as usize].vnum as i32) == Some(zrnum)
    };
    match cmd.command {
        b'M' => mob(cmd.arg1) || room(cmd.arg3),
        b'O' => obj(cmd.arg1) || room(cmd.arg3),
        b'G' | b'E' => obj(cmd.arg1),
        b'P' => obj(cmd.arg1) || obj(cmd.arg3),
        b'D' => room(cmd.arg1),
        b'R' => room(cmd.arg1) || obj(cmd.arg2),
        b'T' => trg(cmd.arg2) || room(cmd.arg3),
        b'V' => room(cmd.arg3),
        _ => false,
    }
}

/// zdelete: take a zone out of the world.
///
/// The zone's files are set aside and its lines come out of each world index
/// and index.mini, so the next boot does not read it. Nothing in memory moves:
/// the rooms, mobiles and objects the running game holds stay where they are
/// and every rnum in play stays valid. Removing the members one at a time
/// instead would renumber everything above them, on a running MUD, for a zone
/// that is going away at the reboot regardless.
///
/// It always takes two commands, and the second names no zone. The first
/// reports and arms; the second deletes what that report named. A report
/// stands for exactly one command: the interpreter cancels it for anything
/// that is not zdelete, and this function ends it on every path but a fresh
/// report, so a confirmation can only ever land on the zone its operator was
/// looking at when they typed it.
pub fn do_zdelete(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    const WORLD_FILES: [&str; 7] = ["wld", "mob", "obj", "zon", "shp", "qst", "trg"];

    let (arg, rest) = one_argument(argument);
    let (arg2, _) = one_argument(rest);

    // Take the standing report away first; one goes back only where a fresh
    // report is printed below.
    let di = g.ch(chid).desc;
    let was_armed = di.and_then(|d| g.descriptors.get(d).and_then(|x| x.zdelete_armed));
    if let Some(d) = di {
        if let Some(x) = g.descriptors.get_mut(d) {
            x.zdelete_armed = None;
        }
    }
    let confirming = arg.eq_ignore_ascii_case(b"confirm");
    if let Some(z) = was_armed {
        if !confirming {
            let m = format!("The pending deletion of zone {} is cancelled.\r\n", z);
            send_to_char(g, chid, m.as_bytes());
        }
    }

    if arg.is_empty() {
        send_to_char(g, chid, ZDELETE_SYNTAX);
        return;
    }

    let zvnum: i32;
    if confirming {
        if !arg2.is_empty() {
            send_to_char(g, chid, ZDELETE_CONFIRM_BARE);
            if let Some(z) = was_armed {
                let m = format!("The pending deletion of zone {} is cancelled; run zdelete {} again if you meant it.\r\n", z, z);
                send_to_char(g, chid, m.as_bytes());
            }
            return;
        }
        let Some(z) = was_armed else {
            send_to_char(g, chid, ZDELETE_NOTHING_ARMED);
            return;
        };
        zvnum = z;
    } else {
        if !arg2.is_empty() {
            let m = format!("zdelete takes the zone on its own. Run zdelete {} to see what deleting it would cost, then zdelete confirm to go through with it.\r\n", String::from_utf8_lossy(&arg));
            send_to_char(g, chid, m.as_bytes());
            return;
        }
        // A word would otherwise come back as zone 0 and report on the void.
        if !arg.first().is_some_and(|c| c.is_ascii_digit()) {
            let m = format!("There is no zone {}.\r\n", String::from_utf8_lossy(&arg));
            send_to_char(g, chid, m.as_bytes());
            return;
        }
        zvnum = atoi(&arg);
    }

    let Some(zrnum) = g.world.zones.iter().position(|z| z.number as i32 == zvnum) else {
        let m = if confirming {
            format!("Zone {} is gone already.\r\n", zvnum)
        } else {
            format!("There is no zone {}.\r\n", zvnum)
        };
        send_to_char(g, chid, m.as_bytes());
        return;
    };

    // A zone holding a start room cannot go: the next boot would have nowhere
    // to put anyone who was not already somewhere valid.
    let starts = [
        g.config.mortal_start_room,
        g.config.immort_start_room,
        g.config.frozen_start_room,
        0,
    ];
    if starts.iter().any(|&v| {
        g.real_room(v)
            .is_some_and(|r| g.world.rooms[r as usize].zone as usize == zrnum)
    }) {
        let m = format!(
            "Zone {} holds a start room. Move the start rooms in cedit first.\r\n",
            zvnum
        );
        send_to_char(g, chid, m.as_bytes());
        return;
    }

    let rooms = g.world.rooms.iter().filter(|r| r.zone as usize == zrnum).count();
    let mob_v: Vec<i32> = g.world.mob_protos.iter().map(|m| m.vnum as i32).collect();
    let obj_v: Vec<i32> = g.world.obj_protos.iter().map(|o| o.vnum as i32).collect();
    let trg_v: Vec<i32> = g.world.triggers.iter().map(|t| t.vnum as i32).collect();
    let shp_v: Vec<i32> = g.world.shops.iter().map(|s| s.vnum as i32).collect();
    let qst_v: Vec<i32> = g.world.quests.iter().map(|q| q.vnum as i32).collect();
    let inz = |v: &i32| real_zone_by_thing(g, *v) == Some(zrnum);
    let counts = [
        rooms,
        mob_v.iter().filter(|v| inz(v)).count(),
        obj_v.iter().filter(|v| inz(v)).count(),
        trg_v.iter().filter(|v| inz(v)).count(),
        shp_v.iter().filter(|v| inz(v)).count(),
        qst_v.iter().filter(|v| inz(v)).count(),
    ];

    if !confirming {
        zdelete_report(g, chid, zrnum, zvnum, counts);
        if let Some(d) = di {
            if let Some(x) = g.descriptors.get_mut(d) {
                x.zdelete_armed = Some(zvnum);
            }
        }
        return;
    }

    // NOBUILD below stops anyone entering an editor on the zone, but it cannot
    // reach one that is already open: that save writes the files right back.
    if let Some(who) = zdelete_editor_open(g, zrnum) {
        let mut out = who;
        out.extend_from_slice(
            format!(" has an editor open on zone {}. A save from it would write the zone's files back after this; have them leave the editor first.\r\n", zvnum).as_bytes(),
        );
        send_to_char(g, chid, &out);
        return;
    }

    // An earlier deletion's files are not ours to overwrite: a rename would
    // replace them without a word, and what they hold is the only copy of a
    // zone somebody already deleted once.
    for ext in WORLD_FILES {
        let p = g
            .lib_dir
            .join("world")
            .join(ext)
            .join(format!("{}.{}.deleted", zvnum, ext));
        if p.exists() {
            let m = format!("world/{}/{}.{}.deleted is already there from an earlier deletion.\r\nMove it aside first; this would overwrite it.\r\n", ext, zvnum, ext);
            send_to_char(g, chid, m.as_bytes());
            return;
        }
    }

    let mut moved = 0usize;
    for ext in WORLD_FILES {
        // Index first, and only rename if it worked: an index naming a file
        // that is gone stops the next boot dead, while a file that no index
        // names is simply never read.
        if !crate::olc::genzon::remove_world_index(g, zvnum, ext) {
            let had = if moved == 1 { " has" } else { "s have" };
            let m = format!("Could not rewrite the {} index files -- see the syslog.\r\nZone {} is now only partly deleted: {} file{} already been set aside and world/{}/{}.{} was left in place. The world still boots, because a file no index names is never read. Put it right by hand before trying again.\r\n", ext, zvnum, moved, had, ext, zvnum, ext);
            send_to_char(g, chid, m.as_bytes());
            return;
        }
        let dir = g.lib_dir.join("world").join(ext);
        // Not every zone has all seven; a missing one is nothing to report.
        let from = dir.join(format!("{}.{}", zvnum, ext));
        let to = dir.join(format!("{}.{}.deleted", zvnum, ext));
        if std::fs::rename(from, to).is_ok() {
            moved += 1;
        }
    }

    // The zone is still in memory, so every editor still works on it and a
    // save would write its file back -- unindexed for most types, but the
    // shop and quest writers re-index too. NOBUILD is what can_edit_zone
    // already honours; the pending saves go because the save-list flush at
    // saveall and at shutdown does not consult it.
    set_zone_flag(g, zrnum, flags::ZONE_NOBUILD, true);
    g.save_list.retain(|&(z, _)| z as i32 != zvnum);

    let who = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    let zname =
        String::from_utf8_lossy(&g.world.zones[zrnum].name.clone().unwrap_or_default()).into_owned();
    let s = |v: usize| if v == 1 { "" } else { "s" };
    let m = format!(
        "(GC) {} has deleted zone {} ({}): {} file{} set aside, {} room{}, {} mobile{}, {} object{}.",
        who, zvnum, zname, moved, s(moved), counts[0], s(counts[0]),
        counts[1], s(counts[1]), counts[2], s(counts[2])
    );
    g.mudlog(MudlogKind::Brf, LVL_GOD.max(invis as u8), true, &m);

    let was = if moved == 1 { " was" } else { "s were" };
    let m = format!("Zone {} is out of the world index; {} file{} set aside as <name>.deleted, and the zone is now NOBUILD so that no editor writes it back.\r\nIt stays loaded until the next reboot.\r\nTo put it back: rename those files to their original names, and add each one to its index -- and to its index.mini if it was listed there -- in ASCENDING numeric order. The index order is the order the tables are built in, and they are binary-searched, so a line appended at the end loads the zone but leaves parts of the world unreachable.\r\n", zvnum, moved, was);
    send_to_char(g, chid, m.as_bytes());
}

pub fn do_zpurge(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let mut zone = 0usize;
    let mut purge_all = false;
    if arg.first() == Some(&b'.') || arg.is_empty() {
        zone = g.world.rooms[g.ch(chid).in_room as usize].zone as usize;
    } else if is_number(&arg) {
        match g.world.zones.iter().position(|z| z.number as i32 == atoi(&arg)) {
            Some(z) => zone = z,
            None => {
                send_to_char(g, chid, b"That zone doesn't exist!\r\n");
                return;
            }
        }
    } else if arg.first() == Some(&b'*') {
        purge_all = true;
    } else {
        send_to_char(g, chid, b"That isn't a valid zone number!\r\n");
        return;
    }
    if g.ch(chid).level < LVL_GOD && !crate::dg::commands::can_edit_zone(g, chid, Some(zone)) {
        send_to_char(g, chid, b"You can only purge your own zone!\r\n");
        return;
    }
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    if !purge_all {
        let (bot, top, num, zname) = {
            let z = &g.world.zones[zone];
            (z.bot as i32, z.top as i32, z.number, z.name.clone().unwrap_or_default())
        };
        for vroom in bot..=top {
            if let Some(r) = g.real_room(vroom) {
                purge_room(g, r);
            }
        }
        let mut out = format!("Purged zone #{}: ", num).into_bytes();
        out.extend_from_slice(&zname);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GRGOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} purged zone {} ({})", name, num, String::from_utf8_lossy(&zname)),
        );
    } else {
        for room in 0..g.world.rooms.len() {
            purge_room(g, room as RoomRnum);
        }
        send_to_char(g, chid, b"Purged world.\r\n");
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GRGOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} purged entire world.", name),
        );
    }
}

pub fn do_saveall(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).level < LVL_BUILDER {
        send_to_char(g, chid, b"You are not holy enough to use this privilege.\r\n");
        return;
    }
    crate::db::save_all(g);
    crate::house::house_save_all(g);
    send_to_char(g, chid, b"World and house files saved.\r\n");
}

pub fn do_wizupdate(g: &mut Game, chid: CharId, _argument: &[u8], _cmd: usize, _subcmd: i32) {
    crate::limits::run_autowiz(g);
    send_to_char(g, chid, b"Wizlists updated.\r\n");
}

/// The `[%3d] %-*s %-1s` row shared by the zlock/zunlock listings.
fn zone_lock_row(g: &Game, chid: CharId, zn: usize) -> BStr {
    let (grn, cyn, yel, nrm) = (
        cc(g, chid, C_SPR, KGRN),
        cc(g, chid, C_SPR, KCYN),
        cc(g, chid, C_SPR, KYEL),
        cc(g, chid, C_SPR, KNRM),
    );
    let z = &g.world.zones[zn];
    let name = z.name.as_deref().unwrap_or(b"");
    let width = crate::act::other::count_color_chars(name) + 30;
    let mut row = b"[".to_vec();
    row.extend_from_slice(grn);
    row.extend_from_slice(format!("{:3}", z.number).as_bytes());
    row.extend_from_slice(nrm);
    row.extend_from_slice(b"] ");
    row.extend_from_slice(cyn);
    row.extend_from_slice(&crate::act::pad_right(name, width));
    row.push(b' ');
    row.extend_from_slice(yel);
    row.extend_from_slice(z.builders.as_deref().unwrap_or(b"None."));
    row.extend_from_slice(nrm);
    row.extend_from_slice(b"\r\n");
    row
}

/// The `Usage: <yellow>...<normal>` header both zone-lock commands print.
fn zlock_usage(g: &mut Game, chid: CharId, cmd: &[u8], list_line: &[u8], body: &[&[u8]]) {
    let yel = cc(g, chid, C_SPR, KYEL).to_vec();
    let nrm = cc(g, chid, C_SPR, KNRM).to_vec();
    let mut out = b"Usage: ".to_vec();
    out.extend_from_slice(&yel);
    out.extend_from_slice(cmd);
    out.extend_from_slice(b" <zone number>");
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
    if list_line.is_empty() {
        return;
    }
    let mut out = yel.clone();
    out.extend_from_slice(list_line);
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n\r\n");
    send_to_char(g, chid, &out);
    for line in body {
        send_to_char(g, chid, line);
    }
}

pub fn do_zlock(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, arg2, _) = two_arguments(argument);
    if arg.is_empty() {
        zlock_usage(
            g,
            chid,
            b"zlock",
            b"       zlock list",
            &[
                b"Locks a zone so that building or editing is not possible.\r\n",
                b"The 'list' shows all currently locked zones.\r\n",
                b"'zlock all' will lock every zone with the GRID flag set.\r\n",
                b"'zlock all all' will lock every zone in the MUD.\r\n",
            ],
        );
        return;
    }
    let mut counter = 0;
    let mut fail = false;
    if is_abbrev(&arg, b"all") {
        if g.ch(chid).level < LVL_GRGOD {
            send_to_char(g, chid, b"You do not have sufficient access to lock all zones.\r\n");
            return;
        }
        let grid_only = arg2.is_empty();
        if grid_only || is_abbrev(&arg2, b"all") {
            for zn in 0..g.world.zones.len() {
                let nobuild = zone_flagged(g, zn, flags::ZONE_NOBUILD);
                let grid = zone_flagged(g, zn, flags::ZONE_GRID);
                if nobuild || (grid_only && !grid) {
                    continue;
                }
                counter += 1;
                set_zone_flag(g, zn, flags::ZONE_NOBUILD, true);
                if crate::db::save_zone(g, zn) {
                    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                    let num = g.world.zones[zn].number;
                    g.log(format!("(GC) {} has locked zone {}", name, num));
                } else {
                    fail = true;
                }
            }
        }
        if counter == 0 {
            send_to_char(g, chid, b"There are no unlocked zones to lock!\r\n");
            return;
        }
        if fail {
            send_to_char(g, chid, b"Unable to save zone changes.  Check syslog!\r\n");
            return;
        }
        send_to_char(g, chid, format!("{} zones have now been locked.\r\n", counter).as_bytes());
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Brf,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} has locked ALL zones!", name),
        );
        return;
    }
    if is_abbrev(&arg, b"list") {
        let mut rows: Vec<BStr> = Vec::new();
        for zn in 0..g.world.zones.len() {
            if !zone_flagged(g, zn, flags::ZONE_NOBUILD) {
                continue;
            }
            if counter == 0 {
                rows.push(b"Locked Zones\r\n".to_vec());
            }
            rows.push(zone_lock_row(g, chid, zn));
            counter += 1;
        }
        for r in rows {
            send_to_char(g, chid, &r);
        }
        if counter == 0 {
            send_to_char(g, chid, b"There are currently no locked zones!\r\n");
        }
        return;
    }
    let znvnum = atoi(&arg);
    if znvnum == 0 {
        zlock_usage(g, chid, b"zlock", b"", &[]);
        return;
    }
    let Some(zn) = g.world.zones.iter().position(|z| z.number as i32 == znvnum) else {
        send_to_char(g, chid, b"That zone does not exist!\r\n");
        return;
    };
    if !zone_builder_access(g, chid, zn, znvnum) {
        send_to_char(g, chid, b"You do not have sufficient access to lock that zone!\r\n");
        return;
    }
    if zone_flagged(g, zn, flags::ZONE_NOBUILD) {
        send_to_char(g, chid, format!("Zone {} is already locked!\r\n", znvnum).as_bytes());
        return;
    }
    set_zone_flag(g, zn, flags::ZONE_NOBUILD, true);
    if crate::db::save_zone(g, zn) {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GRGOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} has locked zone {}", name, znvnum),
        );
    } else {
        send_to_char(g, chid, b"Unable to save zone changes.  Check syslog!\r\n");
    }
}

pub fn do_zunlock(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        zlock_usage(
            g,
            chid,
            b"zunlock",
            b"       zunlock list",
            &[
                b"Unlocks a 'locked' zone to allow building or editing.\r\n",
                b"The 'list' shows all currently unlocked zones.\r\n",
                b"'zunlock all' will unlock every zone in the MUD.\r\n",
            ],
        );
        return;
    }
    let mut counter = 0;
    let mut fail = false;
    if is_abbrev(&arg, b"all") {
        if g.ch(chid).level < LVL_GRGOD {
            send_to_char(g, chid, b"You do not have sufficient access to lock zones.\r\n");
            return;
        }
        for zn in 0..g.world.zones.len() {
            if !zone_flagged(g, zn, flags::ZONE_NOBUILD) {
                continue;
            }
            // A deleted zone is locked too, and save_zone below would write
            // its .zon back with nothing in the index naming it. Skip it here
            // as the single-zone form refuses it, or "unlock every zone"
            // quietly undoes part of a deletion nobody mentioned.
            if zone_was_deleted(g, g.world.zones[zn].number as i32) {
                let m = format!("Zone {} has been deleted; leaving it locked.\r\n",
                                g.world.zones[zn].number);
                send_to_char(g, chid, m.as_bytes());
                continue;
            }
            counter += 1;
            set_zone_flag(g, zn, flags::ZONE_NOBUILD, false);
            if crate::db::save_zone(g, zn) {
                let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                let num = g.world.zones[zn].number;
                g.log(format!("(GC) {} has unlocked zone {}", name, num));
            } else {
                fail = true;
            }
        }
        if counter == 0 {
            send_to_char(g, chid, b"There are no locked zones to unlock!\r\n");
            return;
        }
        if fail {
            send_to_char(g, chid, b"Unable to save zone changes.  Check syslog!\r\n");
            return;
        }
        send_to_char(g, chid, format!("{} zones have now been unlocked.\r\n", counter).as_bytes());
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Brf,
            (LVL_GOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} has unlocked ALL zones!", name),
        );
        return;
    }
    if is_abbrev(&arg, b"list") {
        let mut rows: Vec<BStr> = Vec::new();
        for zn in 0..g.world.zones.len() {
            if zone_flagged(g, zn, flags::ZONE_NOBUILD) {
                continue;
            }
            if counter == 0 {
                rows.push(b"Unlocked Zones\r\n".to_vec());
            }
            rows.push(zone_lock_row(g, chid, zn));
            counter += 1;
        }
        for r in rows {
            send_to_char(g, chid, &r);
        }
        if counter == 0 {
            send_to_char(g, chid, b"There are currently no unlocked zones!\r\n");
        }
        return;
    }
    let znvnum = atoi(&arg);
    if znvnum == 0 {
        zlock_usage(g, chid, b"zunlock", b"", &[]);
        return;
    }
    let Some(zn) = g.world.zones.iter().position(|z| z.number as i32 == znvnum) else {
        send_to_char(g, chid, b"That zone does not exist!\r\n");
        return;
    };
    if !zone_builder_access(g, chid, zn, znvnum) {
        send_to_char(g, chid, b"You do not have sufficient access to unlock that zone!\r\n");
        return;
    }
    if !zone_flagged(g, zn, flags::ZONE_NOBUILD) {
        send_to_char(g, chid, format!("Zone {} is already unlocked!\r\n", znvnum).as_bytes());
        return;
    }
        // A zone that zdelete took out of the world is locked as well, and
    // unlocking it would write its .zon straight back -- save_zone below does
    // exactly that -- leaving a file the index no longer names.
    if zone_was_deleted(g, znvnum) {
        let m = format!("Zone {} has been deleted -- it is no longer in the zone index.\r\nUnlocking it would write its .zon back with nothing naming it, and reopen it to every editor. Restore the zone first.\r\n", znvnum);
        send_to_char(g, chid, m.as_bytes());
        return;
    }
    set_zone_flag(g, zn, flags::ZONE_NOBUILD, false);
    if crate::db::save_zone(g, zn) {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Nrm,
            (LVL_GRGOD as i16).max(invis) as u8,
            true,
            &format!("(GC) {} has unlocked zone {}", name, znvnum),
        );
    } else {
        send_to_char(g, chid, b"Unable to save zone changes.  Check syslog!\r\n");
    }
}

pub fn zone_flagged(g: &Game, zn: usize, bit: usize) -> bool {
    g.world.zones[zn].zone_flags[bit / 32] & (1 << (bit % 32)) != 0
}

fn set_zone_flag(g: &mut Game, zn: usize, bit: usize, on: bool) {
    if on {
        g.world.zones[zn].zone_flags[bit / 32] |= 1 << (bit % 32);
    } else {
        g.world.zones[zn].zone_flags[bit / 32] &= !(1 << (bit % 32));
    }
}

/// The zlock/zunlock builder gate.
fn zone_builder_access(g: &Game, chid: CharId, zn: usize, znvnum: i32) -> bool {
    if g.ch(chid).level >= LVL_GRGOD {
        return true;
    }
    let name = g.ch(chid).get_name().to_vec();
    if let Some(b) = g.world.zones[zn].builders.as_deref() {
        if crate::handler::isname(&name, b) {
            return true;
        }
    }
    g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone) == znvnum
}

// ---------------------------------------------------------------------------
// oset and its four setters
// ---------------------------------------------------------------------------

pub fn do_oset(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    const USAGE: &[u8] = b"Usage: \r\nOptions: alias, apply, longdesc, shortdesc\r\n> oset <object> <option> <value>\r\n";
    if g.ch(chid).is_npc() || g.ch(chid).desc.is_none() {
        send_to_char(g, chid, b"oset is only usable by connected players.\r\n");
        return;
    }
    let (arg, rest) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, USAGE);
        return;
    }
    let carrying = g.ch(chid).carrying.clone();
    let room = g.ch(chid).in_room;
    let contents = g.rooms[room as usize].contents.clone();
    let obj = crate::handler::get_obj_in_list_vis(g, chid, &arg, None, &carrying)
        .or_else(|| crate::handler::get_obj_in_list_vis(g, chid, &arg, None, &contents));
    let Some(obj) = obj else {
        let mut out = b"You don't seem to have ".to_vec();
        out.extend_from_slice(crate::act::informative::an_for(&arg));
        out.push(b' ');
        out.extend_from_slice(&arg);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
        return;
    };
    let (arg2, rest2) = one_argument(rest);
    if arg2.is_empty() {
        send_to_char(g, chid, USAGE);
        return;
    }
    let value = crate::interpreter::skip_spaces(rest2).to_vec();
    let success;
    if is_abbrev(&arg2, b"alias") {
        success = oset_alias(g, obj, &value);
        if success {
            send_to_char(g, chid, b"Object alias set.\r\n");
            return;
        }
    } else if is_abbrev(&arg2, b"longdesc") {
        success = oset_long_description(g, obj, &value);
        if success {
            send_to_char(g, chid, b"Object long description set.\r\n");
            return;
        }
    } else if is_abbrev(&arg2, b"shortdesc") {
        success = oset_short_description(g, obj, &value);
        if success {
            send_to_char(g, chid, b"Object short description set.\r\n");
            return;
        }
    } else if is_abbrev(&arg2, b"apply") {
        success = oset_apply(g, obj, &value);
        if success {
            send_to_char(g, chid, b"Object apply set.\r\n");
            return;
        }
    } else {
        send_to_char(g, chid, USAGE);
        return;
    }
    if !success {
        let mut out = arg2.clone();
        out.extend_from_slice(b" was unsuccessful.\r\n");
        send_to_char(g, chid, &out);
    }
}

fn oset_alias(g: &mut Game, oid: mud_data::ids::ObjId, argument: &[u8]) -> bool {
    if argument.is_empty() {
        return false;
    }
    g.obj_mut(oid).name = Some(argument.to_vec());
    true
}

fn oset_short_description(g: &mut Game, oid: mud_data::ids::ObjId, argument: &[u8]) -> bool {
    if argument.is_empty() {
        return false;
    }
    g.obj_mut(oid).short_description = Some(argument.to_vec());
    true
}

fn oset_long_description(g: &mut Game, oid: mud_data::ids::ObjId, argument: &[u8]) -> bool {
    if argument.is_empty() {
        return false;
    }
    g.obj_mut(oid).description = Some(argument.to_vec());
    true
}

/// oset_apply: `<slot> <apply> <modifier>`.
fn oset_apply(g: &mut Game, oid: mud_data::ids::ObjId, argument: &[u8]) -> bool {
    let (arg1, arg2, rest) = two_arguments(argument);
    let (arg3, _) = one_argument(rest);
    if arg1.is_empty() || arg2.is_empty() || arg3.is_empty() {
        return false;
    }
    if !is_number(&arg1) || !is_number(&arg2) || !is_number(&arg3) {
        return false;
    }
    let loc = atoi(&arg1);
    let apply = atoi(&arg2);
    let modifier = atoi(&arg3);
    if !(0..MAX_OBJ_AFFECT as i32).contains(&loc) {
        return false;
    }
    if !(0..mud_data::tables::APPLY_TYPES.len() as i32).contains(&apply) {
        return false;
    }
    let o = g.obj_mut(oid);
    o.affected[loc as usize].location = apply;
    o.affected[loc as usize].modifier = modifier;
    true
}

// ---------------------------------------------------------------------------
// plist / changelog / file
// ---------------------------------------------------------------------------

const PLIST_FORMAT: &[u8] =
    b"Usage: plist [minlev[-maxlev]] [-n name] [-d days] [-h hours] [-i] [-m]";

pub fn do_plist(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let mut buf = crate::interpreter::skip_spaces(argument).to_vec();
    let mut name_search: BStr = Vec::new();
    let (mut low, mut high) = (0i32, LVL_IMPL as i32);
    let (mut low_day, mut high_day) = (0i32, 10000i32);
    let (mut low_hr, mut high_hr) = (0i32, 24i32);

    while !buf.is_empty() {
        let (arg, buf1) = half_chop(&buf);
        if arg.first().is_some_and(|c| c.is_ascii_digit()) {
            let (a, b) = parse_range_pub(&arg);
            low = a;
            high = b.unwrap_or(a);
            buf = buf1;
        } else if arg.first() == Some(&b'-') {
            match arg.get(1).copied().unwrap_or(0) {
                b'l' => {
                    let (a, rest) = half_chop(&buf1);
                    let (x, y) = parse_range_pub(&a);
                    low = x;
                    if let Some(y) = y {
                        high = y;
                    }
                    buf = rest;
                }
                b'n' => {
                    let (a, rest) = half_chop(&buf1);
                    name_search = a;
                    buf = rest;
                }
                b'i' => {
                    buf = buf1;
                    low = LVL_IMMORT as i32;
                }
                b'm' => {
                    buf = buf1;
                    high = LVL_IMMORT as i32 - 1;
                }
                b'd' => {
                    let (a, rest) = half_chop(&buf1);
                    let (x, y) = parse_range_pub(&a);
                    low_day = x;
                    high_day = y.unwrap_or(x);
                    buf = rest;
                }
                b'h' => {
                    let (a, rest) = half_chop(&buf1);
                    let (x, y) = parse_range_pub(&a);
                    low_hr = x;
                    high_hr = y.unwrap_or(x);
                    buf = rest;
                }
                _ => {
                    let mut out = PLIST_FORMAT.to_vec();
                    out.extend_from_slice(b"\r\n");
                    send_to_char(g, chid, &out);
                    return;
                }
            }
        } else {
            let mut out = PLIST_FORMAT.to_vec();
            out.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &out);
            return;
        }
    }

    let cyn = cc(g, chid, C_NRM, KCYN).to_vec();
    let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
    let mut out = b"\tW[ Id] (Lv) Name         Last\tn\r\n".to_vec();
    out.extend_from_slice(&cyn);
    out.extend_from_slice(b"-------------------------------------");
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n");

    let mut count = 0;
    for i in 0..g.player_table.len() {
        let p = &g.player_table[i];
        if p.level < low || p.level > high {
            continue;
        }
        if !name_search.is_empty() && !name_search.eq_ignore_ascii_case(&p.name) {
            continue;
        }
        let (away_days, away_hours) =
            crate::gametime::real_time_passed_hours_days(g.now - p.last);
        if away_days > high_day as i64 || away_days < low_day as i64 {
            continue;
        }
        if away_hours > high_hr as i64 || away_hours < low_hr as i64 {
            continue;
        }
        let timestr = ctime_like(p.last, g.tz_offset_secs);
        let mut name = p.name.clone();
        if let Some(c) = name.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        out.extend_from_slice(format!("[{:3}] ({:2}) ", p.id, p.level).as_bytes());
        out.extend_from_slice(&crate::act::pad_right(&name, 15));
        out.push(b' ');
        out.extend_from_slice(timestr.as_bytes());
        out.extend_from_slice(b"\r\n");
        count += 1;
    }
    out.extend_from_slice(&cyn);
    out.extend_from_slice(b"-------------------------------------");
    out.extend_from_slice(&nrm);
    out.extend_from_slice(format!("\r\n{} players listed.\r\n", count).as_bytes());
    crate::act::informative::page_string(g, chid, &out);
}

/// Parse `a-b`. The second value is None when it is absent.
pub fn parse_range_pub(arg: &[u8]) -> (i32, Option<i32>) {
    let a = atoi(arg);
    let rest = arg.iter().position(|&c| c == b'-').map(|p| &arg[p + 1..]);
    match rest {
        Some(r) if !r.is_empty() && (r[0].is_ascii_digit() || r[0] == b'-') => (a, Some(atoi(r))),
        _ => (a, None),
    }
}

/// do_changelog: prepend a dated entry to the
/// changelog, merging into today's block when the header already matches.
pub fn do_changelog(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    if argument.is_empty() {
        send_to_char(g, chid, b"Usage: changelog <change>\r\n");
        return;
    }
    let path = g.lib_dir.join("..").join("changelog");
    let bak = g.lib_dir.join("..").join("changelog.bak");
    if std::fs::rename(&path, &bak).is_err() {
        let msg = format!("SYSERR: Error making backup changelog file ({})", bak.display());
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    }
    let Ok(old) = std::fs::read(&bak) else {
        let msg = format!("SYSERR: Error opening backup changelog file ({})", bak.display());
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    };

    let mut out: Vec<u8> = Vec::new();
    let mut lines = old.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let mut last_buf: BStr = Vec::new();
    let mut current: Option<BStr> = None;
    for line in lines.by_ref() {
        // get_line skips blank and '*' lines.
        if line.is_empty() || line.first() == Some(&b'*') {
            continue;
        }
        if line.first() != Some(&b'[') {
            out.extend_from_slice(line);
            out.push(b'\n');
        } else {
            last_buf = line.to_vec();
            current = Some(line.to_vec());
            break;
        }
    }

    // "%b %d %Y" — "Aug 23 2026".
    let stamp = {
        let c = ctime_like(g.now, g.tz_offset_secs);
        let p: Vec<&str> = c.split_whitespace().collect();
        format!("{} {:02} {}", p[1], p[2].parse::<i32>().unwrap_or(0), p[4])
    };
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let header = format!("[{}] - {}", stamp, name).into_bytes();

    out.extend_from_slice(&header);
    out.push(b'\n');
    out.extend_from_slice(b"  ");
    out.extend_from_slice(&argument);
    out.push(b'\n');
    if header != last_buf {
        if let Some(line) = current {
            out.extend_from_slice(&line);
            out.push(b'\n');
        }
    }
    for line in lines {
        if line.is_empty() || line.first() == Some(&b'*') {
            continue;
        }
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    if let Err(e) = std::fs::write(&path, &out) {
        let msg = format!("SYSERR: Error opening new changelog file ({}): {}", path.display(), e);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    }
    send_to_char(g, chid, b"Change added.\r\n");
}

/// do_file's table: (name, level, path relative to
/// the working directory, read-backwards).
const FILE_FIELDS: [(&[u8], u8, &str, bool); 17] = [
    (b"xnames", LVL_GOD, "misc/xnames", true),
    (b"levels", LVL_GOD, "../log/levels", true),
    (b"rip", LVL_GOD, "../log/rip", true),
    (b"players", LVL_GOD, "../log/newplayers", true),
    (b"rentgone", LVL_GOD, "../log/rentgone", true),
    (b"errors", LVL_GOD, "../log/errors", true),
    (b"godcmds", LVL_GOD, "../log/godcmds", true),
    (b"syslog", LVL_GOD, "../syslog", true),
    (b"crash", LVL_GOD, "../syslog.CRASH", true),
    (b"help", LVL_GOD, "../log/help", true),
    (b"changelog", LVL_GOD, "../changelog", false),
    (b"deletes", LVL_GOD, "../log/delete", true),
    (b"restarts", LVL_GOD, "../log/restarts", true),
    (b"usage", LVL_GOD, "../log/usage", true),
    (b"badpws", LVL_GOD, "../log/badpws", true),
    (b"olc", LVL_GOD, "../log/olc", true),
    (b"trigger", LVL_GOD, "../log/trigger", true),
];

/// Raw lines, as `fgetc` counts them.
fn raw_lines(data: &[u8]) -> Vec<&[u8]> {
    data.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).collect()
}

/// get_line's view: blank and `*` lines are skipped.
fn get_lines(lines: &[&[u8]]) -> Vec<Vec<u8>> {
    lines
        .iter()
        .filter(|l| !l.is_empty() && l.first() != Some(&b'*'))
        .map(|l| l.to_vec())
        .collect()
}

pub fn do_file(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    const DEF_LINES: i32 = 15;
    const MAX_LINES: i32 = 300;
    let argument = crate::interpreter::skip_spaces(argument).to_vec();
    let level = g.ch(chid).level;

    if argument.is_empty() {
        send_to_char(g, chid, b"USAGE: file <filename> <num lines>\r\n\r\nFile options:\r\n");
        let rows: Vec<BStr> = FILE_FIELDS
            .iter()
            .filter(|(_, lvl, _, _)| *lvl <= level)
            .map(|(name, _, path, _)| {
                let mut out = crate::act::pad_right(name, 15);
                out.extend_from_slice(path.as_bytes());
                out.extend_from_slice(b"\r\n");
                out
            })
            .collect();
        for r in rows {
            send_to_char(g, chid, &r);
        }
        return;
    }

    let (field, value, _) = two_arguments(&argument);
    let Some(l) = FILE_FIELDS.iter().position(|(name, _, _, _)| name.starts_with(&field[..])) else {
        let mut out = b"'".to_vec();
        out.extend_from_slice(&field);
        out.extend_from_slice(b"' is not a valid file.\r\n");
        send_to_char(g, chid, &out);
        return;
    };
    let (fname, flevel, fpath, backwards) = FILE_FIELDS[l];
    if level < flevel {
        let mut out = b"You have not achieved a high enough level to view '".to_vec();
        out.extend_from_slice(fname);
        out.extend_from_slice(b"'.\r\n");
        send_to_char(g, chid, &out);
        return;
    }
    let req_lines = if value.is_empty() {
        DEF_LINES
    } else if !value[0].is_ascii_digit() {
        let mut out = b"'".to_vec();
        out.extend_from_slice(&value);
        out.extend_from_slice(b"' is not a valid number of lines to view.\r\n");
        send_to_char(g, chid, &out);
        return;
    } else {
        atoi(&value).min(MAX_LINES)
    };

    let path = g.lib_dir.join(fpath);
    let Ok(data) = std::fs::read(&path) else {
        send_to_char(
            g,
            chid,
            format!("The file {} can not be opened.\r\n", fpath).as_bytes(),
        );
        let msg = format!("SYSERR: Error opening file {} using 'file' command.", fpath);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    };

    // file_sizeof counts one past EOF (fgetc runs once more before feof).
    let req_file_size = data.len() + 1;
    let req_file_lines = data.iter().filter(|&&c| c == b'\n').count() as i32;

    let mut buf = format!(
        "\tgFile:\tn {}\tg; Min. Level to read:\tn {}\tg; File Location:\tn {}\tg\r\nFile size (bytes):\tn {}\tg; Total num lines:\tn {}\r\n",
        String::from_utf8_lossy(fname),
        flevel,
        fpath,
        req_file_size,
        req_file_lines
    )
    .into_bytes();

    let raw = raw_lines(&data);
    let lines = if backwards && req_lines < req_file_lines {
        buf.extend_from_slice(b"\tgReading from the tail of the file.\tn\r\n\r\n");
        // file_tail fast-forwards over *raw* newlines, then
        // reads what is left through get_line — so a comment or blank inside
        // the tail is skipped after the seek, not before it.
        let skip = ((req_file_lines - req_lines).max(0) as usize).min(raw.len());
        get_lines(&raw[skip..])
    } else {
        buf.extend_from_slice(b"\tgReading from the head of the file.\tn\r\n\r\n");
        get_lines(&raw)
    };
    let mut lines_read = 0;
    for line in lines.iter().take(req_lines.max(0) as usize) {
        buf.extend_from_slice(line);
        buf.extend_from_slice(b"\r\n");
        lines_read += 1;
    }

    if lines_read == req_file_lines {
        buf.extend_from_slice(
            format!("\r\n\tgEntire file returned (\tn{} \tglines).\tn\r\n", lines_read).as_bytes(),
        );
    } else if lines_read == MAX_LINES {
        buf.extend_from_slice(
            format!("\r\n\tgMaximum number of \tn{} \tglines returned.\tn\r\n", lines_read)
                .as_bytes(),
        );
    } else {
        buf.extend_from_slice(format!("\r\n{} \tglines returned.\tn\r\n", lines_read).as_bytes());
    }
    crate::act::informative::page_string(g, chid, &buf);
}

// ---------------------------------------------------------------------------
// skillset
// ---------------------------------------------------------------------------

pub fn do_skillset(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use mud_data::spells::{spell_info, TOP_SPELL_DEFINE, UNUSED_SPELLNAME};
    let (name, rest) = one_argument(argument);

    if name.is_empty() {
        send_to_char(
            g,
            chid,
            b"Syntax: skillset <name> '<skill>' <value>\r\nSkill being one of the following:\r\n",
        );
        let mut out: BStr = Vec::new();
        let mut qend = 0;
        for i in 0..=TOP_SPELL_DEFINE {
            let n = spell_info(i).name;
            if n == UNUSED_SPELLNAME {
                continue;
            }
            out.extend_from_slice(&crate::act::pad_left(n.as_bytes(), 18));
            if qend % 4 == 3 {
                out.extend_from_slice(b"\r\n");
            }
            qend += 1;
        }
        if qend % 4 != 0 {
            out.extend_from_slice(b"\r\n");
        }
        send_to_char(g, chid, &out);
        return;
    }

    let Some(vict) = get_char_world_vis(g, chid, &name, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    let argument = crate::interpreter::skip_spaces(rest).to_vec();
    let pc = g.ch(vict).class as usize;
    let pl = g.ch(vict).level as i32;

    if argument.is_empty() {
        send_to_char(g, chid, b"Skill name expected.\r\n");
        return;
    }
    if argument[0] != b'\'' {
        send_to_char(g, chid, b"Skill must be enclosed in: ''\r\n");
        return;
    }
    let mut qend = 1;
    while qend < argument.len() && argument[qend] != b'\'' {
        qend += 1;
    }
    if argument.get(qend) != Some(&b'\'') {
        send_to_char(g, chid, b"Skill must be enclosed in: ''\r\n");
        return;
    }
    let helpbuf = argument[1..qend].to_ascii_lowercase();
    let Some(skill) = crate::spec::find_skill_num(&helpbuf).filter(|&s| s > 0) else {
        send_to_char(g, chid, b"Unrecognized skill.\r\n");
        return;
    };
    let (buf, _) = one_argument(&argument[qend + 1..]);
    if buf.is_empty() {
        send_to_char(g, chid, b"Learned value expected.\r\n");
        return;
    }
    let value = atoi(&buf);
    if value < 0 {
        send_to_char(g, chid, b"Minimum value for learned is 0.\r\n");
        return;
    }
    if value > 100 {
        send_to_char(g, chid, b"Max value for learned is 100.\r\n");
        return;
    }
    if g.ch(vict).is_npc() {
        send_to_char(g, chid, b"You can't set NPC skills.\r\n");
        return;
    }
    let info = spell_info(skill);
    let min_level = info.min_level[pc.min(3)];
    if min_level >= LVL_IMMORT as i32 && pl < LVL_IMMORT as i32 {
        // min_level is LVL_IMMORT both for a skill no mortal may learn and for a
        // class that may not learn it, so the two have to be told apart here or a
        // warrior's skill offered to a mage reads as though nobody can learn it.
        let learners: Vec<usize> = (0..info.min_level.len())
            .filter(|&c| info.min_level[c] < LVL_IMMORT as i32)
            .collect();
        if learners.is_empty() {
            send_to_char(
                g,
                chid,
                format!("{} cannot be learned by mortals.\r\n", info.name).as_bytes(),
            );
        } else {
            let cls = String::from_utf8_lossy(crate::act::informative::PC_CLASS_TYPES[pc.min(3)])
                .into_owned();
            send_to_char(
                g,
                chid,
                format!("{} cannot be learned by the {} class.\r\n", info.name, cls)
                    .as_bytes(),
            );
            send_to_char(g, chid, b"It is learned by:\r\n");
            for c in learners {
                let name =
                    String::from_utf8_lossy(crate::act::informative::PC_CLASS_TYPES[c]).into_owned();
                send_to_char(
                    g,
                    chid,
                    format!("  {:<12} at level {}.\r\n", name, info.min_level[c]).as_bytes(),
                );
            }
        }
        return;
    } else if min_level > pl {
        let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
        let cls = String::from_utf8_lossy(crate::act::informative::PC_CLASS_TYPES[pc.min(3)])
            .into_owned();
        send_to_char(g, chid, format!("{} is a level {} {}.\r\n", vname, pl, cls).as_bytes());
        send_to_char(
            g,
            chid,
            format!(
                "The minimum level for {} is {} for the {} class.\r\n",
                info.name, min_level, cls
            )
            .as_bytes(),
        );
        // Deliberate: an immortal may set a skill the character has not levelled
        // into. Say so, rather than leaving two lines that read like a refusal in
        // front of a change that happened anyway.
        send_to_char(g, chid, b"Setting it anyway.\r\n");
    }

    g.ch_mut(vict).set_skill(skill, value);
    let gname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
    g.mudlog(
        MudlogKind::Brf,
        LVL_IMMORT,
        true,
        &format!("{} changed {}'s {} to {}.", gname, vname, info.name, value),
    );
    send_to_char(
        g,
        chid,
        format!("You change {}'s {} to {}.\r\n", vname, info.name, value).as_bytes(),
    );
}

/// do_reboot — the `reload` command: re-read one text screen
/// (or all of them) from disk.
pub fn do_reboot(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let lib = g.lib_dir.clone();
    let text = lib.join("text");
    let help_dir = text.join("help");

    // (file, field, error message). GREETINGS is special: it is pruned and
    // reports only in the single-file branch.
    let load = |_g: &mut Game, path: std::path::PathBuf| -> Option<BStr> {
        let mut s = crate::text::file_to_string(&path)?;
        crate::text::parse_at(&mut s);
        Some(s)
    };
    macro_rules! reload_one {
        ($g:expr, $path:expr, $field:ident, $err:expr) => {{
            match load($g, $path) {
                Some(v) => $g.texts.$field = v,
                None => send_to_char($g, chid, $err),
            }
        }};
    }

    let all = arg.eq_ignore_ascii_case(b"all") || arg.first() == Some(&b'*');
    if all {
        if let Some(mut v) = load(g, text.join("greetings")) {
            crate::text::prune_crlf(&mut v);
            g.texts.greetings = v;
        }
        reload_one!(g, text.join("wizlist"), wizlist, b"Cannot read wizlist\r\n");
        reload_one!(g, text.join("immlist"), immlist, b"Cannot read immlist\r\n");
        reload_one!(g, text.join("news"), news, b"Cannot read news\r\n");
        reload_one!(g, text.join("credits"), credits, b"Cannot read credits\r\n");
        reload_one!(g, text.join("motd"), motd, b"Cannot read motd\r\n");
        reload_one!(g, text.join("imotd"), imotd, b"Cannot read imotd\r\n");
        reload_one!(g, help_dir.join("help"), help_screen, b"Cannot read help front page\r\n");
        reload_one!(g, help_dir.join("ihelp"), ihelp_screen, b"Cannot read help front page\r\n");
        reload_one!(g, text.join("info"), info, b"Cannot read info file\r\n");
        reload_one!(g, text.join("policies"), policies, b"Cannot read policies\r\n");
        reload_one!(g, text.join("handbook"), handbook, b"Cannot read handbook\r\n");
        reload_one!(g, text.join("background"), background, b"Cannot read background\r\n");
        reload_help(g);
    } else if arg.eq_ignore_ascii_case(b"wizlist") {
        reload_one!(g, text.join("wizlist"), wizlist, b"Cannot read wizlist\r\n");
    } else if arg.eq_ignore_ascii_case(b"immlist") {
        reload_one!(g, text.join("immlist"), immlist, b"Cannot read immlist\r\n");
    } else if arg.eq_ignore_ascii_case(b"news") {
        reload_one!(g, text.join("news"), news, b"Cannot read news\r\n");
    } else if arg.eq_ignore_ascii_case(b"credits") {
        reload_one!(g, text.join("credits"), credits, b"Cannot read credits\r\n");
    } else if arg.eq_ignore_ascii_case(b"motd") {
        reload_one!(g, text.join("motd"), motd, b"Cannot read motd\r\n");
    } else if arg.eq_ignore_ascii_case(b"imotd") {
        reload_one!(g, text.join("imotd"), imotd, b"Cannot read imotd\r\n");
    } else if arg.eq_ignore_ascii_case(b"help") {
        reload_one!(g, help_dir.join("help"), help_screen, b"Cannot read help front page\r\n");
    } else if arg.eq_ignore_ascii_case(b"ihelp") {
        reload_one!(g, help_dir.join("ihelp"), ihelp_screen, b"Cannot read help front page\r\n");
    } else if arg.eq_ignore_ascii_case(b"info") {
        reload_one!(g, text.join("info"), info, b"Cannot read info\r\n");
    } else if arg.eq_ignore_ascii_case(b"policy") {
        reload_one!(g, text.join("policies"), policies, b"Cannot read policy\r\n");
    } else if arg.eq_ignore_ascii_case(b"handbook") {
        reload_one!(g, text.join("handbook"), handbook, b"Cannot read handbook\r\n");
    } else if arg.eq_ignore_ascii_case(b"background") {
        reload_one!(g, text.join("background"), background, b"Cannot read background\r\n");
    } else if arg.eq_ignore_ascii_case(b"greetings") {
        match load(g, text.join("greetings")) {
            Some(mut v) => {
                crate::text::prune_crlf(&mut v);
                g.texts.greetings = v;
            }
            None => send_to_char(g, chid, b"Cannot read greetings.\r\n"),
        }
    } else if arg.eq_ignore_ascii_case(b"xhelp") {
        reload_help(g);
    } else {
        send_to_char(g, chid, b"Unknown reload option.\r\n");
        return;
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
}

fn reload_help(g: &mut Game) {
    let lib = g.lib_dir.clone();
    let mini = g.mini_mud;
    let mut lines = Vec::new();
    g.help_table = crate::text::boot_help(&lib, mini, &mut lines);
    // Every index into the old table is meaningless now. hedit refuses a
    // second editor but not a reload, so this is exactly the case its
    // generation check exists for.
    g.help_table_version += 1;
    for l in lines {
        g.log(l);
    }
}
