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

#[test]
fn received_letters_are_independent_of_world_prototypes() {
    use mud_data::{flags, types::*};
    use mud_game::{ch::{Char, PlayerSpecials}, handler, objsave};
    let mut f = fixture("letter-prototype");
    let g = &mut f.game;
    let ch = g.chars.insert(Char {
        name: Some(b"Reader".to_vec()), idnum: 12345, level: 10,
        player_specials: Some(Box::new(PlayerSpecials::default())), ..Default::default()
    });
    let mut d = mud_net::descriptor::Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = ConState::Playing;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    handler::char_to_room(g, ch, 0);
    let counts = g.obj_counts.clone();
    std::fs::write(g.lib_dir.join("etc/plrmail"), b"### 12345 2 3\nunique letter body~\n").unwrap();
    let receive = mud_game::interpreter::find_command(g, b"receive").unwrap();
    assert!(mail::postmaster(g, ch, ch, receive, b""));
    let letter = g.ch(ch).carrying[0];
    assert_eq!(g.obj(letter).item_number, NOTHING);
    assert!(!objsave::crash_is_unrentable(g, letter));
    let body = g.obj(letter).action_description.clone();
    let mut saved = Vec::new();
    objsave::objsave_save_obj_record(g, letter, &mut saved, 0);
    handler::extract_obj(g, letter);
    assert_eq!(g.obj_counts, counts);
    let records = objsave::objsave_parse_objects(g, &mut mud_world::lex::Reader::new(&saved));
    assert_eq!(records.len(), 1);
    let restored = records[0].obj;
    assert_eq!(g.obj(restored).item_number, NOTHING);
    assert_eq!(g.obj(restored).type_flag, flags::ITEM_NOTE);
    assert_eq!(g.obj(restored).action_description, body);
    assert_eq!(g.obj(restored).weight, 1);
    assert_eq!(g.obj(restored).cost_per_day, 10);
    assert!(g.obj(restored).script.is_none());
    assert!(!objsave::crash_is_unrentable(g, restored));
    handler::obj_to_char(g, restored, ch);
    objsave::crash_rentsave(g, ch, 0);
    assert!(g.ch(ch).carrying.is_empty());
    objsave::crash_load(g, ch);
    assert_eq!(g.ch(ch).carrying.len(), 1);
    let rented = g.ch(ch).carrying[0];
    assert_eq!(g.obj(rented).action_description, body);
    assert_eq!(g.obj(rented).item_number, NOTHING);
    // Explicit nonrent flags and negative rent must still take precedence.
    g.obj_mut(rented).extra_flags.set(flags::ITEM_NORENT);
    assert!(objsave::crash_is_unrentable(g, rented));
    g.obj_mut(rented).extra_flags.remove(flags::ITEM_NORENT);
    g.obj_mut(rented).cost_per_day = -1;
    assert!(objsave::crash_is_unrentable(g, rented));
    g.obj_mut(rented).cost_per_day = 10;
    g.obj_mut(rented).type_flag = flags::ITEM_TREASURE;
    assert!(objsave::crash_is_unrentable(g, rented));
    let last_proto = (g.world.obj_protos.len() - 1) as u16;
    mud_game::olc::genobj::delete_object(g, last_proto).unwrap();
    assert_eq!(g.obj(rented).item_number, NOTHING);
    let counts_after_edit = g.obj_counts.clone();
    handler::extract_obj(g, rented);
    assert_eq!(g.obj_counts, counts_after_edit);
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
