use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
use mud_net::descriptor::Descriptor;
use mud_world::model::ZoneCommand;
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
    let root = std::env::temp_dir().join(format!("rustmud-prototype-delete-open-editors-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    let id = g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: LVL_IMPL,
        passwd: mud_data::crypt::crypt(b"secret", name).unwrap().to_vec(),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    });
    mud_game::handler::char_to_room(g, id, 0);
    id
}

fn descriptor(g: &mut Game, ch: mud_data::ids::CharId, state: ConState) -> usize {
    let mut d = Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = state;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    di
}

/// Builder A's zedit session for room rnum 0, sitting on the save prompt
/// with `cmds` as its scratch reset list.
fn open_zedit(g: &mut Game, cmds: Vec<ZoneCommand>) -> usize {
    let a = player(g, b"Alpha", 12345);
    let di = descriptor(g, a, ConState::Zedit);
    let mut olc = mud_game::olc::OlcData::new();
    olc.mode = mud_game::olc::zedit::ZEDIT_CONFIRM_SAVESTRING;
    olc.number = g.world.rooms[0].vnum as i32;
    olc.zone_num = g.world.rooms[0].zone as i32;
    olc.zone_age = 1;
    let mut zone = g.world.zones[g.world.rooms[0].zone as usize].clone();
    zone.number = 0;
    zone.cmds = cmds;
    olc.zone = Some(Box::new(zone));
    g.olc.insert(di, olc);
    di
}

fn open_editor(g: &mut Game, name: &[u8], idnum: i64, state: ConState) -> usize {
    let b = player(g, name, idnum);
    let di = descriptor(g, b, state);
    g.olc.insert(di, mud_game::olc::OlcData::new());
    di
}

