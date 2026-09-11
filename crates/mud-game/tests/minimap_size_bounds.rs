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
    let root = std::env::temp_dir().join(format!("rustmud-minimap-size-{}-{label}", std::process::id()));
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

fn link(g: &mut Game, from: RoomRnum, dir: usize, to: RoomRnum) {
    g.world.rooms[from as usize].dir_option[dir] = Some(Box::new(mud_world::model::Exit {
        general_description: None, keyword: None, exit_info: 0, key: NOTHING,
        to_room_vnum: to as i32, to_room: to,
    }));
}

/// Rooms 1..=14 form a 13-step chain north of room 1, and rooms 14/15 are an
/// east/west pair at its end. With a minimap size of 13 the pair lies off the
/// 51x51 canvas, where put() is a no-op and at() always reads SECT_EMPTY.
fn setup(label: &str) -> (Fixture, mud_data::ids::CharId, usize) {
    let mut f = fixture(label);
    let g = &mut f.game;
    assert!(g.world.rooms.len() > 15, "the mini world needs rooms 1..=15");
    for z in &mut g.world.zones { z.zone_flags = [0; 4]; }
    for rnum in 1..=15usize {
        g.world.rooms[rnum].room_flags = [0; 4];
        g.world.rooms[rnum].sector_type = mud_data::flags::SECT_CITY;
        g.world.rooms[rnum].dir_option = Default::default();
    }
    for rnum in 1..14 {
        link(g, rnum, NORTH, rnum + 1);
        link(g, rnum + 1, SOUTH, rnum);
    }
    link(g, 14, EAST, 15);
    link(g, 15, WEST, 14);
    let ch = player(g, b"Mapper", 12345);
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 1);
    g.ch_mut(ch).ps_mut().pref.set(mud_data::flags::PRF_AUTOMAP);
    g.config.map_option = mud_game::asciimap::MAP_ON;
    (f, ch, di)
}

fn minimap(g: &mut Game, ch: mud_data::ids::CharId, di: usize, size: i32) -> Vec<u8> {
    g.config.default_minimap_size = size;
    g.descriptors.get_mut(di).unwrap().output.clear();
    mud_game::asciimap::str_and_map(g, ch, b"A long chain of city streets.", 1);
    std::mem::take(&mut g.descriptors.get_mut(di).unwrap().output)
}

fn line_count(out: &[u8]) -> usize {
    out.windows(2).filter(|w| w == b"\r\n").count()
}

#[test]
fn oversized_minimap_size_is_clamped_instead_of_walking_off_the_canvas() {
    let (mut f, ch, di) = setup("oversized");
    let g = &mut f.game;
    let at_max = minimap(g, ch, di, 12);
    let oversized = minimap(g, ch, di, 13);
    assert_eq!(line_count(&oversized), 12 * 2 + 1);
    assert_eq!(oversized, at_max);
}

#[test]
fn maximum_minimap_size_renders_full_height() {
    let (mut f, ch, di) = setup("maximum");
    let g = &mut f.game;
    let out = minimap(g, ch, di, 12);
    assert_eq!(line_count(&out), 12 * 2 + 1);
}
