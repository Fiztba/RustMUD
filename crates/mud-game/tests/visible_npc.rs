use mud_data::{flags, types::*};
use mud_game::game::Game;
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
    let root = std::env::temp_dir().join(format!("rustmud-visible-npc-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

/// An invisible NPC of the given level standing in room 0, with no
/// player_specials (as any loaded mobile, switched into or not).
fn invisible_npc(label: &str, level: u8) -> (Fixture, mud_data::ids::CharId) {
    let mut f = fixture(label);
    let g = &mut f.game;
    let npc = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(npc).name = Some(b"testnpc".to_vec());
    g.ch_mut(npc).position = POS_STANDING;
    g.ch_mut(npc).level = level;
    g.ch_mut(npc).affected_by.set(flags::AFF_INVISIBLE);
    mud_game::handler::char_to_room(g, npc, 0);
    assert!(g.ch(npc).is_npc());
    assert!(g.ch(npc).player_specials.is_none());
    (f, npc)
}

#[test]
fn immortal_level_npc_visible_does_not_access_player_storage() {
    let (mut f, npc) = invisible_npc("immortal", LVL_IMMORT + 3);
    mud_game::act::other::do_visible(&mut f.game, npc, b"", 0, 0);
    let g = &f.game;
    assert!(!g.ch(npc).aff(flags::AFF_INVISIBLE), "immortal-level NPC should appear");
    assert!(g.ch(npc).player_specials.is_none());
}

#[test]
fn immortal_level_npc_visible_via_command_interpreter() {
    let (mut f, npc) = invisible_npc("interpreter", LVL_IMMORT);
    mud_game::interpreter::command_interpreter(&mut f.game, npc, b"visible");
    assert!(!f.game.ch(npc).aff(flags::AFF_INVISIBLE));
}

#[test]
fn mortal_level_npc_visible_breaks_invisibility() {
    let (mut f, npc) = invisible_npc("mortal", LVL_IMMORT - 1);
    mud_game::act::other::do_visible(&mut f.game, npc, b"", 0, 0);
    assert!(!f.game.ch(npc).aff(flags::AFF_INVISIBLE), "mortal-level NPC should appear");
}
