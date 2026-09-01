//! The mobile editor.
//!
//! The numeric-response guard at the head of `medit_parse` looked like a
//! validation and is not enough on its own
//! can only be true for a lone '-', so "abc" sailed through as atoi == 0
//! and would set the field to its clamp minimum. That is **B32**: a
//! mistyped stat re-prompts instead.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{ACTION_BITS, AFFECTED_BITS, GENDERS, POSITION_TYPES};
use mud_data::types::*;
use mud_world::model::MobProto;

use crate::act::informative::{column_list, sprintbitarray};
use crate::act::other::count_color_chars;
use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::interpreter::{is_number, two_arguments};
use crate::olc::genmob::{add_mobile, delete_mobile, save_mobiles};
use crate::olc::trigedit::{dg_olc_script_copy, dg_script_menu, SCRIPT_MAIN_MENU};
use crate::olc::{
    can_edit_zone, clear_screen, genolc_checkstring, get_char_colors, send_cannot_edit, str_udup,
    OlcData, StrTarget, CLEANUP_ALL, MAX_MOB_DESC,
};

/// Submodes of MEDIT connectedness.
pub const MEDIT_MAIN_MENU: i32 = 0;
pub const MEDIT_KEYWORD: i32 = 1;
pub const MEDIT_S_DESC: i32 = 2;
pub const MEDIT_L_DESC: i32 = 3;
pub const MEDIT_D_DESC: i32 = 4;
pub const MEDIT_NPC_FLAGS: i32 = 5;
pub const MEDIT_AFF_FLAGS: i32 = 6;
pub const MEDIT_CONFIRM_SAVESTRING: i32 = 7;
pub const MEDIT_STATS_MENU: i32 = 8;
/// Everything above this is a numerical response.
pub const MEDIT_NUMERICAL_RESPONSE: i32 = 10;
pub const MEDIT_SEX: i32 = 11;
pub const MEDIT_HITROLL: i32 = 12;
pub const MEDIT_DAMROLL: i32 = 13;
pub const MEDIT_NDD: i32 = 14;
pub const MEDIT_SDD: i32 = 15;
pub const MEDIT_NUM_HP_DICE: i32 = 16;
pub const MEDIT_SIZE_HP_DICE: i32 = 17;
pub const MEDIT_ADD_HP: i32 = 18;
pub const MEDIT_AC: i32 = 19;
pub const MEDIT_EXP: i32 = 20;
pub const MEDIT_GOLD: i32 = 21;
pub const MEDIT_POS: i32 = 22;
pub const MEDIT_DEFAULT_POS: i32 = 23;
pub const MEDIT_ATTACK: i32 = 24;
pub const MEDIT_LEVEL: i32 = 25;
pub const MEDIT_ALIGNMENT: i32 = 26;
pub const MEDIT_DELETE: i32 = 27;
pub const MEDIT_COPY: i32 = 28;
pub const MEDIT_STR: i32 = 29;
pub const MEDIT_INT: i32 = 30;
pub const MEDIT_WIS: i32 = 31;
pub const MEDIT_DEX: i32 = 32;
pub const MEDIT_CON: i32 = 33;
pub const MEDIT_CHA: i32 = 34;
pub const MEDIT_PARA: i32 = 35;
pub const MEDIT_ROD: i32 = 36;
pub const MEDIT_PETRI: i32 = 37;
pub const MEDIT_BREATH: i32 = 38;
pub const MEDIT_SPELL: i32 = 39;

/// MAX_MOB_GOLD / MAX_MOB_EXP.
const MAX_MOB_GOLD: i32 = 100000;
const MAX_MOB_EXP: i32 = 150000;

fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

