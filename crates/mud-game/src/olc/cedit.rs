//! The game configuration editor.
//!
//! The scratch copy is a whole `Config` — the same type the live
//! configuration has. Saving copies it back over the live configuration
//! and, when `auto_save` is on, writes `etc/config` through
//! [`crate::config_file::save_config`].
//!
//! Three defects are worth recording, since the fixes are load-bearing:
//!
//! * **B71**: the Game Play Options menu would redisplay itself **twice**
//! after an invalid key, its `default:` arm calling the display function
//! and then falling into the common trailing call. The other four menus
//! answer once; so does this one now.
//! * **B72**: `save_config` wrote `disp_closed_doors` while `load_config`
//! read `display_closed_doors`, so option `P` never survived a reboot.
//! * **B73**: `min_rent_cost` (option `C` on the crashsave menu) was
//! edited and read but never written, so it reverted to 100 on every boot.
//!
//! `add_to_save_list(NOWHERE, SL_CFG)` at the end of a save is a no-op,
//! since SL_CFG is rejected there — so "config" never appears in the `olc`
//! list. [`crate::db::add_to_save_list`] already models that.

use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::comm::{
    act, cc, send_editor_help, send_to_char, string_write, write_to_desc, C_NRM, KCYN, KGRN,
    KNRM, TO_ROOM,
};
use crate::config::Config;
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::interpreter::one_argument;
use crate::olc::{clear_screen, OlcData, StrTarget, CLEANUP_CONFIG};

pub const CEDIT_MAIN_MENU: i32 = 0;
pub const CEDIT_CONFIRM_SAVESTRING: i32 = 1;
pub const CEDIT_GAME_OPTIONS_MENU: i32 = 2;
pub const CEDIT_CRASHSAVE_OPTIONS_MENU: i32 = 3;
pub const CEDIT_OPERATION_OPTIONS_MENU: i32 = 4;
pub const CEDIT_ROOM_NUMBERS_MENU: i32 = 6;
pub const CEDIT_AUTOWIZ_OPTIONS_MENU: i32 = 7;
pub const CEDIT_OK: i32 = 8;
pub const CEDIT_HUH: i32 = 9;
pub const CEDIT_NOPERSON: i32 = 10;
pub const CEDIT_NOEFFECT: i32 = 11;
pub const CEDIT_DFLT_IP: i32 = 12;
pub const CEDIT_DFLT_DIR: i32 = 13;
pub const CEDIT_LOGNAME: i32 = 14;
pub const CEDIT_MENU: i32 = 15;
pub const CEDIT_WELC_MESSG: i32 = 16;
pub const CEDIT_START_MESSG: i32 = 17;
pub const CEDIT_LEVEL_CAN_SHOUT: i32 = 21;
pub const CEDIT_HOLLER_MOVE_COST: i32 = 22;
pub const CEDIT_TUNNEL_SIZE: i32 = 23;
pub const CEDIT_MAX_EXP_GAIN: i32 = 24;
pub const CEDIT_MAX_EXP_LOSS: i32 = 25;
pub const CEDIT_MAX_NPC_CORPSE_TIME: i32 = 26;
pub const CEDIT_MAX_PC_CORPSE_TIME: i32 = 27;
pub const CEDIT_IDLE_VOID: i32 = 28;
pub const CEDIT_IDLE_RENT_TIME: i32 = 29;
pub const CEDIT_IDLE_MAX_LEVEL: i32 = 30;
pub const CEDIT_MAX_OBJ_SAVE: i32 = 35;
pub const CEDIT_MIN_RENT_COST: i32 = 36;
pub const CEDIT_AUTOSAVE_TIME: i32 = 37;
pub const CEDIT_CRASH_FILE_TIMEOUT: i32 = 38;
pub const CEDIT_RENT_FILE_TIMEOUT: i32 = 39;
pub const CEDIT_MORTAL_START_ROOM: i32 = 40;
pub const CEDIT_IMMORT_START_ROOM: i32 = 41;
pub const CEDIT_FROZEN_START_ROOM: i32 = 42;
pub const CEDIT_DONATION_ROOM_1: i32 = 43;
pub const CEDIT_DONATION_ROOM_2: i32 = 44;
pub const CEDIT_DONATION_ROOM_3: i32 = 45;
pub const CEDIT_DFLT_PORT: i32 = 46;
pub const CEDIT_MAX_PLAYING: i32 = 47;
pub const CEDIT_MAX_FILESIZE: i32 = 48;
pub const CEDIT_MAX_BAD_PWS: i32 = 49;
pub const CEDIT_MIN_WIZLIST_LEV: i32 = 53;
pub const CEDIT_MAP_OPTION: i32 = 54;
pub const CEDIT_MAP_SIZE: i32 = 55;
pub const CEDIT_MINIMAP_SIZE: i32 = 56;
pub const CEDIT_DEBUG_MODE: i32 = 57;
pub const CEDIT_PK_SETTING: i32 = 58;
pub const CEDIT_PT_SETTING: i32 = 59;

/// CHECK_VAR: `(var == YES) ? "Yes": "No"`.
fn check_var(v: bool) -> &'static str {
    if v {
        "Yes"
    } else {
        "No"
    }
}

/// YESNO — upper case, unlike CHECK_VAR.
fn yesno(v: bool) -> &'static str {
    if v {
        "YES"
    } else {
        "NO"
    }
}

fn pk_pt_label(v: i32) -> &'static str {
    match v {
        0 => "Off",
        1 => "Limited",
        2 => "Free-for-all",
        _ => "Invalid!",
    }
}

fn map_label(v: i32) -> &'static str {
    match v {
        0 => "Off",
        1 => "On",
        2 => "Imm-Only",
        _ => "Invalid!",
    }
}

fn debug_label(v: i32) -> &'static str {
    match v {
        0 => "OFF",
        1 => "BRIEF",
        2 => "NORMAL",
        _ => "COMPLETE",
    }
}

fn cfg<'a>(olc: &'a OlcData) -> &'a Config {
    olc.config.as_ref().expect("cedit without a config copy")
}

