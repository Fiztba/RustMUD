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
    let root = std::env::temp_dir().join(format!("rustmud-builder-vnum-{}-{label}", std::process::id()));
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
fn load_does_not_wrap_large_virtual_numbers() {
    let mut f = fixture("load"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    let objects = g.objs.len(); let mobiles = g.chars.len();
    let object_vnum = u32::from(g.world.obj_protos[0].vnum) + 65536;
    let mobile_vnum = u32::from(g.world.mob_protos[0].vnum) + 65536;
    mud_game::act::wizard::do_load(g, actor, format!("obj {object_vnum}").as_bytes(), 0, 0);
    mud_game::act::wizard::do_load(g, actor, format!("mob {mobile_vnum}").as_bytes(), 0, 0);
    assert_eq!(g.objs.len(), objects); assert_eq!(g.chars.len(), mobiles);
}

#[test]
fn inspection_commands_reject_wrapped_numbers_and_accept_boundaries() {
    let mut f = fixture("inspect"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); let di = descriptor(g, actor, ConState::Playing);
    for checkload in [false, true] {
        for value in ["65535".to_string(), "65536".to_string(), i32::MAX.to_string(), "9".repeat(400)] {
            g.descriptors.get_mut(di).unwrap().output.clear();
            let args = format!("o {value}");
            if checkload { mud_game::act::wizard::do_checkloadstatus(g, actor, args.as_bytes(), 0, 0); }
            else { mud_game::act::wizstat::do_vstat(g, actor, args.as_bytes(), 0, 0); }
            let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
            assert!(out.contains("between 0 and 65534"), "{out}");
        }
        for value in [0, 65534] {
            g.descriptors.get_mut(di).unwrap().output.clear();
            let args = format!("o {value}");
            if checkload { mud_game::act::wizard::do_checkloadstatus(g, actor, args.as_bytes(), 0, 0); }
            else { mud_game::act::wizstat::do_vstat(g, actor, args.as_bytes(), 0, 0); }
            let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
            assert!(!out.contains("between 0 and 65534"), "{out}");
        }
    }
}
#[test]
fn valid_load_still_uses_the_requested_prototype() {
    let mut f = fixture("valid"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    g.world.obj_protos[0].proto_script.clear(); g.world.mob_protos[0].proto_script.clear();
    let objects = g.objs.len(); let mobiles = g.chars.len();
    mud_game::act::wizard::do_load(g, actor, format!("obj {}", g.world.obj_protos[0].vnum).as_bytes(), 0, 0);
    mud_game::act::wizard::do_load(g, actor, format!("mob {}", g.world.mob_protos[0].vnum).as_bytes(), 0, 0);
    assert_eq!(g.objs.len(), objects + 1); assert_eq!(g.chars.len(), mobiles + 1);
}
