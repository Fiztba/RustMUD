//! The zone half of the generic OLC library: creating a zone
//! (seven files, seven index rewrites and an in-memory insertion), the
//! reset-command list editing primitives, and the room-command sweep redit
//! uses.
//!
//! `save_zone` itself lives in db.rs, where stage 8's `zlock`/`zreset` path
//! already needed it.

use mud_data::types::*;
use mud_world::model::{Zone, ZoneCommand};

use crate::db::{add_to_save_list, SL_ZON};
use crate::game::{Game, MudlogKind};

/// The seven world directories a new zone gets a file in, in creation
/// order.
const NEW_ZONE_FILES: [(&str, &str, &str); 7] = [
    // (subdir, extension, initial contents — filled in per zone below)
    ("zon", "zon", ""),
    ("wld", "wld", ""),
    ("mob", "mob", "$\n"),
    ("obj", "obj", "$\n"),
    ("shp", "shp", "$~\n"),
    ("qst", "qst", "$~\n"),
    ("trg", "trg", "$~\n"),
];

/// create_new_zone. Returns the new zone's rnum, or an error message for
/// the caller to relay.
pub fn create_new_zone(
    g: &mut Game,
    vzone_num: i32,
    bottom: i32,
    top: i32,
) -> Result<ZoneRnum, &'static str> {
    // CIRCLE_UNSIGNED_INDEX limits.
    if vzone_num == NOWHERE as i32 {
        return Err("You can't make negative zones.\r\n");
    } else if vzone_num > 655 {
        return Err("New zone cannot be higher than 655.\r\n");
    } else if bottom > top {
        return Err("Bottom room cannot be greater than top room.\r\n");
    } else if bottom <= 0 {
        return Err("Bottom room cannot be less then 0.\r\n");
    } else if top >= u16::MAX as i32 {
        return Err("Top greater than IDXTYPE_MAX. (Commonly 65535)\r\n");
    }

    // The loop covers every zone, including the one with the highest
    // zone as a count, never comparing the highest-numbered zone, and
    // `zedit new <that vnum>` would produce a duplicate.
    for z in g.world.zones.iter() {
        if z.number as i32 == vzone_num {
            return Err("That virtual zone already exists.\r\n");
        }
    }

    // Create the seven files.
    for (subdir, ext, body) in NEW_ZONE_FILES {
        let contents: String = match subdir {
            "zon" => format!("#{}\nNone~\nNew Zone~\n{} {} 30 2\nS\n$\n", vzone_num, bottom, top),
            "wld" => format!(
                "#{}\nThe Beginning~\nNot much here.\n~\n{} 0 0\nS\n$\n",
                bottom, vzone_num
            ),
            _ => body.to_string(),
        };
        let path = g.lib_dir.join("world").join(subdir).join(format!("{}.{}", vzone_num, ext));
        if std::fs::write(&path, contents.as_bytes()).is_err() {
            let what = match subdir {
                "zon" => ("zone file", "Could not write zone file.\r\n"),
                "wld" => ("world file", "Could not write world file.\r\n"),
                "mob" => ("mob file", "Could not write mobile file.\r\n"),
                "obj" => ("obj file", "Could not write object file.\r\n"),
                "shp" => ("shop file", "Could not write shop file.\r\n"),
                "qst" => ("quest file", "Could not write quest file.\r\n"),
                _ => ("trigger file", "Could not write trigger file.\r\n"),
            };
            let msg = format!("SYSERR: OLC: Can't write new {}.", what.0);
            g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
            return Err(what.1);
        }
    }

    // Update index files, in this order.
    for t in ["qst", "zon", "wld", "mob", "obj", "shp", "trg"] {
        create_world_index(g, vzone_num, t);
    }

    // Insert the zone in vnum order. Rooms belonging to every zone that
    // shifts up take a zone++: each moved zone's vnum window is walked and
    // whatever rooms it holds are bumped.
    let rznum: usize = if g
        .world
        .zones
        .last()
        .is_some_and(|z| vzone_num > z.number as i32)
    {
        g.world.zones.len()
    } else {
        let mut i = g.world.zones.len();
        while i > 0 && vzone_num < g.world.zones[i - 1].number as i32 {
            let (bot, top) = (g.world.zones[i - 1].bot, g.world.zones[i - 1].top);
            for v in bot..=top {
                if let Some(room) = g.world.real_room(v) {
                    g.world.rooms[room as usize].zone += 1;
                }
                if v == u16::MAX {
                    break;
                }
            }
            i -= 1;
        }
        i
    };

    let zone = Zone {
        name: Some(b"New Zone".to_vec()),
        number: vzone_num as Idx,
        builders: Some(b"None".to_vec()),
        bot: bottom as Idx,
        top: top as Idx,
        lifespan: 30,
        reset_mode: 2,
        min_level: -1,
        max_level: -1,
        zone_flags: [0; 4],
        cmds: Vec::new(),
    };
    g.world.zones.insert(rznum, zone);
    g.zones_rt.insert(rznum, crate::game::ZoneRt { age: 0 });

    // Queued resets hold zone rnums.
    for z in g.reset_q.iter_mut() {
        if *z as usize >= rznum {
            *z += 1;
        }
    }

    add_to_save_list(g, vzone_num as Idx, SL_ZON);
    Ok(rznum as ZoneRnum)
}

