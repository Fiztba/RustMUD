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
    let root = std::env::temp_dir().join(format!("rustmud-shop-editor-runtime-{}-{label}", std::process::id()));
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

use mud_game::olc::{OlcData, sedit::*};
#[test]
fn saving_shop_edits_preserves_runtime_changes_since_the_editor_opened() {
    let mut f = fixture("live"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    let di = descriptor(g, admin, ConState::Sedit); g.config.auto_save_olc = false;
    g.shops_rt[0].bank = 100; g.shops_rt[0].sort = 2;
    let mut olc = OlcData::new(); sedit_setup_existing(g, &mut olc, 0); olc.number = g.world.shops[0].vnum as i32;
    g.shops_rt[0].bank = 350; g.shops_rt[0].sort = 7;
    olc.mode = SEDIT_CONFIRM_SAVESTRING;
    assert!(sedit_parse(g, di, olc, b"y").is_none());
    assert_eq!((g.shops_rt[0].bank, g.shops_rt[0].sort), (350, 7));
}
#[test]
fn copying_a_shop_does_not_copy_its_runtime_money_or_sort_state() {
    let mut f = fixture("copy"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); g.ch_mut(admin).level = LVL_IMPL;
    let di = descriptor(g, admin, ConState::Sedit); g.config.auto_save_olc = false;
    g.shops_rt[0].bank = 900; g.shops_rt[0].sort = 42;
    let source = g.world.shops[0].vnum;
    let mut olc = OlcData::new(); sedit_setup_existing(g, &mut olc, 0); olc.number = 65534;
    olc.mode = SEDIT_CONFIRM_SAVESTRING;
    assert!(sedit_parse(g, di, olc, b"y").is_none());
    let new = mud_game::olc::genshp::real_shop(g, 65534).unwrap();
    let old = mud_game::olc::genshp::real_shop(g, source as i32).unwrap();
    assert_eq!((g.shops_rt[new].bank, g.shops_rt[new].sort), (0, 0));
    assert_eq!((g.shops_rt[old].bank, g.shops_rt[old].sort), (900, 42));
}
