//! The preference editor.
//!
//! The one editor that writes no file: it takes a copy of a player's toggles,
//! lets an immortal work on the copy, and on save writes them back to the
//! live character and calls `save_char`. The victim is held as a `CharId`,
//! so a quit mid-edit cannot leave a stale reference behind.
//!
//! Two shapes worth naming:
//!
//! * **B65**: the "already being edited" guard calls `act(buf,...)` in both
//! branches but only fills `buf` in the `ch != vict` one, so editing your
//! own prefs while someone else has them open renders uninitialized stack.
//! The act belongs to that branch and lives there now.
//! * **B74**: `PRF_AUTOMAP`, `PRF_AUTOKEY`, `PRF_AUTODOOR` and `PRF_VERBOSE`
//! are each commented "On" and were written `if (FLAGGED(x)) SET(x)` — set
//! only when already set — so "Restore all default values" left all four
//! exactly as it found them. Verified with Autoloot and Autogold as the
//! control: all six documented "On", all six turned off, and one press of
//! `d` bringing back only the two using `!FLAGGED`. Distinct from B67, a
//! genuine toggle on three other flags in the same function.
//! * The protocol half of the toggles menu (`J`-`R`) reads and writes the
//! EDITOR's own connection, not the victim's — protocol state is per
//! connection, so it cannot be anything else — even though the menu is
//! titled with the victim's name.

use mud_data::flags::{self, FlagSet};
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::{
    act, cc, send_to_char, C_NRM, KBGRN, KBRED, KBWHT, KBYEL, KCYN, KNRM, KWHT, KYEL, TO_ROOM,
};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::two_arguments;
use crate::olc::{OlcData, CLEANUP_ALL};

pub const PREFEDIT_MAIN_MENU: i32 = 0;
pub const PREFEDIT_PROMPT: i32 = 1;
pub const PREFEDIT_COLOR: i32 = 2;
pub const PREFEDIT_PAGELENGTH: i32 = 3;
pub const PREFEDIT_SCREENWIDTH: i32 = 4;
pub const PREFEDIT_WIMPY: i32 = 5;
pub const PREFEDIT_CONFIRM_SAVE: i32 = 6;
pub const PREFEDIT_SYSLOG: i32 = 7;
pub const PREFEDIT_TOGGLE_MENU: i32 = 8;

/// struct prefs_data: the working copy, plus who it belongs to.
#[derive(Debug, Clone)]
pub struct PrefsScratch {
    pub pref: FlagSet,
    pub wimp_level: i32,
    pub page_length: i32,
    pub screen_width: i32,
    /// OLC_PREFS(d)->ch, as an id rather than a pointer that can go stale.
    pub ch: CharId,
}

/// ONOFF.
fn onoff(v: bool) -> &'static str {
    if v {
        "ON"
    } else {
        "OFF"
    }
}

/// The four-way label prompt/colour/syslog levels share.
const MULTI_TYPES: [&str; 4] = ["Off", "Brief", "Normal", "Complete"];

fn flagged(olc: &OlcData, bit: usize) -> bool {
    olc.prefs.as_ref().is_some_and(|p| p.pref.is_set(bit))
}

fn toggle(olc: &mut OlcData, bit: usize) {
    let p = olc.prefs.as_mut().unwrap();
    if p.pref.is_set(bit) {
        p.pref.remove(bit);
    } else {
        p.pref.set(bit);
    }
}

/// Left-justified, width-limited: `%-<w>.<w>s`.
fn padl(out: &mut BStr, s: &[u8], w: usize) {
    let n = s.len().min(w);
    out.extend_from_slice(&s[..n]);
    out.extend(std::iter::repeat(b' ').take(w - n));
}

/// Right-justified, width-limited: `%<w>.<w>s`.
fn padr(out: &mut BStr, s: &[u8], w: usize) {
    let n = s.len().min(w);
    out.extend(std::iter::repeat(b' ').take(w - n));
    out.extend_from_slice(&s[..n]);
}

