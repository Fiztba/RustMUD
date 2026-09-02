//! nanny — the connection state machine, plus character initialization
//! (init_char/do_start/advance_level) and enter_player_game.

use mud_data::flags::{self};
use mud_data::ids::CharId;
use mud_data::tables;
use mud_data::types::*;

use crate::act::BStr;
use crate::ch::{Char, PlayerSpecials, DRUNK, HUNGER, THIRST};
use crate::comm::{self, act, send_to_char};
use crate::game::{Game, MudlogKind};
use crate::handler::char_to_room;
use crate::interpreter::skip_spaces;

/// _parse_name: alphabetic only, case preserved.
fn parse_name(arg: &[u8]) -> Option<BStr> {
    let arg = skip_spaces(arg);
    if arg.is_empty() {
        return None;
    }
    if arg.iter().any(|c| !c.is_ascii_alphabetic()) {
        return None;
    }
    Some(arg.to_vec())
}

/// Reserved/fill words.
fn fill_or_reserved(name: &[u8]) -> bool {
    crate::interpreter::fill_word(&name.to_ascii_lowercase())
        || crate::interpreter::reserved_word(&name.to_ascii_lowercase())
}

pub fn valid_name(g: &Game, newname: &[u8]) -> bool {
    // (a) A connected in-creation character with the same name.
    for &di in &g.descriptors.order {
        let Some(d) = g.descriptors.get(di) else { continue };
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if let Some(n) = &ch.name {
            if n.eq_ignore_ascii_case(newname) && ch.idnum == -1 {
                // Still in creation: valid only if that desc is playing.
                return d.is_playing();
            }
        }
    }
    // (b) at least one vowel.
    if !newname.iter().any(|c| b"aeiouyAEIOUY".contains(c)) {
        return false;
    }
    // (c) no spaces (parse_name enforces already).
    // (d) xnames substrings.
    let lower = newname.to_ascii_lowercase();
    for bad in &g.invalid_names {
        if !bad.is_empty() && lower.windows(bad.len()).any(|w| w == &bad[..]) {
            return false;
        }
    }
    true
}

/// CAP the first letter (names are stored capitalized).
fn cap_name(name: &[u8]) -> BStr {
    let mut n = name.to_vec();
    if let Some(c) = n.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    n
}

fn write_desc(g: &mut Game, di: usize, txt: &[u8]) {
    crate::comm::write_to_desc(g, di, txt);
}

fn echo_off(g: &mut Game, di: usize) {
    g.descriptors.echo_off(di);
}

fn echo_on(g: &mut Game, di: usize) {
    g.descriptors.echo_on(di);
}

fn set_state(g: &mut Game, di: usize, state: ConState) {
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = state;
    }
}

/// CRYPT truncated compare:
/// strncmp(CRYPT(typed, stored), stored, MAX_PWD_LENGTH) == 0. With DES
/// crypt both sides are 13 bytes, so this is hash equality (F6: no plaintext
/// path exists on any platform).
fn password_matches(typed: &[u8], stored: &[u8]) -> bool {
    if stored.len() < 2 {
        return false;
    }
    match mud_data::crypt::crypt(typed, stored) {
        Some(hash) => {
            let n = MAX_PWD_LENGTH.min(stored.len()).min(hash.len());
            hash[..n] == stored[..n]
        }
        None => false,
    }
}

fn crypt_new_password(typed: &[u8], name: &[u8]) -> BStr {
    mud_data::crypt::crypt(typed, name).map(|h| h.to_vec()).unwrap_or_default()
}

/// Allocate the char shell at CON_GET_NAME.
fn ensure_char_shell(g: &mut Game, di: usize) -> CharId {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        return chid;
    }
    let mut ch = Char {
        player_specials: Some(Box::new(PlayerSpecials::default())),
        idnum: -1, // creation-in-progress marker (valid_name checks it)
        ..Default::default()
    };
    let host = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    ch.ps_mut().host = Some(host);
    ch.desc = Some(di);
    let chid = g.chars.insert(ch);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.character = Some(chid);
    }
    chid
}

/// init_char — at class selection.
pub fn init_char(g: &mut Game, chid: CharId) {
    let first_player = g.player_table.len() == 1; // entry just created
    {
        let ch = g.ch_mut(chid);
        if first_player {
            ch.level = LVL_IMPL;
            ch.points.exp = 7_000_000;
            ch.points.max_hit = 500;
            ch.points.max_mana = 100;
            ch.points.max_move = 82;
        } else {
            ch.points.max_hit = 0;
            ch.points.max_mana = 100;
            ch.points.max_move = 82;
        }
        ch.points.hit = ch.points.max_hit;
        ch.points.mana = ch.points.max_mana;
        ch.points.mov = ch.points.max_move;
        ch.points.armor = 100;
    }
    set_title(g, chid, None);
    {
        let now = g.now;
        let ch = g.ch_mut(chid);
        ch.description = None;
        ch.time.birth = now;
        ch.time.logon = now;
        ch.time.played = 0;
    }
    // Height/weight by sex. The RNG draw order is load-bearing.
    let sex = g.ch(chid).sex;
    let (weight, height) = if sex == SEX_MALE {
        let w = g.rng.rand_number(120, 180);
        let h = g.rng.rand_number(160, 200);
        (w, h)
    } else {
        let w = g.rng.rand_number(100, 160);
        let h = g.rng.rand_number(150, 180);
        (w, h)
    };
    {
        let ch = g.ch_mut(chid);
        ch.weight = weight as u8;
        ch.height = height as u8;
    }
    // Idnum assignment via player table.
    g.top_idnum += 1;
    let id = g.top_idnum;
    {
        let ch = g.ch_mut(chid);
        ch.idnum = id;
    }
    let name = g.ch(chid).name.clone().unwrap_or_default().to_ascii_lowercase();
    if let Some(row) = g.player_table.iter_mut().find(|p| p.name == name) {
        row.id = id;
    } else {
        g.log("SYSERR: init_char: Character not found! Could not set ID number.".to_string());
    }
    {
        let ch = g.ch_mut(chid);
        let level = ch.level;
        let ps = ch.ps_mut();
        for s in ps.skills.iter_mut() {
            *s = if level == LVL_IMPL { 100 } else { 0 };
        }
        ch.affected_by = mud_data::flags::FlagSet::EMPTY;
        ch.apply_saving_throw = [0; 5];
        // Placeholder 25s until do_start rolls real stats (study 02 §4.1).
        ch.real_abils.intel = 25;
        ch.real_abils.wis = 25;
        ch.real_abils.dex = 25;
        ch.real_abils.str_ = 25;
        ch.real_abils.str_add = 100;
        ch.real_abils.con = 25;
        ch.real_abils.cha = 25;
        ch.aff_abils = ch.real_abils;
        let cond = if level == LVL_IMPL { -1 } else { 24 };
        let ps = ch.ps_mut();
        ps.conditions = [cond; 3];
        ps.load_room = NOWHERE;
        ps.screen_width = 80;
    }
    // Starting toggles, through the one definition of what
    // "default" means.
    let pref = set_default_prefs(g, chid);
    g.ch_mut(chid).ps_mut().pref = pref;
}

