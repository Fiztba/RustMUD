use mud_game::game::Game;
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
    let root = std::env::temp_dir().join(format!("rustmud-copyover-house-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}


#[test]
fn copyover_persists_house_additions_and_removals() {
    use mud_data::flags;
    use mud_game::handler::{obj_to_room, obj_from_room, set_room_flag, room_flagged};
    let mut f = fixture("save");
    let g = &mut f.game;
    let room = 0;
    let vnum = g.world.rooms[room].vnum as i32;
    g.rooms[room].contents.clear();
    g.houses.push(mud_game::house::HouseControl { vnum, ..Default::default() });
    set_room_flag(g, room as u16, flags::ROOM_HOUSE);
    let ch = g.chars.insert(mud_game::ch::Char { name: Some(b"Tester".to_vec()), ..Default::default() });
    let oid = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(oid).name = Some(b"copyover-test-object".to_vec());
    g.obj_mut(oid).type_flag = flags::ITEM_CONTAINER;
    g.obj_mut(oid).weight = 7;
    g.obj_mut(oid).values[3] = 0;
    let child = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(child).weight = 3;
    mud_game::handler::obj_to_obj(g, child, oid);
    let path = g.lib_dir.join("house").join(format!("{vnum}.house"));
    mud_game::copyover::do_copyover(g, ch, b"", 0, 0);
    assert!(g.copyover.is_some());
    // A later command in the same pulse can still change the house.
    obj_to_room(g, oid, room as u16);
    assert!(mud_game::copyover::take_copyover_plan(g).is_some());
    let data = std::fs::read(&path).expect("copyover must save dirty houses");
    let records = mud_game::objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(&data));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].locate, -1);
    assert_eq!(g.obj(records[0].obj).weight, 3);
    assert_eq!(g.obj(records[1].obj).name.as_deref(), Some(b"copyover-test-object".as_slice()));
    assert_eq!(g.obj(records[1].obj).weight, 7);
    assert_eq!(g.obj(oid).weight, 10, "saving must restore live container weight");
    assert!(!room_flagged(g, room as u16, flags::ROOM_HOUSE_CRASH));
    mud_game::copyover::do_copyover(g, ch, b"", 0, 0);
    obj_from_room(g, oid);
    assert!(mud_game::copyover::take_copyover_plan(g).is_some());
    assert_eq!(std::fs::read(path).unwrap(), b"$~\n", "removed items must not reappear after recovery");
}
