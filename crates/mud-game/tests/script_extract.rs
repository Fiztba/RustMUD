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
    let root = std::env::temp_dir().join(format!("rustmud-script-extract-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn extract_stops_at_the_end_of_the_input_and_continues_the_script() {
    let mut f = fixture("words"); let g = &mut f.game;
    let go = GoId::Room(1);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: dg::WLD_TRIGGER,
        cmdlist: vec![b"extract result 2147483647 one two".to_vec(), b"global result".to_vec(), b"set continued yes".to_vec(), b"global continued".to_vec()],
        ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap(); let iid = t.iid;
    dg::add_trigger_at(g.ensure_script(go), t, -1);
    dg::driver::script_driver(g, go, iid, dg::TRIG_NEW);
    let vars = &g.script_of(go).unwrap().global_vars;
    assert!(vars.iter().any(|v| v.name == b"result" && v.value.is_empty()));
    assert!(vars.iter().any(|v| v.name == b"continued" && v.value == b"yes"));
    for (input, words) in [("", vec![]), ("ONE", vec!["one"]), ("  ONE  Two   THREE ", vec!["one", "two", "three"])] {
        for n in [1, 2, 3, 4, i32::MAX] {
            g.script_of_mut(go).unwrap().global_vars.clear();
            g.world.triggers[nr as usize].cmdlist[0] = format!("extract result {n} {input}").into_bytes();
            dg::driver::script_driver(g, go, iid, dg::TRIG_NEW);
            let vars = &g.script_of(go).unwrap().global_vars;
            let expected = words.get(n as usize - 1).copied().unwrap_or("");
            assert!(vars.iter().any(|v| v.name == b"result" && v.value == expected.as_bytes()), "input={input}, n={n}");
            assert!(vars.iter().any(|v| v.name == b"continued" && v.value == b"yes"));
        }
    }
}