/// get_line: skip blank and '*' lines, strip the EOL.
fn index_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for raw in data.split(|&b| b == b'\n') {
        if raw.first() == Some(&b'*') || raw.is_empty() || raw.first() == Some(&b'\r') {
            continue;
        }
        let mut line = raw.to_vec();
        while matches!(line.last(), Some(b'\n') | Some(b'\r')) {
            line.pop();
        }
        out.push(line);
    }
    out
}

/// create_world_index: splice `<znum>.<type>` into the
/// directory's index, in numeric order. Comments and blank lines in the old
/// index are dropped, since the file is rewritten through get_line.
pub fn create_world_index(g: &mut Game, znum: i32, type_: &str) {
    let prefix = match type_.as_bytes().first() {
        Some(b'z') => "zon",
        Some(b'w') => "wld",
        Some(b'o') => "obj",
        Some(b'm') => "mob",
        Some(b's') => "shp",
        Some(b't') => "trg",
        Some(b'q') => "qst",
        // Caller messed up.
        _ => return,
    };
    let dir = g.lib_dir.join("world").join(prefix);
    let old_name = dir.join("index");
    let new_name = dir.join("newindex");

    let Ok(data) = std::fs::read(&old_name) else {
        let msg = format!("SYSERR: OLC: Failed to open {}/index.", prefix);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    };

    let entry = format!("{}.{}", znum, type_);
    let mut out: Vec<u8> = Vec::new();
    let mut found = false;
    for line in index_lines(&data) {
        if line.first() == Some(&b'$') {
            if !found {
                out.extend_from_slice(entry.as_bytes());
                out.extend_from_slice(b"\n$\n");
            } else {
                out.extend_from_slice(b"$\n");
            }
            break;
        } else if !found {
            let num = crate::handler::atoi(&line);
            if num > znum {
                found = true;
                out.extend_from_slice(entry.as_bytes());
                out.push(b'\n');
            } else if num == znum {
                // The index already had an entry for this zone.
                return;
            }
        }
        out.extend_from_slice(&line);
        out.push(b'\n');
    }

    if std::fs::write(&new_name, &out).is_err() {
        let msg = format!("SYSERR: OLC: Failed to open {}/newindex.", prefix);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return;
    }
    if let Err(e) = std::fs::rename(&new_name, &old_name) {
        let msg = format!("SYSERR: OLC: Failed to install {}: {}", old_name.display(), e);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
    }
}

