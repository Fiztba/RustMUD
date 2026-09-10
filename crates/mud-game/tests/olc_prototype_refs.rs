use mud_game::{game::Game, olc::{genobj, genmob}};
use std::path::{Path, PathBuf};
use mud_world::model::{ObjProto, Shop, ZoneCommand};
use mud_data::types::{NOTHING, NOBODY};

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
    let root = std::env::temp_dir().join(format!("rustmud-olcrefs-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn command(kind: u8, arg1: i32, arg2: i32, arg3: i32) -> ZoneCommand {
    ZoneCommand { command: kind, arg1, arg2, arg3, ..Default::default() }
}

#[test]
fn adding_an_object_keeps_unresolved_board_references() {
    let mut f = fixture("insert");
    let g = &mut f.game;
    let missing = (0..65535).find(|&v| g.world.real_object(v).is_none()).unwrap();
    g.boards.rnum.fill(NOTHING);
    let last = (g.world.obj_protos.len() - 1) as u16;
    g.boards.rnum[1] = last;
    let inserted = genobj::add_object(g, &ObjProto::default(), missing).unwrap();
    assert_eq!(g.boards.rnum[0], NOTHING);
    assert_eq!(g.boards.rnum[1], if last >= inserted { last + 1 } else { last });
}

#[test]
fn object_deletion_updates_every_reset_board_and_product_reference() {
    let mut f = fixture("object-delete");
    let g = &mut f.game;
    let deleted_vnum = g.world.obj_protos[1].vnum as i32;
    let surviving_vnum = g.world.obj_protos[2].vnum as i32;
    g.world.shops = vec![Shop {
        vnum: g.world.zones[0].bot, producing: vec![deleted_vnum, surviving_vnum], ..Default::default()
    }];
    g.shops_rt = vec![mud_game::shop::ShopRt { producing: vec![1, 2], ..Default::default() }];
    g.boards.rnum.fill(NOTHING);
    g.boards.rnum[0] = 1;
    g.boards.rnum[1] = 2;
    for zone in &mut g.world.zones { zone.cmds.clear(); }
    g.world.zones[0].cmds = vec![
        command(b'P', 2, 1, 1), command(b'M', 1, 1, 0),
        command(b'O', 1, 1, 0), command(b'O', 1, 1, 0), command(b'O', 2, 1, 0),
        command(b'R', 0, 1, 0), command(b'R', 0, 2, 0), command(b'P', 2, 1, 2),
    ];
    genobj::delete_object(g, 1).unwrap();
    let commands: Vec<_> = g.world.zones[0].cmds.iter()
        .map(|c| (c.command, c.arg1, c.arg2, c.arg3)).collect();
    assert_eq!(commands, vec![(b'M', 1, 1, 0), (b'O', 1, 1, 0), (b'R', 0, 1, 0), (b'P', 1, 1, 1)]);
    assert_eq!(g.boards.rnum[0], NOTHING);
    assert_eq!(g.boards.rnum[1], 1);
    assert_eq!(g.boards.rnum[2], NOTHING);
    assert_eq!(g.shops_rt[0].producing, vec![1]);
    assert_eq!(g.world.shops[0].producing, vec![surviving_vnum]);
}

#[test]
fn mobile_deletion_preserves_counts_until_pending_extraction() {
    let mut f = fixture("mobile-delete");
    let g = &mut f.game;
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    mud_game::handler::char_to_room(g, mob, 0);
    let surviving_vnum = g.world.mob_protos[1].vnum as i32;
    g.world.shops = vec![Shop {
        vnum: g.world.zones[0].bot, keeper_vnum: g.world.mob_protos[0].vnum as i32, ..Default::default()
    }, Shop { vnum: g.world.zones[0].bot + 1, keeper_vnum: surviving_vnum, ..Default::default() }];
    g.shops_rt = vec![mud_game::shop::ShopRt { keeper: 0, ..Default::default() },
        mud_game::shop::ShopRt { keeper: 1, ..Default::default() }];
    for zone in &mut g.world.zones { zone.cmds.clear(); }
    g.world.zones[0].cmds = vec![command(b'M', 0, 1, 0), command(b'M', 0, 1, 0), command(b'M', 1, 1, 0)];
    let counts = g.mob_counts[1..].to_vec();
    genmob::delete_mobile(g, 0).unwrap();
    assert_eq!(g.ch(mob).mob_rnum, NOBODY);
    mud_game::handler::extract_pending_chars(g);
    assert!(g.try_ch(mob).is_none());
    assert_eq!(g.mob_counts, counts);
    assert_eq!(g.world.zones[0].cmds.len(), 1);
    assert_eq!(g.world.zones[0].cmds[0].arg1, 0);
    assert_eq!(g.shops_rt[0].keeper, NOBODY);
    assert_eq!(g.world.shops[0].keeper_vnum, NOBODY as i32);
    assert_eq!(g.shops_rt[1].keeper, 0);
    assert_eq!(g.world.shops[1].keeper_vnum, surviving_vnum);
}


