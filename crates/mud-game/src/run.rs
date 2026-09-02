//! Boot orchestration (boot_db) and the per-pulse engine
//! (game_loop steps 5-13 + heartbeat). Socket polling and
//! accept live in mud-server; everything after "bytes arrived" is here.

use mud_data::flags::{self};
use mud_data::rng::CircleRng;
use mud_data::types::*;
use mud_net::descriptor::{Descriptor, Descriptors};

use crate::act::BStr;
use crate::config::Config;
use crate::game::{EventKind, Game, MudEvent, MudlogKind, RoomRt, ZoneRt};
use crate::gametime::reset_time;
use crate::text::Texts;

pub struct BootFlags {
    pub mini_mud: bool,
    pub no_rent_check: bool,
    pub no_specials: bool,
    pub restrict: u8,
}

impl Default for BootFlags {
    fn default() -> Self {
        Self { mini_mud: false, no_rent_check: false, no_specials: false, restrict: 0 }
    }
}

pub fn local_tz_offset_secs(now: i64) -> i64 {
    // Logs and history are stamped with local time; chrono supplies the
    // local
    // offset. MUD_TZ_OFFSET (seconds) overrides it.
    if let Some(v) = std::env::var("MUD_TZ_OFFSET").ok().and_then(|v| v.parse().ok()) {
        return v;
    }
    let _ = now;
    chrono::Local::now().offset().local_minus_utc() as i64
}

