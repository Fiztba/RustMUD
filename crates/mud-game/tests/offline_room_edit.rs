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
    let root = std::env::temp_dir().join(format!("rustmud-offline-room-{}-{label}", std::process::id()));
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
fn offline_room_edits_do_not_leave_a_freed_player_in_the_room() {
    let mut f = fixture("set"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, admin, 0); let di = descriptor(g, admin, ConState::Playing);
    let target = player(g, b"Target", 12346);
    let slot = mud_game::players_glue::create_entry(g, b"Target");
    g.ch_mut(target).pfilepos = slot as i32;
    mud_game::players_glue::save_char(g, target);
    mud_game::players_glue::free_offline_char(g, target);
    let original_people = g.rooms[0].people.clone();
    let vnum = g.world.rooms[0].vnum;
    mud_game::act::wizset::do_set(g, admin, format!("file Target room {vnum}").as_bytes(), 0, 0);
    assert_eq!(g.rooms[0].people, original_people);
    assert!(String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains("loadroom"));
    mud_game::act::wizset::do_set(g, admin, b"file Target gold 42", 0, 0);
    let loaded = mud_game::players_glue::load_char_offline(g, b"Target").unwrap();
    assert_eq!(g.ch(loaded).points.gold, 42);
    assert_eq!(g.ch(loaded).in_room, NOWHERE);
    mud_game::players_glue::free_offline_char(g, loaded);
    assert_eq!(g.rooms[0].people, original_people);
}

