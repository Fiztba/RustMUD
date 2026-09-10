use mud_game::{game::Game, dg::{self, GoId}};
use std::path::{Path, PathBuf};
use mud_game::{handler, limits, ch::{Char, PlayerSpecials}};
use mud_data::{flags, types::*};

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
    let root = std::env::temp_dir().join(format!("rustmud-objecttimer-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn object(g: &mut Game, kind: i32) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.type_flag = kind;
    obj.weight = 1;
    let oid = g.objs.insert(obj);
    g.object_list.push_front(oid);
    oid
}

#[test]
fn script_timers_tick_once_in_every_object_location() {
    let mut f = fixture("locations");
    let g = &mut f.game;
    let ch = g.chars.insert(Char {
        level: LVL_IMPL, position: POS_STANDING,
        player_specials: Some(Box::new(PlayerSpecials::default())), ..Default::default()
    });
    g.character_list.push_front(ch);
    handler::char_to_room(g, ch, 0);
    let bag = object(g, flags::ITEM_CONTAINER);
    handler::obj_to_char(g, bag, ch);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: dg::OBJ_TRIGGER, trigger_type: dg::OTRIG_TIMER,
        cmdlist: vec![b"eval fires %fires% + 1".to_vec(), b"global fires".to_vec()],
        ..Default::default()
    });
    for initial in [1, 2] {
        let mut objects = Vec::new();
        for location in 0..4 {
            let oid = object(g, flags::ITEM_TREASURE);
            g.obj_mut(oid).timer = initial;
            let trigger = dg::read_trigger(g, nr).unwrap();
            dg::add_trigger_at(g.ensure_script(GoId::Obj(oid)), trigger, -1);
            dg::add_var(&mut g.ensure_script(GoId::Obj(oid)).global_vars, b"fires", b"0", 0);
            match location {
                0 => handler::obj_to_room(g, oid, 0),
                1 => handler::obj_to_char(g, oid, ch),
                2 => { assert!(handler::equip_char(g, ch, oid, WEAR_HOLD)); }
                _ => handler::obj_to_obj(g, oid, bag),
            }
            objects.push(oid);
        }
        for elapsed in 1..=initial + 1 {
            limits::point_update(g);
            for &oid in &objects {
                assert_eq!(g.obj(oid).timer, (initial - elapsed).max(0));
                let vars = &g.script_of(GoId::Obj(oid)).unwrap().global_vars;
                let fires = vars.iter().find(|v| v.name == b"fires").unwrap();
                assert_eq!(fires.value, if elapsed < initial { b"0" } else { b"1" });
            }
        }
        for oid in objects { handler::extract_obj(g, oid); }
    }
}