fn cfg_mut<'a>(olc: &'a mut OlcData) -> &'a mut Config {
    olc.config.as_mut().expect("cedit without a config copy")
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_cedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, _) = one_argument(argument);

    if g.ch(chid).level < LVL_IMPL {
        send_to_char(g, chid, b"You can't modify the game configuration.\r\n");
        return;
    }

    if buf1.is_empty() {
        let mut olc = Box::new(OlcData::default());
        olc.zone_num = 0;
        cedit_setup(g, di, &mut olc);
        g.descriptors.get_mut(di).map(|d| d.state = ConState::Cedit);
        act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
        g.ch_mut(chid).act.set(mud_data::flags::PLR_WRITING);

        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(
            MudlogKind::Brf,
            level,
            true,
            &format!("OLC: {} starts editing the game configuration.", name),
        );
        g.olc.insert(di, olc);
        return;
    } else if crate::text::cmp_ci(b"save", &buf1) != std::cmp::Ordering::Equal {
        send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
        return;
    }

    send_to_char(g, chid, b"Saving the game configuration.\r\n");
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
    g.mudlog(
        MudlogKind::Cmp,
        level,
        true,
        &format!("OLC: {} saves the game configuration.", name),
    );

    cedit_save_to_disk(g);
}

// ---------------------------------------------------------------------------
// cedit_setup / cedit_save_internally / cedit_save_to_disk
// ---------------------------------------------------------------------------

/// cedit_setup. The whole struct is the working copy, so a clone is all
/// that is needed.
///
/// The one wrinkle: the four canned messages are defaulted, so an
/// empty one becomes the literal "undefined" in the editor.
fn cedit_setup(g: &mut Game, di: usize, olc: &mut OlcData) {
    let mut c = g.config.clone();
    c.ok = str_udup(&c.ok);
    c.huh = str_udup(&c.huh);
    c.noperson = str_udup(&c.noperson);
    c.noeffect = str_udup(&c.noeffect);
    olc.config = Some(Box::new(c));
    cedit_disp_menu(g, di, olc);
}

fn str_udup(s: &[u8]) -> BStr {
    if s.is_empty() {
        b"undefined".to_vec()
    } else {
        s.to_vec()
    }
}

/// str_udupnl: "undefined" when empty, and "\r\n" appended.
fn str_udupnl(s: &[u8]) -> BStr {
    let mut v = if s.is_empty() { b"undefined".to_vec() } else { s.to_vec() };
    v.extend_from_slice(b"\r\n");
    v
}

fn cedit_save_internally(g: &mut Game, di: usize, olc: &OlcData) {
    let new = cfg(olc).clone();
    let reassign = g.config.dts_are_dumps != new.dts_are_dumps;
    g.config = new;

    // "if we changed the dts to/from dumps, reassign - Welcor"
    if reassign {
        reassign_rooms(g);
    }

    crate::db::add_to_save_list(g, NOWHERE, crate::db::SL_CFG);
    let _ = di;
}

pub fn cedit_save_to_disk(g: &mut Game) -> bool {
    let lib = g.lib_dir.clone();
    let ok = crate::config_file::save_config(&lib, &g.config);
    if ok && crate::db::in_save_list(g, NOWHERE, crate::db::SL_CFG) {
        crate::db::remove_from_save_list(g, NOWHERE, crate::db::SL_CFG);
    }
    ok
}

/// reassign_rooms.
///
/// The loop covers every room, including the one with the highest rnum
/// of the LAST room — so the highest-numbered room keeps whatever proc it
/// had. Deliberate.
fn reassign_rooms(g: &mut Game) {
    // leaves the last room holding its old spec_proc.
    for s in g.room_specs.iter_mut() {
        *s = None;
    }
    crate::spec::assign_rooms(g);
}

// ---------------------------------------------------------------------------
// The menus
// ---------------------------------------------------------------------------

fn colors(g: &mut Game, di: usize) -> (&'static [u8], &'static [u8], &'static [u8]) {
    let ed = g.descriptors.get(di).and_then(|d| d.character);
    match ed {
        Some(ed) => (
            cc(g, ed, C_NRM, KGRN),
            cc(g, ed, C_NRM, KNRM),
            cc(g, ed, C_NRM, KCYN),
        ),
        None => (b"", b"", b""),
    }
}

fn cedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, _cyn) = colors(g, di);
    clear_screen(g, di);

    let mut out: BStr = Vec::new();
    out.extend_from_slice(b"OasisOLC MUD Configuration Editor\r\n");
    for (key, label) in [
        (&b"G"[..], &b") Game Play Options\r\n"[..]),
        (b"C", b") Crashsave/Rent Options\r\n"),
        (b"R", b") Room Numbers\r\n"),
        (b"O", b") Operation Options\r\n"),
        (b"A", b") Autowiz Options\r\n"),
        (b"Q", b") Quit\r\n"),
    ] {
        out.extend_from_slice(grn);
        out.extend_from_slice(key);
        out.extend_from_slice(nrm);
        out.extend_from_slice(label);
    }
    out.extend_from_slice(b"Enter your choice : ");
    write_to_desc(g, di, &out);

    olc.mode = CEDIT_MAIN_MENU;
}

/// One `%sX%s) label: %s<value>` row.
fn row(out: &mut BStr, grn: &[u8], nrm: &[u8], cyn: &[u8], key: &[u8], label: &str, val: &str) {
    out.extend_from_slice(grn);
    out.extend_from_slice(key);
    out.extend_from_slice(nrm);
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(cyn);
    out.extend_from_slice(val.as_bytes());
}

