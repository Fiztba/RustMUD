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
    let root = std::env::temp_dir().join(format!("rustmud-item-transfer-{}-{label}", std::process::id()));
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
fn transfer_case(verb: &[u8], input: &[u8], purge_container: bool) {
    let label = format!("{}-{purge_container}", String::from_utf8_lossy(verb));
    let mut f = fixture(&label); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.ch_mut(actor).level = LVL_IMPL;
    char_to_room(g, actor, 0); g.rooms[0].light = 1;
    let _di = descriptor(g, actor, ConState::Playing);
    let mob = listener(g, verb, &[if purge_container { b"mpurge chest" } else { b"mpurge %object%" }]);
    let oid = item(g, b"gem");
    let chest = item(g, b"chest");
    g.obj_mut(chest).type_flag = mud_data::flags::ITEM_CONTAINER;
    g.obj_mut(chest).values[0] = 1000;
    obj_to_room(g, chest, 0);
    if verb == b"gets" { obj_to_room(g, oid, 0); } else { obj_to_char(g, oid, actor); }
    g.rooms[0].people = [mob, actor].into_iter().collect();
    match verb {
        b"puts" => mud_game::act::item::do_put(g, actor, input, 0, 0),
        b"gets" => mud_game::act::item::do_get(g, actor, input, 0, 0),
        b"drops" => mud_game::act::item::do_drop(g, actor, input, 0, mud_game::interpreter::SCMD_DROP),
        _ => unreachable!(),
    }
    assert!(g.try_obj(oid).is_none(), "listener did not purge the item");
    assert!(!g.ch(actor).carrying.contains(&oid));
    assert_eq!(g.ch(actor).carry_items, 0);
}
#[test]
fn get_survives_act_listener_removing_the_item() { transfer_case(b"gets", b"gem", false); }
#[test]
fn drop_survives_act_listener_removing_the_item() { transfer_case(b"drops", b"gem", false); }
#[test]
fn put_survives_act_listener_removing_the_item() { transfer_case(b"puts", b"gem chest", false); }
#[test]
fn put_survives_act_listener_removing_the_container() { transfer_case(b"puts", b"gem chest", true); }
