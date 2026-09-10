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
    let ctx = dg::DgCtx { go, iid: 0 };
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(1)% %text.charat(2)%"), b"a b");
    assert_eq!(dg::variables::var_subst(g, ctx, b"%text.charat(2)% %text.charat(1)%"), b"b a");
}

#[test]
fn text_comparisons_and_searches_handle_order_and_overlaps() {
    assert_eq!(dg::expr::eval_op(b">=", b"b", b"a"), b"1");
    assert_eq!(dg::expr::eval_op(b">=", b"a", b"b"), b"0");
    assert_eq!(dg::expr::eval_op(b">=", b"A", b"a"), b"1");
    assert!(dg::variables::str_str(b"aaab", b"aab"));
    assert!(dg::triggers::is_substring(b"he", b"the he"));
    assert!(!dg::triggers::is_substring(b"he", b"the"));
}
