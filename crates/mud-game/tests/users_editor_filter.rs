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
    let root = std::env::temp_dir().join(format!("rustmud-users-editor-{}-{label}", std::process::id()));
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
fn users_filters_treat_editors_as_playing_connections() {
    let mut f = fixture("filters"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    let target = player(g, b"Target", 12346);
    for ch in [admin, target] { mud_game::handler::char_to_room(g, ch, 0); }
    g.rooms[0].light = 1;
    let di = descriptor(g, admin, ConState::Playing); let td = descriptor(g, target, ConState::Oedit);
    for state in [ConState::Playing, ConState::Oedit, ConState::Redit, ConState::Zedit, ConState::Medit, ConState::Sedit, ConState::Tedit, ConState::Cedit, ConState::Aedit, ConState::Trigedit, ConState::Hedit, ConState::Qedit, ConState::Prefedit, ConState::Ibtedit, ConState::Msgedit] {
        g.descriptors.get_mut(td).unwrap().state = state;
    for (argument, expected) in [(b"-p".as_slice(), true), (b"-n Target".as_slice(), true), (b"-d".as_slice(), false), (b"-p -d".as_slice(), false), (b"-h localhost".as_slice(), true), (b"-n Missing".as_slice(), false)] {
        g.descriptors.get_mut(di).unwrap().output.clear();
        mud_game::act::informative::do_users(g, admin, argument, 0, 0);
        let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert_eq!(output.contains("Target"), expected, "argument={argument:?}");
    }
    }
    g.descriptors.get_mut(td).unwrap().state = ConState::Password;
    g.descriptors.get_mut(di).unwrap().output.clear();
    mud_game::act::informative::do_users(g, admin, b"-p", 0, 0);
    assert!(!String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains("Target"));
}
