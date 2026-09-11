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
    let root = std::env::temp_dir().join(format!("rustmud-item-count-{}-{label}", std::process::id()));
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

fn bread(g: &mut Game) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(b"bread".to_vec());
    obj.short_description = Some(b"a loaf of bread".to_vec());
    obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
    g.objs.insert(obj)
}

fn bag(g: &mut Game) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(b"bag".to_vec());
    obj.short_description = Some(b"a bag".to_vec());
    obj.type_flag = flags::ITEM_CONTAINER;
    obj.values[0] = 100;
    obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
    g.objs.insert(obj)
}

fn output(g: &mut Game, di: usize) -> String {
    let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).into_owned();
    g.descriptors.get_mut(di).unwrap().output.clear();
    out
}

const REFUSAL: &str = "Yeah, that makes sense.";

#[test]
fn get_with_minimum_count_refuses_instead_of_overflowing() {
    let mut f = fixture("get-min"); let g = &mut f.game;
    let actor = player(g, b"Getter", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::handler::char_to_room(g, actor, 0);
    for _ in 0..2 { let b = bread(g); mud_game::handler::obj_to_room(g, b, 0); }
    output(g, di);
    mud_game::act::item::do_get(g, actor, b"-2147483648 bread", 0, 0);
    let out = output(g, di);
    assert_eq!(g.ch(actor).carry_items, 0, "{out}");
    assert!(out.contains(REFUSAL), "{out}");
}

#[test]
fn get_with_negative_count_refuses() {
    let mut f = fixture("get-neg"); let g = &mut f.game;
    let actor = player(g, b"Getter", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::handler::char_to_room(g, actor, 0);
    for _ in 0..2 { let b = bread(g); mud_game::handler::obj_to_room(g, b, 0); }
    output(g, di);
    mud_game::act::item::do_get(g, actor, b"-1 bread", 0, 0);
    let out = output(g, di);
    assert_eq!(g.ch(actor).carry_items, 0, "{out}");
    assert!(out.contains(REFUSAL), "{out}");
}

#[test]
fn put_with_negative_count_refuses() {
    let mut f = fixture("put-neg"); let g = &mut f.game;
    let actor = player(g, b"Putter", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::handler::char_to_room(g, actor, 0);
    let sack = bag(g); mud_game::handler::obj_to_char(g, sack, actor);
    for _ in 0..2 { let b = bread(g); mud_game::handler::obj_to_char(g, b, actor); }
    output(g, di);
    mud_game::act::item::do_put(g, actor, b"-1 bread bag", 0, 0);
    let out = output(g, di);
    assert!(g.obj(sack).contains.is_empty(), "{out}");
    assert_eq!(g.ch(actor).carry_items, 3, "{out}");
    assert!(out.contains(REFUSAL), "{out}");
}

#[test]
fn give_with_negative_count_refuses() {
    let mut f = fixture("give-neg"); let g = &mut f.game;
    let actor = player(g, b"Giver", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    let vict = player(g, b"Taker", 12346);
    for ch in [actor, vict] { mud_game::handler::char_to_room(g, ch, 0); }
    for _ in 0..2 { let b = bread(g); mud_game::handler::obj_to_char(g, b, actor); }
    output(g, di);
    mud_game::act::item::do_give(g, actor, b"-1 bread taker", 0, 0);
    let out = output(g, di);
    assert_eq!(g.ch(actor).carry_items, 2, "{out}");
    assert_eq!(g.ch(vict).carry_items, 0, "{out}");
    assert!(out.contains(REFUSAL), "{out}");
}

#[test]
fn get_with_positive_count_still_takes_that_many() {
    let mut f = fixture("get-two"); let g = &mut f.game;
    let actor = player(g, b"Getter", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::handler::char_to_room(g, actor, 0);
    for _ in 0..3 { let b = bread(g); mud_game::handler::obj_to_room(g, b, 0); }
    output(g, di);
    mud_game::act::item::do_get(g, actor, b"2 bread", 0, 0);
    let out = output(g, di);
    assert_eq!(g.ch(actor).carry_items, 2, "{out}");
    assert!(!out.contains(REFUSAL), "{out}");
}

#[test]
fn get_from_container_with_negative_count_refuses() {
    let mut f = fixture("get-cont-neg"); let g = &mut f.game;
    let actor = player(g, b"Getter", 12345);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::handler::char_to_room(g, actor, 0);
    let sack = bag(g); mud_game::handler::obj_to_char(g, sack, actor);
    for _ in 0..2 { let b = bread(g); mud_game::handler::obj_to_obj(g, b, sack); }
    output(g, di);
    mud_game::act::item::do_get(g, actor, b"-1 bread bag", 0, 0);
    let out = output(g, di);
    assert_eq!(g.obj(sack).contains.len(), 2, "{out}");
    assert_eq!(g.ch(actor).carry_items, 1, "{out}");
    assert!(out.contains(REFUSAL), "{out}");
}
