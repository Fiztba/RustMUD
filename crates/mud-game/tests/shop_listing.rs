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
    let root = std::env::temp_dir().join(format!("rustmud-shoplist-{}-{label}", std::process::id()));
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
fn filtered_stock_includes_last_item_and_quotes_quest_cost() {
    let mut f = fixture("filtered");
    let g = &mut f.game;
    let buyer = player(g, b"Buyer", 12345);
    let di = descriptor(g, buyer, ConState::Playing);
    let keeper = player(g, b"Keeper", 0);
    g.ch_mut(keeper).act.set(flags::MOB_ISNPC);
    g.ch_mut(keeper).mob_rnum = 0;
    for ch in [buyer, keeper] { mud_game::handler::char_to_room(g, ch, 0); }
    g.world.rooms[0].room_flags = [0; 4];
    g.world.shops = vec![mud_world::model::Shop {
        profit_buy: 2.0, close1: 24,
        in_rooms: vec![g.world.rooms[0].vnum as i32],
        ..Default::default()
    }];
    g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, ..Default::default() }];
    let mut obj = mud_game::obj::create_obj();
    obj.cost = 123;
    obj.name = Some(b"gem".to_vec());
    obj.short_description = Some(b"a gleaming gem".to_vec());
    obj.extra_flags.set(flags::ITEM_QUEST);
    let gem = g.objs.insert(obj);
    mud_game::handler::obj_to_char(g, gem, keeper);
    let command = mud_game::interpreter::find_command(g, b"list").unwrap();
    for filter in [b"gem".as_slice(), b""] {
        g.descriptors.get_mut(di).unwrap().output.clear();
        assert!(mud_game::shop::shop_keeper(g, buyer, keeper, command, filter));
        let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(out.contains("A gleaming gem"), "{out}");
        assert!(out.contains("123 qp"), "quest listing must quote purchase cost: {out}");
    }
}
