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
    let root = std::env::temp_dir().join(format!("rustmud-attach-zone-{}-{label}", std::process::id()));
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

use mud_game::dg::{self, GoId};
fn attach_case(object: bool) {
    let mut f = fixture(if object { "object" } else { "mobile" }); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345); g.ch_mut(actor).level = LVL_GOD;
    mud_game::handler::char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let own_zone = g.world.rooms[0].zone as usize;
    g.ch_mut(actor).ps_mut().olc_zone = g.world.zones[own_zone].number as i32;
    for zone in &mut g.world.zones { zone.builders = None; zone.zone_flags = [0;4]; }
    let (target, zone) = if object {
        let (nr, zone) = g.world.obj_protos.iter().enumerate().find_map(|(nr,p)|
            dg::mobcmd::real_zone_by_thing(g, p.vnum as i32).filter(|z| *z != own_zone).map(|z| (nr,z))).unwrap();
        let oid = mud_game::db::read_object(g, nr as u16).unwrap();
        g.obj_mut(oid).name = Some(b"zonetarget".to_vec());
        mud_game::handler::obj_to_room(g, oid, 0);
        (GoId::Obj(oid), zone)
    } else {
        let (nr, zone) = g.world.mob_protos.iter().enumerate().find_map(|(nr,p)|
            dg::mobcmd::real_zone_by_thing(g, p.vnum as i32).filter(|z| *z != own_zone).map(|z| (nr,z))).unwrap();
        let mob = mud_game::db::read_mobile(g, nr as u16).unwrap();
        g.ch_mut(mob).name = Some(b"zonetarget".to_vec());
        g.ch_mut(mob).affected_by = Default::default();
        mud_game::handler::char_to_room(g, mob, 0);
        (GoId::Char(mob), zone)
    };
    dg::extract_script(g, target);
    let kind = if object { "obj" } else { "mob" };
    let args = format!("{kind} {} zonetarget", g.world.triggers[0].vnum);
    dg::commands::do_attach(g, actor, args.as_bytes(), 0, 0);
    assert!(g.script_of(target).is_none(), "attachment used builder location instead of target zone");
    g.ch_mut(actor).ps_mut().olc_zone = g.world.zones[zone].number as i32;
    dg::commands::do_attach(g, actor, args.as_bytes(), 0, 0);
    assert_eq!(g.script_of(target).unwrap().trig_list.len(), 1);
    dg::commands::do_detach(g, actor, format!("{kind} zonetarget all").as_bytes(), 0, 0);
    assert!(g.script_of(target).is_none());
}
#[test]
fn mobile_attachment_checks_the_mobile_zone() { attach_case(false); }
#[test]
fn object_attachment_checks_the_object_zone() { attach_case(true); }
