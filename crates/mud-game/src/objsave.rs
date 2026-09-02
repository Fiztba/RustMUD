//! Rent, crash and cryo object files
//! (`lib/plrobjs/<bucket>/<name>.objs`).
//!
//! The file is a rent header line (the only `\r\n` line in the tree) followed
//! by delta-compressed object records: every field that still matches the
//! prototype is omitted, so a record is usually just `#vnum` plus the three
//! bit-array lines, which are always emitted: the "differs from prototype"
//! test compares array addresses there, and those are never equal (study
//! doc 04 §13.1). Containment rides on `Loc:`: worn = slot+1, inventory = 0,
//! contents = one less than the enclosing location, and contents are written
//! before their container so the loader can refill it.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::types::{ConState, *};
use mud_world::lex::{asciiflag_conv, tag_argument, Reader};
use mud_world::model::{ExtraDesc, ObjAffect, ObjProto};
use mud_world::players;

use crate::comm::{act, objs_vis, send_to_char, TO_NOTVICT, TO_ROOM, TO_VICT};
use crate::game::{Game, MudlogKind};
use crate::handler::{
    atoi, equip_char, extract_obj, invalid_align, invalid_class, obj_from_char, obj_to_char,
    obj_to_obj, unequip_char,
};

/// the per-day rent multiplier and the one-time cryo fee.
pub const RENT_FACTOR: i32 = 1;
pub const CRYO_FACTOR: i32 = 4;

const LOC_INVENTORY: i32 = 0;
/// Deepest container nesting a rent file can express.
pub const MAX_BAG_ROWS: usize = 5;

// Rent codes.
pub const RENT_UNDEF: i32 = 0;
pub const RENT_CRASH: i32 = 1;
pub const RENT_RENTED: i32 = 2;
pub const RENT_CRYO: i32 = 3;
pub const RENT_FORCED: i32 = 4;
pub const RENT_TIMEDOUT: i32 = 5;

// ---------------------------------------------------------------- writing

/// Drop every '\r'.
fn strip_cr(s: &[u8]) -> Vec<u8> {
    s.iter().copied().filter(|&b| b != b'\r').collect()
}

fn proto_of(g: &Game, oid: ObjId) -> Option<&ObjProto> {
    let rnum = g.obj(oid).item_number;
    if rnum == NOTHING {
        return None;
    }
    g.world.obj_protos.get(rnum as usize)
}

/// TEST_OBJS: write when either side is NULL or the text
/// differs. `cur` is the object's effective text, `proto` the prototype's.
fn text_differs(cur: Option<&[u8]>, proto: Option<&[u8]>) -> bool {
    match (cur, proto) {
        (Some(a), Some(b)) => a != b,
        _ => true,
    }
}

/// objsave_save_obj_record. Appends one record.
///
/// The diff is taken straight from the prototype row, which holds exactly
/// the values a throwaway instance would — same bytes, and no object churns
/// through the arena mid-save.
pub fn objsave_save_obj_record(g: &mut Game, oid: ObjId, out: &mut Vec<u8>, locate: i32) {
    let vnum = crate::dg::obj_vnum(g, oid);
    let proto = proto_of(g, oid).cloned();
    let o = g.obj(oid).clone();

    let p_values = proto.as_ref().map_or([0; 4], |p| p.values);
    let p_name = proto.as_ref().and_then(|p| p.name.as_deref());
    let p_short = proto.as_ref().and_then(|p| p.short_description.as_deref());
    let p_desc = proto.as_ref().and_then(|p| p.description.as_deref());
    let p_ades = proto.as_ref().and_then(|p| p.action_description.as_deref());

    out.extend_from_slice(format!("#{}\n", vnum).as_bytes());
    if locate != 0 {
        out.extend_from_slice(format!("Loc : {}\n", locate).as_bytes());
    }
    if o.values != p_values {
        out.extend_from_slice(
            format!("Vals: {} {} {} {}\n", o.values[0], o.values[1], o.values[2], o.values[3])
                .as_bytes(),
        );
    }
    // Always written: the "differs from prototype" test compares array
    // addresses, which are never equal.
    let f = o.extra_flags.0;
    out.extend_from_slice(format!("Flag: {} {} {} {}\n", f[0], f[1], f[2], f[3]).as_bytes());

    let name = o.name.as_deref().or(p_name);
    if text_differs(name, p_name) {
        out.extend_from_slice(b"Name: ");
        out.extend_from_slice(name.unwrap_or(b"Undefined"));
        out.push(b'\n');
    }
    let short = o.short_description.as_deref().or(p_short);
    if text_differs(short, p_short) {
        out.extend_from_slice(b"Shrt: ");
        out.extend_from_slice(short.unwrap_or(b"Undefined"));
        out.push(b'\n');
    }
    let desc = o.description.as_deref().or(p_desc);
    if text_differs(desc, p_desc) {
        out.extend_from_slice(b"Desc: ");
        out.extend_from_slice(desc.unwrap_or(b"Undefined"));
        out.push(b'\n');
    }
    let ades = o.action_description.as_deref().or(p_ades);
    if (ades.is_some() || p_ades.is_some()) && text_differs(ades, p_ades) {
        out.extend_from_slice(b"ADes:\n");
        out.extend_from_slice(&strip_cr(ades.unwrap_or(b"")));
        out.extend_from_slice(b"~\n");
    }

    if o.type_flag != proto.as_ref().map_or(0, |p| p.type_flag) {
        out.extend_from_slice(format!("Type: {}\n", o.type_flag).as_bytes());
    }
    if o.weight != proto.as_ref().map_or(0, |p| p.weight) {
        out.extend_from_slice(format!("Wght: {}\n", o.weight).as_bytes());
    }
    if o.cost != proto.as_ref().map_or(0, |p| p.cost) {
        out.extend_from_slice(format!("Cost: {}\n", o.cost).as_bytes());
    }
    if o.cost_per_day != proto.as_ref().map_or(0, |p| p.cost_per_day) {
        out.extend_from_slice(format!("Rent: {}\n", o.cost_per_day).as_bytes());
    }
    let a = o.perm_affects.0;
    out.extend_from_slice(format!("Perm: {} {} {} {}\n", a[0], a[1], a[2], a[3]).as_bytes());
    let w = o.wear_flags.0;
    out.extend_from_slice(format!("Wear: {} {} {} {}\n", w[0], w[1], w[2], w[3]).as_bytes());

    for i in 0..MAX_OBJ_AFFECT {
        let pm = proto.as_ref().map_or(0, |p| p.affected[i].modifier);
        if o.affected[i].modifier != pm {
            out.extend_from_slice(
                format!("Aff : {} {} {}\n", i, o.affected[i].location, o.affected[i].modifier)
                    .as_bytes(),
            );
        }
    }

    // Extra descriptions are written only when the object owns its own list
    // rather than sharing the prototype's.
    if let Some(list) = o.ex_descriptions.as_ref() {
        for ed in list {
            let (Some(kw), Some(d)) = (ed.keyword.as_deref(), ed.description.as_deref()) else {
                continue;
            };
            if kw.is_empty() || d.is_empty() {
                continue;
            }
            out.extend_from_slice(b"EDes:\n");
            out.extend_from_slice(kw);
            out.extend_from_slice(b"~\n");
            out.extend_from_slice(&strip_cr(d));
            out.extend_from_slice(b"~\n");
        }
    }

    out.push(b'\n');
}