/// Left-justified number: `%-<w>d`, which does not truncate.
fn padn(out: &mut BStr, v: i32, w: usize) {
    let s = v.to_string();
    out.extend_from_slice(s.as_bytes());
    out.extend(std::iter::repeat(b' ').take(w.saturating_sub(s.len())));
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_prefedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    let (buf1, _buf2, _) = two_arguments(argument);

    let vict = if buf1.is_empty() {
        chid
    } else if g.ch(chid).level >= LVL_IMPL {
        match crate::handler::get_player_vis(g, chid, &buf1, false) {
            Some(v) => v,
            None => {
                send_to_char(g, chid, b"There is no-one here by that name.\r\n");
                return;
            }
        }
    } else {
        send_to_char(g, chid, b"You can't do that!\r\n");
        return;
    };

    if g.ch(vict).is_npc() {
        send_to_char(g, chid, b"Don't be ridiculous! Mobs don't have toggles.\r\n");
        return;
    }

    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Prefedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).and_then(|o| o.prefs.as_ref().map(|p| p.ch)) != Some(vict) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        if chid == vict {
            // Act runs on an unfilled buffer here as well.
            let mut msg = b"Your preferences are currently being edited by ".to_vec();
            msg.extend_from_slice(&who);
            msg.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &msg);
        } else {
            let mut msg = b"$S$u preferences are currently being edited by ".to_vec();
            msg.extend_from_slice(&who);
            msg.push(b'.');
            act(g, &msg, false, Some(chid), None, Some(vict.into()), crate::comm::TO_CHAR);
        }
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis_prefedit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();
    olc.number = 0;
    prefedit_setup(g, &mut olc, vict);
    prefedit_disp_main_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Prefedit;
    }

    act(g, b"$n starts editing toggles.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);
    // No mudlog here: it is done elsewhere.
}

fn prefedit_setup(g: &Game, olc: &mut OlcData, vict: CharId) {
    let ch = g.ch(vict);
    olc.prefs = Some(Box::new(PrefsScratch {
        pref: ch.player_specials.as_ref().map(|ps| ps.pref).unwrap_or(FlagSet::EMPTY),
        wimp_level: ch.player_specials.as_ref().map_or(0, |ps| ps.wimp_level),
        page_length: ch.player_specials.as_ref().map_or(0, |ps| ps.page_length),
        screen_width: ch.player_specials.as_ref().map_or(0, |ps| ps.screen_width),
        ch: vict,
    }));
    olc.value = 0;
}

fn prefedit_save_to_char(g: &mut Game, di: usize, olc: &OlcData) {
    let p = olc.prefs.as_ref().unwrap();
    let vict = p.ch;
    let playing = g.try_ch(vict).is_some()
        && g.ch(vict)
            .desc
            .and_then(|d| g.descriptors.get(d).map(|d| d.is_playing()))
            .unwrap_or(false);

    if g.try_ch(vict).is_some() && playing {
        {
            let ps = g.ch_mut(vict).ps_mut();
            ps.pref = p.pref;
            ps.wimp_level = p.wimp_level;
            ps.page_length = p.page_length;
            ps.screen_width = p.screen_width;
        }
        crate::players_glue::save_char(g, vict);
        return;
    }

    // The failure is split into four messages by cause.
    let (why, tail) = if g.try_ch(vict).is_none() {
        ("no vict", "no vict")
    } else if g.ch(vict).desc.is_none() {
        ("no vict descriptor", "no vict descriptor")
    } else {
        ("vict not playing", "vict not playing")
    };
    g.mudlog(
        MudlogKind::Brf,
        LVL_BUILDER,
        true,
        &format!("SYSERR: Unable to save toggles ({})", why),
    );
    // No trailing newline on this one.
    let msg = format!("Unable to save toggles ({})", tail);
    crate::comm::write_to_desc(g, di, msg.as_bytes());
}

// ---------------------------------------------------------------------------
// The menus
// ---------------------------------------------------------------------------

