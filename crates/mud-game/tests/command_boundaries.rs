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
    let root = std::env::temp_dir().join(format!("rustmud-commandbounds-{}-{label}", std::process::id()));
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

fn setup(label: &str) -> (Fixture, mud_data::ids::CharId, usize) {
    let mut f = fixture(label);
    let g = &mut f.game;
    let ch = player(g, b"Tester", 12345);
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    g.world.rooms[0].room_flags = [0; 4];
    g.world.rooms[0].sector_type = mud_data::flags::SECT_INSIDE;
    (f, ch, di)
}

#[test]
fn minimum_map_distance_uses_the_maximum_rectangle() {
    let (mut f, ch, di) = setup("map");
    let g = &mut f.game;
    g.config.map_option = mud_game::asciimap::MAP_ON;
    mud_game::asciimap::do_map(g, ch, b"-12", 0, 0);
    let expected = std::mem::take(&mut g.descriptors.get_mut(di).unwrap().output);
    mud_game::asciimap::do_map(g, ch, b"-2147483648", 0, 0);
    assert_eq!(g.descriptors.get(di).unwrap().output, expected);
}

#[test]
fn maximum_level_range_is_clamped_before_incrementing() {
    let (mut f, ch, di) = setup("levels");
    let g = &mut f.game;
    mud_game::act::informative::do_levels(g, ch, b"1-100", 0, 0);
    let expected = std::mem::take(&mut g.descriptors.get_mut(di).unwrap().output);
    mud_game::act::informative::do_levels(g, ch, b"1-2147483647", 0, 0);
    assert_eq!(g.descriptors.get(di).unwrap().output, expected);
}
