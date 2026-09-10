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
    let root = std::env::temp_dir().join(format!("rustmud-olc-vnum-{}-{label}", std::process::id()));
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

use mud_game::olc::{OlcData, qedit::*, redit::*};
#[test]
fn quest_object_field_does_not_wrap_to_an_existing_object() {
    let mut f = fixture("quest"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Qedit);
    let mut olc = OlcData::new(); olc.quest = Some(Box::new(g.world.quests[0].clone()));
    olc.quest.as_mut().unwrap().obj_reward = NOTHING as i32; olc.mode = QEDIT_OBJ;
    let input = (u32::from(g.world.obj_protos[0].vnum) + 65536).to_string();
    let olc = qedit_parse(g, di, olc, input.as_bytes()).unwrap();
    assert_eq!(olc.quest.as_ref().unwrap().obj_reward, NOTHING as i32);
}
#[test]
fn room_exit_fields_do_not_wrap_virtual_numbers() {
    let mut f = fixture("exit"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Redit);
    for mode in [REDIT_EXIT_NUMBER, REDIT_EXIT_KEY] {
        let mut olc = OlcData::new(); olc.room = Some(Box::new(g.world.rooms[0].clone()));
        olc.room.as_mut().unwrap().dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
            general_description: None, keyword: None, exit_info: 0, key: NOTHING,
            to_room_vnum: g.world.rooms[2].vnum as i32, to_room: 2,
        }));
        olc.value = NORTH as i32; olc.mode = mode;
        let input = (u32::from(g.world.rooms[1].vnum) + 65536).to_string();
        let olc = redit_parse(g, di, olc, input.as_bytes()).unwrap();
        let exit = olc.room.as_ref().unwrap().dir_option[NORTH].as_ref().unwrap();
        assert_eq!((exit.to_room, exit.key), (2, NOTHING));
    }
}

#[test]
fn quest_vnum_fields_reject_invalid_input_and_accept_existing_entries_and_none() {
    let mut f = fixture("quest-matrix"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Qedit);
    for mode in [QEDIT_QUESTMASTER, QEDIT_PREREQ, QEDIT_RETURNMOB, QEDIT_TARGET,
        QEDIT_NEXTQUEST, QEDIT_PREVQUEST, QEDIT_OBJ] {
        let valid = match mode {
            QEDIT_QUESTMASTER | QEDIT_RETURNMOB => g.world.mob_protos[0].vnum as i32,
            QEDIT_NEXTQUEST | QEDIT_PREVQUEST => g.world.quests[0].vnum as i32,
            _ => g.world.obj_protos[0].vnum as i32,
        };
        let field = |olc: &OlcData| {
            let q = olc.quest.as_ref().unwrap();
            match mode { QEDIT_QUESTMASTER => q.qm_vnum, QEDIT_PREREQ => q.prereq,
                QEDIT_RETURNMOB => q.obj_in, QEDIT_TARGET => q.target,
                QEDIT_NEXTQUEST => q.next_quest, QEDIT_PREVQUEST => q.prev_quest,
                _ => q.obj_reward }
        };
        for input in ["".to_string(), "garbage".into(), "1junk".into(), "-2".into(),
            "-2147483648".into(), "65535".into(), "65536".into(), "2147483647".into(),
            "9".repeat(400), (valid + 65536).to_string()] {
            let mut olc = OlcData::new(); olc.quest = Some(Box::new(g.world.quests[0].clone()));
            olc.mode = mode; let before = field(&olc);
            let olc = qedit_parse(g, di, olc, input.as_bytes()).unwrap();
            assert_eq!(olc.mode, mode, "mode={mode}, input={input}");
            assert_eq!(field(&olc), before);
        }
        for number in [valid, -1] {
            let mut olc = OlcData::new(); olc.quest = Some(Box::new(g.world.quests[0].clone()));
            olc.mode = mode;
            let olc = qedit_parse(g, di, olc, number.to_string().as_bytes()).unwrap();
            let expected = if number == -1 && !matches!(mode, QEDIT_RETURNMOB | QEDIT_TARGET) {
                NOTHING as i32
            } else { number };
            assert_eq!(field(&olc), expected, "mode={mode}, number={number}");
            assert_eq!(olc.mode, QEDIT_MAIN_MENU);
        }
    }
}

#[test]
fn exit_vnum_fields_preserve_invalid_input_and_accept_valid_numbers_and_none() {
    let mut f = fixture("exit-matrix"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); let di = descriptor(g, actor, ConState::Redit);
    for mode in [REDIT_EXIT_NUMBER, REDIT_EXIT_KEY] {
        for input in ["", "garbage", "1junk", "-2", "-2147483648", "65535", "65536", "2147483647"] {
            let mut olc = OlcData::new(); olc.room = Some(Box::new(g.world.rooms[0].clone()));
            olc.room.as_mut().unwrap().dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
                general_description: None, keyword: None, exit_info: 0, key: NOTHING,
                to_room_vnum: g.world.rooms[2].vnum as i32, to_room: 2,
            }));
            olc.value = NORTH as i32; olc.mode = mode;
            let olc = redit_parse(g, di, olc, input.as_bytes()).unwrap();
            assert_eq!(olc.mode, mode);
            let exit = olc.room.as_ref().unwrap().dir_option[NORTH].as_ref().unwrap();
            assert_eq!((exit.to_room, exit.key), (2, NOTHING));
        }
        let valid = g.world.rooms[1].vnum as i32;
        let numbers = if mode == REDIT_EXIT_KEY { vec![0, valid, 65534, -1] } else { vec![valid, -1] };
        for number in numbers {
            let mut olc = OlcData::new(); olc.room = Some(Box::new(g.world.rooms[0].clone()));
            olc.room.as_mut().unwrap().dir_option[NORTH] = Some(Box::new(mud_world::model::Exit {
                general_description: None, keyword: None, exit_info: 0, key: NOTHING,
                to_room_vnum: g.world.rooms[2].vnum as i32, to_room: 2,
            }));
            olc.value = NORTH as i32; olc.mode = mode;
            let olc = redit_parse(g, di, olc, number.to_string().as_bytes()).unwrap();
            let exit = olc.room.as_ref().unwrap().dir_option[NORTH].as_ref().unwrap();
            let actual = if mode == REDIT_EXIT_NUMBER { exit.to_room } else { exit.key };
            let expected = if number == -1 { NOTHING } else if mode == REDIT_EXIT_NUMBER { 1 } else { number as u16 };
            assert_eq!(actual, expected);
            assert_eq!(olc.mode, REDIT_EXIT_MENU);
        }
    }
}