fn prefedit_disp_main_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let Some(ed) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let vict = olc.prefs.as_ref().unwrap().ch;

    let (nrm, yel, cyn, wht) = (
        cc(g, ed, C_NRM, KNRM),
        cc(g, ed, C_NRM, KYEL),
        cc(g, ed, C_NRM, KCYN),
        cc(g, ed, C_NRM, KWHT),
    );
    let (byel, bwht) = (cc(g, ed, C_NRM, KBYEL), cc(g, ed, C_NRM, KBWHT));

    let mut prompt_string: BStr = Vec::new();
    if flagged(olc, flags::PRF_DISPHP) {
        prompt_string.push(b'H');
    }
    if flagged(olc, flags::PRF_DISPMANA) {
        prompt_string.push(b'M');
    }
    if flagged(olc, flags::PRF_DISPMOVE) {
        prompt_string.push(b'V');
    }
    let color_idx = (flagged(olc, flags::PRF_COLOR_1) as usize)
        + (flagged(olc, flags::PRF_COLOR_2) as usize) * 2;
    let vict_name = g.ch(vict).get_name().to_vec();
    let vict_level = g.ch(vict).level;
    let (page_length, screen_width, wimp) = {
        let p = olc.prefs.as_ref().unwrap();
        (p.page_length, p.screen_width, p.wimp_level)
    };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(yel);
    out.extend_from_slice(b"Preferences for ");
    out.extend_from_slice(&vict_name);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(wht);
    out.extend_from_slice(b"Preferences\r\n");

    // P / L
    out.extend_from_slice(byel);
    out.extend_from_slice(b"P");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Prompt : ");
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    padl(&mut out, &prompt_string, 3);
    out.extend_from_slice(cyn);
    out.extend_from_slice(b"]         ");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"L");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Pagelength : ");
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    padn(&mut out, page_length, 3);
    out.extend_from_slice(cyn);
    out.extend_from_slice(b"]\r\n");

    // C / S
    out.extend_from_slice(byel);
    out.extend_from_slice(b"C");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Color  : ");
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    padl(&mut out, MULTI_TYPES[color_idx].as_bytes(), 8);
    out.extend_from_slice(cyn);
    out.extend_from_slice(b"]    ");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"S");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Screenwidth: ");
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    padn(&mut out, screen_width, 3);
    out.extend_from_slice(cyn);
    out.extend_from_slice(b"]\r\n");

    // W
    out.extend_from_slice(byel);
    out.extend_from_slice(b"W");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Wimpy  : ");
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    padn(&mut out, wimp, 4);
    out.extend_from_slice(cyn);
    out.push(b']');
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(byel);
    out.extend_from_slice(b"T");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Toggle Preferences...\r\n");

    if vict_level >= LVL_IMMORT {
        let syslog_idx = (flagged(olc, flags::PRF_LOG1) as usize)
            + (flagged(olc, flags::PRF_LOG2) as usize) * 2;
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(bwht);
        out.extend_from_slice(b"Immortal Preferences\r\n");

        // 1 / 4
        out.extend_from_slice(byel);
        out.extend_from_slice(b"1");
        out.extend_from_slice(nrm);
        out.extend_from_slice(b") Syslog Level ");
        out.extend_from_slice(cyn);
        out.push(b'[');
        out.extend_from_slice(yel);
        padr(&mut out, MULTI_TYPES[syslog_idx].as_bytes(), 8);
        out.extend_from_slice(cyn);
        out.extend_from_slice(b"]   ");
        imm_cell(&mut out, byel, nrm, cyn, yel, b"4", b") ClsOLC    ", flagged(olc, flags::PRF_CLS));
        out.extend_from_slice(b"\r\n");

        imm_cell(&mut out, byel, nrm, cyn, yel, b"2", b") Show Flags   ", flagged(olc, flags::PRF_SHOWVNUMS));
        out.extend_from_slice(b"        ");
        imm_cell(&mut out, byel, nrm, cyn, yel, b"5", b") No WizNet ", flagged(olc, flags::PRF_NOWIZ));
        out.extend_from_slice(b"\r\n");

        imm_cell(&mut out, byel, nrm, cyn, yel, b"3", b") No Hassle    ", flagged(olc, flags::PRF_NOHASSLE));
        out.extend_from_slice(b"        ");
        imm_cell(&mut out, byel, nrm, cyn, yel, b"6", b") Holylight ", flagged(olc, flags::PRF_HOLYLIGHT));
        out.extend_from_slice(b"\r\n");

        imm_cell(&mut out, byel, nrm, cyn, yel, b"7", b") Verbose      ", flagged(olc, flags::PRF_VERBOSE));
        out.extend_from_slice(b"        ");

        if vict_level == LVL_IMPL {
            imm_cell(&mut out, byel, nrm, cyn, yel, b"8", b") Zone Resets  ", flagged(olc, flags::PRF_ZONERESETS));
            out.extend_from_slice(b"\r\n");
        } else {
            out.extend_from_slice(b"\r\n");
        }
    }

    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"D");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Restore all default values\r\n");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"Q");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Quit\r\n\r\n");

    send_to_char(g, ed, &out);
    olc.mode = PREFEDIT_MAIN_MENU;
}

