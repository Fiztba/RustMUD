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
    let root = std::env::temp_dir().join(format!("rustmud-board-ownership-{}-{label}", std::process::id()));
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
fn mentioning_a_player_in_the_subject_does_not_make_them_the_author() {
    let mut f = fixture("subject"); let g = &mut f.game;
    let writer = player(g, b"Alice", 12345); let reader = player(g, b"Bob", 12346);
    let writer_di = descriptor(g, writer, ConState::Playing); descriptor(g, reader, ConState::Playing);
    for ch in [writer, reader] { mud_game::handler::char_to_room(g, ch, 0); }
    let rnum = g.world.real_object(3099).unwrap(); let board = mud_game::db::read_object(g, rnum).unwrap();
    mud_game::handler::obj_to_room(g, board, 0);
    let write = mud_game::interpreter::find_command(g, b"write").unwrap(); let remove = mud_game::interpreter::find_command(g, b"remove").unwrap();
    mud_game::boards::gen_board(g, writer, board, write, b""); mud_game::boards::board_clear_all(g);
    for subject in [&b"A message for (Bob)"[..], b"(Bob) :: (Bob)", b"Nothing special"] {
        for (author_level, reader_level, can_remove) in [(10, 10, false), (10, LVL_GOD, true), (LVL_IMPL, LVL_GOD, false)] {
            g.ch_mut(writer).level = author_level; g.ch_mut(reader).level = reader_level;
            mud_game::boards::gen_board(g, writer, board, write, subject);
            let slot = g.boards.msgs[0][0].slot_num; g.descriptors.get_mut(writer_di).unwrap().editing = None;
            mud_game::boards::board_finish_write(g, writer, slot, 0, Some(b"Message body".to_vec()));
            // Exercise the persisted heading as well as the fresh in-memory one.
            g.boards.loaded = false;
            assert!(mud_game::boards::gen_board(g, reader, board, remove, b"1"));
            assert_eq!(g.boards.msgs[0].len(), if can_remove { 0 } else { 1 });
            if !can_remove {
                assert!(mud_game::boards::gen_board(g, writer, board, remove, b"1"));
                assert!(g.boards.msgs[0].is_empty());
            }
        }
    }
}
