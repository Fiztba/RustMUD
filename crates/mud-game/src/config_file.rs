//! `lib/etc/config`: the reader (`load_config`) and the writer
//! (`save_config`).
//!
//! No `etc/config` ships with the game, so a fresh install runs on the
//! compiled-in defaults and prints
//!
//! ```text
//! No etc/config file, using defaults: No such file or directory
//! ```
//!
//! to stderr on the way past — not through `log`, so it carries no
//! timestamp and no SYSERR marker. cedit writes the file; `load_config`
//! reads it back on the next boot.
//!
//! Two writer/reader mismatches are worth recording. Each would make a
//! cedit setting silently revert on the next boot:
//!
//! * **B72**: `disp_closed_doors` was written; `display_closed_doors` is
//! what the reader looks for.
//! * **B73**: `min_rent_cost` is read and fully editable in cedit, but was
//! never written.

use crate::config::Config;
use crate::handler::atoi;
use mud_data::types::NOWHERE;
use mud_world::lex::Reader;
use std::path::{Path, PathBuf};

pub type BStr = Vec<u8>;

/// `CONFIG_FILE`: `LIB_ETC "config"`, resolved against the game's
/// working directory (which is the lib dir by the time this runs).
pub fn config_path(lib_dir: &Path) -> PathBuf {
    lib_dir.join("etc").join("config")
}

/// `strip_cr`: drop every '\r'.
fn strip_cr(s: &[u8]) -> BStr {
    s.iter().copied().filter(|&b| b != b'\r').collect()
}

/// `convert_from_tabs`: `parse_tab` into a scratch buffer.
fn convert_from_tabs(s: &[u8]) -> BStr {
    let mut out = s.to_vec();
    mud_net::editor::parse_tab(&mut out);
    out
}

/// Case-insensitive equality.
fn tag_is(tag: &[u8], name: &str) -> bool {
    tag.eq_ignore_ascii_case(name.as_bytes())
}

/// Append CRLF, for the four
/// canned-message tags.
fn with_crlf(line: &[u8]) -> BStr {
    let mut v = line.to_vec();
    v.extend_from_slice(b"\r\n");
    v
}

// ---------------------------------------------------------------------------
// load_config
// ---------------------------------------------------------------------------

