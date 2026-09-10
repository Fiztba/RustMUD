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
    let root = std::env::temp_dir().join(format!("rustmud-shop-produced-{}-{label}", std::process::id()));
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
fn selling_a_produced_container_does_not_delete_its_contents() {
    let mut f = fixture("container"); let g = &mut f.game;
    let seller = player(g, b"Seller", 12345); descriptor(g, seller, ConState::Playing);
    let keeper = player(g, b"Keeper", 0); g.ch_mut(keeper).act.set(flags::MOB_ISNPC);
    g.ch_mut(keeper).mob_rnum = 0; g.ch_mut(keeper).points.gold = 1000;
    for ch in [seller, keeper] { mud_game::handler::char_to_room(g, ch, 0); }
    g.world.rooms[0].room_flags = [0;4];
    let proto = &mut g.world.obj_protos[0];
    proto.name = Some(b"bag".to_vec()); proto.short_description = Some(b"a bag".to_vec());
    proto.type_flag = flags::ITEM_CONTAINER; proto.cost = 100; proto.extra_flags = [0;4];
    proto.proto_script.clear();
    g.world.shops = vec![mud_world::model::Shop {
        profit_buy: 1.0, profit_sell: 1.0, close1: 24,
        type_list: vec![mud_world::model::ShopBuyData { type_: flags::ITEM_CONTAINER, keywords: None }],
        in_rooms: vec![g.world.rooms[0].vnum as i32], ..Default::default()
    }];
    g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, producing: vec![0], ..Default::default() }];
    let bag = mud_game::db::read_object(g, 0).unwrap(); mud_game::handler::obj_to_char(g, bag, seller);
    let child = g.objs.insert(mud_game::obj::create_obj()); mud_game::handler::obj_to_obj(g, child, bag);
    let cmd = mud_game::interpreter::find_command(g, b"sell").unwrap();
    assert!(mud_game::shop::shop_keeper(g, seller, keeper, cmd, b"bag"));
    assert!(g.try_obj(child).is_some(), "selling a bag must not delete its contents");
    assert_eq!(g.obj(bag).carried_by, Some(keeper));
    assert_eq!(g.obj(child).in_obj, Some(bag));
}
