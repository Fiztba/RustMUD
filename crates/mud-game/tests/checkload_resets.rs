use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
use mud_net::descriptor::Descriptor;
use std::path::{Path, PathBuf};

struct Fixture { game: Game, root: PathBuf }
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else { std::fs::copy(entry.path(), dest).unwrap(); }
    }
}

fn fixture(label: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("rustmud-checkload-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: 10,
        passwd: mud_data::crypt::crypt(b"secret", name).unwrap().to_vec(),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    })
}

fn descriptor(g: &mut Game, ch: mud_data::ids::CharId, state: ConState) -> usize {
    let mut d = Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = state;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    di
}

use mud_world::model::ZoneCommand;
use mud_game::act::wizard::do_checkloadstatus;
fn command(command: u8, arg1: i32, arg2: i32, arg3: i32) -> ZoneCommand {
    ZoneCommand { command, arg1, arg2, arg3, ..Default::default() }
}
#[test]
fn checkload_handles_objects_loaded_without_a_room() {
    let mut f = fixture("nowhere");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    let di = descriptor(g, ch, ConState::Playing);
    for z in &mut g.world.zones { z.cmds.clear(); }
    g.world.zones[0].cmds = vec![command(b'O', 0, 1, NOWHERE as i32)];
    do_checkloadstatus(g, ch, format!("o {}", g.world.obj_protos[0].vnum).as_bytes(), 0, 0);
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(output.contains("Nowhere"), "{output}");
    g.descriptors.get_mut(di).unwrap().output.clear();
    do_checkloadstatus(g, ch, format!("t {}", g.world.triggers[0].vnum).as_bytes(), 0, 0);
}
#[test]
fn checkload_uses_room_trigger_destination_and_ignores_remove_commands() {
    let mut f = fixture("triggers");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    let di = descriptor(g, ch, ConState::Playing);
    for z in &mut g.world.zones { z.cmds.clear(); }
    g.world.zones[0].cmds = vec![command(b'T', mud_game::dg::WLD_TRIGGER, 0, 2)];
    do_checkloadstatus(g, ch, format!("t {}", g.world.triggers[0].vnum).as_bytes(), 0, 0);
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(output.contains(&format!("room [{:5}]", g.world.rooms[2].vnum)), "{output}");
    g.descriptors.get_mut(di).unwrap().output.clear();
    g.world.zones[0].cmds = vec![command(b'R', mud_game::dg::WLD_TRIGGER, 0, 0)];
    do_checkloadstatus(g, ch, format!("t {}", g.world.triggers[0].vnum).as_bytes(), 0, 0);
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(!output.contains("(zedit)"), "{output}");
}