#[test]
fn deleting_the_last_mobile_drops_it_from_an_open_zone_editor_before_save() {
    let mut f = fixture("last-mob"); let g = &mut f.game;
    let last = (g.world.mob_protos.len() - 1) as Idx;
    let cmds = vec![
        ZoneCommand { command: b'M', arg1: last as i32, arg2: 1, arg3: 0, ..Default::default() },
        ZoneCommand { command: b'G', if_flag: 1, arg1: 0, arg2: 1, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    let _db = open_editor(g, b"Beta", 12346, ConState::Medit);
    assert_eq!(mud_game::olc::genmob::delete_mobile(g, last), Some(last));
    assert!(g.olc[&da].zone.as_ref().unwrap().cmds.iter().all(|c| c.command != b'M'));

    // A's save copies its scratch list into the live zone and writes it out.
    assert!(g.config.auto_save_olc);
    let zone = g.world.rooms[0].zone as usize;
    assert!(mud_game::olc::olc_parse(g, da, b"y"));
    assert!(g.olc.get(&da).is_none());
    let protos = g.world.mob_protos.len() as i32;
    assert!(g.world.zones[zone].cmds.iter().all(|c| c.command != b'M' || c.arg1 < protos));
    mud_game::db::reset_zone(g, zone);
}

#[test]
fn deleting_the_last_object_drops_it_from_an_open_zone_editor_before_save() {
    let mut f = fixture("last-obj"); let g = &mut f.game;
    let last = (g.world.obj_protos.len() - 1) as Idx;
    let cmds = vec![
        ZoneCommand { command: b'O', arg1: last as i32, arg2: 1, arg3: 0, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    let _db = open_editor(g, b"Beta", 12346, ConState::Oedit);
    assert_eq!(mud_game::olc::genobj::delete_object(g, last), Some(last));
    assert!(g.olc[&da].zone.as_ref().unwrap().cmds.iter().all(|c| c.command != b'O'));

    let zone = g.world.rooms[0].zone as usize;
    assert!(mud_game::olc::olc_parse(g, da, b"y"));
    assert!(g.olc.get(&da).is_none());
    let protos = g.world.obj_protos.len() as i32;
    assert!(g.world.zones[zone].cmds.iter().all(|c| c.command != b'O' || c.arg1 < protos));
    mud_game::db::reset_zone(g, zone);
}

#[test]
fn deleting_a_mobile_renumbers_higher_references_in_open_editors() {
    let mut f = fixture("renumber-mob"); let g = &mut f.game;
    let kept_vnum = g.world.mob_protos[2].vnum;
    let cmds = vec![
        ZoneCommand { command: b'M', arg1: 2, arg2: 1, arg3: 0, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    let dm = open_editor(g, b"Beta", 12346, ConState::Medit);
    g.olc.get_mut(&dm).unwrap().mob_rnum = 2;
    let ds = open_editor(g, b"Gamma", 12347, ConState::Sedit);
    g.olc.get_mut(&ds).unwrap().shop_keeper = 2;
    let dd = open_editor(g, b"Delta", 12348, ConState::Medit);
    g.olc.get_mut(&dd).unwrap().mob_rnum = 0;

    assert_eq!(mud_game::olc::genmob::delete_mobile(g, 0), Some(0));
    let cmds = &g.olc[&da].zone.as_ref().unwrap().cmds;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].arg1, 1);
    assert_eq!(g.world.mob_protos[1].vnum, kept_vnum);
    assert_eq!(g.olc[&dm].mob_rnum, 1);
    assert_eq!(g.olc[&ds].shop_keeper, 1);
    assert_eq!(g.olc[&dd].mob_rnum, NOBODY);
}

#[test]
fn deleting_an_object_renumbers_higher_references_in_open_editors() {
    let mut f = fixture("renumber-obj"); let g = &mut f.game;
    let kept_vnum = g.world.obj_protos[2].vnum;
    let cmds = vec![
        ZoneCommand { command: b'O', arg1: 2, arg2: 1, arg3: 0, ..Default::default() },
        ZoneCommand { command: b'P', if_flag: 1, arg1: 2, arg2: 1, arg3: 2, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    let d2 = open_editor(g, b"Beta", 12346, ConState::Oedit);
    g.olc.get_mut(&d2).unwrap().obj_rnum = 2;
    let d0 = open_editor(g, b"Gamma", 12347, ConState::Oedit);
    g.olc.get_mut(&d0).unwrap().obj_rnum = 0;

    assert_eq!(mud_game::olc::genobj::delete_object(g, 0), Some(0));
    let cmds = &g.olc[&da].zone.as_ref().unwrap().cmds;
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0].arg1, 1);
    assert_eq!((cmds[1].arg1, cmds[1].arg3), (1, 1));
    assert_eq!(g.world.obj_protos[1].vnum, kept_vnum);
    assert_eq!(g.olc[&d2].obj_rnum, 1);
    assert_eq!(g.olc[&d0].obj_rnum, NOTHING);
}

/// zedit keeps a cursor (`olc.value`) into its scratch command list while a
/// command is being filled in. Killing the command under it must disable it
/// in place, keep every later command at its index, and put the builder back
/// at the zone menu, since no argument prompt has a case for '*'.
#[test]
fn deleting_a_mobile_under_an_open_zedit_cursor_disables_it_in_place() {
    let mut f = fixture("cursor-mob"); let g = &mut f.game;
    let last = (g.world.mob_protos.len() - 1) as Idx;
    let cmds = vec![
        ZoneCommand { command: b'M', arg1: last as i32, arg2: 1, arg3: 0, ..Default::default() },
        ZoneCommand { command: b'O', arg1: 0, arg2: 1, arg3: 0, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    {
        let olc = g.olc.get_mut(&da).unwrap();
        olc.mode = mud_game::olc::zedit::ZEDIT_ARG2;
        olc.value = 0;
    }
    assert_eq!(mud_game::olc::genmob::delete_mobile(g, last), Some(last));
    let olc = &g.olc[&da];
    let cmds = &olc.zone.as_ref().unwrap().cmds;
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0].command, b'*');
    assert_eq!((cmds[1].command, cmds[1].arg1), (b'O', 0));
    assert_eq!(olc.mode, mud_game::olc::zedit::ZEDIT_MAIN_MENU);
    assert!(g.descriptors.get(da).unwrap().output.windows(11).any(|w| w == b"has been de"));
}

#[test]
fn deleting_an_object_under_an_open_zedit_cursor_disables_it_in_place() {
    let mut f = fixture("cursor-obj"); let g = &mut f.game;
    let last = (g.world.obj_protos.len() - 1) as Idx;
    let cmds = vec![
        ZoneCommand { command: b'M', arg1: 0, arg2: 1, arg3: 0, ..Default::default() },
        ZoneCommand { command: b'G', if_flag: 1, arg1: last as i32, arg2: 1, ..Default::default() },
        ZoneCommand { command: b'O', arg1: 0, arg2: 1, arg3: 0, ..Default::default() },
    ];
    let da = open_zedit(g, cmds);
    {
        let olc = g.olc.get_mut(&da).unwrap();
        olc.mode = mud_game::olc::zedit::ZEDIT_ARG1;
        olc.value = 1;
    }
    assert_eq!(mud_game::olc::genobj::delete_object(g, last), Some(last));
    let olc = &g.olc[&da];
    let cmds = &olc.zone.as_ref().unwrap().cmds;
    assert_eq!(cmds.len(), 3);
    assert_eq!(cmds[0].command, b'M');
    assert_eq!(cmds[1].command, b'*');
    assert_eq!((cmds[2].command, cmds[2].arg1), (b'O', 0));
    assert_eq!(olc.mode, mud_game::olc::zedit::ZEDIT_MAIN_MENU);
}
