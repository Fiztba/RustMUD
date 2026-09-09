use mud_game::{game::Game, dg::{self, GoId}};
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
    let root = std::env::temp_dir().join(format!("rustmud-login-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn rejected_wait_does_not_leave_a_trigger_running_forever() {
    let mut fixture = fixture("rejected-wait");
    let g = &mut fixture.game;
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, name: Some(b"wait lifecycle".to_vec()),
        attach_type: dg::WLD_TRIGGER,
        cmdlist: vec![b"wait %missing%".to_vec(), b"return 7".to_vec()],
        ..Default::default()
    });
    let trigger = dg::read_trigger(g, nr).unwrap();
    let iid = trigger.iid;
    let go = GoId::Room(0);
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    assert_eq!(dg::driver::script_driver(g, go, iid, dg::TRIG_NEW), 7);
    assert_eq!(g.trig(go, iid).unwrap().depth, 0);
    assert_eq!(g.trig(go, iid).unwrap().wait_event, None);
    assert_eq!(g.dg_script_depth, 0);
    assert_eq!(dg::driver::script_driver(g, go, iid, dg::TRIG_NEW), 7);

    g.world.triggers[nr as usize].cmdlist = vec![
        b"wait 1".to_vec(), b"set resumed yes".to_vec(), b"global resumed".to_vec(),
    ];
    dg::driver::script_driver(g, go, iid, dg::TRIG_NEW);
    let event = g.trig(go, iid).unwrap().wait_event.expect("valid wait must schedule");
    assert_eq!(g.trig(go, iid).unwrap().depth, 1);
    assert!(g.script_of(go).unwrap().global_vars.is_empty());
    assert_eq!(g.dg_script_depth, 0);
    dg::driver::trig_wait_event(g, go, iid, event);
    assert_eq!(g.trig(go, iid).unwrap().depth, 0);
    assert_eq!(g.trig(go, iid).unwrap().wait_event, None);
    assert_eq!(g.script_of(go).unwrap().global_vars[0].value, b"yes");
    assert_eq!(g.dg_script_depth, 0);
}
