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
    let root = std::env::temp_dir().join(format!("rustmud-item-actions-{}-{label}", std::process::id()));
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
fn listener(g: &mut Game, verb: &[u8], body: &[&[u8]]) -> mud_data::ids::CharId {
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(mob).script = None;
    g.ch_mut(mob).affected_by = Default::default();
    g.ch_mut(mob).position = POS_STANDING;
    char_to_room(g, mob, 0);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 60000 + nr, name: Some(b"broadcast listener".to_vec()),
        attach_type: dg::MOB_TRIGGER, trigger_type: dg::MTRIG_ACT,
        narg: 1, arglist: Some(verb.to_vec()),
        cmdlist: body.iter().map(|s| s.to_vec()).collect(), ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap(); dg::add_trigger_at(g.ensure_script(GoId::Char(mob)), t, -1);
    mob
}

fn item(g: &mut Game, name: &[u8]) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(name.to_vec()); obj.short_description = Some(name.to_vec());
    obj.wear_flags.set(mud_data::flags::ITEM_WEAR_TAKE);
    g.objs.insert(obj)
}
use mud_data::flags;
use mud_game::interpreter::*;
fn action_case(action: &str) {
    let mut f = fixture(action); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.ch_mut(actor).level = LVL_IMPL;
    char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let verb = match action { "drink" => b"drinks".as_slice(), "sip" => b"sips", "eat" => b"eats", "taste" => b"tastes", "pour" => b"empties",
        "fill" | "fill-source" => b"fills", "wear" => b"wears", _ => b"sacrifices" };
    listener(g, verb, &[if action == "fill-source" { b"mpurge fountain" } else { b"mpurge %object%" }]);
    let oid = item(g, b"item"); obj_to_char(g, oid, actor);
    let mut source = None;
    match action {
        "drink" | "sip" | "pour" => { g.obj_mut(oid).type_flag = flags::ITEM_DRINKCON; g.obj_mut(oid).values = [10, 10, 0, 0]; }
        "eat" | "taste" => { g.obj_mut(oid).type_flag = flags::ITEM_FOOD; g.obj_mut(oid).values[0] = 5; }
        "fill" | "fill-source" => {
            g.obj_mut(oid).type_flag = flags::ITEM_DRINKCON; g.obj_mut(oid).values = [10, 0, 0, 0];
            let fountain = item(g, b"fountain"); g.obj_mut(fountain).type_flag = flags::ITEM_FOUNTAIN;
            g.obj_mut(fountain).values = [100, 100, 0, 0]; obj_to_room(g, fountain, 0); source = Some(fountain);
        }
        "wear" => { g.obj_mut(oid).type_flag = flags::ITEM_ARMOR; g.obj_mut(oid).wear_flags.set(flags::ITEM_WEAR_BODY); }
        _ => {}
    }
    match action {
        "drink" => mud_game::act::item::do_drink(g, actor, b"item", 0, SCMD_DRINK),
        "sip" => mud_game::act::item::do_drink(g, actor, b"item", 0, SCMD_SIP),
        "taste" => mud_game::act::item::do_eat(g, actor, b"item", 0, SCMD_TASTE),
        "eat" => mud_game::act::item::do_eat(g, actor, b"item", 0, SCMD_EAT),
        "pour" => mud_game::act::item::do_pour(g, actor, b"item out", 0, SCMD_POUR),
        "fill" | "fill-source" => mud_game::act::item::do_pour(g, actor, b"item fountain", 0, SCMD_FILL),
        "wear" => mud_game::act::item::do_wear(g, actor, b"item body", 0, 0),
        _ => mud_game::act::item::do_sac(g, actor, b"item", 0, 0),
    }
    if action == "fill-source" {
        assert!(g.try_obj(source.unwrap()).is_none());
        assert_eq!(g.obj(oid).values[1], 0);
    } else { assert!(g.try_obj(oid).is_none(), "listener did not remove item during {action}"); }
    assert!(g.ch(actor).equipment.iter().all(Option::is_none));
}
#[test] fn drink_survives_listener_extraction() { action_case("drink"); }
#[test] fn eat_survives_listener_extraction() { action_case("eat"); }
#[test] fn pour_survives_listener_extraction() { action_case("pour"); }
#[test] fn fill_survives_target_extraction() { action_case("fill"); }
#[test] fn fill_survives_source_extraction() { action_case("fill-source"); }
#[test] fn wear_survives_listener_extraction() { action_case("wear"); }
#[test] fn sacrifice_survives_listener_extraction() { action_case("sac"); }