/// set_default_prefs: the default preference toggles, in one place.
///
/// One list, shared by character creation and by prefedit's "restore
/// defaults". Two lists disagree sooner or later, and the symptom is a
/// player whose restored defaults are a set no new character has. There
/// cannot
/// be two of them.
///
/// Colour is set for anyone with a descriptor. Guarding it with
/// `pProtocol->pVariables[eMSDP_ANSI_COLORS]` reads like a check that the
/// client advertised colour, but is not one: `pVariables` is an array of
/// pointers with every slot allocated at init, so the test is true whenever
/// `ch->desc` is. Only the descriptor is a real condition, and a genuine
/// capability check here would turn colour off for dumb clients that
/// currently get it.
pub fn set_default_prefs(g: &Game, chid: CharId) -> flags::FlagSet {
    let mut pref = flags::FlagSet::EMPTY;

    pref.set(flags::PRF_AUTOEXIT);
    pref.set(flags::PRF_DISPHP);
    pref.set(flags::PRF_DISPMANA);
    pref.set(flags::PRF_DISPMOVE);

    let ch = g.ch(chid);
    if ch.desc.is_some() {
        pref.set(flags::PRF_COLOR_1);
        pref.set(flags::PRF_COLOR_2);
    }

    // The three the game grants for rank rather than for taste. `init_char`
    // never needed these — a new character is level 0 — but prefedit restores
    // an existing one, who may not be.
    if ch.level > LVL_IMMORT {
        pref.set(flags::PRF_NOHASSLE);
        pref.set(flags::PRF_HOLYLIGHT);
        pref.set(flags::PRF_SHOWVNUMS);
    }

    pref
}

/// set_title: NULL → default class/level/sex title.
pub fn set_title(g: &mut Game, chid: CharId, title: Option<BStr>) {
    let title = match title {
        Some(t) => t,
        None => {
            let ch = g.ch(chid);
            let class = ch.class.clamp(0, 3) as usize;
            let level = (ch.level as usize).min(34);
            let t = if ch.sex == SEX_FEMALE {
                tables::TITLES_FEMALE[class][level]
            } else {
                tables::TITLES_MALE[class][level]
            };
            t.as_bytes().to_vec()
        }
    };
    let mut t = title;
    t.truncate(MAX_TITLE_LENGTH);
    g.ch_mut(chid).title = Some(t);
}

/// roll_real_abils: six 4d6-drop-lowest rolls, sorted
/// descending, assigned by class priority. Consumes exactly 24 RNG draws.
pub fn roll_real_abils(g: &mut Game, chid: CharId) {
    let mut table = [0i32; 6];
    let mut rolls = [0i32; 4];
    for i in 0..6 {
        for roll in rolls.iter_mut() {
            *roll = g.rng.rand_number(1, 6);
        }
        let sum: i32 = rolls.iter().sum();
        let min = *rolls.iter().min().unwrap();
        let val = sum - min;
        // Insertion into a descending table: walk, then shift.
        let mut placed = false;
        for j in 0..6 {
            if !placed && val > table[j] {
                table[j..=i.min(5)].rotate_right(1);
                table[j] = val;
                placed = true;
                break;
            }
        }
        if !placed {
            table[i] = val;
        }
    }
    let class = g.ch(chid).class;
    let ch = g.ch_mut(chid);
    ch.real_abils.str_add = 0;
    match class {
        CLASS_MAGIC_USER => {
            ch.real_abils.intel = table[0] as i8;
            ch.real_abils.wis = table[1] as i8;
            ch.real_abils.dex = table[2] as i8;
            ch.real_abils.str_ = table[3] as i8;
            ch.real_abils.con = table[4] as i8;
            ch.real_abils.cha = table[5] as i8;
        }
        CLASS_CLERIC => {
            ch.real_abils.wis = table[0] as i8;
            ch.real_abils.intel = table[1] as i8;
            ch.real_abils.str_ = table[2] as i8;
            ch.real_abils.dex = table[3] as i8;
            ch.real_abils.con = table[4] as i8;
            ch.real_abils.cha = table[5] as i8;
        }
        CLASS_THIEF => {
            ch.real_abils.dex = table[0] as i8;
            ch.real_abils.str_ = table[1] as i8;
            ch.real_abils.con = table[2] as i8;
            ch.real_abils.intel = table[3] as i8;
            ch.real_abils.wis = table[4] as i8;
            ch.real_abils.cha = table[5] as i8;
        }
        CLASS_WARRIOR => {
            ch.real_abils.str_ = table[0] as i8;
            ch.real_abils.dex = table[1] as i8;
            ch.real_abils.con = table[2] as i8;
            ch.real_abils.wis = table[3] as i8;
            ch.real_abils.intel = table[4] as i8;
            ch.real_abils.cha = table[5] as i8;
        }
        _ => {}
    }
    if class == CLASS_WARRIOR && g.ch(chid).real_abils.str_ == 18 {
        let add = g.rng.rand_number(0, 100);
        g.ch_mut(chid).real_abils.str_add = add as i8;
    }
    let ch = g.ch_mut(chid);
    ch.aff_abils = ch.real_abils;
}

