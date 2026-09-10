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
    let root = std::env::temp_dir().join(format!("rustmud-editor-delete-{}-{label}", std::process::id()));
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
fn copied_object_and_room_delete_the_edited_target() {
    let mut f = fixture("copy-targets");
    let g = &mut f.game;
    g.config.auto_save_olc = false;
    let ch = player(g, b"Builder", 12345);
    g.ch_mut(ch).level = LVL_IMPL;
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let target = 3098;
    let mut object = g.world.obj_protos[0].clone();
    object.proto_script.clear();
    let object_source = object.vnum;
    mud_game::olc::genobj::add_object(g, &object, target).unwrap();
    let mut room = g.world.rooms[0].clone();
    room.proto_script.clear();
    room.vnum = target;
    room.zone = g.world.real_zone(30).unwrap();
    mud_game::olc::genwld::add_room(g, &room, 0).unwrap();
    let room_source = g.world.rooms.iter().find(|r| r.zone == room.zone && r.vnum != target).unwrap().vnum;
    for existing in [true, false] {
        mud_game::olc::oedit::do_oasis_oedit(g, ch, b"3098", 0, 0);
        for input in [b"w".as_slice(), object_source.to_string().as_bytes(), b"x", b"y"] {
            assert!(mud_game::olc::olc_parse(g, di, input));
        }
        assert!(g.world.real_object(object_source).is_some(), "object source removed (existing={existing})");
        assert!(g.world.real_object(target).is_none());
        assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
        mud_game::olc::redit::do_oasis_redit(g, ch, b"3098", 0, 0);
        for input in [b"w".as_slice(), room_source.to_string().as_bytes(), b"x", b"y"] {
            assert!(mud_game::olc::olc_parse(g, di, input));
        }
        assert!(g.world.real_room(room_source).is_some(), "room source removed (existing={existing})");
        assert!(g.world.real_room(target).is_none());
        assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
        assert!(!g.ch(ch).act.is_set(flags::PLR_WRITING));
    }

    // Another builder inserts a prototype before the open editor's target.
    // Missing board sentinels are a separate insertion defect covered by PR #34.
    g.boards.rnum.fill(0);
    mud_game::olc::genobj::add_object(g, &object, target).unwrap();
    mud_game::olc::oedit::do_oasis_oedit(g, ch, b"3098", 0, 0);
    let inserted = (1..target).find(|&v| g.world.real_object(v).is_none()).unwrap();
    mud_game::olc::genobj::add_object(g, &object, inserted).unwrap();
    for input in [b"x".as_slice(), b"n"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert!(g.world.real_object(target).is_some());
    for input in [b"x".as_slice(), b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert!(g.world.real_object(target).is_none());
    assert!(g.world.real_object(inserted).is_some());

    // If the target disappears while editing, confirmation cannot delete its neighbor.
    mud_game::olc::genwld::add_room(g, &room, 0).unwrap();
    mud_game::olc::redit::do_oasis_redit(g, ch, b"3098", 0, 0);
    let rnum = g.world.real_room(target).unwrap();
    assert!(mud_game::olc::genwld::delete_room(g, rnum));
    let count = g.world.rooms.len();
    for input in [b"x".as_slice(), b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert_eq!(g.world.rooms.len(), count);
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
}
