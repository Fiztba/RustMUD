//! Player houses: the control file, per-house object files, the
//! `hcontrol` admin command and the `house` guest list.
//!
//! House object records carry the same `Loc:` depth encoding rent files use,
//! so what is inside a chest is still inside it after a reboot. A file with
//! no `Loc:` lines loads flat.
//!
//! `lib/etc/hcontrol` is a versioned ASCII file. Two older binary layouts --
//! 64-bit and 32-bit images of the control array, whose meaning depended on
//! pointer width and padding -- are still read so an existing control file is
//! not lost, but neither is ever written.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::comm::send_to_char;
use crate::game::{Game, MudlogKind};
use crate::handler::{
    atoi, extract_obj, is_abbrev, obj_to_obj, obj_to_room, remove_room_flag, rev_dir,
    room_flagged, set_room_flag,
};
use crate::interpreter::{half_chop, one_argument};
use crate::objsave::{objsave_parse_objects, objsave_save_obj_record, MAX_BAG_ROWS};
use mud_world::lex::{tag_argument, Reader};

pub const MAX_HOUSES: usize = 100;
pub const MAX_GUESTS: usize = 10;
pub const HOUSE_PRIVATE: i32 = 0;

/// struct house_control_rec.
#[derive(Debug, Clone, Default)]
pub struct HouseControl {
    pub vnum: i32,
    pub atrium: i32,
    pub exit_num: i32,
    pub built_on: i64,
    pub mode: i32,
    pub owner: i64,
    pub guests: Vec<i64>,
    pub last_payment: i64,
}

const HCONTROL_FORMAT: &[u8] = b"Usage: hcontrol build <house vnum> <exit direction> <player name>\r\n       hcontrol destroy <house vnum>\r\n       hcontrol pay <house vnum>\r\n       hcontrol show [house vnum | .]\r\n";

fn hcontrol_path(g: &Game) -> std::path::PathBuf {
    g.lib_dir.join("etc").join("hcontrol")
}

fn house_file_path(g: &Game, vnum: i32) -> std::path::PathBuf {
    g.lib_dir.join("house").join(format!("{}.house", vnum))
}

pub fn find_house(g: &Game, vnum: i32) -> Option<usize> {
    g.houses.iter().position(|h| h.vnum == vnum)
}

// -------------------------------------------------------- control file I/O

/// The legacy raw-struct layouts. Record size identifies which compiler
/// wrote the file: 192 bytes = LP64 (Linux/macOS x86-64), 100 = ILP32.
fn parse_binary_control(data: &[u8]) -> Option<Vec<HouseControl>> {
    let (rec, long_sz) = if !data.is_empty() && data.len() % 192 == 0 {
        (192usize, 8usize)
    } else if !data.is_empty() && data.len() % 100 == 0 {
        (100usize, 4usize)
    } else {
        return None;
    };
    let rd = |b: &[u8], off: usize, sz: usize| -> i64 {
        match sz {
            2 => u16::from_le_bytes([b[off], b[off + 1]]) as i64,
            4 => i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]) as i64,
            _ => i64::from_le_bytes(b[off..off + 8].try_into().unwrap()),
        }
    };
    // Offsets follow the natural alignment of the struct on each ABI.
    let (o_built, o_mode, o_owner, o_guests, o_ngue) = if long_sz == 8 {
        (8usize, 16usize, 24usize, 40usize, 32usize)
    } else {
        (8usize, 12usize, 16usize, 24usize, 20usize)
    };
    let o_pay = o_guests + MAX_GUESTS * long_sz;

    let mut out = Vec::new();
    for chunk in data.chunks_exact(rec) {
        let num_guests = rd(chunk, o_ngue, 4).clamp(0, MAX_GUESTS as i64) as usize;
        out.push(HouseControl {
            vnum: rd(chunk, 0, 2) as i32,
            atrium: rd(chunk, 2, 2) as i32,
            exit_num: rd(chunk, 4, 2) as i32,
            built_on: rd(chunk, o_built, long_sz),
            mode: rd(chunk, o_mode, 4) as i32,
            owner: rd(chunk, o_owner, long_sz),
            guests: (0..num_guests).map(|i| rd(chunk, o_guests + i * long_sz, long_sz)).collect(),
            last_payment: rd(chunk, o_pay, long_sz),
        });
    }
    Some(out)
}