/// One `%sN%s) Label %s[%s%3s%s]` cell from the immortal block.
#[allow(clippy::too_many_arguments)]
fn imm_cell(
    out: &mut BStr,
    byel: &[u8],
    nrm: &[u8],
    cyn: &[u8],
    yel: &[u8],
    key: &[u8],
    label: &[u8],
    on: bool,
) {
    out.extend_from_slice(byel);
    out.extend_from_slice(key);
    out.extend_from_slice(nrm);
    out.extend_from_slice(label);
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(yel);
    let s = onoff(on);
    out.extend(std::iter::repeat(b' ').take(3usize.saturating_sub(s.len())));
    out.extend_from_slice(s.as_bytes());
    out.extend_from_slice(cyn);
    out.push(b']');
}

fn prefedit_disp_prompt_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let Some(ed) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let (nrm, cyn) = (cc(g, ed, C_NRM, KNRM), cc(g, ed, C_NRM, KCYN));
    let (byel, bwht) = (cc(g, ed, C_NRM, KBYEL), cc(g, ed, C_NRM, KBWHT));

    let prompt_string: BStr = if flagged(olc, flags::PRF_DISPAUTO) {
        b"<Auto>".to_vec()
    } else {
        let mut s = Vec::new();
        if flagged(olc, flags::PRF_DISPHP) {
            s.push(b'H');
        }
        if flagged(olc, flags::PRF_DISPMANA) {
            s.push(b'M');
        }
        if flagged(olc, flags::PRF_DISPMOVE) {
            s.push(b'V');
        }
        s
    };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(bwht);
    out.extend_from_slice(b"Prompt Settings\r\n");
    for (k, label) in [
        (&b"1"[..], &b") Toggle HP\r\n"[..]),
        (b"2", b") Toggle Mana\r\n"),
        (b"3", b") Toggle Moves\r\n"),
        (b"4", b") Toggle auto flag\r\n\r\n"),
    ] {
        out.extend_from_slice(byel);
        out.extend_from_slice(k);
        out.extend_from_slice(nrm);
        out.extend_from_slice(label);
    }
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"Current Prompt: ");
    out.extend_from_slice(cyn);
    out.extend_from_slice(&prompt_string);
    out.extend_from_slice(nrm);
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"0");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Quit (to main menu)\r\n");
    out.extend_from_slice(b"Enter Choice :");
    send_to_char(g, ed, &out);
    olc.mode = PREFEDIT_PROMPT;
}

