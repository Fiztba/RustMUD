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
    let root = std::env::temp_dir().join(format!("rustmud-shop-total-{}-{label}", std::process::id()));
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
fn bulk_sales_stop_before_exceeding_the_sellers_gold_capacity() {
    for (starting, cost, expected_sold) in [(0, 1_500_000_000, 1), (MAX_GOLD - 100, 100, 1), (MAX_GOLD, 100, 0)] {
        let mut f = fixture(&starting.to_string()); let g = &mut f.game;
        let seller = player(g, b"Seller", 12345); descriptor(g, seller, ConState::Playing);
        let keeper = player(g, b"Keeper", 0); g.ch_mut(keeper).act.set(flags::MOB_ISNPC);
        g.ch_mut(keeper).mob_rnum = 0;
        for ch in [seller, keeper] { mud_game::handler::char_to_room(g, ch, 0); }
        g.world.rooms[0].room_flags = [0;4];
        g.world.shops = vec![mud_world::model::Shop {
            profit_buy: 1.0, profit_sell: 1.0, close1: 24,
            bitvector: mud_game::shop::HAS_UNLIMITED_CASH,
            type_list: vec![mud_world::model::ShopBuyData { type_: flags::ITEM_TREASURE, keywords: None }],
            in_rooms: vec![g.world.rooms[0].vnum as i32], ..Default::default()
        }];
        g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, ..Default::default() }];
        g.ch_mut(seller).points.gold = starting;
        for _ in 0..2 {
            let mut obj = mud_game::obj::create_obj(); obj.type_flag = flags::ITEM_TREASURE;
            obj.cost = cost; obj.name = Some(b"gem".to_vec()); obj.short_description = Some(b"a gem".to_vec());
            let oid = g.objs.insert(obj); mud_game::handler::obj_to_char(g, oid, seller);
        }
        let cmd = mud_game::interpreter::find_command(g, b"sell").unwrap();
        assert!(mud_game::shop::shop_keeper(g, seller, keeper, cmd, b"2 gem"));
        assert_eq!(g.ch(seller).points.gold, starting + cost * expected_sold);
        assert_eq!(g.ch(seller).carrying.len(), (2 - expected_sold) as usize);
    }
}

fn god_purchase(quest: bool) {
    let mut f = fixture(if quest { "god-quest" } else { "god" }); let g = &mut f.game;
    let buyer = player(g, b"Buyer", 12345); g.character_list.push_back(buyer); let di = descriptor(g, buyer, ConState::Playing);
    g.ch_mut(buyer).level = LVL_IMPL; g.ch_mut(buyer).aff_abils.str_ = 25; g.ch_mut(buyer).aff_abils.dex = 25;
    let keeper = player(g, b"Keeper", 0); g.ch_mut(keeper).act.set(flags::MOB_ISNPC);
    g.ch_mut(keeper).mob_rnum = 0;
    for ch in [buyer, keeper] { mud_game::handler::char_to_room(g, ch, 0); }
    g.world.rooms[0].room_flags = [0;4];
    g.world.shops = vec![mud_world::model::Shop {
        profit_buy: 1.0, profit_sell: 1.0, close1: 24,
        in_rooms: vec![g.world.rooms[0].vnum as i32],
        message_buy: Some(b"%s That costs %d.".to_vec()), ..Default::default()
    }];
    g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, ..Default::default() }];
    for _ in 0..2 {
        let mut obj = mud_game::obj::create_obj(); obj.type_flag = flags::ITEM_TREASURE;
        if quest { obj.extra_flags.set(flags::ITEM_QUEST); }
        obj.cost = 1_500_000_000; obj.name = Some(b"gem".to_vec()); obj.short_description = Some(b"a gem".to_vec());
        let oid = g.objs.insert(obj); mud_game::handler::obj_to_char(g, oid, keeper);
    }
    let cmd = mud_game::interpreter::find_command(g, b"buy").unwrap();
    assert!(mud_game::shop::shop_keeper(g, buyer, keeper, cmd, b"2 gem"));
    assert_eq!(g.ch(buyer).carrying.len(), 2);
    assert_eq!(g.ch(buyer).points.gold, 0);
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(output.contains("3000000000"), "{output}");
}

#[test] fn god_bulk_purchases_can_quote_a_total_above_i32_max() { god_purchase(false); }
#[test] fn god_bulk_quest_purchases_can_quote_a_total_above_i32_max() { god_purchase(true); }
