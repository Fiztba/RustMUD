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
    let root = std::env::temp_dir().join(format!("rustmud-shopbank-{}-{label}", std::process::id()));
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
fn sales_debit_cash_and_bank_and_stop_when_unaffordable() {
    let mut f = fixture("payments");
    let g = &mut f.game;
    let seller = player(g, b"Seller", 12345);
    descriptor(g, seller, ConState::Playing);
    let keeper = player(g, b"Keeper", 0);
    g.ch_mut(keeper).act.set(flags::MOB_ISNPC);
    g.ch_mut(keeper).mob_rnum = 0;
    g.ch_mut(keeper).short_descr = Some(b"the keeper".to_vec());
    for ch in [seller, keeper] { mud_game::handler::char_to_room(g, ch, 0); }
    g.world.rooms[0].room_flags = [0; 4];
    g.world.shops = vec![mud_world::model::Shop {
        profit_buy: 1.0, profit_sell: 1.0, close1: 24,
        type_list: vec![mud_world::model::ShopBuyData { type_: flags::ITEM_TREASURE, keywords: None }],
        in_rooms: vec![g.world.rooms[0].vnum as i32],
        message_sell: Some(b"%s Paid %d coins.".to_vec()),
        ..Default::default()
    }];
    g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, bank: 45_000, ..Default::default() }];
    g.ch_mut(keeper).points.gold = 5_000;
    g.ch_mut(seller).points.gold = 0;
    for _ in 0..3 {
        let mut obj = mud_game::obj::create_obj();
        obj.type_flag = flags::ITEM_TREASURE;
        obj.cost = 20_000;
        obj.name = Some(b"gem".to_vec());
        obj.short_description = Some(b"a gem".to_vec());
        let oid = g.objs.insert(obj);
        mud_game::handler::obj_to_char(g, oid, seller);
    }
    let command = mud_game::interpreter::find_command(g, b"sell").unwrap();
    assert!(mud_game::shop::shop_keeper(g, seller, keeper, command, b"3 gem"));
    assert_eq!(g.ch(seller).points.gold, 40_000);
    assert_eq!(g.ch(seller).carrying.len(), 1);
    assert_eq!(g.ch(keeper).points.gold + g.shops_rt[0].bank, 10_000);
    assert!(mud_game::shop::shop_keeper(g, seller, keeper, command, b"gem"));
    assert_eq!(g.ch(seller).points.gold, 40_000);
    assert_eq!(g.ch(seller).carrying.len(), 1);
}