/// The colour and syslog menus share a shape.
fn prefedit_disp_level_menu(g: &mut Game, di: usize, olc: &mut OlcData, syslog: bool) {
    let Some(ed) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let (nrm, yel) = (cc(g, ed, C_NRM, KNRM), cc(g, ed, C_NRM, KYEL));
    let (byel, bwht) = (cc(g, ed, C_NRM, KBYEL), cc(g, ed, C_NRM, KBWHT));

    let rows: [(&[u8], &[u8]); 4] = if syslog {
        [
            (b"1) Off      ", b"(do not display any logs or error messages)"),
            (b"2) Brief    ", b"(show only important warnings or errors)"),
            (b"3) Normal   ", b"(show all warnings and errors)"),
            (b"4) Complete ", b"(show all logged information for your level)"),
        ]
    } else {
        [
            (b"1) Off      ", b"(do not display any color - monochrome)"),
            (b"2) Brief    ", b"(show minimal color where necessary)"),
            (b"3) Normal   ", b"(show game-enhancing color)"),
            (b"4) On       ", b"(show all colors whenever possible)"),
        ]
    };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(bwht);
    out.extend_from_slice(if syslog { &b"Syslog level\r\n"[..] } else { &b"Color level\r\n"[..] });
    for (head, tail) in rows {
        out.extend_from_slice(byel);
        out.extend_from_slice(&head[..1]);
        out.extend_from_slice(nrm);
        out.extend_from_slice(&head[1..]);
        out.extend_from_slice(yel);
        out.extend_from_slice(tail);
        out.extend_from_slice(nrm);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Enter Choice :");
    send_to_char(g, ed, &out);
    olc.mode = if syslog { PREFEDIT_SYSLOG } else { PREFEDIT_COLOR };
}

/// One `%sK%s) Label %s[%s%3s%s]` cell. `val_colour` is the colour for the
/// value: the auto-flags use bright green/red on the flag itself, the
/// channels use the same pair inverted, and the rest use plain yellow.
#[allow(clippy::too_many_arguments)]
fn cell(
    out: &mut BStr,
    byel: &[u8],
    nrm: &[u8],
    cyn: &[u8],
    val_colour: &[u8],
    key: &[u8],
    label: &[u8],
    on: bool,
) {
    out.extend_from_slice(byel);
    out.extend_from_slice(key);
    out.extend_from_slice(nrm);
    out.extend_from_slice(label);
    out.extend_from_slice(cyn);
    out.push(b'[');
    out.extend_from_slice(val_colour);
    let s = onoff(on);
    out.extend(std::iter::repeat(b' ').take(3usize.saturating_sub(s.len())));
    out.extend_from_slice(s.as_bytes());
    out.extend_from_slice(cyn);
    out.push(b']');
}

fn prefedit_disp_toggles_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let Some(ed) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let vict = olc.prefs.as_ref().unwrap().ch;
    let name = g.ch(vict).get_name().to_vec();

    let (nrm, yel, cyn) = (
        cc(g, ed, C_NRM, KNRM),
        cc(g, ed, C_NRM, KYEL),
        cc(g, ed, C_NRM, KCYN),
    );
    let (byel, bwht, bgrn, bred) = (
        cc(g, ed, C_NRM, KBYEL),
        cc(g, ed, C_NRM, KBWHT),
        cc(g, ed, C_NRM, KBGRN),
        cc(g, ed, C_NRM, KBRED),
    );
    let pick = |on: bool| if on { bgrn } else { bred };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"Toggle preferences for ");
    out.extend_from_slice(bgrn);
    padl(&mut out, &name, 20);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(bwht);
    out.extend_from_slice(b"Auto-flags                 Channels\r\n");

    // The auto-flag column pairs with the channel column; a channel is shown
    // as the inverse of its NO- flag, and coloured on that inverse too.
    let autos: [(&[u8], &[u8], usize); 6] = [
        (b"1", b") Autoexits    ", flags::PRF_AUTOEXIT),
        (b"2", b") Autoloot     ", flags::PRF_AUTOLOOT),
        (b"3", b") Autogold     ", flags::PRF_AUTOGOLD),
        (b"4", b") Autosac      ", flags::PRF_AUTOSAC),
        (b"5", b") Autoassist   ", flags::PRF_AUTOASSIST),
        (b"6", b") Autosplit    ", flags::PRF_AUTOSPLIT),
    ];
    let chans: [(&[u8], &[u8], usize); 5] = [
        (b"A", b") Gossip   ", flags::PRF_NOGOSS),
        (b"B", b") Shout    ", flags::PRF_NOSHOUT),
        (b"C", b") Tell     ", flags::PRF_NOTELL),
        (b"D", b") Auction  ", flags::PRF_NOAUCT),
        (b"E", b") Gratz    ", flags::PRF_NOGRATZ),
    ];
    for (i, (key, label, bit)) in autos.iter().enumerate() {
        let on = flagged(olc, *bit);
        cell(&mut out, byel, nrm, cyn, pick(on), key, label, on);
        if let Some((ck, cl, cbit)) = chans.get(i) {
            out.extend_from_slice(b"      ");
            let chan_on = !flagged(olc, *cbit);
            cell(&mut out, byel, nrm, cyn, pick(chan_on), ck, cl, chan_on);
        }
        out.extend_from_slice(b"\r\n");
    }

    for (key, label, bit) in [
        (&b"7"[..], &b") Automap      "[..], flags::PRF_AUTOMAP),
        (b"8", b") Autokey      ", flags::PRF_AUTOKEY),
        (b"9", b") Autodoor     ", flags::PRF_AUTODOOR),
    ] {
        let on = flagged(olc, bit);
        cell(&mut out, byel, nrm, cyn, pick(on), key, label, on);
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(bwht);
    out.extend_from_slice(b"Other Flags\r\n");
    // "No Summon" is the inverse of PRF_SUMMONABLE; these use plain yellow.
    cell(&mut out, byel, nrm, cyn, yel, b"F", b") No Summon    ", !flagged(olc, flags::PRF_SUMMONABLE));
    out.extend_from_slice(b"      ");
    cell(&mut out, byel, nrm, cyn, yel, b"H", b") Brief    ", flagged(olc, flags::PRF_BRIEF));
    out.extend_from_slice(b"\r\n");
    cell(&mut out, byel, nrm, cyn, yel, b"G", b") No Repeat    ", flagged(olc, flags::PRF_NOREPEAT));
    out.extend_from_slice(b"      ");
    cell(&mut out, byel, nrm, cyn, yel, b"I", b") Compact  ", flagged(olc, flags::PRF_COMPACT));
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(bwht);
    out.extend_from_slice(b"Protocol Settings:\r\n");
    // These read the EDITOR's connection, not the victim's: protocol state is
    // per connection, so there is nothing else they could read.
    let p = protocol_flags(g, di);
    for (lk, ll, lv, rk, rl, rv) in [
        (&b"J"[..], &b") Xterm 256    "[..], p.0, &b"M"[..], &b") MXP      "[..], p.4),
        (b"K", b") ANSI         ", p.1, b"N", b") MSDP     ", p.5),
        (b"L", b") Charset      ", p.2, b"O", b") ATCP     ", p.6),
        (b"P", b") UTF-8        ", p.3, b"R", b") MSP      ", p.7),
    ] {
        cell(&mut out, byel, nrm, cyn, yel, lk, ll, lv);
        out.extend_from_slice(b"      ");
        cell(&mut out, byel, nrm, cyn, yel, rk, rl, rv);
        out.extend_from_slice(b"\r\n");
    }

    // the protocol block's format string ends with a
    // bare "\r\n", so a blank line sits above the Q row.
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(byel);
    out.extend_from_slice(b"Q");
    out.extend_from_slice(nrm);
    out.extend_from_slice(b") Quit toggle preferences...\r\n");
    send_to_char(g, ed, &out);
    olc.mode = PREFEDIT_TOGGLE_MENU;
}

