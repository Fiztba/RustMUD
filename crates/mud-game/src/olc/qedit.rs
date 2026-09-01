//! The quest editor.
//!
//! Three shapes worth naming, because all three are observable:
//!
//! * The main menu printed `quest_types[quest->type]`, and a quest that has
//! never had a type set holds AQ_UNDEFINED (-1) — so every `qedit` on a
//! new vnum would read one pointer *below* the table, where the word is
//! NULL and `%s` renders it `(null)`. The menu says `Undefined` instead
//! (B55).
//! * `qedit save <n>` names a zone. Resolving it as a vnum instead would
//! make `qedit save 30` save zone 0, and using a vnum when the builder
//! omits the argument would make the no-argument save fail outright.
//! * The five vnum-typed quest fields (`qm`, `prereq`, `obj_reward`,
//! `prev_quest`, `next_quest`) are stored as u16, so the editor's
//! "-1 for none" answers land in them as 65535. `idx_store` does that.

use std::cmp::Ordering;

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;
use mud_world::model::Quest;

use crate::act::informative::column_list;
use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::{is_number, two_arguments};
use crate::olc::genqst::{add_quest, copy_quest, delete_quest, save_quests};
use crate::olc::{
    can_edit_zone, clear_screen, genolc_checkstring, get_char_colors, str_udup, OlcData,
    StrTarget, CLEANUP_ALL, CLEANUP_STRUCTS,
};
use crate::quest::{
    real_quest, sprintbit, AQ_FLAGS, AQ_OBJ_FIND, AQ_OBJ_RETURN, AQ_ROOM_CLEAR, AQ_ROOM_FIND,
    AQ_UNDEFINED, QUEST_TYPES,
};

/// QEDIT connectedness.
pub const QEDIT_MAIN_MENU: i32 = 0;
pub const QEDIT_CONFIRM_SAVESTRING: i32 = 1;
pub const QEDIT_NAME: i32 = 2;
pub const QEDIT_DESC: i32 = 3;
pub const QEDIT_INFO: i32 = 4;
pub const QEDIT_COMPLETE: i32 = 5;
pub const QEDIT_ABANDON: i32 = 6;
pub const QEDIT_QUESTMASTER: i32 = 7;
pub const QEDIT_TYPES: i32 = 8;
pub const QEDIT_FLAGS: i32 = 9;
pub const QEDIT_TARGET: i32 = 10;
pub const QEDIT_QUANTITY: i32 = 11;
pub const QEDIT_POINTSCOMP: i32 = 12;
pub const QEDIT_POINTSQUIT: i32 = 13;
pub const QEDIT_LEVELMIN: i32 = 14;
pub const QEDIT_LEVELMAX: i32 = 15;
pub const QEDIT_PREREQ: i32 = 16;
pub const QEDIT_TIMELIMIT: i32 = 17;
pub const QEDIT_RETURNMOB: i32 = 18;
pub const QEDIT_NEXTQUEST: i32 = 19;
pub const QEDIT_PREVQUEST: i32 = 20;
pub const QEDIT_CONFIRM_DELETE: i32 = 21;
pub const QEDIT_GOLD: i32 = 22;
pub const QEDIT_EXP: i32 = 23;
pub const QEDIT_OBJ: i32 = 24;

/// MAX_QUEST_* bounds.
const MAX_QUEST_NAME: usize = 40;
const MAX_QUEST_DESC: usize = 75;
const MAX_QUEST_MSG: usize = 2048;

const NUM_AQ_TYPES: i32 = 7;
const NUM_AQ_FLAGS: i32 = 1;

fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

/// Narrow to u16 and back, which is what a store into a vnum-typed quest
/// field does: -1 becomes 65535.
fn idx_store(n: i32) -> i32 {
    (n as u16) as i32
}

fn mob_of(g: &Game, vnum: i32) -> Option<usize> {
    g.world.real_mobile(vnum as u16).map(|r| r as usize)
}

fn obj_of(g: &Game, vnum: i32) -> Option<usize> {
    g.world.real_object(vnum as u16).map(|r| r as usize)
}

