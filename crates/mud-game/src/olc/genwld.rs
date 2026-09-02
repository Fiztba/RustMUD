//! The room half of the generic OLC library: inserting a room
//! into the rnum-ordered world table (and shifting every rnum reference
//! that follows), deleting one, and writing a zone's rooms back out.
//!
//! Three things `add_room` has to get right:
//!
//! * **B25** — it fixed zone commands only from the new room's own zone
//! upward while `delete_room` scans from 0, so a command stored lower
//! kept a stale rnum. Observed live: creating room 7226
//! left zone 71's door command operating on room 7234 instead of 7279.
//! * **B27** — `copy_room` is a whole-struct assignment, so saving an
//! existing room restored the **light count** captured when editing
//! began, and the count never recovered.
//! * **B27** — inserting a room shifted every `in_room` but never
//! `was_in_room`, so a player idled into the void returned one room off.

use mud_data::types::*;
use mud_world::model::Room;

use crate::db::{
    add_to_save_list, in_save_list, remove_from_save_list, write_world_file, SL_WLD, SL_ZON,
};
use crate::game::{EventKind, EventOwner, Game, MudlogKind, RoomRt};

/// Room rnums live inside event payloads and in the lazily created event
/// lists; both move with the room table. `delta` is +1 (insert) or -1
/// (delete), applied to every room rnum strictly above `from`.
fn shift_room_events(g: &mut Game, from: RoomRnum, delta: i32) {
    let bump = |r: RoomRnum| -> RoomRnum {
        if r != NOWHERE && (delta > 0 && r >= from || delta < 0 && r > from) {
            (r as i32 + delta) as RoomRnum
        } else {
            r
        }
    };
    for e in g.events.iter_mut() {
        match &mut e.kind {
            EventKind::SplDarkness { room } => *room = bump(*room),
            EventKind::TrigWait { go: crate::dg::GoId::Room(r), .. } => *r = bump(*r),
            _ => {}
        }
    }
    let owners: Vec<EventOwner> = g.event_lists.iter().copied().collect();
    for o in owners {
        if let EventOwner::Room(r) = o {
            let n = bump(r);
            if n != r {
                g.event_lists.remove(&o);
                g.event_lists.insert(EventOwner::Room(n));
            }
        }
    }
}

