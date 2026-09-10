use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
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
    let root = std::env::temp_dir().join(format!("rustmud-remote-storage-{}-{label}", std::process::id()));
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

use mud_game::dg::{self, GoId};
#[test]
fn remote_creates_variable_storage_on_targets_without_triggers() {
    let mut f = fixture("targets"); let g = &mut f.game;
    let pc = player(g, b"Player", 12345); g.character_list.push_back(pc);
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    let obj = mud_game::db::read_object(g, 0).unwrap();
    for target in [GoId::Char(pc), GoId::Char(mob), GoId::Obj(obj), GoId::Room(2)] {
        dg::extract_script(g, target);
        assert!(g.script_of(target).is_none());
        let uid = match target {
            GoId::Char(c) => dg::char_script_id(g, c),
            GoId::Obj(o) => dg::obj_script_id(g, o),
            GoId::Room(r) => dg::room_script_id(g, r),
        };
      for context in [7, 8] {
        let nr = g.world.triggers.len() as u16;
        g.world.triggers.push(mud_world::model::Trigger {
            vnum: 65000, attach_type: dg::WLD_TRIGGER,
            cmdlist: vec![format!("context {context}").into_bytes(), b"set marker 42".to_vec(),
                format!("remote marker {uid}").into_bytes(), b"set marker 43".to_vec(), format!("remote marker {uid}").into_bytes()],
            ..Default::default()
        });
        let trigger = dg::read_trigger(g, nr).unwrap(); let iid = trigger.iid;
        dg::add_trigger_at(g.ensure_script(GoId::Room(1)), trigger, -1);
        dg::driver::script_driver(g, GoId::Room(1), iid, dg::TRIG_NEW);
        let vars = &g.script_of(target).expect("remote discarded variable for a target without scripts").global_vars;
        let expected_context = if target == GoId::Char(pc) { 0 } else { context };
        assert!(vars.iter().any(|v| v.name == b"marker" && v.value == b"43" && v.context == expected_context), "target {target:?}, context {context}: {vars:?}");
        assert_eq!(vars.iter().filter(|v| v.name == b"marker").count(), if context == 8 && target != GoId::Char(pc) { 2 } else { 1 });
        if context == 7 {
            dg::add_var(&mut g.ensure_script(target).global_vars, b"keep", b"unchanged", 0);
        } else {
            assert!(vars.iter().any(|v| v.name == b"keep" && v.value == b"unchanged"));
        }
      }
    }
}