/// quest_types[] guarded against AQ_UNDEFINED (B55).
pub fn quest_type_name(t: i32) -> &'static str {
    if (0..NUM_AQ_TYPES).contains(&t) {
        QUEST_TYPES[t as usize]
    } else {
        "Undefined"
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn do_oasis_qedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    let mut number: i32 = NOWHERE as i32;
    let mut save = false;

    if buf1.is_empty() {
        send_to_char(g, chid, b"Specify a quest VNUM to edit.\r\n");
        return;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) != Ordering::Equal {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
        save = true;
        if is_number(&buf2) {
            number = atoi(&buf2);
        } else {
            // The argument this path stands in for is a zone
            // number, not a vnum, so that is what it produces.
            let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
            if olc_zone > 0 {
                number = match g.world.real_zone(olc_zone as Idx) {
                    None => NOWHERE as i32,
                    Some(_) => olc_zone,
                };
            }
        }
        if number == NOWHERE as i32 {
            send_to_char(g, chid, b"Save which zone?\r\n");
            return;
        }
    }

    if number == NOWHERE as i32 {
        number = atoi(&buf1);
    }
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That quest VNUM can't exist.\r\n");
        return;
    }

    // Check that the quest isn't already being edited.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Qedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        let mut msg = b"That quest is currently being edited by ".to_vec();
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
            "SYSERR: do_oasis_quest: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();

    // A save names a zone; anything else names a quest vnum.
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
        send_to_char(g, chid, b"You do not have permission to edit this zone.\r\n");
        return;
    }

    if save {
        let zvnum = g.world.zones[znum as usize].number;
        send_to_char(g, chid, format!("Saving all quests in zone {}.\r\n", zvnum).as_bytes());
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves quest info for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_quests(g, Some(znum as usize));
        return;
    }

    olc.number = number;

    match real_quest(g, number) {
        Some(real_num) => qedit_setup_existing(g, &mut olc, real_num),
        None => qedit_setup_new(&mut olc),
    }
    qedit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Qedit;
    }

    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum as usize].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!("OLC: {} starts editing zone {} allowed zone {}", name, zvnum, allowed);
    g.mudlog(MudlogKind::Brf, level, true, &msg);
}

fn qedit_setup_new(olc: &mut OlcData) {
    // The value[] slots, in this struct's names: value = points for
    // completing, penalty = points for abandoning, min_level, max_level,
    // time = time limit, obj_in = mob to return the object to, obj_out =
    // quantity of targets.
    let quest = Quest {
        vnum: olc.number as Idx,
        qm_vnum: NOBODY as i32,
        flags: 0,
        type_: AQ_UNDEFINED,
        target: NOTHING as i32,
        prereq: NOTHING as i32,
        value: 0,
        penalty: 0,
        min_level: 0,
        max_level: LVL_IMPL as i32,
        time: -1,
        obj_in: NOBODY as i32,
        obj_out: 1,
        prev_quest: NOTHING as i32,
        next_quest: NOTHING as i32,
        gold_reward: 0,
        exp_reward: 0,
        obj_reward: NOTHING as i32,
        name: Some(b"Undefined Quest".to_vec()),
        desc: Some(b"Quest definition is incomplete.".to_vec()),
        info: Some(b"There is no information on this quest.\r\n".to_vec()),
        done: Some(b"You have completed the quest.\r\n".to_vec()),
        quit: Some(b"You have abandoned the quest.\r\n".to_vec()),
    };
    olc.quest = Some(Box::new(quest));
    olc.quest_func = None;
}

/// qedit_setup_existing: copy_quest off the table, which
/// str_udups every string on the way out.
fn qedit_setup_existing(g: &Game, olc: &mut OlcData, rnum: usize) {
    olc.quest = Some(Box::new(copy_quest(&g.world.quests[rnum])));
    olc.quest_func = g.quest_secondary[rnum];
}

fn qedit_save_internally(g: &mut Game, olc: &mut OlcData) {
    let quest = olc.quest.as_ref().unwrap().as_ref().clone();
    let func = olc.quest_func;
    add_quest(g, &quest, func);
}

// ---------------------------------------------------------------------------
// The menus
// ---------------------------------------------------------------------------

