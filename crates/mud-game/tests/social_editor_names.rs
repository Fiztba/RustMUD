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
    let root = std::env::temp_dir().join(format!("rustmud-social-names-{}-{label}", std::process::id()));
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

use mud_game::olc::{OlcData, aedit::*};
#[test]
fn social_rename_rejects_existing_commands() {
    let mut f = fixture("collision"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Aedit);
    let original = g.socials[0].command.clone();
    for name in [b"look".to_vec(), b"LOOK".to_vec(), g.socials[1].command.clone()] {
        let mut olc = OlcData::new(); olc.zone_num = 0; olc.action = Some(Box::new(g.socials[0].clone()));
        olc.mode = AEDIT_ACTION_NAME;
        let olc = aedit_parse(g, di, olc, &name).unwrap();
        assert_eq!(olc.action.unwrap().command, original);
    }
}
#[test]
fn social_identifiers_reject_tabs() {
    let mut f = fixture("tabs"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Aedit);
    for mode in [AEDIT_ACTION_NAME, AEDIT_SORT_AS] {
        let original = g.socials[0].clone();
        let mut olc = OlcData::new(); olc.zone_num = 0; olc.action = Some(Box::new(original.clone())); olc.mode = mode;
        let olc = aedit_parse(g, di, olc, b"broken\tname").unwrap();
        let action = olc.action.unwrap();
        assert_eq!((action.command, action.sort_as), (original.command, original.sort_as));
    }
}

#[test]
fn own_name_and_unused_names_remain_editable() {
    let mut f = fixture("valid"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Aedit);
    for name in [g.socials[0].command.clone(), b"reviewwave".to_vec()] {
        let mut olc = OlcData::new(); olc.zone_num = 0; olc.action = Some(Box::new(g.socials[0].clone()));
        olc.mode = AEDIT_ACTION_NAME;
        let mut olc = aedit_parse(g, di, olc, &name).unwrap();
        assert_eq!(olc.action.as_ref().unwrap().command, name);
        assert_eq!(olc.mode, AEDIT_MAIN_MENU);
        olc.mode = AEDIT_SORT_AS;
        let olc = aedit_parse(g, di, olc, b"revieww").unwrap();
        assert_eq!(olc.action.as_ref().unwrap().sort_as, b"revieww");
    }
}