/// boot_db + init_game glue: build a fully-booted Game.
pub fn boot_game(lib_dir: std::path::PathBuf, flags: BootFlags, seed: i64, now: i64) -> Result<Game, String> {
    let mut log_lines: Vec<String> = Vec::new();
    let mut config = Config::default();
    // cedit writes lib/etc/config, and it is read back here, before the world
    // boots, so an edit made in game outlives the reboot that follows it.
    // Anything the file does not set keeps the default above.
    log_lines.extend(crate::config_file::load_config(&lib_dir, &mut config));

    let mut rng = CircleRng::new(seed);

    log_lines.push("Boot db -- BEGIN.".to_string());
    log_lines.push("Resetting the game time:".to_string());
    let epoch = read_time_file(&lib_dir).unwrap_or(crate::gametime::DEFAULT_BEGINNING_OF_TIME);
    let (time_info, weather) = reset_time(epoch, now, &mut rng);
    log_lines.push(format!(
        "   Current Gametime: {}H {}D {}M {}Y.",
        time_info.hours, time_info.day, time_info.month, time_info.year
    ));

    log_lines.push("Reading news, credits, help, ideas, bugs, typos and motd files.".to_string());
    let texts = Texts::load(&lib_dir, &mut log_lines);

    log_lines.push("Loading spell definitions.".to_string());
    // mag_assign_spells: stage 5.

    let report = mud_world::boot::boot_world(&lib_dir)?;
    // Already carry their own "SYSERR: " — the parsers write the whole line.
    for w in &report.load_warnings {
        log_lines.push(w.clone());
    }
    for e in &report.zone_errors {
        log_lines.push(format!("SYSERR: {}", e));
    }
    let world = report.world;

    log_lines.push("Loading help entries.".to_string());
    let help_table = crate::text::boot_help(&lib_dir, flags.mini_mud, &mut log_lines);

    log_lines.push("Generating player index.".to_string());
    let (player_table, top_idnum) = match mud_world::players::load_index(&lib_dir) {
        Some((entries, top)) => (
            entries
                .into_iter()
                .map(|e| crate::game::PlayerIndexElement {
                    name: e.name,
                    id: e.id,
                    level: e.level,
                    flags: e.flags,
                    last: e.last,
                })
                .collect(),
            top,
        ),
        None => {
            log_lines.push("   No player index file!  First new char will be IMP!".to_string());
            (Vec::new(), 0)
        }
    };

    log_lines.push("Loading fight messages.".to_string());
    let fight_messages = match crate::fight::load_messages(&lib_dir) {
        Ok(t) => t,
        Err(e) => {
            return Err(e);
        }
    };

    log_lines.push("Loading social messages.".to_string());
    let socials = match crate::social::boot_social_messages(&lib_dir) {
        Ok((socials, lines)) => {
            log_lines.extend(lines);
            socials
        }
        Err(e) => {
            return Err(e);
        }
    };

    let invalid_names = read_invalid_list(&lib_dir);

    let rooms_rt = vec![RoomRt::default(); world.rooms.len()];
    let zones_rt = vec![ZoneRt::default(); world.zones.len()];
    let mob_counts = vec![0; world.mob_protos.len()];
    let obj_counts = vec![0; world.obj_protos.len()];

    let mut g = Game {
        world,
        rooms: rooms_rt,
        zones_rt,
        reset_q: Default::default(),
        autosave_minutes: 0,
        chars: Default::default(),
        objs: Default::default(),
        character_list: std::collections::VecDeque::new(),
        object_list: std::collections::VecDeque::new(),
        mob_counts,
        obj_counts,
        descriptors: Descriptors::default(),
        events: Vec::new(),
        time_info,
        weather,
        beginning_of_time: epoch,
        rng,
        now,
        boot_time: now,
        pulse: 0,
        player_table,
        top_idnum,
        zone_timer: 0,
        tz_offset_secs: local_tz_offset_secs(now),
        invalid_names,
        ban_list: Vec::new(),
        save_list: Vec::new(),
        r_mortal_start_room: 0,
        r_immort_start_room: 0,
        r_frozen_start_room: 0,
        olc: Default::default(),
        olc_colors: Default::default(),
        copyover: None,
        event_lists: Default::default(),
        config,
        texts,
        lib_dir,
        extractions_pending: 0,
        circle_shutdown: false,
        circle_reboot: 0,
        circle_restrict: flags.restrict,
        mini_mud: flags.mini_mud,
        no_rent_check: flags.no_rent_check,
        no_specials: flags.no_specials,
        dg_lookup: Default::default(),
        max_mob_id: crate::dg::MOB_ID_BASE,
        max_obj_id: crate::dg::OBJ_ID_BASE,
        next_trig_iid: 0,
        next_dg_event_id: 0,
        next_event_seq: 0,
        trig_counts: Vec::new(),
        trig_line_state: Default::default(),
        dg_script_depth: 0,
        dg_owner_purged: false,
        dg_act_check: false,
        log_lines,
        socials,
        commands: Vec::new(),
        help_table,
        help_table_version: 0,
        mob_specs: Vec::new(),
        obj_specs: Vec::new(),
        room_specs: Vec::new(),
        shops_rt: Vec::new(),
        shop_cmds: Default::default(),
        combat_list: Vec::new(),
        next_combat: None,
        fight_messages,
        mob_paths: Default::default(),
        groups: Vec::new(),
        next_group_id: 1,
        cast_arg2: Vec::new(),
        houses: Vec::new(),
        boards: Default::default(),
        ibt: Default::default(),
        happy: Default::default(),
        recent_list: Vec::new(),
        next_tick: SECS_PER_MUD_HOUR as i32,
        quest_secondary: Vec::new(),
        no_mail: false,
    };
    g.mob_specs = vec![None; g.world.mob_protos.len()];
    g.obj_specs = vec![None; g.world.obj_protos.len()];
    g.room_specs = vec![None; g.world.rooms.len()];
    g.shops_rt = vec![crate::shop::ShopRt::default(); g.world.shops.len()];
    g.trig_counts = vec![0; g.world.triggers.len()];
    // Parallel to world.quests, so add_quest/delete_quest can shift it with
    // the table. Sized here rather than in assign_the_quests, which `-s`
    // skips entirely.
    g.quest_secondary = vec![None; g.world.quests.len()];
    // check_start_rooms: resolve the three start rooms
    // once, through the fallback chain. A missing mortal start room is
    // fatal.
    match g.real_room(g.config.mortal_start_room) {
        Some(r) => g.r_mortal_start_room = r,
        None => {
            g.log("SYSERR:  Mortal start room does not exist.  Change in config.c.".to_string());
            return Err("Mortal start room does not exist".to_string());
        }
    }
    g.r_immort_start_room = match g.real_room(g.config.immort_start_room) {
        Some(r) => r,
        None => {
            if !g.mini_mud {
                g.log(
                    "SYSERR:  Warning: Immort start room does not exist.  Change in config.c."
                        .to_string(),
                );
            }
            g.r_mortal_start_room
        }
    };
    g.r_frozen_start_room = match g.real_room(g.config.frozen_start_room) {
        Some(r) => r,
        None => {
            if !g.mini_mud {
                g.log(
                    "SYSERR:  Warning: Frozen start room does not exist.  Change in config.c."
                        .to_string(),
                );
            }
            g.r_mortal_start_room
        }
    };
    // Room T-line scripts are recorded at world parse; instantiate them
    // here, before any zone reset can fire reset_wtrigger.
    crate::dg::boot_room_scripts(&mut g);

    g.log("Building command list.".to_string());
    crate::interpreter::create_command_list(&mut g);

    // Spec-proc assignment (boot_db order).
    if !g.no_specials {
        g.log("Assigning function pointers:".to_string());
        g.log("   Mobiles.".to_string());
        crate::spec::assign_mobiles(&mut g);
        g.log("   Shopkeepers.".to_string());
        crate::shop::assign_the_shopkeepers(&mut g);
        g.log("   Objects.".to_string());
        crate::spec::assign_objects(&mut g);
        g.log("   Rooms.".to_string());
        crate::spec::assign_rooms(&mut g);
        g.log("   Questmasters.".to_string());
        crate::quest::assign_the_quests(&mut g);
    }

    g.log("Assigning spell and skill levels.".to_string());
    // init_spell_levels ran with the spello table (stage 5).
    g.log("Sorting command list and spells.".to_string());
    // Both tables are already in create_command_list's sorted order.

    g.log("Booting mail system.".to_string());
    if !crate::mail::scan_file(&mut g) {
        g.log("    Mail boot failed -- Mail system disabled".to_string());
        g.no_mail = true;
    }
    g.log("Reading banned site and invalid-name list.".to_string());
    {
        let lib = g.lib_dir.clone();
        let mut lines = Vec::new();
        g.ban_list = crate::ban::load_banned(&lib, &mut lines);
        for l in lines {
            g.log(l);
        }
    }

    g.log("Loading Ideas.".to_string());
    crate::ibt::load_ibt_file(&mut g, crate::interpreter::SCMD_IDEA);
    g.log("Loading Bugs.".to_string());
    crate::ibt::load_ibt_file(&mut g, crate::interpreter::SCMD_BUG);
    g.log("Loading Typos.".to_string());
    crate::ibt::load_ibt_file(&mut g, crate::interpreter::SCMD_TYPO);

    if !g.no_rent_check {
        g.log("Deleting timed-out crash and rent files:".to_string());
        crate::objsave::update_obj_file(&mut g);
        g.log("   Done.".to_string());
    }

    // Moved here so the object limit code works.
    if !g.mini_mud {
        g.log("Booting houses.".to_string());
        crate::house::house_boot(&mut g);
    }

    g.log("Cleaning up last log.".to_string());
    crate::llog::clean_llog_entries(&mut g);

    // Boot-time zone resets in table order.
    crate::db::reset_all_zones(&mut g);
    g.log("Boot db -- DONE.".to_string());
    Ok(g)
}

