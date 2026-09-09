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

    let root = std::env::temp_dir().join(format!("rustmud-staff-{}-{label}", std::process::id()));

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



fn use_item(kind: i32, spell: i32, label: &str) -> Vec<i32> {
    let mut f = fixture(label);
    let g = &mut f.game;
    let caster = player(g, b"Caster", 12345);
    descriptor(g, caster, ConState::Playing);
    mud_game::handler::char_to_room(g, caster, 0);
    g.world.rooms[0].room_flags = [0; 4];
    g.ch_mut(caster).points.hit = 77;
    let mut targets = Vec::new();
    for name in [b"Targeta", b"Targetb"] {
        let ch = player(g, name, 0);
        g.ch_mut(ch).act.set(flags::MOB_ISNPC);
        g.ch_mut(ch).points.hit = if spell == mud_data::spells::SPELL_HEAL { 250 } else { 500 };
        g.ch_mut(ch).points.max_hit = 500;
        g.ch_mut(ch).position = POS_STANDING;
        mud_game::handler::char_to_room(g, ch, 0);
        targets.push(ch);
    }
    let mut obj = mud_game::obj::create_obj();
    obj.type_flag = kind;
    obj.values = [1, 5, 5, spell];
    obj.name = Some(b"test implement".to_vec());
    let item = g.objs.insert(obj);
    mud_game::handler::obj_to_char(g, item, caster);
    mud_game::spell_parser::mag_objectmagic(g, caster, item, b"");
    assert_eq!(g.obj(item).values[2], 4);
    assert_eq!(g.ch(caster).points.hit, 77);
    let hits: Vec<_> = targets.iter().map(|&ch| g.ch(ch).points.hit).collect();
    g.obj_mut(item).values[2] = 0;
    mud_game::spell_parser::mag_objectmagic(g, caster, item, b"");
    assert_eq!(g.obj(item).values[2], 0);
    assert_eq!(targets.iter().map(|&ch| g.ch(ch).points.hit).collect::<Vec<_>>(), hits);
    hits
}

#[test]
fn staff_area_spell_hits_each_target_once_per_charge() {
    let wand = use_item(flags::ITEM_WAND, mud_data::spells::SPELL_EARTHQUAKE, "wand");
    let staff = use_item(flags::ITEM_STAFF, mud_data::spells::SPELL_EARTHQUAKE, "staff");
    assert!(wand.iter().all(|&hp| hp < 500));
    assert_eq!(staff, wand, "one staff charge must cast the area spell only once");
}

#[test]
fn ordinary_staff_spell_still_reaches_each_target() {
    let hits = use_item(flags::ITEM_STAFF, mud_data::spells::SPELL_HEAL, "healing-staff");
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|&hp| (353..=374).contains(&hp)));
}