fn parse_ascii_control(data: &[u8]) -> Vec<HouseControl> {
    let mut out: Vec<HouseControl> = Vec::new();
    let mut cur: Option<HouseControl> = None;
    let mut r = Reader::new(data);
    while let Some(line) = r.get_line() {
        if line.starts_with(b"$~") {
            break;
        }
        if line.first() == Some(&b'#') {
            if let Some(h) = cur.take() {
                out.push(h);
            }
            cur = Some(HouseControl {
                vnum: atoi(&line[1..]),
                mode: HOUSE_PRIVATE,
                ..Default::default()
            });
            continue;
        }
        let Some(h) = cur.as_mut() else { continue };
        let (tag, value) = tag_argument(&line);
        match tag.as_slice() {
            b"Atrm" => h.atrium = atoi(&value),
            b"Exit" => h.exit_num = atoi(&value),
            b"Bilt" => h.built_on = atoi(&value) as i64,
            b"Mode" => h.mode = atoi(&value),
            b"Ownr" => h.owner = atoi(&value) as i64,
            b"Pay " => h.last_payment = atoi(&value) as i64,
            b"Gsts" => {
                h.guests = value
                    .split(|b| *b == b' ')
                    .filter(|t| !t.is_empty())
                    .map(|t| atoi(t) as i64)
                    .take(MAX_GUESTS)
                    .collect()
            }
            _ => {}
        }
    }
    if let Some(h) = cur {
        out.push(h);
    }
    out
}

/// House_save_control, D3-style: versioned ASCII.
pub fn house_save_control(g: &mut Game) {
    let mut out = b"* tbaMUD house control file (ASCII v1)\n".to_vec();
    for h in &g.houses {
        out.extend_from_slice(format!("#{}\n", h.vnum).as_bytes());
        out.extend_from_slice(format!("Atrm: {}\n", h.atrium).as_bytes());
        out.extend_from_slice(format!("Exit: {}\n", h.exit_num).as_bytes());
        out.extend_from_slice(format!("Bilt: {}\n", h.built_on).as_bytes());
        out.extend_from_slice(format!("Mode: {}\n", h.mode).as_bytes());
        out.extend_from_slice(format!("Ownr: {}\n", h.owner).as_bytes());
        out.extend_from_slice(format!("Pay : {}\n", h.last_payment).as_bytes());
        if !h.guests.is_empty() {
            let ids: Vec<String> = h.guests.iter().map(|i| i.to_string()).collect();
            out.extend_from_slice(format!("Gsts: {}\n", ids.join(" ")).as_bytes());
        }
    }
    out.extend_from_slice(b"$~\n");
    let path = hcontrol_path(g);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, &out) {
        g.log(format!("SYSERR: Unable to save house control file: {}", e));
    }
}

/// House_boot. Records whose owner, rooms or exit no
/// longer check out are dropped, and the file is rewritten without them.
pub fn house_boot(g: &mut Game) {
    let path = hcontrol_path(g);
    let Ok(data) = std::fs::read(&path) else {
        // The logged path is the compile-time constant, relative to lib/.
        g.log("   No houses to load. File 'etc/hcontrol' does not exist.".to_string());
        let _ = &path;
        return;
    };

    let is_ascii = data.starts_with(b"*") || data.starts_with(b"#");
    let candidates = if is_ascii {
        parse_ascii_control(&data)
    } else {
        match parse_binary_control(&data) {
            Some(v) => {
                g.log(format!(
                    "   Converting legacy binary hcontrol ({} records) to ASCII.",
                    v.len()
                ));
                v
            }
            None => {
                g.log("SYSERR: hcontrol file is neither ASCII nor a known binary layout.".to_string());
                return;
            }
        }
    };

    for h in candidates {
        if g.houses.len() >= MAX_HOUSES {
            break;
        }
        if crate::players_glue::get_name_by_id(g, h.owner).is_none() {
            continue; // owner no longer exists
        }
        let Some(real_house) = g.real_room(h.vnum) else { continue };
        if find_house(g, h.vnum).is_some() {
            continue; // already a house
        }
        let Some(real_atrium) = g.real_room(h.atrium) else { continue };
        if h.exit_num < 0 || h.exit_num >= crate::fight::dir_count(g) as i32 {
            continue;
        }
        let to = g.world.rooms[real_house as usize].dir_option[h.exit_num as usize]
            .as_ref()
            .map_or(NOWHERE, |e| e.to_room);
        if to != real_atrium {
            continue; // exit no longer leads to the atrium
        }

        let vnum = h.vnum;
        g.houses.push(h);
        set_room_flag(g, real_house, flags::ROOM_HOUSE);
        set_room_flag(g, real_house, flags::ROOM_PRIVATE);
        set_room_flag(g, real_atrium, flags::ROOM_ATRIUM);
        house_load(g, vnum);
    }

    house_save_control(g);
}