fn cedit_disp_game_play_options(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, cyn) = colors(g, di);
    clear_screen(g, di);
    let c = cfg(olc).clone();

    let mut o: BStr = Vec::new();
    o.extend_from_slice(b"\r\n\r\n");
    let n = |v: i32| v.to_string();

    row(&mut o, grn, nrm, cyn, b"A", ") Player Killing Allowed  : ", pk_pt_label(c.pk_setting));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"B", ") Player Thieving Allowed : ", pk_pt_label(c.pt_setting));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"C", ") Minimum Level To Shout  : ", &n(c.level_can_shout));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"D", ") Holler Move Cost        : ", &n(c.holler_move_cost));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"E", ") Tunnel Size             : ", &n(c.tunnel_size));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"F", ") Maximum Experience Gain : ", &n(c.max_exp_gain));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"G", ") Maximum Experience Loss : ", &n(c.max_exp_loss));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"H", ") Max Time for NPC Corpse : ", &n(c.max_npc_corpse_time));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"I", ") Max Time for PC Corpse  : ", &n(c.max_pc_corpse_time));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"J", ") Tics before PC sent to void : ", &n(c.idle_void));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"K", ") Tics before PC is autosaved : ", &n(c.idle_rent_time));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"L", ") Level Immune To IDLE        : ", &n(c.idle_max_level));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"M", ") Death Traps Junk Items      : ", check_var(c.dts_are_dumps));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"N", ") Objects Load Into Inventory : ", check_var(c.load_into_inventory));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"O", ") Track Through Doors         : ", check_var(c.track_through_doors));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"P", ") Display Closed Doors        : ", check_var(c.display_closed_doors));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"R", ") Diagonal Directions         : ", check_var(c.diagonal_dirs));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"S", ") Prevent Mortal Level To Immortal : ", check_var(c.no_mort_to_immort));
    o.extend_from_slice(b"\r\n");

    // These four already end in "\r\n", so the format string adds none.
    for (key, label, val) in [
        (&b"1"[..], ") OK Message Text         : ", &c.ok),
        (b"2", ") HUH Message Text        : ", &c.huh),
        (b"3", ") NOPERSON Message Text   : ", &c.noperson),
        (b"4", ") NOEFFECT Message Text   : ", &c.noeffect),
    ] {
        o.extend_from_slice(grn);
        o.extend_from_slice(key);
        o.extend_from_slice(nrm);
        o.extend_from_slice(label.as_bytes());
        o.extend_from_slice(cyn);
        o.extend_from_slice(val);
    }

    row(&mut o, grn, nrm, cyn, b"5", ") Map/Automap Option      : ", map_label(c.map_option));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"6", ") Default map size        : ", &n(c.default_map_size));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"7", ") Default minimap size    : ", &n(c.default_minimap_size));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"8", ") Scripts on PC's         : ", check_var(c.script_players));
    o.extend_from_slice(b"\r\n");

    o.extend_from_slice(grn);
    o.extend_from_slice(b"Q");
    o.extend_from_slice(nrm);
    o.extend_from_slice(b") Exit To The Main Menu\r\nEnter your choice : ");
    write_to_desc(g, di, &o);

    olc.mode = CEDIT_GAME_OPTIONS_MENU;
}

fn cedit_disp_crash_save_options(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, cyn) = colors(g, di);
    clear_screen(g, di);
    let c = cfg(olc).clone();

    let mut o: BStr = Vec::new();
    o.extend_from_slice(b"\r\n\r\n");
    row(&mut o, grn, nrm, cyn, b"A", ") Free Rent          : ", check_var(c.free_rent));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"B", ") Max Objects Saved  : ", &c.max_obj_save.to_string());
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"C", ") Minimum Rent Cost  : ", &c.min_rent_cost.to_string());
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"D", ") Auto Save          : ", check_var(c.auto_save));
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"E",
        ") Auto Save Time     : ",
        &format!("{} minute(s)", c.autosave_time),
    );
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"F",
        ") Crash File Timeout : ",
        &format!("{} day(s)", c.crash_file_timeout),
    );
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"G",
        ") Rent File Timeout  : ",
        &format!("{} day(s)", c.rent_file_timeout),
    );
    o.extend_from_slice(b"\r\n");
    o.extend_from_slice(grn);
    o.extend_from_slice(b"Q");
    o.extend_from_slice(nrm);
    o.extend_from_slice(b") Exit To The Main Menu\r\nEnter your choice : ");
    write_to_desc(g, di, &o);

    olc.mode = CEDIT_CRASHSAVE_OPTIONS_MENU;
}

fn cedit_disp_room_numbers(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, cyn) = colors(g, di);
    clear_screen(g, di);
    let c = cfg(olc).clone();

    let mut o: BStr = Vec::new();
    o.extend_from_slice(b"\r\n\r\n");
    for (key, label, val) in [
        (&b"A"[..], ") Mortal Start Room   : ", c.mortal_start_room),
        (b"B", ") Immortal Start Room : ", c.immort_start_room),
        (b"C", ") Frozen Start Room   : ", c.frozen_start_room),
        (b"1", ") Donation Room #1    : ", c.donation_room_1),
        (b"2", ") Donation Room #2    : ", c.donation_room_2),
        (b"3", ") Donation Room #3    : ", c.donation_room_3),
    ] {
        row(&mut o, grn, nrm, cyn, key, label, &val.to_string());
        o.extend_from_slice(b"\r\n");
    }
    o.extend_from_slice(grn);
    o.extend_from_slice(b"Q");
    o.extend_from_slice(nrm);
    o.extend_from_slice(b") Exit To The Main Menu\r\nEnter your choice : ");
    write_to_desc(g, di, &o);

    olc.mode = CEDIT_ROOM_NUMBERS_MENU;
}

