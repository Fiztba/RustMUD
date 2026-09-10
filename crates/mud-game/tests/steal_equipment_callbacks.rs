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
    let root = std::env::temp_dir().join(format!("rustmud-steal-equip-{}-{label}", std::process::id()));
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
#[test]
fn theft_does_not_take_replacement_equipment_after_an_act_trigger() {
    let mut f = fixture("replacement"); let g = &mut f.game;
    let thief = player(g, b"Thief", 12345);
    let victim = player(g, b"Victim", 12346);
    for ch in [thief, victim] { char_to_room(g, ch, 0); g.character_list.push_back(ch); }
    descriptor(g, thief, ConState::Playing);
    g.ch_mut(thief).set_skill(mud_data::spells::SKILL_STEAL, 100);
    g.ch_mut(victim).position = POS_STUNNED;
    g.config.pt_setting = 2;
    g.world.rooms[0].room_flags = [0; 4]; g.rooms[0].light = 1;
    let original = item(g, b"original");
    let replacement = item(g, b"replacement");
    for oid in [original, replacement] {
        g.object_list.push_back(oid);
        g.obj_mut(oid).wear_flags.set(mud_data::flags::ITEM_WEAR_BODY);
        g.obj_mut(oid).type_flag = mud_data::flags::ITEM_ARMOR;
    }
    equip_char(g, victim, original, WEAR_BODY);
    obj_to_char(g, replacement, victim);
    listener(g, b"steals", &[b"mpurge %object%", b"eval awake %victim.pos(standing)%", b"mforce %victim% wear replacement body"]);
    mud_game::act::other::do_steal(g, thief, b"original Victim", 0, 0);
    assert!(g.try_obj(original).is_none());
    assert_eq!(g.ch(victim).equipment[WEAR_BODY], Some(replacement));
    assert_eq!(g.obj(replacement).worn_by, Some(victim));
    assert_eq!(g.ch(thief).carry_items, 0);
}
