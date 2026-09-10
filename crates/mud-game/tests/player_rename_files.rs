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
    let root = std::env::temp_dir().join(format!("rustmud-rename-files-{}-{label}", std::process::id()));
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
fn failed_file_rename_keeps_player_identity_and_original_files() {
    let mut f = fixture("blocked"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let victim = player(g, b"Oldname", 12346);
    descriptor(g, actor, ConState::Playing);
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"oldname".to_vec(), id: 12346, level: 10, flags: 0, last: g.now,
    });
    use mud_world::players::{get_filename, FileKind};
    let old = g.lib_dir.join(get_filename(FileKind::Objs, b"Oldname").unwrap());
    let new = g.lib_dir.join(get_filename(FileKind::Objs, b"Newname").unwrap());
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, b"original inventory").unwrap();
    std::fs::create_dir_all(&new).unwrap();
    assert!(!mud_game::act::wizset::change_player_name(g, actor, victim, b"Newname"));
    assert_eq!(g.ch(victim).get_name(), b"Oldname");
    assert_eq!(g.player_table.last().unwrap().name, b"oldname");
    assert_eq!(std::fs::read(old).unwrap(), b"original inventory");
}

fn all_files_case(locked: bool) {
    let mut f = fixture(if locked { "locked" } else { "success" }); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let victim = player(g, b"Oldname", 12346);
    descriptor(g, actor, ConState::Playing);
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"oldname".to_vec(), id: 12346, level: 10, flags: 0, last: g.now,
    });
    use mud_world::players::{get_filename, FileKind};
    let mut paths = vec![];
    for kind in [FileKind::Plr, FileKind::Objs, FileKind::Text, FileKind::Vars] {
        let old = g.lib_dir.join(get_filename(kind, b"Oldname").unwrap());
        let new = g.lib_dir.join(get_filename(kind, b"Newname").unwrap());
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        let data = format!("original file {}", paths.len()).into_bytes();
        std::fs::write(&old, &data).unwrap(); paths.push((old, new, data));
    }
    #[cfg(windows)]
    let lock = if locked {
        use std::os::windows::fs::OpenOptionsExt;
        Some(std::fs::OpenOptions::new().read(true).share_mode(0).open(&paths[1].0).unwrap())
    } else { None };
    assert_eq!(mud_game::act::wizset::change_player_name(g, actor, victim, b"Newname"), !locked);
    #[cfg(windows)] drop(lock);
    assert_eq!(g.ch(victim).get_name(), if locked { b"Oldname" } else { b"Newname" });
    assert_eq!(g.player_table.last().unwrap().name, if locked { b"oldname" } else { b"newname" });
    for (old, new, data) in paths {
        assert_eq!(std::fs::read(if locked { &old } else { &new }).unwrap(), data);
        assert!(!if locked { new } else { old }.exists());
    }
}
#[test] fn successful_rename_moves_all_four_files() { all_files_case(false); }
#[cfg(windows)]
#[test] fn later_file_failure_rolls_back_earlier_move() { all_files_case(true); }