pub fn advance_level(g: &mut Game, chid: CharId) {
    let (class, level, con, wis) = {
        let ch = g.ch(chid);
        (ch.class, ch.level as i32, ch.aff_abils.con.clamp(0, 25) as usize, ch.aff_abils.wis.clamp(0, 25) as usize)
    };
    let con_hp = tables::CON_APP[con];
    let mut add_hp = con_hp;
    let mut add_mana = 0;
    let add_move;
    match class {
        CLASS_MAGIC_USER => {
            add_hp += g.rng.rand_number(3, 8);
            add_mana = g.rng.rand_number(level, (3 * level) / 2).min(10);
            add_move = g.rng.rand_number(0, 2);
        }
        CLASS_CLERIC => {
            add_hp += g.rng.rand_number(5, 10);
            add_mana = g.rng.rand_number(level, (3 * level) / 2).min(10);
            add_move = g.rng.rand_number(0, 2);
        }
        CLASS_THIEF => {
            add_hp += g.rng.rand_number(7, 13);
            add_move = g.rng.rand_number(1, 3);
        }
        CLASS_WARRIOR => {
            add_hp += g.rng.rand_number(10, 15);
            add_move = g.rng.rand_number(1, 3);
        }
        _ => {
            add_move = 0;
        }
    }
    {
        let ch = g.ch_mut(chid);
        ch.points.max_hit += add_hp.max(1);
        ch.points.max_move += add_move.max(1);
        if level > 1 {
            ch.points.max_mana += add_mana;
        }
    }
    let gain = crate::limits::practices_per_level(class as i32, wis as i32);
    g.ch_mut(chid).ps_mut().practices += gain;
    if level >= LVL_IMMORT as i32 {
        let ch = g.ch_mut(chid);
        ch.ps_mut().conditions = [-1; 3];
        ch.ps_mut().pref.set(flags::PRF_HOLYLIGHT);
    }
    crate::players_glue::save_char(g, chid);
}

/// do_start: level 0 → 1 at first entry.
pub fn do_start(g: &mut Game, chid: CharId) {
    {
        let ch = g.ch_mut(chid);
        ch.level = 1;
        ch.points.exp = 1;
    }
    set_title(g, chid, None);
    roll_real_abils(g, chid);
    {
        let ch = g.ch_mut(chid);
        ch.points.max_hit = 10;
        ch.points.max_mana = 100;
        ch.points.max_move = 82;
    }
    let class = g.ch(chid).class;
    if class == CLASS_THIEF {
        // Thief starting skills.
        use mud_data::spells::*;
        let ps = g.ch_mut(chid).ps_mut();
        ps.skills[SKILL_SNEAK as usize] = 10;
        ps.skills[SKILL_HIDE as usize] = 5;
        ps.skills[SKILL_STEAL as usize] = 15;
        ps.skills[SKILL_BACKSTAB as usize] = 10;
        ps.skills[SKILL_PICK_LOCK as usize] = 10;
        ps.skills[SKILL_TRACK as usize] = 10;
    }
    advance_level(g, chid);
    {
        let ch = g.ch_mut(chid);
        ch.points.hit = ch.points.max_hit;
        ch.points.mana = ch.points.max_mana;
        ch.points.mov = ch.points.max_move;
        let ps = ch.ps_mut();
        ps.conditions[THIRST] = 24;
        ps.conditions[HUNGER] = 24;
        ps.conditions[DRUNK] = 0;
    }
    if g.config.siteok_everyone {
        g.ch_mut(chid).act.set(flags::PLR_SITEOK);
    }
}

pub fn enter_player_game(g: &mut Game, di: usize) -> i32 {
    let chid = g.descriptors.get(di).and_then(|d| d.character).expect("enter without char");
    // reset_char.
    {
        let ch = g.ch_mut(chid);
        ch.equipment = [None; NUM_WEARS];
        ch.carrying.clear();
        ch.followers.clear();
        ch.master = None;
        ch.in_room = NOWHERE;
        ch.was_in_room = NOWHERE;
        ch.fighting = None;
        ch.position = POS_STANDING;
        ch.carry_weight = 0;
        ch.carry_items = 0;
        if ch.points.hit <= 0 {
            ch.points.hit = 1;
        }
        if ch.points.mov <= 0 {
            ch.points.mov = 1;
        }
        if ch.points.mana <= 0 {
            ch.points.mana = 1;
        }
        ch.ps_mut().last_tell = crate::ch::NOBODY_TELL;
    }
    if g.ch(chid).plr(flags::PLR_INVSTART) {
        let lvl = g.ch(chid).level;
        g.ch_mut(chid).ps_mut().invis_level = lvl as i16;
    }

    // Load room resolution.
    let load_room_vnum = g.ch(chid).ps().load_room;
    let mut load_room = if load_room_vnum != NOWHERE {
        g.real_room(load_room_vnum as i32)
    } else {
        None
    };
    if load_room.is_none() {
        // The cached rnums, with check_start_rooms' boot-time fallback
        // chain already applied.
        load_room = Some(if g.ch(chid).level >= LVL_IMMORT {
            g.r_immort_start_room
        } else {
            g.r_mortal_start_room
        });
    }
    if g.ch(chid).plr(flags::PLR_FROZEN) {
        load_room = Some(g.r_frozen_start_room);
    }
    let room = load_room.unwrap_or(0);

    g.ch_mut(chid).script_id = g.ch(chid).idnum;
    let sid = g.ch(chid).script_id;
    crate::dg::add_to_lookup_table(g, sid, crate::dg::UidEntry::Char(chid));
    // After moving variable saving to the pfile, this runs only when the
    // pfile had no Vars (SCRIPT(ch) unset) -.
    if g.ch(chid).script.is_none() {
        crate::players_glue::read_saved_vars(g, chid);
    }
    g.character_list.push_front(chid);
    char_to_room(g, chid, room);
    let load_result = crate::objsave::crash_load(g, chid);
    // Save the character and their object file.
    crate::players_glue::save_char(g, chid);
    crate::objsave::crash_crashsave(g, chid);
    // Check for a login trigger in the players' start room.
    let in_room = g.ch(chid).in_room;
    crate::dg::triggers::login_wtrigger(g, in_room, chid);
    load_result
}