fn read_time_file(lib: &std::path::Path) -> Option<i64> {
    let data = std::fs::read(lib.join("etc").join("time")).ok()?;
    let s = String::from_utf8_lossy(&data);
    let v: i64 = s.split_whitespace().next()?.parse().ok()?;
    if v == 0 { None } else { Some(v) }
}

/// save_mud_time: rewrite etc/time.
pub fn save_mud_time(g: &mut Game) {
    let secs = crate::gametime::mud_time_to_secs(&g.time_info, g.now);
    let path = g.lib_dir.join("etc").join("time");
    if let Err(e) = std::fs::write(&path, format!("{}\n", secs)) {
        g.log(format!("SYSERR: Couldn't write time file: {}", e));
    }
}

fn read_invalid_list(lib: &std::path::Path) -> Vec<Vec<u8>> {
    let Ok(data) = std::fs::read(lib.join("misc").join("xnames")) else {
        return Vec::new();
    };
    data.split(|c| *c == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter(|l| !l.is_empty() && l[0] != b'$')
        .take(200)
        .map(|l| l.to_ascii_lowercase())
        .collect()
}

/// new_descriptor tail after the server accepted and
/// resolved the host: create the descriptor, start negotiation or greet.
pub fn new_connection(g: &mut Game, stream: Option<mio::net::TcpStream>, host: &[u8]) -> usize {
    let negotiate = g.config.protocol_negotiation;
    let d = Descriptor::new(stream, host, 0, g.now, negotiate);
    let di = g.descriptors.insert(d);
    if negotiate {
        g.queue_event(3 * PASSES_PER_SEC / 2, EventKind::Protocols { desc: di });
        crate::comm::write_to_desc(g, di, b"Attempting to Detect Client, Please Wait...\r\n");
        let empty = g.descriptors.get(di).map(|d| d.output.is_empty()).unwrap_or(true);
        if let Some(d) = g.descriptors.get_mut(di) {
            mud_net::protocol::negotiate(&mut d.protocol, empty);
        }
        g.descriptors.pump_protocol_out(di);
    } else {
        let greetings = g.texts.greetings.clone();
        crate::comm::write_to_desc(g, di, &greetings);
    }
    di
}

/// copyover_recover's per-connection half: build the
/// descriptor around an already-open socket, reload the pfile, and drop the
/// player straight back into the world, in play.
pub fn copyover_attach(
    g: &mut Game,
    stream: mio::net::TcpStream,
    host: &[u8],
    guiopt: &[u8],
    name: &[u8],
    pref: i64,
) -> Option<usize> {
    let d = Descriptor::new(Some(stream), host, 0, g.now, false);
    let di = g.descriptors.insert(d);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Close;
        crate::copyover::copyover_set(&mut d.protocol, guiopt);
    }

    let shell = crate::ch::Char {
        player_specials: Some(Box::new(crate::ch::PlayerSpecials::default())),
        desc: Some(di),
        ..Default::default()
    };
    let chid = g.chars.insert(shell);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.character = Some(chid);
    }

    let mut ok = match crate::players_glue::load_char_into(g, chid, name) {
        Some(player_i) => {
            g.ch_mut(chid).pfilepos = player_i as i32;
            true
        }
        None => false,
    };
    if ok {
        if g.ch(chid).plr(mud_data::flags::PLR_DELETED) {
            ok = false;
        } else {
            for bit in [
                mud_data::flags::PLR_WRITING,
                mud_data::flags::PLR_MAILING,
                mud_data::flags::PLR_CRYO,
            ] {
                g.ch_mut(chid).act.remove(bit);
            }
        }
    }
    if !ok {
        crate::comm::write_direct(
            g,
            di,
            b"\r\nSomehow, your character was lost in the copyover. Sorry.\r\n",
        );
        close_socket(g, di);
        return None;
    }

    crate::comm::write_direct(g, di, b"\r\nCopyover recovery complete.\r\n");
    g.ch_mut(chid).punique = pref as i32;
    crate::login::enter_player_game(g, di);

    if !g.ch(chid).plr(mud_data::flags::PLR_LOADROOM) {
        g.ch_mut(chid).ps_mut().load_room = NOWHERE;
    }
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Playing;
    }
    crate::act::informative::look_at_room(g, chid, false);

    let pname = g.ch(chid).get_name().to_vec();
    let phost = g.descriptors.get(di).map(|d| d.host.clone()).unwrap_or_default();
    if !crate::llog::add_recent_player(g, &pname, &phost, false, true) {
        let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
        g.mudlog(
            crate::game::MudlogKind::Brf,
            lvl,
            true,
            "Failure to AddRecentPlayer (returned FALSE).",
        );
    }
    Some(di)
}


