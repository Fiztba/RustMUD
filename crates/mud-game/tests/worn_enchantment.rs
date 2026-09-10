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
    let root = std::env::temp_dir().join(format!("rustmud-worn-enchantment-{}-{label}", std::process::id()));
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
fn enchanting_a_wielded_weapon_updates_and_later_removes_its_bonuses() {
    let mut f = fixture("weapon"); let g = &mut f.game;
    let ch = player(g, b"Caster", 12345); mud_game::handler::char_to_room(g, ch, 0);
    descriptor(g, ch, ConState::Playing);
    g.ch_mut(ch).points.hitroll = 5; g.ch_mut(ch).points.damroll = 3;
    let mut object = mud_game::obj::create_obj(); object.type_flag = mud_data::flags::ITEM_WEAPON;
    let oid = g.objs.insert(object); assert!(mud_game::handler::equip_char(g, ch, oid, WEAR_WIELD));
    mud_game::spells::spell_enchant_weapon(g, 20, ch, None, Some(oid));
    assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (7, 5));
    assert_eq!(g.ch(ch).equipment[WEAR_WIELD], Some(oid));
    mud_game::spells::spell_enchant_weapon(g, 20, ch, None, Some(oid));
    assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (7, 5));
    mud_game::handler::unequip_char(g, ch, WEAR_WIELD);
    assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (5, 3));
    assert!(mud_game::handler::equip_char(g, ch, oid, WEAR_WIELD));
    assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (7, 5));
}
