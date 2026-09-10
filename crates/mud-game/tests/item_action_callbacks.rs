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
    let verb = match action { "drink" => b"drinks".as_slice(), "eat" => b"eats", "pour" => b"empties",
        "fill" | "fill-source" => b"fills", "wear" => b"wears", _ => b"sacrifices" };
    listener(g, verb, &[if action == "fill-source" { b"mpurge fountain" } else { b"mpurge %object%" }]);
    let oid = item(g, b"item"); obj_to_char(g, oid, actor);
    let mut source = None;
    match action {
        "drink" | "pour" => { g.obj_mut(oid).type_flag = flags::ITEM_DRINKCON; g.obj_mut(oid).values = [10, 10, 0, 0]; }
        "eat" => { g.obj_mut(oid).type_flag = flags::ITEM_FOOD; g.obj_mut(oid).values[0] = 5; }
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