pub fn close_socket(g: &mut Game, di: usize) {
    let Some(d) = g.descriptors.get(di) else { return };
    let state = d.state;
    let chid = d.character;
    // A scanner that drops mid-handshake never sends a name, so the host is
    // the only thing identifying it. Taken here because the descriptor is
    // gone by the time the losing lines below are written.
    let host = d.host.clone();
    let is_playing = d.is_playing() || state == ConState::Disconnect;
    // Forget snooping.
    if let Some(sd) = g.descriptors.get(di).and_then(|d| d.snooping) {
        if let Some(d) = g.descriptors.get_mut(sd) {
            d.snoop_by = None;
        }
    }
    if let Some(bd) = g.descriptors.get(di).and_then(|d| d.snoop_by) {
        crate::comm::write_to_desc(g, bd, b"Your victim is no longer among us.\r\n");
        if let Some(d) = g.descriptors.get_mut(bd) {
            d.snooping = None;
        }
    }

    if let Some(chid) = chid {
        if g.try_ch(chid).is_some() {
            crate::llog::add_llog_entry(g, chid, crate::llog::LAST_DISCONNECT);
        }
    }
    if let Some(chid) = chid {
        if g.try_ch(chid).is_some() {
            if is_playing {
                g.ch_mut(chid).desc = None;
                crate::players_glue::save_char(g, chid);
                crate::comm::act(
                    g,
                    b"$n has lost $s link.",
                    true,
                    Some(chid),
                    None,
                    None,
                    crate::comm::TO_ROOM,
                );
                let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                let invis = g.ch(chid).invis_lev();
                g.mudlog(
                    MudlogKind::Nrm,
                    (LVL_IMMORT as i16).max(invis) as u8,
                    true,
                    &format!("Closing link to: {}.", name),
                );
            } else {
                let name = g
                    .try_ch(chid)
                    .and_then(|c| c.name.clone())
                    .map(|n| String::from_utf8_lossy(&n).into_owned())
                    .unwrap_or_else(|| "<null>".to_string());
                g.mudlog(
                    MudlogKind::Cmp,
                    LVL_IMMORT,
                    true,
                    &format!("Losing player: {} [{}].", name, String::from_utf8_lossy(&host)),
                );
                crate::handler::free_char(g, chid);
            }
        }
    } else {
        g.mudlog(
            MudlogKind::Cmp,
            LVL_IMMORT,
            true,
            &format!(
                "Losing descriptor without char from [{}].",
                String::from_utf8_lossy(&host)
            ),
        );
    }
    // Cancel pending protocol events for this descriptor.
    g.events.retain(|e| !matches!(e.kind, EventKind::Protocols { desc } if desc == di));
    // Kill any OLC stuff.
    crate::olc::cleanup_olc_on_close(g, di);
    g.descriptors.remove(di);
}

pub fn make_prompt(g: &Game, di: usize) -> BStr {
    let Some(d) = g.descriptors.get(di) else { return Vec::new() };
    if d.paging() {
        return format!(
            "[ Return to continue, (q)uit, (r)efresh, (b)ack, or page number ({}/{}) ]",
            d.showstr_page,
            d.showstr_count
        )
        .into_bytes();
    }
    if d.editing.is_some() {
        return b"] ".to_vec();
    }
    if d.state == ConState::Playing {
        let Some(chid) = d.character else { return Vec::new() };
        let Some(ch) = g.try_ch(chid) else { return Vec::new() };
        if ch.is_npc() {
            let mut p = ch.get_name().to_vec();
            p.extend_from_slice(b"> ");
            return p;
        }
        let mut prompt: BStr = Vec::new();
        let invis = ch.invis_lev();
        if invis != 0 {
            prompt.extend_from_slice(format!("i{} ", invis).as_bytes());
        }
        let p = &ch.points;
        if ch.prf(flags::PRF_DISPAUTO) {
            if p.hit << 2 < p.max_hit {
                prompt.extend_from_slice(format!("{}H ", p.hit).as_bytes());
            }
            if p.mana << 2 < p.max_mana {
                prompt.extend_from_slice(format!("{}M ", p.mana).as_bytes());
            }
            if p.mov << 2 < p.max_move {
                prompt.extend_from_slice(format!("{}V ", p.mov).as_bytes());
            }
        } else {
            if ch.prf(flags::PRF_DISPHP) {
                prompt.extend_from_slice(format!("{}H ", p.hit).as_bytes());
            }
            if ch.prf(flags::PRF_DISPMANA) {
                prompt.extend_from_slice(format!("{}M ", p.mana).as_bytes());
            }
            if ch.prf(flags::PRF_DISPMOVE) {
                prompt.extend_from_slice(format!("{}V ", p.mov).as_bytes());
            }
        }
        if ch.prf(flags::PRF_BUILDWALK) {
            prompt.extend_from_slice(b"BUILDWALKING ");
        }
        if ch.prf(flags::PRF_AFK) {
            prompt.extend_from_slice(b"AFK ");
        }
        let ps = ch.ps();
        if ps.last_news < g.texts.newsmod {
            prompt.extend_from_slice(b"(news) ");
        }
        if ps.last_motd < g.texts.motdmod {
            prompt.extend_from_slice(b"(motd) ");
        }
        prompt.extend_from_slice(b"> ");
        prompt.truncate(mud_data::types::MAX_PROMPT_LENGTH - 1);
        prompt
    } else {
        Vec::new()
    }
}

