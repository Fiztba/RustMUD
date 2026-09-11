use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, dg::{self, GoId}, game::Game, handler::*};
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
    let root = std::env::temp_dir().join(format!("rustmud-wizard-object-{}-{label}", std::process::id()));
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

/// An NPC in room 0 whose ACT trigger fires on `verb` and purges the
/// announced object out from under the wizard command.
fn purging_listener(g: &mut Game, verb: &[u8]) -> mud_data::ids::CharId {
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(mob).script = None;
    g.ch_mut(mob).affected_by = Default::default();
    g.ch_mut(mob).position = POS_STANDING;
    char_to_room(g, mob, 0);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 60000 + nr, name: Some(b"purging listener".to_vec()),
        attach_type: dg::MOB_TRIGGER, trigger_type: dg::MTRIG_ACT,
        narg: 1, arglist: Some(verb.to_vec()),
        cmdlist: vec![b"mpurge %object%".to_vec()], ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap();
    dg::add_trigger_at(g.ensure_script(GoId::Char(mob)), t, -1);
    mob
}

fn builder(g: &mut Game) -> (mud_data::ids::CharId, usize) {
    let actor = player(g, b"Builder", 12345);
    g.ch_mut(actor).level = LVL_IMPL;
    char_to_room(g, actor, 0);
    g.rooms[0].light = 1;
    let di = descriptor(g, actor, ConState::Playing);
    (actor, di)
}

fn output(g: &Game, di: usize) -> String {
    String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).into_owned()
}

#[test]
fn purge_survives_a_listener_purging_the_object_first() {
    let mut f = fixture("purge"); let g = &mut f.game;
    let (actor, di) = builder(g);
    purging_listener(g, b"destroys");
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(b"gem".to_vec()); obj.short_description = Some(b"a gem".to_vec());
    let oid = g.objs.insert(obj);
    obj_to_room(g, oid, 0);

    mud_game::act::wizard::do_purge(g, actor, b"gem", 0, 0);

    assert!(g.try_obj(oid).is_none());
    assert!(!g.rooms[0].contents.contains(&oid));
    let ok = String::from_utf8_lossy(&g.config.ok).into_owned();
    assert!(output(g, di).contains(ok.trim()), "{:?}", output(g, di));
}

#[test]
fn load_obj_survives_a_listener_purging_the_new_object() {
    let mut f = fixture("load"); let g = &mut f.game;
    let (actor, di) = builder(g);
    purging_listener(g, b"created");
    let vnum = g.world.obj_protos[0].vnum;
    let before = g.obj_counts[0];

    mud_game::act::wizard::do_load(g, actor, format!("obj {vnum}").as_bytes(), 0, 0);

    assert!(g.ch(actor).carrying.is_empty());
    assert_eq!(g.ch(actor).carry_items, 0);
    assert_eq!(g.obj_counts[0], before);
    assert!(!output(g, di).contains("You create"), "{:?}", output(g, di));
}

#[test]
fn load_obj_keeps_counting_after_a_listener_purges_each_copy() {
    let mut f = fixture("load-count"); let g = &mut f.game;
    let (actor, _) = builder(g);
    purging_listener(g, b"created");
    let viewer = player(g, b"Viewer", 12346);
    char_to_room(g, viewer, 0);
    g.ch_mut(viewer).position = POS_STANDING;
    let vdi = descriptor(g, viewer, ConState::Playing);
    let vnum = g.world.obj_protos[0].vnum;

    mud_game::act::wizard::do_load(g, actor, format!("obj {vnum} 2").as_bytes(), 0, 0);

    assert!(g.ch(actor).carrying.is_empty());
    assert_eq!(output(g, vdi).matches("Builder has created").count(), 2, "{:?}", output(g, vdi));
}

#[test]
fn load_obj_without_a_purging_listener_still_creates_every_copy() {
    let mut f = fixture("load-plain"); let g = &mut f.game;
    let (actor, di) = builder(g);
    let vnum = g.world.obj_protos[0].vnum;

    mud_game::act::wizard::do_load(g, actor, format!("obj {vnum} 2").as_bytes(), 0, 0);

    assert_eq!(g.ch(actor).carrying.len(), 2);
    assert_eq!(output(g, di).matches("You create").count(), 2, "{:?}", output(g, di));
}