/// Crash_save: walk the list backwards, each object
/// preceded by its own contents, subtracting the written object's weight
/// from every enclosing container as it goes.
fn crash_save_list(g: &mut Game, list: &[ObjId], out: &mut Vec<u8>, location: i32) {
    for &oid in list.iter().rev() {
        if g.try_obj(oid).is_none() {
            continue;
        }
        let contents = g.obj(oid).contains.clone();
        crash_save_list(g, &contents, out, location.min(0) - 1);
        objsave_save_obj_record(g, oid, out, location);
        // Take this object's weight back out of the containers that were
        // carrying it, so the record just written holds each container's own
        // weight.
        //
        // Only the ones that track it have it to give back: subtracting from
        // a container that never added it is what left an unlimited one
        // lighter by its contents every time it was saved, and negative if it
        // was saved often enough. Stop at the first container that does not
        // track, rather than skipping it and carrying on up: a weight that
        // stopped there never reached the containers above it either.
        // `crash_restore_weight` climbs the same unbroken run of tracking
        // containers and no further, so the two remain exact inverses.
        //
        // Contents were saved first, so `weight` here is already this
        // object's own.
        let w = g.obj(oid).weight;
        let mut up = g.obj(oid).in_obj;
        while let Some(t) = up {
            if !crate::handler::weight_gate_open(g, t) {
                break;
            }
            g.obj_mut(t).weight -= w;
            up = g.obj(t).in_obj;
        }
    }
}

fn crash_restore_weight(g: &mut Game, oid: ObjId) {
    let contents = g.obj(oid).contains.clone();
    for c in contents {
        if g.try_obj(c).is_some() {
            crash_restore_weight(g, c);
        }
    }
    // Contents are restored first, so a weight climbs the chain one hop per
    // level and stops at the first container that does not track -- the same
    // unbroken run the save above walks.
    let w = g.obj(oid).weight;
    if let Some(up) = g.obj(oid).in_obj {
        if crate::handler::weight_gate_open(g, up) {
            g.obj_mut(up).weight += w;
        }
    }
}

fn crash_restore_weight_list(g: &mut Game, list: &[ObjId]) {
    for &oid in list {
        if g.try_obj(oid).is_some() {
            crash_restore_weight(g, oid);
        }
    }
}

/// objsave_write_rentcode — the lone `\r\n` line.
fn write_rentcode(g: &Game, chid: CharId, rentcode: i32, cost_per_day: i32) -> Vec<u8> {
    let p = &g.ch(chid).points;
    format!("{} {} {} {} {} {}\r\n", rentcode, g.now, cost_per_day, p.gold, p.bank_gold, 0)
        .into_bytes()
}

fn objs_path(g: &Game, chid: CharId) -> Option<std::path::PathBuf> {
    let name = g.try_ch(chid)?.name.clone()?;
    let rel = players::get_filename(players::FileKind::Objs, &name)?;
    Some(g.lib_dir.join(rel))
}

fn objs_path_for_name(g: &Game, name: &[u8]) -> Option<std::path::PathBuf> {
    players::get_filename(players::FileKind::Objs, name).map(|rel| g.lib_dir.join(rel))
}

/// The printed name — relative to lib/ with forward slashes — rather than
/// the host path. Player-visible in Crash_listrent/House_listrent.
fn objs_name_for(name: &[u8]) -> Vec<u8> {
    // get_filename already builds it with '/' separators.
    players::get_filename(players::FileKind::Objs, name)
        .map(|p| p.to_string_lossy().replace('\\', "/").into_bytes())
        .unwrap_or_default()
}

fn write_objs_file(g: &mut Game, path: &std::path::Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, bytes) {
        g.log(format!("SYSERR: Couldn't write rent file {}: {}", path.display(), e));
    }
}

/// The body every save variant shares: eq slot by slot (location = slot+1),
/// then the inventory (location 0), then `$~`.
fn save_body(g: &mut Game, chid: CharId, out: &mut Vec<u8>, extract: bool) {
    for j in 0..NUM_WEARS {
        let Some(eq) = g.ch(chid).equipment[j] else { continue };
        crash_save_list(g, &[eq], out, j as i32 + 1);
        crash_restore_weight(g, eq);
        if extract {
            crash_extract_objs(g, eq);
        }
    }
    let carrying = g.ch(chid).carrying.clone();
    crash_save_list(g, &carrying, out, 0);
    if !extract {
        crash_restore_weight_list(g, &carrying);
    }
    out.extend_from_slice(b"$~\n");
    if extract {
        for oid in carrying {
            if g.try_obj(oid).is_some() {
                crash_extract_objs(g, oid);
            }
        }
    }
}