fn cedit_disp_operation_options(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, cyn) = colors(g, di);
    clear_screen(g, di);
    let c = cfg(olc).clone();

    let mut o: BStr = Vec::new();
    o.extend_from_slice(b"\r\n\r\n");
    row(&mut o, grn, nrm, cyn, b"A", ") Default Port : ", &c.dflt_port.to_string());
    o.extend_from_slice(b"\r\n");

    let or_none = |v: &Option<BStr>| -> BStr {
        match v {
            Some(s) => s.clone(),
            None => b"<None>".to_vec(),
        }
    };
    for (key, label, val) in [
        (&b"B"[..], ") Default IP   : ", or_none(&c.dflt_ip)),
        (
            b"C",
            ") Default Directory   : ",
            if c.dflt_dir.is_empty() { b"<None>".to_vec() } else { c.dflt_dir.clone() },
        ),
        (b"D", ") Logfile Name : ", or_none(&c.logname)),
    ] {
        o.extend_from_slice(grn);
        o.extend_from_slice(key);
        o.extend_from_slice(nrm);
        o.extend_from_slice(label.as_bytes());
        o.extend_from_slice(cyn);
        o.extend_from_slice(&val);
        o.extend_from_slice(b"\r\n");
    }

    row(&mut o, grn, nrm, cyn, b"E", ") Max Players  : ", &c.max_playing.to_string());
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"F", ") Max Filesize : ", &c.max_filesize.to_string());
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"G", ") Max Bad Pws  : ", &c.max_bad_pws.to_string());
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"H", ") Site Ok Everyone : ", yesno(c.siteok_everyone));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"I", ") Name Server Is Slow : ", yesno(c.nameserver_is_slow));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"J", ") Use new socials file: ", yesno(c.use_new_socials));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"K", ") OLC autosave to disk: ", yesno(c.auto_save_olc));
    o.extend_from_slice(b"\r\n");

    // The three block fields print on their own lines.
    for (key, label, val) in [
        (&b"L"[..], ") Main Menu           : \r\n", or_none(&opt(&c.menu))),
        (b"M", ") Welcome Message     : \r\n", or_none(&opt(&c.welc_messg))),
        (b"N", ") Start Message       : \r\n", or_none(&opt(&c.start_messg))),
    ] {
        o.extend_from_slice(grn);
        o.extend_from_slice(key);
        o.extend_from_slice(nrm);
        o.extend_from_slice(label.as_bytes());
        o.extend_from_slice(cyn);
        o.extend_from_slice(&val);
        o.extend_from_slice(b"\r\n");
    }

    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"O",
        ") Medit Stats Menu    : ",
        if c.medit_advanced_stats { "Advanced" } else { "Standard" },
    );
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"P",
        ") Autosave bugs when resolved from commandline : ",
        if c.ibt_autosave { "Yes" } else { "No" },
    );
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"R",
        ") Enable Protocol Negotiation : ",
        if c.protocol_negotiation { "Yes" } else { "No" },
    );
    o.extend_from_slice(b"\r\n");
    row(
        &mut o,
        grn,
        nrm,
        cyn,
        b"S",
        ") Enable Special Char in Comm : ",
        if c.special_in_comm { "Yes" } else { "No" },
    );
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"T", ") Current Debug Mode : ", debug_label(c.debug_mode));
    o.extend_from_slice(b"\r\n");

    o.extend_from_slice(grn);
    o.extend_from_slice(b"Q");
    o.extend_from_slice(nrm);
    o.extend_from_slice(b") Exit To The Main Menu\r\nEnter your choice : ");
    write_to_desc(g, di, &o);

    olc.mode = CEDIT_OPERATION_OPTIONS_MENU;
}

/// An empty message prints as "<None>".
fn opt(s: &[u8]) -> Option<BStr> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_vec())
    }
}

fn cedit_disp_autowiz_options(g: &mut Game, di: usize, olc: &mut OlcData) {
    let (grn, nrm, cyn) = colors(g, di);
    clear_screen(g, di);
    let c = cfg(olc).clone();

    let mut o: BStr = Vec::new();
    o.extend_from_slice(b"\r\n\r\n");
    row(&mut o, grn, nrm, cyn, b"A", ") Use the autowiz        : ", check_var(c.use_autowiz));
    o.extend_from_slice(b"\r\n");
    row(&mut o, grn, nrm, cyn, b"B", ") Minimum wizlist level  : ", &c.min_wizlist_lev.to_string());
    o.extend_from_slice(b"\r\n");
    o.extend_from_slice(grn);
    o.extend_from_slice(b"Q");
    o.extend_from_slice(nrm);
    o.extend_from_slice(b") Exit To The Main Menu\r\nEnter your choice : ");
    write_to_desc(g, di, &o);

    olc.mode = CEDIT_AUTOWIZ_OPTIONS_MENU;
}

// ---------------------------------------------------------------------------
// cedit_parse
// ---------------------------------------------------------------------------

/// Prompt-and-switch-mode, the shape most menu keys take.
fn ask(g: &mut Game, di: usize, olc: &mut OlcData, prompt: &[u8], mode: i32) {
    write_to_desc(g, di, prompt);
    olc.mode = mode;
}

const INVALID: &[u8] = b"\r\nThat is an invalid choice!\r\n";

