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
    let root = std::env::temp_dir().join(format!("rustmud-steal-gold-{}-{label}", std::process::id()));
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
fn stealing_large_balances_preserves_gold_at_the_receiver_limit() {
    let mut f = fixture("balances"); let g = &mut f.game;
    let thief = player(g, b"Thief", 12345);
    let victim = player(g, b"Victim", 12346);
    for ch in [thief, victim] { mud_game::handler::char_to_room(g, ch, 0); }
    descriptor(g, thief, ConState::Playing);
    g.ch_mut(thief).set_skill(mud_data::spells::SKILL_STEAL, 100);
    g.ch_mut(victim).position = POS_SLEEPING;
    g.config.pt_setting = 2;
    g.world.rooms[0].room_flags = [0; 4];
    for (balance, expected) in [(0, 1782), (MAX_GOLD - 5, 5), (MAX_GOLD, 0)] {
        g.ch_mut(thief).points.gold = balance;
        g.ch_mut(victim).points.gold = MAX_GOLD;
        g.rng = mud_data::rng::CircleRng::new(12345);
        mud_game::act::other::do_steal(g, thief, b"gold Victim", 0, 0);
        assert_eq!(g.ch(thief).points.gold, balance + expected);
        assert_eq!(g.ch(victim).points.gold, MAX_GOLD - expected);
    }
}