/// Crash_crashsave: everything as-is, norents included,
/// nothing extracted — the player keeps playing.
pub fn crash_crashsave(g: &mut Game, chid: CharId) {
    if g.ch(chid).is_npc() {
        return;
    }
    let Some(path) = objs_path(g, chid) else { return };
    let mut out = write_rentcode(g, chid, RENT_CRASH, 0);
    save_body(g, chid, &mut out, false);
    write_objs_file(g, &path, &out);
    g.ch_mut(chid).act.remove(flags::PLR_CRASH);
}

pub fn crash_rentsave(g: &mut Game, chid: CharId, cost: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let Some(path) = objs_path(g, chid) else { return };
    crash_extract_norent_eq(g, chid);
    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        if g.try_obj(oid).is_some() {
            crash_extract_norents(g, oid);
        }
    }
    let mut out = write_rentcode(g, chid, RENT_RENTED, cost);
    save_body(g, chid, &mut out, true);
    write_objs_file(g, &path, &out);
}

/// Crash_cryosave. The fee comes out of carried gold
/// only — the bank counted toward affordability but is never debited
/// (study doc 04 §7.7).
pub fn crash_cryosave(g: &mut Game, chid: CharId, cost: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let Some(path) = objs_path(g, chid) else { return };
    crash_extract_norent_eq(g, chid);
    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        if g.try_obj(oid).is_some() {
            crash_extract_norents(g, oid);
        }
    }
    let gold = g.ch(chid).points.gold;
    g.ch_mut(chid).points.gold = (gold - cost).max(0);

    let mut out = write_rentcode(g, chid, RENT_CRYO, 0);
    save_body(g, chid, &mut out, true);
    write_objs_file(g, &path, &out);
    g.ch_mut(chid).act.set(flags::PLR_CRYO);
}

/// Crash_idlesave: forced rent at double price, with
/// the eviction loop that destroys the priciest top-level item until the
/// bill fits.
pub fn crash_idlesave(g: &mut Game, chid: CharId) {
    if g.ch(chid).is_npc() {
        return;
    }
    let Some(path) = objs_path(g, chid) else { return };
    crash_extract_norent_eq(g, chid);
    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        if g.try_obj(oid).is_some() {
            crash_extract_norents(g, oid);
        }
    }

    let mut cost = 0;
    let carrying = g.ch(chid).carrying.clone();
    for oid in &carrying {
        if g.try_obj(*oid).is_some() {
            crash_calculate_rent(g, *oid, &mut cost);
        }
    }
    let mut cost_eq = 0;
    for j in 0..NUM_WEARS {
        if let Some(eq) = g.ch(chid).equipment[j] {
            crash_calculate_rent(g, eq, &mut cost_eq);
        }
    }
    cost += cost_eq;
    cost *= 2; // forcerent costs twice normal rent

    let purse = |g: &Game| g.ch(chid).points.gold + g.ch(chid).points.bank_gold;
    if cost > purse(g) {
        for j in 0..NUM_WEARS {
            if g.ch(chid).equipment[j].is_some() {
                if let Some(oid) = unequip_char(g, chid, j) {
                    obj_to_char(g, oid, chid);
                }
            }
        }
        while cost > purse(g) && !g.ch(chid).carrying.is_empty() {
            crash_extract_expensive(g, chid);
            cost = 0;
            let carrying = g.ch(chid).carrying.clone();
            for oid in &carrying {
                if g.try_obj(*oid).is_some() {
                    crash_calculate_rent(g, *oid, &mut cost);
                }
            }
            cost *= 2;
        }
    }

    if g.ch(chid).carrying.is_empty() && (0..NUM_WEARS).all(|j| g.ch(chid).equipment[j].is_none()) {
        // No equipment or inventory: nothing worth a file.
        crash_delete_file(g, chid);
        return;
    }

    let mut out = write_rentcode(g, chid, RENT_TIMEDOUT, cost);
    save_body(g, chid, &mut out, true);
    write_objs_file(g, &path, &out);
}

/// Crash_save_all: the autosave tick.
pub fn crash_save_all(g: &mut Game) {
    let ids: Vec<CharId> = g
        .descriptors
        .order
        .iter()
        .filter_map(|&di| g.descriptors.get(di))
        .filter(|d| d.state == ConState::Playing)
        .filter_map(|d| d.character)
        .collect();
    for chid in ids {
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.is_npc() || !ch.plr(flags::PLR_CRASH) {
            continue;
        }
        crash_crashsave(g, chid);
        crate::players_glue::save_char(g, chid);
        g.ch_mut(chid).act.remove(flags::PLR_CRASH);
    }
}

// -------------------------------------------------------------- norents

pub fn crash_is_unrentable(g: &mut Game, oid: ObjId) -> bool {
    let (norent, rent, unique, is_key) = {
        let o = g.obj(oid);
        (
            o.extra_flags.is_set(flags::ITEM_NORENT),
            o.cost_per_day,
            o.item_number == NOTHING,
            o.type_flag == flags::ITEM_KEY,
        )
    };
    if norent || rent < 0 || unique || is_key {
        let short = crate::handler::obj_short(g, oid).to_vec();
        g.log(format!(
            "Crash_is_unrentable: removing object {}",
            String::from_utf8_lossy(&short)
        ));
        return true;
    }
    false
}

fn crash_extract_norent_eq(g: &mut Game, chid: CharId) {
    for j in 0..NUM_WEARS {
        let Some(eq) = g.ch(chid).equipment[j] else { continue };
        if crash_is_unrentable(g, eq) {
            if let Some(oid) = unequip_char(g, chid, j) {
                obj_to_char(g, oid, chid);
            }
        } else {
            crash_extract_norents(g, eq);
        }
    }
}

