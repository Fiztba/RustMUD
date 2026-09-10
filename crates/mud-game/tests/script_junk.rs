use mud_game::{game::Game, dg};
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
    let root = std::env::temp_dir().join(format!("rustmud-junk-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn junk_selects_one_named_item_or_all_matching_items() {
    use mud_game::{ch::Char, obj::Obj, handler};
    use mud_data::{flags, types::*};
    let mut f = fixture("selection");
    let g = &mut f.game;
    for (arg, remaining_gems, remaining_sword) in [
        (b"gem".as_slice(), 2, true), (b"all.gem", 0, true), (b"all", 0, false),
        (b"missing", 3, true),
    ] {
        let mut npc = Char::default();
        npc.act.set(flags::MOB_ISNPC);
        let ch = g.chars.insert(npc);
        let mut gems = Vec::new();
        for _ in 0..3 {
            let o = g.objs.insert(Obj { name: Some(b"gem".to_vec()), weight: 1, ..Default::default() });
            gems.push(o);
            handler::obj_to_char(g, o, ch);
        }
        handler::obj_from_char(g, gems[0]);
        handler::equip_char(g, ch, gems[0], WEAR_HOLD);
        let sword = g.objs.insert(Obj { name: Some(b"sword".to_vec()), weight: 1, ..Default::default() });
        handler::obj_to_char(g, sword, ch);
        dg::mobcmd::do_mjunk(g, ch, arg);
        assert_eq!(gems.iter().filter(|&&o| g.try_obj(o).is_some()).count(), remaining_gems, "{arg:?}");
        assert_eq!(g.try_obj(sword).is_some(), remaining_sword, "unrelated item changed for {arg:?}");
        if arg == b"gem" { assert!(g.try_obj(gems[0]).is_none(), "named selection should prefer equipment"); }
    }
}
