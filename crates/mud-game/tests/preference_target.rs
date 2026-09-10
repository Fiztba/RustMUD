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
    let root = std::env::temp_dir().join(format!("rustmud-preference-target-{}-{label}", std::process::id()));
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
fn removed_target_closes_preference_editor() {
    let mut f = fixture("removed");
    let g = &mut f.game;
    let editor = player(g, b"Builder", 12345);
    g.ch_mut(editor).level = LVL_IMPL;
    let di = descriptor(g, editor, ConState::Playing);
    mud_game::handler::char_to_room(g, editor, 0);
    for input in [b"p".as_slice(), b"q", b"y"] {
        let target = player(g, b"Target", 12346);
        g.character_list.push_back(target);
        mud_game::handler::char_to_room(g, target, 0);
        mud_game::olc::prefedit::do_oasis_prefedit(g, editor, b"Target", 0, 0);
        assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Prefedit);
        if input == b"y" {
            for edit in [b"l".as_slice(), b"30", b"q"] {
                assert!(mud_game::olc::olc_parse(g, di, edit));
            }
        }
        mud_game::handler::extract_char_final(g, target);
        assert!(g.try_ch(target).is_none());
        assert!(mud_game::olc::olc_parse(g, di, input));
        assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
        assert!(!g.olc.contains_key(&di));
        assert!(!g.ch(editor).act.is_set(flags::PLR_WRITING));
    }
}