/// Crash_extract_norents: depth-first over contents.
fn crash_extract_norents(g: &mut Game, oid: ObjId) {
    let contents = g.obj(oid).contains.clone();
    for c in contents {
        if g.try_obj(c).is_some() {
            crash_extract_norents(g, c);
        }
    }
    if g.try_obj(oid).is_some() && crash_is_unrentable(g, oid) {
        extract_obj(g, oid);
    }
}

fn crash_extract_objs(g: &mut Game, oid: ObjId) {
    let contents = g.obj(oid).contains.clone();
    for c in contents {
        if g.try_obj(c).is_some() {
            crash_extract_objs(g, c);
        }
    }
    if g.try_obj(oid).is_some() {
        extract_obj(g, oid);
    }
}

/// Crash_extract_expensive: the priciest TOP-LEVEL
/// carried item — container contents are never sold individually even
/// though their rent counts toward the bill.
fn crash_extract_expensive(g: &mut Game, chid: CharId) {
    let carrying = g.ch(chid).carrying.clone();
    let Some(&first) = carrying.first() else { return };
    let mut max = first;
    for &oid in &carrying {
        if g.obj(oid).cost_per_day > g.obj(max).cost_per_day {
            max = oid;
        }
    }
    extract_obj(g, max);
}

fn crash_calculate_rent(g: &Game, oid: ObjId, cost: &mut i32) {
    let o = g.obj(oid);
    *cost += o.cost_per_day.max(0);
    let contents = o.contains.clone();
    for c in contents {
        if g.try_obj(c).is_some() {
            crash_calculate_rent(g, c, cost);
        }
    }
}

// -------------------------------------------------------------- file mgmt

pub fn crash_delete_file(g: &mut Game, chid: CharId) -> bool {
    let Some(path) = objs_path(g, chid) else { return false };
    if !path.exists() {
        return false;
    }
    let _ = std::fs::remove_file(&path);
    true
}

/// Crash_delete_crashfile: only RENT_CRASH files die,
/// so gear dropped on the ground by a no-rent quit is never duplicated.
pub fn crash_delete_crashfile(g: &mut Game, chid: CharId) {
    let Some(path) = objs_path(g, chid) else { return };
    let Ok(data) = std::fs::read(&path) else { return };
    let first = data.split(|c| *c == b'\n').next().unwrap_or(b"");
    if atoi(first) == RENT_CRASH {
        let _ = std::fs::remove_file(&path);
    }
}

/// Crash_clean_file + update_obj_file (368-375): boot
/// expiry, header line only. Cryo files never expire.
pub fn update_obj_file(g: &mut Game) {
    let names: Vec<Vec<u8>> = g.player_table.iter().map(|p| p.name.clone()).collect();
    for name in names {
        if name.is_empty() {
            continue;
        }
        let Some(path) = objs_path_for_name(g, &name) else { continue };
        let Ok(data) = std::fs::read(&path) else { continue };
        let Some(line) = Reader::new(&data).get_line() else { continue };
        let t = scan_ints(&line, 6);
        let (rentcode, timed) = (t[0], t[1] as i64);

        let pname = String::from_utf8_lossy(&name).into_owned();
        if rentcode == RENT_CRASH || rentcode == RENT_FORCED || rentcode == RENT_TIMEDOUT {
            if timed < g.now - (g.config.crash_file_timeout as i64 * SECS_PER_REAL_DAY as i64) {
                let _ = std::fs::remove_file(&path);
                let filetype = match rentcode {
                    RENT_CRASH => "crash",
                    RENT_FORCED => "forced rent",
                    RENT_TIMEDOUT => "idlesave",
                    _ => "UNKNOWN!",
                };
                g.log(format!("    Deleting {}'s {} file.", pname, filetype));
            }
        } else if rentcode == RENT_RENTED
            && timed < g.now - (g.config.rent_file_timeout as i64 * SECS_PER_REAL_DAY as i64)
        {
            let _ = std::fs::remove_file(&path);
            g.log(format!("    Deleting {}'s rent file.", pname));
        }
    }
}

// ---------------------------------------------------------------- parsing

/// One parsed record: the instantiated object and its saved location.
pub struct ObjSaveData {
    pub obj: ObjId,
    pub locate: i32,
}