// ------------------------------------------------------------ object files

/// House_load with the A4 rebuild: `Loc:` depths restore
/// containment instead of everything landing on the floor.
fn house_load(g: &mut Game, vnum: i32) -> bool {
    let Some(rnum) = g.real_room(vnum) else { return false };
    let path = house_file_path(g, vnum);
    let Ok(data) = std::fs::read(&path) else { return false };

    let mut r = Reader::new(&data);
    let loaded = objsave_parse_objects(g, &mut r);
    let mut cont_row: [Vec<ObjId>; MAX_BAG_ROWS] = Default::default();
    for rec in loaded {
        if g.try_obj(rec.obj).is_none() {
            continue;
        }
        handle_obj_room(g, rec.obj, rnum, rec.locate, &mut cont_row);
    }
    // Anything whose container vanished from the file lands on the floor.
    for row in cont_row.iter_mut() {
        for o in std::mem::take(row) {
            obj_to_room(g, o, rnum);
        }
    }
    true
}

/// handle_obj for a room rather than a character:
/// depth 0 is the floor, deeper rows queue up for their container.
fn handle_obj_room(
    g: &mut Game,
    oid: ObjId,
    rnum: RoomRnum,
    locate: i32,
    cont_row: &mut [Vec<ObjId>; MAX_BAG_ROWS],
) {
    let mut j = MAX_BAG_ROWS - 1;
    while j as i32 > -locate {
        for o in std::mem::take(&mut cont_row[j]) {
            obj_to_room(g, o, rnum);
        }
        j -= 1;
    }

    if j as i32 == -locate && !cont_row[j].is_empty() {
        if g.obj(oid).type_flag == flags::ITEM_CONTAINER {
            g.obj_mut(oid).contains.clear();
            for o in std::mem::take(&mut cont_row[j]) {
                obj_to_obj(g, o, oid);
            }
        } else {
            for o in std::mem::take(&mut cont_row[j]) {
                obj_to_room(g, o, rnum);
            }
        }
    }

    if locate < 0 && locate >= -(MAX_BAG_ROWS as i32) {
        cont_row[(-locate - 1) as usize].push(oid);
    } else {
        obj_to_room(g, oid, rnum);
    }
}

/// House_save + A4: contents are written immediately
/// before their container, carrying the `Loc:` depth, exactly as rent
/// files do.
fn house_save_list(g: &mut Game, list: &[ObjId], out: &mut Vec<u8>, location: i32) {
    for &oid in list.iter().rev() {
        if g.try_obj(oid).is_none() {
            continue;
        }
        let contents = g.obj(oid).contains.clone();
        house_save_list(g, &contents, out, location.min(0) - 1);
        objsave_save_obj_record(g, oid, out, location);
        // Only containers that track content weight have it to give
        // back, and only up to the first one that does not -- a weight that
        // never climbed past a non-tracking container is not in the ones
        // above it either. `house_restore_weight` walks the same unbroken
        // run, so the two stay exact inverses. Without the gate an unlimited
        // container was written lighter every save, and eventually negative.
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

fn house_restore_weight(g: &mut Game, oid: ObjId) {
    let contents = g.obj(oid).contains.clone();
    for c in contents {
        if g.try_obj(c).is_some() {
            house_restore_weight(g, c);
        }
    }
    // Contents are restored first, so a weight climbs one hop per level and
    // stops where the save stopped -- the same unbroken run of tracking
    // containers.
    let w = g.obj(oid).weight;
    if let Some(up) = g.obj(oid).in_obj {
        if crate::handler::weight_gate_open(g, up) {
            g.obj_mut(up).weight += w;
        }
    }
}

/// House_crashsave: write the room's contents and clear
/// the dirty bit.
pub fn house_crashsave(g: &mut Game, vnum: i32) {
    let Some(rnum) = g.real_room(vnum) else { return };
    let contents = g.rooms[rnum as usize].contents.clone();
    let mut out = Vec::new();
    house_save_list(g, &contents, &mut out, 0);
    out.extend_from_slice(b"$~\n");

    let path = house_file_path(g, vnum);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let written = std::fs::write(&path, &out);
    // The save walk took each object's weight back out of the containers
    // above it, so the restore has to run whether or not the file reached the
    // disk -- C restores immediately after the save and checks the result
    // afterwards. Returning early instead leaves every container in the house
    // permanently light by its contents.
    for oid in contents {
        if g.try_obj(oid).is_some() {
            house_restore_weight(g, oid);
        }
    }
    if let Err(e) = written {
        g.log(format!("SYSERR: Error saving house file: {}", e));
        return;
    }
    remove_room_flag(g, rnum, flags::ROOM_HOUSE_CRASH);
}

fn house_delete_file(g: &mut Game, vnum: i32) {
    let path = house_file_path(g, vnum);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            g.log(format!("SYSERR: Error deleting house file #{}. (2): {}", vnum, e));
        }
    }
}