/// Rewrite one index file without `<znum>.<type>`'s line. `complain` is false
/// for index.mini, which a world is not obliged to have at all. Returns
/// whether the file is now in the state the caller wanted.
fn strip_index_entry(g: &mut Game, dir: &std::path::Path, index_name: &str, znum: i32) -> bool {
    let old_name = dir.join(index_name);
    let new_name = dir.join(format!("new{}", index_name));

    let Ok(data) = std::fs::read(&old_name) else {
        // index.mini need not exist; the caller decides whether that matters.
        return false;
    };

    let mut out: Vec<u8> = Vec::new();
    let mut found = false;
    for line in index_lines(&data) {
        if line.first() == Some(&b'$') {
            out.extend_from_slice(b"$
");
            break;
        }
        if !found && crate::handler::atoi(&line) == znum {
            found = true; // the line being removed; do not copy it
            continue;
        }
        out.extend_from_slice(&line);
        out.push(b'\n');
    }

    // Only disturb the real file if there was something to take out of it.
    if !found {
        return true;
    }
    if std::fs::write(&new_name, &out).is_err() {
        let msg = format!("SYSERR: OLC: Failed to open {}.", new_name.display());
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return false;
    }
    if std::fs::rename(&new_name, &old_name).is_err() {
        let msg = format!("SYSERR: OLC: Failed to install {}.", old_name.display());
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return false;
    }
    true
}

/// Take a zone back out of the world index files: the inverse of
/// create_world_index. The files themselves are left for the caller.
///
/// Both indexes, index.mini first, and stop if that half fails. A zone still
/// named in index.mini after its file has gone stops a `-m` boot dead, because
/// index_boot exits on a listed file it cannot open; but a zone merely absent
/// from index.mini is only not loaded in mini mode, which is harmless. Doing
/// the harmless one first and returning on failure is what keeps the
/// boot-critical index untouched when the pair cannot be completed.
pub fn remove_world_index(g: &mut Game, znum: i32, type_: &str) -> bool {
    let prefix = match type_.as_bytes().first() {
        Some(b'z') => "zon",
        Some(b'w') => "wld",
        Some(b'o') => "obj",
        Some(b'm') => "mob",
        Some(b's') => "shp",
        Some(b't') => "trg",
        Some(b'q') => "qst",
        // Caller messed up.
        _ => return false,
    };
    let dir = g.lib_dir.join("world").join(prefix);

    // A missing index.mini is not a failure; a missing index is.
    if dir.join("index.mini").exists() && !strip_index_entry(g, &dir, "index.mini", znum) {
        return false;
    }
    if !dir.join("index").exists() {
        let msg = format!("SYSERR: OLC: Failed to open {}/index.", prefix);
        g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &msg);
        return false;
    }
    strip_index_entry(g, &dir, "index", znum)
}

/// remove_room_zone_commands: drop every command in the
/// zone that targets this room. `cmd_room` is deliberately *not* reset
/// between iterations, so a command type it does not recognise
/// (anything but M/O/T/V/D/R) is judged against the previous command's
/// room — and removed with it. Return the first removed position (or the end).
pub fn remove_room_zone_commands(g: &mut Game, zone: usize, room_num: RoomRnum) -> usize {
    let mut first_removed = None;
    let mut subcmd = 0usize;
    let mut cmd_room: i32 = -2;
    while subcmd < g.world.zones[zone].cmds.len() {
        let cmd = &g.world.zones[zone].cmds[subcmd];
        match cmd.command {
            b'M' | b'O' | b'T' | b'V' => cmd_room = cmd.arg3,
            b'D' | b'R' => cmd_room = cmd.arg1,
            _ => {}
        }
        if cmd_room == room_num as i32 {
            first_removed.get_or_insert(subcmd);
            g.world.zones[zone].cmds.remove(subcmd);
        } else {
            subcmd += 1;
        }
    }
    first_removed.unwrap_or(g.world.zones[zone].cmds.len())
}

/// count_commands. Our list has no 'S' terminator, so
/// this is simply its length.
pub fn count_commands(zone: &Zone) -> usize {
    zone.cmds.len()
}

