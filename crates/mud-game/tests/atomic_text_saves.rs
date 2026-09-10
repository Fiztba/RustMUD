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
    let root = std::env::temp_dir().join(format!("rustmud-atomic-saves-{}-{label}", std::process::id()));
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

/// The bytes `hedit_save_to_disk` has always produced, built the same way
/// the in-place writer did, so the temp-file path must match it exactly.
fn expected_help_file(g: &Game) -> Vec<u8> {
    let mut out = Vec::new();
    for e in &g.help_table {
        if e.duplicate != 0 { continue; }
        let mut buf = if e.entry.is_empty() { b"Empty\r\n".to_vec() } else { e.entry.as_ref().clone() };
        buf.retain(|&b| b != b'\r');
        mud_net::editor::parse_tab(&mut buf);
        out.extend_from_slice(&buf);
        out.extend_from_slice(format!("#{}\n", e.min_level).as_bytes());
    }
    out.extend_from_slice(b"$~\n");
    out
}

#[test]
fn help_save_replaces_the_file_whole_and_leaves_no_temporary() {
    let mut f = fixture("help-save"); let g = &mut f.game;
    let expected = expected_help_file(g);
    let path = g.lib_dir.join("text/help/help.hlp");
    assert!(path.exists(), "rename must replace an existing help.hlp");
    assert!(mud_game::olc::hedit::hedit_save_to_disk(g));
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    assert!(!path.with_extension("tmp").exists());
}

#[test]
fn failed_help_save_preserves_original_file_and_reports_failure() {
    let mut f = fixture("help-failure"); let g = &mut f.game;
    let path = g.lib_dir.join("text/help/help.hlp");
    let before = std::fs::read(&path).unwrap();
    let entries = g.help_table.len();
    let temporary = path.with_extension("tmp"); std::fs::create_dir(&temporary).unwrap();
    assert!(!mud_game::olc::hedit::hedit_save_to_disk(g), "a failed write must be reported");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(g.help_table.len(), entries);
    std::fs::remove_dir(&temporary).unwrap();
    assert!(mud_game::olc::hedit::hedit_save_to_disk(g));
    let mut log = Vec::new();
    assert_eq!(mud_game::text::boot_help(&g.lib_dir, g.mini_mud, &mut log).len(), entries);
}

#[test]
fn social_save_replaces_the_file_whole_and_reloads_every_social() {
    let mut f = fixture("social-save"); let g = &mut f.game;
    let path = g.lib_dir.join("misc/socials.new");
    let count = g.socials.len();
    assert!(mud_game::olc::aedit::aedit_save_to_disk(g));
    assert_eq!(std::fs::read(&path).unwrap(), expected_social_file(g));
    assert!(!path.with_extension("tmp").exists());
    let (socials, _) = mud_game::social::boot_social_messages(&g.lib_dir).unwrap();
    assert_eq!(socials.len(), count);
}

#[test]
fn failed_social_save_preserves_original_file_and_reports_failure() {
    let mut f = fixture("social-failure"); let g = &mut f.game;
    let path = g.lib_dir.join("misc/socials.new");
    let before = std::fs::read(&path).unwrap();
    let temporary = path.with_extension("tmp"); std::fs::create_dir(&temporary).unwrap();
    assert!(!mud_game::olc::aedit::aedit_save_to_disk(g), "a failed write must be reported");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(&temporary).unwrap();
    assert!(mud_game::olc::aedit::aedit_save_to_disk(g));
    assert!(mud_game::social::boot_social_messages(&g.lib_dir).is_ok());
}

/// The bytes `aedit_save_to_disk` has always produced for the current
/// social table, built the way the in-place writer did.
fn expected_social_file(g: &Game) -> Vec<u8> {
    let mut out = Vec::new();
    for s in &g.socials {
        out.push(b'~');
        out.extend_from_slice(&s.command);
        out.push(b' ');
        out.extend_from_slice(&s.sort_as);
        out.extend_from_slice(
            format!(" {} {} {} {}\n", s.hide, s.min_char_position, s.min_victim_position, s.min_level_char).as_bytes(),
        );
        let groups: [&[&Option<Vec<u8>>]; 4] = [
            &[&s.char_no_arg, &s.others_no_arg, &s.char_found, &s.others_found],
            &[&s.vict_found, &s.not_found, &s.char_auto, &s.others_auto],
            &[&s.char_body_found, &s.others_body_found, &s.vict_body_found],
            &[&s.char_obj_found, &s.others_obj_found],
        ];
        for (n, group) in groups.iter().enumerate() {
            let mut buf = Vec::new();
            for field in group.iter() {
                match field { Some(t) => buf.extend_from_slice(t), None => buf.push(b'#') }
                buf.push(b'\n');
            }
            if n == 3 { buf.push(b'\n'); }
            mud_net::editor::parse_tab(&mut buf);
            out.extend_from_slice(&buf);
        }
    }
    out.extend_from_slice(b"$\n");
    out
}

#[test]
fn text_save_replaces_the_file_whole_and_leaves_no_temporary() {
    let mut f = fixture("text-save"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, actor, b"news", 0, 0);
    let olc = g.olc.remove(&di).unwrap();
    mud_game::olc::tedit::tedit_string_cleanup(g, di, olc, Some(b"First\r\nSecond\r\n".to_vec()), true);
    let path = g.lib_dir.join("text/news");
    assert_eq!(std::fs::read(&path).unwrap(), b"First\nSecond\n");
    assert!(!path.with_extension("tmp").exists());
    assert_eq!(g.texts.news, b"First\r\nSecond\r\n");
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(output.contains("Saved."));
}

#[test]
fn failed_text_save_preserves_original_file_and_buffer() {
    let mut f = fixture("text-failure"); let g = &mut f.game;
    let actor = player(g, b"Admin", 12345); g.ch_mut(actor).level = LVL_IMPL;
    let di = descriptor(g, actor, ConState::Playing);
    mud_game::olc::tedit::do_tedit(g, actor, b"news", 0, 0);
    let olc = g.olc.remove(&di).unwrap();
    let path = g.lib_dir.join("text/news");
    let before = std::fs::read(&path).unwrap();
    let original = g.texts.news.clone();
    let temporary = path.with_extension("tmp"); std::fs::create_dir(&temporary).unwrap();
    mud_game::olc::tedit::tedit_string_cleanup(g, di, olc, Some(b"Replacement\r\n".to_vec()), true);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(g.texts.news, original);
    let output = String::from_utf8_lossy(&g.descriptors.get(di).unwrap().output);
    assert!(!output.contains("Saved."), "a failed write must not be reported as saved");
    std::fs::remove_dir(&temporary).unwrap();
}
