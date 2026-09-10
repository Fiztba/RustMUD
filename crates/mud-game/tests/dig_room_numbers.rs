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
    let root = std::env::temp_dir().join(format!("rustmud-dig-numbers-{}-{label}", std::process::id()));
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
fn dig_rejects_wrapping_room_numbers_without_changing_exits() {
    let mut f = fixture("numbers"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); g.ch_mut(builder).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, builder, 2); descriptor(g, builder, ConState::Playing);
    for z in &mut g.world.zones { z.zone_flags = [0; 4]; }
    for value in ["65535", "131071", "-65537"] {
        g.world.rooms[2].dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
            general_description: None, keyword: None, exit_info: 0, key: NOTHING, to_room_vnum: 1, to_room: 1,
        }));
        mud_game::olc::copy::do_dig(g, builder, format!("north {value}").as_bytes(), 0, 0);
        assert_eq!(g.world.rooms[2].dir_option[NORTH].as_ref().map(|e| e.to_room), Some(1), "value={value}");
    }
    let target = g.world.rooms[1].vnum;
    for value in [format!("{}", target as u32 + 65536), format!("{target}junk")] {
        g.world.rooms[2].dir_option[NORTH] = None;
        mud_game::olc::copy::do_dig(g, builder, format!("north {value}").as_bytes(), 0, 0);
        assert!(g.world.rooms[2].dir_option[NORTH].is_none(), "value={value}");
    }
}