/// House_save_all: the autosave tick, dirty houses only.
pub fn house_save_all(g: &mut Game) {
    let vnums: Vec<i32> = g.houses.iter().map(|h| h.vnum).collect();
    for vnum in vnums {
        let Some(rnum) = g.real_room(vnum) else { continue };
        if room_flagged(g, rnum, flags::ROOM_HOUSE_CRASH) {
            house_crashsave(g, vnum);
        }
    }
}

fn house_listrent(g: &mut Game, chid: CharId, vnum: i32) {
    let path = house_file_path(g, vnum);
    let Ok(data) = std::fs::read(&path) else {
        let msg = format!("No objects on file for house #{}.\r\n", vnum);
        send_to_char(g, chid, msg.as_bytes());
        return;
    };
    // House_get_filename yields "house/<vnum>.house", relative to lib/.
    let mut out = format!("filename: house/{}.house\r\n", vnum).into_bytes();
    let mut r = Reader::new(&data);
    let loaded = objsave_parse_objects(g, &mut r);
    for rec in &loaded {
        if g.try_obj(rec.obj).is_none() {
            continue;
        }
        let v = crate::dg::obj_vnum(g, rec.obj);
        let rent = g.obj(rec.obj).cost_per_day;
        let short = crate::handler::obj_short(g, rec.obj).to_vec();
        out.extend_from_slice(format!(" [{:5}] ({:5}au) ", v, rent).as_bytes());
        out.extend_from_slice(&short);
        out.extend_from_slice(b"\r\n");
    }
    for rec in loaded {
        if g.try_obj(rec.obj).is_some() {
            extract_obj(g, rec.obj);
        }
    }
    crate::act::informative::page_string(g, chid, &out);
}

// ------------------------------------------------------------- the commands

pub fn hcontrol_list_houses(g: &mut Game, chid: CharId, arg: &[u8]) {
    if !arg.is_empty() {
        let toshow = if arg[0] == b'.' {
            let room = g.ch(chid).in_room;
            g.world.rooms[room as usize].vnum as i32
        } else {
            atoi(arg)
        };
        if find_house(g, toshow).is_none() {
            let mut msg = b"Unknown house, \"".to_vec();
            msg.extend_from_slice(arg);
            msg.extend_from_slice(b"\".\r\n");
            send_to_char(g, chid, &msg);
            return;
        }
        house_listrent(g, chid, toshow);
        return;
    }

    if g.houses.is_empty() {
        send_to_char(g, chid, b"No houses have been defined.\r\n");
        return;
    }
    send_to_char(
        g,
        chid,
        b"Address  Atrium  Build Date       Guests  Owner        Last Paymt\r\n\
          -------  ------  ---------------  ------  ------------ ---------------\r\n",
    );

    for i in 0..g.houses.len() {
        let h = g.houses[i].clone();
        let Some(owner) = crate::players_glue::get_name_by_id(g, h.owner) else { continue };
        let built_on = if h.built_on != 0 {
            crate::act::wizard::strftime_date(h.built_on, g.tz_offset_secs)
        } else {
            "Unknown".to_string()
        };
        let last_pay = if h.last_payment != 0 {
            crate::act::wizard::strftime_date(h.last_payment, g.tz_offset_secs)
        } else {
            "None".to_string()
        };
        let mut own_name = owner.clone();
        if let Some(c) = own_name.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        let line = format!(
            "{:7} {:7}  {:<15}    {:2}    {:<12} {}\r\n",
            h.vnum,
            h.atrium,
            built_on,
            h.guests.len(),
            String::from_utf8_lossy(&own_name),
            last_pay
        );
        send_to_char(g, chid, line.as_bytes());
        house_list_guests(g, chid, i, true);
    }
}