pub fn do_oasis_medit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    let mut number: i32 = NOBODY as i32;
    let mut save = false;

    if buf1.is_empty() {
        send_to_char(g, chid, b"Specify a mobile VNUM to edit.\r\n");
        return;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) != std::cmp::Ordering::Equal {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
        save = true;
        if is_number(&buf2) {
            number = atoi(&buf2);
        } else {
            let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
            // Note: medit tests `> 0` where redit tests `!= NOWHERE`.
            if olc_zone > 0 {
                number = match g.world.real_zone(olc_zone as Idx) {
                    None => NOWHERE as i32,
                    // The zone below is resolved with real_zone, so
                    // this has to be the zone NUMBER. Handing it
                    // a vnum here would stop any
                    // argument-less save from ever resolving.
                    Some(zlok) => g.world.zones[zlok as usize].number as i32,
                };
            }
        }
        if number == NOWHERE as i32 {
            send_to_char(g, chid, b"Save which zone?\r\n");
            return;
        }
    }

    if number == NOBODY as i32 {
        number = atoi(&buf1);
    }
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That mobile VNUM can't exist.\r\n");
        return;
    }

    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Medit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = g
            .descriptors
            .get(other)
            .and_then(|d| d.character)
            .map(|c| g.ch(c).get_name().to_vec())
            .unwrap_or_else(|| b"(null)".to_vec());
        let mut msg = b"That mobile is currently being edited by ".to_vec();
        msg.extend_from_slice(&who);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis_medit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }
    let mut olc = OlcData::new();

    let znum = if save {
        g.world.real_zone(number as Idx).map(|z| z as i32)
    } else {
        crate::dg::mobcmd::real_zone_by_thing(g, number).map(|z| z as i32)
    };
    let Some(znum) = znum else {
        send_to_char(g, chid, b"Sorry, there is no zone for that number!\r\n");
        return;
    };
    olc.zone_num = znum;

    if !can_edit_zone(g, chid, znum) {
        let zvnum = g.world.zones[znum as usize].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }

    if save {
        let zvnum = g.world.zones[znum as usize].number;
        send_to_char(
            g,
            chid,
            format!("Saving all mobiles in zone {}.\r\n", zvnum).as_bytes(),
        );
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves mobile info for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_mobiles(g, Some(znum as usize));
        return;
    }

    olc.number = number;

    match g.world.real_mobile(number as Idx) {
        None => medit_setup_new(&mut olc),
        Some(real_num) => medit_setup_existing(g, &mut olc, real_num as usize),
    }

    medit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Medit;
    }
    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum as usize].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!("OLC: {} starts editing zone {} allowed zone {}", name, zvnum, allowed);
    g.mudlog(MudlogKind::Cmp, level, true, &msg);
}

fn medit_save_to_disk(g: &mut Game, zone_vnum: Idx) -> bool {
    let rz = g.world.real_zone(zone_vnum).map(|z| z as usize);
    save_mobiles(g, rz)
}

/// init_mobile + medit_setup_new.
fn medit_setup_new(olc: &mut OlcData) {
    let mob = MobProto {
        vnum: NOBODY,
        keywords: Some(b"mob unfinished".to_vec()),
        short_descr: Some(b"the unfinished mob".to_vec()),
        long_descr: Some(b"An unfinished mob stands here.\r\n".to_vec()),
        ddescription: Some(b"It looks unfinished.\r\n".to_vec()),
        hit: 1,
        mana: 1,
        damnodice: 1,
        damsizedice: 1,
        str_: Some(11),
        intel: Some(11),
        wis: Some(11),
        dex: Some(11),
        con: Some(11),
        cha: Some(11),
        saving_para: Some(0),
        saving_rod: Some(0),
        saving_petri: Some(0),
        saving_breath: Some(0),
        saving_spell: Some(0),
        ..Default::default()
    };
    olc.mob = Some(Box::new(mob));
    olc.mob_rnum = NOBODY;
    // SET_BIT_AR(MOB_FLAGS(mob), MOB_ISNPC)
    set_mob_flag(olc, flags::MOB_ISNPC, true);
    olc.script = None;
    olc.value = 0;
    olc.item_type = crate::dg::MOB_TRIGGER;
}

pub fn medit_setup_existing(g: &mut Game, olc: &mut OlcData, rmob_num: usize) {
    let proto = g.world.mob_protos[rmob_num].clone();
    olc.mob_rnum = rmob_num as Idx;
    olc.item_type = crate::dg::MOB_TRIGGER;
    olc.mob = Some(Box::new(proto));
    dg_olc_script_copy(olc);
    if let Some(m) = olc.mob.as_mut() {
        m.proto_script.clear();
    }
}

fn set_mob_flag(olc: &mut OlcData, bit: usize, on: bool) {
    if let Some(m) = olc.mob.as_mut() {
        if on {
            m.act[bit / 32] |= 1 << (bit % 32);
        } else {
            m.act[bit / 32] &= !(1 << (bit % 32));
        }
    }
}

