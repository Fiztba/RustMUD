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
    let root = std::env::temp_dir().join(format!("rustmud-summon-callback-{}-{label}", std::process::id()));
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

use mud_game::{dg::{self, GoId}, handler::*};
fn listener(g: &mut Game, verb: &[u8], body: &[&[u8]]) -> mud_data::ids::CharId {
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(mob).script = None;
    g.ch_mut(mob).affected_by = Default::default();
    g.ch_mut(mob).position = POS_STANDING;
    char_to_room(g, mob, 0);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 60000 + nr, name: Some(b"broadcast listener".to_vec()),
        attach_type: dg::MOB_TRIGGER, trigger_type: dg::MTRIG_ACT,
        narg: 1, arglist: Some(verb.to_vec()),
        cmdlist: body.iter().map(|s| s.to_vec()).collect(), ..Default::default()
    });
    let t = dg::read_trigger(g, nr).unwrap(); dg::add_trigger_at(g.ensure_script(GoId::Char(mob)), t, -1);
    mob
}

fn item(g: &mut Game, name: &[u8]) -> mud_data::ids::ObjId {
    let mut obj = mud_game::obj::create_obj();
    obj.name = Some(name.to_vec()); obj.short_description = Some(name.to_vec());
    obj.wear_flags.set(mud_data::flags::ITEM_WEAR_TAKE);
    g.objs.insert(obj)
}
use mud_data::flags;
fn summon_case(mode: u8) {
    let mut f = fixture(&format!("mode-{mode}")); let g = &mut f.game;
    let actor = player(g, b"Actor", 12345); char_to_room(g, actor, 0); g.rooms[0].light = 1;
    descriptor(g, actor, ConState::Playing);
    let zombie = g.world.real_mobile(11).unwrap();
    g.world.mob_protos[zombie as usize].keywords = Some(b"servant".to_vec());
    g.world.mob_protos[zombie as usize].proto_script.clear();
    let verb = match mode { 2 => b"never".as_slice(), 4 => b"starts", _ => b"animates" };
    let body = match mode { 0 | 4 => b"mpurge corpse".as_slice(), 3 => b"mforce %actor% get corpse", _ => b"mpurge servant" };
    listener(g, verb, &[body]);
    let corpse = item(g, b"corpse"); g.obj_mut(corpse).type_flag = flags::ITEM_CONTAINER;
    g.obj_mut(corpse).values[3] = 1; obj_to_room(g, corpse, 0);
    let gem = item(g, b"gem"); obj_to_obj(g, gem, corpse);
    let before = g.character_list.clone();
    let mut summoned = None;
    for _ in 0..20 {
        mud_game::magic::mag_summons(g, 20, actor, Some(corpse), mud_data::spells::SPELL_ANIMATE_DEAD, 0);
        summoned = g.character_list.iter().copied().find(|id| !before.contains(id) && g.ch(*id).mob_rnum == zombie);
        if summoned.is_some() { break; }
    }
    let summoned = summoned.expect("deterministic attempts did not summon a creature");
    if mode == 1 {
        assert!(g.ch(summoned).mob_flagged(flags::MOB_NOTDEADYET));
        assert_eq!(g.ch(summoned).master, None);
        assert!(g.try_obj(corpse).is_some());
        assert_eq!(g.obj(gem).in_obj, Some(corpse));
    } else if mode == 3 {
        assert_eq!(g.obj(corpse).carried_by, Some(actor));
        assert_eq!(g.obj(gem).in_obj, Some(corpse));
        assert_eq!(g.ch(summoned).master, Some(actor));
    } else {
        assert!(g.try_obj(corpse).is_none());
        assert_eq!(g.ch(summoned).master, Some(actor));
        if mode == 0 || mode == 4 { assert!(g.try_obj(gem).is_none()); }
        else { assert_eq!(g.obj(gem).carried_by, Some(summoned)); }
        let count = g.character_list.len();
        mud_game::magic::mag_summons(g, 20, actor, Some(corpse), mud_data::spells::SPELL_ANIMATE_DEAD, 0);
        assert_eq!(g.character_list.len(), count);
    }
}
#[test] fn animation_survives_corpse_removal_during_its_announcement() { summon_case(0); }
#[test] fn a_removed_summon_does_not_take_the_corpses_contents() { summon_case(1); }
#[test] fn normal_animation_transfers_contents_and_adds_a_follower() { summon_case(2); }

#[test] fn a_relocated_corpse_keeps_its_contents() { summon_case(3); }
#[test] fn follower_announcement_corpse_removal_is_safe() { summon_case(4); }
