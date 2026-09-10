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
    let root = std::env::temp_dir().join(format!("rustmud-export-target-{}-{label}", std::process::id()));
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
fn export_rejects_target_zone_too_large_for_i64_arithmetic() {
    let mut f = fixture("huge"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); let di = descriptor(g, actor, ConState::Playing);
    let zone = g.world.zones[g.world.rooms[0].zone as usize].number;
    let export_dir = g.lib_dir.join("world").join("export");
    // Each fits an i64, but `target * 100` does not.
    for target in ["92233720368547759", "9223372036854775807", "99999999999999999"] {
        g.descriptors.get_mut(di).unwrap().output.clear();
        let args = format!("{zone} {target}");
        mud_game::olc::export::do_export_zone(g, actor, args.as_bytes(), 0, 0);
        let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(out.contains("can be stored"), "{out}");
        assert!(!out.contains("saved"), "{out}");
        assert!(!export_dir.exists(), "export wrote files for target {target}");
    }
}

#[test]
fn export_still_rejects_target_just_past_the_vnum_ceiling() {
    let mut f = fixture("ceiling"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); let di = descriptor(g, actor, ConState::Playing);
    let zone = g.world.zones[g.world.rooms[0].zone as usize].number;
    mud_game::olc::export::do_export_zone(g, actor, format!("{zone} 656").as_bytes(), 0, 0);
    let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(out.contains("highest vnum at"), "{out}");
    assert!(!g.lib_dir.join("world").join("export").exists());
}
