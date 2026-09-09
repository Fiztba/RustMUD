use mud_data::flags::EX_ISDOOR;
use mud_world::{model::{World, Room, Zone, Exit}, parse, write};

#[test]
fn all_diagonal_exits_survive_saving_and_loading() {
    let mut world = World::default();
    world.zones.push(Zone { number: 1, bot: 100, top: 199, ..Zone::default() });
    let mut room = Room { vnum: 100, ..Room::default() };
    for dir in 6..10 {
        room.dir_option[dir] = Some(Box::new(Exit {
            general_description: Some(format!("Exit {dir}").into_bytes()),
            keyword: Some(b"door".to_vec()), exit_info: EX_ISDOOR,
            key: 101, to_room_vnum: 100, to_room: 0,
        }));
    }
    world.rooms.push(room);
    world.room_map.insert(100, 0);
    let bytes = write::wld::write_file(&world, 0);
    for dir in 6..10 {
        assert!(String::from_utf8_lossy(&bytes).contains(&format!("D{dir}\n")), "D{dir} lost on save");
    }
    let mut loaded = World::default();
    loaded.zones = world.zones.clone();
    parse::wld::parse_file(&mut loaded, &bytes, "1.wld").unwrap();
    for dir in 6..10 {
        let exit = loaded.rooms[0].dir_option[dir].as_ref().unwrap();
        assert_eq!(exit.to_room_vnum, 100);
        assert_eq!(exit.key, 101);
        assert_eq!(exit.exit_info, EX_ISDOOR);
        assert_eq!(exit.general_description, Some(format!("Exit {dir}").into_bytes()));
    }
    let exported = write::wld::write_file_fmt(&world, 0, write::VnumFmt::renumber(&world.zones[0], 2));
    assert!(String::from_utf8_lossy(&exported).contains("1 201 200\n"));
}
