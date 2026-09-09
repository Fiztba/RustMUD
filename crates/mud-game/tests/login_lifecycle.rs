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
    let root = std::env::temp_dir().join(format!("rustmud-login-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: LVL_IMPL,
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
fn reconnecting_while_switched_does_not_free_the_borrowed_body() {
    let mut fixture = fixture("original");
    let g = &mut fixture.game;
    let original = player(g, b"Tester", 12345);
    let mut npc = Char::default();
    npc.act.set(flags::MOB_ISNPC);
    let body = g.chars.insert(npc);
    g.character_list.push_front(original);
    g.character_list.push_front(body);
    mud_game::handler::char_to_room(g, original, 0);
    mud_game::handler::char_to_room(g, body, 0);
    let old = descriptor(g, body, ConState::Playing);
    g.descriptors.get_mut(old).unwrap().original = Some(original);
    let fresh = player(g, b"Tester", 12345);
    let new = descriptor(g, fresh, ConState::Password);
    mud_game::login::nanny(g, new, b"secret");
    mud_game::run::close_socket(g, old);
    assert!(g.try_ch(body).is_some(), "closing old connection freed a live NPC");
    assert_eq!(g.ch(body).desc, None);
    assert_eq!(g.descriptors.get(new).unwrap().character, Some(original));
    assert_eq!(g.ch(original).desc, Some(new));
}

#[test]
fn reconnecting_player_reclaims_body_from_switched_immortal() {
    let mut fixture = fixture("borrowed");
    let g = &mut fixture.game;
    let immortal = player(g, b"Immortal", 12344);
    let body = player(g, b"Tester", 12345);
    g.character_list.push_front(immortal);
    g.character_list.push_front(body);
    mud_game::handler::char_to_room(g, immortal, 0);
    mud_game::handler::char_to_room(g, body, 0);
    let old = descriptor(g, body, ConState::Playing);
    g.descriptors.get_mut(old).unwrap().original = Some(immortal);
    let fresh = player(g, b"Tester", 12345);
    let new = descriptor(g, fresh, ConState::Password);
    mud_game::login::nanny(g, new, b"secret");
    assert_eq!(g.descriptors.get(new).unwrap().character, Some(body));
    assert_eq!(g.descriptors.get(new).unwrap().state, ConState::Playing);
    assert_eq!(g.descriptors.get(old).unwrap().character, Some(immortal));
    assert_eq!(g.descriptors.get(old).unwrap().original, None);
    assert_eq!(g.ch(immortal).desc, Some(old));
    assert_eq!(g.ch(body).desc, Some(new));
    assert!(g.try_ch(fresh).is_none());
}