fn hcontrol_build_house(g: &mut Game, chid: CharId, arg: &[u8]) {
    if g.houses.len() >= MAX_HOUSES {
        send_to_char(g, chid, b"Max houses already defined.\r\n");
        return;
    }

    let (arg1, rest) = one_argument(arg);
    if arg1.is_empty() {
        send_to_char(g, chid, HCONTROL_FORMAT);
        return;
    }
    let virt_house = atoi(&arg1);
    let Some(real_house) = g.real_room(virt_house) else {
        send_to_char(g, chid, b"No such room exists.\r\n");
        return;
    };
    if find_house(g, virt_house).is_some() {
        send_to_char(g, chid, b"House already exists.\r\n");
        return;
    }

    let (arg1, rest) = one_argument(rest);
    if arg1.is_empty() {
        send_to_char(g, chid, HCONTROL_FORMAT);
        return;
    }
    let Some(exit_num) = crate::act::informative::search_block(&arg1, &mud_data::tables::DIRS)
    else {
        let mut msg = b"'".to_vec();
        msg.extend_from_slice(&arg1);
        msg.extend_from_slice(b"' is not a valid direction.\r\n");
        send_to_char(g, chid, &msg);
        return;
    };
    let to = g.world.rooms[real_house as usize].dir_option[exit_num]
        .as_ref()
        .map_or(NOWHERE, |e| e.to_room);
    if to == NOWHERE {
        let msg = format!(
            "There is no exit {} from room {}.\r\n",
            mud_data::tables::DIRS[exit_num],
            virt_house
        );
        send_to_char(g, chid, msg.as_bytes());
        return;
    }

    let real_atrium = to;
    let virt_atrium = g.world.rooms[real_atrium as usize].vnum as i32;
    let back = g.world.rooms[real_atrium as usize].dir_option[rev_dir(exit_num)]
        .as_ref()
        .map_or(NOWHERE, |e| e.to_room);
    if back != real_house {
        send_to_char(g, chid, b"A house's exit must be a two-way door.\r\n");
        return;
    }

    let (arg1, _) = one_argument(rest);
    if arg1.is_empty() {
        send_to_char(g, chid, HCONTROL_FORMAT);
        return;
    }
    let Some(owner) = crate::players_glue::get_id_by_name(g, &arg1) else {
        let mut msg = b"Unknown player '".to_vec();
        msg.extend_from_slice(&arg1);
        msg.extend_from_slice(b"'.\r\n");
        send_to_char(g, chid, &msg);
        return;
    };

    g.houses.push(HouseControl {
        vnum: virt_house,
        atrium: virt_atrium,
        exit_num: exit_num as i32,
        built_on: g.now,
        mode: HOUSE_PRIVATE,
        owner,
        guests: Vec::new(),
        last_payment: 0,
    });

    set_room_flag(g, real_house, flags::ROOM_HOUSE);
    set_room_flag(g, real_house, flags::ROOM_PRIVATE);
    set_room_flag(g, real_atrium, flags::ROOM_ATRIUM);
    house_crashsave(g, virt_house);

    send_to_char(g, chid, b"House built.  Mazel tov!\r\n");
    house_save_control(g);
}

fn hcontrol_destroy_house(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        send_to_char(g, chid, HCONTROL_FORMAT);
        return;
    }
    let Some(i) = find_house(g, atoi(arg)) else {
        send_to_char(g, chid, b"Unknown house.\r\n");
        return;
    };
    let h = g.houses[i].clone();
    match g.real_room(h.atrium) {
        None => g.log(format!("SYSERR: House {} had invalid atrium {}!", atoi(arg), h.atrium)),
        Some(ra) => remove_room_flag(g, ra, flags::ROOM_ATRIUM),
    }
    match g.real_room(h.vnum) {
        None => g.log(format!("SYSERR: House {} had invalid vnum {}!", atoi(arg), h.vnum)),
        Some(rh) => {
            remove_room_flag(g, rh, flags::ROOM_HOUSE);
            remove_room_flag(g, rh, flags::ROOM_PRIVATE);
            remove_room_flag(g, rh, flags::ROOM_HOUSE_CRASH);
        }
    }
    house_delete_file(g, h.vnum);
    g.houses.remove(i);

    send_to_char(g, chid, b"House deleted.\r\n");
    house_save_control(g);

    // A destroyed house may have shared its atrium with another one.
    let atria: Vec<i32> = g.houses.iter().map(|h| h.atrium).collect();
    for a in atria {
        if let Some(ra) = g.real_room(a) {
            set_room_flag(g, ra, flags::ROOM_ATRIUM);
        }
    }
}

