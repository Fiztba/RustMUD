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
    let root = std::env::temp_dir().join(format!("rustmud-dying-quit-{}-{label}", std::process::id()));
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
fn quitting_while_dying_runs_death_penalties_and_creates_a_corpse() {
    let mut f = fixture("death"); let g = &mut f.game;
    let actor = player(g, b"Dying", 12345); g.character_list.push_back(actor);
    mud_game::handler::char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    g.ch_mut(actor).points.hit = -5; g.ch_mut(actor).position = POS_INCAP;
    g.ch_mut(actor).points.exp = 1000;
    let mut obj = mud_game::obj::create_obj(); obj.name = Some(b"keepsake".to_vec());
    let item = g.objs.insert(obj); mud_game::handler::obj_to_char(g, item, actor);
    mud_game::act::other::do_quit(g, actor, b"", 0, mud_game::interpreter::SCMD_QUIT);
    assert!(g.ch(actor).points.exp < 1000);
    let corpse = g.obj(item).in_obj.expect("possessions should be placed in the corpse");
    assert!(mud_game::handler::is_corpse(g, corpse));
    assert_eq!(g.obj(corpse).in_room, 0);
}