pub fn medit_save_internally(g: &mut Game, di: usize, olc: &mut OlcData) {
    let is_new = g.world.real_mobile(olc.number as Idx).is_none();

    let mob = olc.mob.as_ref().expect("medit without a mob").as_ref().clone();
    let Some(new_rnum) = add_mobile(g, &mob, olc.number as Idx) else {
        g.log("medit_save_internally: add_mobile failed.".to_string());
        return;
    };

    // Update triggers and free the old proto list.
    let script = olc.script.clone().unwrap_or_default();
    g.world.mob_protos[new_rnum as usize].proto_script = script.clone();

    // This takes care of the mobs currently in-game.
    for id in g.character_list.clone() {
        if g.chars.get(id).map(|c| c.mob_rnum) != Some(new_rnum) {
            continue;
        }
        if g.chars.get(id).is_some_and(|c| c.script.is_some()) {
            crate::dg::extract_script(g, crate::dg::GoId::Char(id));
        }
        if let Some(c) = g.chars.get_mut(id) {
            c.proto_script = script.clone();
        }
        crate::dg::assign_triggers(g, crate::dg::GoId::Char(id));
    }

    if !is_new {
        return;
    }

    // Keepers in shops being edited, and other mobs being edited. Unlike
    // redit, the descriptor doing the saving is NOT skipped — its own copy
    // is bumped too, so the checked-out `olc` gets the same treatment.
    if olc.mob_rnum != NOBODY && olc.mob_rnum >= new_rnum {
        olc.mob_rnum += 1;
    }
    let _ = di;
    let others: Vec<usize> = g.descriptors.order.clone();
    for dsc in others.iter().copied() {
        let state = g.descriptors.get(dsc).map(|d| d.state);
        let Some(other) = g.olc.get_mut(&dsc) else { continue };
        match state {
            Some(ConState::Sedit) => {
                if other.shop_keeper != NOBODY && other.shop_keeper >= new_rnum {
                    other.shop_keeper += 1;
                }
            }
            Some(ConState::Medit) => {
                if other.mob_rnum != NOBODY && other.mob_rnum >= new_rnum {
                    other.mob_rnum += 1;
                }
            }
            _ => {}
        }
    }
    // And zedit sessions.
    for dsc in others {
        let state = g.descriptors.get(dsc).map(|d| d.state);
        if state != Some(ConState::Zedit) {
            continue;
        }
        let Some(other) = g.olc.get_mut(&dsc) else { continue };
        let Some(zone) = other.zone.as_mut() else { continue };
        for cmd in zone.cmds.iter_mut() {
            if cmd.command == b'M' && cmd.arg1 >= new_rnum as i32 {
                cmd.arg1 += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn medit_disp_positions(g: &mut Game, di: usize) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
        clear_screen(g, di);
        let names: Vec<BStr> =
            POSITION_TYPES.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &names, true);
    }
    write_to_desc(g, di, b"Enter position number : ");
}

fn medit_disp_sex(g: &mut Game, di: usize) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
        clear_screen(g, di);
        let names: Vec<BStr> = GENDERS.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &names, true);
    }
    write_to_desc(g, di, b"Enter gender number : ");
}

fn medit_disp_attack_types(g: &mut Game, di: usize) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    for (i, (singular, _)) in crate::fight::ATTACK_HIT_TEXT.iter().enumerate() {
        out.extend_from_slice(format!("{}{:2}{}) ", c.grn_s(), i, c.nrm_s()).as_bytes());
        out.extend_from_slice(singular);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Enter attack type : ");
    write_to_desc(g, di, &out);
}

fn medit_illegal_mob_flag(fl: usize) -> bool {
    fl == flags::MOB_ISNPC || fl == flags::MOB_NOTDEADYET
}

fn medit_get_mob_flag_by_number(num: i32) -> i32 {
    let mut count = 0;
    for i in 0..flags::NUM_MOB_FLAGS {
        if medit_illegal_mob_flag(i) {
            continue;
        }
        count += 1;
        if count == num {
            return i as i32;
        }
    }
    -1
}

fn medit_disp_mob_flags(g: &mut Game, di: usize, olc: &OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut count = 0;
    let mut columns = 0;
    let mut out: BStr = Vec::new();
    for i in 0..flags::NUM_MOB_FLAGS {
        if medit_illegal_mob_flag(i) {
            continue;
        }
        count += 1;
        columns += 1;
        // "%s%2d%s) %-20.20s %s"
        let name = ACTION_BITS[i];
        let trimmed: String = name.chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20}  {}",
                c.grn_s(),
                count,
                c.nrm_s(),
                trimmed,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(&olc.mob.as_ref().unwrap().act, &ACTION_BITS, &mut bits);
    out.extend_from_slice(b"\r\nCurrent flags : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter mob flags (0 to quit) : ");
    write_to_desc(g, di, &out);
}

fn medit_disp_aff_flags(g: &mut Game, di: usize, olc: &OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
        clear_screen(g, di);
        // +1/-1 antics needed because AFF_FLAGS doesn't start at 0.
        let names: Vec<BStr> = AFFECTED_BITS[1..flags::NUM_AFF_FLAGS]
            .iter()
            .map(|s| s.as_bytes().to_vec())
            .collect();
        column_list(g, chid, 0, &names, true);
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(&olc.mob.as_ref().unwrap().affected_by, &AFFECTED_BITS, &mut bits);
    let c = g.olc_colors;
    let mut out: BStr = b"\r\nCurrent flags   : ".to_vec();
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter aff flags (0 to quit) : ");
    write_to_desc(g, di, &out);
}

