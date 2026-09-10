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
    let root = std::env::temp_dir().join(format!("rustmud-config-rooms-{}-{label}", std::process::id()));
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
fn saved_start_room_changes_apply_to_the_next_login() {
    let mut f = fixture("login"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, admin, 0); let di = descriptor(g, admin, ConState::Cedit);
    let mut olc = mud_game::olc::OlcData::new(); let mut config = g.config.clone(); config.auto_save = false;
    config.mortal_start_room = g.world.rooms[1].vnum as i32;
    config.immort_start_room = g.world.rooms[2].vnum as i32;
    config.frozen_start_room = g.world.rooms[3].vnum as i32;
    olc.config = Some(Box::new(config)); olc.mode = mud_game::olc::cedit::CEDIT_CONFIRM_SAVESTRING;
    assert!(mud_game::olc::cedit::cedit_parse(g, di, olc, b"y").is_none());
    assert_eq!((g.r_mortal_start_room, g.r_immort_start_room, g.r_frozen_start_room), (1, 2, 3));
    for (name, level, frozen, room) in [(b"Mortal".as_slice(), 10, false, 1), (b"Immortal".as_slice(), LVL_IMMORT, false, 2), (b"Frozen".as_slice(), 10, true, 3)] {
        let ch = player(g, name, 12346 + room as i64); g.ch_mut(ch).level = level; g.ch_mut(ch).ps_mut().load_room = NOWHERE;
        if frozen { g.ch_mut(ch).act.set(mud_data::flags::PLR_FROZEN); }
        let td = descriptor(g, ch, ConState::Menu); mud_game::login::enter_player_game(g, td);
        assert_eq!(g.ch(ch).in_room, room);
    }
}

#[test]
fn config_save_rejects_missing_mortal_room_and_keeps_optional_fallbacks() {
    let mut f = fixture("fallback"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    let di = descriptor(g, admin, ConState::Cedit);
    let original = g.config.clone(); let cached = (g.r_mortal_start_room, g.r_immort_start_room, g.r_frozen_start_room);
    let mut olc = mud_game::olc::OlcData::new(); let mut config = original.clone(); config.auto_save = false;
    config.mortal_start_room = i32::MAX;
    olc.config = Some(Box::new(config)); olc.mode = mud_game::olc::cedit::CEDIT_CONFIRM_SAVESTRING;
    let mut olc = mud_game::olc::cedit::cedit_parse(g, di, olc, b"y").expect("failed save retains editor");
    assert_eq!(g.config.mortal_start_room, original.mortal_start_room);
    assert_eq!((g.r_mortal_start_room, g.r_immort_start_room, g.r_frozen_start_room), cached);
    let config = olc.config.as_mut().unwrap();
    config.mortal_start_room = g.world.rooms[1].vnum as i32; config.immort_start_room = i32::MAX; config.frozen_start_room = -1;
    olc.mode = mud_game::olc::cedit::CEDIT_CONFIRM_SAVESTRING;
    assert!(mud_game::olc::cedit::cedit_parse(g, di, olc, b"y").is_none());
    assert_eq!((g.r_mortal_start_room, g.r_immort_start_room, g.r_frozen_start_room), (1, 1, 1));
}
