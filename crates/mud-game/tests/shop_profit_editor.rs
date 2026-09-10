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
    let root = std::env::temp_dir().join(format!("rustmud-shop-profit-{}-{label}", std::process::id()));
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
fn invalid_profit_multipliers_leave_shop_settings_unchanged() {
    let mut f = fixture("invalid"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); let di = descriptor(g, admin, ConState::Sedit);
    for mode in [SEDIT_BUY_PROFIT, SEDIT_SELL_PROFIT] {
        for value in [b"-1".to_vec(), b"-0.01".to_vec(), vec![b'9'; 400]] {
            let mut olc = OlcData::new(); sedit_setup_existing(g, &mut olc, 0);
            olc.shop.as_mut().unwrap().profit_buy = 1.5; olc.shop.as_mut().unwrap().profit_sell = 0.5;
            olc.mode = mode;
            let olc = sedit_parse(g, di, olc, &value).unwrap();
            assert_eq!(olc.mode, mode);
            assert_eq!((olc.shop.as_ref().unwrap().profit_buy, olc.shop.as_ref().unwrap().profit_sell), (1.5, 0.5));
        }
    }
}