fn medit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let mob = olc.mob.as_ref().unwrap().as_ref().clone();
    let c = g.olc_colors;

    let gender = GENDERS
        .get(mob.sex.clamp(0, i32::MAX) as usize)
        .copied()
        .unwrap_or("(null)");
    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!("-- Mob Number:  [{}{}{}]\r\n", c.cyn_s(), olc.number, c.nrm_s()).as_bytes(),
    );
    // "%s1%s) Sex: %s%-7.7s%s\t %s2%s) Keywords: %s%s\r\n"
    let g7: String = gender.chars().take(7).collect();
    out.extend_from_slice(
        format!(
            "{}1{}) Sex: {}{:<7}{}\t         {}2{}) Keywords: {}",
            c.grn_s(),
            c.nrm_s(),
            c.yel_s(),
            g7,
            c.nrm_s(),
            c.grn_s(),
            c.nrm_s(),
            c.yel_s()
        )
        .as_bytes(),
    );
    out.extend_from_slice(mob.keywords.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}3{}) S-Desc: {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(mob.short_descr.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}4{}) L-Desc:-\r\n{}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(mob.long_descr.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}5{}) D-Desc:-\r\n{}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(mob.ddescription.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    write_to_desc(g, di, &out);

    let mut flags_buf: BStr = Vec::new();
    sprintbitarray(&mob.act, &ACTION_BITS, &mut flags_buf);
    let mut flag2: BStr = Vec::new();
    sprintbitarray(&mob.affected_by, &AFFECTED_BITS, &mut flag2);

    let pos = |p: i32| -> &'static str {
        POSITION_TYPES.get(p.clamp(0, i32::MAX) as usize).copied().unwrap_or("(null)")
    };
    let attack = crate::fight::ATTACK_HIT_TEXT
        .get(mob.bare_hand_attack.unwrap_or(0).clamp(0, i32::MAX) as usize)
        .map(|(s, _)| *s)
        .unwrap_or(b"(null)");

    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!(
            "{}6{}) Position  : {}{}\r\n{}7{}) Default   : {}{}\r\n{}8{}) Attack    : {}",
            c.grn_s(),
            c.nrm_s(),
            c.yel_s(),
            pos(mob.position),
            c.grn_s(),
            c.nrm_s(),
            c.yel_s(),
            pos(mob.default_pos),
            c.grn_s(),
            c.nrm_s(),
            c.yel_s()
        )
        .as_bytes(),
    );
    out.extend_from_slice(attack);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}9{}) Stats Menu...\r\n", c.grn_s(), c.nrm_s()).as_bytes());
    out.extend_from_slice(format!("{}A{}) NPC Flags : {}", c.grn_s(), c.nrm_s(), c.cyn_s()).as_bytes());
    out.extend_from_slice(&flags_buf);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}B{}) AFF Flags : {}", c.grn_s(), c.nrm_s(), c.cyn_s()).as_bytes());
    out.extend_from_slice(&flag2);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}S{}) Script    : {}{}\r\n{}W{}) Copy mob\r\n{}X{}) Delete mob\r\n{}Q{}) Quit\r\nEnter choice : ",
            c.grn_s(),
            c.nrm_s(),
            c.cyn_s(),
            if olc.script.is_some() { "Set." } else { "Not Set." },
            c.grn_s(),
            c.nrm_s(),
            c.grn_s(),
            c.nrm_s(),
            c.grn_s(),
            c.nrm_s()
        )
        .as_bytes(),
    );
    write_to_desc(g, di, &out);

    olc.mode = MEDIT_MAIN_MENU;
}

