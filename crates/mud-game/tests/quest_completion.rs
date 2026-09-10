use mud_game::{game::Game, quest};
use std::path::{Path, PathBuf};
use mud_game::ch::{Char, PlayerSpecials};
use mud_world::model::Quest;
use mud_data::types::NOTHING;

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
    let root = std::env::temp_dir().join(format!("rustmud-quest-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn player(g: &mut Game) -> mud_data::ids::CharId {
    let ch = g.chars.insert(Char {
        name: Some(b"Quester".to_vec()), level: 10,
        player_specials: Some(Box::new(PlayerSpecials::default())), ..Default::default()
    });
    mud_game::handler::char_to_room(g, ch, 0);
    ch
}

#[test]
fn finding_multiple_mobs_does_not_complete_the_next_quest() {
    let mut f = fixture("chain");
    let g = &mut f.game;
    let ch = player(g);
    let target = g.world.mob_protos[0].vnum as i32;
    for _ in 0..3 {
        let mob = mud_game::db::read_mobile(g, 0).unwrap();
        mud_game::handler::char_to_room(g, mob, 0);
    }
    g.world.quests = vec![
        Quest { vnum: 65000, type_: quest::AQ_MOB_FIND, target, obj_out: 2,
            value: 7, next_quest: 65001, ..Default::default() },
        Quest { vnum: 65001, type_: quest::AQ_MOB_KILL, target, obj_out: 1,
            value: 11, next_quest: -1, ..Default::default() },
    ];
    quest::set_quest(g, ch, 0);
    quest::autoquest_trigger_check(g, ch, None, None, quest::AQ_MOB_FIND);
    assert_eq!(g.ch(ch).ps().current_quest, 65001);
    assert_eq!(g.ch(ch).ps().quest_counter, 1);
    assert_eq!(g.ch(ch).ps().questpoints, 7);
    assert_eq!(g.ch(ch).ps().completed_quests, vec![65000]);
}

#[test]
fn a_matching_object_reward_cannot_complete_the_same_quest_again() {
    let mut f = fixture("reward");
    let g = &mut f.game;
    let ch = player(g);
    let target = g.world.obj_protos[0].vnum as i32;
    g.world.quests = vec![Quest {
        vnum: 65000, type_: quest::AQ_OBJ_FIND, target, obj_out: 1,
        obj_reward: target, value: 7, next_quest: -1, ..Default::default()
    }];
    quest::set_quest(g, ch, 0);
    let found = mud_game::db::read_object(g, 0).unwrap();
    mud_game::handler::obj_to_char(g, found, ch);
    assert_eq!(g.ch(ch).ps().current_quest, NOTHING);
    assert_eq!(g.ch(ch).ps().questpoints, 7);
    assert_eq!(g.ch(ch).ps().completed_quests, vec![65000]);
    assert_eq!(g.ch(ch).carrying.len(), 2);

    // Repeatable quests give one reward per completion and no history entry.
    // A matching reward must not count toward the next stage either.
    g.world.quests[0].flags = quest::AQ_REPEATABLE;
    g.world.quests[0].next_quest = 65001;
    g.world.quests.push(Quest {
        vnum: 65001, type_: quest::AQ_OBJ_FIND, target, obj_out: 3,
        next_quest: -1, ..Default::default()
    });
    quest::set_quest(g, ch, 0);
    let found = mud_game::db::read_object(g, 0).unwrap();
    mud_game::handler::obj_to_char(g, found, ch);
    assert_eq!(g.ch(ch).ps().questpoints, 14);
    assert_eq!(g.ch(ch).ps().completed_quests, vec![65000]);
    assert_eq!(g.ch(ch).ps().current_quest, 65001);
    assert_eq!(g.ch(ch).ps().quest_counter, 3);
    assert_eq!(g.ch(ch).carrying.len(), 4);
}