#[test] fn sip_survives_listener_extraction() { action_case("sip"); }
#[test] fn taste_survives_listener_extraction() { action_case("taste"); }
fn poisoned_case(drink: bool) {
    let mut f = fixture(if drink { "poison-drink" } else { "poison-food" }); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let oid = item(g, b"item");
    g.obj_mut(oid).type_flag = if drink { flags::ITEM_DRINKCON } else { flags::ITEM_FOOD };
    g.obj_mut(oid).values = [5, 5, 0, 1]; obj_to_char(g, oid, actor);
    let uid = dg::obj_script_id(g, oid);
    let purge = format!("mpurge }}{uid}");
    listener(g, if drink { b"chokes" } else { b"coughs" }, &[purge.as_bytes()]);
    if drink { mud_game::act::item::do_drink(g, actor, b"item", 0, SCMD_DRINK); }
    else { mud_game::act::item::do_eat(g, actor, b"item", 0, SCMD_EAT); }
    assert!(g.try_obj(oid).is_none());
    assert!(g.ch(actor).affected_by.is_set(flags::AFF_POISON));
}
#[test] fn poisoned_drink_survives_the_second_callback() { poisoned_case(true); }
#[test] fn poisoned_food_survives_the_second_callback() { poisoned_case(false); }

#[test]
fn callbacks_that_drop_items_cancel_consumption_and_equipping() {
    let mut f = fixture("relocation"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    for action in ["drinks", "eats", "wears"] {
        listener(g, action.as_bytes(), &[b"mforce %actor% drop item"]);
        let oid = item(g, b"item");
        g.obj_mut(oid).type_flag = match action { "drinks" => flags::ITEM_DRINKCON, "eats" => flags::ITEM_FOOD, _ => flags::ITEM_ARMOR };
        g.obj_mut(oid).values = [5, 5, 0, 0];
        g.obj_mut(oid).wear_flags.set(flags::ITEM_WEAR_BODY);
        obj_to_char(g, oid, actor);
        match action {
            "drinks" => mud_game::act::item::do_drink(g, actor, b"item", 0, SCMD_DRINK),
            "eats" => mud_game::act::item::do_eat(g, actor, b"item", 0, SCMD_EAT),
            _ => mud_game::act::item::do_wear(g, actor, b"item body", 0, 0),
        }
        assert_eq!(g.obj(oid).in_room, 0, "{action}");
        assert_eq!(g.obj(oid).values, [5, 5, 0, 0]);
        assert_eq!(g.ch(actor).carry_items, 0);
        assert!(g.ch(actor).equipment.iter().all(Option::is_none));
    }
}

#[test]
fn a_slot_filled_by_the_wear_announcement_does_not_orphan_the_original() {
    let mut f = fixture("occupied-slot"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    listener(g, b"wears", &[b"mforce %actor% wear other body"]);
    let oid = item(g, b"item"); let other = item(g, b"other");
    for object in [oid, other] {
        g.obj_mut(object).type_flag = flags::ITEM_ARMOR;
        g.obj_mut(object).wear_flags.set(flags::ITEM_WEAR_BODY);
        obj_to_char(g, object, actor);
    }
    mud_game::act::item::do_wear(g, actor, b"item body", 0, 0);
    assert_eq!(g.ch(actor).equipment[WEAR_BODY], Some(other));
    assert_eq!(g.obj(oid).carried_by, Some(actor));
    assert!(g.ch(actor).carrying.contains(&oid));
}
