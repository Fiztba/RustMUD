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
    let root = std::env::temp_dir().join(format!("rustmud-reset-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn reset_does_not_attach_to_object_purged_by_load_trigger() {
    use mud_world::model::{Trigger, ZoneCommand};
    let mut f = fixture("purged");
    let g = &mut f.game;
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(Trigger {
        vnum: 65000, attach_type: dg::OBJ_TRIGGER, trigger_type: dg::OTRIG_LOAD,
        narg: 100, cmdlist: vec![b"opurge self".to_vec()], ..Default::default()
    });
    g.world.trig_map.insert(65000, nr);
    let obj = 0;
    g.world.obj_protos[obj].proto_script = vec![65000];
    let before = g.obj_counts[obj];
    g.world.zones[0].cmds = vec![
        ZoneCommand { command: b'O', arg1: obj as i32, arg2: before + 1, arg3: 0, ..Default::default() },
        ZoneCommand { command: b'T', arg1: dg::OBJ_TRIGGER, arg2: nr as i32, ..Default::default() },
    ];
    mud_game::db::reset_zone(g, 0);
    assert_eq!(g.obj_counts[obj], before);
}

#[test]
fn missing_put_container_does_not_leave_an_unplaced_object() {
    use mud_world::model::ZoneCommand;
    let mut f = fixture("missing");
    let g = &mut f.game;
    let container = (0..g.world.obj_protos.len()).find(|&r| g.obj_counts[r] == 0).unwrap();
    let obj = if container == 0 { 1 } else { 0 };
    let before = g.obj_counts[obj];
    let total = g.object_list.len();
    g.world.zones[0].cmds = vec![ZoneCommand {
        command: b'P', arg1: obj as i32, arg2: before + 1, arg3: container as i32,
        ..Default::default()
    }];
    mud_game::db::reset_zone(g, 0);
    assert_eq!(g.obj_counts[obj], before, "failed put leaked a live object");
    assert_eq!(g.object_list.len(), total);
}