/// One full pulse: input already read by the server; here we run command
/// dispatch, output, prompts, closes, then heartbeat (steps 8-12).
pub fn game_pulse(g: &mut Game) {
    // Step 8: one command per descriptor, wait-state gated.
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        let chid = d.character;
        if let Some(chid) = chid {
            if let Some(ch) = g.chars.get_mut(chid) {
                if ch.wait > 0 {
                    ch.wait -= 1;
                }
                if ch.wait != 0 {
                    continue;
                }
            }
        }
        let Some(d) = g.descriptors.get_mut(di) else { continue };
        let Some((mut comm, aliased)) = d.input.pop_front() else { continue };
        if let Some(chid) = d.character {
            if let Some(ch) = g.chars.get_mut(chid) {
                ch.timer = 0;
                // Return from the void.
                if d.state == ConState::Playing && ch.was_in_room != NOWHERE {
                    let was_in = ch.was_in_room;
                    if ch.in_room != NOWHERE {
                        crate::handler::char_from_room(g, chid);
                    }
                    let target = if (was_in as usize) < g.world.rooms.len() { was_in } else { 0 };
                    crate::handler::char_to_room(g, chid, target);
                    g.ch_mut(chid).was_in_room = NOWHERE;
                    crate::comm::act(g, b"$n has returned.", true, Some(chid), None, None, crate::comm::TO_ROOM);
                }
                if let Some(ch) = g.chars.get_mut(chid) {
                    ch.wait = 1;
                }
            }
        }
        let Some(d) = g.descriptors.get_mut(di) else { continue };
        d.has_prompt = false;

        if d.paging() {
            // Pager input.
            let chid = d.character;
            let (pl, sw, compact) = match chid.and_then(|c| g.try_ch(c)) {
                Some(ch) if !ch.is_npc() => {
                    (ch.ps().page_length, ch.ps().screen_width, ch.prf(flags::PRF_COMPACT))
                }
                _ => (22, 80, false),
            };
            let allowed = crate::comm::color_allowed_for_desc(g, di);
            g.descriptors.show_string(di, &comm, pl, sw, compact, allowed);
        } else if g.descriptors.get(di).is_some_and(|d| d.editing.is_some()) {
            editor_input(g, di, &comm);
        } else if g.descriptors.get(di).is_some_and(|d| d.state != ConState::Playing) {
            crate::login::nanny(g, di, &comm);
        } else if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
            if aliased {
                if let Some(d) = g.descriptors.get_mut(di) {
                    d.has_prompt = true;
                }
                crate::interpreter::command_interpreter(g, chid, &comm);
            } else if crate::interpreter::perform_alias(g, di, &mut comm) {
                // Complex alias queued expansions: run the first now.
                if let Some(d) = g.descriptors.get_mut(di) {
                    if let Some((first, _)) = d.input.pop_front() {
                        crate::interpreter::command_interpreter(g, chid, &first);
                    }
                }
            } else {
                crate::interpreter::command_interpreter(g, chid, &comm);
            }
        }
    }

    // Step 9-10: output + prompts.
    let mut dead: Vec<usize> = Vec::new();
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        let has_output = !d.output.is_empty();
        if has_output {
            let (compact, playing_pc) = match d.character.and_then(|c| g.try_ch(c)) {
                Some(ch) if d.state == ConState::Playing && !ch.is_npc() => {
                    (ch.prf(flags::PRF_COMPACT), true)
                }
                _ => (false, false),
            };
            let prompt = make_prompt(g, di);
            match g.descriptors.process_output(di, compact, playing_pc, &prompt) {
                Ok(()) => {
                    if let Some(d) = g.descriptors.get_mut(di) {
                        d.has_prompt = true;
                    }
                }
                Err(()) => dead.push(di),
            }
        }
    }
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if !d.has_prompt && !dead.contains(&di) {
            let prompt = make_prompt(g, di);
            let Some(d) = g.descriptors.get_mut(di) else { continue };
            if d.write_direct(&prompt).is_err() {
                dead.push(di);
            } else {
                d.has_prompt = true;
            }
        }
    }
    // Step 11: closes.
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if matches!(d.state, ConState::Close | ConState::Disconnect) && !dead.contains(&di) {
            dead.push(di);
        }
    }
    for di in dead {
        close_socket(g, di);
    }

    // Step 12: heartbeat.
    g.pulse += 1;
    heartbeat(g);
}

fn editor_input(g: &mut Game, di: usize, line: &[u8]) {
    let outcome = {
        let Some(d) = g.descriptors.get_mut(di) else { return };
        let Some(session) = d.editing.as_mut() else { return };
        let (action, msgs, paged) =
            mud_net::editor::editor_add_line(&mut session.buf, line, true, false);
        (action, msgs, paged)
    };
    let (action, msgs, paged) = outcome;
    for m in msgs {
        crate::comm::write_to_desc(g, di, &m);
    }
    // improved-edit.c reaches page_string() in exactly two places, the two
    // buffer listings; everything else it says goes straight out. That
    // matters twice over: a listing longer than a page is paged rather than
    // dumped, and show_string's last page carries the `\tn` Welcor added to
    // stop colour bleeding, which a direct write does not.
    if let Some(listing) = paged {
        crate::act::informative::page_string_desc(g, di, &listing);
    }
    use mud_net::editor::EditorAction;
    match action {
        EditorAction::Save | EditorAction::Abort => {
            let state = g.descriptors.get(di).map(|d| d.state);
            let session = g.descriptors.get_mut(di).and_then(|d| d.editing.take());
            let Some(session) = session else { return };
            if state == Some(ConState::PlrDesc) {
                // exdesc_string_cleanup.
                let chid = g.descriptors.get(di).and_then(|d| d.character);
                if let Some(chid) = chid {
                    if action == EditorAction::Save {
                        g.ch_mut(chid).description = session.buf.buf;
                    } else {
                        crate::comm::write_to_desc(g, di, b"Description aborted.\r\n");
                        g.ch_mut(chid).description = session.backstr;
                    }
                }
                let menu = g.config.menu.clone();
                crate::comm::write_to_desc(g, di, &menu);
                if let Some(d) = g.descriptors.get_mut(di) {
                    d.state = ConState::Menu;
                }
            } else if state == Some(ConState::Playing) {
                playing_string_cleanup(g, di, session, action == EditorAction::Save);
            } else {
                // The OLC half of string_add's cleanup table.
                // On save the field takes the buffer; on abort it takes
                // d->backstr back.
                let saved = action == EditorAction::Save;
                let text = if saved { session.buf.buf } else { session.backstr };
                crate::olc::string_cleanup(g, di, text, saved);
            }
            // Common post cleanup code.
            if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                if g.try_ch(chid).is_some_and(|c| !c.is_npc()) {
                    let a = &mut g.ch_mut(chid).act;
                    a.remove(flags::PLR_BUG);
                    a.remove(flags::PLR_IDEA);
                    a.remove(flags::PLR_TYPO);
                    a.remove(flags::PLR_MAILING);
                    a.remove(flags::PLR_WRITING);
                }
            }
        }
        _ => {}
    }
}

