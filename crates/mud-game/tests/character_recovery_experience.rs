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
    let root = std::env::temp_dir().join(format!("rustmud-character-state-{}-{label}", std::process::id()));
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

#[test]
fn successful_bandaging_stops_bleeding() {
    let mut f = fixture("bandage"); let g = &mut f.game;
    let healer = player(g, b"Healer", 12345); let victim = player(g, b"Patient", 12346);
    for ch in [healer, victim] { descriptor(g, ch, ConState::Playing); mud_game::handler::char_to_room(g, ch, 0); }
    g.rooms[0].light = 1;
    g.ch_mut(healer).position = POS_STANDING;
    g.ch_mut(healer).set_skill(mud_data::spells::SKILL_BANDAGE, 101);
    for hp in [-2, -4, -8] {
        g.ch_mut(victim).points.hit = hp; mud_game::fight::update_pos(g, victim);
        mud_game::act::offensive::do_bandage(g, healer, b"Patient", 0, 0);
        assert_eq!(g.ch(victim).points.hit, 0);
        assert_eq!(g.ch(victim).position, POS_STUNNED);
    }
}

#[test]
fn combat_happy_hour_bonus_is_applied_once() {
    let mut f = fixture("happy"); let g = &mut f.game;
    let ch = player(g, b"Fighter", 12345); let di = descriptor(g, ch, ConState::Playing);
    mud_game::handler::char_to_room(g, ch, 0); g.rooms[0].light = 1;
    g.world.rooms[0].room_flags[0] &= !(1 << mud_data::flags::ROOM_PEACEFUL);
    g.happy.ticks_left = 10; g.happy.exp_rate = 100;
    for ticks in [0, 10] { for rate in [0, 25, 100, i32::MAX] { for cap in [200, 1000] { for grouped in [false, true] {
        g.happy.ticks_left = ticks; g.happy.exp_rate = rate; g.config.max_exp_gain = cap;
        g.groups.clear(); g.ch_mut(ch).group = None;
        g.ch_mut(ch).points.exp = 0;
        if grouped {
            g.groups.push(mud_game::game::Group { id: 99, leader: Some(ch), members: vec![ch], group_flags: 0 });
            g.ch_mut(ch).group = Some(99);
        }
        let victim = mud_game::db::read_mobile(g, 0).unwrap();
        mud_game::handler::char_to_room(g, victim, 0);
        { let v = g.ch_mut(victim); v.level = 10; v.position = POS_STANDING; v.points.hit = -11; v.points.exp = 900; v.act.remove(mud_data::flags::MOB_NOKILL); }
        g.descriptors.get_mut(di).unwrap().output.clear();
        mud_game::fight::damage(g, ch, victim, 0, mud_data::spells::TYPE_SUFFERING);
        let base = i64::from(cap.min(300));
        let expected = (if ticks > 0 { base + base * i64::from(rate) / 100 } else { base }).min(i64::from(cap)) as i32;
        assert_eq!(g.ch(ch).points.exp, expected, "grouped={grouped}, rate={rate}, ticks={ticks}, cap={cap}");
        assert!(String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output).contains(&expected.to_string()));
    } } } }
}

#[test]
fn demotion_rebuilds_stats_and_advance_ignores_happy_hour() {
    let mut f = fixture("advance"); let g = &mut f.game;
    let admin = player(g, b"Admin", 12345); let victim = player(g, b"Student", 12346);
    for ch in [admin, victim] { descriptor(g, ch, ConState::Playing); mud_game::handler::char_to_room(g, ch, 0); g.character_list.push_back(ch); }
    g.ch_mut(admin).level = LVL_IMPL; g.rooms[0].light = 1;
    g.ch_mut(victim).level = 20; g.ch_mut(victim).class = CLASS_WARRIOR;
    mud_game::act::wizard::do_advance(g, admin, b"Student 10", 0, 0);
    assert_eq!(g.ch(victim).level, 10);
    assert!(g.ch(victim).points.max_hit > 30, "demoted warrior must receive levels 2 through 10");
    g.happy.ticks_left = 10; g.happy.exp_rate = 100;
    mud_game::act::wizard::do_advance(g, admin, b"Student 15", 0, 0);
    assert_eq!(g.ch(victim).level, 15);
    assert_eq!(g.ch(victim).points.exp, mud_data::tables::level_exp(CLASS_WARRIOR as i32, 15));
    mud_game::act::wizard::do_advance(g, admin, b"Student 5", 0, 0);
    assert_eq!(g.ch(victim).level, 5);
    assert_eq!(g.ch(victim).points.exp, mud_data::tables::level_exp(CLASS_WARRIOR as i32, 5));
    mud_game::act::wizard::do_advance(g, admin, b"Student 1", 0, 0);
    assert_eq!(g.ch(victim).level, 1);
    assert_eq!(g.ch(victim).points.exp, mud_data::tables::level_exp(CLASS_WARRIOR as i32, 1));
}
