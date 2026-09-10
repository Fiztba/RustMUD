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
    let root = std::env::temp_dir().join(format!("rustmud-fill-location-{}-{label}", std::process::id()));
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
fn fill_case(redirect: bool) {
    let mut f = fixture(if redirect { "redirect" } else { "normal" }); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.character_list.push_back(actor);
    char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    g.rooms[0].light = 1; g.world.rooms[2].room_flags = [0;4];
    if redirect {
        let body = format!("mteleport %actor% {}", g.world.rooms[2].vnum);
        listener(g, b"fills", &[body.as_bytes()]);
    }
    let bottle = item(g, b"bottle"); g.obj_mut(bottle).type_flag = flags::ITEM_DRINKCON;
    g.obj_mut(bottle).values = [10, 0, 0, 0]; obj_to_char(g, bottle, actor);
    let fountain = item(g, b"fountain"); g.obj_mut(fountain).type_flag = flags::ITEM_FOUNTAIN;
    g.obj_mut(fountain).values = [100, 100, 0, 0]; obj_to_room(g, fountain, 0);
    mud_game::act::item::do_pour(g, actor, b"bottle fountain", 0, SCMD_FILL);
    assert_eq!(g.ch(actor).in_room, if redirect { 2 } else { 0 });
    assert_eq!(g.obj(bottle).values[1], if redirect { 0 } else { 10 });
    assert_eq!(g.obj(fountain).values[1], if redirect { 100 } else { 90 });
}
#[test] fn filling_stops_when_the_actor_leaves_the_fountain() { fill_case(true); }
#[test] fn filling_at_the_fountain_still_transfers_liquid() { fill_case(false); }
