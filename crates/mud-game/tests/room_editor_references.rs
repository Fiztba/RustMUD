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
    let root = std::env::temp_dir().join(format!("rustmud-room-references-{}-{label}", std::process::id()));
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

#[test]
fn inserting_a_copied_room_preserves_live_and_pending_references() {
    use mud_game::{olc::OlcData, game::EventOwner};
    use mud_world::model::{Exit, Zone, ZoneCommand};
    let mut f = fixture("insert");
    let g = &mut f.game;
    g.config.auto_save_olc = false;
    g.config.diagonal_dirs = false;
    let deleted = g.real_room(1).unwrap();
    assert!(mud_game::olc::genwld::delete_room(g, deleted));
    let destination = g.real_room(4).unwrap();
    let other_room = g.real_room(3).unwrap();
    let exit = |to_room| Box::new(Exit { general_description: Some(b"A doorway.".to_vec()),
        keyword: None, exit_info: 0, key: NOTHING, to_room_vnum: 4, to_room });
    g.world.rooms[other_room as usize].dir_option[NORTHWEST] = Some(exit(destination));
    let builder = player(g, b"Builder", 12345);
    g.ch_mut(builder).level = LVL_IMPL;
    let di = descriptor(g, builder, ConState::Playing);
    mud_game::handler::char_to_room(g, builder, 0);
    let other = player(g, b"Other", 12346);
    let other_di = descriptor(g, other, ConState::Redit);
    let mut pending = OlcData::default();
    let mut scratch = g.world.rooms[other_room as usize].clone();
    scratch.dir_option[NORTH] = Some(exit(destination));
    scratch.dir_option[SOUTH] = Some(exit(NOWHERE));
    pending.room = Some(Box::new(scratch));
    g.olc.insert(other_di, Box::new(pending));
    let zone_builder = player(g, b"Resetter", 12347);
    let zone_di = descriptor(g, zone_builder, ConState::Zedit);
    let mut pending = OlcData::default();
    pending.zone = Some(Box::new(Zone { cmds: vec![
        ZoneCommand { command: b'D', arg1: destination as i32, arg2: NORTHWEST as i32, ..Default::default() },
        ZoneCommand { command: b'O', arg3: NOWHERE as i32, ..Default::default() },
    ], ..Default::default() }));
    g.olc.insert(zone_di, Box::new(pending));
    let room_vnums: Vec<_> = g.world.rooms.iter().map(|r| r.vnum).collect();
    for r in 0..g.rooms.len() { g.event_lists.insert(EventOwner::Room(r as RoomRnum)); }
    let source = g.real_room(2).unwrap();
    g.rooms[source as usize].light = 7;
    mud_game::olc::redit::do_oasis_redit(g, builder, b"1", 0, 0);
    for input in [b"w".as_slice(), b"2", b"q", b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    let new_room = g.real_room(1).unwrap();
    assert_eq!(g.rooms[new_room as usize].light, 0, "copied room inherited phantom light");
    let destination = g.real_room(4).unwrap();
    let scratch = g.olc[&other_di].room.as_ref().unwrap();
    assert_eq!(scratch.dir_option[NORTH].as_ref().unwrap().to_room, destination);
    assert_eq!(scratch.dir_option[SOUTH].as_ref().unwrap().to_room, NOWHERE);
    let cmds = &g.olc[&zone_di].zone.as_ref().unwrap().cmds;
    assert_eq!(cmds[0].arg1, destination as i32);
    assert_eq!(cmds[0].arg2, NORTHWEST as i32);
    assert_eq!(cmds[1].arg3, NOWHERE as i32);
    let other_room = g.real_room(3).unwrap();
    assert_eq!(g.world.rooms[other_room as usize].dir_option[NORTHWEST].as_ref().unwrap().to_room, destination);
    for vnum in room_vnums {
        assert!(g.event_lists.contains(&EventOwner::Room(g.world.real_room(vnum).unwrap())), "lost event owner {vnum}");
    }
    assert!(mud_game::olc::genwld::delete_room(g, new_room));
    let other_room = g.real_room(3).unwrap();
    assert_eq!(g.world.rooms[other_room as usize].dir_option[NORTHWEST].as_ref().unwrap().to_room, g.real_room(4).unwrap());
}
