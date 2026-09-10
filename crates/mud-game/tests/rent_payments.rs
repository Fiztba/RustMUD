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
    let root = std::env::temp_dir().join(format!("rustmud-rent-payment-{}-{label}", std::process::id()));
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
fn cryogenic_storage_charges_the_bank_remainder() {
    let mut f = fixture("cryo");
    let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    for (gold, bank, cost, expected) in [
        (50, 100, 80, (0, 70)), (0, 80, 80, (0, 0)),
        (100, 50, 80, (20, 50)), (MAX_GOLD, MAX_BANK, MAX_GOLD, (0, MAX_BANK)),
    ] {
        g.ch_mut(ch).points.gold = gold;
        g.ch_mut(ch).points.bank_gold = bank;
        mud_game::objsave::crash_cryosave(g, ch, cost);
        assert_eq!((g.ch(ch).points.gold, g.ch(ch).points.bank_gold), expected);
        let path = g.lib_dir.join(mud_world::players::get_filename(mud_world::players::FileKind::Objs, b"Customer").unwrap());
        let saved = std::fs::read_to_string(path).unwrap();
        let header: Vec<i64> = saved.lines().next().unwrap().split_whitespace().map(|s| s.parse().unwrap()).collect();
        assert_eq!((header[3], header[4]), (expected.0 as i64, expected.1 as i64));
    }
}

#[test]
fn rental_payment_handles_large_balances_and_elapsed_costs() {
    let mut f = fixture("rent");
    let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    for (elapsed, expected) in [(2 * SECS_PER_REAL_DAY, (0, 1_000_000_000)), (-SECS_PER_REAL_DAY, (2_000_000_000, 2_000_000_000))] {
        g.ch_mut(ch).points.gold = 2_000_000_000;
        g.ch_mut(ch).points.bank_gold = 2_000_000_000;
        mud_game::objsave::crash_rentsave(g, ch, 1_500_000_000);
        g.now += elapsed;
        assert_eq!(mud_game::objsave::crash_load(g, ch), 0);
        assert_eq!((g.ch(ch).points.gold, g.ch(ch).points.bank_gold), expected);
    }
    g.ch_mut(ch).points.gold = MAX_GOLD;
    g.ch_mut(ch).points.bank_gold = MAX_BANK;
    mud_game::act::wizstat::do_stat_character(g, ch, ch);
}
