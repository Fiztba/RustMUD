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
    let root = std::env::temp_dir().join(format!("rustmud-house-locations-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}



#[test]
fn invalid_house_locations_recover_onto_the_floor() {
    let mut f = fixture("bounds"); let g = &mut f.game;
    let (room, direction, atrium) = g.world.rooms.iter().enumerate().find_map(|(r, room)| {
        room.dir_option.iter().enumerate().find_map(|(d, e)| {
            e.as_ref().filter(|e| e.to_room != mud_data::types::NOWHERE).map(|e| (r, d, e.to_room))
        })
    }).unwrap();
    let vnum = g.world.rooms[room].vnum as i32;
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"owner".to_vec(), id: 12345, level: 10, flags: 0, last: g.now,
    });
    g.houses = vec![mud_game::house::HouseControl {
        vnum, atrium: g.world.rooms[atrium as usize].vnum as i32,
        exit_num: direction as i32, owner: 12345, ..Default::default()
    }];
    mud_game::house::house_save_control(g);
    let object = mud_game::db::read_object(g, 0).unwrap();
    for location in [1, i32::MIN, i32::MAX, -1000, 0, -1] {
        let name = format!("location-test-{location}").into_bytes(); g.obj_mut(object).name = Some(name.clone());
        let mut data = vec![];
        mud_game::objsave::objsave_save_obj_record(g, object, &mut data, location);
        data.extend_from_slice(b"$~\n");
        std::fs::write(g.lib_dir.join("house").join(format!("{vnum}.house")), data).unwrap();
        g.houses.clear(); mud_game::house::house_boot(g);
        assert!(g.rooms[room].contents.iter().any(|&o| g.obj(o).name.as_deref() == Some(name.as_slice())), "location {location}");
    }
    let child = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(child).name = Some(b"nested-child".to_vec());
    g.obj_mut(object).name = Some(b"nested-parent".to_vec());
    g.obj_mut(object).type_flag = mud_data::flags::ITEM_CONTAINER;
    let mut data = vec![];
    mud_game::objsave::objsave_save_obj_record(g, child, &mut data, -1);
    mud_game::objsave::objsave_save_obj_record(g, object, &mut data, 0);
    data.extend_from_slice(b"$~\n");
    std::fs::write(g.lib_dir.join("house").join(format!("{vnum}.house")), data).unwrap();
    g.houses.clear(); mud_game::house::house_boot(g);
    let parent = *g.rooms[room].contents.iter().find(|&&o| g.obj(o).name.as_deref() == Some(b"nested-parent")).unwrap();
    assert_eq!(g.obj(parent).contains.len(), 1);
    assert_eq!(g.obj(g.obj(parent).contains[0]).name.as_deref(), Some(b"nested-child".as_slice()));

}
