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
    let root = std::env::temp_dir().join(format!("rustmud-zone-save-{}-{label}", std::process::id()));
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

fn editor(g: &Game, room: usize) -> Box<mud_game::olc::OlcData> {
    let mut olc = mud_game::olc::OlcData::new(); olc.zone_num = 0; olc.number = g.world.rooms[room].vnum as i32;
    let mut zone = g.world.zones[0].clone(); zone.number = 0; zone.cmds.clear(); olc.zone = Some(Box::new(zone)); olc
}
fn command(kind: u8, room: i32) -> mud_world::model::ZoneCommand {
    mud_world::model::ZoneCommand { command: kind, arg3: room, ..Default::default() }
}
fn save(g: &mut Game, di: usize, mut olc: Box<mud_game::olc::OlcData>) {
    olc.mode = mud_game::olc::zedit::ZEDIT_CONFIRM_SAVESTRING;
    assert!(mud_game::olc::zedit::zedit_parse(g, di, olc, b"y").is_none());
}
#[test]
fn header_only_save_preserves_reset_commands_and_order() {
    use mud_game::olc::zedit::*;
    let mut f = fixture("header"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    g.world.zones[0].cmds = vec![command(b'O', 2), command(b'M', 1), command(b'G', 0), command(b'O', 0)];
    let expected = format!("{:?}", g.world.zones[0].cmds);
    let mut olc = editor(g, 1); olc.zone.as_mut().unwrap().cmds = g.world.zones[0].cmds[1..3].to_vec();
    olc.mode = ZEDIT_ZONE_NAME;
    let olc = zedit_parse(g, di, olc, b"Renamed zone").unwrap(); save(g, di, olc);
    assert_eq!(format!("{:?}", g.world.zones[0].cmds), expected);
}
#[test]
fn valid_reset_dependencies_survive_command_saves_at_their_original_position() {
    let mut f = fixture("commands"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    let commands = vec![command(b'M', 1), command(b'T', 1), command(b'G', 0), command(b'P', 0), command(b'V', 1), command(b'E', 0)];
    let mut source = vec![command(b'O', 2)]; source.extend(commands.clone()); source.push(command(b'O', 0));
    g.world.zones[0].cmds = source.clone();
    let mut olc = editor(g, 1); olc.zone_age = 1; olc.zone.as_mut().unwrap().cmds = commands;
    save(g, di, olc);
    assert_eq!(format!("{:?}", g.world.zones[0].cmds), format!("{:?}", source));
}
#[test]
fn concurrent_room_editors_preserve_each_others_zone_header_fields() {
    use mud_game::olc::zedit::*;
    let mut f = fixture("concurrent"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    let mut first = editor(g, 1); let mut second = editor(g, 2);
    first.mode = ZEDIT_ZONE_NAME; second.mode = ZEDIT_ZONE_LIFE;
    let first = zedit_parse(g, di, first, b"Renamed zone").unwrap();
    let second = zedit_parse(g, di, second, b"123").unwrap();
    save(g, di, first); save(g, di, second);
    assert_eq!(g.world.zones[0].name.as_deref(), Some(&b"Renamed zone"[..]));
    assert_eq!(g.world.zones[0].lifespan, 123);
}
#[test]
fn final_zone_accepts_top_vnums_above_32000() {
    use mud_game::olc::zedit::*;
    let mut f = fixture("top"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    let mut olc = editor(g, 0); olc.zone_num = g.world.zones.len() as i32 - 1;
    olc.zone.as_mut().unwrap().bot = 50000; olc.zone.as_mut().unwrap().top = 60000; olc.mode = ZEDIT_ZONE_TOP;
    let olc = zedit_parse(g, di, olc, b"65000").unwrap();
    assert_eq!(olc.zone.as_ref().unwrap().top, 65000);
    let mut olc = olc; olc.mode = ZEDIT_ZONE_TOP;
    let olc = zedit_parse(g, di, olc, b"65535").unwrap();
    assert_eq!(olc.zone.as_ref().unwrap().top, 65534);
    let mut olc = olc; olc.mode = ZEDIT_ZONE_TOP;
    let olc = zedit_parse(g, di, olc, b"-1").unwrap();
    assert_eq!(olc.zone.as_ref().unwrap().top, 50000);
}

#[test]
fn orphan_commands_do_not_move_to_another_rooms_reset_list() {
    let mut f = fixture("orphans"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    let prefix = command(b'O', 2); g.world.zones[0].cmds = vec![prefix.clone()];
    let mut olc = editor(g, 1); olc.zone_age = 1;
    olc.zone.as_mut().unwrap().cmds = vec![command(b'P', 0), command(b'G', 0), command(b'M', 1), command(b'P', 0), command(b'G', 0)];
    save(g, di, olc);
    assert_eq!(g.world.zones[0].cmds.iter().map(|c| c.command).collect::<Vec<_>>(), b"OMPG");
    assert_eq!(g.world.zones[0].cmds[0].arg3, 2);
}

#[test]
fn header_dirty_fields_accumulate_without_overwriting_other_edits() {
    use mud_game::olc::zedit::*;
    let mut f = fixture("fields"); let g = &mut f.game;
    let ch = player(g, b"Builder", 12345); let di = descriptor(g, ch, ConState::Zedit);
    let mut first = editor(g, 1); let mut second = editor(g, 2);
    for (mode, text) in [(ZEDIT_ZONE_NAME, &b"Name"[..]), (ZEDIT_ZONE_BUILDERS, b"Alice"), (ZEDIT_LEV_MIN, b"4"), (ZEDIT_ZONE_RESET, b"2")] {
        first.mode = mode; first = zedit_parse(g, di, first, text).unwrap();
    }
    for (mode, text) in [(ZEDIT_ZONE_LIFE, &b"123"[..]), (ZEDIT_LEV_MAX, b"20"), (ZEDIT_ZONE_FLAGS, b"1")] {
        second.mode = mode; second = zedit_parse(g, di, second, text).unwrap();
    }
    let flags = second.zone.as_ref().unwrap().zone_flags;
    save(g, di, second); save(g, di, first);
    let zone = &g.world.zones[0];
    assert_eq!(zone.name.as_deref(), Some(&b"Name"[..])); assert_eq!(zone.builders.as_deref(), Some(&b"Alice"[..]));
    assert_eq!(zone.min_level, 4); assert_eq!(zone.max_level, 20); assert_eq!(zone.lifespan, 123); assert_eq!(zone.reset_mode, 2); assert_eq!(zone.zone_flags, flags);
    let mut olc = editor(g, 1); olc.mode = ZEDIT_LEVELS;
    let olc = zedit_parse(g, di, olc, b"3").unwrap(); save(g, di, olc);
    assert_eq!(g.world.zones[0].min_level, -1); assert_eq!(g.world.zones[0].max_level, -1);
}
