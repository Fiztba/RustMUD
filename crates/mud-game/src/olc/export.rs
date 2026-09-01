//! `do_export_zone` and the seven `export_save_*` helpers behind it — a
//! zone *portability* command, not a backup: it writes a copy of the zone
//! with every vnum rewritten so someone on another MUD can drop it in at a
//! vnum of their choosing.
//!
//! Five things here are worth knowing before changing them:
//!
//! * **No `system`.** Shelling out to `rm`, `tar` and `gzip` interpolates
//! the zone name unquoted, and `fix_filename` passes `;`, `&`, `` ` ``
//! and `$` straight through — so a builder who can rename their own zone
//! could run commands as the MUD user the next time an implementor
//! exports it. The archive bytes are emitted directly
//! (`super::archive`), which removes the shell entirely and makes the
//! command work on Windows.
//! * **Quests are exported.** The help text lists `.qst` among a zone's
//! files, so all eight formats are written, not seven.
//! * **The info file tells the truth about exits.** A `zone_exits` counter
//! incremented by `export_save_rooms` would be read by
//! `export_info_file`, which runs first — so every info file would claim
//! the zone has no exits out of it while the `.wld` is full of `ZZ`
//! markers. The outbound exits are gathered up front and reported from
//! that list.
//! * **Success accumulates.** `if (!(success = export_save_X(...)))`
//! overwrites the previous result, so only the last failure could stop
//! the archive. Every writer's result is kept instead.
//! * **The archive is named what the help promises.** The help says
//! `<zone #>_<zone name>.tgz`; `<name>.tar.gz` with no number would let
//! two zones with the same name overwrite each other.
//!
//! The vnum rewriting itself lives in `mud_world::write::VnumFmt`, so an
//! export runs through the same writers a real save does.

use mud_data::tables::DIRS;
use mud_data::types::{Idx, LVL_IMPL, NOTHING, NOWHERE};
use mud_world::write::VnumFmt;

use crate::comm::send_to_char;
use crate::game::{Game, MudlogKind};
use crate::interpreter::{is_number, one_argument, skip_spaces};
use mud_data::ids::CharId;

use super::archive::{self, Member};

/// What the recipient has to reattach by hand, gathered before anything is
/// written so the info file can be honest about it.
#[derive(Default)]
struct Findings {
    /// (room vnum, direction) for every exit leaving the zone.
    exits: Vec<(Idx, usize)>,
    /// (room vnum, direction, key vnum) for doors whose key object lives
    /// somewhere else. Marked ZZ like an exit, and just as fatal to the
    /// recipient's boot, so they have to be listed too.
    keys: Vec<(Idx, usize, i32)>,
    /// (shop vnum, object vnum) products dropped for being out of zone.
    products: Vec<(Idx, i32)>,
    /// (shop vnum, room vnum) shop rooms dropped for the same reason.
    rooms: Vec<(Idx, i32)>,
    /// (shop vnum, keeper vnum) keepers that live in another zone.
    keepers: Vec<(Idx, i32)>,
}

/// `fix_filename`, tightened. Mapping space to `_`, parens to braces and
/// dropping quotes leaves everything else — `/`, `\`, `..`, control
/// characters — to pass through. The result becomes a real path here, so
/// anything not plainly safe is dropped.
fn fix_filename(name: &[u8]) -> String {
    let mut out = String::new();
    for &b in name {
        match b {
            b' ' => out.push('_'),
            b'(' => out.push('{'),
            b')' => out.push('}'),
            b'\'' | b'"' => {}
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => out.push(b as char),
            _ => {}
        }
    }
    if out.is_empty() || out.chars().all(|c| c == '.') {
        out = "zone".to_string();
    }
    out
}

