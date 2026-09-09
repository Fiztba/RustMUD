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
    let root = std::env::temp_dir().join(format!("rustmud-login-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: LVL_IMPL, pfilepos: 0,
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
fn failed_password_counts_do_not_overflow() {
    let mut fixture = fixture("failure-overflow");
    let g = &mut fixture.game;
    let ch = player(g, b"Tester", 12345);
    let di = descriptor(g, ch, ConState::Password);
    g.ch_mut(ch).ps_mut().bad_pws = 255;
    mud_game::login::nanny(g, di, b"wrong");
    assert_eq!(g.ch(ch).ps().bad_pws, 255);
    let (saved, _) = mud_world::players::load_char(&g.lib_dir, b"Tester").unwrap();
    assert_eq!(saved.bad_pws, 255);
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Password);
    g.config.max_bad_pws = 257;
    g.descriptors.get_mut(di).unwrap().bad_pws = 255;
    mud_game::login::nanny(g, di, b"wrong");
    assert_eq!(g.descriptors.get(di).unwrap().bad_pws, 256);
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Password);
    mud_game::login::nanny(g, di, b"wrong");
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Close);
}
