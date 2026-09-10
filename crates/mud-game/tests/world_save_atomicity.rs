use mud_game::game::Game;
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
    let root = std::env::temp_dir().join(format!("rustmud-world-save-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[cfg(windows)]
fn hold_source(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).share_mode(3).open(path).unwrap()
}
#[cfg(windows)]
#[test]
fn failed_index_install_keeps_the_previous_file() {
    let mut f = fixture("add"); let g = &mut f.game;
    let dir = g.lib_dir.join("world/wld"); let index = dir.join("index");
    let original = std::fs::read(&index).unwrap();
    let lock = hold_source(&dir.join("newindex"));
    mud_game::olc::genzon::create_world_index(g, 65534, "wld");
    assert_eq!(std::fs::read(&index).ok().as_deref(), Some(original.as_slice()));
    drop(lock);
    mud_game::olc::genzon::create_world_index(g, 65534, "wld");
    assert!(std::fs::read_to_string(&index).unwrap().contains("65534.wld"));
}
#[cfg(windows)]
#[test]
fn failed_index_removal_keeps_both_world_indexes() {
    let mut f = fixture("remove"); let g = &mut f.game;
    let dir = g.lib_dir.join("world/wld");
    for name in ["index", "index.mini"] { std::fs::write(dir.join(name), b"123.wld\n$\n").unwrap(); }
    let lock = hold_source(&dir.join("newindex.mini"));
    assert!(!mud_game::olc::genzon::remove_world_index(g, 123, "wld"));
    for name in ["index", "index.mini"] { assert_eq!(std::fs::read(dir.join(name)).ok().as_deref(), Some(b"123.wld\n$\n".as_slice())); }
    drop(lock);
    assert!(mud_game::olc::genzon::remove_world_index(g, 123, "wld"));
    for name in ["index", "index.mini"] { assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"$\n"); }
}
#[cfg(windows)]
#[test]
fn failed_trigger_install_keeps_the_previous_zone_file() {
    let mut f = fixture("triggers"); let g = &mut f.game;
    let vnum = g.world.triggers[0].vnum;
    let zone = g.world.zones.iter().position(|z| z.bot <= vnum && vnum <= z.top).unwrap();
    let dir = g.lib_dir.join("world/trg"); let old = dir.join(format!("{}.trg", g.world.zones[zone].number));
    let original = std::fs::read(&old).unwrap();
    let lock = hold_source(&dir.join(format!("{}.new", g.world.zones[zone].number)));
    let mut olc = mud_game::olc::OlcData::new(); mud_game::olc::trigedit::trigedit_setup_existing(g, &mut olc, 0);
    olc.number = vnum as i32; olc.zone_num = zone as i32;
    olc.trig.as_mut().unwrap().name = Some(b"updated trigger".to_vec());
    mud_game::olc::trigedit::trigedit_save(g, usize::MAX, &mut olc);
    assert_eq!(std::fs::read(&old).ok().as_deref(), Some(original.as_slice()));
    drop(lock);
    mud_game::olc::trigedit::trigedit_save(g, usize::MAX, &mut olc);
    assert!(std::fs::read_to_string(&old).unwrap().contains("updated trigger"));
}

#[test]
fn index_replacement_preserves_order_and_removes_only_the_selected_entry() {
    let mut f = fixture("normal"); let g = &mut f.game;
    for kind in ["qst", "zon", "wld", "mob", "obj", "shp", "trg"] {
        let dir = g.lib_dir.join("world").join(kind);
        let original = format!("1.{kind}\n3.{kind}\n$\n");
        for name in ["index", "index.mini"] { std::fs::write(dir.join(name), &original).unwrap(); }
        mud_game::olc::genzon::create_world_index(g, 2, kind);
        let expected = format!("1.{kind}\n2.{kind}\n3.{kind}\n$\n");
        assert_eq!(std::fs::read_to_string(dir.join("index")).unwrap(), expected);
        mud_game::olc::genzon::create_world_index(g, 2, kind);
        assert_eq!(std::fs::read_to_string(dir.join("index")).unwrap(), expected);
        assert!(mud_game::olc::genzon::remove_world_index(g, 1, kind));
        assert_eq!(std::fs::read_to_string(dir.join("index")).unwrap(), format!("2.{kind}\n3.{kind}\n$\n"));
        assert_eq!(std::fs::read_to_string(dir.join("index.mini")).unwrap(), format!("3.{kind}\n$\n"));
    }
}
