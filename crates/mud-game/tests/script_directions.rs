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
    let root = std::env::temp_dir().join(format!("rustmud-script-directions-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn scripts_can_read_all_ten_exit_directions() {
    let mut f = fixture("exits"); let g = &mut f.game;
    let go = GoId::Room(1);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger { vnum: 65000, attach_type: dg::WLD_TRIGGER, ..Default::default() });
    let t = dg::read_trigger(g, nr).unwrap(); let ctx = dg::DgCtx { go, iid: t.iid };
    dg::add_trigger_at(g.ensure_script(go), t, -1);
    for (dir, name) in mud_data::tables::DIRS.iter().enumerate() {
        g.world.rooms[1].dir_option[dir] = Some(Box::new(mud_world::model::Exit {
            general_description: None, keyword: None, exit_info: 0, key: 100 + dir as u16,
            to_room_vnum: g.world.rooms[2].vnum as i32, to_room: 2,
        }));
        let result = dg::variables::var_subst(g, ctx, format!("%self.{name}(vnum)%").as_bytes());
        assert_eq!(result, g.world.rooms[2].vnum.to_string().as_bytes(), "direction={name}");
    }
}
