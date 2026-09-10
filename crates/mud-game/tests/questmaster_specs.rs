use mud_game::game::Game;
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
    let root = std::env::temp_dir().join(format!("rustmud-questmaster-spec-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

use mud_game::{olc::genqst::{add_quest, delete_quest}, spec::MobSpec};
use mud_world::model::Quest;
#[test]
fn removing_the_first_quest_preserves_the_masters_original_job() {
    let mut f = fixture("delete"); let g = &mut f.game;
    g.world.quests.clear(); g.quest_secondary.clear();
    let qm = g.world.mob_protos[0].vnum as i32;
    g.mob_specs[0] = Some(MobSpec::Postmaster);
    add_quest(g, &Quest { vnum: 60000, qm_vnum: qm, ..Default::default() }, None);
    add_quest(g, &Quest { vnum: 60001, qm_vnum: qm, ..Default::default() }, None);
    assert!(delete_quest(g, 0));
    assert_eq!(g.quest_secondary[0], Some(MobSpec::Postmaster));
    assert_eq!(g.mob_specs[0], Some(MobSpec::QuestMaster));
    assert!(delete_quest(g, 0));
    assert_eq!(g.mob_specs[0], Some(MobSpec::Postmaster));
}
#[test]
fn moving_a_quest_restores_the_old_master_and_uses_the_new_masters_job() {
    let mut f = fixture("move"); let g = &mut f.game;
    g.world.quests.clear(); g.quest_secondary.clear();
    let old = g.world.mob_protos[0].vnum as i32; let new = g.world.mob_protos[1].vnum as i32;
    g.mob_specs[0] = Some(MobSpec::Postmaster); g.mob_specs[1] = None;
    let mut quest = Quest { vnum: 60000, qm_vnum: old, ..Default::default() };
    add_quest(g, &quest, None); let copied_func = g.quest_secondary[0];
    quest.qm_vnum = new; add_quest(g, &quest, copied_func);
    assert_eq!(g.mob_specs[0], Some(MobSpec::Postmaster));
    assert_eq!(g.quest_secondary[0], None);
    delete_quest(g, 0); assert_eq!(g.mob_specs[1], None);
}

#[test]
fn boot_assignment_and_reordered_edits_keep_each_masters_own_job() {
    let mut f = fixture("boot"); let g = &mut f.game;
    g.world.quests.clear(); g.quest_secondary.clear();
    let a = g.world.mob_protos[0].vnum as i32; let b = g.world.mob_protos[1].vnum as i32;
    g.mob_specs[0] = Some(MobSpec::Postmaster); g.mob_specs[1] = Some(MobSpec::Receptionist);
    for (vnum, master) in [(60000, a), (60001, a), (60002, b)] {
        g.world.quests.push(Quest { vnum, qm_vnum: master, ..Default::default() });
        g.quest_secondary.push(None);
    }
    mud_game::quest::assign_the_quests(g);
    assert_eq!(g.quest_secondary, [Some(MobSpec::Postmaster), Some(MobSpec::Postmaster), Some(MobSpec::Receptionist)]);
    mud_game::quest::assign_the_quests(g);
    let mut edited = g.world.quests[0].clone(); edited.qm_vnum = b;
    add_quest(g, &edited, Some(MobSpec::Postmaster));
    assert_eq!(g.quest_secondary, [Some(MobSpec::Receptionist), Some(MobSpec::Postmaster), Some(MobSpec::Receptionist)]);
    assert_eq!(g.mob_specs[0], Some(MobSpec::QuestMaster));
    add_quest(g, &Quest { vnum: 59999, qm_vnum: b, ..Default::default() }, Some(MobSpec::Mayor));
    assert_eq!(g.quest_secondary[0], Some(MobSpec::Receptionist));
    let current = g.world.quests[0].clone(); add_quest(g, &current, None);
    assert_eq!(g.quest_secondary[0], Some(MobSpec::Receptionist));
    for vnum in [60002, 59999, 60000, 60001] {
        let rnum = mud_game::quest::real_quest(g, vnum).unwrap(); assert!(delete_quest(g, rnum));
    }
    assert_eq!(g.mob_specs[0], Some(MobSpec::Postmaster));
    assert_eq!(g.mob_specs[1], Some(MobSpec::Receptionist));
    assert!(!delete_quest(g, 0));
    for master in [-1, 65535, i32::MAX] {
        add_quest(g, &Quest { vnum: 60000, qm_vnum: master, ..Default::default() }, Some(MobSpec::Mayor));
        delete_quest(g, 0);
        assert_eq!(g.mob_specs[0], Some(MobSpec::Postmaster));
        assert_eq!(g.mob_specs[1], Some(MobSpec::Receptionist));
    }
}
