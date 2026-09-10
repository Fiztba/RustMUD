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
    let root = std::env::temp_dir().join(format!("rustmud-area-save-{}-{label}", std::process::id()));
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

use mud_data::spells::*;
#[test]
fn earthquake_uses_the_casts_saving_throw_category() {
    let mut f = fixture("categories"); let g = &mut f.game;
    let caster = player(g, b"Caster", 12345);
    mud_game::handler::char_to_room(g, caster, 0);
    descriptor(g, caster, ConState::Playing);
    let victim = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, victim, 0);
    g.ch_mut(victim).act = flags::FlagSet::EMPTY;
    g.ch_mut(victim).act.set(flags::MOB_ISNPC);
    g.ch_mut(victim).affected_by = flags::FlagSet::EMPTY;
    g.ch_mut(victim).script = None;
    g.ch_mut(victim).level = 10;
    g.world.rooms[0].room_flags = [0; 4];
    g.rooms[0].people = [caster, victim].into_iter().collect();
    let mut expected_rng = mud_data::rng::CircleRng::new(12345);
    let full_damage = expected_rng.dice(2, 8) + 20;
    assert!(expected_rng.rand_number(0, 99) > 1);
    for protected_save in [SAVING_SPELL, SAVING_ROD, SAVING_BREATH] {
      g.ch_mut(victim).apply_saving_throw = [1000; 5];
      g.ch_mut(victim).apply_saving_throw[protected_save as usize] = -1000;
      for (cast_type, save) in [(CAST_SPELL, SAVING_SPELL), (CAST_WAND, SAVING_ROD),
          (CAST_STAFF, SAVING_ROD), (CAST_SCROLL, SAVING_ROD), (CAST_POTION, SAVING_ROD),
          (-1, SAVING_BREATH)] {
        let expected = if save == protected_save { full_damage / 2 } else { full_damage };
        mud_game::fight::stop_fighting(g, caster);
        mud_game::fight::stop_fighting(g, victim);
        g.ch_mut(victim).position = POS_STANDING;
        g.ch_mut(victim).points.max_hit = 1000; g.ch_mut(victim).points.hit = 1000;
        g.rng = mud_data::rng::CircleRng::new(12345);
        mud_game::spell_parser::call_magic(g, caster, None, None, SPELL_EARTHQUAKE, 20, cast_type);
        assert_eq!(1000 - g.ch(victim).points.hit, expected, "cast type {cast_type}, protected save {protected_save}");
      }
    }
}
