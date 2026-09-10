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
    let root = std::env::temp_dir().join(format!("rustmud-message-count-{}-{label}", std::process::id()));
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
fn new_message_sets_remain_counted_and_save_on_quit() {
    let mut f = fixture("new-slot");
    let g = &mut f.game;
    let ch = player(g, b"Builder", 12345);
    let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0);
    let slot = g.fight_messages.iter().position(|s| s.msg.is_empty()).unwrap();
    mud_game::olc::msgedit::do_msgedit(g, ch, slot.to_string().as_bytes(), 0, 0);
    for input in [b"1".as_slice(), b"480", b"g", b"First hit", b"q", b"y"] {
        assert!(mud_game::olc::olc_parse(g, di, input));
    }
    assert_eq!(g.fight_messages[slot].number_of_attacks, 1);
    assert!(mud_game::fight::skill_message(g, 1, ch, ch, 480));
    mud_game::olc::msgedit::do_msgedit(g, ch, slot.to_string().as_bytes(), 0, 0);
    assert!(mud_game::olc::olc_parse(g, di, b"n"));
    assert!(mud_game::olc::olc_parse(g, di, b"q"));
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Msgedit, "adding a set is an unsaved edit");
    assert!(mud_game::olc::olc_parse(g, di, b"y"));
    assert_eq!(g.fight_messages[slot].number_of_attacks, 2);
    assert_eq!(g.fight_messages[slot].msg.len(), 2);
    let loaded = mud_game::fight::load_messages(&g.lib_dir).unwrap();
    let saved = loaded.iter().find(|s| s.a_type == 480).unwrap();
    assert_eq!(saved.number_of_attacks, 2);
    assert_eq!(saved.msg.len(), 2);
}
