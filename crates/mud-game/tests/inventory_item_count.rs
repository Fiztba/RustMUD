use mud_data::types::*;
use mud_game::{game::Game};

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
    let root = std::env::temp_dir().join(format!("rustmud-inventory-count-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn large_npc_inventories_keep_exact_counts_through_transfers() {
    let mut f = fixture("stock"); let g = &mut f.game;
    let first = mud_game::db::read_mobile(g, 0).unwrap();
    let second = mud_game::db::read_mobile(g, 0).unwrap();
    let mut objects = Vec::new();
    for count in 1..=300 {
        let mut obj = mud_game::obj::create_obj(); obj.weight = 1;
        let oid = g.objs.insert(obj);
        mud_game::handler::obj_to_char(g, oid, first); objects.push(oid);
        assert_eq!(g.ch(first).carry_items as usize, count);
        assert_eq!(g.ch(first).carrying.len(), count);
        assert_eq!(g.ch(first).carry_weight as usize, count);
    }
    let worn = objects[0];
    mud_game::handler::obj_from_char(g, worn);
    mud_game::handler::equip_char(g, first, worn, WEAR_HOLD);
    assert_eq!(g.ch(first).carry_items, 299);
    assert_eq!(mud_game::handler::unequip_char(g, first, WEAR_HOLD), Some(worn));
    mud_game::handler::obj_to_char(g, worn, first);
    assert_eq!(g.ch(first).carry_items, 300);
    let contained = objects[1];
    g.obj_mut(worn).type_flag = mud_data::flags::ITEM_CONTAINER;
    g.obj_mut(worn).values[0] = 10;
    mud_game::handler::obj_from_char(g, contained);
    mud_game::handler::obj_to_obj(g, contained, worn);
    assert_eq!(g.ch(first).carry_items, 299);
    mud_game::handler::obj_from_obj(g, contained);
    mud_game::handler::obj_to_char(g, contained, first);
    assert_eq!(g.ch(first).carry_items, 300);
    for (index, &oid) in objects.iter().enumerate() {
        mud_game::handler::obj_from_char(g, oid);
        mud_game::handler::obj_to_char(g, oid, second);
        assert_eq!(g.ch(first).carry_items as usize, 299 - index);
        assert_eq!(g.ch(second).carry_items as usize, index + 1);
    }
    for (index, oid) in objects.into_iter().enumerate() {
        mud_game::handler::extract_obj(g, oid);
        assert_eq!(g.ch(second).carry_items as usize, 299 - index);
        assert_eq!(g.ch(second).carrying.len(), 299 - index);
    }
    assert_eq!(g.ch(first).carry_weight, 0); assert_eq!(g.ch(second).carry_weight, 0);
}