/// The eight protocol switches the toggles menu shows, in menu order:
/// Xterm256, ANSI, Charset, UTF-8, MXP, MSDP, ATCP, MSP.
fn protocol_flags(g: &Game, di: usize) -> (bool, bool, bool, bool, bool, bool, bool, bool) {
    use mud_net::protocol::Var;
    match g.descriptors.get(di) {
        Some(d) => (
            d.protocol.vars[Var::XTERM_256_COLORS as usize].value_int != 0,
            d.protocol.vars[Var::ANSI_COLORS as usize].value_int != 0,
            d.protocol.charset,
            d.protocol.vars[Var::UTF_8 as usize].value_int != 0,
            d.protocol.mxp,
            d.protocol.msdp,
            d.protocol.atcp,
            d.protocol.msp,
        ),
        None => (false, false, false, false, false, false, false, false),
    }
}

fn toggle_protocol(g: &mut Game, di: usize, which: u8) {
    use mud_net::protocol::Var;
    let Some(d) = g.descriptors.get_mut(di) else { return };
    let mut flip_var = |v: Var| {
        let slot = &mut d.protocol.vars[v as usize].value_int;
        *slot = i64::from(*slot == 0);
    };
    match which {
        b'J' => flip_var(Var::XTERM_256_COLORS),
        b'K' => flip_var(Var::ANSI_COLORS),
        b'P' => flip_var(Var::UTF_8),
        b'L' => d.protocol.charset = !d.protocol.charset,
        b'M' => d.protocol.mxp = !d.protocol.mxp,
        b'N' => d.protocol.msdp = !d.protocol.msdp,
        b'O' => d.protocol.atcp = !d.protocol.atcp,
        _ => d.protocol.msp = !d.protocol.msp,
    }
}

