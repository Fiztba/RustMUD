//! World boot orchestration: index-driven loading in dependency order
//! (zon → trg → wld → renum_world → mob → obj → renum_zone_table → shp →
//! qst), the renumbering passes, and boot counts.

use std::fs;
use std::path::{Path, PathBuf};

use mud_data::types::NOWHERE;

use crate::lex::Reader;
use crate::model::World;
use crate::parse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    Zon,
    Trg,
    Wld,
    Mob,
    Obj,
    Shp,
    Qst,
}

impl BootMode {
    fn dir(self) -> &'static str {
        match self {
            BootMode::Zon => "zon",
            BootMode::Trg => "trg",
            BootMode::Wld => "wld",
            BootMode::Mob => "mob",
            BootMode::Obj => "obj",
            BootMode::Shp => "shp",
            BootMode::Qst => "qst",
        }
    }
}

/// Read a world index file: entries until a line starting with '$'.
/// get_line semantics apply, so blank and '*' lines are skipped.
fn read_index(dir: &Path) -> Result<Vec<String>, String> {
    let path = dir.join("index");
    let data = fs::read(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let mut r = Reader::new(&data);
    let mut out = Vec::new();
    while let Some(line) = r.get_line() {
        if line.starts_with(b"$") {
            break;
        }
        out.push(String::from_utf8_lossy(&line).into_owned());
    }
    Ok(out)
}

fn boot_type(world: &mut World, world_dir: &Path, mode: BootMode) -> Result<(), String> {
    let dir = world_dir.join(mode.dir());
    for name in read_index(&dir)? {
        let path: PathBuf = dir.join(&name);
        // A file listed in the index that cannot be opened is fatal.
        let data =
            fs::read(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
        let res = match mode {
            BootMode::Zon => parse::zon::parse_file(world, &data, &name),
            BootMode::Trg => parse::trg::parse_file(world, &data, &name),
            BootMode::Wld => parse::wld::parse_file(world, &data, &name),
            BootMode::Mob => parse::mob::parse_file(world, &data, &name),
            BootMode::Obj => parse::obj::parse_file(world, &data, &name),
            BootMode::Shp => parse::shp::parse_file(world, &data, &name),
            BootMode::Qst => parse::qst::parse_file(world, &data, &name),
        };
        res.map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Resolve every exit's to_room vnum into an rnum. Unresolvable targets
/// become NOWHERE.
fn renum_world(world: &mut World) {
    let map = world.room_map.clone();
    for room in &mut world.rooms {
        for door in room.dir_option.iter_mut().flatten() {
            if door.to_room != NOWHERE {
                continue; // already NOWHERE from parse (0 / -1)
            }
            if door.to_room_vnum > 0 && door.to_room_vnum < 65535 {
                door.to_room = map
                    .get(&(door.to_room_vnum as u16))
                    .copied()
                    .unwrap_or(NOWHERE);
            }
        }
    }
}

/// Vnum→rnum for zone command args, disabling commands with unresolvable
/// references.
fn renum_zone_table(world: &mut World) -> Vec<String> {
    let mut errors = Vec::new();
    let room_map = world.room_map.clone();
    let mob_map = world.mob_map.clone();
    let obj_map = world.obj_map.clone();
    let trig_map = world.trig_map.clone();
    let real =
        |map: &std::collections::HashMap<u16, u16>, v: i32| -> i32 {
            if (0..65535).contains(&v) {
                map.get(&(v as u16)).map(|&r| r as i32).unwrap_or(NOWHERE as i32)
            } else {
                NOWHERE as i32
            }
        };

    for zone in &mut world.zones {
        for (cmd_no, cmd) in zone.cmds.iter_mut().enumerate() {
            // a=b=c start at 0, not NOWHERE, so unchecked slots never trip.
            let (mut a, mut b, mut c) = (0i32, 0i32, 0i32);
            let (olda, oldb, oldc) = (cmd.arg1, cmd.arg2, cmd.arg3);
            match cmd.command {
                b'M' => {
                    cmd.arg1 = real(&mob_map, cmd.arg1);
                    a = cmd.arg1;
                    cmd.arg3 = real(&room_map, cmd.arg3);
                    c = cmd.arg3;
                }
                b'O' => {
                    cmd.arg1 = real(&obj_map, cmd.arg1);
                    a = cmd.arg1;
                    if cmd.arg3 != NOWHERE as i32 {
                        cmd.arg3 = real(&room_map, cmd.arg3);
                        c = cmd.arg3;
                    }
                }
                b'G' | b'E' => {
                    cmd.arg1 = real(&obj_map, cmd.arg1);
                    a = cmd.arg1;
                }
                b'P' => {
                    cmd.arg1 = real(&obj_map, cmd.arg1);
                    a = cmd.arg1;
                    cmd.arg3 = real(&obj_map, cmd.arg3);
                    c = cmd.arg3;
                }
                b'D' => {
                    cmd.arg1 = real(&room_map, cmd.arg1);
                    a = cmd.arg1;
                }
                b'R' => {
                    cmd.arg1 = real(&room_map, cmd.arg1);
                    a = cmd.arg1;
                    cmd.arg2 = real(&obj_map, cmd.arg2);
                    b = cmd.arg2;
                }
                b'T' => {
                    cmd.arg2 = real(&trig_map, cmd.arg2);
                    b = cmd.arg2;
                    cmd.arg3 = real(&room_map, cmd.arg3);
                    c = cmd.arg3;
                }
                b'V' => {
                    cmd.arg3 = real(&room_map, cmd.arg3);
                    b = cmd.arg3;
                }
                _ => {}
            }
            let nw = NOWHERE as i32;
            if a == nw || b == nw || c == nw {
                let bad = if a == nw {
                    olda
                } else if b == nw {
                    oldb
                } else {
                    oldc
                };
                errors.push(format!(
                    "zone #{}, cmd {}: Invalid vnum {}, cmd disabled",
                    zone.number, cmd_no, bad
                ));
                cmd.command = b'*';
            }
        }
    }
    errors
}

#[derive(Debug)]
pub struct BootReport {
    pub world: World,
    pub zone_errors: Vec<String>,
    /// SYSERR lines the parsers produced while correcting a malformed
    /// record, in file order. These only ever appear for a world file that is
    /// already wrong; the correction itself is what matters.
    pub load_warnings: Vec<String>,
}

/// The boot_world order.
pub fn boot_world(lib_dir: &Path) -> Result<BootReport, String> {
    let world_dir = lib_dir.join("world");
    let mut world = World::default();
    boot_type(&mut world, &world_dir, BootMode::Zon)?;
    boot_type(&mut world, &world_dir, BootMode::Trg)?;
    boot_type(&mut world, &world_dir, BootMode::Wld)?;
    renum_world(&mut world);
    boot_type(&mut world, &world_dir, BootMode::Mob)?;
    boot_type(&mut world, &world_dir, BootMode::Obj)?;
    let zone_errors = renum_zone_table(&mut world);
    boot_type(&mut world, &world_dir, BootMode::Shp)?;
    boot_type(&mut world, &world_dir, BootMode::Qst)?;
    let load_warnings = std::mem::take(&mut world.load_warnings);
    Ok(BootReport { world, zone_errors, load_warnings })
}

/// The COUNTS line.
pub fn counts_line(world: &World) -> String {
    format!(
        "COUNTS rooms={} zones={} mobs={} objs={} trigs={} shops={} quests={}",
        world.rooms.len(),
        world.zones.len(),
        world.mob_protos.len(),
        world.obj_protos.len(),
        world.triggers.len(),
        world.shops.len(),
        world.quests.len()
    )
}
