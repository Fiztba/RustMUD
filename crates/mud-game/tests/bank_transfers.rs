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
    let root = std::env::temp_dir().join(format!("rustmud-bank-transfers-{}-{label}", std::process::id()));
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
fn bank_transfers_are_atomic_at_balance_limits() {
    let mut f = fixture("limits");
    let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let bank = mud_game::db::read_object(g, 0).unwrap();
    mud_game::handler::obj_to_room(g, bank, 0);
    g.obj_specs[0] = Some(mud_game::spec::ObjSpec::Bank);
    for (command, gold, saved, amount, expected) in [
        ("deposit", 20, MAX_BANK-5, 10, (20, MAX_BANK-5)),
        ("withdraw", MAX_GOLD-5, 20, 10, (MAX_GOLD-5, 20)),
        ("deposit", 20, MAX_BANK-5, 5, (15, MAX_BANK)),
        ("withdraw", MAX_GOLD-5, 20, 5, (MAX_GOLD, 15)),
        ("deposit", 20, MAX_BANK, 1, (20, MAX_BANK)),
        ("withdraw", MAX_GOLD, 20, 1, (MAX_GOLD, 20)),
        ("deposit", 4, 0, 5, (4, 0)),
        ("withdraw", 0, 4, 5, (0, 4)),
        ("deposit", MAX_GOLD, 0, MAX_GOLD, (0, MAX_GOLD)),
        ("withdraw", 0, MAX_BANK, MAX_BANK, (MAX_BANK, 0)),
        ("deposit", 10, 20, -1, (10, 20)),
        ("withdraw", 10, 20, 0, (10, 20)),
    ] {
        g.ch_mut(ch).points.gold = gold;
        g.ch_mut(ch).points.bank_gold = saved;
        let cmd = g.commands.iter().position(|c| c.command == command.as_bytes()).unwrap();
        assert!(mud_game::spec::special(g, ch, cmd, amount.to_string().as_bytes()));
        let p = g.ch(ch).points;
        assert_eq!((p.gold, p.bank_gold), expected, "{command} {amount}");
        assert_eq!(p.gold as i64 + p.bank_gold as i64, gold as i64 + saved as i64);
    }
}
