use mud_data::{flags, spells::*};
use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
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
    let root = std::env::temp_dir().join(format!("rustmud-affect-join-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: 30,
        passwd: mud_data::crypt::crypt(b"secret", name).unwrap().to_vec(),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    })
}

/// A level-30 caster and a plain NPC victim sharing room 0.
fn caster_and_npc(g: &mut Game) -> (mud_data::ids::CharId, mud_data::ids::CharId) {
    let caster = player(g, b"Caster", 12345);
    mud_game::handler::char_to_room(g, caster, 0);
    let victim = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, victim, 0);
    g.ch_mut(victim).act = flags::FlagSet::EMPTY;
    g.ch_mut(victim).act.set(flags::MOB_ISNPC);
    g.ch_mut(victim).affected_by = flags::FlagSet::EMPTY;
    g.ch_mut(victim).script = None;
    g.ch_mut(victim).level = 10;
    (caster, victim)
}

fn spell_affect(g: &Game, ch: mud_data::ids::CharId, spell: i32) -> mud_game::ch::Affect {
    g.ch(ch).affected.iter().find(|a| a.spell == spell as i16).cloned()
        .unwrap_or_else(|| panic!("victim should be affected by spell {spell}"))
}

#[test]
fn repeated_strength_casts_on_an_npc_saturate_the_modifier() {
    let mut f = fixture("strength"); let g = &mut f.game;
    let (caster, victim) = caster_and_npc(g);
    // NPCs never reach the mortal-only `str_add == 100` early return, so
    // the +2 modifier accumulates on every cast: 64 casts exceed i8::MAX.
    for _ in 0..70 {
        mud_game::magic::mag_affects(g, 30, caster, Some(victim), SPELL_STRENGTH, SAVING_SPELL);
    }
    let af = spell_affect(g, victim, SPELL_STRENGTH);
    assert_eq!(af.location, flags::APPLY_STR as u8);
    assert_eq!(af.modifier, i8::MAX);
    assert!(af.duration > 0);
    let str_ = g.ch(victim).aff_abils.str_;
    assert!((0..=25).contains(&str_), "affected STR {str_} should stay within the NPC range");
    assert_eq!(g.ch(victim).affected.len(), 1);
}

#[test]
fn repeated_waterwalk_casts_on_an_npc_saturate_the_duration() {
    let mut f = fixture("waterwalk"); let g = &mut f.game;
    let (caster, victim) = caster_and_npc(g);
    // Waterwalk adds 24 hours per cast; 1366 casts exceed i16::MAX.
    for _ in 0..1400 {
        mud_game::magic::mag_affects(g, 30, caster, Some(victim), SPELL_WATERWALK, SAVING_SPELL);
    }
    let af = spell_affect(g, victim, SPELL_WATERWALK);
    assert_eq!(af.duration, i16::MAX);
    assert!(g.ch(victim).aff(flags::AFF_WATERWALK));
    assert_eq!(g.ch(victim).affected.len(), 1);
}