/// prefedit_Restore_Defaults.
///
/// The three immortal flags are written as
/// `if (!FLAGGED(x) && level > LVL_IMMORT) SET(x); else REMOVE(x);` — the
/// `!FLAGGED` clause makes "restore defaults" TOGGLE them for an immortal who
/// already has them, so running it twice gives opposite results. The default
/// does not depend on the current value, so the clause is gone.
fn prefedit_restore_defaults(g: &Game, olc: &mut OlcData) {
    let vict = olc.prefs.as_ref().unwrap().ch;

    // One list, shared with `init_char`, rather than a second copy here that
    // had drifted six flags away from it: the old list additionally turned on
    // AUTOLOOT, AUTOGOLD, AUTOMAP, AUTOKEY, AUTODOOR and VERBOSE, none of
    // which a new character is given. It also only ever touched the flags it
    // named, so BUILDWALK, ZONERESETS and the rest survived a "restore
    // defaults"; assigning the whole set clears them. Restoring defaults now
    // gives exactly what a newly created character is given.
    //
    // B67's correction is subsumed: the three immortal flags come from the
    // shared list, which decides them from the level alone rather than from
    // their current value, so restoring twice still gives the same answer.
    // B74's four are simply not defaults any more.
    let pref = crate::login::set_default_prefs(g, vict);

    let p = olc.prefs.as_mut().unwrap();
    p.pref = pref;
    // The non-toggle options, from the same defaults the player file uses
    // for a missing field (PFDEF_WIMPLEV/PAGELENGTH/SCREENWIDTH) rather
    // than from numbers written out again here.
    p.wimp_level = 0;
    p.page_length = 22;
    p.screen_width = 80;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn prefedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let Some(ed) = g.descriptors.get(di).and_then(|d| d.character) else { return Some(olc) };
    let vict_level = g.ch(olc.prefs.as_ref().unwrap().ch).level;

    let invalid = |g: &mut Game, ed: CharId| {
        let mut m: BStr = cc(g, ed, C_NRM, KBRED).to_vec();
        m.extend_from_slice(b"Invalid choice!");
        m.extend_from_slice(cc(g, ed, C_NRM, KNRM));
        m.extend_from_slice(b"\r\n");
        send_to_char(g, ed, &m);
    };

    match olc.mode {
        PREFEDIT_CONFIRM_SAVE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    prefedit_save_to_char(g, di, &olc);
                    let name = String::from_utf8_lossy(g.ch(ed).get_name()).into_owned();
                    let vname = {
                        let v = olc.prefs.as_ref().unwrap().ch;
                        String::from_utf8_lossy(g.ch(v).get_name()).into_owned()
                    };
                    let level = (LVL_BUILDER as i16).max(g.ch(ed).invis_lev()) as u8;
                    g.mudlog(
                        MudlogKind::Cmp,
                        level,
                        true,
                        &format!("OLC: {} edits toggles for {}", name, vname),
                    );
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    send_to_char(g, ed, b"Invalid choice!\r\n");
                    send_to_char(g, ed, b"Do you wish to save these toggle settings? : ");
                }
            }
            return Some(olc);
        }

        PREFEDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        send_to_char(g, ed, b"Do you wish to save these toggle settings? : ");
                        olc.mode = PREFEDIT_CONFIRM_SAVE;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    return Some(olc);
                }
                Some(b'p') | Some(b'P') => {
                    prefedit_disp_prompt_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'c') | Some(b'C') => {
                    prefedit_disp_level_menu(g, di, &mut olc, false);
                    return Some(olc);
                }
                Some(b'l') | Some(b'L') => {
                    send_to_char(g, ed, b"Enter number of lines per page (10-60): ");
                    olc.mode = PREFEDIT_PAGELENGTH;
                    return Some(olc);
                }
                Some(b's') | Some(b'S') => {
                    send_to_char(g, ed, b"Enter number of columns per page (40-120): ");
                    olc.mode = PREFEDIT_SCREENWIDTH;
                    return Some(olc);
                }
                Some(b'w') | Some(b'W') => {
                    // The prompt names the editor's own max_hit; the clamp
                    // below is a flat 0-500 regardless.
                    let cap = (g.ch(ed).points.max_hit as i32 / 2).min(500);
                    let msg = format!("Enter HP at which to flee (0-{}): ", cap);
                    send_to_char(g, ed, msg.as_bytes());
                    olc.mode = PREFEDIT_WIMPY;
                    return Some(olc);
                }
                Some(b't') | Some(b'T') => {
                    prefedit_disp_toggles_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'd') | Some(b'D') => {
                    prefedit_restore_defaults(g, &mut olc);
                }
                Some(c @ (b'1'..=b'8')) => {
                    let needed = if c == b'8' { LVL_IMPL } else { LVL_IMMORT };
                    if vict_level < needed {
                        invalid(g, ed);
                        prefedit_disp_main_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    match c {
                        b'1' => {
                            prefedit_disp_level_menu(g, di, &mut olc, true);
                            return Some(olc);
                        }
                        b'2' => toggle(&mut olc, flags::PRF_SHOWVNUMS),
                        b'3' => toggle(&mut olc, flags::PRF_NOHASSLE),
                        b'4' => toggle(&mut olc, flags::PRF_CLS),
                        b'5' => toggle(&mut olc, flags::PRF_NOWIZ),
                        b'6' => toggle(&mut olc, flags::PRF_HOLYLIGHT),
                        b'7' => toggle(&mut olc, flags::PRF_VERBOSE),
                        _ => toggle(&mut olc, flags::PRF_ZONERESETS),
                    }
                }
                _ => {
                    invalid(g, ed);
                    prefedit_disp_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
        }

        PREFEDIT_PAGELENGTH => {
            olc.prefs.as_mut().unwrap().page_length = atoi(arg).clamp(10, 60);
        }
        PREFEDIT_SCREENWIDTH => {
            olc.prefs.as_mut().unwrap().screen_width = atoi(arg).clamp(40, 120);
        }
        PREFEDIT_WIMPY => {
            olc.prefs.as_mut().unwrap().wimp_level = atoi(arg).clamp(0, 500);
        }

        PREFEDIT_COLOR | PREFEDIT_SYSLOG => {
            let syslog = olc.mode == PREFEDIT_SYSLOG;
            let number = atoi(arg) - 1;
            if !(0..=3).contains(&number) {
                let mut m: BStr = cc(g, ed, C_NRM, KBRED).to_vec();
                m.extend_from_slice(b"That's not a valid choice!");
                m.extend_from_slice(cc(g, ed, C_NRM, KNRM));
                m.extend_from_slice(b"\r\n");
                send_to_char(g, ed, &m);
                prefedit_disp_level_menu(g, di, &mut olc, syslog);
                return Some(olc);
            }
            let (lo, hi) = if syslog {
                (flags::PRF_LOG1, flags::PRF_LOG2)
            } else {
                (flags::PRF_COLOR_1, flags::PRF_COLOR_2)
            };
            let p = olc.prefs.as_mut().unwrap();
            p.pref.remove(lo);
            p.pref.remove(hi);
            if number % 2 == 1 {
                p.pref.set(lo);
            }
            if number >= 2 {
                p.pref.set(hi);
            }
        }

        PREFEDIT_PROMPT => {
            let number = atoi(arg);
            if !(0..=7).contains(&number) {
                invalid(g, ed);
                prefedit_disp_prompt_menu(g, di, &mut olc);
                return Some(olc);
            }
            match number {
                0 => {
                    prefedit_disp_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
                1 => toggle(&mut olc, flags::PRF_DISPHP),
                2 => toggle(&mut olc, flags::PRF_DISPMANA),
                3 => toggle(&mut olc, flags::PRF_DISPMOVE),
                4 => toggle(&mut olc, flags::PRF_DISPAUTO),
                _ => {}
            }
            olc.value = 1;
            prefedit_disp_prompt_menu(g, di, &mut olc);
            return Some(olc);
        }

        PREFEDIT_TOGGLE_MENU => {
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') | Some(b'x') | Some(b'X') => {
                    prefedit_disp_main_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(c) => {
                    let bit = match c.to_ascii_uppercase() {
                        b'1' => Some(flags::PRF_AUTOEXIT),
                        b'2' => Some(flags::PRF_AUTOLOOT),
                        b'3' => Some(flags::PRF_AUTOGOLD),
                        b'4' => Some(flags::PRF_AUTOSAC),
                        b'5' => Some(flags::PRF_AUTOASSIST),
                        b'6' => Some(flags::PRF_AUTOSPLIT),
                        b'7' => Some(flags::PRF_AUTOMAP),
                        b'8' => Some(flags::PRF_AUTOKEY),
                        b'9' => Some(flags::PRF_AUTODOOR),
                        b'A' => Some(flags::PRF_NOGOSS),
                        b'B' => Some(flags::PRF_NOSHOUT),
                        b'C' => Some(flags::PRF_NOTELL),
                        b'D' => Some(flags::PRF_NOAUCT),
                        b'E' => Some(flags::PRF_NOGRATZ),
                        b'F' => Some(flags::PRF_SUMMONABLE),
                        b'G' => Some(flags::PRF_NOREPEAT),
                        b'H' => Some(flags::PRF_BRIEF),
                        b'I' => Some(flags::PRF_COMPACT),
                        _ => None,
                    };
                    match bit {
                        Some(b) => toggle(&mut olc, b),
                        None => {
                            if matches!(c.to_ascii_uppercase(), b'J'..=b'R') {
                                toggle_protocol(g, di, c.to_ascii_uppercase());
                            }
                        }
                    }
                }
                None => {}
            }
            olc.value = 1;
            prefedit_disp_toggles_menu(g, di, &mut olc);
            return Some(olc);
        }

        _ => {}
    }

    olc.value = 1;
    prefedit_disp_main_menu(g, di, &mut olc);
    Some(olc)
}