pub fn cedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let first = arg.first().copied().unwrap_or(0);
    let lower = first.to_ascii_lowercase();

    match olc.mode {
        CEDIT_CONFIRM_SAVESTRING => {
            match lower {
                b'y' => {
                    cedit_save_internally(g, di, &olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        g.mudlog(
                            MudlogKind::Cmp,
                            level,
                            true,
                            &format!("OLC: {} modifies the game configuration.", name),
                        );
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_CONFIG);
                    if g.config.auto_save {
                        if cedit_save_to_disk(g) {
                            write_to_desc(g, di, b"Game configuration saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the game configuration"));
                        }
                    } else {
                        write_to_desc(g, di, b"Game configuration saved to memory.\r\n");
                    }
                    return None;
                }
                b'n' => {
                    write_to_desc(g, di, b"Game configuration not saved to memory.\r\n");
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_CONFIG);
                    return None;
                }
                _ => {
                    write_to_desc(g, di, INVALID);
                    write_to_desc(g, di, b"Do you wish to save your changes? : ");
                    return Some(olc);
                }
            }
        }

        CEDIT_MAIN_MENU => {
            match lower {
                b'g' => cedit_disp_game_play_options(g, di, &mut olc),
                b'c' => cedit_disp_crash_save_options(g, di, &mut olc),
                b'r' => cedit_disp_room_numbers(g, di, &mut olc),
                b'o' => cedit_disp_operation_options(g, di, &mut olc),
                b'a' => cedit_disp_autowiz_options(g, di, &mut olc),
                b'q' => {
                    write_to_desc(g, di, b"Do you wish to save your changes? : ");
                    olc.mode = CEDIT_CONFIRM_SAVESTRING;
                }
                _ => {
                    write_to_desc(g, di, b"That is an invalid choice!\r\n");
                    cedit_disp_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        CEDIT_GAME_OPTIONS_MENU => {
            match lower {
                b'a' => {
                    write_to_desc(
                        g,
                        di,
                        b"1) No Player Killing\r\n2) Limited Player Killing\r\n3) Free-for-all!\r\nEnter choice: ",
                    );
                    olc.mode = CEDIT_PK_SETTING;
                    return Some(olc);
                }
                b'b' => {
                    write_to_desc(
                        g,
                        di,
                        b"1) No Player Thieving\r\n2) Limited Player Thieving\r\n3) Free-for-all!\r\nEnter choice: ",
                    );
                    olc.mode = CEDIT_PT_SETTING;
                    return Some(olc);
                }
                b'c' => {
                    ask(g, di, &mut olc, b"Enter the minimum level a player must be to shout, gossip, etc : ", CEDIT_LEVEL_CAN_SHOUT);
                    return Some(olc);
                }
                b'd' => {
                    ask(g, di, &mut olc, b"Enter the amount it costs (in move points) to holler : ", CEDIT_HOLLER_MOVE_COST);
                    return Some(olc);
                }
                b'e' => {
                    ask(g, di, &mut olc, b"Enter the maximum number of people allowed in a tunnel : ", CEDIT_TUNNEL_SIZE);
                    return Some(olc);
                }
                b'f' => {
                    ask(g, di, &mut olc, b"Enter the maximum gain of experience per kill for players : ", CEDIT_MAX_EXP_GAIN);
                    return Some(olc);
                }
                b'g' => {
                    ask(g, di, &mut olc, b"Enter the maximum loss of experience per death for players : ", CEDIT_MAX_EXP_LOSS);
                    return Some(olc);
                }
                b'h' => {
                    ask(g, di, &mut olc, b"Enter the number of tics before NPC corpses decompose : ", CEDIT_MAX_NPC_CORPSE_TIME);
                    return Some(olc);
                }
                b'i' => {
                    ask(g, di, &mut olc, b"Enter the number of tics before PC corpses decompose : ", CEDIT_MAX_PC_CORPSE_TIME);
                    return Some(olc);
                }
                b'j' => {
                    ask(g, di, &mut olc, b"Enter the number of tics before PC's are sent to the void (idle) : ", CEDIT_IDLE_VOID);
                    return Some(olc);
                }
                b'k' => {
                    ask(g, di, &mut olc, b"Enter the number of tics before PC's are automatically rented and forced to quit : ", CEDIT_IDLE_RENT_TIME);
                    return Some(olc);
                }
                b'l' => {
                    ask(g, di, &mut olc, b"Enter the level a player must be to become immune to IDLE : ", CEDIT_IDLE_MAX_LEVEL);
                    return Some(olc);
                }
                b'm' => {
                    let c = cfg_mut(&mut olc);
                    c.dts_are_dumps = !c.dts_are_dumps;
                }
                b'n' => {
                    let c = cfg_mut(&mut olc);
                    c.load_into_inventory = !c.load_into_inventory;
                }
                b'o' => {
                    let c = cfg_mut(&mut olc);
                    c.track_through_doors = !c.track_through_doors;
                }
                b'p' => {
                    let c = cfg_mut(&mut olc);
                    c.display_closed_doors = !c.display_closed_doors;
                }
                b'r' => {
                    let c = cfg_mut(&mut olc);
                    c.diagonal_dirs = !c.diagonal_dirs;
                }
                b's' => {
                    let c = cfg_mut(&mut olc);
                    c.no_mort_to_immort = !c.no_mort_to_immort;
                }
                b'1' => {
                    ask(g, di, &mut olc, b"Enter the OK message : ", CEDIT_OK);
                    return Some(olc);
                }
                b'2' => {
                    ask(g, di, &mut olc, b"Enter the HUH message : ", CEDIT_HUH);
                    return Some(olc);
                }
                b'3' => {
                    ask(g, di, &mut olc, b"Enter the NOPERSON message : ", CEDIT_NOPERSON);
                    return Some(olc);
                }
                b'4' => {
                    ask(g, di, &mut olc, b"Enter the NOEFFECT message : ", CEDIT_NOEFFECT);
                    return Some(olc);
                }
                b'5' => {
                    write_to_desc(
                        g,
                        di,
                        b"1) Disable maps\r\n2) Enable Maps\r\n3) Maps for Immortals only\r\nEnter choice: ",
                    );
                    olc.mode = CEDIT_MAP_OPTION;
                    return Some(olc);
                }
                b'6' => {
                    ask(g, di, &mut olc, b"Enter default map size (1-12) : ", CEDIT_MAP_SIZE);
                    return Some(olc);
                }
                b'7' => {
                    ask(g, di, &mut olc, b"Enter default mini-map size (1-12) : ", CEDIT_MINIMAP_SIZE);
                    return Some(olc);
                }
                b'8' => {
                    let c = cfg_mut(&mut olc);
                    c.script_players = !c.script_players;
                }
                b'q' => {
                    cedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    // Displaying the menu here and then falling into
                    // the trailing call below would print it twice for one
                    // invalid key. The other four menus print once.
                    write_to_desc(g, di, INVALID);
                }
            }
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_CRASHSAVE_OPTIONS_MENU => {
            match lower {
                b'a' => {
                    let c = cfg_mut(&mut olc);
                    c.free_rent = !c.free_rent;
                }
                b'b' => {
                    ask(g, di, &mut olc, b"Enter the maximum number of items players can rent : ", CEDIT_MAX_OBJ_SAVE);
                    return Some(olc);
                }
                b'c' => {
                    ask(g, di, &mut olc, b"Enter the surcharge on top of item costs : ", CEDIT_MIN_RENT_COST);
                    return Some(olc);
                }
                b'd' => {
                    let c = cfg_mut(&mut olc);
                    c.auto_save = !c.auto_save;
                }
                b'e' => {
                    ask(g, di, &mut olc, b"Enter how often (in minutes) should the MUD save players : ", CEDIT_AUTOSAVE_TIME);
                    return Some(olc);
                }
                b'f' => {
                    ask(g, di, &mut olc, b"Enter the lifetime of crash and idlesave files (days) : ", CEDIT_CRASH_FILE_TIMEOUT);
                    return Some(olc);
                }
                b'g' => {
                    ask(g, di, &mut olc, b"Enter the lifetime of normal rent files (days) : ", CEDIT_RENT_FILE_TIMEOUT);
                    return Some(olc);
                }
                b'q' => {
                    cedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => write_to_desc(g, di, INVALID),
            }
            cedit_disp_crash_save_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_ROOM_NUMBERS_MENU => {
            match lower {
                b'a' => {
                    ask(g, di, &mut olc, b"Enter the room's vnum where mortals should load into : ", CEDIT_MORTAL_START_ROOM);
                    return Some(olc);
                }
                b'b' => {
                    ask(g, di, &mut olc, b"Enter the room's vnum where immortals should load into : ", CEDIT_IMMORT_START_ROOM);
                    return Some(olc);
                }
                b'c' => {
                    ask(g, di, &mut olc, b"Enter the room's vnum where frozen people should load into : ", CEDIT_FROZEN_START_ROOM);
                    return Some(olc);
                }
                b'1' => {
                    ask(g, di, &mut olc, b"Enter the vnum for donation room #1 : ", CEDIT_DONATION_ROOM_1);
                    return Some(olc);
                }
                b'2' => {
                    ask(g, di, &mut olc, b"Enter the vnum for donation room #2 : ", CEDIT_DONATION_ROOM_2);
                    return Some(olc);
                }
                b'3' => {
                    ask(g, di, &mut olc, b"Enter the vnum for donation room #3 : ", CEDIT_DONATION_ROOM_3);
                    return Some(olc);
                }
                b'q' => {
                    cedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => write_to_desc(g, di, INVALID),
            }
            cedit_disp_room_numbers(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_OPERATION_OPTIONS_MENU => {
            match lower {
                b'a' => {
                    ask(g, di, &mut olc, b"Enter the default port number : ", CEDIT_DFLT_PORT);
                    return Some(olc);
                }
                b'b' => {
                    ask(g, di, &mut olc, b"Enter the default IP Address : ", CEDIT_DFLT_IP);
                    return Some(olc);
                }
                b'c' => {
                    ask(g, di, &mut olc, b"Enter the default directory : ", CEDIT_DFLT_DIR);
                    return Some(olc);
                }
                b'd' => {
                    ask(g, di, &mut olc, b"Enter the name of the logfile : ", CEDIT_LOGNAME);
                    return Some(olc);
                }
                b'e' => {
                    ask(g, di, &mut olc, b"Enter the maximum number of players : ", CEDIT_MAX_PLAYING);
                    return Some(olc);
                }
                b'f' => {
                    ask(g, di, &mut olc, b"Enter the maximum size of the logs : ", CEDIT_MAX_FILESIZE);
                    return Some(olc);
                }
                b'g' => {
                    ask(g, di, &mut olc, b"Enter the maximum number of password attempts : ", CEDIT_MAX_BAD_PWS);
                    return Some(olc);
                }
                b'h' => {
                    let c = cfg_mut(&mut olc);
                    c.siteok_everyone = !c.siteok_everyone;
                }
                b'i' => {
                    let c = cfg_mut(&mut olc);
                    c.nameserver_is_slow = !c.nameserver_is_slow;
                }
                b'j' => {
                    {
                        let c = cfg_mut(&mut olc);
                        c.use_new_socials = !c.use_new_socials;
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_to_char(
                            g,
                            chid,
                            b"Please note that using the stock social file will disable AEDIT.\r\n",
                        );
                    }
                }
                b'k' => {
                    let c = cfg_mut(&mut olc);
                    c.auto_save_olc = !c.auto_save_olc;
                }
                b'l' => {
                    olc.mode = CEDIT_MENU;
                    return string_field(g, di, olc, b"Enter the new MENU :\r\n\r\n", StrTarget::CeditMenu);
                }
                b'm' => {
                    olc.mode = CEDIT_WELC_MESSG;
                    return string_field(
                        g,
                        di,
                        olc,
                        b"Enter the new welcome message :\r\n\r\n",
                        StrTarget::CeditWelcMessg,
                    );
                }
                b'n' => {
                    olc.mode = CEDIT_START_MESSG;
                    return string_field(
                        g,
                        di,
                        olc,
                        b"Enter the new newbie start message :\r\n\r\n",
                        StrTarget::CeditStartMessg,
                    );
                }
                b'o' => {
                    let c = cfg_mut(&mut olc);
                    c.medit_advanced_stats = !c.medit_advanced_stats;
                }
                b'p' => {
                    let c = cfg_mut(&mut olc);
                    c.ibt_autosave = !c.ibt_autosave;
                }
                b'r' => {
                    let c = cfg_mut(&mut olc);
                    c.protocol_negotiation = !c.protocol_negotiation;
                }
                b's' => {
                    let c = cfg_mut(&mut olc);
                    c.special_in_comm = !c.special_in_comm;
                }
                b't' => {
                    ask(
                        g,
                        di,
                        &mut olc,
                        b"Enter the current debug level (0: Off, 1: Brief, 2: Normal, 3: Complete) : ",
                        CEDIT_DEBUG_MODE,
                    );
                    return Some(olc);
                }
                b'q' => {
                    cedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => write_to_desc(g, di, INVALID),
            }
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_AUTOWIZ_OPTIONS_MENU => {
            match lower {
                b'a' => {
                    let c = cfg_mut(&mut olc);
                    c.use_autowiz = !c.use_autowiz;
                }
                b'b' => {
                    ask(g, di, &mut olc, b"Enter the minimum level for players to appear on the wizlist : ", CEDIT_MIN_WIZLIST_LEV);
                    return Some(olc);
                }
                b'q' => {
                    cedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => write_to_desc(g, di, INVALID),
            }
            cedit_disp_autowiz_options(g, di, &mut olc);
            return Some(olc);
        }

        _ => {}
    }

    // --- the value prompts -------------------------------------------------
    let empty = arg.is_empty();
    let num = atoi(arg);

    /// Shared shape: empty re-prompts, anything else assigns and redisplays.
    macro_rules! numeric {
        ($field:ident, $prompt:expr, $menu:ident) => {{
            if empty {
                write_to_desc(g, di, b"That is an invalid choice!\r\n");
                write_to_desc(g, di, $prompt);
            } else {
                cfg_mut(&mut olc).$field = num;
                $menu(g, di, &mut olc);
            }
            return Some(olc);
        }};
    }

    match olc.mode {
        CEDIT_LEVEL_CAN_SHOUT => numeric!(
            level_can_shout,
            b"Enter the minimum level a player must be to shout, gossip, etc : ",
            cedit_disp_game_play_options
        ),
        CEDIT_HOLLER_MOVE_COST => numeric!(
            holler_move_cost,
            b"Enter the amount it costs (in move points) to holler : ",
            cedit_disp_game_play_options
        ),
        CEDIT_TUNNEL_SIZE => numeric!(
            tunnel_size,
            b"Enter the maximum number of people allowed in a tunnel : ",
            cedit_disp_game_play_options
        ),

        // These two take an empty line without complaint.
        CEDIT_MAX_EXP_GAIN => {
            if !empty {
                cfg_mut(&mut olc).max_exp_gain = num;
            }
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }
        CEDIT_MAX_EXP_LOSS => {
            if !empty {
                cfg_mut(&mut olc).max_exp_loss = num;
            }
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_MAX_NPC_CORPSE_TIME => numeric!(
            max_npc_corpse_time,
            b"Enter the number of tics before NPC corpses decompose : ",
            cedit_disp_game_play_options
        ),
        CEDIT_MAX_PC_CORPSE_TIME => numeric!(
            max_pc_corpse_time,
            b"Enter the number of tics before PC corpses decompose : ",
            cedit_disp_game_play_options
        ),
        CEDIT_IDLE_VOID => numeric!(
            idle_void,
            b"Enter the number of tics before PC's are sent to the void (idle) : ",
            cedit_disp_game_play_options
        ),
        CEDIT_IDLE_RENT_TIME => numeric!(
            idle_rent_time,
            b"Enter the number of tics before PC's are automatically rented and forced to quit : ",
            cedit_disp_game_play_options
        ),
        CEDIT_IDLE_MAX_LEVEL => numeric!(
            idle_max_level,
            b"Enter the level a player must be to become immune to IDLE : ",
            cedit_disp_game_play_options
        ),

        CEDIT_OK | CEDIT_HUH | CEDIT_NOPERSON | CEDIT_NOEFFECT => {
            // genolc_checkstring: smash_tilde + parse_at, always true.
            let mut text = arg.to_vec();
            smash_tilde(&mut text);
            mud_net::editor::parse_at(&mut text);
            let v = str_udupnl(&text);
            let mode = olc.mode;
            let c = cfg_mut(&mut olc);
            match mode {
                CEDIT_OK => c.ok = v,
                CEDIT_HUH => c.huh = v,
                CEDIT_NOPERSON => c.noperson = v,
                _ => c.noeffect = v,
            }
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_MAX_OBJ_SAVE => numeric!(
            max_obj_save,
            b"Enter the maximum objects a player can save : ",
            cedit_disp_crash_save_options
        ),
        CEDIT_MIN_RENT_COST => numeric!(
            min_rent_cost,
            b"Enter the minimum amount it costs to rent : ",
            cedit_disp_crash_save_options
        ),
        CEDIT_AUTOSAVE_TIME => numeric!(
            autosave_time,
            b"Enter the interval for player's being autosaved : ",
            cedit_disp_crash_save_options
        ),
        CEDIT_CRASH_FILE_TIMEOUT => numeric!(
            crash_file_timeout,
            b"Enter the lifetime of crash and idlesave files (days) : ",
            cedit_disp_crash_save_options
        ),
        CEDIT_RENT_FILE_TIMEOUT => numeric!(
            rent_file_timeout,
            b"Enter the lifetime of rent files (days) : ",
            cedit_disp_crash_save_options
        ),

        CEDIT_MORTAL_START_ROOM
        | CEDIT_IMMORT_START_ROOM
        | CEDIT_FROZEN_START_ROOM
        | CEDIT_DONATION_ROOM_1
        | CEDIT_DONATION_ROOM_2
        | CEDIT_DONATION_ROOM_3 => {
            let prompt: &[u8] = match olc.mode {
                CEDIT_MORTAL_START_ROOM => b"Enter the room's vnum where mortals should load into : ",
                CEDIT_IMMORT_START_ROOM => b"Enter the room's vnum where immortals should load into : ",
                CEDIT_FROZEN_START_ROOM => b"Enter the room's vnum where frozen people should load into : ",
                CEDIT_DONATION_ROOM_1 => b"Enter the vnum for donation room #1 : ",
                CEDIT_DONATION_ROOM_2 => b"Enter the vnum for donation room #2 : ",
                _ => b"Enter the vnum for donation room #3 : ",
            };
            if empty {
                write_to_desc(g, di, b"That is an invalid choice!\r\n");
                write_to_desc(g, di, prompt);
            } else if g.real_room(num).is_none() {
                write_to_desc(g, di, b"That room doesn't exist!\r\n");
                write_to_desc(g, di, prompt);
            } else {
                let mode = olc.mode;
                let c = cfg_mut(&mut olc);
                match mode {
                    CEDIT_MORTAL_START_ROOM => c.mortal_start_room = num,
                    CEDIT_IMMORT_START_ROOM => c.immort_start_room = num,
                    CEDIT_FROZEN_START_ROOM => c.frozen_start_room = num,
                    CEDIT_DONATION_ROOM_1 => c.donation_room_1 = num,
                    CEDIT_DONATION_ROOM_2 => c.donation_room_2 = num,
                    _ => c.donation_room_3 = num,
                }
                cedit_disp_room_numbers(g, di, &mut olc);
            }
            return Some(olc);
        }

        CEDIT_DFLT_PORT => {
            cfg_mut(&mut olc).dflt_port = num as u16;
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_DFLT_IP | CEDIT_DFLT_DIR | CEDIT_LOGNAME => {
            let prompt: &[u8] = match olc.mode {
                CEDIT_DFLT_IP => b"Enter the default ip address : ",
                CEDIT_DFLT_DIR => b"Enter the default directory : ",
                _ => b"Enter the name of the logfile : ",
            };
            if empty {
                write_to_desc(g, di, b"That is an invalid choice!\r\n");
                write_to_desc(g, di, prompt);
            } else {
                let v = str_udup(arg);
                let mode = olc.mode;
                let c = cfg_mut(&mut olc);
                match mode {
                    CEDIT_DFLT_IP => c.dflt_ip = Some(v),
                    CEDIT_DFLT_DIR => c.dflt_dir = v,
                    _ => c.logname = Some(v),
                }
                cedit_disp_operation_options(g, di, &mut olc);
            }
            return Some(olc);
        }

        CEDIT_MAX_PLAYING => {
            cfg_mut(&mut olc).max_playing = num;
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }
        CEDIT_MAX_FILESIZE => {
            cfg_mut(&mut olc).max_filesize = num;
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }
        CEDIT_MAX_BAD_PWS => {
            cfg_mut(&mut olc).max_bad_pws = num;
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }
        CEDIT_DEBUG_MODE => {
            cfg_mut(&mut olc).debug_mode = num.clamp(0, 3);
            cedit_disp_operation_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_MIN_WIZLIST_LEV => {
            if num > LVL_IMPL as i32 {
                write_to_desc(
                    g,
                    di,
                    format!(
                        "The minimum wizlist level can't be greater than {}.\r\nEnter the minimum level for players to appear on the wizlist : ",
                        LVL_IMPL
                    )
                    .as_bytes(),
                );
            } else {
                cfg_mut(&mut olc).min_wizlist_lev = num;
                cedit_disp_autowiz_options(g, di, &mut olc);
            }
            return Some(olc);
        }

        CEDIT_MAP_OPTION => {
            if empty {
                write_to_desc(
                    g,
                    di,
                    b"That is an invalid choice!\r\nSelect 1, 2 or 3 (0 to cancel) :",
                );
            } else {
                if (1..=3).contains(&num) {
                    cfg_mut(&mut olc).map_option = num - 1;
                }
                cedit_disp_game_play_options(g, di, &mut olc);
            }
            return Some(olc);
        }

        // An empty line here restores the default rather than complaining.
        CEDIT_MAP_SIZE => {
            cfg_mut(&mut olc).default_map_size = if empty { 6 } else { num.clamp(1, 12) };
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }
        CEDIT_MINIMAP_SIZE => {
            cfg_mut(&mut olc).default_minimap_size = if empty { 2 } else { num.clamp(1, 12) };
            cedit_disp_game_play_options(g, di, &mut olc);
            return Some(olc);
        }

        CEDIT_PK_SETTING | CEDIT_PT_SETTING => {
            if empty || num < 0 || num > 3 {
                write_to_desc(
                    g,
                    di,
                    b"That is an invalid choice!\r\nSelect 1, 2 or 3 (0 to cancel) :",
                );
            } else {
                if (1..=3).contains(&num) {
                    let mode = olc.mode;
                    let c = cfg_mut(&mut olc);
                    if mode == CEDIT_PK_SETTING {
                        c.pk_setting = num - 1;
                    } else {
                        c.pt_setting = num - 1;
                    }
                }
                cedit_disp_game_play_options(g, di, &mut olc);
            }
            return Some(olc);
        }

        _ => {
            // "We should never get here, but just in case..."
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_CONFIG);
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: cedit_parse(): Reached default case!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
            None
        }
    }
}

/// The three block fields share one shape: clear, editor help, prompt, echo
/// the current value, then hand off to the string editor.
fn string_field(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    prompt: &[u8],
    target: StrTarget,
) -> Option<Box<OlcData>> {
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        send_editor_help(g, chid);
    }
    write_to_desc(g, di, prompt);

    let cur = match target {
        StrTarget::CeditMenu => cfg(&olc).menu.clone(),
        StrTarget::CeditWelcMessg => cfg(&olc).welc_messg.clone(),
        _ => cfg(&olc).start_messg.clone(),
    };
    let old = if cur.is_empty() { None } else { Some(cur) };
    if let Some(text) = &old {
        write_to_desc(g, di, text);
    }
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        string_write(g, chid, MAX_CONFIG_TEXT, 0, old);
    }
    olc.str_target = Some(target);
    Some(olc)
}

/// The ceiling for cedit's three login-time text screens.
///
/// It is `MAX_INPUT_LENGTH`, handed to `string_write` at all three call
/// sites -- an odd choice for a whole-buffer cap, given the constant is
/// documented as the maximum length of one *line* of input and the
/// MENU already spends 213 of its 512 bytes. Odd or not, it is the limit a
/// builder actually meets, so it is the limit here. This had been 4096.
const MAX_CONFIG_TEXT: usize = MAX_INPUT_LENGTH;

/// A trailing '~' becomes a space.
fn smash_tilde(s: &mut BStr) {
    if let Some(last) = s.last_mut() {
        if *last == b'~' {
            *last = b' ';
        }
    }
}

/// cedit_string_cleanup: every one of the three block
/// fields returns to the operation menu.
pub fn cedit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    saved: bool,
) -> Option<Box<OlcData>> {
    if saved {
        if let Some(t) = text {
            match olc.str_target {
                Some(StrTarget::CeditMenu) => cfg_mut(&mut olc).menu = t,
                Some(StrTarget::CeditWelcMessg) => cfg_mut(&mut olc).welc_messg = t,
                Some(StrTarget::CeditStartMessg) => cfg_mut(&mut olc).start_messg = t,
                _ => {}
            }
        }
    }
    olc.str_target = None;

    match olc.mode {
        CEDIT_MENU | CEDIT_WELC_MESSG | CEDIT_START_MESSG => {
            cedit_disp_operation_options(g, di, &mut olc);
        }
        _ => {}
    }
    Some(olc)
}
