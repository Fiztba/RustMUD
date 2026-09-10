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
    let root = std::env::temp_dir().join(format!("rustmud-shop-products-{}-{label}", std::process::id()));
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
fn object_insertion_does_not_renumber_products_in_shop_editor() {
    use mud_game::olc::{OlcData, oedit::oedit_save_internally, sedit::*};
    let mut f = fixture("insert");
    let g = &mut f.game;
    g.config.auto_save_olc = false;
    // Isolate the separate missing-board sentinel issue fixed in PR #34.
    g.boards.rnum.fill(0);
    let builder = player(g, b"Builder", 12345);
    g.ch_mut(builder).level = LVL_IMPL;
    let di = descriptor(g, builder, ConState::Sedit);
    let products = vec![g.world.obj_protos[0].vnum as i32, g.world.obj_protos.last().unwrap().vnum as i32];
    g.world.shops[0].producing = products.clone();
    g.shops_rt[0].producing = products.iter().map(|&v| g.world.real_object(v as Idx).unwrap()).collect();
    let mut shop = Box::new(OlcData::default());
    sedit_setup_existing(g, &mut shop, 0);
    shop.number = g.world.shops[0].vnum as i32;
    shop.zone_num = mud_game::dg::mobcmd::real_zone_by_thing(g, shop.number).unwrap() as i32;
    g.olc.insert(di, shop);
    let vnum = (1..NOTHING).find(|&v| g.world.real_object(v).is_none()).unwrap();
    let mut object = OlcData::default();
    object.number = vnum as i32;
    object.obj = Some(Box::new(g.world.obj_protos[0].clone()));
    oedit_save_internally(g, usize::MAX, &mut object);
    assert_eq!(g.olc[&di].shop.as_ref().unwrap().producing, products);
    // Updating an existing object and then inserting another must be equally stable.
    oedit_save_internally(g, usize::MAX, &mut object);
    object.number = (1..NOTHING).find(|&v| g.world.real_object(v).is_none()).unwrap() as i32;
    oedit_save_internally(g, usize::MAX, &mut object);
    assert_eq!(g.olc[&di].shop.as_ref().unwrap().producing, products);
    let produced: Vec<_> = g.shops_rt[0].producing.iter().map(|&r| g.world.obj_protos[r as usize].vnum as i32).collect();
    assert_eq!(produced, products);
    let mut shop = g.olc.remove(&di).unwrap();
    shop.mode = SEDIT_CONFIRM_SAVESTRING;
    assert!(sedit_parse(g, di, shop, b"y").is_none());
    assert_eq!(g.world.shops[0].producing, products);
    let produced: Vec<_> = g.shops_rt[0].producing.iter().map(|&r| g.world.obj_protos[r as usize].vnum as i32).collect();
    assert_eq!(produced, products);
}
