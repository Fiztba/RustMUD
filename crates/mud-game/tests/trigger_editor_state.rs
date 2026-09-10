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
    let root = std::env::temp_dir().join(format!("rustmud-trigger-state-{}-{label}", std::process::id()));
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

fn save_trigger(g: &mut Game, di: usize, vnum: i32, types: u32) {
    let mut olc = mud_game::olc::OlcData::default();
    olc.number = vnum;
    olc.zone_num = g.world.real_zone(30).unwrap() as i32;
    olc.trig = Some(Box::new(mud_world::model::Trigger { vnum: vnum as Idx,
        attach_type: mud_game::dg::MOB_TRIGGER, trigger_type: types, narg: 100,
        name: Some(format!("trigger {vnum}").into_bytes()), arglist: Some(b"probe".to_vec()), ..Default::default() }));
    olc.storage = Some(b"set edited yes\r\nglobal edited\r\nreturn 1\r\n".to_vec());
    mud_game::olc::trigedit::trigedit_save(g, di, &mut olc);
}

#[test]
fn live_and_menu_scripts_follow_trigger_edits() {
    use mud_game::{dg::*, olc::trigedit::*};
    let mut f = fixture("live");
    let g = &mut f.game;
    let builder = player(g, b"Builder", 12345);
    g.ch_mut(builder).level = LVL_IMPL;
    let di = descriptor(g, builder, ConState::Playing);
    mud_game::handler::char_to_room(g, builder, 0);
    let menu = player(g, b"Menu", 12346);
    descriptor(g, menu, ConState::Menu);
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, mob, 0);
    save_trigger(g, di, 3098, MTRIG_RANDOM);
    save_trigger(g, di, 3099, MTRIG_GREET);
    for owner in [GoId::Char(mob), GoId::Char(menu)] {
        let mut t = read_trigger(g, g.world.real_trigger(3098).unwrap()).unwrap();
        let event_id = t.iid;
        t.wait_event = Some(event_id);
        g.queue_event(100, mud_game::game::EventKind::TrigWait { go: owner, iid: t.iid, event_id });
        add_trigger_at(g.ensure_script(owner), t, -1);
        let t = read_trigger(g, g.world.real_trigger(3099).unwrap()).unwrap();
        add_trigger_at(g.ensure_script(owner), t, -1);
        add_var(&mut g.ensure_script(owner).global_vars, b"saved", b"keep", 17);
    }
    save_trigger(g, di, 3098, MTRIG_COMMAND);
    for owner in [GoId::Char(mob), GoId::Char(menu)] {
        assert!(g.script_check(owner, MTRIG_COMMAND));
        assert!(!g.script_check(owner, MTRIG_RANDOM));
        assert!(g.script_check(owner, MTRIG_GREET));
        assert!(g.script_of(owner).unwrap().trig_list[0].wait_event.is_none());
    }
    assert!(!g.events.iter().any(|e| matches!(e.kind, mud_game::game::EventKind::TrigWait { go: GoId::Char(c), .. } if c == mob || c == menu)));
    g.ch_mut(builder).level = 10;
    assert!(mud_game::dg::triggers::command_mtrigger(g, builder, b"probe", b""));
    g.ch_mut(builder).level = LVL_IMPL;
    assert!(g.script_of(GoId::Char(mob)).unwrap().global_vars.iter().any(|v| v.name == b"edited" && v.value == b"yes"));
    save_trigger(g, di, 3097, MTRIG_GREET);
    for owner in [GoId::Char(mob), GoId::Char(menu)] {
        assert_eq!(g.script_of(owner).unwrap().trig_list[0].nr, g.world.real_trigger(3098).unwrap());
    }
    let rnum = g.world.real_trigger(3098).unwrap();
    assert!(delete_trigger(g, rnum));
    for owner in [GoId::Char(mob), GoId::Char(menu)] {
        assert_eq!(g.script_of(owner).unwrap().types, MTRIG_GREET);
        assert_eq!(g.script_of(owner).unwrap().trig_list.len(), 1);
    }
    let rnum = g.world.real_trigger(3099).unwrap();
    assert!(delete_trigger(g, rnum));
    for owner in [GoId::Char(mob), GoId::Char(menu)] {
        let sc = g.script_of(owner).expect("deleting a trigger erased persistent variables");
        assert!(sc.trig_list.is_empty());
        assert_eq!(sc.types, 0);
        assert!(sc.global_vars.iter().any(|v| v.name == b"saved" && v.value == b"keep" && v.context == 17));
    }
}

#[test]
fn copied_triggers_and_attachment_positions_are_saved() {
    use mud_game::{dg::*, olc::{OlcData, trigedit::*}};
    let mut f = fixture("copy");
    let g = &mut f.game;
    let builder = player(g, b"Builder", 12345);
    g.ch_mut(builder).level = LVL_IMPL;
    let di = descriptor(g, builder, ConState::Playing);
    mud_game::handler::char_to_room(g, builder, 0);
    for vnum in [3097,3098,3099] { save_trigger(g, di, vnum, MTRIG_COMMAND); }
    do_oasis_trigedit(g, builder, b"3099", 0, 0);
    for input in [b"w".as_slice(), b"3098", b"q"] { assert!(mud_game::olc::olc_parse(g, di, input)); }
    assert_eq!(g.olc[&di].mode, TRIGEDIT_CONFIRM_SAVESTRING);
    assert!(mud_game::olc::olc_parse(g, di, b"y"));
    assert_eq!(g.world.triggers[g.world.real_trigger(3099).unwrap() as usize].name.as_deref(), Some(b"trigger 3098".as_slice()));
    let mut olc = OlcData::default();
    olc.item_type = MOB_TRIGGER;
    olc.script = Some(vec![3097,3099]);
    olc.script_mode = SCRIPT_NEW_TRIGGER;
    dg_script_edit_parse(g, di, &mut olc, b"2, 3098");
    assert_eq!(olc.script, Some(vec![3097,3098,3099]));
    for position in 1..=5 {
        olc.script = Some(vec![3097,3099]);
        olc.script_mode = SCRIPT_NEW_TRIGGER;
        dg_script_edit_parse(g, di, &mut olc, format!("{position}, 3098").as_bytes());
        let mut expected = vec![3097,3099];
        expected.insert((position - 1).min(2), 3098);
        assert_eq!(olc.script, Some(expected));
    }
    for kind in [-1, 0, 1, 2, 3, i32::MAX, i32::MIN] {
        let mut editing = Box::new(OlcData::default());
        editing.mode = TRIGEDIT_INTENDED;
        editing.trig = Some(Box::new(mud_world::model::Trigger { attach_type: OBJ_TRIGGER, ..Default::default() }));
        let editing = trigedit_parse(g, di, editing, kind.to_string().as_bytes()).unwrap();
        assert_eq!(editing.trig.unwrap().attach_type, if (0..=2).contains(&kind) { kind } else { OBJ_TRIGGER });
    }
}
