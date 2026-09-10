use mud_data::types::*;
use mud_game::{ch::{Char, PlayerSpecials}, game::Game};
use mud_net::descriptor::Descriptor;
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
    let root = std::env::temp_dir().join(format!("rustmud-global-search-{}-{label}", std::process::id()));
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

fn descriptor(g: &mut Game, ch: mud_data::ids::CharId, state: ConState) -> usize {
    let mut d = Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = state;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    di
}

fn object(g: &mut Game) -> mud_data::ids::ObjId {
    let mut o = mud_game::obj::create_obj(); o.name = Some(b"reviewtarget".to_vec());
    let oid = g.objs.insert(o); g.object_list.push_front(oid); oid
}
#[test]
fn world_object_search_does_not_recount_inventory_or_room_objects() {
    use mud_game::handler::*;
    let mut f = fixture("objects"); let g = &mut f.game;
    let ch = player(g, b"Viewer", 12345); char_to_room(g, ch, 0); g.rooms[0].light = 1; g.rooms[1].light = 1;
    let remote = object(g); obj_to_room(g, remote, 1);
    let room = object(g); obj_to_room(g, room, 0);
    let carried = object(g); obj_to_char(g, carried, ch);
    let mut number = 3; assert_eq!(get_obj_vis_counted(g, ch, b"reviewtarget", &mut number), Some(remote));
    let (_, _, got) = generic_find(g, ch, b"3.reviewtarget", FIND_OBJ_INV | FIND_OBJ_ROOM | FIND_OBJ_WORLD);
    assert_eq!(got, Some(remote));
    let mut number = 4; assert_eq!(get_obj_vis_counted(g, ch, b"reviewtarget", &mut number), None); assert_eq!(number, 1);
}
#[test]
fn combined_character_and_object_searches_share_one_countdown() {
    use mud_game::handler::*;
    let mut f = fixture("characters"); let g = &mut f.game;
    let ch = player(g, b"Viewer", 12345); char_to_room(g, ch, 0); g.rooms[0].light = 1; g.rooms[1].light = 1;
    let remote = mud_game::db::read_mobile(g, 0).unwrap(); g.ch_mut(remote).name = Some(b"reviewtarget".to_vec()); char_to_room(g, remote, 1);
    let local = mud_game::db::read_mobile(g, 0).unwrap(); g.ch_mut(local).name = Some(b"reviewtarget".to_vec()); char_to_room(g, local, 0);
    let item = object(g); obj_to_char(g, item, ch);
    let domains = FIND_CHAR_ROOM | FIND_CHAR_WORLD | FIND_OBJ_INV;
    assert_eq!(generic_find(g, ch, b"2.reviewtarget", domains).1, Some(remote));
    assert_eq!(generic_find(g, ch, b"3.reviewtarget", domains).2, Some(item));
    assert_eq!(generic_find(g, ch, b"4.reviewtarget", domains).0, 0);
}
#[test]
fn zero_named_player_search_can_find_a_player_in_another_room() {
    let mut f = fixture("named"); let g = &mut f.game;
    let ch = player(g, b"Viewer", 12345); let target = player(g, b"Remote", 12346);
    mud_game::handler::char_to_room(g, ch, 0); mud_game::handler::char_to_room(g, target, 1);
    g.character_list.push_front(target); g.rooms[0].light = 1; g.rooms[1].light = 1;
    assert_eq!(mud_game::handler::get_char_world_vis(g, ch, b"0.Remote", None), Some(target));
}
