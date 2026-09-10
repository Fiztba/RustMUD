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
    let root = std::env::temp_dir().join(format!("rustmud-locate-keywords-{}-{label}", std::process::id()));
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
fn locate_object_reports_later_alias_matches_in_room_and_inventory() {
    let mut f = fixture("locate"); let g = &mut f.game;
    let ch = player(g, b"Caster", 12345); mud_game::handler::char_to_room(g, ch, 0);
    let di = descriptor(g, ch, ConState::Playing);
    let mut ids = Vec::new();
    for (name, short) in [(b"shimmering ring".as_slice(), b"a golden ring".as_slice()), (b"boring ring".as_slice(), b"a silver ring".as_slice()), (b"shimmering".as_slice(), b"a glass bauble".as_slice())] {
        let mut o = mud_game::obj::create_obj(); o.name = Some(name.to_vec()); o.short_description = Some(short.to_vec());
        let oid = g.objs.insert(o); g.object_list.push_front(oid); ids.push(oid);
    }
    mud_game::handler::obj_to_room(g, ids[0], 0);
    mud_game::handler::obj_to_char(g, ids[1], ch);
    mud_game::handler::obj_to_room(g, ids[2], 0);
    g.cast_arg2 = b"ring".to_vec();
    mud_game::spells::spell_locate_object(g, 10, ch, None, Some(ids[0]));
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(output.contains("A golden ring is in"));
    assert!(output.contains("A silver ring is being carried"));
    assert!(!output.contains("glass bauble"));
}
