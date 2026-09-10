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
    let root = std::env::temp_dir().join(format!("rustmud-config-exclusion-{}-{label}", std::process::id()));
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

fn editor_case(save: bool) {
    let mut f = fixture(if save { "save" } else { "cancel" }); let g = &mut f.game;
    let original_ok = g.config.ok.clone();
    let first = player(g, b"First", 12345); let second = player(g, b"Second", 12346);
    g.ch_mut(first).level = LVL_IMPL; g.ch_mut(second).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, first, 0); mud_game::handler::char_to_room(g, second, 0);
    let a = descriptor(g, first, ConState::Playing); let b = descriptor(g, second, ConState::Playing);
    mud_game::olc::cedit::do_oasis_cedit(g, first, b"", 0, 0);
    mud_game::olc::cedit::do_oasis_cedit(g, second, b"", 0, 0);
    assert_eq!(g.descriptors.get(b).unwrap().state, ConState::Playing);
    assert!(!g.olc.contains_key(&b));
    let mut olc = g.olc.remove(&a).unwrap();
    olc.config.as_mut().unwrap().ok = b"Changed!\r\n".to_vec();
    olc.config.as_mut().unwrap().auto_save = false;
    olc.mode = mud_game::olc::cedit::CEDIT_CONFIRM_SAVESTRING;
    assert!(mud_game::olc::cedit::cedit_parse(g, a, olc, if save { b"y" } else { b"n" }).is_none());
    mud_game::olc::cedit::do_oasis_cedit(g, second, b"", 0, 0);
    assert_eq!(g.descriptors.get(b).unwrap().state, ConState::Cedit);
    assert_eq!(g.olc[&b].config.as_ref().unwrap().ok, if save { b"Changed!\r\n".to_vec() } else { original_ok });
}

#[test] fn saving_releases_config_editor_with_latest_settings() { editor_case(true); }
#[test] fn cancelling_releases_config_editor_without_changes() { editor_case(false); }
