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
    let root = std::env::temp_dir().join(format!("rustmud-put-container-{}-{label}", std::process::id()));
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

use mud_data::flags;
use mud_game::{dg::{self, GoId}, handler::*};
fn put_case(redirect: bool, close: bool) {
    let mut f = fixture(&format!("{redirect}-{close}")); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.character_list.push_back(actor);
    char_to_room(g, actor, 1); g.rooms[1].light = 1; descriptor(g, actor, ConState::Playing);
    g.world.rooms[2].room_flags = [0;4];
    let mut chest = mud_game::obj::create_obj(); chest.name = Some(b"chest".to_vec());
    chest.type_flag = flags::ITEM_CONTAINER; chest.values[1] = flags::CONT_CLOSEABLE;
    let chest = g.objs.insert(chest); obj_to_room(g, chest, 1);
    let mut items = vec![];
    for _ in 0..2 {
        let mut obj = mud_game::obj::create_obj(); obj.name = Some(b"gem".to_vec());
        let item = g.objs.insert(obj); obj_to_char(g, item, actor); items.push(item);
    }
    let mut cmds = vec![];
    if redirect { cmds.push(format!("oteleport %actor% {}", g.world.rooms[2].vnum).into_bytes()); }
    if close { cmds.push(b"oforce %actor% close chest".to_vec()); }
    cmds.push(b"return 1".to_vec());
    for &item in &items {
        let nr = g.world.triggers.len() as u16;
        g.world.triggers.push(mud_world::model::Trigger {
            vnum: 64000 + nr, attach_type: dg::OBJ_TRIGGER, trigger_type: dg::OTRIG_DROP,
            narg: 100, cmdlist: cmds.clone(), ..Default::default()
        });
        let t = dg::read_trigger(g, nr).unwrap(); dg::add_trigger_at(g.ensure_script(GoId::Obj(item)), t, -1);
    }
    mud_game::act::item::do_put(g, actor, b"all.gem chest", 0, 0);
    assert_eq!(g.ch(actor).in_room, if redirect { 2 } else { 1 });
    if close { assert_ne!(g.obj(chest).values[1] & flags::CONT_CLOSED, 0); }
    for item in items {
        if redirect || close { assert_eq!(g.obj(item).carried_by, Some(actor)); }
        else { assert_eq!(g.obj(item).in_obj, Some(chest)); }
    }
}
#[test] fn redirected_actor_does_not_put_items_in_remote_container() { put_case(true, false); }
#[test] fn closed_container_does_not_receive_items_after_trigger() { put_case(false, true); }
#[test] fn unchanged_container_receives_all_items() { put_case(false, false); }
