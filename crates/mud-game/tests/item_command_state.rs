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
    let root = std::env::temp_dir().join(format!("rustmud-item-state-{}-{label}", std::process::id()));
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
fn pouring_preserves_poison_when_the_source_empties() {
    let mut f = fixture("pour"); let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0); g.rooms[0].light = 1;
    let source = object(g, ch, b"source"); let dest = object(g, ch, b"dest");
    g.obj_mut(source).type_flag = mud_data::flags::ITEM_DRINKCON;
    g.obj_mut(dest).type_flag = mud_data::flags::ITEM_DRINKCON;
    for source_poison in [0, 1] { for dest_poison in [0, 1] { for capacity in [2, 10] {
        g.obj_mut(source).values = [10, 5, 0, source_poison];
        g.obj_mut(dest).values = [capacity, 0, 0, dest_poison];
        let old_weight = g.ch(ch).carry_weight;
        mud_game::act::item::do_pour(g, ch, b"source dest", 0, mud_game::interpreter::SCMD_POUR);
        let amount = capacity.min(5);
        assert_eq!(g.obj(dest).values, [capacity, amount, 0, source_poison | dest_poison]);
        assert_eq!(g.obj(source).values, [10, 5 - amount, 0, if amount == 5 { 0 } else { source_poison }]);
        assert_eq!(g.ch(ch).carry_weight, old_weight);
    } } }
    mud_game::handler::obj_from_char(g, source);
    mud_game::handler::obj_to_room(g, source, 0);
    g.obj_mut(source).type_flag = mud_data::flags::ITEM_FOUNTAIN;
    for capacity in [10, -1] {
        g.obj_mut(source).values = [capacity, 5, 0, 1];
        g.obj_mut(dest).values = [10, 0, 0, 0];
        mud_game::act::item::do_pour(g, ch, b"dest source", 0, mud_game::interpreter::SCMD_FILL);
        assert_eq!(g.obj(dest).values[3], 1);
        assert_eq!(g.obj(dest).values[1], if capacity < 0 { 10 } else { 5 });
    }

}

fn object(g: &mut Game, ch: mud_data::ids::CharId, name: &[u8]) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(name.to_vec()); obj.short_description = Some(name.to_vec());
    obj.weight = 20;
    let oid = g.objs.insert(obj);
    mud_game::handler::obj_to_char(g, oid, ch); oid
}

#[test]
fn wear_all_checks_each_matching_items_level() {
    let mut f = fixture("wear"); let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0); g.rooms[0].light = 1;
    for reverse in [false, true] {
        let first = object(g, ch, b"ring"); let second = object(g, ch, b"ring");
        let (high, low) = if reverse { (second, first) } else { (first, second) };
        for oid in [high, low] { g.obj_mut(oid).wear_flags.set(mud_data::flags::ITEM_WEAR_FINGER); }
        g.obj_mut(high).level = 30; g.obj_mut(low).level = 1;
        mud_game::act::item::do_wear(g, ch, b"all.ring", 0, 0);
        assert_eq!(g.obj(low).worn_by, Some(ch));
        assert_eq!(g.obj(high).carried_by, Some(ch));
        for oid in [high, low] { mud_game::handler::extract_obj(g, oid); }
    }

}

#[test]
fn donations_only_select_configured_existing_rooms() {
    let mut f = fixture("donate"); let g = &mut f.game;
    let ch = player(g, b"Customer", 12345);
    descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0); g.rooms[0].light = 1;
    for mask in 0..8 {
        g.config.donation_room_1 = if mask & 1 != 0 { g.world.rooms[1].vnum as i32 } else { NOWHERE as i32 };
        g.config.donation_room_2 = if mask & 2 != 0 { g.world.rooms[2].vnum as i32 } else { NOWHERE as i32 };
        g.config.donation_room_3 = if mask & 4 != 0 { g.world.rooms[3].vnum as i32 } else { NOWHERE as i32 };
        let mut donated = 0;
        for _ in 0..60 {
            let oid = object(g, ch, b"gift");
            mud_game::act::item::do_drop(g, ch, b"gift", 0, mud_game::interpreter::SCMD_DONATE);
            if let Some(o) = g.try_obj(oid) {
                if mask == 0 { assert_eq!(o.carried_by, Some(ch)); }
                else { assert!(o.in_room >= 1 && o.in_room <= 3, "unexpected room {} for config {}", o.in_room, mask); assert_ne!(mask & (1 << (o.in_room - 1)), 0); donated += 1; }
                mud_game::handler::extract_obj(g, oid);
            }
        }
        if mask != 0 { assert!(donated > 0); }
    }
}
