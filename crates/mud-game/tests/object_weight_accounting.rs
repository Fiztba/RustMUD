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
    let root = std::env::temp_dir().join(format!("rustmud-weight-accounting-{}-{label}", std::process::id()));
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

fn object(g: &mut Game, weight: i32, capacity: i32) -> mud_data::ids::ObjId {
    let mut o = mud_game::obj::create_obj(); o.weight = weight;
    o.type_flag = mud_data::flags::ITEM_CONTAINER; o.values[0] = capacity;
    g.objs.insert(o)
}

#[test]
fn nested_weight_propagation_stops_at_every_unlimited_container() {
    let mut f = fixture("nested"); let g = &mut f.game;
    let ch = player(g, b"Carrier", 12345);
    for outer_capacity in [0, 100] {
        let outer = object(g, 2, outer_capacity); let inner = object(g, 3, 100); let item = object(g, 7, 0);
        mud_game::handler::obj_to_char(g, outer, ch);
        mud_game::handler::obj_to_obj(g, inner, outer);
        mud_game::handler::obj_to_obj(g, item, inner);
        let expected = if outer_capacity > 0 { 12 } else { 2 };
        assert_eq!(g.obj(inner).weight, 10);
        assert_eq!(g.obj(outer).weight, expected);
        assert_eq!(g.ch(ch).carry_weight, expected);
        mud_game::handler::obj_from_obj(g, item);
        assert_eq!(g.obj(inner).weight, 3);
        assert_eq!(g.obj(outer).weight, if outer_capacity > 0 { 5 } else { 2 });
        mud_game::handler::extract_obj(g, item); mud_game::handler::extract_obj(g, outer);
        assert_eq!(g.ch(ch).carry_weight, 0);
    }
}

#[test]
fn weight_changes_do_not_reacquire_items_or_complete_find_quests() {
    let mut f = fixture("quest"); let g = &mut f.game;
    let ch = player(g, b"Carrier", 12345); descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let item = mud_game::db::read_object(g, 0).unwrap();
    mud_game::handler::obj_to_char(g, item, ch);
    let other = object(g, 1, 0); mud_game::handler::obj_to_char(g, other, ch);
    g.world.quests = vec![mud_world::model::Quest { vnum: 65000, type_: mud_game::quest::AQ_OBJ_FIND,
        target: g.world.obj_protos[0].vnum as i32, obj_out: 1, value: 7, next_quest: -1, ..Default::default() }];
    mud_game::quest::set_quest(g, ch, 0);
    let before = g.ch(ch).carrying.clone(); let weight = g.ch(ch).carry_weight;
    mud_game::act::item::weight_change_object(g, item, -1);
    assert_eq!(g.ch(ch).ps().current_quest, 65000);
    assert_eq!(g.ch(ch).ps().questpoints, 0);
    assert_eq!(g.ch(ch).carrying, before);
    assert_eq!(g.ch(ch).carry_weight, weight - 1);
}

#[test]
fn script_weight_changes_update_carrier_accounting() {
    use mud_game::dg::{self, GoId};
    let mut f = fixture("script"); let g = &mut f.game;
    let ch = player(g, b"Carrier", 12345);
    let item = object(g, 7, 0); mud_game::handler::obj_to_char(g, item, ch);
    let go = GoId::Obj(item);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger { vnum: 65000, attach_type: dg::OBJ_TRIGGER, ..Default::default() });
    let trigger = dg::read_trigger(g, nr).unwrap(); let ctx = dg::DgCtx { go, iid: trigger.iid };
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.weight(5)%"), b"12");
    assert_eq!(g.ch(ch).carry_weight, 12);
}