/// objsave_parse_objects. Reads records from the
/// reader's current position; unknown vnums are skipped whole, `#65535`
/// makes a unique object.
pub fn objsave_parse_objects(g: &mut Game, r: &mut Reader) -> Vec<ObjSaveData> {
    let mut out: Vec<ObjSaveData> = Vec::new();
    let mut temp: Option<ObjId> = None;
    let mut locate: i32 = 0;

    loop {
        let Some(line) = r.get_line() else { break };
        if line.starts_with(b"$~") {
            break;
        }

        if line.first() == Some(&b'#') {
            let Some(nr) = scan_vnum(&line) else { continue };
            if nr != NOTHING as i32 && g.world.real_object(nr as Idx).is_none() {
                g.log(format!("SYSERR: Prevented loading of non-existant item #{}.", nr));
                continue;
            }
            if let Some(prev) = temp.take() {
                out.push(ObjSaveData { obj: prev, locate });
                locate = 0;
            }
            if nr == NOTHING as i32 {
                let o = crate::obj::create_obj();
                let id = g.objs.insert(o);
                g.object_list.push_front(id);
                temp = Some(id);
            } else if nr < 0 {
                continue;
            } else if let Some(rnum) = g.world.real_object(nr as Idx) {
                temp = crate::db::read_object(g, rnum);
            } else {
                g.log(format!("Nonexistent object {} found in rent file.", nr));
            }
            continue;
        }

        let Some(oid) = temp else { continue };
        let (tag, value) = tag_argument(&line);
        let num = atoi(&value);
        match tag.as_slice() {
            b"ADes" => {
                let s = r.fread_string("rent(Ades)").ok().flatten();
                g.obj_mut(oid).action_description = s;
            }
            b"Aff " => {
                let t = scan_ints(&value, 3);
                let slot = t[0] as usize;
                if slot < MAX_OBJ_AFFECT {
                    g.obj_mut(oid).affected[slot] = ObjAffect { location: t[1], modifier: t[2] };
                }
            }
            b"Cost" => g.obj_mut(oid).cost = num,
            b"Desc" => g.obj_mut(oid).description = Some(value.clone()),
            b"EDes" => {
                let kw = r.fread_string("rent(Edes)").ok().flatten();
                let d = r.fread_string("rent(Edes)").ok().flatten();
                // A prototype-shared list is detached before prepending; the
                // override Option is that detachment, structurally.
                let list = g.obj_mut(oid).ex_descriptions.get_or_insert_with(Vec::new);
                list.insert(0, ExtraDesc { keyword: kw, description: d });
            }
            b"Flag" => {
                let w = scan_flags(&value);
                g.obj_mut(oid).extra_flags = flags::FlagSet::from_words(w);
            }
            b"Loc " => locate = num,
            b"Name" => g.obj_mut(oid).name = Some(value.clone()),
            b"Perm" => {
                let w = scan_flags(&value);
                g.obj_mut(oid).perm_affects = flags::FlagSet::from_words(w);
            }
            b"Rent" => g.obj_mut(oid).cost_per_day = num,
            b"Shrt" => g.obj_mut(oid).short_description = Some(value.clone()),
            b"Type" => g.obj_mut(oid).type_flag = num,
            b"Wear" => {
                let w = scan_flags(&value);
                g.obj_mut(oid).wear_flags = flags::FlagSet::from_words(w);
            }
            b"Wght" => g.obj_mut(oid).weight = num,
            b"Vals" => {
                let t = scan_ints(&value, 4);
                g.obj_mut(oid).values = [t[0], t[1], t[2], t[3]];
            }
            _ => g.log(format!("Unknown tag in rentfile: {}", String::from_utf8_lossy(&tag))),
        }
    }

    if let Some(last) = temp {
        out.push(ObjSaveData { obj: last, locate });
    }
    out
}

/// A '#' line whose remainder is not a number is not a record.
fn scan_vnum(line: &[u8]) -> Option<i32> {
    let rest = &line[1..];
    let mut i = 0;
    if rest.first() == Some(&b'-') {
        i = 1;
    }
    if !rest.get(i).is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(atoi(rest))
}

fn scan_ints(s: &[u8], n: usize) -> Vec<i32> {
    let mut out: Vec<i32> = s
        .split(|b| *b == b' ' || *b == b'\t')
        .filter(|t| !t.is_empty())
        .map(|t| atoi(t))
        .collect();
    out.resize(n, 0);
    out
}

/// The four bitvector tokens go through asciiflag_conv, which passes
/// all-numeric tokens straight to atol.
fn scan_flags(s: &[u8]) -> [u32; 4] {
    let toks: Vec<&[u8]> =
        s.split(|b| *b == b' ' || *b == b'\t').filter(|t| !t.is_empty()).collect();
    let mut w = [0u32; 4];
    for (i, slot) in w.iter_mut().enumerate() {
        if let Some(t) = toks.get(i) {
            *slot = asciiflag_conv(t);
        }
    }
    w
}

// ---------------------------------------------------------------- loading

/// auto_equip: validate the saved slot against the
/// object's wear flags, then equip only if the slot is free and neither
/// alignment nor class objects. Anything else lands in inventory.
fn auto_equip(g: &mut Game, chid: CharId, oid: ObjId, mut location: i32) {
    if location > 0 {
        let j = (location - 1) as usize;
        let can = |g: &Game, bit: usize| g.obj(oid).can_wear(bit);
        let ok = match j {
            WEAR_LIGHT => true,
            WEAR_FINGER_R | WEAR_FINGER_L => can(g, flags::ITEM_WEAR_FINGER),
            WEAR_NECK_1 | WEAR_NECK_2 => can(g, flags::ITEM_WEAR_NECK),
            WEAR_BODY => can(g, flags::ITEM_WEAR_BODY),
            WEAR_HEAD => can(g, flags::ITEM_WEAR_HEAD),
            WEAR_LEGS => can(g, flags::ITEM_WEAR_LEGS),
            WEAR_FEET => can(g, flags::ITEM_WEAR_FEET),
            WEAR_HANDS => can(g, flags::ITEM_WEAR_HANDS),
            WEAR_ARMS => can(g, flags::ITEM_WEAR_ARMS),
            WEAR_SHIELD => can(g, flags::ITEM_WEAR_SHIELD),
            WEAR_ABOUT => can(g, flags::ITEM_WEAR_ABOUT),
            WEAR_WAIST => can(g, flags::ITEM_WEAR_WAIST),
            WEAR_WRIST_R | WEAR_WRIST_L => can(g, flags::ITEM_WEAR_WRIST),
            WEAR_WIELD => can(g, flags::ITEM_WEAR_WIELD),
            WEAR_HOLD => {
                can(g, flags::ITEM_WEAR_HOLD)
                    || (g.ch(chid).class == CLASS_WARRIOR
                        && can(g, flags::ITEM_WEAR_WIELD)
                        && g.obj(oid).type_flag == flags::ITEM_WEAPON)
            }
            _ => false,
        };
        if !ok {
            location = LOC_INVENTORY;
        }

        if location > 0 {
            if g.ch(chid).equipment[j].is_none() {
                if invalid_align(g, chid, oid) || invalid_class(g, chid, oid) {
                    location = LOC_INVENTORY;
                } else {
                    equip_char(g, chid, oid, j);
                }
            } else {
                let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                let invis = g.ch(chid).invis_lev();
                g.mudlog(
                    MudlogKind::Brf,
                    (LVL_IMMORT as i16).max(invis) as u8,
                    true,
                    &format!(
                        "SYSERR: autoeq: '{}' already equipped in position {}.",
                        name, location
                    ),
                );
                location = LOC_INVENTORY;
            }
        }
    }
    if location <= 0 {
        obj_to_char(g, oid, chid);
    }
}

