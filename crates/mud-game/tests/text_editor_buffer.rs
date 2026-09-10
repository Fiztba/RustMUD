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
    let root = std::env::temp_dir().join(format!("rustmud-text-editor-{}-{label}", std::process::id()));
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
fn saved_text_preserves_terminal_line_endings_in_memory() {
    let mut f = fixture("save"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, actor, b"news", 0, 0);
    let olc = g.olc.remove(&di).unwrap();
    mud_game::olc::tedit::tedit_string_cleanup(g, di, olc, Some(b"First\r\nSecond\r\n".to_vec()), true);
    assert_eq!(g.texts.news, b"First\r\nSecond\r\n");
    assert_eq!(std::fs::read(g.lib_dir.join("text/news")).unwrap(), b"First\nSecond\n");
}
#[test]
fn abort_does_not_restore_a_stale_shared_buffer() {
    let mut f = fixture("abort"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, actor, b"news", 0, 0);
    let olc = g.olc.remove(&di).unwrap();
    g.texts.news = b"New content\r\n".to_vec();
    mud_game::olc::tedit::tedit_string_cleanup(g, di, olc, Some(b"Old content\r\n".to_vec()), false);
    assert_eq!(g.texts.news, b"New content\r\n");
}

#[test]
fn same_file_is_excluded_but_other_files_remain_editable() {
    let mut f = fixture("parallel"); let g = &mut f.game;
    let first = player(g, b"First", 12345); let second = player(g, b"Second", 12346);
    g.ch_mut(first).level = LVL_IMPL; g.ch_mut(second).level = LVL_IMPL;
    let a = descriptor(g, first, ConState::Playing); let b = descriptor(g, second, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, first, b"news", 0, 0);
    mud_game::olc::tedit::do_tedit(g, second, b"news", 0, 0);
    assert_eq!(g.descriptors.get(b).unwrap().state, ConState::Playing);
    assert!(!g.olc.contains_key(&b));
    mud_game::olc::tedit::do_tedit(g, second, b"motd", 0, 0);
    assert_eq!(g.descriptors.get(b).unwrap().state, ConState::Tedit);
    let olc = g.olc.remove(&a).unwrap();
    mud_game::olc::tedit::tedit_string_cleanup(g, a, olc, None, false);
    mud_game::olc::tedit::do_tedit(g, first, b"news", 0, 0);
    assert_eq!(g.descriptors.get(a).unwrap().state, ConState::Tedit);
}
#[test]
fn failed_write_keeps_the_current_terminal_buffer() {
    let mut f = fixture("failure"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, actor, b"news", 0, 0);
    let mut olc = g.olc.remove(&di).unwrap();
    olc.storage = Some(g.lib_dir.to_string_lossy().as_bytes().to_vec());
    let original = g.texts.news.clone();
    mud_game::olc::tedit::tedit_string_cleanup(g, di, olc, Some(b"Replacement\r\n".to_vec()), true);
    assert_eq!(g.texts.news, original);
}
