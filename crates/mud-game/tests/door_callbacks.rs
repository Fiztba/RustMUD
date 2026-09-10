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
    let root = std::env::temp_dir().join(format!("rustmud-door-actions-{}-{label}", std::process::id()));
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
        attach_type: dg::MOB_TRIGGER, trigger_type: dg::MTRIG_DOOR,
        narg: 100, arglist: Some(verb.to_vec()),
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

#[test]
fn container_removed_by_door_trigger_stops_open() {
    let mut f = fixture("purge"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345);
    char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let oid = item(g, b"chest");
    g.obj_mut(oid).type_flag = flags::ITEM_CONTAINER;
    g.obj_mut(oid).values[1] = flags::CONT_CLOSEABLE | flags::CONT_CLOSED;
    obj_to_room(g, oid, 0);
    listener(g, b"", &[b"mpurge chest", b"return 1"]);
    mud_game::act::movement::do_gen_door(g, actor, b"chest", 0, SCMD_OPEN);
    assert!(g.try_obj(oid).is_none());
}

fn door_case(world: bool, exit: bool, redirect: bool, deny: bool, autokey: bool) {
    let mut f = fixture(&format!("{world}-{exit}-{redirect}-{deny}-{autokey}")); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.character_list.push_back(actor);
    char_to_room(g, actor, 1); g.rooms[1].light = 1;
    descriptor(g, actor, ConState::Playing);
    g.ch_mut(actor).ps_mut().pref.set(flags::PRF_AUTOKEY);
    let key = mud_game::db::read_object(g, 0).unwrap(); obj_to_char(g, key, actor);
    let key_vnum = g.world.obj_protos[0].vnum;
    let oid = item(g, b"chest");
    g.obj_mut(oid).type_flag = flags::ITEM_CONTAINER;
    g.obj_mut(oid).values[1] = flags::CONT_CLOSEABLE | flags::CONT_CLOSED | if autokey { flags::CONT_LOCKED } else { 0 };
    g.obj_mut(oid).values[2] = key_vnum as i32;
    obj_to_room(g, oid, 1);
    let info = flags::EX_ISDOOR | flags::EX_CLOSED | if autokey { flags::EX_LOCKED } else { 0 };
    g.world.rooms[1].dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
        general_description: None, keyword: Some(b"gate".to_vec()), exit_info: info, key: key_vnum,
        to_room_vnum: g.world.rooms[2].vnum as i32, to_room: 2,
    }));
    let prefix = if world { "w" } else { "m" };
    let mut cmds = vec![];
    if redirect { cmds.push(format!("{prefix}teleport %actor% {}", g.world.rooms[2].vnum).into_bytes()); }
    if deny { cmds.push(b"if %cmd% == unlock".to_vec()); cmds.push(b"return 0".to_vec()); cmds.push(b"else".to_vec()); }
    cmds.push(b"return 1".to_vec());
    if deny { cmds.push(b"end".to_vec()); }
    let go = if world { GoId::Room(1) } else {
        let mob = listener(g, b"", &[]); char_from_room(g, mob); char_to_room(g, mob, 1);
        g.ch_mut(mob).script = None; GoId::Char(mob)
    };
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 64000, attach_type: if world { dg::WLD_TRIGGER } else { dg::MOB_TRIGGER },
        trigger_type: if world { dg::WTRIG_DOOR } else { dg::MTRIG_DOOR },
        narg: 100, cmdlist: cmds, ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap(); dg::add_trigger_at(g.ensure_script(go), t, -1);
    mud_game::act::movement::do_gen_door(g, actor, if exit { b"gate north" } else { b"chest" }, 0, SCMD_OPEN);
    assert_eq!(g.ch(actor).in_room, if redirect { 2 } else { 1 });
    let closed = redirect || (deny && autokey);
    if exit {
        assert_eq!(g.world.rooms[1].dir_option[NORTH].as_ref().unwrap().exit_info & flags::EX_CLOSED != 0, closed);
    } else { assert_eq!(g.obj(oid).values[1] & flags::CONT_CLOSED != 0, closed); }
}
#[test] fn mob_redirect_stops_container_command() { door_case(false, false, true, false, false); }
#[test] fn world_redirect_stops_exit_command() { door_case(true, true, true, false, false); }
#[test] fn denied_unlock_does_not_open_container() { door_case(false, false, false, true, true); }
#[test] fn denied_unlock_does_not_open_exit() { door_case(true, true, false, true, true); }
#[test] fn allowed_autokey_opens_container() { door_case(false, false, false, false, true); }
#[test] fn allowed_autokey_opens_exit() { door_case(true, true, false, false, true); }
#[test] fn ordinary_open_container() { door_case(true, false, false, false, false); }
#[test] fn ordinary_open_exit() { door_case(false, true, false, false, false); }
