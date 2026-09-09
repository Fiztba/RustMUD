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
    let root = std::env::temp_dir().join(format!("rustmud-npccommands-{}-{label}", std::process::id()));
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

fn setup(label: &str) -> (Fixture, mud_data::ids::CharId) {
    let mut f = fixture(label);
    let g = &mut f.game;
    let npc = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(npc).name = Some(b"testnpc".to_vec());
    g.ch_mut(npc).position = POS_STANDING;
    mud_game::handler::char_to_room(g, npc, 0);
    assert!(g.ch(npc).player_specials.is_none());
    (f, npc)
}

#[test]
fn npc_history_does_not_access_player_storage() {
    let (mut f, npc) = setup("history");
    mud_game::interpreter::command_interpreter(&mut f.game, npc, b"history all");
}

#[test]
fn npc_quest_does_not_access_player_storage() {
    let (mut f, npc) = setup("quest");
    for cmd in [b"quest history".as_slice(), b"quest progress", b"quest leave"] {
        mud_game::interpreter::command_interpreter(&mut f.game, npc, cmd);
    }
}

#[test]
fn switched_npc_can_page_help_without_player_storage() {
    let (mut f, npc) = setup("pager");
    let g = &mut f.game;
    let di = descriptor(g, npc, ConState::Playing);
    mud_game::act::informative::page_string(g, npc, b"Help text\r\n");
    assert!(String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains("Help text"));
    assert!(g.ch(npc).player_specials.is_none());
}

#[test]
fn set_conditions_refuses_npcs() {
    let (mut f, npc) = setup("conditions");
    let g = &mut f.game;
    let admin = player(g, b"Admin", 12345);
    g.ch_mut(admin).level = LVL_IMPL;
    descriptor(g, admin, ConState::Playing);
    mud_game::handler::char_to_room(g, admin, 0);
    for cmd in [b"testnpc hunger off".as_slice(), b"testnpc thirst 10", b"testnpc drunk 0"] {
        mud_game::act::wizset::do_set(g, admin, cmd, 0, 0);
    }
    assert!(g.ch(npc).player_specials.is_none());
}