/// The connect-time banner + greeting (the get_protocols event).
pub fn get_protocols_event(g: &mut Game, di: usize) {
    let Some(d) = g.descriptors.get(di) else { return };
    let p = &d.protocol;
    let client = String::from_utf8_lossy(p.var_str(mud_net::protocol::Var::CLIENT_ID)).into_owned();
    let color: &[u8] = if p.var_int(mud_net::protocol::Var::XTERM_256_COLORS) != 0 {
        b"\tO[\toColors\tO] \tw256\tn | "
    } else if p.var_int(mud_net::protocol::Var::ANSI_COLORS) != 0 {
        b"\tO[\toColors\tO] \twAnsi\tn | "
    } else {
        b"[Colors] No Color | "
    };
    let mxp = p.mxp;
    let msdp = p.msdp;
    let atcp = p.atcp;
    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"\x1B[H\x1B[J");
    out.extend_from_slice(b"\tO[\toClient\tO] \tw");
    out.extend_from_slice(client.as_bytes());
    out.extend_from_slice(b"\tn | ");
    out.extend_from_slice(color);
    out.extend_from_slice(b"\tO[\toMXP\tO] \tw");
    out.extend_from_slice(if mxp { b"Yes" as &[u8] } else { b"No" });
    out.extend_from_slice(b"\tn | \tO[\toMSDP\tO] \tw");
    out.extend_from_slice(if msdp { b"Yes" as &[u8] } else { b"No" });
    out.extend_from_slice(b"\tn | \tO[\toATCP\tO] \tw");
    out.extend_from_slice(if atcp { b"Yes" as &[u8] } else { b"No" });
    out.extend_from_slice(b"\tn\r\n\r\n");
    let greet = g.texts.greetings.clone();
    out.extend_from_slice(&greet);
    write_desc(g, di, &out);
    set_state(g, di, ConState::GetName);
}

/// perform_dupe_check. Returns true when the
/// caller must skip motd/menu (reconnect/usurp complete).
/// MXPSendTag through a descriptor index: read `output_empty`, write, pump.
///
/// Protocol bytes are staged in `protocol.out` rather than written to the
/// descriptor's output buffer directly, so the pump has to happen here --
/// otherwise they wait for a caller that may never come.
fn mxp_send_tag_desc(g: &mut Game, di: usize, tag: &[u8]) {
    let Some(d) = g.descriptors.get_mut(di) else { return };
    let empty = d.output.is_empty();
    mud_net::protocol::mxp_send_tag(&mut d.protocol, tag, empty);
    g.descriptors.pump_protocol_out(di);
}

fn perform_dupe_check(g: &mut Game, di: usize) -> bool {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let id = g.ch(chid).idnum;

    let mut target: Option<CharId> = None;
    #[derive(PartialEq)]
    enum Mode {
        Recon,
        Usurp,
        Unswitch,
    }
    let mut mode = Mode::Recon;

    // Phase 1: other descriptors with the same character.
    for odi in g.descriptors.indices() {
        if odi == di {
            continue;
        }
        let Some(od) = g.descriptors.get(odi) else { continue };
        let orig_match = od.original.and_then(|c| g.try_ch(c)).is_some_and(|c| c.idnum == id);
        let char_match = od.character.and_then(|c| g.try_ch(c)).is_some_and(|c| c.idnum == id);
        if orig_match {
            let orig = g.descriptors.get(odi).unwrap().original;
            write_desc(g, odi, b"\r\nMultiple login detected -- disconnecting.\r\n");
            set_state(g, odi, ConState::Close);
            if target.is_none() {
                target = orig;
                mode = Mode::Unswitch;
            }
        } else if char_match {
            let od = g.descriptors.get(odi).unwrap();
            if od.original.is_some() {
                // Someone (an imm) switched INTO this body: do_return is
                // stage 8; sever the link.
                let odc = od.character;
                if let Some(odc) = odc {
                    let _ = odc;
                }
            } else {
                if od.state == ConState::Playing && target.is_none() {
                    write_desc(g, odi, b"\r\nThis body has been usurped!\r\n");
                    target = g.descriptors.get(odi).unwrap().character;
                    mode = Mode::Usurp;
                }
                write_desc(g, odi, b"\r\nMultiple login detected -- disconnecting.\r\n");
                set_state(g, odi, ConState::Close);
                if let Some(od) = g.descriptors.get_mut(odi) {
                    if let Some(oc) = od.character {
                        if let Some(occ) = g.chars.get_mut(oc) {
                            occ.desc = None;
                        }
                    }
                    od.character = None;
                }
            }
        }
    }

    // Phase 2: desc-less bodies in the world.
    let list = g.character_list.clone();
    for other in list {
        if Some(other) == g.descriptors.get(di).and_then(|d| d.character) {
            continue;
        }
        let Some(oc) = g.try_ch(other) else { continue };
        if oc.desc.is_some() || oc.idnum != id || oc.is_npc() {
            continue;
        }
        if target.is_none() {
            target = Some(other);
            mode = Mode::Recon;
        } else if target != Some(other) {
            // Duplicate bodies: dump to the void (rnum 1) and extract.
            crate::handler::char_from_room(g, other);
            char_to_room(g, other, 1);
            crate::handler::extract_char(g, other);
        }
    }

    let Some(target) = target else {
        // Fresh PREF id (rand 1..128000) — RNG call point preserved.
        let pref = g.rng.rand_number(1, 128000);
        g.ch_mut(chid).punique = pref;
        return false;
    };

    // Transfer the descriptor onto the existing body.
    let fresh = g.descriptors.get(di).and_then(|d| d.character);
    if let Some(fresh) = fresh {
        if fresh != target {
            crate::handler::free_char(g, fresh);
        }
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.character = Some(target);
        d.state = ConState::Playing;
        mxp_send_tag_desc(g, di, b"<VERSION>");
    }
    {
        let t = g.ch_mut(target);
        t.desc = Some(di);
        t.timer = 0;
        t.act.remove(flags::PLR_MAILING);
        t.act.remove(flags::PLR_WRITING);
    }
    crate::llog::add_llog_entry(g, target, crate::llog::LAST_RECONNECT);
    let name = String::from_utf8_lossy(g.ch(target).get_name()).into_owned();
    let host = g.descriptors.get(di).map(|d| String::from_utf8_lossy(&d.host).into_owned()).unwrap_or_default();
    match mode {
        Mode::Recon => {
            write_desc(g, di, b"Reconnecting.\r\n");
            act(g, b"$n has reconnected.", true, Some(target), None, None, comm::TO_ROOM);
            let invis = g.ch(target).invis_lev();
            g.mudlog(
                MudlogKind::Nrm,
                (LVL_IMMORT as i16).max(invis) as u8,
                true,
                &format!("{} [{}] has reconnected.", name, host),
            );
            // Mail notice: stage 7.
        }
        Mode::Usurp => {
            write_desc(g, di, b"You take over your own body, already in use!\r\n");
            act(
                g,
                b"$n suddenly keels over in pain, surrounded by a white aura...\r\n$n's body has been taken over by a new spirit!",
                true,
                Some(target),
                None,
                None,
                comm::TO_ROOM,
            );
            let invis = g.ch(target).invis_lev();
            g.mudlog(
                MudlogKind::Nrm,
                (LVL_IMMORT as i16).max(invis) as u8,
                true,
                &format!("{} has re-logged in ... disconnecting old socket.", name),
            );
        }
        Mode::Unswitch => {
            write_desc(g, di, b"Reconnecting to unswitched char.");
            let invis = g.ch(target).invis_lev();
            g.mudlog(
                MudlogKind::Nrm,
                (LVL_IMMORT as i16).max(invis) as u8,
                true,
                &format!("{} [{}] has reconnected.", name, host),
            );
        }
    }
    true
}

