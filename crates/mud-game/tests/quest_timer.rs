use mud_game::{game::Game, quest, players_glue};
use std::path::{Path, PathBuf};
use mud_game::ch::{Char, PlayerSpecials};
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
    let root = std::env::temp_dir().join(format!("rustmud-questtimer-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn timed_quests_keep_their_remaining_time_across_reload() {
    let mut f = fixture("reload");
    let g = &mut f.game;
    g.player_table.push(mud_game::game::PlayerIndexElement {
        name: b"timer".to_vec(), id: 12345, level: 10, flags: 0, last: g.now,
    });
    let ch = g.chars.insert(Char {
        name: Some(b"Timer".to_vec()), idnum: 12345, level: 10,
        pfilepos: (g.player_table.len() - 1) as i32,
        player_specials: Some(Box::new(PlayerSpecials::default())), ..Default::default()
    });
    g.character_list.push_front(ch);
    g.world.quests.push(mud_world::model::Quest {
        vnum: 65000, time: 5, obj_out: 2, ..Default::default()
    });
    quest::set_quest(g, ch, g.world.quests.len() - 1);
    quest::check_timed_quests(g);
    assert_eq!(g.ch(ch).ps().quest_time, 4);
    players_glue::save_char(g, ch);
    let pos = players_glue::load_char_into(g, ch, b"Timer").unwrap();
    g.ch_mut(ch).pfilepos = pos as i32;
    assert_eq!(g.ch(ch).ps().quest_time, 4);
    quest::check_timed_quests(g);
    players_glue::save_char(g, ch);
    let pos = players_glue::load_char_into(g, ch, b"Timer").unwrap();
    g.ch_mut(ch).pfilepos = pos as i32;
    assert_eq!(g.ch(ch).ps().quest_time, 3);
    for _ in 0..3 { quest::check_timed_quests(g); }
    assert_eq!(g.ch(ch).ps().current_quest, NOTHING);
    players_glue::save_char(g, ch);
    players_glue::load_char_into(g, ch, b"Timer").unwrap();
    assert_eq!(g.ch(ch).ps().quest_time, -1);
}


