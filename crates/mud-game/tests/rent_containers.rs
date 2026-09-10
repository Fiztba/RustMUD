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
    let root = std::env::temp_dir().join(format!("rustmud-container-recovery-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}


#[test]
fn children_survive_when_saved_container_cannot_be_equipped() {
    use mud_data::{flags, types::*};
    let mut f = fixture("equipment");
    let g = &mut f.game;
    for (locate, wear, worn) in [(WEAR_BODY as i32 + 1, 0, false), (NUM_WEARS as i32 + 20, 0, false), (WEAR_BODY as i32 + 1, 1u32 << flags::ITEM_WEAR_BODY, true)] {
        let ch = g.chars.insert(mud_game::ch::Char {
            name: Some(b"Tester".to_vec()),
            player_specials: Some(Box::new(Default::default())),
            ..Default::default()
        });
        let data = format!("1 1800000000 0 0 0 0\n#65535\nName: child\nWght: 3\nLoc : -1\n#65535\nName: bag\nWear: {wear} 0 0 0\nType: {}\nVals: 100 0 0 0\nWght: 7\nLoc : {locate}\n$~\n", flags::ITEM_CONTAINER);
        let path = g.lib_dir.join("plrobjs/P-T/tester.objs");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
        mud_game::objsave::crash_load(g, ch);
        assert_eq!(g.ch(ch).carrying.len(), if worn { 0 } else { 1 });
        let bag = if worn { g.ch(ch).equipment[WEAR_BODY].unwrap() } else { g.ch(ch).carrying[0] };
        assert_eq!(g.obj(bag).contains.len(), 1, "fallback bag lost its child");
        let child = g.obj(bag).contains[0];
        assert_eq!(g.obj(child).name.as_deref(), Some(b"child".as_slice()));
        assert_eq!(g.obj(bag).weight, 10);
        assert_eq!(g.ch(ch).carry_weight, if worn { 0 } else { 10 });
    }
}

#[test]
fn children_of_missing_final_container_are_returned_to_inventory() {
    let mut f = fixture("missing");
    let g = &mut f.game;
    let missing = (0..65535).find(|&n| g.world.real_object(n).is_none()).unwrap();
    let ch = g.chars.insert(mud_game::ch::Char {
        name: Some(b"Tester".to_vec()),
        player_specials: Some(Box::new(Default::default())),
        ..Default::default()
    });
    let data = format!("1 1800000000 0 0 0 0\n#65535\nName: child\nWght: 3\nLoc : -1\n#{missing}\nLoc : 0\n$~\n");
    let path = g.lib_dir.join("plrobjs/P-T/tester.objs");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
    mud_game::objsave::crash_load(g, ch);
    assert_eq!(g.ch(ch).carrying.len(), 1, "orphaned child disappeared");
    assert_eq!(g.obj(g.ch(ch).carrying[0]).name.as_deref(), Some(b"child".as_slice()));
}
