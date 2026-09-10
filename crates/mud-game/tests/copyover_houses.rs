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
    obj_to_room(g, oid, room as u16);
    let path = g.lib_dir.join("house").join(format!("{vnum}.house"));
    mud_game::copyover::do_copyover(g, ch, b"", 0, 0);
    assert!(g.copyover.is_some());
    let data = std::fs::read(&path).expect("copyover must save dirty houses");
    let records = mud_game::objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(&data));
    assert_eq!(records.len(), 1);
    assert_eq!(g.obj(records[0].obj).name.as_deref(), Some(b"copyover-test-object".as_slice()));
    assert!(!room_flagged(g, room as u16, flags::ROOM_HOUSE_CRASH));
    obj_from_room(g, oid);
    mud_game::copyover::do_copyover(g, ch, b"", 0, 0);
    assert_eq!(std::fs::read(path).unwrap(), b"$~\n", "removed items must not reappear after recovery");
}