fn perform_new_char_dupe_check(g: &mut Game, di: usize) {
    let name = g
        .descriptors
        .get(di)
        .and_then(|d| d.character)
        .and_then(|c| g.try_ch(c))
        .and_then(|c| c.name.clone());
    let Some(name) = name else { return };
    for odi in g.descriptors.indices() {
        if odi == di {
            continue;
        }
        let Some(od) = g.descriptors.get(odi) else { continue };
        let Some(oc) = od.character.and_then(|c| g.try_ch(c)) else { continue };
        let same = oc.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&name));
        if !same {
            continue;
        }
        let state = od.state as u8;
        let in_creation = state > ConState::Playing as u8 && state < ConState::Qclass as u8;
        if in_creation {
            write_desc(g, odi, b"\r\nMultiple login detected -- disconnecting.\r\n");
            set_state(g, odi, ConState::Close);
            g.mudlog(
                MudlogKind::Cmp,
                LVL_GOD,
                true,
                &format!("Multiple logins detected in char creation for {}.", String::from_utf8_lossy(&name)),
            );
        } else {
            // Inconsistent: boot both.
            set_state(g, odi, ConState::Close);
            write_desc(g, di, b"\r\nSorry, due to multiple connections, all your connections are being closed.\r\n");
            write_desc(g, di, b"\r\nPlease reconnect.\r\n");
            set_state(g, di, ConState::Close);
            g.mudlog(
                MudlogKind::Cmp,
                LVL_GOD,
                true,
                "SYSERR: Multiple logins with 1st in-game and the 2nd in char creation.",
            );
        }
    }
}

/// The class menu.
const CLASS_MENU: &[u8] = b"\r\nSelect a class:\r\n  [\t(C\t)]leric\r\n  [\t(T\t)]hief\r\n  [\t(W\t)]arrior\r\n  [\t(M\t)]agic-user\r\n";

fn parse_class(c: u8) -> i8 {
    match c.to_ascii_lowercase() {
        b'm' => CLASS_MAGIC_USER,
        b'c' => CLASS_CLERIC,
        b'w' => CLASS_WARRIOR,
        b't' => CLASS_THIEF,
        _ => CLASS_UNDEFINED,
    }
}

pub fn nanny(g: &mut Game, di: usize, arg: &[u8]) {
    let arg = skip_spaces(arg).to_vec();
    let state = match g.descriptors.get(di) {
        Some(d) => d.state,
        None => return,
    };
    // Quick check for the OLC states.
    if crate::olc::olc_parse(g, di, &arg) {
        return;
    }
    match state {
        ConState::GetProtocol => {
            write_desc(g, di, b"Collecting Protocol Information... Please Wait.\r\n");
        }
        ConState::GetName => con_get_name(g, di, &arg),
        ConState::NameCnfrm => con_name_cnfrm(g, di, &arg),
        ConState::Password => con_password(g, di, &arg),
        ConState::Newpasswd | ConState::ChpwdGetnew => con_newpasswd(g, di, &arg, state),
        ConState::Cnfpasswd | ConState::ChpwdVrfy => con_cnfpasswd(g, di, &arg, state),
        ConState::Qsex => con_qsex(g, di, &arg),
        ConState::Qclass => con_qclass(g, di, &arg),
        ConState::Rmotd => con_rmotd(g, di),
        ConState::Menu => con_menu(g, di, &arg),
        ConState::ChpwdGetold => con_chpwd_getold(g, di, &arg),
        ConState::Delcnf1 => con_delcnf1(g, di, &arg),
        ConState::Delcnf2 => con_delcnf2(g, di, &arg),
        ConState::Close | ConState::Disconnect => {}
        _ => {
            let num = state as u8;
            let name = g
                .descriptors
                .get(di)
                .and_then(|d| d.character)
                .and_then(|c| g.try_ch(c))
                .map(|c| String::from_utf8_lossy(c.get_name()).into_owned())
                .unwrap_or_else(|| "<unknown>".to_string());
            g.log(format!(
                "SYSERR: Nanny: illegal state of con'ness ({}) for '{}'; closing connection.",
                num, name
            ));
            set_state(g, di, ConState::Disconnect);
        }
    }
}

