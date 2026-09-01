//! The shipped world contains exactly one `T` zone command — zone 345's
//! "Secret room enter", which attaches trigger 34509 to room 34537. It is the
//! only coverage the T branch of renum_zone_table has, so it gets a test.

use std::path::PathBuf;

fn lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lib")
}

#[test]
fn zone_t_command_resolves_its_trigger() {
    let report = mud_world::boot::boot_world(&lib()).expect("boot");
    let world = &report.world;

    assert!(
        world.trig_map.contains_key(&34509),
        "trigger 34509 missing from the index"
    );

    let zone = world
        .zones
        .iter()
        .find(|z| z.number == 345)
        .expect("zone 345");
    let t = zone
        .cmds
        .iter()
        .find(|c| c.command == b'T')
        .expect("zone 345 has the world's only T command");

    let trig_rnum = world.trig_map[&34509] as i32;
    let room_rnum = world.real_room(34537).expect("room 34537") as i32;
    assert_eq!(t.arg2, trig_rnum, "T arg2 should renumber to the trigger rnum");
    assert_eq!(t.arg3, room_rnum, "T arg3 should renumber to the room rnum");

    assert!(
        !report.zone_errors.iter().any(|e| e.contains("zone #345")),
        "zone 345 reported: {:?}",
        report.zone_errors
    );
}
