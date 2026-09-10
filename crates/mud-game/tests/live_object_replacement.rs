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
    let root = std::env::temp_dir().join(format!("rustmud-object-replacement-{}-{label}", std::process::id()));
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

fn prepare_proto(g: &mut Game, rnum: usize, weight: i32) {
    let p = &mut g.world.obj_protos[rnum]; p.weight = weight; p.type_flag = mud_data::flags::ITEM_CONTAINER;
    p.values = [100, 0, 0, 0]; p.extra_flags = [0; 4]; p.perm_affects = [0; 4]; p.affected = Default::default(); p.proto_script.clear();
}

#[test]
fn transformation_updates_counts_and_keeps_contents_weight() {
    let mut f = fixture("transform"); let g = &mut f.game;
    let ch = player(g, b"Carrier", 12345);
    prepare_proto(g, 0, 2); prepare_proto(g, 1, 5);
    let item = mud_game::db::read_object(g, 0).unwrap();
    let mut child = mud_game::obj::create_obj(); child.weight = 7; let child = g.objs.insert(child);
    mud_game::handler::obj_to_obj(g, child, item); mud_game::handler::obj_to_char(g, item, ch);
    let old_count = g.obj_counts[0]; let new_count = g.obj_counts[1];
    let command = format!("otransform {}", g.world.obj_protos[1].vnum);
    mud_game::dg::objcmd::obj_command_interpreter(g, item, command.as_bytes());
    assert_eq!(g.obj(item).item_number, 1);
    assert_eq!(g.obj(item).weight, 12);
    assert_eq!(g.ch(ch).carry_weight, 12);
    assert_eq!(g.obj(item).contains, vec![child]); assert_eq!(g.obj(child).in_obj, Some(item));
    assert_eq!(g.obj_counts[0], old_count - 1); assert_eq!(g.obj_counts[1], new_count + 1);
    mud_game::handler::extract_obj(g, item);
    assert_eq!(g.obj_counts[1], new_count); assert_eq!(g.ch(ch).carry_weight, 0);
}

#[test]
fn editing_a_worn_object_replaces_its_effects() {
    use mud_data::flags;
    let mut f = fixture("worn"); let g = &mut f.game;
    let ch = player(g, b"Wearer", 12345); mud_game::handler::char_to_room(g, ch, 0);
    prepare_proto(g, 0, 2);
    g.world.obj_protos[0].affected[0] = mud_world::model::ObjAffect { location: flags::APPLY_HIT, modifier: 10 };
    let item = mud_game::db::read_object(g, 0).unwrap(); g.ch_mut(ch).points.max_hit = 100;
    assert!(mud_game::handler::equip_char(g, ch, item, WEAR_HOLD));
    assert_eq!(g.ch(ch).points.max_hit, 110);
    let mut proto = g.world.obj_protos[0].clone(); proto.weight = 5; proto.affected[0].modifier = 30;
    mud_game::olc::genobj::add_object(g, &proto, proto.vnum);
    assert_eq!(g.ch(ch).points.max_hit, 130);
    assert_eq!(g.obj(item).worn_by, Some(ch));
    mud_game::handler::unequip_char(g, ch, WEAR_HOLD);
    assert_eq!(g.ch(ch).points.max_hit, 100);
}

#[test]
fn editing_an_object_cancels_its_old_waiting_triggers() {
    use mud_game::{dg::{self, GoId}, game::EventKind};
    let mut f = fixture("wait"); let g = &mut f.game;
    prepare_proto(g, 0, 2);
    let item = mud_game::db::read_object(g, 0).unwrap();
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger { vnum: 65000, attach_type: dg::OBJ_TRIGGER, ..Default::default() });
    let mut trigger = dg::read_trigger(g, nr).unwrap(); let iid = trigger.iid; trigger.wait_event = Some(98765);
    dg::add_trigger_at(g.ensure_script(GoId::Obj(item)), trigger, -1);
    g.queue_event(10, EventKind::TrigWait { go: GoId::Obj(item), iid, event_id: 98765 });
    let proto = g.world.obj_protos[0].clone();
    mud_game::olc::genobj::add_object(g, &proto, proto.vnum);
    assert!(!g.events.iter().any(|ev| matches!(ev.kind, EventKind::TrigWait { event_id: 98765, .. })));
}