fn con_get_name(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = ensure_char_shell(g, di);
    if arg.is_empty() {
        set_state(g, di, ConState::Close);
        return;
    }
    let reject = |g: &mut Game, di: usize| {
        write_desc(g, di, b"Invalid name, please try another.\r\nName: ");
    };
    let Some(tmp_name) = parse_name(arg) else {
        reject(g, di);
        return;
    };
    if tmp_name.len() < 2 || tmp_name.len() > MAX_NAME_LENGTH || !valid_name(g, &tmp_name) || fill_or_reserved(&tmp_name)
    {
        reject(g, di);
        return;
    }

    // Try loading an existing player.
    match crate::players_glue::load_char_into(g, chid, &tmp_name).map(|player_i| {
        g.ch_mut(chid).pfilepos = player_i as i32;
    }) {
        Some(()) => {
            let deleted = g.ch(chid).plr(flags::PLR_DELETED);
            if deleted {
                // Wipe and recreate path.
                crate::players_glue::remove_player_by_name(g, &tmp_name);
                let host = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
                {
                    // Fresh shell.
                    let ch = g.ch_mut(chid);
                    *ch = Char {
                        player_specials: Some(Box::new(PlayerSpecials::default())),
                        idnum: -1,
                        desc: Some(di),
                        ..Default::default()
                    };
                    ch.ps_mut().host = Some(host);
                }
                if !valid_name(g, &tmp_name) {
                    reject(g, di);
                    return;
                }
                let capped = cap_name(&tmp_name);
                g.ch_mut(chid).name = Some(capped.clone());
                let mut msg = b"Did I get that right, ".to_vec();
                msg.extend_from_slice(&capped);
                msg.extend_from_slice(b" (\t(Y\t)/\t(N\t))? ");
                write_desc(g, di, &msg);
                set_state(g, di, ConState::NameCnfrm);
            } else {
                {
                    let now = g.now;
                    let ch = g.ch_mut(chid);
                    ch.act.remove(flags::PLR_WRITING);
                    ch.act.remove(flags::PLR_MAILING);
                    ch.act.remove(flags::PLR_CRYO);
                    ch.time.logon = now;
                }
                write_desc(g, di, b"Password: ");
                echo_off(g, di);
                if let Some(d) = g.descriptors.get_mut(di) {
                    d.idle_tics = 0;
                }
                set_state(g, di, ConState::Password);
            }
        }
        None => {
            // New character.
            if !valid_name(g, &tmp_name) {
                reject(g, di);
                return;
            }
            let capped = cap_name(&tmp_name);
            g.ch_mut(chid).name = Some(capped.clone());
            let mut msg = b"Did I get that right, ".to_vec();
            msg.extend_from_slice(&capped);
            msg.extend_from_slice(b" (\t(Y\t)/\t(N\t))? ");
            write_desc(g, di, &msg);
            set_state(g, di, ConState::NameCnfrm);
        }
    }
}

fn con_name_cnfrm(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    match arg.first().map(|c| c.to_ascii_uppercase()) {
        Some(b'Y') => {
            let host = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
            let host_s = String::from_utf8_lossy(&host).into_owned();
            let name =
                String::from_utf8_lossy(g.ch(chid).name.as_deref().unwrap_or(b"")).into_owned();
            if crate::ban::isbanned(g, &host) >= crate::ban::BAN_NEW {
                g.mudlog(
                    MudlogKind::Nrm,
                    LVL_GOD,
                    true,
                    &format!("Request for new char {} denied from [{}] (siteban)", name, host_s),
                );
                write_desc(
                    g,
                    di,
                    b"Sorry, new characters are not allowed from your site!\r\n",
                );
                set_state(g, di, ConState::Close);
                return;
            }
            if g.circle_restrict != 0 {
                write_desc(g, di, b"Sorry, new players can't be created at the moment.\r\n");
                g.mudlog(
                    MudlogKind::Nrm,
                    LVL_GOD,
                    true,
                    &format!("Request for new char {} denied from [{}] (wizlock)", name, host_s),
                );
                set_state(g, di, ConState::Close);
                return;
            }
            perform_new_char_dupe_check(g, di);
            if g.descriptors.get(di).map(|d| d.state) == Some(ConState::Close) {
                return;
            }
            let mut msg = b"New character.\r\nGive me a password for ".to_vec();
            msg.extend_from_slice(name.as_bytes());
            msg.extend_from_slice(b": ");
            write_desc(g, di, &msg);
            echo_off(g, di);
            set_state(g, di, ConState::Newpasswd);
        }
        Some(b'N') => {
            write_desc(g, di, b"Okay, what IS it, then? ");
            g.ch_mut(chid).name = None;
            set_state(g, di, ConState::GetName);
        }
        _ => {
            write_desc(g, di, b"Please type Yes or No: ");
        }
    }
}

fn con_password(g: &mut Game, di: usize, arg: &[u8]) {
    echo_on(g, di);
    write_desc(g, di, b"\r\n");
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    if arg.is_empty() {
        set_state(g, di, ConState::Close);
        return;
    }
    let stored = g.ch(chid).passwd.clone();
    if !password_matches(arg, &stored) {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let host = g.descriptors.get(di).map(|d| String::from_utf8_lossy(&d.host).into_owned()).unwrap_or_default();
        g.mudlog(MudlogKind::Brf, LVL_GOD, true, &format!("Bad PW: {} [{}]", name, host));
        {
            let ps = g.ch_mut(chid).ps_mut();
            ps.bad_pws += 1;
        }
        crate::players_glue::save_char(g, chid);
        let bad = {
            let d = g.descriptors.get_mut(di).unwrap();
            d.bad_pws += 1;
            d.bad_pws
        };
        if bad as i32 >= g.config.max_bad_pws {
            write_desc(g, di, b"Wrong password... disconnecting.\r\n");
            set_state(g, di, ConState::Close);
        } else {
            write_desc(g, di, b"Wrong password.\r\nPassword: ");
            echo_off(g, di);
        }
        return;
    }

    // Correct password.
    let load_result = {
        let ps = g.ch_mut(chid).ps_mut();
        let n = ps.bad_pws;
        ps.bad_pws = 0;
        n
    };
    if let Some(d) = g.descriptors.get_mut(di) {
        d.bad_pws = 0;
    }
    let host = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    if crate::ban::isbanned(g, &host) == crate::ban::BAN_SELECT
        && !g.ch(chid).plr(mud_data::flags::PLR_SITEOK)
    {
        write_desc(
            g,
            di,
            b"Sorry, this char has not been cleared for login from your site!\r\n",
        );
        set_state(g, di, ConState::Close);
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let host_s = String::from_utf8_lossy(&host).into_owned();
        g.mudlog(
            MudlogKind::Nrm,
            LVL_GOD,
            true,
            &format!("Connection attempt for {} denied from {}", name, host_s),
        );
        return;
    }
    let level = g.ch(chid).level;
    if level < g.circle_restrict {
        write_desc(g, di, b"The game is temporarily restricted.. try again later.\r\n");
        set_state(g, di, ConState::Close);
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let host = g.descriptors.get(di).map(|d| String::from_utf8_lossy(&d.host).into_owned()).unwrap_or_default();
        g.mudlog(
            MudlogKind::Nrm,
            LVL_GOD,
            true,
            &format!("Request for login denied for {} [{}] (wizlock)", name, host),
        );
        return;
    }
    if perform_dupe_check(g, di) {
        return;
    }
    let motd = if level >= LVL_IMMORT { g.texts.imotd.clone() } else { g.texts.motd.clone() };
    write_desc(g, di, &motd);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let host = g.descriptors.get(di).map(|d| String::from_utf8_lossy(&d.host).into_owned()).unwrap_or_default();
    let invis = g.ch(chid).invis_lev();
    if invis != 0 {
        g.mudlog(
            MudlogKind::Brf,
            (LVL_IMMORT as i16).max(invis) as u8,
            true,
            &format!("{} [{}] has connected. (invis {})", name, host, invis),
        );
    } else {
        g.mudlog(MudlogKind::Brf, LVL_IMMORT, true, &format!("{} [{}] has connected.", name, host));
    }

    // Add to the list of 'recent' players (since last reboot).
    let hostb = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    let nameb = g.ch(chid).get_name().to_vec();
    if !crate::llog::add_recent_player(g, &nameb, &hostb, false, false) {
        let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(MudlogKind::Brf, lvl, true, "Failure to AddRecentPlayer (returned FALSE).");
    }

    if load_result != 0 {
        let red = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KRED).to_vec();
        let nrm = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM).to_vec();
        let mut msg = b"\r\n\r\n\x07\x07\x07".to_vec();
        msg.extend_from_slice(&red);
        msg.extend_from_slice(
            format!("{} LOGIN FAILURE{} SINCE LAST SUCCESSFUL LOGIN.", load_result, if load_result > 1 { "S" } else { "" })
                .as_bytes(),
        );
        msg.extend_from_slice(&nrm);
        msg.extend_from_slice(b"\r\n");
        write_desc(g, di, &msg);
        g.ch_mut(chid).ps_mut().bad_pws = 0;
    }
    write_desc(g, di, b"\r\n*** PRESS RETURN: ");
    set_state(g, di, ConState::Rmotd);
}

