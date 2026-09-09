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
    let root = std::env::temp_dir().join(format!("rustmud-ibtraces-{}-{label}", std::process::id()));
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

fn input(g: &mut Game, di: usize, text: &[u8]) {
    g.descriptors.get_mut(di).unwrap().input.push_back((text.to_vec(), false));
    mud_game::run::game_pulse(g);
}

#[test]
fn simultaneous_reports_keep_their_authors_and_bodies() {
    let mut f = fixture("concurrent");
    let g = &mut f.game;
    g.ibt.lists[0].clear();
    let a = player(g, b"Alice", 12345);
    let b = player(g, b"Bobby", 12346);
    let da = descriptor(g, a, ConState::Playing);
    let db = descriptor(g, b, ConState::Playing);
    for ch in [a, b] { mud_game::handler::char_to_room(g, ch, 0); }
    let cmd = mud_game::interpreter::find_command(g, b"bug").unwrap();
    mud_game::ibt::do_ibt(g, a, b"submit First report", cmd, mud_game::interpreter::SCMD_BUG);
    mud_game::ibt::do_ibt(g, b, b"submit Second report", cmd, mud_game::interpreter::SCMD_BUG);
    input(g, da, b"Alice body");
    input(g, da, b"/s");
    input(g, db, b"Bobby body");
    input(g, db, b"/s");
    assert_eq!(g.ibt.lists[0].len(), 2);
    assert_eq!(g.ibt.lists[0][0].name, b"Alice");
    assert_eq!(g.ibt.lists[0][0].body, b"Alice body\r\n");
    assert_eq!(g.ibt.lists[0][1].name, b"Bobby");
    assert_eq!(g.ibt.lists[0][1].body, b"Bobby body\r\n");
}
