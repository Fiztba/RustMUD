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
    let root = std::env::temp_dir().join(format!("rustmud-editor-numeric-{}-{label}", std::process::id()));
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

use mud_game::olc::{OlcData, prefedit::*, qedit::*, medit::*};

#[test]
fn preference_choices_reject_minimum_integer() {
    let mut f = fixture("prefs");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    g.ch_mut(ch).level = LVL_IMPL;
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    do_oasis_prefedit(g, ch, b"", 0, 0);
    let mut olc = g.olc.remove(&di).unwrap();
    for mode in [PREFEDIT_COLOR, PREFEDIT_SYSLOG] {
        for input in ["-2147483648", "-2147483649", "0", "5", "2147483647", "999999999999999999999"] {
            olc.mode = mode;
            let before = olc.prefs.as_ref().unwrap().pref.clone();
            olc = prefedit_parse(g, di, olc, input.as_bytes()).unwrap();
            assert_eq!(olc.mode, mode);
            assert_eq!(olc.prefs.as_ref().unwrap().pref, before);
        }
        for choice in 1..=4 {
            olc.mode = mode;
            olc = prefedit_parse(g, di, olc, choice.to_string().as_bytes()).unwrap();
            let pref = &olc.prefs.as_ref().unwrap().pref;
            let (lo, hi) = if mode == PREFEDIT_COLOR { (flags::PRF_COLOR_1, flags::PRF_COLOR_2) }
                else { (flags::PRF_LOG1, flags::PRF_LOG2) };
            assert_eq!(pref.is_set(lo), (choice - 1) & 1 != 0);
            assert_eq!(pref.is_set(hi), (choice - 1) & 2 != 0);
        }
    }
}

#[test]
fn quest_choices_reject_minimum_integer() {
    let mut f = fixture("quests");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    g.ch_mut(ch).level = LVL_IMPL;
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    do_oasis_qedit(g, ch, b"3098", 0, 0);
    let mut olc = g.olc.remove(&di).unwrap();
    for input in ["-2147483648", "-2147483649", "0", "8", "2147483647", "999999999999999999999"] {
        olc.mode = QEDIT_TYPES;
        let before = olc.quest.as_ref().unwrap().type_;
        olc = qedit_parse(g, di, olc, input.as_bytes()).unwrap();
        assert_eq!(olc.mode, QEDIT_TYPES);
        assert_eq!(olc.quest.as_ref().unwrap().type_, before);
    }
    for choice in 1..=7 {
        olc.mode = QEDIT_TYPES;
        olc = qedit_parse(g, di, olc, choice.to_string().as_bytes()).unwrap();
        assert_eq!(olc.quest.as_ref().unwrap().type_, choice - 1);
    }
}

#[test]
fn mobile_choices_clamp_minimum_integer() {
    let mut f = fixture("mobiles");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    let di = descriptor(g, ch, ConState::Medit);
    let mut olc = OlcData::new();
    medit_setup_existing(g, &mut olc, 0);
    for mode in [MEDIT_SEX, MEDIT_POS, MEDIT_DEFAULT_POS] {
        let max = if mode == MEDIT_SEX { NUM_GENDERS as i32 } else { NUM_POSITIONS as i32 };
        for input in [i32::MIN, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, i32::MAX] {
            olc.mode = mode;
            olc = medit_parse(g, di, olc, input.to_string().as_bytes()).unwrap();
            let mob = olc.mob.as_ref().unwrap();
            let actual = match mode { MEDIT_SEX => mob.sex, MEDIT_POS => mob.position, _ => mob.default_pos };
            assert_eq!(actual, (i64::from(input) - 1).clamp(0, i64::from(max - 1)) as i32);
        }
    }
}