/// Overlay `etc/config` onto `cfg`, which the caller has already filled with
/// `load_default_config`'s values. Returns the lines destined for stderr
/// (empty when the file was read).
pub fn load_config(lib_dir: &Path, cfg: &mut Config) -> Vec<String> {
    let path = config_path(lib_dir);
    let Ok(data) = std::fs::read(&path) else {
        // perror("No etc/config file, using defaults")
        return vec![format!(
            "No {} file, using defaults: No such file or directory",
            path.strip_prefix(lib_dir).unwrap_or(&path).display().to_string().replace('\\', "/")
        )];
    };

    let mut r = Reader::new(&data);
    while let Some(line) = r.get_line_sized(254) {
        let (tag, rest) = crate::olc::split_argument(&line);
        let num = atoi(&rest);
        let Some(&first) = tag.first() else { continue };

        match first.to_ascii_lowercase() {
            b'a' => {
                if tag_is(&tag, "auto_save") {
                    cfg.auto_save = num != 0;
                } else if tag_is(&tag, "autosave_time") {
                    cfg.autosave_time = num;
                } else if tag_is(&tag, "auto_save_olc") {
                    cfg.auto_save_olc = num != 0;
                }
            }
            b'c' => {
                if tag_is(&tag, "crash_file_timeout") {
                    cfg.crash_file_timeout = num;
                }
            }
            b'd' => {
                if tag_is(&tag, "debug_mode") {
                    cfg.debug_mode = num;
                } else if tag_is(&tag, "display_closed_doors") {
                    cfg.display_closed_doors = num != 0;
                } else if tag_is(&tag, "diagonal_dirs") {
                    cfg.diagonal_dirs = num != 0;
                } else if tag_is(&tag, "dts_are_dumps") {
                    cfg.dts_are_dumps = num != 0;
                } else if tag_is(&tag, "donation_room_1") {
                    cfg.donation_room_1 = if num == -1 { NOWHERE as i32 } else { num };
                } else if tag_is(&tag, "donation_room_2") {
                    cfg.donation_room_2 = if num == -1 { NOWHERE as i32 } else { num };
                } else if tag_is(&tag, "donation_room_3") {
                    cfg.donation_room_3 = if num == -1 { NOWHERE as i32 } else { num };
                } else if tag_is(&tag, "dflt_dir") {
                    cfg.dflt_dir = if rest.is_empty() { b"lib".to_vec() } else { rest.clone() };
                } else if tag_is(&tag, "dflt_ip") {
                    cfg.dflt_ip = if rest.is_empty() { None } else { Some(rest.clone()) };
                } else if tag_is(&tag, "dflt_port") {
                    cfg.dflt_port = num as u16;
                } else if tag_is(&tag, "default_map_size") {
                    cfg.default_map_size = num;
                } else if tag_is(&tag, "default_minimap_size") {
                    cfg.default_minimap_size = num;
                }
            }
            b'f' => {
                if tag_is(&tag, "free_rent") {
                    cfg.free_rent = num != 0;
                } else if tag_is(&tag, "frozen_start_room") {
                    cfg.frozen_start_room = num;
                }
            }
            b'h' => {
                if tag_is(&tag, "holler_move_cost") {
                    cfg.holler_move_cost = num;
                } else if tag_is(&tag, "huh") {
                    cfg.huh = with_crlf(&rest);
                }
            }
            b'i' => {
                if tag_is(&tag, "idle_void") {
                    cfg.idle_void = num;
                } else if tag_is(&tag, "idle_rent_time") {
                    cfg.idle_rent_time = num;
                } else if tag_is(&tag, "idle_max_level") {
                    cfg.idle_max_level = num;
                } else if tag_is(&tag, "immort_start_room") {
                    cfg.immort_start_room = num;
                } else if tag_is(&tag, "ibt_autosave") {
                    cfg.ibt_autosave = num != 0;
                }
            }
            b'l' => {
                if tag_is(&tag, "level_can_shout") {
                    cfg.level_can_shout = num;
                } else if tag_is(&tag, "load_into_inventory") {
                    cfg.load_into_inventory = num != 0;
                } else if tag_is(&tag, "logname") {
                    cfg.logname = if rest.is_empty() { None } else { Some(rest.clone()) };
                }
            }
            b'm' => {
                if tag_is(&tag, "max_bad_pws") {
                    cfg.max_bad_pws = num;
                } else if tag_is(&tag, "max_exp_gain") {
                    cfg.max_exp_gain = num;
                } else if tag_is(&tag, "max_exp_loss") {
                    cfg.max_exp_loss = num;
                } else if tag_is(&tag, "max_filesize") {
                    cfg.max_filesize = num;
                } else if tag_is(&tag, "max_npc_corpse_time") {
                    cfg.max_npc_corpse_time = num;
                } else if tag_is(&tag, "max_obj_save") {
                    cfg.max_obj_save = num;
                } else if tag_is(&tag, "max_pc_corpse_time") {
                    cfg.max_pc_corpse_time = num;
                } else if tag_is(&tag, "max_playing") {
                    cfg.max_playing = num;
                } else if tag_is(&tag, "menu") {
                    // fread_string already applied parse_at; applying it
                    // a second time here is a no-op (parse_at leaves only
                    // doubled '@' behind, and steps over those).
                    if let Ok(Some(s)) = r.fread_string("Reading menu in load_config()") {
                        cfg.menu = s;
                    }
                } else if tag_is(&tag, "min_rent_cost") {
                    cfg.min_rent_cost = num;
                } else if tag_is(&tag, "min_wizlist_lev") {
                    cfg.min_wizlist_lev = num;
                } else if tag_is(&tag, "mortal_start_room") {
                    cfg.mortal_start_room = num;
                } else if tag_is(&tag, "map_option") {
                    cfg.map_option = num;
                } else if tag_is(&tag, "medit_advanced_stats") {
                    cfg.medit_advanced_stats = num != 0;
                }
            }
            b'n' => {
                if tag_is(&tag, "nameserver_is_slow") {
                    cfg.nameserver_is_slow = num != 0;
                } else if tag_is(&tag, "no_mort_to_immort") {
                    cfg.no_mort_to_immort = num != 0;
                } else if tag_is(&tag, "noperson") {
                    cfg.noperson = with_crlf(&rest);
                } else if tag_is(&tag, "noeffect") {
                    cfg.noeffect = with_crlf(&rest);
                }
            }
            b'o' => {
                if tag_is(&tag, "ok") {
                    cfg.ok = with_crlf(&rest);
                }
            }
            b'p' => {
                if tag_is(&tag, "pk_setting") {
                    cfg.pk_setting = num;
                } else if tag_is(&tag, "protocol_negotiation") {
                    cfg.protocol_negotiation = num != 0;
                } else if tag_is(&tag, "pt_setting") {
                    cfg.pt_setting = num;
                }
            }
            b'r' => {
                if tag_is(&tag, "rent_file_timeout") {
                    cfg.rent_file_timeout = num;
                }
            }
            b's' => {
                if tag_is(&tag, "siteok_everyone") {
                    cfg.siteok_everyone = num != 0;
                } else if tag_is(&tag, "script_players") {
                    cfg.script_players = num != 0;
                } else if tag_is(&tag, "special_in_comm") {
                    cfg.special_in_comm = num != 0;
                } else if tag_is(&tag, "start_messg") {
                    if let Ok(Some(s)) =
                        r.fread_string("Reading start message in load_config()")
                    {
                        cfg.start_messg = s;
                    }
                }
            }
            b't' => {
                if tag_is(&tag, "tunnel_size") {
                    cfg.tunnel_size = num;
                } else if tag_is(&tag, "track_through_doors") {
                    cfg.track_through_doors = num != 0;
                }
            }
            b'u' => {
                if tag_is(&tag, "use_autowiz") {
                    cfg.use_autowiz = num != 0;
                } else if tag_is(&tag, "use_new_socials") {
                    cfg.use_new_socials = num != 0;
                }
            }
            b'w' => {
                if tag_is(&tag, "welc_messg") {
                    // The one string tag load_config does NOT re-parse_at;
                    // fread_string's own pass is the only one.
                    if let Ok(Some(s)) =
                        r.fread_string("Reading welcome message in load_config()")
                    {
                        cfg.welc_messg = s;
                    }
                }
            }
            _ => {}
        }
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// save_config
// ---------------------------------------------------------------------------

const HEADER: &str = "\
* This file is autogenerated by OasisOLC (CEdit).
* Please note the following information about this file's format.
*
* - If variable is a yes/no or true/false based variable, use 1's and 0's
*   where YES or TRUE = 1 and NO or FALSE = 0.
* - Variable names in this file are case-insensitive.  Variable values
*   are not case-insensitive.
* -----------------------------------------------------------------------
* Lines starting with * are comments, and are not parsed.
* -----------------------------------------------------------------------

* [ Game Play Options ]
";

/// Write `etc/config`. Returns false when the file could not be opened;
/// the caller removes the SL_CFG save-list entry on success.
pub fn save_config(lib_dir: &Path, cfg: &Config) -> bool {
    let mut out: BStr = Vec::new();

    let opt = |out: &mut BStr, comment: &str, tag: &str, val: i32| {
        out.extend_from_slice(comment.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(format!("{tag} = {val}\n\n").as_bytes());
    };
    let b = |v: bool| if v { 1 } else { 0 };

    out.extend_from_slice(HEADER.as_bytes());

    opt(&mut out, "* Is player killing allowed on the mud?", "pk_setting", cfg.pk_setting);
    opt(&mut out, "* Is player thieving allowed on the mud?", "pt_setting", cfg.pt_setting);
    opt(
        &mut out,
        "* What is the minimum level a player can shout/gossip/etc?",
        "level_can_shout",
        cfg.level_can_shout,
    );
    opt(
        &mut out,
        "* How many movement points does shouting cost the player?",
        "holler_move_cost",
        cfg.holler_move_cost,
    );
    opt(&mut out, "* How many players can fit in a tunnel?", "tunnel_size", cfg.tunnel_size);
    opt(&mut out, "* Maximum experience gainable per kill?", "max_exp_gain", cfg.max_exp_gain);
    opt(&mut out, "* Maximum experience loseable per death?", "max_exp_loss", cfg.max_exp_loss);
    opt(
        &mut out,
        "* Number of tics before NPC corpses decompose.",
        "max_npc_corpse_time",
        cfg.max_npc_corpse_time,
    );
    opt(
        &mut out,
        "* Number of tics before PC corpses decompose.",
        "max_pc_corpse_time",
        cfg.max_pc_corpse_time,
    );
    opt(&mut out, "* Number of tics before a PC is sent to the void.", "idle_void", cfg.idle_void);
    opt(
        &mut out,
        "* Number of tics before a PC is autorented.",
        "idle_rent_time",
        cfg.idle_rent_time,
    );
    opt(
        &mut out,
        "* Level and above of players whom are immune to idle penalties.",
        "idle_max_level",
        cfg.idle_max_level,
    );
    opt(
        &mut out,
        "* Should the items in death traps be junked automatically?",
        "dts_are_dumps",
        b(cfg.dts_are_dumps),
    );
    opt(
        &mut out,
        "* When an immortal loads an object, should it load into their inventory?",
        "load_into_inventory",
        b(cfg.load_into_inventory),
    );
    opt(
        &mut out,
        "* Should PC's be able to track through hidden or closed doors?",
        "track_through_doors",
        b(cfg.track_through_doors),
    );
    opt(
        &mut out,
        "* Should players who reach enough exp be prevented from automatically levelling to immortal?",
        "no_mort_to_immort",
        b(cfg.no_mort_to_immort),
    );
    // The writer and reader once disagreed on this tag's name
    // (`disp_closed_doors` vs `display_closed_doors`), so the option never
    // survived a reboot.
    opt(
        &mut out,
        "* Should closed doors be shown on autoexit / exit?",
        "display_closed_doors",
        b(cfg.display_closed_doors),
    );
    opt(&mut out, "* Are diagonal directions enabled?", "diagonal_dirs", b(cfg.diagonal_dirs));
    opt(
        &mut out,
        "* Who can use the map functions? 0=off, 1=on, 2=imm_only",
        "map_option",
        cfg.map_option,
    );
    opt(
        &mut out,
        "* Default size of map shown by 'map' command",
        "default_map_size",
        cfg.default_map_size,
    );
    opt(
        &mut out,
        "* Default minimap size shown to the right of room descriptions",
        "default_minimap_size",
        cfg.default_minimap_size,
    );
    opt(
        &mut out,
        "* Do you want scripts to be attachable to players?",
        "script_players",
        b(cfg.script_players),
    );

    let msg = |out: &mut BStr, comment: &str, tag: &str, val: &[u8], trailing: &str| {
        out.extend_from_slice(comment.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(tag.as_bytes());
        out.extend_from_slice(b" = ");
        out.extend_from_slice(&strip_cr(val));
        out.extend_from_slice(trailing.as_bytes());
    };

    msg(
        &mut out,
        "* Text sent to players when OK is all that is needed.",
        "ok",
        &cfg.ok,
        "\n\n",
    );
    msg(
        &mut out,
        "* Text sent to players for an unrecognized command.",
        "huh",
        &cfg.huh,
        "\n\n",
    );
    msg(
        &mut out,
        "* Text sent to players when noone is available.",
        "noperson",
        &cfg.noperson,
        "\n\n",
    );
    // One '\n' here, not two -- the only asymmetric tag in the file.
    msg(
        &mut out,
        "* Text sent to players when an effect fails.",
        "noeffect",
        &cfg.noeffect,
        "\n",
    );

    out.extend_from_slice(b"\n\n\n* [ Rent/Crashsave Options ]\n");
    out.extend_from_slice(
        b"* Should the MUD allow you to 'rent' for free?  (i.e. if you just quit,\n",
    );
    out.extend_from_slice(b"* your objects are saved at no cost, as in Merc-type MUDs.)\n");
    out.extend_from_slice(format!("free_rent = {}\n\n", b(cfg.free_rent)).as_bytes());
    opt(
        &mut out,
        "* Maximum number of items players are allowed to rent.",
        "max_obj_save",
        cfg.max_obj_save,
    );
    // Cedit edits this and load_config reads it, so it has to be
    // written. Placed after max_obj_save so the file follows the menu's
    // order.
    opt(
        &mut out,
        "* Surcharge added on top of item costs when renting.",
        "min_rent_cost",
        cfg.min_rent_cost,
    );
    opt(&mut out, "* Should the game automatically save people?", "auto_save", b(cfg.auto_save));
    opt(
        &mut out,
        "* If auto_save = 1, how often (in minutes) should the game save people's objects?",
        "autosave_time",
        cfg.autosave_time,
    );
    opt(
        &mut out,
        "* Lifetime of crashfiles and force-rent (idlesave) files in days.",
        "crash_file_timeout",
        cfg.crash_file_timeout,
    );
    opt(
        &mut out,
        "* Lifetime of normal rent files in days.",
        "rent_file_timeout",
        cfg.rent_file_timeout,
    );

    out.extend_from_slice(b"\n\n\n* [ Room Numbers ]\n");
    opt(
        &mut out,
        "* The virtual number of the room that mortals should enter at.",
        "mortal_start_room",
        cfg.mortal_start_room,
    );
    opt(
        &mut out,
        "* The virtual number of the room that immorts should enter at.",
        "immort_start_room",
        cfg.immort_start_room,
    );
    opt(
        &mut out,
        "* The virtual number of the room that frozen people should enter at.",
        "frozen_start_room",
        cfg.frozen_start_room,
    );
    let dn = |v: i32| if v != NOWHERE as i32 { v } else { -1 };
    out.extend_from_slice(
        b"* The virtual numbers of the donation rooms.  Note: Add donation rooms\n",
    );
    out.extend_from_slice(
        b"* sequentially (1 & 2 before 3). If you don't, you might not be able to\n",
    );
    out.extend_from_slice(b"* donate. Use -1 for 'no such room'.\n");
    out.extend_from_slice(
        format!(
            "donation_room_1 = {}\ndonation_room_2 = {}\ndonation_room_3 = {}\n\n",
            dn(cfg.donation_room_1),
            dn(cfg.donation_room_2),
            dn(cfg.donation_room_3)
        )
        .as_bytes(),
    );

    out.extend_from_slice(b"\n\n\n* [ Game Operation Options ]\n");
    out.extend_from_slice(
        b"* This is the default port on which the game should run if no port is\n",
    );
    out.extend_from_slice(b"* given on the command-line.  NOTE WELL: If you're using the\n");
    out.extend_from_slice(b"* 'autorun' script, the port number there will override this setting.\n");
    out.extend_from_slice(b"* Change the PORT= line in autorun instead of (or in addition to)\n");
    out.extend_from_slice(b"* changing this.\n");
    out.extend_from_slice(format!("DFLT_PORT = {}\n\n", cfg.dflt_port).as_bytes());

    if let Some(ip) = &cfg.dflt_ip {
        msg(&mut out, "* IP address to which the MUD should bind.", "DFLT_IP", ip, "\n\n");
    }
    if !cfg.dflt_dir.is_empty() {
        msg(
            &mut out,
            "* default directory to use as data directory.",
            "DFLT_DIR",
            &cfg.dflt_dir,
            "\n\n",
        );
    }
    if let Some(ln) = &cfg.logname {
        msg(
            &mut out,
            "* What file to log messages to (ex: 'log/syslog').",
            "LOGNAME",
            ln,
            "\n\n",
        );
    }

    opt(
        &mut out,
        "* Maximum number of players allowed before game starts to turn people away.",
        "max_playing",
        cfg.max_playing,
    );
    opt(
        &mut out,
        "* Maximum size of bug, typo, and idea files in bytes (to prevent bombing).",
        "max_filesize",
        cfg.max_filesize,
    );
    opt(
        &mut out,
        "* Maximum number of password attempts before disconnection.",
        "max_bad_pws",
        cfg.max_bad_pws,
    );
    opt(
        &mut out,
        "* Is the site ok for everyone except those that are banned?",
        "siteok_everyone",
        b(cfg.siteok_everyone),
    );
    out.extend_from_slice(b"* If you want to use the original social file format\n");
    out.extend_from_slice(b"* and disable Aedit, set to 0, otherwise, 1.\n");
    out.extend_from_slice(format!("use_new_socials = {}\n\n", b(cfg.use_new_socials)).as_bytes());
    opt(
        &mut out,
        "* If the nameserver is fast, set to 0, otherwise, 1.",
        "nameserver_is_slow",
        b(cfg.nameserver_is_slow),
    );
    opt(
        &mut out,
        "* Should OLC autosave to disk (1) or save internally (0).",
        "auto_save_olc",
        b(cfg.auto_save_olc),
    );

    let block = |out: &mut BStr, comment: &str, tag: &str, val: &[u8]| {
        out.extend_from_slice(comment.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(tag.as_bytes());
        out.extend_from_slice(b" = \n");
        out.extend_from_slice(&convert_from_tabs(&strip_cr(val)));
        out.extend_from_slice(b"~\n\n");
    };

    if !cfg.menu.is_empty() {
        block(&mut out, "* The entrance/exit menu.", "MENU", &cfg.menu);
    }
    if !cfg.welc_messg.is_empty() {
        block(&mut out, "* The welcome message.", "WELC_MESSG", &cfg.welc_messg);
    }
    if !cfg.start_messg.is_empty() {
        block(&mut out, "* NEWBIE start message.", "START_MESSG", &cfg.start_messg);
    }

    opt(
        &mut out,
        "* Should the medit OLC show the advanced stats menu (1) or not (0).",
        "medit_advanced_stats",
        b(cfg.medit_advanced_stats),
    );
    opt(
        &mut out,
        "* Should the idea, bug and typo commands autosave (1) or not (0).",
        "ibt_autosave",
        b(cfg.ibt_autosave),
    );

    out.extend_from_slice(b"\n\n\n* [ Autowiz Options ]\n");
    out.extend_from_slice(
        b"* Should the game automatically create a new wizlist/immlist every time\n",
    );
    out.extend_from_slice(
        b"* someone immorts, or is promoted to a higher (or lower) god level?\n",
    );
    out.extend_from_slice(format!("use_autowiz = {}\n\n", b(cfg.use_autowiz)).as_bytes());
    opt(
        &mut out,
        "* If yes, what is the lowest level which should be on the wizlist?",
        "min_wizlist_lev",
        cfg.min_wizlist_lev,
    );
    opt(
        &mut out,
        "* If yes, enable the protocol negotiation system.",
        "protocol_negotiation",
        b(cfg.protocol_negotiation),
    );
    opt(
        &mut out,
        "* If yes, enable the special character in comm channels.",
        "special_in_comm",
        b(cfg.special_in_comm),
    );
    opt(
        &mut out,
        "* If 0 then off, otherwise 1: Brief, 2: Normal, 3: Complete.",
        "debug_mode",
        cfg.debug_mode,
    );

    let path = config_path(lib_dir);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&path, &out).is_ok()
}
