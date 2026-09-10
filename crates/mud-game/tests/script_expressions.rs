use mud_game::{game::Game, dg::{self, GoId}};
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
    let root = std::env::temp_dir().join(format!("rustmud-expressions-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn field_arguments_are_local_to_each_expansion() {
    let mut f = fixture("arguments");
    let g = &mut f.game;
    let go = GoId::Room(0);
    dg::add_var(&mut g.ensure_script(go).global_vars, b"text", b"abcdef", 0);
    let nr = g.world.triggers.len() as u16;
    g.world.triggers.push(mud_world::model::Trigger {
        vnum: 65000, attach_type: dg::WLD_TRIGGER, ..Default::default()
    });
    let trigger = dg::read_trigger(g, nr).unwrap();
    let ctx = dg::DgCtx { go, iid: trigger.iid };
    dg::add_trigger_at(g.ensure_script(go), trigger, -1);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(1)% %text.charat(2)%"), b"a b");
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(2)% %text.charat(1)%"), b"b a");
    dg::add_var(&mut g.ensure_script(go).global_vars, b"index", b"2", 0);
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(%index%)% %text.charat(1)%"), b"b a");
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(2).charat(1)%"), b"b");
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(%index%).charat(1)% %text.strlen%"), b"b 6");
    assert_eq!(dg::variables::var_subst(g, ctx, b"%% %text.charat(1)% %text%"), b"% a abcdef");
}

#[test]
fn text_comparisons_and_searches_handle_order_and_overlaps() {
    assert_eq!(dg::expr::eval_op(b">=", b"b", b"a"), b"1");
    assert_eq!(dg::expr::eval_op(b">=", b"a", b"b"), b"0");
    assert_eq!(dg::expr::eval_op(b">=", b"A", b"a"), b"1");
    assert!(dg::variables::str_str(b"aaab", b"aab"));
    assert!(dg::triggers::is_substring(b"he", b"the he"));
    assert!(!dg::triggers::is_substring(b"he", b"the"));
    assert!(dg::triggers::is_substring(b"HE", b"the, he!"));
    assert!(!dg::variables::str_str(b"abc", b"abcd"));
    assert!(!dg::variables::str_str(b"abc", b""));
    assert_eq!(dg::expr::eval_op(b">=", b"10", b"2"), b"1");
    assert_eq!(dg::expr::eval_op(b">=", b"-2", b"-1"), b"0");
}
