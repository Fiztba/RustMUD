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
    let root = std::env::temp_dir().join(format!("rustmud-objparse-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn missing_and_malformed_records_do_not_change_previous_object() {
    let mut f = fixture("missing");
    let g = &mut f.game;
    let vnum = g.world.obj_protos[0].vnum;
    let missing = (0..65535).find(|&n| g.world.real_object(n).is_none()).unwrap();
    for header in [format!("#{missing}"), "#invalid".to_string(), "#-1".to_string(), format!("#{}", 65536_u32 + vnum as u32)] {
        let data = format!("#{vnum}\nName: kept\nWght: 7\nLoc : -1\n{header}\nName: discarded\nWght: 99\nLoc : -4\nADes:\n$~ is description text\n#{vnum}\nName: fake\n~\nEDes:\n#{vnum}\n~\n#{vnum}\n~\n#{vnum}\nName: last\n$~\n");
        let records = mud_game::objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(data.as_bytes()));
        assert_eq!(records.len(), 2);
        assert_eq!(g.obj(records[0].obj).name.as_deref(), Some(b"kept".as_slice()));
        assert_eq!(g.obj(records[0].obj).weight, 7);
        assert_eq!(records[0].locate, -1);
        assert_eq!(g.obj(records[1].obj).name.as_deref(), Some(b"last".as_slice()));
        assert_eq!(records[1].locate, 0);
    }
    for ending in ["", "$~\n"] {
        let data = format!("#{missing}\nName: skipped\n#65535\nName: unique\nLoc : 2\n{ending}");
        let records = mud_game::objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(data.as_bytes()));
        assert_eq!(records.len(), 1);
        assert_eq!(g.obj(records[0].obj).name.as_deref(), Some(b"unique".as_slice()));
        assert_eq!(records[0].locate, 2);
    }

}