/// Shared body of the two prototype-reset walks: shift every surviving
/// reference down past `rnum` and work out which commands die -- those
/// naming the prototype, plus the `if_flag` chain and implicit-target
/// commands hanging off them. Returns whether anything changed and one
/// flag per command.
fn kill_prototype_resets(zone: &mut Zone, rnum: Idx, mobile: bool) -> (bool, Vec<bool>) {
    let mut changed = false;
    let (mut mob_removed, mut tmob_removed, mut tobj_removed) = (false, false, false);
    let mut previous_removed = false;
    let mut dead = Vec::with_capacity(zone.cmds.len());
    for cmd in &mut zone.cmds {
        let mut remove = cmd.if_flag != 0 && previous_removed;
        let refs: Vec<&mut i32> = match cmd.command {
            b'M' if mobile => vec![&mut cmd.arg1],
            b'P' if !mobile => vec![&mut cmd.arg1, &mut cmd.arg3],
            b'O' | b'G' | b'E' if !mobile => vec![&mut cmd.arg1],
            b'R' if !mobile => vec![&mut cmd.arg2],
            _ => Vec::new(),
        };
        remove |= refs.iter().any(|value| **value == rnum as i32);
        for value in refs {
            if *value > rnum as i32 && *value != NOTHING as i32 {
                *value -= 1;
                changed = true;
            }
        }
        match cmd.command {
            b'M' => {
                mob_removed = remove;
                tmob_removed = remove;
                tobj_removed = remove;
            }
            b'O' | b'P' | b'G' | b'E' => {
                remove |= matches!(cmd.command, b'G' | b'E') && mob_removed;
                tobj_removed = remove;
                tmob_removed = remove;
            }
            b'T' | b'V' => {
                remove |= (cmd.arg1 == crate::dg::MOB_TRIGGER && tmob_removed)
                    || (cmd.arg1 == crate::dg::OBJ_TRIGGER && tobj_removed);
            }
            b'D' | b'R' => {
                tmob_removed = remove;
                tobj_removed = remove;
            }
            _ => {}
        }
        previous_removed = remove;
        changed |= remove;
        dead.push(remove);
    }
    (changed, dead)
}

/// Remove a deleted prototype's resets and the commands using their implicit
/// mob/object targets, then shift surviving references to the shortened table.
/// `mobile` selects the mobile table; false selects the object table.
pub fn remove_prototype_resets(zone: &mut Zone, rnum: Idx, mobile: bool) -> bool {
    let (changed, dead) = kill_prototype_resets(zone, rnum, mobile);
    let mut i = 0;
    zone.cmds.retain(|_| {
        i += 1;
        !dead[i - 1]
    });
    changed
}

/// The same walk over a zedit scratch zone, which is disabled in place rather
/// than removed: zedit holds a cursor into this array across prompts
/// (`olc.value` indexes `olc.zone.cmds`), and removing an entry from under it
/// leaves it naming a different command -- or, at the end of the list, none.
/// '*' is the mark renum_zone_table already puts on a reset command that cannot
/// resolve; reset_zone treats it as a no-op and the .zon writer drops it.
/// Returns the indices of the commands that were disabled.
pub fn disable_prototype_resets(zone: &mut Zone, rnum: Idx, mobile: bool) -> Vec<usize> {
    let (_, dead) = kill_prototype_resets(zone, rnum, mobile);
    let mut killed = Vec::new();
    for (i, cmd) in zone.cmds.iter_mut().enumerate() {
        if dead[i] {
            cmd.command = b'*';
            killed.push(i);
        }
    }
    killed
}

/// Fix up every open zone editor after a mobile or object prototype has left
/// the tables at `rnum`. zedit_setup takes whole reset commands out of the
/// zone table, so each open zedit holds its own copy of the rnums
/// delete_mobile / delete_object have just renumbered; saving a stale copy
/// puts an out-of-range index back into the live table. A builder part-way
/// through filling in one of the killed commands is put back at the zone
/// menu: every argument prompt switches on the command byte and none of them
/// has a case for '*'.
pub fn disable_prototype_resets_in_open_editors(g: &mut Game, vnum: Idx, rnum: Idx, mobile: bool) {
    use crate::comm::write_to_desc;
    use crate::olc::zedit::{
        ZEDIT_ARG1, ZEDIT_ARG2, ZEDIT_ARG3, ZEDIT_COMMAND_TYPE, ZEDIT_IF_FLAG, ZEDIT_MAIN_MENU,
        ZEDIT_SARG1, ZEDIT_SARG2,
    };

    let sessions: Vec<usize> = g.olc.keys().copied().collect();
    for dsc in sessions {
        let (dead, on_it) = {
            let Some(olc) = g.olc.get_mut(&dsc) else { continue };
            // olc.value only names a command while one is being filled in; in
            // the menu it is a leftover.
            let editing = matches!(
                olc.mode,
                ZEDIT_COMMAND_TYPE
                    | ZEDIT_IF_FLAG
                    | ZEDIT_ARG1
                    | ZEDIT_ARG2
                    | ZEDIT_ARG3
                    | ZEDIT_SARG1
                    | ZEDIT_SARG2
            );
            let cursor = olc.value;
            let Some(zone) = olc.zone.as_mut() else { continue };
            let killed = disable_prototype_resets(zone, rnum, mobile);
            let on_it = editing && killed.iter().any(|&i| i as i32 == cursor);
            (killed.len(), on_it)
        };
        if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Zedit) {
            continue;
        }
        if dead > 0 {
            let msg = format!(
                "\r\n{} {} has been deleted; {} reset command{} in the zone you are \
                 editing no longer do{} anything.\r\n",
                if mobile { "Mobile" } else { "Object" },
                vnum,
                dead,
                if dead == 1 { "" } else { "s" },
                if dead == 1 { "es" } else { "" }
            );
            write_to_desc(g, dsc, msg.as_bytes());
        }
        if on_it {
            if let Some(olc) = g.olc.get_mut(&dsc) {
                olc.mode = ZEDIT_MAIN_MENU;
            }
            write_to_desc(
                g,
                dsc,
                b"That was the command you were editing, so you are back at the zone menu \
                  -- press return for it.\r\n",
            );
        }
    }
}

