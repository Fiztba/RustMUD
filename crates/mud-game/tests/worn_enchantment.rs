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

#[test]
fn enchantment_preserves_level_alignment_and_unworn_behavior() {
    use mud_data::flags;
    use mud_game::handler::*;
    let mut f = fixture("matrix"); let g = &mut f.game;
    for (level, hit, damage) in [(17, 1, 1), (18, 2, 1), (20, 2, 2)] {
        for alignment in [-1000, 0, 1000] {
            for worn in [false, true] {
                let ch = player(g, b"Caster", 12345); char_to_room(g, ch, 0); descriptor(g, ch, ConState::Playing);
                g.ch_mut(ch).alignment = alignment; g.ch_mut(ch).points.hitroll = 5; g.ch_mut(ch).points.damroll = 3;
                let mut object = mud_game::obj::create_obj(); object.type_flag = flags::ITEM_WEAPON;
                object.perm_affects.set(flags::AFF_DETECT_INVIS);
                let oid = g.objs.insert(object);
                if worn { assert!(equip_char(g, ch, oid, WEAR_WIELD)); } else { obj_to_char(g, oid, ch); }
                g.ch_mut(ch).act.remove(flags::PLR_CRASH);
                mud_game::spells::spell_enchant_weapon(g, level, ch, None, Some(oid));
                assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), if worn { (5 + hit, 3 + damage) } else { (5, 3) });
                assert_eq!(g.obj(oid).extra_flags.is_set(flags::ITEM_ANTI_GOOD), alignment < -350);
                assert_eq!(g.obj(oid).extra_flags.is_set(flags::ITEM_ANTI_EVIL), alignment > 350);
                if worn {
                    assert!(g.ch(ch).plr(flags::PLR_CRASH)); assert!(g.ch(ch).aff(flags::AFF_DETECT_INVIS));
                    unequip_char(g, ch, WEAR_WIELD); assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (5, 3));
                } else { obj_from_char(g, oid); }
                assert!(equip_char(g, ch, oid, WEAR_WIELD));
                assert_eq!((g.ch(ch).points.hitroll, g.ch(ch).points.damroll), (5 + hit, 3 + damage));
            }
        }
    }
}