fn medit_disp_stats_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    let mob = olc.mob.as_ref().unwrap().as_ref().clone();
    let c = g.olc_colors;
    let (cy, nr, ye) = (c.cyn_s(), c.nrm_s(), c.yel_s());

    // Colour codes are used here so count_color_chars can measure them.
    let buf = format!(
        "(range \ty{}\tn to \ty{}\tn)",
        mob.hit + mob.mov,
        (mob.hit * mob.mana) + mob.mov
    );
    let width = count_color_chars(buf.as_bytes()) + 28;

    let mut out = String::new();
    out.push_str(&format!("-- Mob Number:  {cy}[{ye}{}{cy}]{nr}\r\n", olc.number));
    out.push_str(&format!("({cy}1{nr}) Level:       {cy}[{ye}{:4}{cy}]{nr}\r\n", mob.level));
    out.push_str(&format!("({cy}2{nr}) {cy}Auto Set Stats (based on level){nr}\r\n\r\n"));
    out.push_str("Hit Points  (xdy+z):        Bare Hand Damage (xdy+z): \r\n");
    out.push_str(&format!(
        "({cy}3{nr}) HP NumDice:  {cy}[{ye}{:5}{cy}]{nr}    ({cy}6{nr}) BHD NumDice:  {cy}[{ye}{:5}{cy}]{nr}\r\n",
        mob.hit, mob.damnodice
    ));
    out.push_str(&format!(
        "({cy}4{nr}) HP SizeDice: {cy}[{ye}{:5}{cy}]{nr}    ({cy}7{nr}) BHD SizeDice: {cy}[{ye}{:5}{cy}]{nr}\r\n",
        mob.mana, mob.damsizedice
    ));
    out.push_str(&format!(
        "({cy}5{nr}) HP Addition: {cy}[{ye}{:5}{cy}]{nr}    ({cy}8{nr}) DamRoll:      {cy}[{ye}{:5}{cy}]{nr}\r\n",
        mob.mov, mob.damroll
    ));
    out.push_str(&format!(
        "{:<width$}(range {ye}{}{nr} to {ye}{}{nr})\r\n\r\n",
        buf,
        mob.damnodice + mob.damroll,
        (mob.damnodice * mob.damsizedice) + mob.damroll,
        width = width
    ));
    out.push_str(&format!(
        "({cy}A{nr}) Armor Class: {cy}[{ye}{:4}{cy}]{nr}        ({cy}D{nr}) Hitroll:   {cy}[{ye}{:5}{cy}]{nr}\r\n",
        mob.armor, mob.hitroll
    ));
    out.push_str(&format!(
        "({cy}B{nr}) Exp Points:  {cy}[{ye}{:10}{cy}]{nr}  ({cy}E{nr}) Alignment: {cy}[{ye}{:5}{cy}]{nr}\r\n",
        mob.exp, mob.alignment
    ));
    out.push_str(&format!(
        "({cy}C{nr}) Gold:        {cy}[{ye}{:10}{cy}]{nr}\r\n\r\n",
        mob.gold
    ));
    write_to_desc(g, di, out.as_bytes());

    if g.config.medit_advanced_stats {
        let str_ = mob.str_.unwrap_or(11);
        let add = mob.str_add.unwrap_or(0);
        let mut out = String::new();
        out.push_str(&format!(
            "({cy}F{nr}) Str: {cy}[{ye}{:2}/{:3}{cy}]{nr}   Saving Throws\r\n",
            str_, add
        ));
        for (key, label, val, skey, slabel, sval) in [
            ("G", "Int", mob.intel.unwrap_or(11), "L", "Paralysis    ", mob.saving_para.unwrap_or(0)),
            ("H", "Wis", mob.wis.unwrap_or(11), "M", "Rods/Staves  ", mob.saving_rod.unwrap_or(0)),
            ("I", "Dex", mob.dex.unwrap_or(11), "N", "Petrification", mob.saving_petri.unwrap_or(0)),
            ("J", "Con", mob.con.unwrap_or(11), "O", "Breath       ", mob.saving_breath.unwrap_or(0)),
            ("K", "Cha", mob.cha.unwrap_or(11), "P", "Spells       ", mob.saving_spell.unwrap_or(0)),
        ] {
            out.push_str(&format!(
                "({cy}{key}{nr}) {label}: {cy}[{ye}{val:3}{cy}]{nr}      ({cy}{skey}{nr}) {slabel} {cy}[{ye}{sval:3}{cy}]{nr}\r\n"
            ));
        }
        out.push_str("\r\n");
        write_to_desc(g, di, out.as_bytes());
    }

    write_to_desc(
        g,
        di,
        format!("({cy}Q{nr}) Quit to main menu\r\nEnter choice : ").as_bytes(),
    );
    olc.mode = MEDIT_STATS_MENU;
}

pub fn medit_autoroll_stats(g: &Game, olc: &mut OlcData) {
    let advanced = g.config.medit_advanced_stats;
    let mob = olc.mob.as_mut().unwrap();
    let mob_lev = limit(mob.level, 1, LVL_IMPL as i32);
    mob.level = mob_lev;

    mob.mov = mob_lev * 10;
    mob.hit = mob_lev / 5;
    mob.mana = mob_lev / 5;
    mob.damnodice = 1.max(mob_lev / 6);
    mob.damsizedice = 2.max(mob_lev / 6);
    mob.damroll = mob_lev / 6;
    mob.hitroll = mob_lev / 3;
    mob.exp = mob_lev * mob_lev * 100;
    mob.gold = mob_lev * 10;
    mob.armor = 100 - (mob_lev * 6);

    if advanced {
        let stat = limit((mob_lev * 2) / 3, 11, 18);
        mob.str_ = Some(stat);
        mob.intel = Some(stat);
        mob.wis = Some(stat);
        mob.dex = Some(stat);
        mob.con = Some(stat);
        mob.cha = Some(stat);
        let save = mob_lev / 4;
        mob.saving_para = Some(save);
        mob.saving_rod = Some(save);
        mob.saving_petri = Some(save);
        mob.saving_breath = Some(save);
        mob.saving_spell = Some(save);
    }
}

// ---------------------------------------------------------------------------
// The main loop
// ---------------------------------------------------------------------------

