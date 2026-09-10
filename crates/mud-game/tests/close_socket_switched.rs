use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
use mud_net::descriptor::Descriptor;
use mud_world::players::{get_filename, FileKind};
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
    let root = std::env::temp_dir().join(format!("rustmud-close-socket-switched-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: 10, position: POS_STANDING,
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

fn pfile(g: &Game, name: &[u8]) -> PathBuf {
    g.lib_dir.join(get_filename(FileKind::Plr, name).unwrap())
}

fn viewer_output(g: &Game, di: usize) -> String {
    String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).into_owned()
}

#[test]
fn link_loss_while_switched_saves_and_announces_the_original_body() {
    let mut f = fixture("switched"); let g = &mut f.game;
    let imm = player(g, b"Godling", 12345); g.ch_mut(imm).level = LVL_IMPL; g.ch_mut(imm).pfilepos = 0;
    let viewer = player(g, b"Viewer", 12346);
    for ch in [imm, viewer] { mud_game::handler::char_to_room(g, ch, 0); g.character_list.push_back(ch); }
    g.rooms[0].light = 1;
    let viewer_di = descriptor(g, viewer, ConState::Playing);
    let body = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, body, 1);
    let di = descriptor(g, imm, ConState::Playing);
    let keyword = g.ch(body).name.clone().unwrap();
    let keyword = keyword.split(|b| *b == b' ').next().unwrap().to_vec();
    mud_game::act::wizard::do_switch(g, imm, &keyword, 0, 0);
    { let d = g.descriptors.get(di).unwrap(); assert_eq!(d.character, Some(body)); assert_eq!(d.original, Some(imm)); }
    assert_eq!(g.ch(imm).desc, None); assert_eq!(g.ch(body).desc, Some(di));
    assert!(!pfile(g, b"Godling").exists());
    g.descriptors.get_mut(viewer_di).unwrap().output.clear();

    mud_game::run::close_socket(g, di);

    assert!(pfile(g, b"Godling").exists(), "the immortal's player file should be saved on link loss");
    let out = viewer_output(g, viewer_di);
    assert!(out.contains("Godling has lost"), "the lost-link message should go to the immortal's room, got {out:?}");
    assert_eq!(g.ch(imm).desc, None);
    assert_eq!(g.ch(body).desc, None);
    assert!(g.descriptors.get(di).is_none());
    assert!(g.rooms[0].people.contains(&imm));
    assert!(g.rooms[1].people.contains(&body), "the mob body stays where it was, just no longer controlled");
}

#[test]
fn plain_link_loss_saves_and_announces_the_player() {
    let mut f = fixture("plain"); let g = &mut f.game;
    let actor = player(g, b"Wanderer", 12345); g.ch_mut(actor).pfilepos = 0;
    let viewer = player(g, b"Viewer", 12346);
    for ch in [actor, viewer] { mud_game::handler::char_to_room(g, ch, 0); g.character_list.push_back(ch); }
    g.rooms[0].light = 1;
    let viewer_di = descriptor(g, viewer, ConState::Playing);
    let di = descriptor(g, actor, ConState::Playing);
    assert!(!pfile(g, b"Wanderer").exists());

    mud_game::run::close_socket(g, di);

    assert!(pfile(g, b"Wanderer").exists());
    let out = viewer_output(g, viewer_di);
    assert!(out.contains("Wanderer has lost"), "got {out:?}");
    assert_eq!(g.ch(actor).desc, None);
    assert!(g.descriptors.get(di).is_none());
    assert!(g.rooms[0].people.contains(&actor));
}
