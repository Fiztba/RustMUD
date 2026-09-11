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
    let root = std::env::temp_dir().join(format!("rustmud-switched-body-{}-{label}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    let game = mud_game::run::boot_game(lib, mud_game::run::BootFlags {
        mini_mud: true, no_rent_check: true, no_specials: true, restrict: 0,
    }, 1, 1_800_000_000).unwrap();
    Fixture { game, root }
}

fn immortal(g: &mut Game, name: &[u8], idnum: i64) -> mud_data::ids::CharId {
    let ch = g.chars.insert(Char {
        name: Some(name.to_vec()), idnum, level: LVL_IMPL,
        passwd: mud_data::crypt::crypt(b"secret", name).unwrap().to_vec(),
        player_specials: Some(Box::new(PlayerSpecials::default())),
        ..Char::default()
    });
    g.character_list.push_back(ch);
    mud_game::handler::char_to_room(g, ch, 0);
    ch
}

fn descriptor(g: &mut Game, ch: mud_data::ids::CharId) -> usize {
    let mut d = Descriptor::new(None, b"localhost", 0, g.now, false);
    d.character = Some(ch);
    d.state = ConState::Playing;
    let di = g.descriptors.insert(d);
    g.ch_mut(ch).desc = Some(di);
    di
}

fn mob(g: &mut Game) -> mud_data::ids::CharId {
    let mob = mud_game::db::read_mobile(g, 0).unwrap();
    g.ch_mut(mob).script = None;
    g.ch_mut(mob).position = POS_STANDING;
    mud_game::handler::char_to_room(g, mob, 0);
    mob
}

#[test]
fn extracting_a_switched_into_body_returns_the_switcher_to_their_own_body() {
    let mut f = fixture("switched"); let g = &mut f.game;
    let imm = immortal(g, b"Switcher", 12345);
    let di = descriptor(g, imm);
    let body = mob(g);

    let keyword = g.ch(body).name.clone().unwrap();
    let keyword = keyword.split(|b| *b == b' ').next().unwrap().to_vec();
    mud_game::act::wizard::do_switch(g, imm, &keyword, 0, 0);
    assert_eq!(g.descriptors.get(di).unwrap().character, Some(body), "switch should take the mob body");
    assert_eq!(g.descriptors.get(di).unwrap().original, Some(imm));
    assert_eq!(g.ch(imm).desc, None);
    g.descriptors.get_mut(di).unwrap().output.clear();

    // Another immortal purges the body (or it dies): the mob is queued and
    // reaped at the end of the pulse.
    mud_game::handler::extract_char(g, body);
    mud_game::handler::extract_pending_chars(g);

    let d = g.descriptors.get(di).expect("descriptor survives the extraction");
    assert_eq!(d.character, Some(imm), "descriptor must point at the switcher's own body");
    assert_eq!(d.original, None);
    assert_eq!(d.state, ConState::Playing, "the switcher stays in the game, not at the menu");
    assert!(
        d.output.windows(b"You return to your original body.".len()).any(|w| w == b"You return to your original body."),
        "do_return's message should be delivered"
    );
    assert_eq!(g.ch(imm).desc, Some(di));
    assert!(g.try_ch(body).is_none(), "the vacated mob body should be freed");
    assert!(!g.character_list.contains(&body));
    assert_eq!(g.extractions_pending, 0);
}

#[test]
fn extracting_a_plain_mob_frees_it_without_touching_descriptors() {
    let mut f = fixture("plain"); let g = &mut f.game;
    let imm = immortal(g, b"Bystander", 12346);
    let di = descriptor(g, imm);
    let body = mob(g);

    mud_game::handler::extract_char(g, body);
    mud_game::handler::extract_pending_chars(g);

    assert!(g.try_ch(body).is_none(), "the mob should be freed");
    assert!(!g.character_list.contains(&body));
    let d = g.descriptors.get(di).unwrap();
    assert_eq!(d.character, Some(imm));
    assert_eq!(d.original, None);
    assert_eq!(d.state, ConState::Playing);
    assert!(d.output.is_empty(), "an unrelated player gets no output");
    assert_eq!(g.ch(imm).desc, Some(di));
    assert_eq!(g.extractions_pending, 0);
}