/// add_room. Returns the rnum the room landed at, or
/// `None` for NOWHERE.
///
/// `light` is the room's live light count travelling with the OLC copy (
/// copies the whole struct, light included).
pub fn add_room(g: &mut Game, room: &Room, light: i32) -> Option<RoomRnum> {
    // Updating a room that already exists: keep the people and the
    // contents, replace everything else.
    if let Some(i) = g.world.real_room(room.vnum) {
        let ri = i as usize;
        if g.rooms[ri].script.is_some() {
            crate::dg::extract_script(g, crate::dg::GoId::Room(i));
        }
        g.world.rooms[ri] = room.clone();
        // `light` counts the light sources standing in the room right
        // now, so it belongs to the live room, not to the copy being saved.
        // A whole-struct copy back would restore the editor's stale value.
        g.rooms[ri].script = None;
        let zvnum = g.world.zones[room.zone as usize].number;
        add_to_save_list(g, zvnum, SL_WLD);
        g.log(format!("GenOLC: add_room: Updated existing room #{}.", room.vnum));
        return Some(i);
    }

    // Find the insertion point: walk down from the top and take the first
    // slot whose predecessor sorts lower.
    let old_len = g.world.rooms.len();
    let mut found: usize = 0;
    for i in (1..=old_len).rev() {
        if room.vnum > g.world.rooms[i - 1].vnum {
            found = i;
            break;
        }
    }

    g.world.rooms.insert(found, room.clone());
    g.rooms.insert(found, RoomRt { light, ..Default::default() });
    // Spec procs live in a parallel array rather than in the room struct,
    // so the shift has to be done by hand or every spec proc from the
    // insertion point up is
    // attributed to the room below it — and `special` indexes this on every
    // command, so once the array is shorter than the room table, standing in
    // the highest room and typing anything panics.
    g.room_specs.insert(found, None);

    // Room map: everything at or past the insertion point moved up one.
    for v in g.world.room_map.values_mut() {
        if *v as usize >= found {
            *v += 1;
        }
    }
    g.world.room_map.insert(room.vnum, found as RoomRnum);

    // People and objects in the rooms that moved. Their
    // in_room values are still pre-insert indices here, so the test is
    // against `found` itself. `was_in_room` is deliberately left alone —
    // nothing else touches it.
    let ids: Vec<_> = g.character_list.iter().copied().collect();
    for id in ids {
        if let Some(c) = g.chars.get_mut(id) {
            if c.in_room != NOWHERE && c.in_room as usize >= found {
                c.in_room += 1;
            }
        }
    }
    let oids: Vec<_> = g.object_list.iter().copied().collect();
    for id in oids {
        if let Some(o) = g.objs.get_mut(id) {
            if o.in_room != NOWHERE && o.in_room as usize >= found {
                o.in_room += 1;
            }
        }
    }
    // update_wait_events: room-owned events follow the room.
    shift_room_events(g, found as RoomRnum, 1);

    g.log(format!("GenOLC: add_room: Added room {} at index #{}.", room.vnum, found));

    // Zone commands, every zone (B25: starting at this room's own zone
    // would miss commands stored in lower ones, and the shipped world has
    // 26 of those).
    let nowhere = NOWHERE as i32;
    let mut unknown = 0;
    for zi in 0..g.world.zones.len() {
        let n = g.world.zones[zi].cmds.len();
        for ci in 0..n {
            let cmd = &mut g.world.zones[zi].cmds[ci];
            match cmd.command {
                b'M' | b'O' | b'T' | b'V' => {
                    if cmd.arg3 != nowhere && cmd.arg3 >= found as i32 {
                        cmd.arg3 += 1;
                    }
                }
                b'D' | b'R' => {
                    if cmd.arg1 != nowhere && cmd.arg1 >= found as i32 {
                        cmd.arg1 += 1;
                    }
                }
                b'G' | b'P' | b'E' | b'*' | b'S' => {}
                _ => unknown += 1,
            }
        }
    }
    for _ in 0..unknown {
        g.mudlog(
            MudlogKind::Brf,
            LVL_GOD,
            true,
            "SYSERR: GenOLC: add_room: Unknown zone entry found!",
        );
    }

    // The load-room table.
    for r in [
        &mut g.r_mortal_start_room,
        &mut g.r_immort_start_room,
        &mut g.r_frozen_start_room,
    ] {
        if *r as usize >= found {
            *r += 1;
        }
    }

    // Characters idled into the void hold their room in was_in_room
    // and are in no room's people list, so the in_room pass above cannot
    // reach them. Without this they come back one room off.
    for id in g.character_list.clone() {
        if let Some(c) = g.chars.get_mut(id) {
            if c.was_in_room != NOWHERE && c.was_in_room as usize >= found {
                c.was_in_room += 1;
            }
        }
    }

    // World exits, the new room's own included.
    let dirs = crate::fight::dir_count(g);
    for r in g.world.rooms.iter_mut() {
        for ex in r.dir_option.iter_mut().take(dirs) {
            if let Some(ex) = ex {
                if ex.to_room != NOWHERE && ex.to_room as usize >= found {
                    ex.to_room += 1;
                }
            }
        }
    }

    let zvnum = g.world.zones[room.zone as usize].number;
    add_to_save_list(g, zvnum, SL_WLD);

    Some(found as RoomRnum)
}

