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
    let root = std::env::temp_dir().join(format!("rustmud-social-targets-{}-{label}", std::process::id()));
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
fn body_part_social_reaches_actor_target_and_observer() {
    let mut f = fixture("body"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); let target = player(g, b"Target", 12346); let observer = player(g, b"Observer", 12347);
    let mut descriptors = vec![];
    for ch in [actor, target, observer] {
        mud_game::handler::char_to_room(g, ch, 0); g.ch_mut(ch).position = POS_STANDING;
        descriptors.push(descriptor(g, ch, ConState::Playing));
    }
    g.rooms[0].light = 1;
    let cmd = mud_game::interpreter::find_command(g, b"waves").unwrap();
    mud_game::act::social::do_action(g, actor, b"Target hand", cmd, 0);
    for di in descriptors {
        let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(out.contains("hand") && !out.contains("<NULL>"), "{out}");
    }
}
#[test]
fn object_social_finds_carried_and_room_objects() {
    let mut f = fixture("object"); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); mud_game::handler::char_to_room(g, actor, 0);
    let di = descriptor(g, actor, ConState::Playing); g.rooms[0].light = 1;
    let cmd = mud_game::interpreter::find_command(g, b"waves").unwrap();
    let social = g.commands[cmd].social.unwrap();
    g.socials[social].char_obj_found = Some(b"You wave to $p.".to_vec());
    g.socials[social].others_obj_found = Some(b"$n waves to $p.".to_vec());
    let mut obj = mud_game::obj::create_obj(); obj.name = Some(b"gem".to_vec()); obj.short_description = Some(b"a gem".to_vec());
    let oid = g.objs.insert(obj); mud_game::handler::obj_to_char(g, oid, actor);
    for carried in [true, false] {
        if !carried { mud_game::handler::obj_from_char(g, oid); mud_game::handler::obj_to_room(g, oid, 0); }
        g.descriptors.get_mut(di).unwrap().output.clear();
        mud_game::act::social::do_action(g, actor, b"gem", cmd, 0);
        let out = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
        assert!(out.contains("You wave to a gem."), "{out}");
    }
}
