use mud_game::{game::Game, mail};
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
    let root = std::env::temp_dir().join(format!("rustmud-mail-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

#[test]
fn malformed_mailbox_is_never_rewritten() {
    let mut f = fixture("malformed");
    let path = f.game.lib_dir.join("etc/plrmail");
    for data in [
        b"### 1 2 3\nfirst~\nmalformed header\nvaluable mail~\n".as_slice(),
        b"### 1 2 3\nfirst~\n### 4 5 6\nunterminated body".as_slice(),
    ] {
        std::fs::write(&path, data).unwrap();
        let _ = mail::read_delete(&mut f.game, 1);
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }
}

#[test]
fn failed_rewrite_preserves_mail_and_does_not_deliver_it() {
    let mut f = fixture("failed-rewrite");
    let path = f.game.lib_dir.join("etc/plrmail");
    let original = b"### 1 2 3\nunique message~\n";
    std::fs::write(&path, original).unwrap();
    std::fs::create_dir(f.game.lib_dir.join("etc/plrmail_tmp")).unwrap();
    let result = mail::read_delete(&mut f.game, 1);
    assert!(result.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn successful_delivery_removes_only_the_selected_letter() {
    let mut f = fixture("successful-delivery");
    let path = f.game.lib_dir.join("etc/plrmail");
    let original = b"### 1 2 3\nfirst~\n### 9 2 3\nother~\n### 1 2 3\nsecond~\n";
    std::fs::write(&path, original).unwrap();
    assert!(mail::read_delete(&mut f.game, 42).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(mail::read_delete(&mut f.game, 1).unwrap().ends_with(b"first"));
    assert_eq!(std::fs::read(&path).unwrap(), b"### 9 2 3\nother~\n### 1 2 3\nsecond~\n");
    assert!(mail::read_delete(&mut f.game, 1).unwrap().ends_with(b"second"));
    assert!(mail::read_delete(&mut f.game, 1).is_none());
    assert!(mail::read_delete(&mut f.game, 9).unwrap().ends_with(b"other"));
    assert!(std::fs::read(&path).unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn failed_windows_replacement_preserves_the_original() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut f = fixture("locked-replacement");
    let path = f.game.lib_dir.join("etc/plrmail");
    let original = b"### 1 2 3\nvaluable mail~\n";
    std::fs::write(&path, original).unwrap();
    // Permit the parser to read, but deny replacement of the open file.
    let lock = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
    assert!(mail::has_mail(&mut f.game, 1));
    assert!(mail::read_delete(&mut f.game, 1).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    drop(lock);
    assert!(mail::read_delete(&mut f.game, 1).unwrap().ends_with(b"valuable mail"));
}
