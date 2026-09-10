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
    let root = std::env::temp_dir().join(format!("rustmud-act-broadcast-{}-{label}", std::process::id()));
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

use mud_game::{comm::{self, ActArg}, dg::{self, GoId}, handler::*};
fn listener(g: &mut Game, body: &[&[u8]]) -> mud_data::ids::CharId {
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(mob).script = None;
    g.ch_mut(mob).affected_by = Default::default();
    g.ch_mut(mob).position = POS_STANDING;
    char_to_room(g, mob, 0);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 60000 + nr, name: Some(b"broadcast listener".to_vec()),
        attach_type: dg::MOB_TRIGGER, trigger_type: dg::MTRIG_ACT,
        narg: 1, arglist: Some(b"holds".to_vec()),
        cmdlist: body.iter().map(|s| s.to_vec()).collect(), ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap(); dg::add_trigger_at(g.ensure_script(GoId::Char(mob)), t, -1);
    mob
}
#[test]
fn broadcast_survives_an_object_purged_by_its_first_listener() {
    let mut f = fixture("purge"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); let viewer = player(g, b"Viewer", 12346);
    for ch in [actor, viewer] { g.ch_mut(ch).position = POS_STANDING; char_to_room(g, ch, 0); }
    let di = descriptor(g, viewer, ConState::Playing); g.rooms[0].light = 1;
    let first = listener(g, &[b"mpurge gem"]);
    let second = listener(g, &[b"set received yes", b"global received"]);
    g.rooms[0].people = [first, second, viewer, actor].into_iter().collect();
    let mut o = mud_game::obj::create_obj(); o.name = Some(b"gem".to_vec()); o.short_description = Some(b"a gem".to_vec());
    let oid = g.objs.insert(o); obj_to_room(g, oid, 0);
    comm::act_full(g, b"$n holds $p.", false, Some(actor), Some(oid), ActArg::None, comm::TO_ROOM);
    assert!(g.try_obj(oid).is_none());
    assert!(String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains("Actor holds a gem."));
    assert!(g.script_of(GoId::Char(second)).unwrap().global_vars.iter().any(|v| v.name == b"received" && v.value == b"yes"));
}
#[test]
fn nested_say_does_not_disable_remaining_broadcast_triggers() {
    let mut f = fixture("nested"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    let first = listener(g, &[b"say hello"]);
    let second = listener(g, &[b"set received yes", b"global received"]);
    g.rooms[0].people = [first, second, actor].into_iter().collect();
    g.dg_act_check = true;
    comm::act(g, b"$n holds something.", false, Some(actor), None, None, comm::TO_ROOM);
    assert!(g.script_of(GoId::Char(second)).unwrap().global_vars.iter().any(|v| v.name == b"received"));
    assert!(g.dg_act_check);
}


