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
    let root = std::env::temp_dir().join(format!("rustmud-give-transfer-{}-{label}", std::process::id()));
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
fn give_case(phase: u8) {
    let mut f = fixture(&format!("phase-{phase}")); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.ch_mut(actor).level = LVL_IMPL;
    char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let verb = if phase == 0 || phase == 3 { b"gives".as_slice() } else { b"never".as_slice() };
    let body = if phase == 3 { b"drop gem".as_slice() } else { b"mpurge %object%".as_slice() };
    let recipient = listener(g, verb, &[body]);
    let viewer = player(g, b"Viewer", 12346); char_to_room(g, viewer, 0);
    g.ch_mut(viewer).position = POS_STANDING;
    descriptor(g, viewer, ConState::Playing);
    if phase == 1 { listener(g, b"gives", &[b"mpurge %object%"]); }
    g.ch_mut(recipient).name = Some(b"recipient".to_vec());
    let oid = item(g, b"gem"); obj_to_char(g, oid, actor);
    g.obj_mut(oid).item_number = 0;
    g.obj_counts[0] += 1;
    g.world.quests = vec![mud_world::model::Quest {
        vnum: 65000, type_: mud_game::quest::AQ_OBJ_RETURN,
        target: g.world.obj_protos[0].vnum as i32,
        obj_in: mud_game::dg::mob_vnum(g, recipient), obj_out: 1,
        value: 7, next_quest: -1, ..Default::default()
    }];
    mud_game::quest::set_quest(g, actor, 0);
    mud_game::act::item::do_give(g, actor, b"gem recipient", 0, 0);
    assert_eq!(g.ch(actor).ps().current_quest, if phase == 2 { NOTHING } else { 65000 });
    assert_eq!(g.ch(actor).ps().questpoints, if phase == 2 { 7 } else { 0 });
    if phase == 3 { assert_eq!(g.obj(oid).in_room, 0); }
    else { assert!(g.try_obj(oid).is_none()); }
    assert_eq!(g.ch(actor).carry_items, 0);
    assert_eq!(g.ch(recipient).carry_items, 0);
}

#[test]
fn give_survives_recipient_act_purging_the_gift() { give_case(0); }
#[test]
fn give_survives_room_act_purging_the_gift() { give_case(1); }
#[test]
fn normal_give_completes_the_return_quest_once() { give_case(2); }
#[test]
fn a_gift_dropped_by_the_recipient_is_not_consumed_by_the_quest() { give_case(3); }