/// handle_obj: the cont_row machine that rebuilds
/// containment from the flat record list.
fn handle_obj(
    g: &mut Game,
    oid: ObjId,
    chid: CharId,
    locate: i32,
    cont_row: &mut [Vec<ObjId>; MAX_BAG_ROWS],
) -> bool {
    auto_equip(g, chid, oid, locate);

    if locate > 0 {
        // Equipped: any deeper pending rows lost their container.
        for j in (1..MAX_BAG_ROWS).rev() {
            for o in std::mem::take(&mut cont_row[j]) {
                obj_to_char(g, o, chid);
            }
        }
        if !cont_row[0].is_empty() {
            if g.obj(oid).type_flag == flags::ITEM_CONTAINER {
                // Unequip, fill, re-equip so the container's weight is right
                // before it lands back on the character.
                let pos = (locate - 1) as usize;
                if let Some(cont) = unequip_char(g, chid, pos) {
                    g.obj_mut(cont).contains.clear();
                    for o in std::mem::take(&mut cont_row[0]) {
                        obj_to_obj(g, o, cont);
                    }
                    equip_char(g, chid, cont, pos);
                }
            } else {
                for o in std::mem::take(&mut cont_row[0]) {
                    obj_to_char(g, o, chid);
                }
            }
        }
    } else {
        let mut j = MAX_BAG_ROWS - 1;
        while j as i32 > -locate {
            for o in std::mem::take(&mut cont_row[j]) {
                obj_to_char(g, o, chid);
            }
            j -= 1;
        }

        if j as i32 == -locate && !cont_row[j].is_empty() {
            if g.obj(oid).type_flag == flags::ITEM_CONTAINER {
                obj_from_char(g, oid);
                g.obj_mut(oid).contains.clear();
                for o in std::mem::take(&mut cont_row[j]) {
                    obj_to_obj(g, o, oid);
                }
                obj_to_char(g, oid, chid);
            } else {
                for o in std::mem::take(&mut cont_row[j]) {
                    obj_to_char(g, o, chid);
                }
            }
        }

        if locate < 0 && locate >= -(MAX_BAG_ROWS as i32) {
            // Append (not prepend) so contents keep their pre-rent order.
            obj_from_char(g, oid);
            cont_row[(-locate - 1) as usize].push(oid);
        }
    }
    true
}

/// Crash_load → Crash_load_objs (1176-1275).
/// Returns 0 (stay in the rent room), 1 (crash items / no file) or 2 (rent
/// unpaid — everything lost).
pub fn crash_load(g: &mut Game, chid: CharId) -> i32 {
    let Some(path) = objs_path(g, chid) else { return 1 };
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    let imm_lvl = (LVL_IMMORT as i16).max(invis) as u8;

    let Ok(data) = std::fs::read(&path) else {
        g.mudlog(
            MudlogKind::Nrm,
            imm_lvl,
            true,
            &format!("{} entering game with no equipment.", name),
        );
        return 1;
    };

    let mut r = Reader::new(&data);
    let mut rentcode = RENT_UNDEF;
    let mut timed: i64 = 0;
    let mut netcost = 0;
    match r.get_line() {
        None => g.mudlog(
            MudlogKind::Nrm,
            imm_lvl,
            true,
            &format!("Failed to read player's rent code: {}.", name),
        ),
        Some(line) => {
            let t = scan_ints(&line, 6);
            rentcode = t[0];
            timed = t[1] as i64;
            netcost = t[2];
        }
    }

    if rentcode == RENT_RENTED || rentcode == RENT_TIMEDOUT {
        // (int)((float)(now - timed) / SECS_PER_REAL_DAY) — truncating.
        let num_of_days = ((g.now - timed) as f32 / SECS_PER_REAL_DAY as f32) as i32;
        let cost = netcost * num_of_days;
        let (gold, bank) = {
            let p = &g.ch(chid).points;
            (p.gold, p.bank_gold)
        };
        if cost > gold + bank {
            g.mudlog(
                MudlogKind::Brf,
                imm_lvl,
                true,
                &format!("{} entering game, rented equipment lost (no $).", name),
            );
            crash_crashsave(g, chid);
            return 2;
        }
        g.ch_mut(chid).points.bank_gold = bank - (cost - gold).max(0);
        g.ch_mut(chid).points.gold = (gold - cost).max(0);
        crate::players_glue::save_char(g, chid);
    }

    let msg = match rentcode {
        RENT_RENTED => format!("{} un-renting and entering game.", name),
        RENT_CRASH => format!("{} retrieving crash-saved items and entering game.", name),
        RENT_CRYO => format!("{} un-cryo'ing and entering game.", name),
        RENT_FORCED | RENT_TIMEDOUT => {
            format!("{} retrieving force-saved items and entering game.", name)
        }
        _ => format!("WARNING: {} entering game with undefined rent code.", name),
    };
    g.mudlog(MudlogKind::Nrm, imm_lvl, true, &msg);

    let loaded = objsave_parse_objects(g, &mut r);
    let mut cont_row: [Vec<ObjId>; MAX_BAG_ROWS] = Default::default();
    let mut num_objs = 0;
    for rec in loaded {
        if g.try_obj(rec.obj).is_none() {
            continue;
        }
        if handle_obj(g, rec.obj, chid, rec.locate, &mut cont_row) {
            num_objs += 1;
        }
    }

    let level = g.ch(chid).level;
    let max_save = g.config.max_obj_save;
    let god_lvl = (LVL_GOD as i16).max(invis) as u8;
    g.mudlog(
        MudlogKind::Nrm,
        god_lvl,
        true,
        &format!(
            "{} (level {}) has {} object{} (max {}).",
            name,
            level,
            num_objs,
            if num_objs != 1 { "s" } else { "" },
            max_save
        ),
    );

    if rentcode == RENT_RENTED || rentcode == RENT_CRYO {
        0
    } else {
        1
    }
}

