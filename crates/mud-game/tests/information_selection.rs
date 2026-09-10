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
    let root = std::env::temp_dir().join(format!("rustmud-info-selection-{}-{label}", std::process::id()));
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

fn output(g: &mut Game, di: usize) -> String {
    String::from_utf8_lossy(&std::mem::take(&mut g.descriptors.get_mut(di).unwrap().output)).into_owned()
}
fn viewer(g: &mut Game) -> (mud_data::ids::CharId, usize) {
    let ch = player(g, b"Viewer", 12345); let di = descriptor(g, ch, ConState::Playing);
    g.ch_mut(ch).position = POS_STANDING; mud_game::handler::char_to_room(g, ch, 0); g.rooms[0].light = 1; (ch, di)
}
#[test]
fn look_ordinals_count_inventory_room_and_equipment_once() {
    use mud_data::flags;
    let mut f = fixture("look"); let g = &mut f.game; let (ch, di) = viewer(g);
    let mut objects = Vec::new();
    for text in ["INVENTORY_NOTE", "ROOM_NOTE", "EQUIPPED_NOTE"] {
        let mut o = mud_game::obj::create_obj(); o.type_flag = flags::ITEM_NOTE; o.name = Some(b"note".to_vec()); o.action_description = Some(text.as_bytes().to_vec());
        objects.push(g.objs.insert(o));
    }
    mud_game::handler::obj_to_char(g, objects[0], ch); mud_game::handler::obj_to_room(g, objects[1], 0); mud_game::handler::equip_char(g, ch, objects[2], WEAR_HOLD);
    for (arg, expected) in [(b"1.note".as_slice(), "INVENTORY_NOTE"), (b"2.note", "ROOM_NOTE"), (b"3.note", "EQUIPPED_NOTE")] {
        mud_game::act::informative::do_look(g, ch, arg, 0, 0);
        assert!(output(g, di).contains(expected), "arg={arg:?}");
    }
    mud_game::act::informative::do_look(g, ch, b"last.note", 0, 0);
    assert!(output(g, di).contains("INVENTORY_NOTE"));
    for arg in [b"0.note".as_slice(), b"4.note"] {
        mud_game::act::informative::do_look(g, ch, arg, 0, 0);
        assert!(output(g, di).contains("do not see"));
    }
    for (i, &oid) in objects.iter().enumerate() {
        g.obj_mut(oid).name = Some(b"box".to_vec()); g.obj_mut(oid).type_flag = flags::ITEM_CONTAINER; g.obj_mut(oid).values[0] = 100;
        let mut child = mud_game::obj::create_obj(); child.name = Some(b"gem".to_vec()); child.short_description = Some(format!("CONTENT_{i}").into_bytes());
        let child = g.objs.insert(child); mud_game::handler::obj_to_obj(g, child, oid);
    }
    for (i, arg) in [b"in 1.box".as_slice(), b"in 2.box", b"in 3.box"].into_iter().enumerate() {
        mud_game::act::informative::do_look(g, ch, arg, 0, 0);
        assert!(output(g, di).contains(&format!("CONTENT_{i}")));
    }
    mud_game::act::informative::do_examine(g, ch, b"3.box", 0, 0);
    assert!(output(g, di).contains("CONTENT_2"));
}
#[test]
fn examine_respects_blindness_for_room_descriptions() {
    let mut f = fixture("blind"); let g = &mut f.game; let (ch, di) = viewer(g);
    g.world.rooms[0].ex_descriptions.push(mud_world::model::ExtraDesc { keyword: Some(b"wall".to_vec()), description: Some(b"HIDDEN_WRITING".to_vec()) });
    g.ch_mut(ch).affected_by.set(mud_data::flags::AFF_BLIND);
    mud_game::act::informative::do_examine(g, ch, b"wall", 0, 0);
    let text = output(g, di); assert!(!text.contains("HIDDEN_WRITING")); assert!(text.contains("blind"));
    g.ch_mut(ch).affected_by.remove(mud_data::flags::AFF_BLIND);
    g.rooms[0].light = 0; g.world.rooms[0].room_flags[0] |= 1 << mud_data::flags::ROOM_DARK;
    mud_game::act::informative::do_examine(g, ch, b"wall", 0, 0);
    let text = output(g, di); assert!(!text.contains("HIDDEN_WRITING")); assert!(text.contains("pitch black"));
    g.rooms[0].light = 1; g.ch_mut(ch).position = POS_STUNNED;
    mud_game::act::informative::do_examine(g, ch, b"wall", 0, 0); assert!(output(g, di).contains("stars"));
    g.ch_mut(ch).position = POS_STANDING;
    mud_game::act::informative::do_examine(g, ch, b"wall", 0, 0); assert!(output(g, di).contains("HIDDEN_WRITING"));
}
#[test]
fn who_group_filters_list_group_members_and_leaders() {
    let mut f = fixture("who"); let g = &mut f.game; let (ch, di) = viewer(g);
    let leader = player(g, b"Leader", 12346); let member = player(g, b"Member", 12347); let solo = player(g, b"Solo", 12348);
    for c in [leader, member, solo] { descriptor(g, c, ConState::Playing); mud_game::handler::char_to_room(g, c, 0); }
    let group = mud_game::handler::create_group(g, leader); mud_game::handler::join_group(g, member, group);
    mud_game::act::informative::do_who(g, ch, b"-g", 0, 0);
    let text = output(g, di); assert!(text.contains("Leader") && text.contains("Member")); assert!(!text.contains("Solo"));
    mud_game::act::informative::do_who(g, ch, b"-l", 0, 0);
    let text = output(g, di); assert!(text.contains("Leader")); assert!(!text.contains("Member") && !text.contains("Solo"));
    mud_game::act::informative::do_who(g, ch, b"-s -g -l", 0, 0);
    let text = output(g, di); assert!(text.contains("Leader")); assert!(!text.contains("Member") && !text.contains("Solo"));
    mud_game::handler::leave_group(g, member);
    mud_game::act::informative::do_who(g, ch, b"-g", 0, 0);
    let text = output(g, di); assert!(text.contains("Leader")); assert!(!text.contains("Member"));
}
#[test]
fn areas_includes_zones_contained_by_the_requested_range() {
    let mut f = fixture("areas"); let g = &mut f.game; let (ch, di) = viewer(g);
    let mut zone = mud_world::model::Zone::default(); zone.name = Some(b"TEST_AREA".to_vec()); zone.min_level = 10; zone.max_level = 20;
    zone.zone_flags[0] |= 1 << mud_data::flags::ZONE_GRID; g.world.zones = vec![zone];
    for (arg, shown, overlap) in [
        ("5-25", true, false), ("25-5", true, false), ("12-18", true, true),
        ("0-10", true, true), ("20-30", true, true), ("0-9", false, false), ("21-30", false, false),
        ("15", true, false), ("9", false, false), ("", true, false),
    ] {
        mud_game::act::informative::do_areas(g, ch, arg.as_bytes(), 0, 0);
        let text = output(g, di); assert_eq!(text.contains("TEST_AREA"), shown, "{arg}"); assert_eq!(text.contains("Areas shown in"), overlap, "{arg}");
    }
    for (min, max, arg, shown) in [(-1, 20, "0-5", true), (10, -1, "30-40", true), (-1, -1, "0-100", true), (10, -1, "0-9", false)] {
        g.world.zones[0].min_level = min; g.world.zones[0].max_level = max;
        mud_game::act::informative::do_areas(g, ch, arg.as_bytes(), 0, 0); assert_eq!(output(g, di).contains("TEST_AREA"), shown);
    }
}
