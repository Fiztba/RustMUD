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
    let root = std::env::temp_dir().join(format!("rustmud-purge-scope-{}-{label}", std::process::id()));
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
fn a_world_purge_requires_permission_for_every_zone_before_changing_anything() {
    let mut f = fixture("scope"); let g = &mut f.game;
    let builder = player(g, b"Builder", 12345); g.ch_mut(builder).level = LVL_BUILDER;
    g.ch_mut(builder).ps_mut().olc_zone = g.world.zones[0].number as i32;
    for z in &mut g.world.zones { z.zone_flags = [0; 4]; z.builders = None; }
    mud_game::handler::char_to_room(g, builder, 0); let di = descriptor(g, builder, ConState::Playing);
    let outside = g.world.rooms.iter().position(|r| r.zone != 0).unwrap() as u16;
    let mut ids = Vec::new();
    for room in [0, outside] {
        let oid = g.objs.insert(mud_game::obj::create_obj()); mud_game::handler::obj_to_room(g, oid, room); ids.push(oid);
    }
    mud_game::act::wizard::do_zpurge(g, builder, b"*", 0, 0);
    assert!(ids.iter().all(|&oid| g.try_obj(oid).is_some()), "a rejected world purge must not remove objects in any zone");
    assert!(String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains("own zone"));
    let own_zone = g.world.zones[0].number;
    mud_game::act::wizard::do_zpurge(g, builder, own_zone.to_string().as_bytes(), 0, 0);
    assert!(g.try_obj(ids[0]).is_none()); assert!(g.try_obj(ids[1]).is_some());
    let other_zone = g.world.zones[g.world.rooms[outside as usize].zone as usize].number;
    mud_game::act::wizard::do_zpurge(g, builder, other_zone.to_string().as_bytes(), 0, 0);
    assert!(g.try_obj(ids[1]).is_some());
    g.ch_mut(builder).level = LVL_GOD;
    mud_game::act::wizard::do_zpurge(g, builder, b"*", 0, 0);
    assert!(g.try_obj(ids[1]).is_none());
    g.ch_mut(builder).level = LVL_BUILDER;
    g.ch_mut(builder).ps_mut().olc_zone = mud_game::act::wizstat::ALL_PERMISSION;
    let oid = g.objs.insert(mud_game::obj::create_obj()); mud_game::handler::obj_to_room(g, oid, outside);
    mud_game::act::wizard::do_zpurge(g, builder, b"*", 0, 0);
    assert!(g.try_obj(oid).is_none());
}

