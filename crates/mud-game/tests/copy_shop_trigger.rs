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
    let root = std::env::temp_dir().join(format!("rustmud-copy-commands-{}-{label}", std::process::id()));
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

fn copy_case(shop: bool) {
    let mut f = fixture(if shop { "shop" } else { "trigger" }); let g = &mut f.game;
    let actor = player(g, b"Builder", 12345); g.ch_mut(actor).level = LVL_IMPL;
    mud_game::handler::char_to_room(g, actor, 0);
    let di = descriptor(g, actor, ConState::Playing);
    let dest = g.world.zones.iter().flat_map(|zone| zone.bot..=zone.top).find(|&v| if shop { !g.world.shops.iter().any(|s| s.vnum == v) } else { g.world.real_trigger(v).is_none() }).unwrap();
    let source = if shop { g.world.shops[0].vnum } else { g.world.triggers[0].vnum };
    if shop { g.shops_rt[0].bank = 1234; }
    let command = format!("{}copy {source} {dest}", if shop { "s" } else { "t" });
    let count = if shop { g.world.shops.len() } else { g.world.triggers.len() };
    g.ch_mut(actor).level = LVL_GOD;
    g.ch_mut(actor).ps_mut().olc_zone = NOWHERE as i32;
    for zone in &mut g.world.zones { zone.builders = None; }
    mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
    assert_eq!(if shop { g.world.shops.len() } else { g.world.triggers.len() }, count);
    g.ch_mut(actor).level = LVL_IMPL;
    let other = player(g, b"Other", 12346);
    let other_di = descriptor(g, other, if shop { ConState::Sedit } else { ConState::Trigedit });
    let mut pending = mud_game::olc::OlcData::new(); pending.number = dest as i32;
    g.olc.insert(other_di, pending);
    mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
    assert_eq!(if shop { g.world.shops.len() } else { g.world.triggers.len() }, count);
    g.olc.remove(&other_di);
    mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
    if shop {
        let original = g.world.shops.iter().find(|s| s.vnum == source).unwrap();
        let copy = g.world.shops.iter().find(|s| s.vnum == dest).expect("scopy did not create the shop");
        assert_eq!(copy.producing, original.producing);
        assert_eq!(copy.in_rooms, original.in_rooms);
        let src = g.world.shops.iter().position(|s| s.vnum == source).unwrap();
        let dst = g.world.shops.iter().position(|s| s.vnum == dest).unwrap();
        assert_eq!(g.shops_rt[src].bank, 1234);
        assert_eq!(g.shops_rt[dst].bank, 0);
        g.world.shops[dst].profit_buy = 123.0;
        mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
        assert_eq!(g.world.shops[dst].profit_buy, 123.0);
    } else {
        let original = &g.world.triggers[g.world.real_trigger(source).unwrap() as usize];
        let copy = &g.world.triggers[g.world.real_trigger(dest).expect("tcopy did not create the trigger") as usize];
        assert_eq!(copy.name, original.name);
        assert_eq!(copy.cmdlist, original.cmdlist);
        assert_eq!(copy.trigger_type, original.trigger_type);
        let dst = g.world.real_trigger(dest).unwrap() as usize;
        g.world.triggers[dst].name = Some(b"keep existing copy".to_vec());
        mud_game::interpreter::command_interpreter(g, actor, command.as_bytes());
        assert_eq!(g.world.triggers[dst].name.as_deref(), Some(b"keep existing copy".as_slice()));
    }
    assert_eq!(g.descriptors.get(di).unwrap().state, ConState::Playing);
    assert!(!g.olc.contains_key(&di));
}
#[test]
fn scopy_creates_a_shop() { copy_case(true); }
#[test]
fn tcopy_creates_a_trigger() { copy_case(false); }
