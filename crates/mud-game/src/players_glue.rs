//! Glue between the runtime `Char` and mud-world's PlayerFile DTO: the
//! save_char/load_char halves that touch game state
//! (affect strip/reapply, index upkeep), plus the stage-2 objsave minimum
//! (empty-inventory crash/rent files).

use mud_data::flags::{self, FlagSet};
use mud_data::ids::CharId;
use mud_data::types::*;
use mud_world::players::{self, PfAffect, PfAlias, PlayerFile};

use crate::ch::{Affect, Alias, Char, PlayerSpecials, DRUNK, HUNGER, THIRST};
use crate::game::{Game, PlayerIndexElement, PINDEX_DELETED, PINDEX_NODELETE, PINDEX_NOWIZLIST};

/// save_char: write the pfile from *unaffected* values
/// (strip affects/equipment aside, write, restore), update the index row.
pub fn save_char(g: &mut Game, chid: CharId) {
    if g.ch(chid).is_npc() || g.ch(chid).pfilepos < 0 {
        return;
    }
    let Some(name) = g.ch(chid).name.clone() else { return };

    // Refresh host + played time: only for a live
    // descriptor, and the clock only while it is in play.
    if let Some(di) = g.ch(chid).desc {
        let now = g.now;
        let (host, playing) = match g.descriptors.get(di) {
            Some(d) => (d.host.clone(), d.state == mud_data::types::ConState::Playing),
            None => (Vec::new(), false),
        };
        let ch = g.ch_mut(chid);
        if !host.is_empty() {
            ch.ps_mut().host = Some(host);
        }
        if playing {
            ch.time.played += (now - ch.time.logon) as i32;
            ch.time.logon = now;
        }
    }

    // Strip: unequip all, remove affects, remember both.
    let mut eq: [Option<mud_data::ids::ObjId>; NUM_WEARS] = [None; NUM_WEARS];
    for pos in 0..NUM_WEARS {
        if g.ch(chid).equipment[pos].is_some() {
            eq[pos] = crate::handler::unequip_char(g, chid, pos);
        }
    }
    let affects: Vec<Affect> = g.ch(chid).affected.clone();
    while !g.ch(chid).affected.is_empty() {
        crate::handler::affect_remove(g, chid, 0);
    }
    {
        let ch = g.ch_mut(chid);
        ch.aff_abils = ch.real_abils;
    }

    let pf = char_to_playerfile(g, chid, &affects);
    let bytes = players::save_char(&pf);
    if let Err(e) = players::write_pfile(&g.lib_dir, &name, &bytes) {
        g.log(format!("SYSERR: Couldn't save player file for {}: {}", String::from_utf8_lossy(&name), e));
    }

    // Restore affects and equipment.
    for af in affects {
        crate::handler::affect_to_char(g, chid, af);
    }
    for (pos, oid) in eq.iter().enumerate() {
        if let Some(oid) = *oid {
            crate::handler::equip_char(g, chid, oid, pos);
        }
    }

    // Index row upkeep.
    let (level, last, plr) = {
        let ch = g.ch(chid);
        (ch.level as i32, ch.time.logon, ch.act)
    };
    let mut flags_bits = 0;
    if plr.is_set(flags::PLR_DELETED) {
        flags_bits |= PINDEX_DELETED;
    }
    if plr.is_set(flags::PLR_NODELETE) || plr.is_set(flags::PLR_CRYO) {
        flags_bits |= PINDEX_NODELETE;
    }
    if plr.is_set(flags::PLR_FROZEN) || plr.is_set(flags::PLR_NOWIZLIST) {
        flags_bits |= PINDEX_NOWIZLIST;
    }
    let lower = name.to_ascii_lowercase();
    let mut changed = false;
    if let Some(row) = g.player_table.iter_mut().find(|p| p.name == lower) {
        if row.level != level || row.last != last || row.flags != flags_bits {
            row.level = level;
            row.last = last;
            row.flags = flags_bits;
            changed = true;
        }
    }
    if changed {
        save_player_index(g);
    }
}