fn hcontrol_pay_house(g: &mut Game, chid: CharId, arg: &[u8]) {
    if arg.is_empty() {
        send_to_char(g, chid, HCONTROL_FORMAT);
        return;
    }
    let Some(i) = find_house(g, atoi(arg)) else {
        send_to_char(g, chid, b"Unknown house.\r\n");
        return;
    };
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    g.mudlog(
        MudlogKind::Nrm,
        lvl,
        true,
        &format!(
            "Payment for house {} collected by {}.",
            String::from_utf8_lossy(arg),
            name
        ),
    );
    g.houses[i].last_payment = g.now;
    house_save_control(g);
    send_to_char(g, chid, b"Payment recorded.\r\n");
}

/// do_hcontrol. `asciiconvert` is gone: the boot loader
/// imports legacy binary control files automatically (D3).
pub fn do_hcontrol(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg1, arg2) = half_chop(argument);
    if is_abbrev(&arg1, b"build") {
        hcontrol_build_house(g, chid, &arg2);
    } else if is_abbrev(&arg1, b"destroy") {
        hcontrol_destroy_house(g, chid, &arg2);
    } else if is_abbrev(&arg1, b"pay") {
        hcontrol_pay_house(g, chid, &arg2);
    } else if is_abbrev(&arg1, b"show") {
        hcontrol_list_houses(g, chid, &arg2);
    } else {
        send_to_char(g, chid, HCONTROL_FORMAT);
    }
}

/// do_house — the owner's guest list.
pub fn do_house(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (arg, _) = one_argument(argument);
    let room = g.ch(chid).in_room;
    if !room_flagged(g, room, flags::ROOM_HOUSE) {
        send_to_char(g, chid, b"You must be in your house to set guests.\r\n");
        return;
    }
    let vnum = g.world.rooms[room as usize].vnum as i32;
    let Some(i) = find_house(g, vnum) else {
        send_to_char(g, chid, b"Um.. this house seems to be screwed up.\r\n");
        return;
    };
    if g.ch(chid).idnum != g.houses[i].owner {
        send_to_char(g, chid, b"Only the primary owner can set guests.\r\n");
        return;
    }
    if arg.is_empty() {
        house_list_guests(g, chid, i, false);
        return;
    }
    let Some(id) = crate::players_glue::get_id_by_name(g, &arg) else {
        send_to_char(g, chid, b"No such player.\r\n");
        return;
    };
    if id == g.ch(chid).idnum {
        send_to_char(g, chid, b"It's your house!\r\n");
        return;
    }
    if let Some(pos) = g.houses[i].guests.iter().position(|&x| x == id) {
        g.houses[i].guests.remove(pos);
        house_save_control(g);
        send_to_char(g, chid, b"Guest deleted.\r\n");
        return;
    }
    if g.houses[i].guests.len() == MAX_GUESTS {
        send_to_char(g, chid, b"You have too many guests.\r\n");
        return;
    }
    g.houses[i].guests.push(id);
    house_save_control(g);
    send_to_char(g, chid, b"Guest added.\r\n");
}

/// House_can_enter: GRGOD+ walks in anywhere.
pub fn house_can_enter(g: &Game, chid: CharId, house: i32) -> bool {
    if g.ch(chid).level >= LVL_GRGOD {
        return true;
    }
    let Some(i) = find_house(g, house) else { return true };
    let h = &g.houses[i];
    if h.mode == HOUSE_PRIVATE {
        let id = g.ch(chid).idnum;
        if id == h.owner || h.guests.contains(&id) {
            return true;
        }
    }
    false
}

/// House_list_guests: guests whose players are gone are
/// skipped, and a list that empties out prints "all dead".
pub fn house_list_guests(g: &mut Game, chid: CharId, i: usize, quiet: bool) {
    let guests = g.houses[i].guests.clone();
    if guests.is_empty() {
        if !quiet {
            send_to_char(g, chid, b"  Guests: None\r\n");
        }
        return;
    }
    send_to_char(g, chid, b"  Guests: ");
    let mut num_printed = 0;
    for id in guests {
        let Some(name) = crate::players_glue::get_name_by_id(g, id) else { continue };
        num_printed += 1;
        let mut out = name.clone();
        if let Some(c) = out.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        out.push(b' ');
        send_to_char(g, chid, &out);
    }
    if num_printed == 0 {
        send_to_char(g, chid, b"all dead");
    }
    send_to_char(g, chid, b"\r\n");
}