fn qedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    let q = olc.quest.as_ref().unwrap().as_ref().clone();
    clear_screen(g, di);

    let quest_flags = sprintbit(q.flags as i64, &AQ_FLAGS);
    // buf2 is only built for a return-object quest, and only used there.
    let buf2: BStr = if q.type_ == AQ_OBJ_RETURN {
        match mob_of(g, q.obj_in) {
            Some(r) => {
                let mut s = b"to ".to_vec();
                s.extend_from_slice(
                    g.world.mob_protos[r].short_descr.as_deref().unwrap_or(b"(null)"),
                );
                s.extend_from_slice(format!(" [{}]", q.obj_in).as_bytes());
                s
            }
            None => format!("to an unknown mob [{}].", q.obj_in).into_bytes(),
        }
    } else {
        Vec::new()
    };

    let targetname: BStr = match q.type_ {
        t if t == AQ_OBJ_FIND || t == AQ_OBJ_RETURN => match obj_of(g, q.target) {
            Some(r) => g.world.obj_protos[r].short_description.clone().unwrap_or_default(),
            None => b"An unknown object".to_vec(),
        },
        t if t == AQ_ROOM_FIND || t == AQ_ROOM_CLEAR => {
            match g.world.real_room(q.target as u16) {
                Some(r) => g.world.rooms[r as usize].name.clone().unwrap_or_default(),
                None => b"An unknown room".to_vec(),
            }
        }
        t if (crate::quest::AQ_MOB_FIND..=crate::quest::AQ_MOB_SAVE).contains(&t) => {
            match mob_of(g, q.target) {
                // A prototype is named by its short description.
                Some(r) => g.world.mob_protos[r].short_descr.clone().unwrap_or_default(),
                None => b"An unknown mobile".to_vec(),
            }
        }
        _ => b"Unknown".to_vec(),
    };

    let qm_name: BStr = match mob_of(g, q.qm_vnum) {
        Some(r) => g.world.mob_protos[r].short_descr.clone().unwrap_or_default(),
        None => b"Invalid Mob".to_vec(),
    };
    let prereq_name: BStr = if q.prereq == NOTHING as i32 {
        Vec::new()
    } else {
        match obj_of(g, q.prereq) {
            Some(r) => g.world.obj_protos[r].short_description.clone().unwrap_or_default(),
            None => b"an unknown object".to_vec(),
        }
    };
    let qdesc = |g: &Game, vnum: i32| -> BStr {
        match real_quest(g, vnum) {
            Some(r) => g.world.quests[r].desc.clone().unwrap_or_default(),
            None => Vec::new(),
        }
    };
    let next_desc = qdesc(g, q.next_quest);
    let prev_desc = qdesc(g, q.prev_quest);

    let none_or = |v: i32| if v == NOTHING as i32 { -1 } else { v };
    let msg_or_nothing = |s: &Option<BStr>| -> BStr {
        match s {
            // Case-insensitive.
            Some(s) if !s.eq_ignore_ascii_case(b"undefined") => s.clone(),
            _ => b"Nothing\r\n".to_vec(),
        }
    };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!("-- Quest Number    : \tn[\tc{:6}\tn]\r\n", q.vnum).as_bytes(),
    );
    out.extend_from_slice(b"\tg 1\tn) Quest Name     : \ty");
    out.extend_from_slice(q.name.as_deref().unwrap_or(b"(null)"));
    out.extend_from_slice(b"\r\n\tg 2\tn) Description    : \ty");
    out.extend_from_slice(q.desc.as_deref().unwrap_or(b"(null)"));
    out.extend_from_slice(b"\r\n\tg 3\tn) Accept Message\r\n\ty");
    out.extend_from_slice(&msg_or_nothing(&q.info));
    out.extend_from_slice(b"\tg 4\tn) Completion Message\r\n\ty");
    out.extend_from_slice(&msg_or_nothing(&q.done));
    out.extend_from_slice(b"\tg 5\tn) Quit Message\r\n\ty");
    out.extend_from_slice(&msg_or_nothing(&q.quit));
    out.extend_from_slice(b"\tg 6\tn) Quest Flags    : \tc");
    out.extend_from_slice(&quest_flags);
    out.extend_from_slice(b"\r\n\tg 7\tn) Quest Type     : \tc");
    out.extend_from_slice(quest_type_name(q.type_).as_bytes());
    out.push(b' ');
    out.extend_from_slice(&buf2);
    out.extend_from_slice(
        format!("\r\n\tg 8\tn) Quest Master   : [\tc{:6}\tn] \ty", none_or(q.qm_vnum)).as_bytes(),
    );
    out.extend_from_slice(&qm_name);
    out.extend_from_slice(
        format!("\r\n\tg 9\tn) Quest Target   : [\tc{:6}\tn] \ty", none_or(q.target)).as_bytes(),
    );
    out.extend_from_slice(&targetname);
    out.extend_from_slice(
        format!(
            "\r\n\tg A\tn) Quantity       : [\tc{:6}\tn]\r\n\
             \tn    Quest Point Rewards\r\n\
             \tg B\tn) Completed      : [\tc{:6}\tn] \tg C\tn) Abandoned   : [\tc{:6}\tn]\r\n\
             \tn    Other Rewards Rewards\r\n\
             \tg G\tn) Gold Coins     : [\tc{:6}\tn] \tg T\tn) Exp Points  : [\tc{:6}\tn] \
             \tg O\tn) Object : [\tc{:6}\tn]\r\n\
             \tn    Level Limits to Accept Quest\r\n\
             \tg D\tn) Lower Level    : [\tc{:6}\tn] \tg E\tn) Upper Level : [\tc{:6}\tn]\r\n\
             \tg F\tn) Prerequisite   : [\tc{:6}\tn] \ty",
            q.obj_out,
            q.value,
            q.penalty,
            q.gold_reward,
            q.exp_reward,
            none_or(q.obj_reward),
            q.min_level,
            q.max_level,
            none_or(q.prereq),
        )
        .as_bytes(),
    );
    out.extend_from_slice(&prereq_name);
    out.extend_from_slice(
        format!(
            "\r\n\tg L\tn) Time Limit     : [\tc{:6}\tn]\r\n\tg N\tn) Next Quest     : [\tc{:6}\tn] \ty",
            q.time,
            none_or(q.next_quest),
        )
        .as_bytes(),
    );
    out.extend_from_slice(&next_desc);
    out.extend_from_slice(
        format!(
            "\r\n\tg P\tn) Previous Quest : [\tc{:6}\tn] \ty",
            none_or(q.prev_quest)
        )
        .as_bytes(),
    );
    out.extend_from_slice(&prev_desc);
    out.extend_from_slice(
        b"\r\n\tg X\tn) Delete Quest\r\n\tg Q\tn) Quit\r\nEnter Choice : ",
    );
    write_to_desc(g, di, &out);
    olc.mode = QEDIT_MAIN_MENU;
}

