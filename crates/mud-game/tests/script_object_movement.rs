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
    let root = std::env::temp_dir().join(format!("rustmud-object-move-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}


use mud_data::{flags, ids::ObjId, types::*};
use mud_game::{ch::{Char, PlayerSpecials}, handler::*};

fn object(g: &mut Game, name: &[u8], weight: i32) -> ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(name.to_vec());
    obj.weight = weight;
    let id = g.objs.insert(obj);
    g.object_list.push_back(id);
    id
}

#[test]
fn script_moves_preserve_location_and_accounting() {
    let mut f = fixture("locations");
    let g = &mut f.game;
    g.world.rooms[0].room_flags = [0; 4];
    g.world.rooms[1].room_flags = [0; 4];
    let dest = g.world.rooms[1].vnum;
    let ch = g.chars.insert(Char {
        name: Some(b"Carrier".to_vec()),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    });
    char_to_room(g, ch, 0);
    let bag = object(g, b"bag", 4);
    g.obj_mut(bag).type_flag = flags::ITEM_CONTAINER;
    g.obj_mut(bag).values[0] = 100;
    obj_to_char(g, bag, ch);
    for source in 0..8 {
        let item = object(g, b"moveitem", 3);
        match source % 4 {
            0 => obj_to_room(g, item, 0),
            1 => obj_to_char(g, item, ch),
            2 => obj_to_obj(g, item, bag),
            _ => { g.obj_mut(item).type_flag = flags::ITEM_LIGHT; g.obj_mut(item).values[2] = 10; equip_char(g, ch, item, WEAR_LIGHT); }
        }
        let weight = g.ch(ch).carry_weight;
        let light = g.rooms[0].light;
        dg::objcmd::obj_command_interpreter(g, item, b"omove 999999");
        assert_eq!(g.ch(ch).carry_weight, weight);
        assert_eq!(g.rooms[0].light, light);
        match source % 4 {
            0 => assert_eq!(g.obj(item).in_room, 0, "invalid destination must not unlink"),
            1 => assert_eq!(g.obj(item).carried_by, Some(ch)),
            2 => assert_eq!(g.obj(item).in_obj, Some(bag)),
            _ => assert_eq!(g.obj(item).worn_by, Some(ch)),
        }
        let uid = dg::obj_script_id(g, item);
        if source < 4 {
            dg::wldcmd::wld_command_interpreter(g, 0, format!("wmove }}{uid} {dest}").as_bytes());
        } else {
            dg::objcmd::obj_command_interpreter(g, item, format!("omove {dest}").as_bytes());
        }
        assert_eq!(g.obj(item).in_room, 1);
        assert_eq!(g.obj(item).in_obj, None);
        assert_eq!(g.obj(item).carried_by, None);
        assert_eq!(g.obj(item).worn_by, None);
        assert!(!g.ch(ch).carrying.contains(&item));
        assert!(!g.obj(bag).contains.contains(&item));
        assert!(!g.ch(ch).equipment.contains(&Some(item)));
        assert!(!g.rooms[0].contents.contains(&item));
        assert_eq!(g.ch(ch).carry_weight, 4);
        assert_eq!(g.ch(ch).carry_items, 1);
        assert_eq!(g.obj(bag).weight, 4);
        if source % 4 == 3 { assert_eq!(g.rooms[0].light, light - 1); }
        extract_obj(g, item);
        assert!(!g.rooms[1].contents.contains(&item));
    }
    // A character outside the world is not a valid omove destination.
    char_from_room(g, ch);
    g.character_list.push_back(ch);
    dg::objcmd::obj_command_interpreter(g, bag, b"omove Carrier");
    assert_eq!(g.obj(bag).carried_by, Some(ch));
    char_to_room(g, ch, 0);
    // Protected destinations are rejected without changing ownership.
    g.world.rooms[1].room_flags[flags::ROOM_HOUSE / 32] |= 1 << (flags::ROOM_HOUSE % 32);
    dg::objcmd::obj_command_interpreter(g, bag, format!("omove {dest}").as_bytes());
    assert_eq!(g.obj(bag).carried_by, Some(ch));
    assert_eq!(g.ch(ch).carry_weight, 4);
    // wmove all affects only the room's contents, leaving inventory alone.
    let floor = object(g, b"flooritem", 1);
    obj_to_room(g, floor, 0);
    dg::wldcmd::wld_command_interpreter(g, 0, format!("wmove all {dest}").as_bytes());
    assert_eq!(g.obj(floor).in_room, 1);
    assert_eq!(g.obj(bag).carried_by, Some(ch));
}