/// Crash_listrent — the imm rent-file view.
pub fn crash_listrent(g: &mut Game, chid: CharId, name: &[u8]) {
    let Some(path) = objs_path_for_name(g, name) else { return };
    let Ok(data) = std::fs::read(&path) else {
        let mut msg = name.to_vec();
        msg.extend_from_slice(b" has no rent file.\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    let mut out = objs_name_for(name);
    out.extend_from_slice(b"\r\n");

    let mut r = Reader::new(&data);
    let Some(line) = r.get_line() else {
        send_to_char(g, chid, b"Error reading rent information.\r\n");
        return;
    };
    let t = scan_ints(&line, 6);
    out.extend_from_slice(match t[0] {
        RENT_RENTED => b"Rent\r\n".as_ref(),
        RENT_CRASH => b"Crash\r\n".as_ref(),
        RENT_CRYO => b"Cryo\r\n".as_ref(),
        RENT_TIMEDOUT | RENT_FORCED => b"TimedOut\r\n".as_ref(),
        _ => b"Undef\r\n".as_ref(),
    });

    let loaded = objsave_parse_objects(g, &mut r);
    for rec in &loaded {
        if g.try_obj(rec.obj).is_none() {
            continue;
        }
        let vnum = crate::dg::obj_vnum(g, rec.obj);
        let rent = g.obj(rec.obj).cost_per_day;
        let short = crate::handler::obj_short(g, rec.obj).to_vec();
        out.extend_from_slice(format!("[{:5}] ({:5}au) ", vnum, rent).as_bytes());
        let mut padded = short.clone();
        while padded.len() < 20 {
            padded.push(b' ');
        }
        out.extend_from_slice(&padded);
        out.extend_from_slice(b"\r\n");
    }
    for rec in loaded {
        if g.try_obj(rec.obj).is_some() {
            extract_obj(g, rec.obj);
        }
    }
    crate::act::informative::page_string(g, chid, &out);
}

// ----------------------------------------------------------- receptionist

fn crash_rent_deadline(g: &mut Game, chid: CharId, recep: CharId, cost: i32) {
    if cost == 0 {
        return;
    }
    let p = &g.ch(chid).points;
    let days = (p.gold + p.bank_gold) / cost;
    let msg = format!(
        "$n tells you, 'You can rent for {} day{} with the gold you have\r\non hand and in the bank.'\r\n",
        days,
        if days != 1 { "s" } else { "" }
    );
    act(g, msg.as_bytes(), false, Some(recep), None, Some(chid), TO_VICT);
}

/// Crash_report_unrentables.
fn report_unrentables(g: &mut Game, chid: CharId, recep: CharId, list: &[ObjId]) -> i32 {
    let mut has = 0;
    for &oid in list {
        if g.try_obj(oid).is_none() {
            continue;
        }
        if crash_is_unrentable(g, oid) {
            has = 1;
            let short = objs_vis(g, oid, chid);
            let mut msg = b"$n tells you, 'You cannot store ".to_vec();
            msg.extend_from_slice(&short);
            msg.extend_from_slice(b".'");
            act(g, &msg, false, Some(recep), None, Some(chid), TO_VICT);
        }
        let contents = g.obj(oid).contains.clone();
        has += report_unrentables(g, chid, recep, &contents);
    }
    has
}

/// Crash_report_rent.
#[allow(clippy::too_many_arguments)]
fn report_rent(
    g: &mut Game,
    chid: CharId,
    recep: CharId,
    list: &[ObjId],
    cost: &mut i64,
    nitems: &mut i64,
    display: bool,
    factor: i32,
) {
    for &oid in list {
        if g.try_obj(oid).is_none() {
            continue;
        }
        if !crash_is_unrentable(g, oid) {
            *nitems += 1;
            let rent = g.obj(oid).cost_per_day * factor;
            *cost += rent.max(0) as i64;
            if display {
                let short = objs_vis(g, oid, chid);
                let mut msg = format!("$n tells you, '{:5} coins for ", rent).into_bytes();
                msg.extend_from_slice(&short);
                msg.extend_from_slice(b"..'");
                act(g, &msg, false, Some(recep), None, Some(chid), TO_VICT);
            }
        }
        let contents = g.obj(oid).contains.clone();
        report_rent(g, chid, recep, &contents, cost, nitems, display, factor);
    }
}

/// Crash_offer_rent: 0 means "no deal".
fn crash_offer_rent(g: &mut Game, chid: CharId, recep: CharId, display: bool, factor: i32) -> i32 {
    let carrying = g.ch(chid).carrying.clone();
    let mut norent = report_unrentables(g, chid, recep, &carrying);
    for i in 0..NUM_WEARS {
        if let Some(eq) = g.ch(chid).equipment[i] {
            norent += report_unrentables(g, chid, recep, &[eq]);
        }
    }
    if norent != 0 {
        return 0;
    }

    let mut totalcost = (g.config.min_rent_cost * factor) as i64;
    let mut numitems = 0i64;
    let carrying = g.ch(chid).carrying.clone();
    report_rent(g, chid, recep, &carrying, &mut totalcost, &mut numitems, display, factor);
    for i in 0..NUM_WEARS {
        if let Some(eq) = g.ch(chid).equipment[i] {
            report_rent(g, chid, recep, &[eq], &mut totalcost, &mut numitems, display, factor);
        }
    }

    if numitems == 0 {
        act(
            g,
            b"$n tells you, 'But you are not carrying anything!  Just quit!'",
            false,
            Some(recep),
            None,
            Some(chid),
            TO_VICT,
        );
        return 0;
    }
    if numitems > g.config.max_obj_save as i64 {
        let msg = format!(
            "$n tells you, 'Sorry, but I cannot store more than {} items.'",
            g.config.max_obj_save
        );
        act(g, msg.as_bytes(), false, Some(recep), None, Some(chid), TO_VICT);
        return 0;
    }
    if display {
        let msg =
            format!("$n tells you, 'Plus, my {} coin fee..'", g.config.min_rent_cost * factor);
        act(g, msg.as_bytes(), false, Some(recep), None, Some(chid), TO_VICT);
        let msg = format!(
            "$n tells you, 'For a total of {} coins{}.'",
            totalcost,
            if factor == RENT_FACTOR { " per day" } else { "" }
        );
        act(g, msg.as_bytes(), false, Some(recep), None, Some(chid), TO_VICT);
        let p = &g.ch(chid).points;
        if totalcost > (p.gold + p.bank_gold) as i64 {
            act(
                g,
                b"$n tells you, '...which I see you can't afford.'",
                false,
                Some(recep),
                None,
                Some(chid),
                TO_VICT,
            );
            return 0;
        } else if factor == RENT_FACTOR {
            crash_rent_deadline(g, chid, recep, totalcost as i32);
        }
    }
    totalcost as i32
}

/// gen_receptionist. `cmd == 0` is the mobact call: a
/// 1-in-6 chance of a random social, and those draws are load-bearing.
pub fn gen_receptionist(
    g: &mut Game,
    chid: CharId,
    recep: CharId,
    cmd: usize,
    _arg: &[u8],
    mode: i32,
) -> bool {
    const ACTION_TABLE: [&[u8]; 9] = [
        b"smile", b"dance", b"sigh", b"blush", b"burp", b"cough", b"fart", b"twiddle", b"yawn",
    ];
    if cmd == 0 && g.rng.rand_number(0, 5) == 0 {
        let which = g.rng.rand_number(0, 8) as usize;
        if let Some(social_cmd) = crate::interpreter::find_command(g, ACTION_TABLE[which]) {
            crate::act::social::do_action(g, recep, b"", social_cmd, 0);
        }
        return false;
    }

    if g.ch(chid).desc.is_none() || g.ch(chid).is_npc() {
        return false;
    }
    if !crate::interpreter::cmd_is(g, cmd, b"offer") && !crate::interpreter::cmd_is(g, cmd, b"rent")
    {
        return false;
    }

    if !g.ch(recep).awake() {
        let mut msg = crate::comm::hssh(g.ch(recep).sex).to_vec();
        msg.extend_from_slice(b" is unable to talk to you...\r\n");
        send_to_char(g, chid, &msg);
        return true;
    }
    if !crate::handler::can_see(g, recep, chid) {
        act(
            g,
            b"$n says, 'I don't deal with people I can't see!'",
            false,
            Some(recep),
            None,
            None,
            TO_ROOM,
        );
        return true;
    }

    if g.config.free_rent {
        act(
            g,
            b"$n tells you, 'Rent is free here.  Just quit, and your objects will be saved!'",
            false,
            Some(recep),
            None,
            Some(chid),
            TO_VICT,
        );
        return true;
    }

    if crate::interpreter::cmd_is(g, cmd, b"rent") {
        let cost = crash_offer_rent(g, chid, recep, false, mode);
        if cost == 0 {
            return true;
        }
        let msg = if mode == RENT_FACTOR {
            format!("$n tells you, 'Rent will cost you {} gold coins per day.'", cost)
        } else {
            format!("$n tells you, 'It will cost you {} gold coins to be frozen.'", cost)
        };
        act(g, msg.as_bytes(), false, Some(recep), None, Some(chid), TO_VICT);

        let (gold, bank) = {
            let p = &g.ch(chid).points;
            (p.gold, p.bank_gold)
        };
        if cost > gold + bank {
            act(
                g,
                b"$n tells you, '...which I see you can't afford.'",
                false,
                Some(recep),
                None,
                Some(chid),
                TO_VICT,
            );
            return true;
        }
        if mode == RENT_FACTOR {
            crash_rent_deadline(g, chid, recep, cost);
        }

        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        let imm_lvl = (LVL_IMMORT as i16).max(invis) as u8;
        if mode == RENT_FACTOR {
            act(
                g,
                b"$n stores your belongings and helps you into your private chamber.",
                false,
                Some(recep),
                None,
                Some(chid),
                TO_VICT,
            );
            crash_rentsave(g, chid, cost);
            let purse = g.ch(chid).points.gold + g.ch(chid).points.bank_gold;
            g.mudlog(
                MudlogKind::Nrm,
                imm_lvl,
                true,
                &format!("{} has rented ({}/day, {} tot.)", name, cost, purse),
            );
        } else {
            act(
                g,
                b"$n stores your belongings and helps you into your private chamber.\r\nA white mist appears in the room, chilling you to the bone...\r\nYou begin to lose consciousness...",
                false,
                Some(recep),
                None,
                Some(chid),
                TO_VICT,
            );
            crash_cryosave(g, chid, cost);
            g.mudlog(MudlogKind::Nrm, imm_lvl, true, &format!("{} has cryo-rented.", name));
            g.ch_mut(chid).act.set(flags::PLR_CRYO);
        }

        act(
            g,
            b"$n helps $N into $S private chamber.",
            false,
            Some(recep),
            None,
            Some(chid),
            TO_NOTVICT,
        );

        let room = g.ch(chid).in_room;
        if room != NOWHERE {
            let vnum = g.world.rooms[room as usize].vnum;
            g.ch_mut(chid).ps_mut().load_room = vnum;
        }
        crate::handler::extract_char(g, chid); // It saves.
    } else {
        crash_offer_rent(g, chid, recep, true, mode);
        act(g, b"$N gives $n an offer.", false, Some(chid), None, Some(recep), TO_ROOM);
    }
    true
}