/// playing_string_cleanup: mail, boards and IBT.
fn playing_string_cleanup(
    g: &mut Game,
    di: usize,
    session: mud_net::descriptor::EditSession,
    saved: bool,
) {
    let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let text = session.buf.buf;

    // do_write's note: the editor writes into the object field, so it
    // writes through it — including on abort, which falls into string_add's
    // fallthrough arm: logs the SYSERR below and leaves
    // the partial text in place rather than restoring d->backstr.
    if let Some(oid) = session.note_obj {
        if g.try_obj(oid).is_some() {
            g.obj_mut(oid).action_description = text.clone();
        }
    }
    if !saved {
        g.log("SYSERR: string_add: Aborting write from unknown origin.".to_string());
    }

    if g.try_ch(chid).is_some_and(|c| c.plr(flags::PLR_MAILING)) {
        if saved && text.is_some() {
            let from = g.ch(chid).idnum;
            crate::mail::store_mail(g, session.mail_to, from, text.clone().unwrap_or_default());
            crate::comm::write_to_desc(g, di, b"Message sent!\r\n");
            crate::mail::notify_if_playing(g, chid, session.mail_to);
        } else {
            crate::comm::write_to_desc(g, di, b"Mail aborted.\r\n");
        }
    }

    // We have no way of knowing which slot the post was sent to, so we can
    // only give the message.
    if session.mail_to >= crate::boards::BOARD_MAGIC {
        let board = (session.mail_to - crate::boards::BOARD_MAGIC) as usize;
        crate::boards::board_finish_write(g, chid, session.str_slot, board, text.clone());
        if !saved {
            crate::comm::write_to_desc(g, di, b"Post not aborted, use REMOVE <post #>.\r\n");
        }
    }

    for (flag, mode, label) in [
        (flags::PLR_IDEA, crate::interpreter::SCMD_IDEA, &b"Idea"[..]),
        (flags::PLR_BUG, crate::interpreter::SCMD_BUG, &b"Bug"[..]),
        (flags::PLR_TYPO, crate::interpreter::SCMD_TYPO, &b"Typo"[..]),
    ] {
        if !g.try_ch(chid).is_some_and(|c| c.plr(flag)) {
            continue;
        }
        if saved && text.is_some() {
            crate::ibt::ibt_finish_write(g, mode, text.clone());
            let mut m = label.to_vec();
            m.extend_from_slice(b" saved!\r\n");
            crate::comm::write_to_desc(g, di, &m);
            crate::ibt::save_ibt_file(g, mode);
        } else {
            let mut m = label.to_vec();
            m.extend_from_slice(b" aborted!\r\n");
            crate::comm::write_to_desc(g, di, &m);
            crate::ibt::clean_ibt_list(g, mode);
        }
    }
}

/// heartbeat, stage-2 cadences.
fn heartbeat(g: &mut Game) {
    let pulse = g.pulse;

    // Mud events (ePROTOCOLS) — event_process runs each pulse.
    let due: Vec<EventKind> = {
        let mut due: Vec<MudEvent> = Vec::new();
        g.events.retain(|e| {
            if e.fire_at <= pulse {
                due.push(*e);
                false
            } else {
                true
            }
        });
        // event_process pops the bucket head-first: ascending key, and
        // newest-first among equal keys (queue_enq inserts equal keys at
        // the front — see MudEvent::seq).
        due.sort_by(|a, b| a.fire_at.cmp(&b.fire_at).then(b.seq.cmp(&a.seq)));
        due.into_iter().map(|e| e.kind).collect::<Vec<_>>()
    };
    for kind in due {
        match kind {
            EventKind::Protocols { desc } => {
                if g.descriptors.get(desc).is_some_and(|d| d.state == ConState::GetProtocol) {
                    crate::login::get_protocols_event(g, desc);
                }
            }
            EventKind::Whirlwind { ch } => {
                if let Some(delay) = crate::act::offensive::event_whirlwind(g, ch) {
                    g.queue_event(delay, EventKind::Whirlwind { ch });
                }
            }
            EventKind::TrigWait { go, iid, event_id } => {
                crate::dg::driver::trig_wait_event(g, go, iid, event_id);
            }
            EventKind::SplDarkness { room } => {
                // event_countdown.
                if (room as usize) < g.world.rooms.len() {
                    g.world.rooms[room as usize].room_flags[flags::ROOM_DARK / 32] &=
                        !(1 << (flags::ROOM_DARK % 32));
                    crate::comm::send_to_room(g, room, b"The dark shroud disappates.\r\n");
                }
            }
        }
    }

    if pulse % PULSE_DG_SCRIPT == 0 {
        crate::dg::triggers::script_trigger_check(g);
    }
    // EVERY second: msdp_update + the tick countdown.
    if pulse % 10 == 0 {
        msdp_update(g);
        g.next_tick -= 1;
    }
    if pulse % PULSE_ZONE == 0 {
        crate::db::zone_update(g);
    }
    if pulse % PULSE_IDLEPWD == 0 {
        check_idle_passwords(g);
    }
    if pulse % PULSE_MOBILE == 0 {
        crate::mobact::mobile_activity(g);
    }
    if pulse % PULSE_VIOLENCE == 0 {
        crate::fight::perform_violence(g);
    }
    if pulse % (SECS_PER_MUD_HOUR * PASSES_PER_SEC) == 0 {
        // Tick!.
        g.next_tick = SECS_PER_MUD_HOUR as i32;
        weather_and_time_tick(g);
        crate::dg::triggers::check_time_triggers(g);
        crate::magic::affect_update(g);
        crate::limits::point_update(g);
        crate::quest::check_timed_quests(g);
    }
    if g.config.auto_save && pulse % PULSE_AUTOSAVE == 0 {
        // Crash_save_all every autosave_time minutes.
        g.autosave_minutes += 1;
        if g.autosave_minutes >= g.config.autosave_time {
            g.autosave_minutes = 0;
            crash_save_all(g);
            crate::house::house_save_all(g);
        }
    }
    // record_usage (PULSE_USAGE): syslog-only, stage 8.
    if pulse % PULSE_TIMESAVE == 0 {
        save_mud_time(g);
    }
    crate::handler::extract_pending_chars(g);
}

