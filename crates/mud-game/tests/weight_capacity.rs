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
    let root = std::env::temp_dir().join(format!("rustmud-weight-capacity-{}-{label}", std::process::id()));
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

use mud_data::flags;
use mud_game::handler::*;
#[test]
fn large_container_and_item_weights_are_compared_without_overflow() {
    let mut f = fixture("container"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let mut chest = mud_game::obj::create_obj(); chest.name = Some(b"chest".to_vec());
    chest.type_flag = flags::ITEM_CONTAINER; chest.weight = i32::MAX; chest.values[0] = i32::MAX;
    let chest = g.objs.insert(chest); obj_to_room(g, chest, 0);
    let mut obj = mud_game::obj::create_obj(); obj.name = Some(b"gem".to_vec()); obj.weight = 1;
    let item = g.objs.insert(obj); obj_to_char(g, item, actor);
    for (weight, capacity, fits) in [(i32::MAX, i32::MAX, false), (9, 10, true), (10, 10, false), (0, 1, true)] {
        g.obj_mut(chest).weight = weight; g.obj_mut(chest).values[0] = capacity;
        mud_game::act::item::do_put(g, actor, b"gem chest", 0, 0);
        assert_eq!(g.obj(item).in_obj == Some(chest), fits);
        if fits { obj_from_obj(g, item); obj_to_char(g, item, actor); }
        assert_eq!(g.obj(item).carried_by, Some(actor));
        assert!(g.obj(chest).contains.is_empty());
    }
}
#[test]
fn an_unrepresentable_carry_total_is_not_takeable() {
    let mut f = fixture("carry"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    let mut obj = mud_game::obj::create_obj(); obj.name = Some(b"heavy".to_vec()); obj.weight = i32::MAX;
    obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
    let item = g.objs.insert(obj); obj_to_room(g, item, 0);
    g.ch_mut(actor).aff_abils.str_ = 10;
    let capacity = can_carry_w(g.ch(actor));
    for carried in [0, 1, capacity, i32::MAX] {
        for weight in [0, 1, capacity, i32::MAX] {
            g.ch_mut(actor).carry_weight = carried; g.obj_mut(item).weight = weight;
            assert_eq!(mud_game::act::item::can_get_obj(g, actor, item),
                i64::from(carried) + i64::from(weight) <= i64::from(capacity));
        }
    }
}
