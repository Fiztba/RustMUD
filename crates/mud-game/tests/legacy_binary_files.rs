use mud_game::game::Game;
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
    let root = std::env::temp_dir().join(format!("rustmud-legacy-binary-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

/// A tbaMUD binary board file as written by an LP64 build: int num_of_msgs,
/// then per message a 32-byte board_msginfo (level at 16, heading length at
/// 20, message length at 24) followed by the NUL-terminated strings.
fn binary_board(count: i32) -> Vec<u8> {
    let mut data = count.to_le_bytes().to_vec();
    for i in 0..count {
        let heading = format!("Legacy heading {i}\0").into_bytes();
        let body = format!("Legacy body {i}\r\n\0").into_bytes();
        let mut rec = [0u8; 32];
        rec[16..20].copy_from_slice(&5i32.to_le_bytes());
        rec[20..24].copy_from_slice(&(heading.len() as i32).to_le_bytes());
        rec[24..28].copy_from_slice(&(body.len() as i32).to_le_bytes());
        data.extend_from_slice(&rec);
        data.extend_from_slice(&heading);
        data.extend_from_slice(&body);
    }
    data
}

fn load_binary_board(count: i32) {
    let mut f = fixture(&format!("board-{count}")); let g = &mut f.game;
    let path = g.lib_dir.join("etc").join("board.mortal");
    std::fs::write(&path, binary_board(count)).unwrap();
    mud_game::boards::board_load_board(g, 0);
    assert_eq!(g.boards.msgs[0].len(), count as usize, "{count}-message binary board");
    assert_eq!(g.boards.msgs[0][0].heading.as_deref(), Some(b"Legacy heading 0".as_slice()));
    let body = g.boards.storage[g.boards.msgs[0][0].slot_num as usize].as_deref();
    assert_eq!(body, Some(b"Legacy body 0\r\n".as_slice()));
    // Whatever was written back must still hold every message.
    mud_game::boards::board_clear_board(g, 0);
    mud_game::boards::board_load_board(g, 0);
    assert_eq!(g.boards.msgs[0].len(), count as usize, "reload after conversion");
}

#[test]
fn binary_board_starting_with_asterisk_count_loads_every_message() { load_binary_board(42); }

#[test]
fn binary_board_with_other_count_loads_every_message() { load_binary_board(41); }

#[test]
fn ascii_board_still_loads() {
    let mut f = fixture("board-ascii"); let g = &mut f.game;
    let path = g.lib_dir.join("etc").join("board.mortal");
    std::fs::write(&path, binary_board(3)).unwrap();
    mud_game::boards::board_load_board(g, 0);
    assert_eq!(g.boards.msgs[0].len(), 3);
    let ascii = std::fs::read(&path).unwrap();
    assert!(ascii.starts_with(b"* tbaMUD board file"), "converted file must be ASCII");
    mud_game::boards::board_clear_board(g, 0);
    mud_game::boards::board_load_board(g, 0);
    assert_eq!(g.boards.msgs[0].len(), 3);
    assert_eq!(g.boards.msgs[0][2].heading.as_deref(), Some(b"Legacy heading 2".as_slice()));
    assert_eq!(std::fs::read(&path).unwrap(), ascii, "ASCII files are not rewritten on load");
}

/// One LP64 house_control_rec: vnum/atrium/exit_num as u16, built_on at 8,
/// mode at 16, owner at 24, num_of_guests at 32, guests at 40, last_payment
/// at 120, then spares out to 192 bytes.
fn binary_hcontrol(vnum: i32, atrium: i32, owner: i64) -> Vec<u8> {
    let mut rec = vec![0u8; 192];
    rec[0..2].copy_from_slice(&(vnum as u16).to_le_bytes());
    rec[2..4].copy_from_slice(&(atrium as u16).to_le_bytes());
    rec[4..6].copy_from_slice(&0u16.to_le_bytes());
    rec[8..16].copy_from_slice(&1_700_000_000i64.to_le_bytes());
    rec[24..32].copy_from_slice(&owner.to_le_bytes());
    rec[32..36].copy_from_slice(&1i32.to_le_bytes());
    rec[40..48].copy_from_slice(&777i64.to_le_bytes());
    rec[120..128].copy_from_slice(&1_750_000_000i64.to_le_bytes());
    rec
}

/// Picks a loaded room whose vnum's low byte is '#' or '*', and makes its
/// exit 0 lead to room 0 so it qualifies as a house with room 0 as atrium.
fn house_room(g: &mut Game) -> i32 {
    let room = g.world.rooms.iter().position(|r| matches!(r.vnum & 0xff, 0x23 | 0x2a)).unwrap();
    g.world.rooms[room].dir_option[0] = Some(Box::new(mud_world::model::Exit {
        general_description: None, keyword: None, exit_info: 0, key: mud_data::types::NOTHING,
        to_room_vnum: g.world.rooms[0].vnum as i32, to_room: 0,
    }));
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"owner".to_vec(), id: 12345, level: 10, flags: 0, last: g.now,
    });
    g.world.rooms[room].vnum as i32
}

