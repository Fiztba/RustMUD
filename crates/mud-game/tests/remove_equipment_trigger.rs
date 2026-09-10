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
    let root = std::env::temp_dir().join(format!("rustmud-remove-slot-{}-{label}", std::process::id()));
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

use mud_game::{dg::{self, GoId}, handler::*};
fn item(g: &mut Game, name: &[u8]) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(name.to_vec()); obj.short_description = Some(name.to_vec());
    obj.wear_flags.set(mud_data::flags::ITEM_WEAR_TAKE);
    g.objs.insert(obj)
}
fn remove_case(scenario: &str) {
    let mut f = fixture(scenario); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345);
    g.character_list.push_back(actor);
    char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let original = item(g, b"original");
    let replacement = item(g, b"replacement");
    for oid in [original, replacement] {
        g.obj_mut(oid).wear_flags.set(mud_data::flags::ITEM_WEAR_BODY);
        g.obj_mut(oid).type_flag = mud_data::flags::ITEM_ARMOR;
    }
    equip_char(g, actor, original, WEAR_BODY);
    obj_to_char(g, replacement, actor);
    let nr = g.world.triggers.len() as u16;
    let cmds = match scenario {
        "replacement" => vec![format!("omove {}", g.world.rooms[0].vnum).into_bytes(),
            b"oforce %actor% wear replacement body".to_vec(), b"return 1".to_vec()],
        "move" => vec![format!("omove {}", g.world.rooms[1].vnum).into_bytes(), b"return 1".to_vec()],
        "purge" => vec![b"opurge self".to_vec()],
        "deny" => vec![b"return 0".to_vec()],
        _ => vec![b"return 1".to_vec()],
    };
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: dg::OBJ_TRIGGER, trigger_type: dg::OTRIG_REMOVE,
        narg: 100, cmdlist: cmds,
        ..Default::default()
    });
    let trigger = dg::read_trigger(g, nr).unwrap();
    dg::add_trigger_at(g.ensure_script(GoId::Obj(original)), trigger, -1);
    mud_game::act::item::do_remove(g, actor, b"original", 0, 0);
    if scenario == "replacement" {
    assert_eq!(g.obj(original).in_room, 0);
    assert_eq!(g.ch(actor).equipment[WEAR_BODY], Some(replacement));
    assert_eq!(g.obj(replacement).worn_by, Some(actor));
    assert_eq!(g.ch(actor).carry_items, 0);
    } else if scenario == "deny" {
        assert_eq!(g.ch(actor).equipment[WEAR_BODY], Some(original));
        assert_eq!(g.ch(actor).carry_items, 1);
    } else {
        assert_eq!(g.ch(actor).equipment[WEAR_BODY], None);
        assert_eq!(g.obj(replacement).carried_by, Some(actor));
        match scenario {
            "purge" => assert!(g.try_obj(original).is_none()),
            "move" => assert_eq!(g.obj(original).in_room, 1),
            _ => assert_eq!(g.obj(original).carried_by, Some(actor)),
        }
        assert_eq!(g.ch(actor).carry_items, if scenario == "normal" { 2 } else { 1 });
    }
}
#[test]
fn replacement_remains_equipped() { remove_case("replacement"); }
#[test]
fn relocated_item_stays_in_its_destination() { remove_case("move"); }
#[test]
fn purged_item_stops_removal() { remove_case("purge"); }
#[test]
fn denied_removal_keeps_equipment() { remove_case("deny"); }
#[test]
fn normal_removal_puts_item_in_inventory() { remove_case("normal"); }
