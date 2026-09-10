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
    let root = std::env::temp_dir().join(format!("rustmud-world-lists-{}-{label}", std::process::id()));
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

#[test]
fn lists_include_the_only_entry_in_each_world_table() {
    let mut f = fixture("single"); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0);
    let di = descriptor(g, actor, ConState::Playing);
    g.world.rooms.truncate(1); g.world.rooms[0].dir_option = Default::default(); g.world.rooms[0].name = Some(b"single room".to_vec());
    g.world.mob_protos.truncate(1); g.world.mob_protos[0].short_descr = Some(b"single mobile".to_vec());
    g.world.obj_protos.truncate(1); g.world.obj_protos[0].short_description = Some(b"single object".to_vec());
    g.world.zones.truncate(1); g.world.zones[0].name = Some(b"single zone".to_vec());
    for (command, expected) in [("rlist 0 65534", "single room"), ("mlist 0 65534", "single mobile"),
        ("olist 0 65534", "single object"), ("zlist 0 65534", "single zone")] {
        g.descriptors.get_mut(di).unwrap().output.clear();
        mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
        let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(output.contains(expected), "{command}: {output}");
    }
}
#[test]
fn list_help_and_name_search_work_without_zone_zero() {
    let mut f = fixture("named"); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0);
    let di = descriptor(g, actor, ConState::Playing);
    for zone in &mut g.world.zones { if zone.number == 0 { zone.number = 60000; } }
    g.world.obj_protos[0].name = Some(b"uniquelisting".to_vec());
    g.world.obj_protos[0].short_description = Some(b"unique object result".to_vec());
    for (command, expected) in [("mlist help", "Usage:"), ("olist help", "Usage:"), ("olist uniquelisting", "unique object result")] {
        g.descriptors.get_mut(di).unwrap().output.clear();
        mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
        let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(output.contains(expected), "{command}: {output}");
    }
}