#[test]
fn binary_hcontrol_whose_vnum_looks_like_ascii_loads_the_house() {
    let mut f = fixture("hcontrol-binary"); let g = &mut f.game;
    let vnum = house_room(g);
    let atrium = g.world.rooms[0].vnum as i32;
    let path = g.lib_dir.join("etc").join("hcontrol");
    std::fs::write(&path, binary_hcontrol(vnum, atrium, 12345)).unwrap();
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert_eq!(g.houses.len(), 1, "house vnum {vnum}");
    assert_eq!(g.houses[0].vnum, vnum);
    assert_eq!(g.houses[0].atrium, atrium);
    assert_eq!(g.houses[0].owner, 12345);
    assert_eq!(g.houses[0].guests, vec![777]);
    assert_eq!(g.houses[0].last_payment, 1_750_000_000);
    // The converted file reloads the same house.
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert_eq!(g.houses.len(), 1);
    assert_eq!(g.houses[0].guests, vec![777]);
}

#[test]
fn ascii_hcontrol_still_loads() {
    let mut f = fixture("hcontrol-ascii"); let g = &mut f.game;
    let vnum = house_room(g);
    let atrium = g.world.rooms[0].vnum as i32;
    g.houses = vec![mud_game::house::HouseControl {
        vnum, atrium, exit_num: 0, owner: 12345, guests: vec![4, 5], ..Default::default()
    }];
    mud_game::house::house_save_control(g);
    let path = g.lib_dir.join("etc").join("hcontrol");
    let ascii = std::fs::read(&path).unwrap();
    assert!(ascii.starts_with(b"* tbaMUD house control file"));
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert_eq!(g.houses.len(), 1);
    assert_eq!(g.houses[0].vnum, vnum);
    assert_eq!(g.houses[0].guests, vec![4, 5]);
    assert_eq!(std::fs::read(&path).unwrap(), ascii);
}

#[test]
fn unreadable_hcontrol_is_left_untouched() {
    let mut f = fixture("hcontrol-garbage"); let g = &mut f.game;
    let path = g.lib_dir.join("etc").join("hcontrol");
    // Neither a known binary record size nor anything the ASCII reader
    // recognises.
    let garbage = b"\x2a\x00\x01\x02\x03\x04\x05".to_vec();
    std::fs::write(&path, &garbage).unwrap();
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert!(g.houses.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), garbage, "boot must not rewrite a file it could not read");
}

fn syserrs(g: &Game) -> Vec<&String> {
    g.log_lines.iter().filter(|l| l.starts_with("SYSERR")).collect()
}