fn msdp_update(g: &mut Game) {
    // Stage-2 MSDP: refresh core character variables for reporting clients.
    use mud_net::protocol::Var;
    let mut count = 0i64;
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.is_npc() {
            continue;
        }
        count += 1;
        let name = ch.get_name().to_vec();
        let (p, level, class, alignment, wimp, practices) = {
            (
                ch.points,
                ch.level,
                ch.class,
                ch.alignment,
                ch.ps().wimp_level,
                ch.ps().practices,
            )
        };
        let ac = crate::act::informative::compute_armor_class(g, chid) as i64;

        // The rest of the character sheet. These are advertised in the same
        // table as the vitals above and were never set either, so a client
        // that asked for them waited forever — indistinguishable from the
        // server not supporting MSDP at all, the same argument as the room
        // variables below.
        //
        // `aff_abils` is what the score sheet shows and `real_abils` holds the
        // scores without modifiers, which is exactly the split MSDP asks for
        // with its plain and `_PERM` pair. Exceptional strength lives in a
        // separate `str_add` that MSDP has no variable for, so STR reports the
        // whole number a player sees and nothing reports the fraction.
        let (aff, real) = {
            let ch = g.ch(chid);
            (ch.aff_abils, ch.real_abils)
        };

        // The same sum score prints, and cut off where score cuts it off.
        // score stops at LVL_IMMORT because immortal levels are not earned
        // with experience: above LVL_IMMORT level_exp returns a fixed
        // offset from EXP_MAX rather than a total anyone works toward, so
        // reporting it would tell a level 31 immortal they are two million
        // exp short of a level 32 that no amount of exp reaches. An immortal
        // is told their own total and nothing left to earn, which is what
        // score says by saying nothing at all.
        let (exp_max, exp_tnl) = if level < LVL_IMMORT {
            let needed = mud_data::tables::level_exp(class as i32, level as i32 + 1);
            (needed as i64, (needed - p.exp).max(0) as i64)
        } else {
            (p.exp as i64, 0)
        };

        // An array of the spells currently on the character, named the way
        // `stat` names them. One spell can hold several affects — a single
        // cast fills one per apply — so each is listed once rather than once
        // per slot. Bounded to MAX_INPUT_LENGTH.
        let affects: BStr = {
            let mut out: Vec<u8> = Vec::new();
            let mut seen: Vec<i16> = Vec::new();
            for af in &g.ch(chid).affected {
                if seen.contains(&af.spell) {
                    continue;
                }
                seen.push(af.spell);
                if out.len() >= 511 {
                    break;
                }
                let mut piece = vec![mud_net::telnet::MSDP_VAL];
                piece.extend_from_slice(crate::dg::misc::skill_name_b(af.spell as i32));
                piece.truncate(511 - out.len());
                out.extend_from_slice(&piece);
            }
            out
        };

        // Who the player is fighting, as a percentage rather than a raw count
        // — OPPONENT_HEALTH_MAX is the literal 100 that goes on the wire.
        // A mob edited down to zero max hit would divide by zero, so it
        // reports 0 instead.
        let opponent = g.ch(chid).fighting.and_then(|opp| {
            let o = g.try_ch(opp)?;
            let (hit, max_hit, olevel) = (o.points.hit, o.points.max_hit, o.level);
            let pct = if max_hit != 0 { (hit * 100) / max_hit } else { 0 };
            Some((pct as i64, olevel as i64, crate::handler::pers(g, chid, opp)))
        });

        let world_hours = g.time_info.hours as i64;
        let server_time = g.now;
        // Where the player is (B75). The variable table has always advertised
        // AREA_NAME / ROOM_EXITS / ROOM_NAME / ROOM_VNUM as reportable and
        // nothing ever set them, so a client that asked for them waited
        // forever and fell back to reading the room out of the scroll.
        let room = g.ch(chid).in_room;
        let here = if room == NOWHERE {
            None
        } else {
            let holy = g.ch(chid).prf(flags::PRF_HOLYLIGHT);
            let r = &g.world.rooms[room as usize];
            let room_name = r.name.clone().unwrap_or_default();
            let room_vnum = r.vnum as i64;
            let area_name = g.world.zones[r.zone as usize].name.clone().unwrap_or_default();
            // Exactly the exits do_auto_exits would show this character:
            // a mapper must not learn a door the player cannot see.
            let mut exits: BStr = Vec::new();
            for door in 0..crate::fight::dir_count(g) {
                let Some(exit) = r.dir_option[door].as_deref() else { continue };
                if exit.to_room == NOWHERE {
                    continue;
                }
                let closed = exit.exit_info & flags::EX_CLOSED != 0;
                let hidden = exit.exit_info & flags::EX_HIDDEN != 0;
                if closed && !g.config.display_closed_doors {
                    continue;
                }
                if hidden && !holy {
                    continue;
                }
                exits.push(mud_net::telnet::MSDP_VAR);
                exits.extend_from_slice(mud_data::tables::AUTOEXITS[door].as_bytes());
                exits.push(mud_net::telnet::MSDP_VAL);
                // A closed door is listed with an empty destination. do_exits
                // withholds the vnum and the room name behind a closed door
                // from everyone, a showvnums immortal included, and says only
                // that it is closed -- so the direction is reported, because
                // the player sees it in the autoexit line, and where it leads
                // is not. Opening the door restores the vnum.
                if !closed {
                    exits.extend_from_slice(
                        g.world.rooms[exit.to_room as usize].vnum.to_string().as_bytes(),
                    );
                }
            }
            Some((room_name, room_vnum, area_name, exits))
        };
        let Some(d) = g.descriptors.get_mut(di) else { continue };
        let empty = d.output.is_empty();
        let pr = &mut d.protocol;
        pr.set_string(Var::CHARACTER_NAME, &name);
        pr.set_number(Var::ALIGNMENT, alignment as i64);
        pr.set_number(Var::EXPERIENCE, p.exp as i64);
        pr.set_number(Var::HEALTH, p.hit as i64);
        pr.set_number(Var::HEALTH_MAX, p.max_hit as i64);
        pr.set_number(Var::LEVEL, level as i64);
        pr.set_string(
            Var::CLASS,
            crate::act::informative::PC_CLASS_TYPES.get(class as usize).copied().unwrap_or(b"Undefined"),
        );
        pr.set_number(Var::MANA, p.mana as i64);
        pr.set_number(Var::MANA_MAX, p.max_mana as i64);
        pr.set_number(Var::WIMPY, wimp as i64);
        pr.set_number(Var::MONEY, p.gold as i64);
        pr.set_number(Var::MOVEMENT, p.mov as i64);
        pr.set_number(Var::MOVEMENT_MAX, p.max_move as i64);
        pr.set_number(Var::AC, ac);

        pr.set_number(Var::HITROLL, p.hitroll as i64);
        pr.set_number(Var::DAMROLL, p.damroll as i64);
        pr.set_number(Var::PRACTICE, practices as i64);

        pr.set_number(Var::STR, aff.str_ as i64);
        pr.set_number(Var::INT, aff.intel as i64);
        pr.set_number(Var::WIS, aff.wis as i64);
        pr.set_number(Var::DEX, aff.dex as i64);
        pr.set_number(Var::CON, aff.con as i64);
        pr.set_number(Var::STR_PERM, real.str_ as i64);
        pr.set_number(Var::INT_PERM, real.intel as i64);
        pr.set_number(Var::WIS_PERM, real.wis as i64);
        pr.set_number(Var::DEX_PERM, real.dex as i64);
        pr.set_number(Var::CON_PERM, real.con as i64);

        pr.set_number(Var::EXPERIENCE_MAX, exp_max);
        pr.set_number(Var::EXPERIENCE_TNL, exp_tnl);
        pr.set_array(Var::AFFECTS, &affects);

        // RACE is left alone deliberately. It is the one advertised variable
        // with nothing behind it: the game has no race, only class, and
        // answering with an invented value would be worse than silence.
        pr.set_string(Var::SERVER_ID, mud_net::protocol::MUD_NAME);
        pr.set_number(Var::SERVER_TIME, server_time);
        pr.set_number(Var::WORLD_TIME, world_hours);

        match &opponent {
            Some((pct, olevel, oname)) => {
                pr.set_number(Var::OPPONENT_HEALTH, *pct);
                pr.set_number(Var::OPPONENT_HEALTH_MAX, 100);
                pr.set_number(Var::OPPONENT_LEVEL, *olevel);
                pr.set_string(Var::OPPONENT_NAME, oname);
            }
            None => {
                // Clear the values. HEALTH_MAX is deliberately left as it
                // was.
                pr.set_number(Var::OPPONENT_HEALTH, 0);
                pr.set_number(Var::OPPONENT_LEVEL, 0);
                pr.set_string(Var::OPPONENT_NAME, b"");
            }
        }

        if let Some((room_name, room_vnum, area_name, exits)) = here {
            pr.set_string(Var::ROOM_NAME, &room_name);
            pr.set_number(Var::ROOM_VNUM, room_vnum);
            pr.set_string(Var::AREA_NAME, &area_name);
            pr.set_table(Var::ROOM_EXITS, &exits);
        }
        mud_net::protocol::msdp_update_flush(pr, empty);
        g.descriptors.pump_protocol_out(di);
    }
    mud_net::protocol::mssp_set_players(count, g.now);
}

fn check_idle_passwords(g: &mut Game) {
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Password && d.state != ConState::GetName {
            continue;
        }
        let Some(d) = g.descriptors.get_mut(di) else { continue };
        if d.idle_tics == 0 {
            d.idle_tics = 1;
        } else {
            g.descriptors.echo_on(di);
            crate::comm::write_to_desc(g, di, b"\r\nTimed out... goodbye.\r\n");
            if let Some(d) = g.descriptors.get_mut(di) {
                d.state = ConState::Close;
            }
        }
    }
}

/// Crash_save_all: crashsave dirty players.
fn crash_save_all(g: &mut Game) {
    crate::objsave::crash_save_all(g);
}

fn weather_and_time_tick(g: &mut Game) {
    let tick1 = crate::gametime::another_hour(&mut g.time_info, &mut g.weather);
    let tick2 = crate::gametime::weather_change(&g.time_info.clone(), &mut g.weather, &mut g.rng);
    for msg in tick1.messages.iter().chain(tick2.messages.iter()) {
        crate::comm::send_to_outdoor(g, msg);
    }
}
