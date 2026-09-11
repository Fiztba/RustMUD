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
    let root = std::env::temp_dir().join(format!("rustmud-announce-lifetime-{}-{label}", std::process::id()));
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
        vnum: 60000 + nr, name: Some(b"announcement listener".to_vec()),
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
    let oid = g.objs.insert(obj);
    g.object_list.push_front(oid);
    oid
}

/// A viewer stands first in room 0 (so the decay TO_CHAR goes to them)
/// while a listener mob reacts to the TO_ROOM announcement.
fn corpse_case(verb: &[u8]) {
    let mut f = fixture(&format!("corpse-{}", String::from_utf8_lossy(verb))); let g = &mut f.game;
    let viewer = player(g, b"Viewer", 12345);
    g.ch_mut(viewer).position = POS_STANDING; char_to_room(g, viewer, 0);
    descriptor(g, viewer, ConState::Playing); g.rooms[0].light = 1;
    let mob = listener(g, verb, &[b"mpurge corpse"]);
    g.rooms[0].people = [viewer, mob].into_iter().collect();

    let corpse = item(g, b"corpse");
    g.obj_mut(corpse).type_flag = mud_data::flags::ITEM_CONTAINER;
    g.obj_mut(corpse).values[3] = 1;
    g.obj_mut(corpse).timer = 1;
    obj_to_room(g, corpse, 0);
    let loot = item(g, b"gem");
    obj_to_obj(g, loot, corpse);

    mud_game::limits::point_update(g);

    assert!(g.try_obj(corpse).is_none(), "the corpse should be gone");
    if verb == b"consumes" {
        assert!(g.try_obj(loot).is_none(), "the purged corpse takes its contents with it");
    } else {
        assert_eq!(g.obj(loot).in_room, 0, "a decayed corpse dumps its contents on the floor");
        assert!(g.rooms[0].contents.contains(&loot));
    }
}

#[test]
fn corpse_decay_survives_a_listener_purging_the_corpse() { corpse_case(b"consumes"); }
#[test]
fn corpse_decay_dumps_contents_when_no_trigger_fires() { corpse_case(b"never"); }

fn create_food_case(verb: &[u8]) {
    let mut f = fixture(&format!("food-{}", String::from_utf8_lossy(verb))); let g = &mut f.game;
    let caster = player(g, b"Caster", 12346);
    g.ch_mut(caster).position = POS_STANDING; char_to_room(g, caster, 0);
    descriptor(g, caster, ConState::Playing); g.rooms[0].light = 1;
    listener(g, verb, &[b"mpurge waybread"]);
    let waybread = g.world.real_object(10).unwrap();
    let before = g.obj_counts[waybread as usize];

    mud_game::magic::mag_creations(g, 10, caster, mud_data::spells::SPELL_CREATE_FOOD);

    let bread = g.ch(caster).carrying.iter().filter(|&&o| g.obj(o).item_number == waybread).count();
    if verb == b"creates" {
        assert_eq!(bread, 0, "the purged waybread should be gone");
        assert_eq!(g.ch(caster).carry_items, 0);
        assert_eq!(g.obj_counts[waybread as usize], before);
    } else {
        assert_eq!(bread, 1, "create food hands the caster one waybread");
        assert_eq!(g.ch(caster).carry_items, 1);
        assert_eq!(g.obj_counts[waybread as usize], before + 1);
    }
}

#[test]
fn create_food_survives_a_listener_purging_the_food() { create_food_case(b"creates"); }
#[test]
fn create_food_hands_over_the_food_when_no_trigger_fires() { create_food_case(b"never"); }
