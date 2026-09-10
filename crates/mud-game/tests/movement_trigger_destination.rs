use mud_data::{flags, types::*};
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
    let root = std::env::temp_dir().join(format!("rustmud-movement-destination-{}-{label}", std::process::id()));
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

use mud_game::{dg::{self, GoId}, handler::*};
fn movement_case(stage: &str, allow: bool, redirect: bool, death: bool) {
    let mut f = fixture(&format!("{stage}-{allow}-{redirect}-{death}")); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); g.character_list.push_back(actor);
    char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    g.ch_mut(actor).position = POS_STANDING;
    g.ch_mut(actor).points.mov = 100;
    for room in 0..3 { g.world.rooms[room].room_flags = [0;4]; g.world.rooms[room].sector_type = 0; }
    if death { g.world.rooms[1].room_flags[flags::ROOM_DEATH / 32] |= 1 << (flags::ROOM_DEATH % 32); }
    g.world.rooms[0].dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
        general_description: None, keyword: None, exit_info: 0, key: NOTHING,
        to_room_vnum: g.world.rooms[1].vnum as i32, to_room: 1,
    }));
    let go = if stage == "greet" {
        let mob = mud_game::db::read_mobile(g, 0).unwrap();
        g.ch_mut(mob).script = None;
        g.ch_mut(mob).affected_by = Default::default();
        g.ch_mut(mob).position = POS_STANDING;
        char_to_room(g, mob, 1);
        GoId::Char(mob)
    } else { GoId::Room(1) };
    let mut cmds = vec![];
    if redirect { cmds.push(format!("{}teleport %actor% {}", if stage == "greet" { "m" } else { "w" }, g.world.rooms[2].vnum).into_bytes()); }
    cmds.push(format!("return {}", i32::from(allow)).into_bytes());
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: if stage == "greet" { dg::MOB_TRIGGER } else { dg::WLD_TRIGGER },
        trigger_type: if stage == "greet" { dg::MTRIG_GREET_ALL } else { dg::WTRIG_ENTER },
        narg: 100, cmdlist: cmds, ..Default::default()
    });
    let trigger = dg::read_trigger(g, nr).unwrap();
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    let moved = mud_game::act::movement::do_simple_move(g, actor, NORTH, false);
    let killed = death && allow && !redirect;
    assert_eq!(g.ch(actor).plr(flags::PLR_NOTDEADYET), killed);
    assert_eq!(g.ch(actor).in_room, if redirect { 2 } else if allow { 1 } else { 0 });
    assert_eq!(moved, allow && !redirect && !killed);
    assert_eq!(g.ch(actor).position, POS_STANDING);
}
#[test]
fn enter_teleport_avoids_the_old_destinations_death_trap() { movement_case("enter", true, true, true); }
#[test]
fn enter_teleport_is_not_undone_by_a_denied_move() { movement_case("enter", false, true, true); }

#[test]
fn normal_entry_succeeds() { movement_case("enter", true, false, false); }
#[test]
fn denied_entry_rolls_back() { movement_case("enter", false, false, false); }
#[test]
fn normal_death_trap_still_extracts_player() { movement_case("enter", true, false, true); }
#[test]
fn allowed_greeting_teleport_stops_original_move() { movement_case("greet", true, true, false); }
#[test]
fn denied_greeting_teleport_is_preserved() { movement_case("greet", false, true, false); }
#[test]
fn denied_greeting_reports_failed_movement() { movement_case("greet", false, false, false); }