#[test]
fn writer_produced_empty_hcontrol_boots_quietly_and_unchanged() {
    let mut f = fixture("hcontrol-empty-ascii"); let g = &mut f.game;
    g.houses.clear();
    mud_game::house::house_save_control(g);
    let path = g.lib_dir.join("etc").join("hcontrol");
    let written = std::fs::read(&path).unwrap();
    assert!(written.starts_with(b"* tbaMUD house control file"));
    for boot in 1..=2 {
        g.log_lines.clear();
        mud_game::house::house_boot(g);
        assert!(g.houses.is_empty());
        assert!(syserrs(g).is_empty(), "boot {boot} logged {:?}", syserrs(g));
        assert_eq!(std::fs::read(&path).unwrap(), written, "boot {boot} changed the file");
    }
}

#[test]
fn zero_byte_hcontrol_boots_quietly_twice() {
    let mut f = fixture("hcontrol-zero-byte"); let g = &mut f.game;
    let path = g.lib_dir.join("etc").join("hcontrol");
    std::fs::write(&path, b"").unwrap();
    g.houses.clear();
    for boot in 1..=2 {
        g.log_lines.clear();
        mud_game::house::house_boot(g);
        assert!(g.houses.is_empty());
        assert!(syserrs(g).is_empty(), "boot {boot} logged {:?}", syserrs(g));
    }
}

#[test]
fn hand_edited_ascii_hcontrol_with_other_comment_loads_its_house() {
    let mut f = fixture("hcontrol-hand-edited"); let g = &mut f.game;
    let vnum = house_room(g);
    let atrium = g.world.rooms[0].vnum as i32;
    // Not the writer's header, and padded to a binary record size so a
    // length-based reader would take it for one LP64 record.
    let mut text = format!(
        "* my houses\n#{vnum}\nAtrm: {atrium}\nExit: 0\nBilt: 1700000000\nMode: 0\nOwnr: 12345\nPay : 1750000000\nGsts: 777\n$~\n"
    ).into_bytes();
    assert!(text.len() < 192);
    while text.len() < 192 {
        text.push(b'\n');
    }
    assert_eq!(text.len(), 192);
    let path = g.lib_dir.join("etc").join("hcontrol");
    std::fs::write(&path, &text).unwrap();
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert_eq!(g.houses.len(), 1, "hand-edited ASCII file of binary record length");
    assert_eq!(g.houses[0].vnum, vnum);
    assert_eq!(g.houses[0].owner, 12345);
    assert_eq!(g.houses[0].guests, vec![777]);
}

#[test]
fn binary_hcontrol_whose_first_bytes_spell_an_ascii_vnum_line_loads_as_binary() {
    let mut f = fixture("hcontrol-hash-zero-newline"); let g = &mut f.game;
    // vnum 0x3023 is the bytes "#0" and atrium 10 follows with "\n\0": the
    // record begins "#0\n", exactly what an ASCII "#<vnum>" line looks like.
    let vnum = 0x3023;
    let atrium = 10;
    let atrium_rnum = g.real_room(atrium).expect("room 10 exists in the mini world");
    let room = g.world.rooms.iter().position(|r| r.vnum as i32 != atrium).unwrap();
    let old_vnum = g.world.rooms[room].vnum;
    g.world.room_map.remove(&old_vnum);
    g.world.rooms[room].vnum = vnum as _;
    g.world.room_map.insert(vnum as _, room as _);
    g.world.rooms[room].dir_option[0] = Some(Box::new(mud_world::model::Exit {
        general_description: None, keyword: None, exit_info: 0, key: mud_data::types::NOTHING,
        to_room_vnum: atrium, to_room: atrium_rnum,
    }));
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"owner".to_vec(), id: 12345, level: 10, flags: 0, last: g.now,
    });
    let data = binary_hcontrol(vnum, atrium, 12345);
    assert!(data.starts_with(b"#0\n"));
    let path = g.lib_dir.join("etc").join("hcontrol");
    std::fs::write(&path, &data).unwrap();
    g.houses.clear();
    mud_game::house::house_boot(g);
    assert_eq!(g.houses.len(), 1);
    assert_eq!(g.houses[0].vnum, vnum);
    assert_eq!(g.houses[0].atrium, atrium);
    assert_eq!(g.houses[0].guests, vec![777]);
    assert!(std::fs::read(&path).unwrap().starts_with(b"* tbaMUD house control file"), "converted");
}
