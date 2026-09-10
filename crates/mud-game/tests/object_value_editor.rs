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
    let root = std::env::temp_dir().join(format!("rustmud-object-values-{}-{label}", std::process::id()));
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

use mud_game::olc::{oedit, OlcData};
#[test]
fn liquid_type_selection_clamps_minimum_integer_without_overflow() {
    let mut f = fixture("liquid"); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345);
    let di = descriptor(g, actor, ConState::Oedit);
    let mut olc = OlcData::new(); olc.obj = Some(Box::new(mud_world::model::ObjProto::default()));
    olc.obj.as_mut().unwrap().type_flag = flags::ITEM_DRINKCON;
    olc.mode = oedit::OEDIT_VALUE_3;
    let olc = oedit::oedit_parse(g, di, olc, b"-2147483648").unwrap();
    assert_eq!(olc.obj.as_ref().unwrap().values[2], 0);
}
#[test]
fn rejected_furniture_capacity_stays_on_the_same_prompt() {
    let mut f = fixture("furniture"); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345);
    let di = descriptor(g, actor, ConState::Oedit);
    let mut olc = OlcData::new(); olc.obj = Some(Box::new(mud_world::model::ObjProto::default()));
    olc.obj.as_mut().unwrap().type_flag = flags::ITEM_FURNITURE;
    olc.obj.as_mut().unwrap().values[0] = 2;
    olc.mode = oedit::OEDIT_VALUE_1;
    let olc = oedit::oedit_parse(g, di, olc, b"-1").unwrap();
    assert_eq!(olc.mode, oedit::OEDIT_VALUE_1);
    assert_eq!(olc.obj.as_ref().unwrap().values[0], 2);
}