fn con_newpasswd(g: &mut Game, di: usize, arg: &[u8], state: ConState) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let name = g.ch(chid).name.clone().unwrap_or_default();
    if arg.is_empty() || arg.len() > MAX_PWD_LENGTH || arg.eq_ignore_ascii_case(&name) {
        write_desc(g, di, b"\r\nIllegal password.\r\nPassword: ");
        return;
    }
    if arg.len() < 3 {
        write_desc(g, di, b"\r\nIllegal password.\r\nPassword: ");
        return;
    }
    let hash = crypt_new_password(arg, &name);
    g.ch_mut(chid).passwd = hash;
    write_desc(g, di, b"\r\nPlease retype password: ");
    set_state(
        g,
        di,
        if state == ConState::Newpasswd { ConState::Cnfpasswd } else { ConState::ChpwdVrfy },
    );
}

fn con_cnfpasswd(g: &mut Game, di: usize, arg: &[u8], state: ConState) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let stored = g.ch(chid).passwd.clone();
    if !password_matches(arg, &stored) {
        write_desc(g, di, b"\r\nPasswords don't match... start over.\r\nPassword: ");
        set_state(
            g,
            di,
            if state == ConState::Cnfpasswd { ConState::Newpasswd } else { ConState::ChpwdGetnew },
        );
        return;
    }
    echo_on(g, di);
    if state == ConState::Cnfpasswd {
        write_desc(g, di, b"\r\nWhat is your sex (\t(M\t)/\t(F\t))? ");
        set_state(g, di, ConState::Qsex);
    } else {
        crate::players_glue::save_char(g, chid);
        let mut msg = b"\r\nDone.\r\n".to_vec();
        msg.extend_from_slice(&g.config.menu.clone());
        write_desc(g, di, &msg);
        set_state(g, di, ConState::Menu);
    }
}

fn con_qsex(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let sex = match arg.first().map(|c| c.to_ascii_lowercase()) {
        Some(b'm') => SEX_MALE,
        Some(b'f') => SEX_FEMALE,
        _ => {
            write_desc(g, di, b"That is not a sex..\r\nWhat IS your sex? ");
            return;
        }
    };
    g.ch_mut(chid).sex = sex;
    let mut msg = CLASS_MENU.to_vec();
    msg.extend_from_slice(b"\r\nClass: ");
    write_desc(g, di, &msg);
    set_state(g, di, ConState::Qclass);
}

fn con_qclass(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let class = parse_class(arg.first().copied().unwrap_or(0));
    if class == CLASS_UNDEFINED {
        write_desc(g, di, b"\r\nThat's not a class.\r\nClass: ");
        return;
    }
    g.ch_mut(chid).class = class;

    // create_entry + init_char + saves.
    let name = g.ch(chid).name.clone().unwrap_or_default();
    let player_i = crate::players_glue::create_entry(g, &name);
    g.ch_mut(chid).pfilepos = player_i as i32;
    init_char(g, chid);
    crate::players_glue::save_char(g, chid);
    crate::players_glue::save_player_index(g);

    let motd = g.texts.motd.clone();
    let mut msg = motd;
    msg.extend_from_slice(b"\r\n*** PRESS RETURN: ");
    write_desc(g, di, &msg);
    set_state(g, di, ConState::Rmotd);

    // GET_PREF roll (RNG call preserved) + host + logs.
    let pref = g.rng.rand_number(1, 128000);
    g.ch_mut(chid).punique = pref;
    let host = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    let host_s = String::from_utf8_lossy(&host).into_owned();
    g.ch_mut(chid).ps_mut().host = Some(host);
    let name_s = String::from_utf8_lossy(&name).into_owned();
    g.mudlog(MudlogKind::Nrm, LVL_GOD, true, &format!("{} [{}] new player.", name_s, host_s));

    // Add to the list of 'recent' players (since last reboot).
    let hostb = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    let nameb = g.ch(chid).get_name().to_vec();
    if !crate::llog::add_recent_player(g, &nameb, &hostb, true, false) {
        let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(MudlogKind::Brf, lvl, true, "Failure to AddRecentPlayer (returned FALSE).");
    }
}

fn con_rmotd(g: &mut Game, di: usize) {
    let menu = g.config.menu.clone();
    write_desc(g, di, &menu);
    if crate::act::other::is_happyhour(g) {
        write_desc(g, di, b"\r\n");
        write_desc(g, di, b"\tyThere is currently a Happyhour!\tn\r\n");
        write_desc(g, di, b"\r\n");
    }
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        crate::llog::add_llog_entry(g, chid, crate::llog::LAST_CONNECT);
    }
    set_state(g, di, ConState::Menu);
}

