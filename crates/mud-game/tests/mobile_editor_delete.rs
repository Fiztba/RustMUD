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
    let root = std::env::temp_dir().join(format!("rustmud-mobile-delete-{}-{label}", std::process::id()));
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
fn mobile_delete_confirmation_accepts_no_and_yes() {
    let mut f = fixture("confirmation");
    let g = &mut f.game;
    g.config.auto_save_olc = false;
    let ch = player(g, b"Builder", 12345);
    g.ch_mut(ch).level = LVL_IMPL;
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let vnum = 3098;
    let mut proto = g.world.mob_protos[0].clone();
    proto.proto_script.clear();
    mud_game::olc::genmob::add_mobile(g, &proto, vnum).unwrap();
    mud_game::olc::medit::do_oasis_medit(g, ch, b"3098", 0, 0);
    for input in [b"x".as_slice(), b"n"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert_eq!(g.olc.get(&di).unwrap().mode, mud_game::olc::medit::MEDIT_MAIN_MENU);
    assert!(g.world.real_mobile(vnum).is_some());
    for input in [b"s".as_slice(), b"n"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert_eq!(g.olc.get(&di).unwrap().script_mode, mud_game::olc::trigedit::SCRIPT_NEW_TRIGGER);
    assert!(mud_game::olc::olc_parse(g, di, b"0"));
    assert!(mud_game::olc::olc_parse(g, di, b"q"));
    assert_eq!(g.olc.get(&di).unwrap().mode, mud_game::olc::medit::MEDIT_MAIN_MENU);
    // Copying changes the scratch prototype, never the deletion target.
    let source = g.world.mob_protos[0].vnum;
    assert!(mud_game::olc::olc_parse(g, di, b"w"));
    assert!(mud_game::olc::olc_parse(g, di, source.to_string().as_bytes()));
    for input in [b"x".as_slice(), b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert!(g.world.real_mobile(vnum).is_none());
    assert!(g.world.real_mobile(source).is_some());
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
    assert!(!g.ch(ch).act.is_set(flags::PLR_WRITING));
    // An unsaved new mobile copied from an existing one cannot delete its source.
    mud_game::olc::medit::do_oasis_medit(g, ch, b"3098", 0, 0);
    for input in [b"w".as_slice(), source.to_string().as_bytes(), b"x", b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert!(g.world.real_mobile(source).is_some());
    assert!(g.world.real_mobile(vnum).is_none());
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
}
