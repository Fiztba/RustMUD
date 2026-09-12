use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, dg, game::Game};
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
    let root = std::env::temp_dir().join(format!("rustmud-dg-vnum-{}-{label}", std::process::id()));
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

/// A script actor mobile (prototype 1) in room 0, plus the wrapped vnums of
/// the first mobile and object prototypes: proto vnum + 65536 narrows back
/// to the proto vnum as a u16.
fn scripted_mobile(g: &mut Game) -> (mud_data::ids::CharId, u32, u32) {
    for p in &mut g.world.mob_protos { p.proto_script.clear(); }
    for p in &mut g.world.obj_protos { p.proto_script.clear(); }
    let actor = mud_game::db::read_mobile(g, 1).unwrap();
    mud_game::handler::char_to_room(g, actor, 0);
    let mobile_vnum = u32::from(g.world.mob_protos[0].vnum) + 65536;
    let object_vnum = u32::from(g.world.obj_protos[0].vnum) + 65536;
    (actor, mobile_vnum, object_vnum)
}

fn logged(g: &Game, needle: &str) -> bool {
    g.log_lines.iter().any(|l| l.contains(needle))
}

#[test]
fn mload_rejects_wrapped_virtual_numbers() {
    let mut f = fixture("mload"); let g = &mut f.game;
    let (actor, mobile_vnum, object_vnum) = scripted_mobile(g);
    let objects = g.objs.len(); let mobiles = g.chars.len();
    dg::mobcmd::do_mload(g, actor, format!("mob {mobile_vnum}").as_bytes());
    dg::mobcmd::do_mload(g, actor, format!("obj {object_vnum}").as_bytes());
    assert_eq!(g.chars.len(), mobiles, "mload mob wrapped to an existing prototype");
    assert_eq!(g.objs.len(), objects, "mload obj wrapped to an existing prototype");
    assert!(logged(g, "mload: bad mob vnum"), "{:?}", g.log_lines);
    assert!(logged(g, "mload: bad object vnum"), "{:?}", g.log_lines);
    dg::mobcmd::do_mload(g, actor, format!("mob {}", g.world.mob_protos[0].vnum).as_bytes());
    dg::mobcmd::do_mload(g, actor, format!("obj {}", g.world.obj_protos[0].vnum).as_bytes());
    assert_eq!(g.chars.len(), mobiles + 1); assert_eq!(g.objs.len(), objects + 1);
}

#[test]
fn oload_rejects_wrapped_virtual_numbers() {
    let mut f = fixture("oload"); let g = &mut f.game;
    let (_, mobile_vnum, object_vnum) = scripted_mobile(g);
    let item = mud_game::db::read_object(g, 0).unwrap();
    mud_game::handler::obj_to_room(g, item, 0);
    let objects = g.objs.len(); let mobiles = g.chars.len();
    dg::objcmd::obj_command_interpreter(g, item, format!("oload mob {mobile_vnum}").as_bytes());
    dg::objcmd::obj_command_interpreter(g, item, format!("oload obj {object_vnum}").as_bytes());
    assert_eq!(g.chars.len(), mobiles, "oload mob wrapped to an existing prototype");
    assert_eq!(g.objs.len(), objects, "oload obj wrapped to an existing prototype");
    assert!(logged(g, "oload: bad mob vnum"), "{:?}", g.log_lines);
    assert!(logged(g, "oload: bad object vnum"), "{:?}", g.log_lines);
}

#[test]
fn wload_rejects_wrapped_virtual_numbers() {
    let mut f = fixture("wload"); let g = &mut f.game;
    let (_, mobile_vnum, object_vnum) = scripted_mobile(g);
    let objects = g.objs.len(); let mobiles = g.chars.len();
    dg::wldcmd::wld_command_interpreter(g, 0, format!("wload mob {mobile_vnum}").as_bytes());
    dg::wldcmd::wld_command_interpreter(g, 0, format!("wload obj {object_vnum}").as_bytes());
    assert_eq!(g.chars.len(), mobiles, "wload mob wrapped to an existing prototype");
    assert_eq!(g.objs.len(), objects, "wload obj wrapped to an existing prototype");
    assert!(logged(g, "mload: bad mob vnum"), "{:?}", g.log_lines);
    assert!(logged(g, "wload: bad object vnum"), "{:?}", g.log_lines);
}

#[test]
fn mtransform_rejects_wrapped_virtual_numbers() {
    let mut f = fixture("mtransform"); let g = &mut f.game;
    let (actor, mobile_vnum, _) = scripted_mobile(g);
    let mobiles = g.chars.len();
    dg::mobcmd::do_mtransform(g, actor, mobile_vnum.to_string().as_bytes());
    assert_eq!(g.ch(actor).mob_rnum, 1, "mtransform wrapped to an existing prototype");
    assert_eq!(g.chars.len(), mobiles);
    assert!(logged(g, "mtransform: bad mobile vnum"), "{:?}", g.log_lines);
}

#[test]
fn attach_rejects_wrapped_trigger_numbers() {
    let mut f = fixture("attach"); let g = &mut f.game;
    let (target, _, _) = scripted_mobile(g);
    g.ch_mut(target).name = Some(b"wraptarget".to_vec());
    g.ch_mut(target).affected_by = Default::default();
    dg::extract_script(g, dg::GoId::Char(target));
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); let di = descriptor(g, actor, ConState::Playing);
    let trigger_vnum = u32::from(g.world.triggers[0].vnum) + 65536;
    dg::commands::do_attach(g, actor, format!("mob {trigger_vnum} wraptarget").as_bytes(), 0, 0);
    assert!(g.script_of(dg::GoId::Char(target)).is_none(), "attach wrapped to an existing trigger");
    let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(out.contains("That trigger does not exist."), "{out}");
    dg::commands::do_attach(g, actor, format!("mob {} wraptarget", g.world.triggers[0].vnum).as_bytes(), 0, 0);
    assert_eq!(g.script_of(dg::GoId::Char(target)).unwrap().trig_list.len(), 1);
}
