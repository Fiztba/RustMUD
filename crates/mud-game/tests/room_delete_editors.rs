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
    let root = std::env::temp_dir().join(format!("rustmud-room-delete-editors-{}-{label}", std::process::id()));
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
fn deleting_a_room_updates_other_builders_pending_room_and_reset_references() {
    let mut f = fixture("references"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); mud_game::handler::char_to_room(g, builder, 0);
    let rd = descriptor(g, builder, ConState::Redit);
    let zd = descriptor(g, builder, ConState::Zedit);
    let mut room_editor = mud_game::olc::OlcData::new(); room_editor.room = Some(Box::new(g.world.rooms[3].clone()));
    for (direction, target) in [0, 1, 2, 3, NOWHERE, 1, 2, 3, NOWHERE, 0].into_iter().enumerate() {
        room_editor.room.as_mut().unwrap().dir_option[direction] = Some(Box::new(mud_world::model::Exit {
            general_description: None, keyword: Some(b"door".to_vec()), exit_info: 1, key: 123,
            to_room_vnum: 0, to_room: target,
        }));
    }
    g.olc.insert(rd, room_editor);
    let mut zone_editor = mud_game::olc::OlcData::new(); zone_editor.zone = Some(Box::new(g.world.zones[0].clone()));
    zone_editor.zone.as_mut().unwrap().cmds = vec![
        mud_world::model::ZoneCommand { command: b'D', arg1: 3, arg2: 1, arg3: 2, ..Default::default() },
        mud_world::model::ZoneCommand { command: b'R', arg1: 1, arg2: 3, ..Default::default() },
        mud_world::model::ZoneCommand { command: b'O', arg1: 0, arg2: 1, arg3: 3, ..Default::default() },
        mud_world::model::ZoneCommand { command: b'T', arg1: mud_game::dg::WLD_TRIGGER, arg2: 0, arg3: NOWHERE as i32, ..Default::default() },
    ];
    g.olc.insert(zd, zone_editor);
    assert!(mud_game::olc::genwld::delete_room(g, 1));
    for (direction, expected) in [0, NOWHERE, 1, 2, NOWHERE, NOWHERE, 1, 2, NOWHERE, 0].into_iter().enumerate() {
        assert_eq!(g.olc[&rd].room.as_ref().unwrap().dir_option[direction].as_ref().unwrap().to_room, expected, "direction={direction}");
    }
    let cmds = &g.olc[&zd].zone.as_ref().unwrap().cmds;
    assert_eq!((cmds[0].arg1, cmds[0].arg2, cmds[0].arg3), (2, 1, 2));
    assert_eq!(cmds[1].command, b'*'); assert_eq!(cmds[2].arg3, 2); assert_eq!(cmds[3].arg3, NOWHERE as i32);
}
