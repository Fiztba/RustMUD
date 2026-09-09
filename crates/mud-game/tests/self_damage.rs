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
    let root = std::env::temp_dir().join(format!("rustmud-selfdamage-{}-{label}", std::process::id()));
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
fn disabling_pk_preserves_poison_and_suffering_damage() {
    let mut f = fixture("pk-disabled");
    let g = &mut f.game;
    g.config.pk_setting = 0;
    let victim = player(g, b"Tester", 12345);
    let other = player(g, b"Other", 12346);
    for ch in [victim, other] {
        mud_game::handler::char_to_room(g, ch, 0);
        descriptor(g, ch, ConState::Playing);
        g.ch_mut(ch).points.hit = 20;
        g.ch_mut(ch).points.max_hit = 20;
        g.ch_mut(ch).position = POS_STANDING;
    }
    assert_eq!(mud_game::fight::damage(g, other, victim, 2, mud_data::spells::SPELL_POISON), 0);
    assert_eq!(g.ch(victim).points.hit, 20);
    assert_eq!(mud_game::fight::damage(g, victim, victim, 2, mud_data::spells::SPELL_POISON), 2);
    assert_eq!(g.ch(victim).points.hit, 18);
    g.ch_mut(victim).points.hit = -3;
    g.ch_mut(victim).position = POS_INCAP;
    assert_eq!(mud_game::fight::damage(g, victim, victim, 1, mud_data::spells::TYPE_SUFFERING), 1);
    assert_eq!(g.ch(victim).points.hit, -4);
    assert!(!g.ch(victim).plr(flags::PLR_KILLER));
    assert_eq!(g.ch(victim).fighting, None);
}
