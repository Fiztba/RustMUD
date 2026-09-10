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
    let root = std::env::temp_dir().join(format!("rustmud-copyover-player-{}-{label}", std::process::id()));
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
fn copyover_saves_player_and_inventory_after_the_final_pulse() {
    let mut f = fixture("final"); let g = &mut f.game;
    let actor = player(g, b"Traveller", 12345);
    g.ch_mut(actor).pfilepos = 0;
    mud_game::handler::char_to_room(g, actor, 0);
    descriptor(g, actor, ConState::Playing);
    let original = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(original).extra_flags.remove(flags::ITEM_NORENT);
    g.obj_mut(original).type_flag = flags::ITEM_TREASURE;
    g.obj_mut(original).cost_per_day = 1;
    mud_game::handler::obj_to_char(g, original, actor);
    mud_game::copyover::do_copyover(g, actor, b"", 0, 0);
    assert_eq!(g.obj(original).carried_by, Some(actor));
    mud_game::handler::obj_from_char(g, original);
    mud_game::handler::obj_to_room(g, original, 0);
    // Later commands and pulse updates can still change player state.
    mud_game::handler::char_from_room(g, actor);
    mud_game::handler::char_to_room(g, actor, 2);
    g.ch_mut(actor).points.gold = 1234;
    let item = mud_game::db::read_object(g, 0).unwrap();
    g.obj_mut(item).extra_flags.remove(flags::ITEM_NORENT);
    g.obj_mut(item).type_flag = flags::ITEM_TREASURE;
    g.obj_mut(item).cost_per_day = 1;
    mud_game::handler::obj_to_char(g, item, actor);
    assert!(mud_game::copyover::take_copyover_plan(g).is_some());
    let (saved, _) = mud_world::players::load_char(&g.lib_dir, b"Traveller").unwrap();
    assert_eq!(saved.gold, 1234);
    assert_eq!(saved.load_room, g.world.rooms[2].vnum as i32);
    let path = g.lib_dir.join(mud_world::players::get_filename(mud_world::players::FileKind::Objs, b"Traveller").unwrap());
    let data = std::fs::read(path).unwrap();
    let records = mud_game::objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(&data));
    assert_eq!(records.len(), 1);
}

#[test]
fn copyover_uses_connections_present_at_handoff() {
    let mut f = fixture("connections"); let g = &mut f.game;
    let departing = player(g, b"Departing", 12345);
    mud_game::handler::char_to_room(g, departing, 0);
    let old_di = descriptor(g, departing, ConState::Playing);
    mud_game::copyover::do_copyover(g, departing, b"", 0, 0);
    mud_game::run::close_socket(g, old_di);
    let arriving = player(g, b"Arriving", 12346);
    mud_game::handler::char_to_room(g, arriving, 1);
    let new_di = descriptor(g, arriving, ConState::Playing);
    let plan = mud_game::copyover::take_copyover_plan(g).unwrap();
    assert_eq!(plan.descs, vec![new_di]);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].name, b"Arriving");
    assert!(mud_game::copyover::take_copyover_plan(g).is_none());
}

#[test]
fn switching_after_the_request_still_saves_the_original_player() {
    let mut f = fixture("switched"); let g = &mut f.game;
    let actor = player(g, b"Original", 12345);
    mud_game::handler::char_to_room(g, actor, 0);
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::copyover::do_copyover(g, actor, b"", 0, 0);
    let body = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, body, 1);
    g.descriptors.get_mut(di).unwrap().original = Some(actor);
    g.descriptors.get_mut(di).unwrap().character = Some(body);
    g.ch_mut(actor).desc = None; g.ch_mut(body).desc = Some(di);
    let plan = mud_game::copyover::take_copyover_plan(g).unwrap();
    assert_eq!(plan.entries[0].name, b"Original");
    assert_eq!(g.descriptors.get(di).unwrap().character, Some(actor));
    assert_eq!(g.ch(body).desc, None);
}
