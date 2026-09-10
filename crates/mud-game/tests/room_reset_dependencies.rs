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
    let root = std::env::temp_dir().join(format!("rustmud-room-reset-deps-{}-{label}", std::process::id()));
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
fn deleting_a_room_does_not_retarget_its_mobile_equipment_resets() {
    let mut f = fixture("mobile"); let g = &mut f.game;
    let mob = 0usize;
    let object = 0usize;
    g.world.mob_protos[mob].proto_script.clear();
    g.world.obj_protos[object].proto_script.clear();
    let commands = vec![
        mud_world::model::ZoneCommand { command: b'M', arg1: mob as i32, arg2: i32::MAX, arg3: 0, ..Default::default() },
        mud_world::model::ZoneCommand { command: b'M', arg1: mob as i32, arg2: i32::MAX, arg3: 1, ..Default::default() },
        mud_world::model::ZoneCommand { command: b'G', arg1: object as i32, arg2: i32::MAX, ..Default::default() },
    ];
    g.world.zones[0].cmds = commands.clone();
    let actor = player(g, b"Builder", 12345);
    let di = descriptor(g, actor, ConState::Zedit);
    let mut olc = mud_game::olc::OlcData::new();
    let mut pending = g.world.zones[0].clone(); pending.cmds = commands;
    olc.zone = Some(Box::new(pending)); g.olc.insert(di, olc);
    assert!(mud_game::olc::genwld::delete_room(g, 1));
    mud_game::db::reset_zone(g, 0);
    let equipped = g.rooms[0].people.iter().filter_map(|id| g.try_ch(*id))
        .filter(|c| c.mob_rnum == mob as u16).any(|c| !c.carrying.is_empty());
    assert!(!equipped, "deleted room's G command equipped the preceding mobile");
    assert_eq!(g.olc[&di].zone.as_ref().unwrap().cmds[2].command, b'*');
}