fn con_menu(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    match arg.first().copied() {
        Some(b'0') => {
            write_desc(g, di, b"Goodbye.\r\n");
            crate::llog::add_llog_entry(g, chid, crate::llog::LAST_QUIT);
            set_state(g, di, ConState::Close);
        }
        Some(b'1') => {
            enter_game(g, di);
        }
        Some(b'2') => {
            // Description editor.
            let desc = g.ch(chid).description.clone();
            if let Some(desc) = &desc {
                let mut msg = b"Current description:\r\n".to_vec();
                msg.extend_from_slice(desc);
                write_desc(g, di, &msg);
            }
            write_desc(
                g,
                di,
                b"Enter the new text you'd like others to see when they look at you.\r\n",
            );
            write_desc(g, di, b"Instructions: /s to save, /h for more options.\r\n");
            if let Some(d) = g.descriptors.get_mut(di) {
                d.editing = Some(mud_net::descriptor::EditSession {
                    buf: mud_net::editor::EditBuf { buf: desc.clone(), max_str: PLR_DESC_LENGTH },
                    backstr: desc,
                    mail_to: 0,
                    str_slot: -1,
                    note_obj: None,
                });
                d.state = ConState::PlrDesc;
            }
        }
        Some(b'3') => {
            let background = g.texts.background.clone();
            crate::act::informative::page_string_desc(g, di, &background);
            set_state(g, di, ConState::Rmotd);
        }
        Some(b'4') => {
            write_desc(g, di, b"\r\nEnter your old password: ");
            echo_off(g, di);
            set_state(g, di, ConState::ChpwdGetold);
        }
        Some(b'5') => {
            write_desc(g, di, b"\r\nEnter your password for verification: ");
            echo_off(g, di);
            set_state(g, di, ConState::Delcnf1);
        }
        _ => {
            let mut msg = b"\r\nThat's not a menu choice!\r\n".to_vec();
            msg.extend_from_slice(&g.config.menu.clone());
            write_desc(g, di, &msg);
        }
    }
}

/// Menu option 1 — the enter-game sequence.
fn enter_game(g: &mut Game, di: usize) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let load_result = enter_player_game(g, di);
    let welc = g.config.welc_messg.clone();
    send_to_char(g, chid, &welc);
    if !g.ch(chid).plr(flags::PLR_LOADROOM) {
        g.ch_mut(chid).ps_mut().load_room = NOWHERE;
    }
    crate::players_glue::save_char(g, chid);
    crate::dg::triggers::greet_mtrigger(g, chid, -1);
    crate::dg::triggers::greet_memory_mtrigger(g, chid);
    act(g, b"$n has entered the game.", true, Some(chid), None, None, comm::TO_ROOM);
    set_state(g, di, ConState::Playing);
    mxp_send_tag_desc(g, di, b"<VERSION>");
    if g.ch(chid).level == 0 {
        do_start(g, chid);
        let start = g.config.start_messg.clone();
        send_to_char(g, chid, &start);
    }
    crate::act::informative::look_at_room(g, chid, false);
    // Mail notice: stage 7.
    if load_result == 2 {
        send_to_char(
            g,
            chid,
            b"\r\n\x07You could not afford your rent!\r\nYour possesions have been donated to the Salvation Army!\r\n",
        );
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.has_prompt = false;
    }
    g.ch_mut(chid).ps_mut().pref.remove(flags::PRF_BUILDWALK);
}

fn con_chpwd_getold(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    let stored = g.ch(chid).passwd.clone();
    if !password_matches(arg, &stored) {
        echo_on(g, di);
        let mut msg = b"\r\nIncorrect password.\r\n".to_vec();
        msg.extend_from_slice(&g.config.menu.clone());
        write_desc(g, di, &msg);
        set_state(g, di, ConState::Menu);
    } else {
        write_desc(g, di, b"\r\nEnter a new password: ");
        set_state(g, di, ConState::ChpwdGetnew);
    }
}

fn con_delcnf1(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    echo_on(g, di);
    let stored = g.ch(chid).passwd.clone();
    if !password_matches(arg, &stored) {
        let mut msg = b"\r\nIncorrect password.\r\n".to_vec();
        msg.extend_from_slice(&g.config.menu.clone());
        write_desc(g, di, &msg);
        set_state(g, di, ConState::Menu);
    } else {
        write_desc(
            g,
            di,
            b"\r\nYOU ARE ABOUT TO DELETE THIS CHARACTER PERMANENTLY.\r\nARE YOU ABSOLUTELY SURE?\r\n\r\nPlease type \"yes\" to confirm: ",
        );
        set_state(g, di, ConState::Delcnf2);
    }
}

fn con_delcnf2(g: &mut Game, di: usize, arg: &[u8]) {
    let chid = g.descriptors.get(di).and_then(|d| d.character).unwrap();
    if arg == b"yes" || arg == b"YES" {
        if g.ch(chid).plr(flags::PLR_FROZEN) {
            write_desc(g, di, b"You try to kill yourself, but the ice stops you.\r\n");
            write_desc(g, di, b"Character not deleted.\r\n\r\n");
            set_state(g, di, ConState::Close);
            return;
        }
        let level = g.ch(chid).level;
        if level < LVL_GRGOD {
            g.ch_mut(chid).act.set(flags::PLR_DELETED);
        }
        crate::players_glue::save_char(g, chid);
        // Crash_delete_file: rent files are stage 3/7.
        let name = g.ch(chid).name.clone().unwrap_or_default();
        if g.config.selfdelete_fastwipe {
            crate::players_glue::remove_player_by_name(g, &name);
        }
        // delete_variables: stage 6.
        let mut msg = b"Character '".to_vec();
        msg.extend_from_slice(&name);
        msg.extend_from_slice(b"' deleted! Goodbye.\r\n");
        write_desc(g, di, &msg);
        let name_s = String::from_utf8_lossy(&name).into_owned();
        g.mudlog(MudlogKind::Nrm, LVL_GOD, true, &format!("{} (lev {}) has self-deleted.", name_s, level));
        set_state(g, di, ConState::Close);
    } else {
        let mut msg = b"\r\nCharacter not deleted.\r\n".to_vec();
        msg.extend_from_slice(&g.config.menu.clone());
        write_desc(g, di, &msg);
        set_state(g, di, ConState::Menu);
    }
}