pub fn medit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let mut arg = arg.to_vec();
    let mut i: i32 = -1;

    if olc.mode > MEDIT_NUMERICAL_RESPONSE {
        i = atoi(&arg);
        // A guard that only rejects a lone "-" lets anything else
        // non-numeric land as atoi == 0 — `abc` at the position prompt
        // would silently set the mob to Dead.
        let bad = arg.is_empty()
            || (!arg[0].is_ascii_digit()
                && (arg[0] != b'-'
                    || !arg.get(1).copied().unwrap_or(0).is_ascii_digit()));
        if bad {
            write_to_desc(g, di, b"Try again : ");
            return Some(olc);
        }
    } else if !genolc_checkstring(&mut arg) {
        return Some(olc);
    }

    match olc.mode {
        MEDIT_CONFIRM_SAVESTRING => {
            // Ensure mob has MOB_ISNPC set.
            set_mob_flag(&mut olc, flags::MOB_ISNPC, true);
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    medit_save_internally(g, di, &mut olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg = format!("OLC: {} edits mob {}", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    if g.config.auto_save_olc {
                        // A zone the mob does not belong to cannot be written,
                        // which is a failure to report like any other.
                        let mut saved = false;
                        if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, olc.number) {
                            let zvnum = g.world.zones[z].number;
                            saved = medit_save_to_disk(g, zvnum);
                        }
                        if saved {
                            write_to_desc(g, di, b"Mobile saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the mobile"));
                        }
                    } else {
                        write_to_desc(g, di, b"Mobile saved to memory.\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    let script = olc.script.take();
                    if let (Some(m), Some(script)) = (olc.mob.as_mut(), script) {
                        m.proto_script = script;
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    write_to_desc(g, di, b"Do you wish to save your changes? : ");
                    return Some(olc);
                }
            }
        }

        MEDIT_MAIN_MENU => {
            i = 0;
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = MEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    return Some(olc);
                }
                Some(b'1') => {
                    olc.mode = MEDIT_SEX;
                    medit_disp_sex(g, di);
                    return Some(olc);
                }
                Some(b'2') => {
                    olc.mode = MEDIT_KEYWORD;
                    i -= 1;
                }
                Some(b'3') => {
                    olc.mode = MEDIT_S_DESC;
                    i -= 1;
                }
                Some(b'4') => {
                    olc.mode = MEDIT_L_DESC;
                    i -= 1;
                }
                Some(b'5') => {
                    olc.mode = MEDIT_D_DESC;
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter mob description:\r\n\r\n");
                    let old = olc.mob.as_ref().unwrap().ddescription.clone();
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, MAX_MOB_DESC, 0, old);
                    }
                    olc.str_target = Some(StrTarget::MobDesc);
                    olc.value = 1;
                    return Some(olc);
                }
                Some(b'6') => {
                    olc.mode = MEDIT_POS;
                    medit_disp_positions(g, di);
                    return Some(olc);
                }
                Some(b'7') => {
                    olc.mode = MEDIT_DEFAULT_POS;
                    medit_disp_positions(g, di);
                    return Some(olc);
                }
                Some(b'8') => {
                    olc.mode = MEDIT_ATTACK;
                    medit_disp_attack_types(g, di);
                    return Some(olc);
                }
                Some(b'9') => {
                    olc.mode = MEDIT_STATS_MENU;
                    medit_disp_stats_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'a') | Some(b'A') => {
                    olc.mode = MEDIT_NPC_FLAGS;
                    medit_disp_mob_flags(g, di, &olc);
                    return Some(olc);
                }
                Some(b'b') | Some(b'B') => {
                    olc.mode = MEDIT_AFF_FLAGS;
                    medit_disp_aff_flags(g, di, &olc);
                    return Some(olc);
                }
                Some(b'w') | Some(b'W') => {
                    write_to_desc(g, di, b"Copy what mob? ");
                    olc.mode = MEDIT_COPY;
                    return Some(olc);
                }
                Some(b'x') | Some(b'X') => {
                    write_to_desc(g, di, b"Are you sure you want to delete this mobile? ");
                    olc.mode = MEDIT_DELETE;
                    return Some(olc);
                }
                Some(b's') | Some(b'S') => {
                    olc.script_mode = SCRIPT_MAIN_MENU;
                    dg_script_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    medit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
            if i == 0 {
                // fall through to the "changed" tail
            } else {
                if i == 1 {
                    write_to_desc(g, di, b"\r\nEnter new value : ");
                } else if i == -1 {
                    write_to_desc(g, di, b"\r\nEnter new text :\r\n] ");
                } else {
                    write_to_desc(g, di, b"Oops...\r\n");
                }
                return Some(olc);
            }
        }

        MEDIT_STATS_MENU => {
            i = 0;
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    medit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'1') => {
                    olc.mode = MEDIT_LEVEL;
                    i += 1;
                }
                Some(b'2') => {
                    medit_autoroll_stats(g, &mut olc);
                    medit_disp_stats_menu(g, di, &mut olc);
                    olc.value = 1;
                    return Some(olc);
                }
                Some(b'3') => {
                    olc.mode = MEDIT_NUM_HP_DICE;
                    i += 1;
                }
                Some(b'4') => {
                    olc.mode = MEDIT_SIZE_HP_DICE;
                    i += 1;
                }
                Some(b'5') => {
                    olc.mode = MEDIT_ADD_HP;
                    i += 1;
                }
                Some(b'6') => {
                    olc.mode = MEDIT_NDD;
                    i += 1;
                }
                Some(b'7') => {
                    olc.mode = MEDIT_SDD;
                    i += 1;
                }
                Some(b'8') => {
                    olc.mode = MEDIT_DAMROLL;
                    i += 1;
                }
                Some(b'a') | Some(b'A') => {
                    olc.mode = MEDIT_AC;
                    i += 1;
                }
                Some(b'b') | Some(b'B') => {
                    olc.mode = MEDIT_EXP;
                    i += 1;
                }
                Some(b'c') | Some(b'C') => {
                    olc.mode = MEDIT_GOLD;
                    i += 1;
                }
                Some(b'd') | Some(b'D') => {
                    olc.mode = MEDIT_HITROLL;
                    i += 1;
                }
                Some(b'e') | Some(b'E') => {
                    olc.mode = MEDIT_ALIGNMENT;
                    i += 1;
                }
                Some(ch @ (b'f' | b'F' | b'g' | b'G' | b'h' | b'H' | b'i' | b'I' | b'j' | b'J'
                | b'k' | b'K' | b'l' | b'L' | b'm' | b'M' | b'n' | b'N' | b'o' | b'O' | b'p'
                | b'P')) => {
                    if !g.config.medit_advanced_stats {
                        write_to_desc(g, di, b"Invalid Choice!\r\nEnter Choice : ");
                        return Some(olc);
                    }
                    olc.mode = match ch.to_ascii_lowercase() {
                        b'f' => MEDIT_STR,
                        b'g' => MEDIT_INT,
                        b'h' => MEDIT_WIS,
                        b'i' => MEDIT_DEX,
                        b'j' => MEDIT_CON,
                        b'k' => MEDIT_CHA,
                        b'l' => MEDIT_PARA,
                        b'm' => MEDIT_ROD,
                        b'n' => MEDIT_PETRI,
                        b'o' => MEDIT_BREATH,
                        _ => MEDIT_SPELL,
                    };
                    i += 1;
                }
                _ => {
                    medit_disp_stats_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
            if i == 0 {
                // fall through
            } else {
                if i == 1 {
                    write_to_desc(g, di, b"\r\nEnter new value : ");
                } else if i == -1 {
                    write_to_desc(g, di, b"\r\nEnter new text :\r\n] ");
                } else {
                    write_to_desc(g, di, b"Oops...\r\n");
                }
                return Some(olc);
            }
        }

        crate::olc::trigedit::OLC_SCRIPT_EDIT => {
            if crate::olc::trigedit::dg_script_edit_parse(g, di, &mut olc, &arg) {
                return Some(olc);
            }
        }

        MEDIT_KEYWORD => {
            mud_net::editor::smash_tilde(&mut arg);
            olc.mob.as_mut().unwrap().keywords = Some(str_udup(&arg));
        }
        MEDIT_S_DESC => {
            mud_net::editor::smash_tilde(&mut arg);
            olc.mob.as_mut().unwrap().short_descr = Some(str_udup(&arg));
        }
        MEDIT_L_DESC => {
            mud_net::editor::smash_tilde(&mut arg);
            let mob = olc.mob.as_mut().unwrap();
            if !arg.is_empty() {
                let mut v = arg.clone();
                v.extend_from_slice(b"\r\n");
                v.truncate(MAX_INPUT_LENGTH - 1);
                mob.long_descr = Some(v);
            } else {
                mob.long_descr = Some(b"undefined".to_vec());
            }
        }
        MEDIT_D_DESC => {
            // We should never get here.
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: medit_parse(): Reached D_DESC case!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
            return None;
        }

        MEDIT_NPC_FLAGS => {
            i = atoi(&arg);
            if i > 0 {
                let j = medit_get_mob_flag_by_number(i);
                if j == -1 {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    write_to_desc(g, di, b"Enter mob flags (0 to quit) :");
                    return Some(olc);
                } else if j <= flags::NUM_MOB_FLAGS as i32 {
                    let bit = j as usize;
                    let m = olc.mob.as_mut().unwrap();
                    m.act[bit / 32] ^= 1 << (bit % 32);
                }
                medit_disp_mob_flags(g, di, &olc);
                return Some(olc);
            }
        }

        MEDIT_AFF_FLAGS => {
            i = atoi(&arg);
            if i > 0 {
                if (i as usize) < flags::NUM_AFF_FLAGS {
                    let bit = i as usize;
                    let m = olc.mob.as_mut().unwrap();
                    m.affected_by[bit / 32] ^= 1 << (bit % 32);
                }
                // Remove unwanted bits right away.
                let m = olc.mob.as_mut().unwrap();
                for bit in [flags::AFF_CHARM, flags::AFF_POISON, flags::AFF_SLEEP] {
                    m.affected_by[bit / 32] &= !(1 << (bit % 32));
                }
                medit_disp_aff_flags(g, di, &olc);
                return Some(olc);
            }
        }

        // Numerical responses.
        MEDIT_SEX => {
            olc.mob.as_mut().unwrap().sex = limit(i - 1, 0, NUM_GENDERS as i32 - 1);
        }
        MEDIT_HITROLL => return stat(g, di, olc, |m, i| m.hitroll = limit(i, 0, 50), i),
        MEDIT_DAMROLL => return stat(g, di, olc, |m, i| m.damroll = limit(i, 0, 50), i),
        MEDIT_NDD => return stat(g, di, olc, |m, i| m.damnodice = limit(i, 0, 30), i),
        MEDIT_SDD => return stat(g, di, olc, |m, i| m.damsizedice = limit(i, 0, 127), i),
        MEDIT_NUM_HP_DICE => return stat(g, di, olc, |m, i| m.hit = limit(i, 0, 30), i),
        MEDIT_SIZE_HP_DICE => return stat(g, di, olc, |m, i| m.mana = limit(i, 0, 1000), i),
        MEDIT_ADD_HP => return stat(g, di, olc, |m, i| m.mov = limit(i, 0, 30000), i),
        MEDIT_AC => return stat(g, di, olc, |m, i| m.armor = limit(i, -200, 200), i),
        MEDIT_EXP => return stat(g, di, olc, |m, i| m.exp = limit(i, 0, MAX_MOB_EXP), i),
        MEDIT_GOLD => return stat(g, di, olc, |m, i| m.gold = limit(i, 0, MAX_MOB_GOLD), i),
        MEDIT_STR => return stat(g, di, olc, |m, i| m.str_ = Some(limit(i, 11, 25)), i),
        MEDIT_INT => return stat(g, di, olc, |m, i| m.intel = Some(limit(i, 11, 25)), i),
        MEDIT_WIS => return stat(g, di, olc, |m, i| m.wis = Some(limit(i, 11, 25)), i),
        MEDIT_DEX => return stat(g, di, olc, |m, i| m.dex = Some(limit(i, 11, 25)), i),
        MEDIT_CON => return stat(g, di, olc, |m, i| m.con = Some(limit(i, 11, 25)), i),
        MEDIT_CHA => return stat(g, di, olc, |m, i| m.cha = Some(limit(i, 11, 25)), i),
        MEDIT_PARA => return stat(g, di, olc, |m, i| m.saving_para = Some(limit(i, 0, 100)), i),
        MEDIT_ROD => return stat(g, di, olc, |m, i| m.saving_rod = Some(limit(i, 0, 100)), i),
        MEDIT_PETRI => return stat(g, di, olc, |m, i| m.saving_petri = Some(limit(i, 0, 100)), i),
        MEDIT_BREATH => return stat(g, di, olc, |m, i| m.saving_breath = Some(limit(i, 0, 100)), i),
        MEDIT_SPELL => return stat(g, di, olc, |m, i| m.saving_spell = Some(limit(i, 0, 100)), i),
        MEDIT_LEVEL => return stat(g, di, olc, |m, i| m.level = limit(i, 1, LVL_IMPL as i32), i),
        MEDIT_ALIGNMENT => {
            return stat(g, di, olc, |m, i| m.alignment = limit(i, -1000, 1000), i)
        }

        MEDIT_POS => {
            olc.mob.as_mut().unwrap().position = limit(i - 1, 0, NUM_POSITIONS as i32 - 1);
        }
        MEDIT_DEFAULT_POS => {
            olc.mob.as_mut().unwrap().default_pos = limit(i - 1, 0, NUM_POSITIONS as i32 - 1);
        }
        MEDIT_ATTACK => {
            olc.mob.as_mut().unwrap().bare_hand_attack =
                Some(limit(i, 0, crate::fight::ATTACK_HIT_TEXT.len() as i32 - 1));
        }

        MEDIT_COPY => {
            match g.world.real_mobile(atoi(&arg).clamp(0, u16::MAX as i32) as Idx) {
                Some(r) => medit_setup_existing(g, &mut olc, r as usize),
                None => write_to_desc(g, di, b"That mob does not exist.\r\n"),
            }
        }

        MEDIT_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let rnum = olc.mob_rnum;
                    if delete_mobile(g, rnum).is_some() {
                        write_to_desc(g, di, b"Mobile deleted.\r\n");
                        // Same toggle the save path honours.
                        if g.config.auto_save_olc {
                            crate::db::save_all(g);
                        }
                    } else {
                        write_to_desc(g, di, b"Couldn't delete the mobile!\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    medit_disp_menu(g, di, &mut olc);
                    olc.mode = MEDIT_MAIN_MENU;
                    return Some(olc);
                }
                _ => write_to_desc(g, di, b"Please answer 'Y' or 'N': "),
            }
        }

        _ => {
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: medit_parse(): Reached default case!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
            return None;
        }
    }

    olc.value = 1;
    medit_disp_menu(g, di, &mut olc);
    Some(olc)
}

/// The stats-menu setters all share a tail: apply, mark changed, redisplay.
fn stat(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    f: impl FnOnce(&mut MobProto, i32),
    i: i32,
) -> Option<Box<OlcData>> {
    f(olc.mob.as_mut().unwrap(), i);
    olc.value = 1;
    medit_disp_stats_menu(g, di, &mut olc);
    Some(olc)
}

/// medit_string_cleanup: every terminator goes back to
/// the main menu.
pub fn medit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    if olc.str_target.take() == Some(StrTarget::MobDesc) {
        olc.mob.as_mut().unwrap().ddescription = text;
    }
    medit_disp_menu(g, di, &mut olc);
    Some(olc)
}
