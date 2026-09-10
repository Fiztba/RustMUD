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
    let root = std::env::temp_dir().join(format!("rustmud-editor-apply-{}-{label}", std::process::id()));
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

use mud_game::olc::{OlcData, oedit::*};
#[test]
fn apply_selection_checks_the_selected_type_in_other_slots() {
    let mut f = fixture("selection"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); g.ch_mut(builder).level = LVL_BUILDER;
    let di = descriptor(g, builder, ConState::Oedit);
    let mut olc = OlcData::new(); oedit_setup_existing(g, &mut olc, 0);
    olc.obj.as_mut().unwrap().affected = Default::default();
    olc.obj.as_mut().unwrap().affected[0].location = 1;
    olc.value = 1; olc.mode = OEDIT_APPLY;
    let olc = oedit_parse(g, di, olc, b"2").unwrap();
    assert_eq!(olc.mode, OEDIT_APPLY, "a second strength apply must be rejected");
}
#[test]
fn apply_selection_allows_adjacent_types_and_reediting_the_same_slot() {
    let mut f = fixture("adjacent"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); g.ch_mut(builder).level = LVL_BUILDER;
    let di = descriptor(g, builder, ConState::Oedit);
    for (slot, existing) in [(1, 2), (0, 1)] {
        let mut olc = OlcData::new(); oedit_setup_existing(g, &mut olc, 0);
        olc.obj.as_mut().unwrap().affected = Default::default();
        olc.obj.as_mut().unwrap().affected[0].location = existing;
        olc.value = slot; olc.mode = OEDIT_APPLY;
        let olc = oedit_parse(g, di, olc, b"2").unwrap();
        assert_eq!(olc.mode, OEDIT_APPLYMOD);
        assert_eq!(olc.obj.as_ref().unwrap().affected[slot as usize].location, 1);
    }
}

#[test]
fn every_apply_and_slot_obeys_duplicate_and_implementor_rules() {
    let mut f = fixture("matrix"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); let di = descriptor(g, builder, ConState::Oedit);
    for level in [LVL_BUILDER, LVL_IMPL] {
        g.ch_mut(builder).level = level;
        for location in 1..mud_data::flags::NUM_APPLIES as i32 {
            for slot in 0..MAX_OBJ_AFFECT {
                for other in 0..MAX_OBJ_AFFECT {
                    let mut olc = OlcData::new(); oedit_setup_existing(g, &mut olc, 0);
                    olc.obj.as_mut().unwrap().affected = Default::default();
                    olc.obj.as_mut().unwrap().affected[other].location = location;
                    olc.obj.as_mut().unwrap().affected[other].modifier = 7;
                    olc.value = slot as i32; olc.mode = OEDIT_APPLY;
                    let olc = oedit_parse(g, di, olc, (location + 1).to_string().as_bytes()).unwrap();
                    let accepted = level == LVL_IMPL || slot == other;
                    assert_eq!(olc.mode, if accepted { OEDIT_APPLYMOD } else { OEDIT_APPLY });
                    assert_eq!(olc.obj.as_ref().unwrap().affected[other].modifier, 7);
                    g.descriptors.get_mut(di).unwrap().output.clear();
                }
            }
        }
    }
    for clear in [b"0".as_slice(), b"1".as_slice()] {
        let mut olc = OlcData::new(); oedit_setup_existing(g, &mut olc, 0);
        olc.value = 0; olc.mode = OEDIT_APPLY;
        olc.obj.as_mut().unwrap().affected[0].location = 1;
        olc.obj.as_mut().unwrap().affected[0].modifier = 7;
        let olc = oedit_parse(g, di, olc, clear).unwrap();
        assert_eq!(olc.obj.as_ref().unwrap().affected[0].location, 0);
        assert_eq!(olc.obj.as_ref().unwrap().affected[0].modifier, 0);
    }
}
