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
    let root = std::env::temp_dir().join(format!("rustmud-numbered-loops-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    let ch = g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: 10, position: POS_STANDING,
        passwd: mud_data::crypt::crypt(b"secret", name).unwrap().to_vec(),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    });
    g.ch_mut(ch).aff_abils.str_ = 18;
    let mut d = Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = ConState::Playing;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    mud_game::handler::char_to_room(g, ch, 0);
    g.rooms[0].light = 1;
    ch
}

fn object(g: &mut Game, keyword: &[u8], short: &str) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(keyword.to_vec()); obj.short_description = Some(short.as_bytes().to_vec());
    obj.wear_flags.set(mud_data::flags::ITEM_WEAR_TAKE);
    obj.weight = 1;
    g.objs.insert(obj)
}

/// Sorted short descriptions of a list of objects.
fn shorts(g: &Game, list: &[mud_data::ids::ObjId]) -> Vec<String> {
    let mut v: Vec<String> = list.iter()
        .map(|&o| String::from_utf8_lossy(g.obj(o).short_description.as_deref().unwrap()).into_owned())
        .collect();
    v.sort();
    v
}

/// Four breads on the floor (obj_to_room appends, so the room lists
/// bread1..bread4 in creation order).
fn breads_on_floor(g: &mut Game) -> Vec<mud_data::ids::ObjId> {
    (1..=4).map(|i| {
        let oid = object(g, b"bread", &format!("bread{i}"));
        mud_game::handler::obj_to_room(g, oid, 0);
        oid
    }).collect()
}

/// Four breads in the inventory (obj_to_char prepends, so the character
/// carries bread4..bread1 in that order).
fn breads_carried(g: &mut Game, ch: mud_data::ids::CharId) -> Vec<mud_data::ids::ObjId> {
    (1..=4).map(|i| {
        let oid = object(g, b"bread", &format!("bread{i}"));
        mud_game::handler::obj_to_char(g, oid, ch);
        oid
    }).collect()
}

#[test]
fn get_count_with_numbered_target_takes_consecutive_matches() {
    let mut f = fixture("get-numbered"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    let breads = breads_on_floor(g);
    mud_game::act::item::do_get(g, ch, b"3 2.bread", 0, 0);
    assert_eq!(g.ch(ch).carry_items, 3);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread2", "bread3", "bread4"]);
    assert_eq!(g.obj(breads[0]).in_room, 0);
}

#[test]
fn get_count_without_number_takes_the_first_matches() {
    let mut f = fixture("get-plain"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    let breads = breads_on_floor(g);
    mud_game::act::item::do_get(g, ch, b"3 bread", 0, 0);
    assert_eq!(g.ch(ch).carry_items, 3);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread1", "bread2", "bread3"]);
    assert_eq!(g.obj(breads[3]).in_room, 0);
}

#[test]
fn get_numbered_target_alone_takes_only_that_item() {
    let mut f = fixture("get-single"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    breads_on_floor(g);
    mud_game::act::item::do_get(g, ch, b"2.bread", 0, 0);
    assert_eq!(g.ch(ch).carry_items, 1);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread2"]);
}

#[test]
fn get_count_from_container_with_numbered_target_takes_consecutive_matches() {
    let mut f = fixture("get-container"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    let bag = object(g, b"bag", "bag");
    g.obj_mut(bag).type_flag = mud_data::flags::ITEM_CONTAINER;
    g.obj_mut(bag).values[0] = 1000;
    mud_game::handler::obj_to_room(g, bag, 0);
    // obj_to_obj prepends: the bag holds bread4..bread1 in that order.
    for i in 1..=4 {
        let oid = object(g, b"bread", &format!("bread{i}"));
        mud_game::handler::obj_to_obj(g, oid, bag);
    }
    mud_game::act::item::do_get(g, ch, b"3 2.bread bag", 0, 0);
    assert_eq!(g.ch(ch).carry_items, 3);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread1", "bread2", "bread3"]);
    assert_eq!(shorts(g, &g.obj(bag).contains), ["bread4"]);
}

#[test]
fn drop_all_dot_numbered_target_drops_the_rest_of_the_matches() {
    let mut f = fixture("drop-alldot"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    breads_carried(g, ch);
    mud_game::act::item::do_drop(g, ch, b"all.2.bread", 0, mud_game::interpreter::SCMD_DROP);
    assert_eq!(g.ch(ch).carry_items, 1);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread4"]);
    assert_eq!(shorts(g, &g.rooms[0].contents), ["bread1", "bread2", "bread3"]);
}

#[test]
fn drop_count_with_numbered_target_drops_consecutive_matches() {
    let mut f = fixture("drop-count"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    breads_carried(g, ch);
    mud_game::act::item::do_drop(g, ch, b"2 2.bread", 0, mud_game::interpreter::SCMD_DROP);
    assert_eq!(g.ch(ch).carry_items, 2);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread1", "bread4"]);
    assert_eq!(shorts(g, &g.rooms[0].contents), ["bread2", "bread3"]);
}

#[test]
fn put_count_with_numbered_target_puts_consecutive_matches() {
    let mut f = fixture("put-count"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    breads_carried(g, ch);
    let bag = object(g, b"bag", "bag");
    g.obj_mut(bag).type_flag = mud_data::flags::ITEM_CONTAINER;
    g.obj_mut(bag).values[0] = 1000;
    mud_game::handler::obj_to_char(g, bag, ch);
    mud_game::act::item::do_put(g, ch, b"2 2.bread bag", 0, 0);
    assert_eq!(shorts(g, &g.obj(bag).contains), ["bread2", "bread3"]);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bag", "bread1", "bread4"]);
}

#[test]
fn give_count_with_numbered_target_gives_consecutive_matches() {
    let mut f = fixture("give-count"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    let vict = player(g, b"Recipient", 12346);
    breads_carried(g, ch);
    mud_game::act::item::do_give(g, ch, b"2 2.bread recipient", 0, 0);
    assert_eq!(g.ch(vict).carry_items, 2);
    assert_eq!(shorts(g, &g.ch(vict).carrying), ["bread2", "bread3"]);
    assert_eq!(shorts(g, &g.ch(ch).carrying), ["bread1", "bread4"]);
}

#[test]
fn wear_all_dot_numbered_target_wears_the_rest_of_the_matches() {
    let mut f = fixture("wear-alldot"); let g = &mut f.game;
    let ch = player(g, b"Actor", 12345);
    // Carried as ring3, ring2, ring1.
    let rings: Vec<_> = (1..=3).map(|i| {
        let oid = object(g, b"ring", &format!("ring{i}"));
        g.obj_mut(oid).wear_flags.set(mud_data::flags::ITEM_WEAR_FINGER);
        mud_game::handler::obj_to_char(g, oid, ch);
        oid
    }).collect();
    mud_game::act::item::do_wear(g, ch, b"all.2.ring", 0, 0);
    let worn = g.ch(ch).equipment.iter().filter(|e| e.is_some()).count();
    assert_eq!(worn, 2);
    assert_eq!(g.obj(rings[0]).worn_by, Some(ch));
    assert_eq!(g.obj(rings[1]).worn_by, Some(ch));
    assert_eq!(g.obj(rings[2]).carried_by, Some(ch));
}