/// Disable resets tied to a deleted room, including commands using their implicit
/// targets. Keep positions stable for open zone editors and shift surviving rooms.
pub fn remove_room_resets(zone: &mut Zone, rnum: Idx) -> bool {
    let mut changed = false;
    let (mut mob_removed, mut tmob_removed, mut tobj_removed) = (false, false, false);
    let mut previous_removed = false;
    for cmd in &mut zone.cmds {
        let mut remove = cmd.if_flag != 0 && previous_removed;
        let room = match cmd.command {
            b'M' | b'O' | b'T' | b'V' => Some(&mut cmd.arg3),
            b'D' | b'R' => Some(&mut cmd.arg1),
            _ => None,
        };
        if let Some(room) = room {
            remove |= *room == rnum as i32;
            if *room > rnum as i32 && *room != NOWHERE as i32 {
                *room -= 1;
                changed = true;
            }
        }
        match cmd.command {
            b'M' => {
                mob_removed = remove;
                tmob_removed = remove;
                tobj_removed = remove;
            }
            b'O' | b'P' | b'G' | b'E' => {
                remove |= matches!(cmd.command, b'G' | b'E') && mob_removed;
                tobj_removed = remove;
                tmob_removed = remove;
            }
            b'T' | b'V' => {
                remove |= (cmd.arg1 == crate::dg::MOB_TRIGGER && tmob_removed)
                    || (cmd.arg1 == crate::dg::OBJ_TRIGGER && tobj_removed);
            }
            b'D' | b'R' => {
                tmob_removed = remove;
                tobj_removed = remove;
            }
            _ => {}
        }
        previous_removed = remove;
        if remove {
            cmd.command = b'*';
            changed = true;
        }
    }
    changed
}

/// add_cmd_to_list.
///
/// Copying into a fresh `count + 2` array with
/// `newlist[i] = (i == pos) ? *newcmd : (*list)[l++]` for `i` in `0..=count`
/// means a `pos` past the end never matches: the copy pulls the old list's
/// 'S' terminator into place and **the new command is silently dropped**.
/// zedit_save_internally is the one caller that can reach that.
pub fn add_cmd_to_list(zone: &mut Zone, newcmd: ZoneCommand, pos: usize) {
    if pos > zone.cmds.len() {
        return;
    }
    zone.cmds.insert(pos, newcmd);
}

pub fn remove_cmd_from_list(zone: &mut Zone, pos: usize) {
    if pos < zone.cmds.len() {
        zone.cmds.remove(pos);
    }
}

/// new_command: a blank 'N' command at `pos`.
pub fn new_command(zone: &mut Zone, pos: i32) -> bool {
    let count = zone.cmds.len() as i32;
    if pos < 0 || pos > count {
        return false;
    }
    add_cmd_to_list(zone, ZoneCommand { command: b'N', ..Default::default() }, pos as usize);
    true
}

pub fn delete_zone_command(zone: &mut Zone, pos: i32) {
    let count = zone.cmds.len() as i32;
    if pos < 0 || pos >= count {
        return;
    }
    remove_cmd_from_list(zone, pos as usize);
}
