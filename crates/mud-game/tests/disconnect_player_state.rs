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
    let root = std::env::temp_dir().join(format!("rustmud-disconnect-state-{}-{label}", std::process::id()));
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
fn disconnect_checks_the_original_player_while_switched_into_an_npc() {
    let mut f = fixture("switched"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_GOD;
    let senior = player(g, b"Senior", 12346); g.ch_mut(senior).level = LVL_IMPL;
    let body = mud_game::db::read_mobile(g, 0).unwrap(); g.ch_mut(body).level = 1;
    for ch in [admin, senior, body] { mud_game::handler::char_to_room(g, ch, 0); }
    g.rooms[0].light = 1; descriptor(g, admin, ConState::Playing);
    let di = descriptor(g, body, ConState::Playing);
    { let d = g.descriptors.get_mut(di).unwrap(); d.original = Some(senior); d.desc_num = 42; }
    mud_game::act::wizard::do_dc(g, admin, b"42", 0, 0);
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
}
#[test]
fn disconnecting_an_editor_keeps_its_player_in_the_world() {
    let mut f = fixture("editor"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    descriptor(g, admin, ConState::Playing); mud_game::handler::char_to_room(g, admin, 0);
    for state in [ConState::Playing, ConState::Oedit, ConState::Msgedit] {
        let target = player(g, b"Target", 12346); mud_game::handler::char_to_room(g, target, 0); g.character_list.push_front(target);
        let di = descriptor(g, target, state); g.descriptors.get_mut(di).unwrap().desc_num = 42;
        mud_game::act::wizard::do_dc(g, admin, b"42", 0, 0);
        assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Disconnect, "state={state:?}");
        mud_game::run::close_socket(g, di);
        assert!(g.try_ch(target).is_some()); assert!(g.rooms[0].people.contains(&target)); assert_eq!(g.ch(target).desc, None);
    }
}
