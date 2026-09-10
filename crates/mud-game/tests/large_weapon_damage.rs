use mud_data::{flags, types::*};
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
    let root = std::env::temp_dir().join(format!("rustmud-large-damage-{}-{label}", std::process::id()));
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
fn large_weapon_damage_reaches_the_normal_damage_cap() {
    let mut f = fixture("weapon"); let g = &mut f.game;
    let actor = player(g, b"Fighter", 12345);
    mud_game::handler::char_to_room(g, actor, 0); descriptor(g, actor, ConState::Playing);
    let victim = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, victim, 0);
    g.ch_mut(victim).act = flags::FlagSet::EMPTY; g.ch_mut(victim).act.set(flags::MOB_ISNPC);
    g.ch_mut(victim).affected_by = flags::FlagSet::EMPTY; g.ch_mut(victim).script = None;
    g.ch_mut(victim).position = POS_SLEEPING;
    g.ch_mut(victim).points.hit = 1000; g.ch_mut(victim).points.max_hit = 1000;
    g.ch_mut(actor).points.damroll = 10;
    g.world.rooms[0].room_flags = [0;4];
    let weapon = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(weapon).type_flag = flags::ITEM_WEAPON;
    g.obj_mut(weapon).values = [0, 8, i32::MAX, 0];
    mud_game::handler::equip_char(g, actor, weapon, WEAR_WIELD);
    g.ch_mut(actor).aff_abils.str_ = 10;
    for (num, size, attack, damage) in [
        (8, i32::MAX, mud_data::spells::TYPE_UNDEFINED, 100),
        (8, i32::MAX, mud_data::spells::SKILL_BACKSTAB, 100),
        (1, 1, mud_data::spells::TYPE_UNDEFINED, 22),
    ] {
        mud_game::fight::stop_fighting(g, actor);
        mud_game::fight::stop_fighting(g, victim);
        g.ch_mut(victim).position = POS_SLEEPING;
        g.ch_mut(victim).points.hit = 1000;
        g.obj_mut(weapon).values[1] = num;
        g.obj_mut(weapon).values[2] = size;
        g.rng = mud_data::rng::CircleRng::new(12345);
        mud_game::fight::hit(g, actor, victim, attack);
        assert_eq!(g.ch(victim).points.hit, 1000 - damage);
    }
}