/// Everything the info file needs that the files themselves cannot say.
fn survey(g: &Game, zone_rnum: usize, fmt: VnumFmt) -> Findings {
    let mut f = Findings::default();
    let z = &g.world.zones[zone_rnum];
    let dirs = crate::fight::dir_count(g);

    for vnum in z.bot..=z.top {
        let Some(rnum) = g.world.real_room(vnum) else { continue };
        let room = &g.world.rooms[rnum as usize];
        for (dir, slot) in room.dir_option.iter().enumerate().take(dirs) {
            let Some(ex) = slot else { continue };
            if ex.key != NOTHING && !fmt.in_zone(i64::from(ex.key)) {
                f.keys.push((vnum, dir, i32::from(ex.key)));
            }
            if ex.to_room == NOWHERE {
                continue;
            }
            let target = g.world.rooms[ex.to_room as usize].vnum;
            if !fmt.in_zone(i64::from(target)) {
                f.exits.push((vnum, dir));
            }
        }
    }

    for shop in &g.world.shops {
        if !fmt.in_zone(i64::from(shop.vnum)) {
            continue;
        }
        for &p in &shop.producing {
            if !fmt.in_zone(i64::from(p)) {
                f.products.push((shop.vnum, p));
            }
        }
        for &r in shop.in_rooms.iter().take_while(|&&r| r != 65535) {
            if !fmt.in_zone(i64::from(r)) {
                f.rooms.push((shop.vnum, r));
            }
        }
        let keeper = shop.keeper_vnum;
        if g.world.mob_map.contains_key(&(keeper as u16)) && !fmt.in_zone(i64::from(keeper)) {
            f.keepers.push((shop.vnum, keeper));
        }
    }
    f
}

/// The generated README. Its promises are now checked against what the
/// writers actually produced.
fn info_file(g: &Game, zone_rnum: usize, fmt: VnumFmt, f: &Findings) -> Vec<u8> {
    let z = &g.world.zones[zone_rnum];
    let name = z.name.as_deref().unwrap_or(b"undefined");
    let builders = z.builders.as_deref().unwrap_or(b"None.");
    let mut o = String::new();

    o.push_str("tbaMUD Area file.\n");
    o.push_str(&format!(
        "The files accompanying this info file contain the area: {}\n",
        String::from_utf8_lossy(name)
    ));
    o.push_str(&format!("It was written by: {}.\n\n", String::from_utf8_lossy(builders)));
    o.push_str(
        "The author has given permission to distribute the area, provided credit is\n\
         given. The area may be modified as you see fit, except you are not allowed to\n\
         remove the builder name or credits.\n\n\
         Implementation:\n",
    );

    match fmt.new_number() {
        None => o.push_str(
            "1. All the files have been QQ'ed. This means all occurences of the zone number\n\
             \x20  have been changed to QQ. In other words, if you decide to have this zone as\n\
             \x20  zone 123, replace all occurences of QQ with 123 and rename the qq.zon file\n\
             \x20  to 123.zon (etc.). And of course add 123.zon to the respective index file.\n",
        ),
        Some(target) => o.push_str(&format!(
            "1. The files have been renumbered into zone {target}: every vnum this zone owns\n\
             \x20  has been rewritten into the {target}00-{target}99 range and the files are\n\
             \x20  named for it, so they can be dropped straight in. Add {target}.zon and its\n\
             \x20  siblings to the respective index files.\n"
        )),
    }

    if f.exits.is_empty() && f.keys.is_empty() {
        o.push_str("2. This area doesn't have any exits _out_ of the zone.\n");
    } else if f.exits.is_empty() {
        o.push_str(
            "2. This area has no exits _out_ of the zone, but some of its doors are\n\
             \x20  locked with keys that live elsewhere. Those key vnums are ZZ'd and the\n\
             \x20  server will refuse to boot until you point them at real objects:\n",
        );
    } else {
        o.push_str(
            "2. Exits out of this zone have been ZZ'd. So all doors leading out have ZZ??\n\
             \x20  instead of the room vnum (?? are numbers 00 - 99). The server will refuse\n\
             \x20  to boot until you point them somewhere real, which is deliberate: a vnum\n\
             \x20  that happens not to exist here would load as a dead exit that `look` still\n\
             \x20  describes.\n\
             \x20  In this zone, the exit rooms in question are:\n",
        );
        for &(room, dir) in &f.exits {
            o.push_str(&format!("      Room {} : Exit to the {}\n", marker(fmt, room), DIRS[dir]));
        }
    }

    if !f.keys.is_empty() {
        if !f.exits.is_empty() {
            o.push_str(
                "\x20  Some doors are also locked with keys from other zones. Those key\n\
                 \x20  vnums are ZZ'd for the same reason, and need pointing at real\n\
                 \x20  objects before the zone will boot:\n",
            );
        }
        for &(room, dir, key) in &f.keys {
            o.push_str(&format!(
                "      Room {} : {} door, key was object {key}\n",
                marker(fmt, room),
                DIRS[dir]
            ));
        }
    }

    if !f.products.is_empty() || !f.rooms.is_empty() || !f.keepers.is_empty() {
        o.push_str(
            "\n3. Shops can legitimately stock other zones' goods, so those entries are\n\
             \x20  dropped rather than marked — the .shp is valid as it stands. What was\n\
             \x20  left out:\n",
        );
        for &(shop, obj) in &f.products {
            o.push_str(&format!("      Shop {} : product, object {obj}\n", marker(fmt, shop)));
        }
        for &(shop, room) in &f.rooms {
            o.push_str(&format!("      Shop {} : shop room {room}\n", marker(fmt, shop)));
        }
        for &(shop, mob) in &f.keepers {
            o.push_str(&format!(
                "      Shop {} : keeper is mob {mob}, from another zone, and has been\n\
                 \x20                     renumbered into this one\n",
                marker(fmt, shop)
            ));
        }
    }

    o.push_str(&format!(
        "\nAdditional zone information is available in the zone description room {}.\n",
        marker(fmt, z.bot)
    ));
    o.push_str(
        "The Builder's Academy is maintaining and improving these zones. Any typo or\n\
         bug reports should be reported to rumble@tbamud.com or stop by The Builder Academy\n\
         port telnet://tbamud.com:9091\n\
         \nAnyone interested in submitting areas or helping improve the existing ones\n\
         please stop by TBA and talk to Rumble.\n\n\
         We at The Builder's Academy hope you will enjoy using the area.\n\n\
         Rumble - Admin of TBA\n\
         Welcor - Coder of TBA\n\
         \ntelnet://tbamud.com:9091/\n",
    );
    o.into_bytes()
}

