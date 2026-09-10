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
    let root = std::env::temp_dir().join(format!("rustmud-script-apply-{}-{label}", std::process::id()));
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

use mud_game::{dg::{self, GoId}, handler};
#[test]
fn script_apply_changes_update_wearer_and_remain_reversible() {
    let mut f = fixture("worn");
    let g = &mut f.game;
    let ch = player(g, b"Wearer", 12345);
    let _di = descriptor(g, ch, ConState::Playing);
    let oid = g.objs.insert(mud_game::obj::create_obj());
    let go = GoId::Obj(oid);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger { vnum: 65000, attach_type: dg::OBJ_TRIGGER, ..Default::default() });
    let trigger = dg::read_trigger(g, nr).unwrap();
    let ctx = dg::DgCtx { go, iid: trigger.iid };
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    assert!(handler::equip_char(g, ch, oid, WEAR_BODY));
    let before = g.ch(ch).points.max_hit;
    g.ch_mut(ch).act.remove(flags::PLR_CRASH);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.oset(apply MAXHIT 7)%"), b"1");
    assert_eq!(g.ch(ch).points.max_hit, before + 7);
    assert!(g.ch(ch).act.is_set(flags::PLR_CRASH));
    assert_eq!(g.ch(ch).equipment[WEAR_BODY], Some(oid));
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.oset(apply MAXHIT -3)%"), b"1");
    assert_eq!(g.ch(ch).points.max_hit, before + 4);
    assert_eq!(handler::unequip_char(g, ch, WEAR_BODY), Some(oid));
    assert_eq!(g.ch(ch).points.max_hit, before);
    assert!(handler::equip_char(g, ch, oid, WEAR_BODY));
    assert_eq!(g.ch(ch).points.max_hit, before + 4);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.oset(apply MAXHIT -4)%"), b"1");
    assert_eq!(g.ch(ch).points.max_hit, before);
    assert!(g.obj(oid).affected.iter().all(|a| a.location == flags::APPLY_NONE));
}

fn stat_snapshot(g: &Game, ch: mud_data::ids::CharId) -> String {
    let c = g.ch(ch);
    format!("{:?} {:?} {:?} {} {} {}", c.points, c.aff_abils, c.apply_saving_throw, c.time.birth, c.weight, c.height)
}
#[test]
fn script_applies_cover_all_locations_and_numeric_limits() {
    let mut f = fixture("limits");
    let g = &mut f.game;
    let ch = player(g, b"Wearer", 12345);
    let _di = descriptor(g, ch, ConState::Playing);
    let oid = g.objs.insert(mud_game::obj::create_obj());
    let go = GoId::Obj(oid);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger { vnum: 65000, attach_type: dg::OBJ_TRIGGER, ..Default::default() });
    let trigger = dg::read_trigger(g, nr).unwrap();
    let ctx = dg::DgCtx { go, iid: trigger.iid };
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    assert!(handler::equip_char(g, ch, oid, WEAR_BODY));

    g.ch_mut(ch).real_abils = mud_game::ch::Abilities {
        str_: 10, str_add: 0, intel: 10, wis: 10, dex: 10, con: 10, cha: 10,
    };
    g.ch_mut(ch).points.max_hit = 100;
    g.ch_mut(ch).points.max_mana = 100;
    g.ch_mut(ch).points.max_move = 100;
    g.ch_mut(ch).points.armor = 100;
    g.ch_mut(ch).points.hitroll = 4;
    g.ch_mut(ch).points.damroll = 4;
    g.ch_mut(ch).apply_saving_throw = [30; 5];
    g.ch_mut(ch).weight = 100;
    g.ch_mut(ch).height = 100;
    g.ch_mut(ch).time.birth = 1_000_000;
    g.obj_mut(oid).perm_affects.set(flags::AFF_DETECT_INVIS);
    handler::affect_total(g, ch);
    let baseline = stat_snapshot(g, ch);
    for name in mud_data::tables::APPLY_TYPES.iter().skip(1) {
        for value in [7, i32::MAX, i32::MIN] {
            let expression = format!("%self.oset(apply {name} {value})%");
            assert_eq!(dg::variables::var_subst(g, ctx, expression.as_bytes()), b"1", "{name} {value}");
            let applied = stat_snapshot(g, ch);
            if value == i32::MAX || value == i32::MIN {
                let delta = if value > 0 { 1 } else { -1 };
                let expression = format!("%self.oset(apply {name} {delta})%");
                assert_eq!(dg::variables::var_subst(g, ctx, expression.as_bytes()), b"0");
                assert_eq!(stat_snapshot(g, ch), applied);
            }
            assert!(g.ch(ch).affected_by.is_set(flags::AFF_DETECT_INVIS));
            assert_eq!(handler::unequip_char(g, ch, WEAR_BODY), Some(oid));
            assert_eq!(stat_snapshot(g, ch), baseline, "removing {name} {value}");
            assert!(handler::equip_char(g, ch, oid, WEAR_BODY));
            assert_eq!(stat_snapshot(g, ch), applied, "equipping {name} {value}");
            // i32::MIN needs two legal increments to cancel.
            let increments = if value == i32::MIN { vec![i32::MAX, 1] } else { vec![-value] };
            for delta in increments {
                let expression = format!("%self.oset(apply {name} {delta})%");
                assert_eq!(dg::variables::var_subst(g, ctx, expression.as_bytes()), b"1");
            }
            assert_eq!(stat_snapshot(g, ch), baseline, "clearing {name} {value}");
        }
    }
    handler::unequip_char(g, ch, WEAR_BODY);
    handler::obj_to_char(g, oid, ch);
    g.ch_mut(ch).act.remove(flags::PLR_CRASH);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.oset(apply MAXHIT 2)%"), b"1");
    assert_eq!(stat_snapshot(g, ch), baseline);
    assert!(g.ch(ch).act.is_set(flags::PLR_CRASH));
    assert_eq!(g.obj(oid).carried_by, Some(ch));
    assert_eq!(dg::variables::var_subst(g, ctx, b"%self.oset(apply MAXHIT 0)%"), b"0");

}