fn qedit_disp_type_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let list: Vec<BStr> = QUEST_TYPES.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &list, true);
    }
    write_to_desc(g, di, b"\r\nEnter Quest type : ");
    olc.mode = QEDIT_TYPES;
}

/// qedit_disp_flag_menu. The `get_char_colors` call sets
/// the shared OLC colour globals for whoever asked last and then goes
/// unused.
fn qedit_disp_flag_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let list: Vec<BStr> = AQ_FLAGS.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &list, true);
    }
    let bits = sprintbit(olc.quest.as_ref().unwrap().flags as i64, &AQ_FLAGS);
    let mut out: BStr = b"\r\nQuest flags: \tc".to_vec();
    out.extend_from_slice(&bits);
    out.extend_from_slice(b"\tn\r\nEnter quest flags, 0 to quit : ");
    write_to_desc(g, di, &out);
    olc.mode = QEDIT_FLAGS;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub fn qedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let number = atoi(arg);
    let chid = g.descriptors.get(di).and_then(|d| d.character);

    match olc.mode {
        QEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    if let Some(chid) = chid {
                        send_to_char(g, chid, b"Saving Quest to memory.\r\n");
                    }
                    qedit_save_internally(g, &mut olc);
                    if let Some(chid) = chid {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg = format!("OLC: {} edits quest {}", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    if g.config.auto_save_olc {
                        let zone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if save_quests(g, zone) {
                            let msg = format!("Quest {} saved to disk.\r\n", olc.number);
                            write_to_desc(g, di, msg.as_bytes());
                        } else {
                            let msg = format!(
                                "Unable to save quest {} to disk. Changes remain marked for saving.\r\n",
                                olc.number
                            );
                            write_to_desc(g, di, msg.as_bytes());
                        }
                    } else {
                        let msg = format!("Quest {} saved to memory.\r\n", olc.number);
                        write_to_desc(g, di, msg.as_bytes());
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_STRUCTS);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to save the quest? : ",
                    );
                }
            }
            return Some(olc);
        }

        QEDIT_CONFIRM_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let rnum = real_quest(g, olc.number);
                    let deleted = match rnum {
                        Some(rnum) => delete_quest(g, rnum),
                        // real_quest -> NOTHING, i.e. an rnum of 65535.
                        None => false,
                    };
                    if deleted {
                        write_to_desc(g, di, b"Quest deleted.\r\n");
                    } else {
                        write_to_desc(g, di, b"Couldn't delete the quest!\r\n");
                    }
                    if g.config.auto_save_olc {
                        let zone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if save_quests(g, zone) {
                            write_to_desc(g, di, b"Quest file saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the quest file"));
                        }
                    } else {
                        write_to_desc(g, di, b"Quest file saved to memory.\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    qedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid choice!\r\nDo you wish to delete the quest? : ",
                    );
                    return Some(olc);
                }
            }
        }

        QEDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(
                            g,
                            di,
                            b"Do you wish to save the changes to the Quest? (y/n) : ",
                        );
                        olc.mode = QEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                    return Some(olc);
                }
                Some(b'x') | Some(b'X') => {
                    olc.mode = QEDIT_CONFIRM_DELETE;
                    write_to_desc(g, di, b"Do you wish to delete the Quest? (y/n) : ");
                    return Some(olc);
                }
                Some(b'1') => {
                    olc.mode = QEDIT_NAME;
                    write_to_desc(g, di, b"Enter the quest name : ");
                    return Some(olc);
                }
                Some(b'2') => {
                    olc.mode = QEDIT_DESC;
                    write_to_desc(g, di, b"Enter the quest description :-\r\n] ");
                    return Some(olc);
                }
                Some(c @ (b'3' | b'4' | b'5')) => {
                    let (mode, prompt, target) = match c {
                        b'3' => (
                            QEDIT_INFO,
                            &b"Enter quest acceptance message:\r\n\r\n"[..],
                            StrTarget::QuestInfo,
                        ),
                        b'4' => (
                            QEDIT_COMPLETE,
                            &b"Enter quest completion message:\r\n\r\n"[..],
                            StrTarget::QuestDone,
                        ),
                        _ => (
                            QEDIT_ABANDON,
                            &b"Enter quest quit message:\r\n\r\n"[..],
                            StrTarget::QuestQuit,
                        ),
                    };
                    olc.mode = mode;
                    clear_screen(g, di);
                    if let Some(chid) = chid {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, prompt);
                    let q = olc.quest.as_ref().unwrap();
                    let old = match target {
                        StrTarget::QuestInfo => q.info.clone(),
                        StrTarget::QuestDone => q.done.clone(),
                        _ => q.quit.clone(),
                    };
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = chid {
                        string_write(g, chid, MAX_QUEST_MSG, 0, old);
                    }
                    olc.str_target = Some(target);
                    olc.value = 1;
                    return Some(olc);
                }
                Some(b'6') => {
                    olc.mode = QEDIT_FLAGS;
                    qedit_disp_flag_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'7') => {
                    olc.mode = QEDIT_TYPES;
                    qedit_disp_type_menu(g, di, &mut olc);
                    return Some(olc);
                }
                Some(b'8') => {
                    olc.mode = QEDIT_QUESTMASTER;
                    write_to_desc(g, di, b"Enter vnum of quest master : ");
                    return Some(olc);
                }
                Some(b'9') => {
                    olc.mode = QEDIT_TARGET;
                    write_to_desc(g, di, b"Enter target vnum : ");
                    return Some(olc);
                }
                Some(b'a') | Some(b'A') => {
                    olc.mode = QEDIT_QUANTITY;
                    write_to_desc(g, di, b"Enter quantity of target : ");
                    return Some(olc);
                }
                Some(b'b') | Some(b'B') => {
                    olc.mode = QEDIT_POINTSCOMP;
                    write_to_desc(g, di, b"Enter points for completing the quest : ");
                    return Some(olc);
                }
                Some(b'c') | Some(b'C') => {
                    olc.mode = QEDIT_POINTSQUIT;
                    write_to_desc(g, di, b"Enter points for quitting the quest : ");
                    return Some(olc);
                }
                Some(b'd') | Some(b'D') => {
                    olc.mode = QEDIT_LEVELMIN;
                    write_to_desc(g, di, b"Enter minimum level to accept the quest : ");
                    return Some(olc);
                }
                Some(b'e') | Some(b'E') => {
                    olc.mode = QEDIT_LEVELMAX;
                    write_to_desc(g, di, b"Enter maximum level to accept the quest : ");
                    return Some(olc);
                }
                Some(b'f') | Some(b'F') => {
                    olc.mode = QEDIT_PREREQ;
                    write_to_desc(g, di, b"Enter a prerequisite object vnum (-1 for none) : ");
                    return Some(olc);
                }
                Some(b'g') | Some(b'G') => {
                    olc.mode = QEDIT_GOLD;
                    write_to_desc(g, di, b"Enter the number of gold coins (0 for none) : ");
                    return Some(olc);
                }
                Some(b't') | Some(b'T') => {
                    olc.mode = QEDIT_EXP;
                    write_to_desc(g, di, b"Enter a number of experience points (0 for none) : ");
                    return Some(olc);
                }
                Some(b'o') | Some(b'O') => {
                    olc.mode = QEDIT_OBJ;
                    write_to_desc(g, di, b"Enter the prize object vnum (-1 for none) : ");
                    return Some(olc);
                }
                Some(b'l') | Some(b'L') => {
                    olc.mode = QEDIT_TIMELIMIT;
                    write_to_desc(g, di, b"Enter time limit to complete (-1 for none) : ");
                    return Some(olc);
                }
                Some(b'n') | Some(b'N') => {
                    olc.mode = QEDIT_NEXTQUEST;
                    write_to_desc(g, di, b"Enter vnum of next quest (-1 for none) : ");
                    return Some(olc);
                }
                Some(b'p') | Some(b'P') => {
                    olc.mode = QEDIT_PREVQUEST;
                    write_to_desc(g, di, b"Enter vnum of previous quest (-1 for none) : ");
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    qedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
        }

        QEDIT_NAME => {
            let mut arg = arg.to_vec();
            genolc_checkstring(&mut arg);
            arg.truncate(MAX_QUEST_NAME - 1);
            olc.quest.as_mut().unwrap().name = Some(str_udup(&arg));
        }
        QEDIT_DESC => {
            let mut arg = arg.to_vec();
            genolc_checkstring(&mut arg);
            arg.truncate(MAX_QUEST_DESC - 1);
            olc.quest.as_mut().unwrap().desc = Some(str_udup(&arg));
        }
        QEDIT_QUESTMASTER => {
            if number != -1 && mob_of(g, idx_store(number)).is_none() {
                write_to_desc(g, di, b"That mobile does not exist, try again : ");
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().qm_vnum = idx_store(number);
        }
        QEDIT_TYPES => {
            let number = number - 1;
            if !(0..NUM_AQ_TYPES).contains(&number) {
                write_to_desc(g, di, b"Invalid choice!\r\n");
                qedit_disp_type_menu(g, di, &mut olc);
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().type_ = number;
            if number == AQ_OBJ_RETURN {
                olc.mode = QEDIT_RETURNMOB;
                write_to_desc(g, di, b"Enter mob vnum to return object to : ");
                return Some(olc);
            }
        }
        QEDIT_FLAGS => {
            if !(0..=NUM_AQ_FLAGS).contains(&number) {
                write_to_desc(g, di, b"That is not a valid choice!\r\n");
                qedit_disp_flag_menu(g, di, &mut olc);
            } else if number == 0 {
                // Fall through to the menu below.
                olc.value = 1;
                qedit_disp_menu(g, di, &mut olc);
                return Some(olc);
            } else {
                // TOGGLE_BIT with the menu index, not (1 << index) — the
                // two agree for the table's single flag.
                olc.quest.as_mut().unwrap().flags ^= number as u32;
                qedit_disp_flag_menu(g, di, &mut olc);
            }
            return Some(olc);
        }
        QEDIT_QUANTITY => olc.quest.as_mut().unwrap().obj_out = limit(number, 1, 50),
        QEDIT_POINTSCOMP => olc.quest.as_mut().unwrap().value = limit(number, 0, 999999),
        QEDIT_POINTSQUIT => olc.quest.as_mut().unwrap().penalty = limit(number, 0, 999999),
        QEDIT_PREREQ => {
            if number != -1 && obj_of(g, idx_store(number)).is_none() {
                write_to_desc(g, di, b"That object does not exist, try again : ");
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().prereq = idx_store(number);
        }
        QEDIT_LEVELMIN => {
            if number < 0 || number > LVL_IMPL as i32 {
                let msg = format!("Level must be between 0 and {}!\r\n", LVL_IMPL);
                write_to_desc(g, di, msg.as_bytes());
                write_to_desc(g, di, b"Enter minimum level to accept the quest : ");
                return Some(olc);
            } else if number > olc.quest.as_ref().unwrap().max_level {
                write_to_desc(g, di, b"Minimum level can't be above maximum level!\r\n");
                write_to_desc(g, di, b"Enter minimum level to accept the quest : ");
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().min_level = number;
        }
        QEDIT_LEVELMAX => {
            if number < 0 || number > LVL_IMPL as i32 {
                let msg = format!("Level must be between 0 and {}!\r\n", LVL_IMPL);
                write_to_desc(g, di, msg.as_bytes());
                write_to_desc(g, di, b"Enter maximum level to accept the quest : ");
                return Some(olc);
            } else if number < olc.quest.as_ref().unwrap().min_level {
                write_to_desc(g, di, b"Maximum level can't be below minimum level!\r\n");
                write_to_desc(g, di, b"Enter maximum level to accept the quest : ");
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().max_level = number;
        }
        QEDIT_TIMELIMIT => olc.quest.as_mut().unwrap().time = limit(number, -1, 100),
        QEDIT_RETURNMOB => {
            if number != -1 && mob_of(g, idx_store(number)).is_none() {
                write_to_desc(g, di, b"That mobile does not exist, try again : ");
                return Some(olc);
            }
            // value[5] is an int, so a -1 answer stays -1 here.
            olc.quest.as_mut().unwrap().obj_in = number;
        }
        QEDIT_TARGET => olc.quest.as_mut().unwrap().target = number,
        QEDIT_NEXTQUEST | QEDIT_PREVQUEST => {
            if number != -1 && real_quest(g, idx_store(number)).is_none() {
                write_to_desc(
                    g,
                    di,
                    b"That is not a valid quest, try again (-1 for none) : ",
                );
                return Some(olc);
            }
            let v = if number == -1 { NOTHING as i32 } else { idx_store(number) };
            if olc.mode == QEDIT_NEXTQUEST {
                olc.quest.as_mut().unwrap().next_quest = v;
            } else {
                olc.quest.as_mut().unwrap().prev_quest = v;
            }
        }
        QEDIT_GOLD => olc.quest.as_mut().unwrap().gold_reward = limit(number, 0, 99999),
        QEDIT_EXP => olc.quest.as_mut().unwrap().exp_reward = limit(number, 0, 99999),
        QEDIT_OBJ => {
            if number != -1 && obj_of(g, idx_store(number)).is_none() {
                write_to_desc(g, di, b"That object does not exist, try again : ");
                return Some(olc);
            }
            olc.quest.as_mut().unwrap().obj_reward = idx_store(number);
        }
        _ => {
            // We should never get here.
            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: qedit_parse(): Reached default case!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
            return None;
        }
    }

    // We have probably changed something, so back to the main menu.
    olc.value = 1;
    qedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

pub fn qedit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    match olc.str_target.take() {
        Some(StrTarget::QuestInfo) => olc.quest.as_mut().unwrap().info = text,
        Some(StrTarget::QuestDone) => olc.quest.as_mut().unwrap().done = text,
        Some(StrTarget::QuestQuit) => olc.quest.as_mut().unwrap().quit = text,
        _ => {}
    }
    if matches!(olc.mode, QEDIT_INFO | QEDIT_COMPLETE | QEDIT_ABANDON) {
        qedit_disp_menu(g, di, &mut olc);
    }
    Some(olc)
}