/// Render one vnum the way the exported files spell it, for the info file.
fn marker(fmt: VnumFmt, vnum: Idx) -> String {
    let mut buf = Vec::new();
    fmt.push_vnum(&mut buf, i64::from(vnum));
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn do_export_zone(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).level < LVL_IMPL {
        return;
    }

    let argument = skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Syntax: export <zone vnum> [<target zone>] [zip]\r\n");
        return;
    }

    let (arg1, rest) = one_argument(argument);
    let (arg2, rest) = one_argument(rest);
    let (arg3, _) = one_argument(rest);

    let zvnum = mud_world::lex::atol(&arg1);
    let zone_rnum = match u16::try_from(zvnum).ok().and_then(|v| g.world.real_zone(v)) {
        Some(zr) => zr as usize,
        None => {
            send_to_char(g, chid, b"Export which zone?\r\n");
            return;
        }
    };

    // Second argument is the target zone when numeric, the format keyword
    // otherwise; the third is only ever the keyword.
    let (target, keyword) = if is_number(&arg2) {
        (Some(mud_world::lex::atol(&arg2) as i64), arg3)
    } else {
        (None, arg2)
    };

    let as_zip = match keyword.as_slice() {
        b"" => false,
        k if k.eq_ignore_ascii_case(b"zip") => true,
        _ => {
            send_to_char(g, chid, b"Syntax: export <zone vnum> [<target zone>] [zip]\r\n");
            return;
        }
    };

    if target.is_some_and(|t| t < 0) {
        send_to_char(g, chid, b"A target zone cannot be negative.\r\n");
        return;
    }

    // The ceiling is the highest vnum that fits in an `Idx`, and not `Idx::MAX`
    // itself, which is NOWHERE and NOTHING. Nothing on the load path
    // range-checks a vnum — the room parser truncates into a `u16` and the zone
    // header is read the same way — so a target that does not fit yields files
    // that are quietly wrong rather than files that are refused.
    //
    // Derived from `Idx::MAX` rather than written as a literal, so the two
    // cannot drift apart. The literal this replaces, 655, was already one too
    // many: zone 655 reaches vnum 65599 and only its first 35 slots fit.
    if let Some(t) = target {
        let bot = if zone_rnum == 0 { 0 } else { g.world.zones[zone_rnum].bot };
        let highest = t * 100 + (g.world.zones[zone_rnum].top as i64 - bot as i64);
        if highest >= Idx::MAX as i64 {
            let msg = format!(
                "Zone {t} would put this zone's highest vnum at {highest}, and no vnum \
                 above {} can be stored.\r\n",
                Idx::MAX as i64 - 1
            );
            send_to_char(g, chid, msg.as_bytes());
            return;
        }
    }

    let zone = g.world.zones[zone_rnum].clone();
    let fmt = match target {
        Some(t) => VnumFmt::renumber(&zone, t as Idx),
        None => VnumFmt::qq(&zone),
    };

    if fmt.spans_over_100() {
        let msg = format!(
            "Note: zone {} spans {}-{}, wider than the 100-vnum grid this scheme assumes.\r\n{}\r\n",
            zone.number,
            zone.bot,
            zone.top,
            match target {
                Some(t) => format!("The copy will spill past zone {t} into the one above it."),
                None =>
                    "Two vnums 100 apart collapse onto the same QQ marker; check the files.".into(),
            }
        );
        send_to_char(g, chid, msg.as_bytes());
    }

    let findings = survey(g, zone_rnum, fmt);

    // Render everything before touching the disk, so a failure cannot
    // leave a half-written set behind.
    let stem = match target {
        Some(t) => t.to_string(),
        None => "qq".to_string(),
    };
    let zr = zone_rnum as u16;
    let members = vec![
        Member { name: format!("{stem}.info"), data: info_file(g, zone_rnum, fmt, &findings) },
        Member {
            name: format!("{stem}.wld"),
            data: mud_world::write::wld::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.zon"),
            data: mud_world::write::zon::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.mob"),
            data: mud_world::write::mob::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.obj"),
            data: mud_world::write::obj::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.shp"),
            data: mud_world::write::shp::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.qst"),
            data: mud_world::write::qst::write_file_fmt(&g.world, zr, fmt),
        },
        Member {
            name: format!("{stem}.trg"),
            data: mud_world::write::trg::write_file_fmt(&g.world, zr, fmt),
        },
    ];

    let dir = g.lib_dir.join("world").join("export");
    if std::fs::create_dir_all(&dir).is_err() {
        send_to_char(g, chid, b"Failed to create export directory.\r\n");
        return;
    }

    // Every writer's result is kept, and the failures are named. Whatever
    // landed is archived anyway.
    let mut failed: Vec<String> = Vec::new();
    for m in &members {
        if std::fs::write(dir.join(&m.name), &m.data).is_err() {
            failed.push(m.name.clone());
        }
    }
    if !failed.is_empty() {
        let msg = format!("Ran into problems writing to files: {}\r\n", failed.join(", "));
        send_to_char(g, chid, msg.as_bytes());
        return;
    }
    send_to_char(g, chid, b"Individual files saved to /lib/world/export.\r\n");

    // "<number>_<name>.tar.gz" — what the help has always promised and the
    // code never did. The number is the target when there is one, so two
    // exports of the same zone at different targets do not collide.
    let number = target.unwrap_or(i64::from(zone.number));
    let fixed = fix_filename(zone.name.as_deref().unwrap_or(b"undefined"));
    // Stamp the archive with the time it was made; an archive dated 1970
    // looks like a broken tool.
    let now = g.now.max(0) as u64;
    let (archive_name, bytes) = if as_zip {
        let local = chrono::DateTime::from_timestamp(g.now + g.tz_offset_secs, 0)
            .unwrap_or_default()
            .naive_utc();
        let (date, time) = archive::dos_stamp(
            chrono::Datelike::year(&local),
            chrono::Datelike::month(&local),
            chrono::Datelike::day(&local),
            chrono::Timelike::hour(&local),
            chrono::Timelike::minute(&local),
            chrono::Timelike::second(&local),
        );
        (format!("{number}_{fixed}.zip"), archive::zip(&members, date, time))
    } else {
        (
            format!("{number}_{fixed}.tar.gz"),
            archive::gzip(&archive::tar(&members, now), now as u32),
        )
    };

    if std::fs::write(dir.join(&archive_name), &bytes).is_err() {
        let msg = format!("Failed to write {archive_name}.\r\n");
        send_to_char(g, chid, msg.as_bytes());
        return;
    }

    let msg = format!(
        "Files archived to \"world/export/{archive_name}\" ({} bytes).\r\n",
        bytes.len()
    );
    send_to_char(g, chid, msg.as_bytes());

    let marked = findings.exits.len() + findings.keys.len();
    if marked > 0 {
        let msg = format!(
            "{marked} reference{} out of this zone {} marked ZZ ({} exit{}, {} door key{}); \
             the info file lists them.\r\n",
            if marked == 1 { "" } else { "s" },
            if marked == 1 { "is" } else { "are" },
            findings.exits.len(),
            if findings.exits.len() == 1 { "" } else { "s" },
            findings.keys.len(),
            if findings.keys.len() == 1 { "" } else { "s" },
        );
        send_to_char(g, chid, msg.as_bytes());
    }

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Nrm,
        (LVL_IMPL as i16).max(invis) as u8,
        true,
        &format!("(GC) {} exported zone {} to {}", name, zone.number, archive_name),
    );
}