pub fn delete_room(g: &mut Game, rnum: RoomRnum) -> bool {
    // "Can't delete void yet."
    if rnum == 0 || rnum == NOWHERE || rnum as usize >= g.world.rooms.len() {
        return false;
    }
    let ri = rnum as usize;

    let zvnum = g.world.zones[g.world.rooms[ri].zone as usize].number;
    add_to_save_list(g, zvnum, SL_WLD);

    let vnum = g.world.rooms[ri].vnum;
    let name = g
        .world
        .rooms[ri]
        .name
        .clone()
        .map(|n| String::from_utf8_lossy(&n).into_owned())
        .unwrap_or_else(|| "(null)".to_string());
    g.log(format!("GenOLC: delete_room: Deleting room #{} ({}).", vnum, name));

    if g.r_mortal_start_room == rnum {
        g.log("WARNING: GenOLC: delete_room: Deleting mortal start room!".to_string());
        g.r_mortal_start_room = 0;
    }
    if g.r_immort_start_room == rnum {
        g.log("WARNING: GenOLC: delete_room: Deleting immortal start room!".to_string());
        g.r_immort_start_room = 0;
    }
    if g.r_frozen_start_room == rnum {
        g.log("WARNING: GenOLC: delete_room: Deleting frozen start room!".to_string());
        g.r_frozen_start_room = 0;
    }

    // Deleting one of these is handled above; deleting a room *below* one is
    // not, and `add_room` has always incremented all three for exactly that
    // reason. Without the decrement a start room quietly points one room high
    // for the rest of the boot — and it is read on every login.
    // `check_start_rooms` resolves all three at boot and falls back rather
    // than leaving NOWHERE, so no guard is needed here, just as `add_room`
    // needs none.
    for r in [
        &mut g.r_mortal_start_room,
        &mut g.r_immort_start_room,
        &mut g.r_frozen_start_room,
    ] {
        if *r > rnum {
            *r -= 1;
        }
    }

    // Dump the contents into the Void.
    let objs: Vec<_> = g.rooms[ri].contents.clone();
    for oid in objs {
        crate::handler::obj_from_room(g, oid);
        crate::handler::obj_to_room(g, oid, 0);
    }
    let people: Vec<_> = g.rooms[ri].people.clone();
    for chid in people {
        crate::handler::char_from_room(g, chid);
        crate::handler::char_to_room(g, chid, 0);
    }

    if g.rooms[ri].script.is_some() {
        crate::dg::extract_script(g, crate::dg::GoId::Room(rnum));
    }
    g.world.rooms[ri].proto_script.clear();

    // Cancel this room's events.
    g.events.retain(|e| !matches!(&e.kind,
        EventKind::SplDarkness { room } if *room == rnum)
        && !matches!(&e.kind,
        EventKind::TrigWait { go: crate::dg::GoId::Room(r), .. } if *r == rnum));
    g.event_lists.remove(&EventOwner::Room(rnum));

    // Exits: retarget or drop, and shift the ones above.
    let dirs = crate::fight::dir_count(g);
    for r in g.world.rooms.iter_mut() {
        for slot in r.dir_option.iter_mut().take(dirs) {
            let Some(ex) = slot else { continue };
            if ex.to_room != NOWHERE && ex.to_room > rnum {
                ex.to_room -= 1;
            } else if ex.to_room == rnum {
                let bare = ex.keyword.as_ref().is_none_or(|k| k.is_empty())
                    && ex.general_description.as_ref().is_none_or(|d| d.is_empty());
                if bare {
                    *slot = None;
                } else {
                    ex.to_room = NOWHERE;
                }
            }
        }
    }

    // Zone commands: cancel the ones that pointed here, shift the rest.
    // A zone whose commands change here has to be written back out or
    // its.zon keeps pointing at a room that no longer exists.
    let nowhere = NOWHERE as i32;
    let mut unknown = 0;
    let mut touched: Vec<Idx> = Vec::new();
    for zi in 0..g.world.zones.len() {
        let n = g.world.zones[zi].cmds.len();
        let mut zone_touched = false;
        for ci in 0..n {
            let cmd = &mut g.world.zones[zi].cmds[ci];
            match cmd.command {
                b'M' | b'O' | b'T' | b'V' => {
                    if cmd.arg3 == rnum as i32 {
                        cmd.command = b'*';
                        zone_touched = true;
                    } else if cmd.arg3 > rnum as i32 && cmd.arg3 != nowhere {
                        cmd.arg3 -= 1;
                        zone_touched = true;
                    }
                }
                b'D' | b'R' => {
                    if cmd.arg1 == rnum as i32 {
                        cmd.command = b'*';
                        zone_touched = true;
                    } else if cmd.arg1 > rnum as i32 && cmd.arg1 != nowhere {
                        cmd.arg1 -= 1;
                        zone_touched = true;
                    }
                }
                b'G' | b'P' | b'E' | b'*' | b'S' => {}
                _ => unknown += 1,
            }
        }
        if zone_touched {
            touched.push(g.world.zones[zi].number);
        }
    }
    for zvnum in touched {
        add_to_save_list(g, zvnum, SL_ZON);
    }
    for _ in 0..unknown {
        g.mudlog(
            MudlogKind::Brf,
            LVL_GOD,
            true,
            "SYSERR: GenOLC: delete_room: Unknown zone entry found!",
        );
    }

    // Shop room lists hold vnums; a deleted room becomes the void.
    for shop in g.world.shops.iter_mut() {
        for room in shop.in_rooms.iter_mut() {
            if *room == vnum as i32 {
                *room = 0;
            }
        }
    }

    // Now move the rooms down.
    g.world.rooms.remove(ri);
    g.rooms.remove(ri);
    // The other half of add_room's pass: deleting a room takes its spec
    // proc with it and moves the rest down.
    g.room_specs.remove(ri);
    g.world.room_map.remove(&vnum);
    for v in g.world.room_map.values_mut() {
        if *v > rnum {
            *v -= 1;
        }
    }
    let ids: Vec<_> = g.character_list.iter().copied().collect();
    for id in ids {
        if let Some(c) = g.chars.get_mut(id) {
            if c.in_room != NOWHERE && c.in_room > rnum {
                c.in_room -= 1;
            }
            // `was_in_room` is a bare room rnum kept on the character, not a
            // link into any room's people list, so the pass above cannot
            // reach it — a linkdead character is not in the room it records.
            // This is `add_room`'s pass in reverse: the room they were pulled
            // out of is gone, so there is nothing to send them back to and the
            // reference is dropped; a room deleted below it moves their index
            // down like every other rnum here. NOWHERE is what the game loop
            // already tests for before returning anyone, so clearing it leaves
            // them where they are rather than somewhere arbitrary.
            if c.was_in_room == rnum {
                c.was_in_room = NOWHERE;
            } else if c.was_in_room != NOWHERE && c.was_in_room > rnum {
                c.was_in_room -= 1;
            }
        }
    }
    let oids: Vec<_> = g.object_list.iter().copied().collect();
    for id in oids {
        if let Some(o) = g.objs.get_mut(id) {
            if o.in_room != NOWHERE && o.in_room > rnum {
                o.in_room -= 1;
            }
        }
    }
    shift_room_events(g, rnum, -1);

    true
}

/// save_rooms. `rzone` is `None` for NOWHERE.
pub fn save_rooms(g: &mut Game, rzone: Option<usize>) -> bool {
    let top = g.world.zones.len().saturating_sub(1);
    let Some(rzone) = rzone.filter(|&z| z < g.world.zones.len()) else {
        g.log(format!(
            "SYSERR: GenOLC: save_rooms: Invalid zone number {} passed! (0-{})",
            NOWHERE, top
        ));
        return false;
    };
    let z = &g.world.zones[rzone];
    let (number, bot, ztop) = (z.number, z.bot, z.top);
    g.log(format!(
        "GenOLC: save_rooms: Saving rooms in zone #{} ({}-{}).",
        number, bot, ztop
    ));

    if write_world_file(g, rzone, SL_WLD).is_none() {
        // The file could not be opened.
        g.log("SYSERR: save_rooms: cannot write file".to_string());
        return false;
    }
    if in_save_list(g, number, SL_WLD) {
        remove_from_save_list(g, number, SL_WLD);
    }
    true
}
