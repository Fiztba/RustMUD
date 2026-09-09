use mud_data::{flags, types::*};
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
    let root = std::env::temp_dir().join(format!("rustmud-boardslots-{}-{label}", std::process::id()));
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
fn empty_headlines_do_not_exhaust_board_slots() {
    let mut f = fixture("empty");
    let g = &mut f.game;
    let ch = player(g, b"Writer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let rnum = g.world.real_object(3099).unwrap();
    let board = mud_game::db::read_object(g, rnum).unwrap();
    mud_game::handler::obj_to_room(g, board, 0);
    let cmd = mud_game::interpreter::find_command(g, b"write").unwrap();
    // Initialize once, then isolate this test from any shipped board posts.
    mud_game::boards::gen_board(g, ch, board, cmd, b"");
    mud_game::boards::board_clear_all(g);
    let taken = g.boards.taken.iter().filter(|&&b| b).count();
    for _ in 0..mud_game::boards::INDEX_SIZE + 1 {
        assert!(mud_game::boards::gen_board(g, ch, board, cmd, b"   "));
    }
    assert_eq!(g.boards.taken.iter().filter(|&&b| b).count(), taken);
    assert!(mud_game::boards::gen_board(g, ch, board, cmd, b"A real headline"));
    assert_eq!(g.boards.msgs[0].len(), 1);
    assert!(g.descriptors.get(g.ch(ch).desc.unwrap()).unwrap().editing.is_some());
}
