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
    let root = std::env::temp_dir().join(format!("rustmud-player-vars-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn player_globals_survive_attached_triggers_and_resaving() {
    use mud_game::{ch::Char, players_glue};
    use mud_world::players::{self, PlayerFile, PfVar};
    let mut f = fixture("roundtrip");
    let g = &mut f.game;
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: dg::MOB_TRIGGER, ..Default::default()
    });
    g.world.trig_map.insert(65000, nr);
    for enabled in [true, false] {
        g.config.script_players = enabled;
        let name = b"Tester";
        let index = players_glue::create_entry(g, name);
        let pf = PlayerFile {
            name: Some(name.to_vec()), triggers: vec![65000],
            vars: vec![PfVar { name: b"progress".to_vec(), value: b"finished".to_vec(), context: 42 }],
            ..Default::default()
        };
        players::write_pfile(&g.lib_dir, name, &players::save_char(&pf)).unwrap();
        for _ in 0..2 {
            let ch = g.chars.insert(Char::default());
            assert_eq!(players_glue::load_char_into(g, ch, name), Some(index));
            let sc = g.script_of(GoId::Char(ch)).unwrap();
            assert_eq!(sc.trig_list.len(), usize::from(enabled));
            assert_eq!(sc.global_vars.len(), 1, "saved globals were lost during login");
            assert_eq!(sc.global_vars[0].name, b"progress");
            assert_eq!(sc.global_vars[0].value, b"finished");
            assert_eq!(sc.global_vars[0].context, 42);
            g.ch_mut(ch).pfilepos = index as i32;
            players_glue::save_char(g, ch);
            players_glue::free_offline_char(g, ch);
        }
    }
}