fn char_to_playerfile(g: &Game, chid: CharId, affects: &[Affect]) -> PlayerFile {
    let ch = g.ch(chid);
    let ps = ch.ps();
    PlayerFile {
        name: ch.name.clone(),
        passwd: ch.passwd.clone(),
        title: ch.title.clone(),
        description: ch.description.clone(),
        poofin: ps.poofin.clone(),
        poofout: ps.poofout.clone(),
        sex: ch.sex as i32,
        class: ch.class as i32,
        level: ch.level as i32,
        idnum: ch.idnum,
        birth: ch.time.birth,
        played: ch.time.played,
        last_logon: ch.time.logon,
        last_motd: ps.last_motd,
        last_news: ps.last_news,
        host: ps.host.clone(),
        height: ch.height as i32,
        weight: ch.weight as i32,
        alignment: ch.alignment,
        plr_flags: ch.act.0,
        aff_flags: ch.affected_by.0,
        prf_flags: ps.pref.0,
        saving_throws: [
            ch.apply_saving_throw[0] as i32,
            ch.apply_saving_throw[1] as i32,
            ch.apply_saving_throw[2] as i32,
            ch.apply_saving_throw[3] as i32,
            ch.apply_saving_throw[4] as i32,
        ],
        wimpy: ps.wimp_level,
        freeze_level: ps.freeze_level as i32,
        invis_level: ps.invis_level as i32,
        load_room: if ps.load_room == NOWHERE { NOWHERE as i32 } else { ps.load_room as i32 },
        bad_pws: ps.bad_pws as i32,
        practices: ps.practices,
        hunger: ps.conditions[HUNGER] as i32,
        thirst: ps.conditions[THIRST] as i32,
        drunk: ps.conditions[DRUNK] as i32,
        hit: ch.points.hit,
        max_hit: ch.points.max_hit,
        mana: ch.points.mana,
        max_mana: ch.points.max_mana,
        mov: ch.points.mov,
        max_move: ch.points.max_move,
        str_: ch.real_abils.str_ as i32,
        str_add: ch.real_abils.str_add as i32,
        intel: ch.real_abils.intel as i32,
        wis: ch.real_abils.wis as i32,
        dex: ch.real_abils.dex as i32,
        con: ch.real_abils.con as i32,
        cha: ch.real_abils.cha as i32,
        ac: ch.points.armor,
        gold: ch.points.gold,
        bank: ch.points.bank_gold,
        exp: ch.points.exp,
        hitroll: ch.points.hitroll as i32,
        damroll: ch.points.damroll as i32,
        olc_zone: ps.olc_zone,
        page_length: ps.page_length,
        screen_width: ps.screen_width,
        questpoints: ps.questpoints,
        quest_counter: ps.quest_counter,
        current_quest: ps.current_quest as i32,
        completed_quests: ps.completed_quests.clone(),
        triggers: ch
            .script
            .as_deref()
            .map(|sc| {
                sc.trig_list
                    .iter()
                    .map(|t| g.world.triggers[t.nr as usize].vnum)
                    .collect()
            })
            .unwrap_or_default(),
        skills: {
            // Written only when level < LVL_IMMORT; the writer handles the
            // gate — pass all nonzero skills.
            let mut v = Vec::new();
            for (num, val) in ps.skills.iter().enumerate().skip(1) {
                if *val != 0 {
                    v.push((num as i32, *val as i32));
                }
            }
            v
        },
        affects: affects
            .iter()
            .map(|a| PfAffect {
                spell: a.spell as i32,
                duration: a.duration as i32,
                modifier: a.modifier as i32,
                location: a.location as i32,
                bitvector: a.bitvector.0,
            })
            .collect(),
        aliases: ps
            .aliases
            .iter()
            .map(|a| PfAlias { alias: a.alias.clone(), replacement: a.replacement.clone(), type_: a.type_ })
            .collect(),
        // save_char_vars_ascii: globals whose name doesn't start with '-'.
        vars: ch
            .script
            .as_deref()
            .map(|sc| {
                sc.global_vars
                    .iter()
                    .filter(|v| v.name.first() != Some(&b'-'))
                    .map(|v| mud_world::players::PfVar {
                        name: v.name.clone(),
                        context: v.context,
                        value: v.value.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// load_char into an existing shell. Returns the player
/// index slot when the pfile existed. Affects are applied via affect_to_char
/// and affect_total runs at the end, in that order.
///
/// Note what it does **not** do: stamp `pfilepos`. That is left to the
/// four call sites that intend to write the character back, which is exactly
/// how `stat file` avoids saving (and deleting the crash file of) a player it
/// only wanted to look at.
pub fn load_char_into(g: &mut Game, chid: CharId, name: &[u8]) -> Option<usize> {
    // The player index is checked first: without an index
    // row, the pfile is unreachable and the name is treated as new.
    let lower = name.to_ascii_lowercase();
    let Some(player_i) = g.player_table.iter().position(|p| p.name == lower) else {
        return None;
    };
    let loaded = players::load_char(&g.lib_dir, name);
    let Some((pf, syserrs)) = loaded else {
        let path = players::get_filename(players::FileKind::Plr, name)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        g.mudlog(
            crate::game::MudlogKind::Nrm,
            LVL_GOD,
            true,
            &format!("SYSERR: Couldn't open player file {}", path),
        );
        return None;
    };
    for line in syserrs {
        g.log(line);
    }
    {
        let ch = g.ch_mut(chid);
        let di = ch.desc;
        let mut fresh = Char {
            player_specials: Some(Box::new(PlayerSpecials::default())),
            desc: di,
            ..Default::default()
        };
        fresh.name = pf.name.clone();
        fresh.passwd = pf.passwd.clone();
        fresh.title = pf.title.clone();
        fresh.description = pf.description.clone();
        fresh.sex = pf.sex.clamp(0, 255) as u8;
        fresh.class = pf.class.clamp(-1, 127) as i8;
        fresh.level = pf.level.clamp(0, 255) as u8;
        fresh.idnum = pf.idnum;
        fresh.time.birth = pf.birth;
        fresh.time.played = pf.played;
        fresh.time.logon = pf.last_logon;
        fresh.height = pf.height.clamp(0, 255) as u8;
        fresh.weight = pf.weight.clamp(0, 255) as u8;
        fresh.alignment = pf.alignment;
        fresh.act = FlagSet::from_words(pf.plr_flags);
        fresh.affected_by = FlagSet::from_words(pf.aff_flags);
        for i in 0..5 {
            fresh.apply_saving_throw[i] = pf.saving_throws[i] as i16;
        }
        fresh.points.hit = pf.hit;
        fresh.points.max_hit = pf.max_hit;
        fresh.points.mana = pf.mana;
        fresh.points.max_mana = pf.max_mana;
        fresh.points.mov = pf.mov;
        fresh.points.max_move = pf.max_move;
        fresh.real_abils.str_ = pf.str_.clamp(-128, 127) as i8;
        fresh.real_abils.str_add = pf.str_add.clamp(-128, 127) as i8;
        fresh.real_abils.intel = pf.intel.clamp(-128, 127) as i8;
        fresh.real_abils.wis = pf.wis.clamp(-128, 127) as i8;
        fresh.real_abils.dex = pf.dex.clamp(-128, 127) as i8;
        fresh.real_abils.con = pf.con.clamp(-128, 127) as i8;
        fresh.real_abils.cha = pf.cha.clamp(-128, 127) as i8;
        fresh.aff_abils = fresh.real_abils;
        fresh.points.armor = pf.ac;
        fresh.points.gold = pf.gold;
        fresh.points.bank_gold = pf.bank;
        fresh.points.exp = pf.exp;
        fresh.points.hitroll = pf.hitroll.clamp(-128, 127) as i8;
        fresh.points.damroll = pf.damroll.clamp(-128, 127) as i8;
        {
            let ps = fresh.ps_mut();
            ps.poofin = pf.poofin.clone();
            ps.poofout = pf.poofout.clone();
            ps.last_motd = pf.last_motd;
            ps.last_news = pf.last_news;
            ps.host = pf.host.clone();
            ps.wimp_level = pf.wimpy;
            ps.freeze_level = pf.freeze_level.clamp(-128, 127) as i8;
            ps.invis_level = pf.invis_level.clamp(-32768, 32767) as i16;
            ps.load_room = pf.load_room.clamp(0, 65535) as Idx;
            ps.bad_pws = pf.bad_pws.clamp(0, 255) as u8;
            ps.practices = pf.practices;
            ps.conditions[HUNGER] = pf.hunger as i16;
            ps.conditions[THIRST] = pf.thirst as i16;
            ps.conditions[DRUNK] = pf.drunk as i16;
            ps.olc_zone = pf.olc_zone;
            ps.page_length = pf.page_length;
            ps.screen_width = pf.screen_width;
            ps.questpoints = pf.questpoints;
            ps.quest_counter = pf.quest_counter;
            ps.current_quest = pf.current_quest.clamp(0, 65535) as Idx;
            ps.num_completed_quests = pf.completed_quests.len() as i32;
            ps.completed_quests = pf.completed_quests.clone();
            ps.pref = FlagSet::from_words(pf.prf_flags);
            for (num, val) in &pf.skills {
                if (1..=MAX_SKILLS as i32).contains(num) {
                    ps.skills[*num as usize] = (*val).clamp(-128, 127) as i8;
                }
            }
            for a in &pf.aliases {
                ps.aliases.insert(
                    0,
                    Alias { alias: a.alias.clone(), replacement: a.replacement.clone(), type_: a.type_ },
                );
            }
        }
        *ch = fresh;
    }
    // Trig: lines attach instances when script_players is on.
    if g.config.script_players {
        for &tv in &pf.triggers {
            if let Some(&rnum) = g.world.trig_map.get(&tv) {
                if let Some(t) = crate::dg::read_trigger(g, rnum) {
                    let go = crate::dg::GoId::Char(chid);
                    crate::dg::add_trigger_at(g.ensure_script(go), t, -1);
                }
            }
        }
    }
    // Vars: -> read_saved_vars_ascii: skipped entirely if
    // SCRIPT(ch) already exists (the Vars-after-Trig desync lives in the
    // parser, which stops consuming payload lines in that case).
    if !pf.vars.is_empty() && g.ch(chid).script.is_none() {
        let go = crate::dg::GoId::Char(chid);
        let vars = pf.vars.clone();
        let sc = g.ensure_script(go);
        for v in &vars {
            crate::dg::add_var(&mut sc.global_vars, &v.name, &v.value, v.context);
        }
    }
    // Spell affects re-applied: affect_to_char per entry.
    for a in &pf.affects {
        let af = Affect {
            spell: a.spell as i16,
            duration: a.duration as i16,
            modifier: a.modifier as i8,
            location: a.location as u8,
            bitvector: FlagSet::from_words(a.bitvector),
        };
        crate::handler::affect_to_char(g, chid, af);
    }
    crate::handler::affect_total(g, chid);
    // Immortal overrides.
    if g.ch(chid).level >= LVL_IMMORT {
        let ch = g.ch_mut(chid);
        {
            let ps = ch.ps_mut();
            for s in ps.skills.iter_mut() {
                *s = 100;
            }
            ps.conditions = [-1; 3];
        }
    }
    Some(player_i)
}

/// create_entry: append an index slot (id filled by
/// init_char).
pub fn create_entry(g: &mut Game, name: &[u8]) -> usize {
    let lower = name.to_ascii_lowercase();
    if let Some(i) = g.player_table.iter().position(|p| p.name == lower) {
        return i;
    }
    g.player_table.push(PlayerIndexElement { name: lower, id: 0, level: 0, flags: 0, last: 0 });
    g.player_table.len() - 1
}

/// The "load a player off disk into a scratch character" idiom
/// (`stat file`, `last <name>`, `show player`, `set file`): the character
/// exists only in the arena — never in character_list, never in a room — and
/// must be released with [`free_offline_char`].
pub fn load_char_offline(g: &mut Game, name: &[u8]) -> Option<CharId> {
    let shell = Char {
        player_specials: Some(Box::new(PlayerSpecials::default())),
        idnum: -1,
        ..Default::default()
    };
    let chid = g.chars.insert(shell);
    if load_char_into(g, chid, name).is_none() {
        g.chars.remove(chid);
        return None;
    }
    Some(chid)
}

/// free_char for an offline shell: drop any script the load attached and
/// take it back out of the arena.
pub fn free_offline_char(g: &mut Game, chid: CharId) {
    if g.try_ch(chid).is_some() {
        crate::dg::extract_script(g, crate::dg::GoId::Char(chid));
        g.chars.remove(chid);
    }
}

pub fn save_player_index(g: &mut Game) {
    let entries: Vec<players::IndexEntry> = g
        .player_table
        .iter()
        .map(|p| players::IndexEntry {
            name: p.name.clone(),
            id: p.id,
            level: p.level,
            flags: p.flags,
            last: p.last,
        })
        .collect();
    if let Err(e) = players::save_index(&g.lib_dir, &entries) {
        g.log(format!("SYSERR: Could not write player index file: {}", e));
    }
}

/// get_name_by_id: index lookup, deleted rows skipped.
pub fn get_name_by_id(g: &Game, id: i64) -> Option<Vec<u8>> {
    g.player_table
        .iter()
        .find(|p| p.id == id && p.flags & crate::game::PINDEX_DELETED == 0)
        .map(|p| p.name.clone())
}

pub fn get_id_by_name(g: &Game, name: &[u8]) -> Option<i64> {
    let lower = name.to_ascii_lowercase();
    g.player_table
        .iter()
        .find(|p| p.name == lower && p.flags & crate::game::PINDEX_DELETED == 0)
        .map(|p| p.id)
}

/// remove_player: unlink files, drop the row.
pub fn remove_player_by_name(g: &mut Game, name: &[u8]) {
    let lower = name.to_ascii_lowercase();
    for kind in [players::FileKind::Plr, players::FileKind::Objs, players::FileKind::Vars, players::FileKind::Text] {
        if let Some(rel) = players::get_filename(kind, &lower) {
            let _ = std::fs::remove_file(g.lib_dir.join(rel));
        }
    }
    g.player_table.retain(|p| p.name != lower);
    save_player_index(g);
}

/// Crash_crashsave — stage-2 minimum: header +
/// terminator (inventories cannot exist yet); clears PLR_CRASH.
pub fn crash_crashsave(g: &mut Game, chid: CharId) {
    if g.ch(chid).is_npc() {
        return;
    }
    write_rent_file(g, chid, 1, 0); // RENT_CRASH
    g.ch_mut(chid).act.remove(flags::PLR_CRASH);
}

/// Crash_rentsave under free rent (cost 0). Writes the rent file and then
/// EXTRACTS every worn and carried object
/// (Crash_extract_objs) — the quitting character leaves nothing behind, so
/// extract_char has nothing to dump in the room. The object records
/// themselves (and Crash_load to restore them) are stage 7; until then the
/// world-state half of the contract is what matters — a quit must not
/// strew gear on the temple floor.
pub fn crash_rentsave(g: &mut Game, chid: CharId) {
    if g.ch(chid).is_npc() {
        return;
    }
    write_rent_file(g, chid, 2, 0); // RENT_RENTED
    for pos in 0..NUM_WEARS {
        if let Some(oid) = g.ch(chid).equipment[pos] {
            crate::handler::extract_obj(g, oid);
        }
    }
    let carried: Vec<_> = g.ch(chid).carrying.clone();
    for oid in carried {
        if g.try_obj(oid).is_some() {
            crate::handler::extract_obj(g, oid);
        }
    }
}

/// Crash_delete_crashfile: delete the objs file only
/// when its rentcode is RENT_CRASH.
pub fn crash_delete_crashfile(g: &mut Game, chid: CharId) {
    let Some(name) = g.try_ch(chid).and_then(|c| c.name.clone()) else { return };
    let Some(rel) = players::get_filename(players::FileKind::Objs, &name) else { return };
    let path = g.lib_dir.join(rel);
    let Ok(data) = std::fs::read(&path) else { return };
    let first = data.split(|c| *c == b'\n').next().unwrap_or(b"");
    let rentcode = crate::handler::atoi(first);
    if rentcode == 1 {
        let _ = std::fs::remove_file(&path);
    }
}

fn write_rent_file(g: &mut Game, chid: CharId, rentcode: i32, cost_per_day: i32) {
    let Some(name) = g.ch(chid).name.clone() else { return };
    let Some(rel) = players::get_filename(players::FileKind::Objs, &name) else { return };
    let (gold, bank) = {
        let p = &g.ch(chid).points;
        (p.gold, p.bank_gold)
    };
    // objsave_write_rentcode: literal \r\n on the header line.
    let mut out = format!("{} {} {} {} {} {}\r\n", rentcode, g.now, cost_per_day, gold, bank, 0).into_bytes();
    out.extend_from_slice(b"$~\n");
    let path = g.lib_dir.join(rel);
    if let Err(e) = std::fs::write(&path, &out) {
        g.log(format!("SYSERR: Couldn't write rent file {}: {}", path.display(), e));
    }
}

/// read_saved_vars: legacy lib/plrvars/<name>.mem
/// fallback. Creates SCRIPT(ch) unconditionally (the "every logged-in PC
/// has a script container" invariant), then loads lines if the file exists.
pub fn read_saved_vars(g: &mut Game, chid: CharId) {
    if g.ch(chid).script.is_some() {
        return;
    }
    let Some(name) = g.ch(chid).name.clone() else { return };
    g.ensure_script(crate::dg::GoId::Char(chid));

    let Some(rel) = players::get_filename(players::FileKind::Vars, &name) else { return };
    let path = g.lib_dir.join(rel);
    let Ok(data) = std::fs::read(&path) else {
        let pname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        g.log(format!("{} had no variable file", pname));
        return;
    };
    for line in data.split(|&b| b == b'\n') {
        let line = if line.last() == Some(&b'\r') { &line[..line.len() - 1] } else { line };
        if line.is_empty() {
            continue;
        }
        let (varname, rest) = crate::interpreter::any_one_arg(line);
        let (context_str, rest2) = crate::interpreter::any_one_arg(rest);
        let value = crate::interpreter::skip_spaces(rest2).to_vec();
        let context = crate::dg::atoi64(&context_str);
        if let Some(sc) = g.script_of_mut(crate::dg::GoId::Char(chid)) {
            crate::dg::add_var(&mut sc.global_vars, &varname, &value, context);
        }
    }
}
