//! The quest half of the OLC generic layer.
//!
//! Two representation notes:
//!
//! * The questmaster's displaced spec proc lives in
//! `Game::quest_secondary`, a Vec parallel to `world.quests`, rather than
//! inside the quest row — so every shift here has to move both.
//! * `copy_quest` defaults all five strings, so an absent or empty one
//! becomes "undefined" the moment a quest is copied into or out of the
//! table. That is where `qedit`'s "Nothing" placeholders come from.
//!
//! B54 is fixed here: `delete_quest` read `QST_MASTER(rnum)` in a
//! declaration initialiser, *before* the bounds guard below it, and then
//! restore the displaced spec through a vnum where an rnum is wanted.

use mud_world::model::Quest;

use crate::db::{add_to_save_list, in_save_list, remove_from_save_list, write_world_file, SL_QST};
use crate::game::{Game, MudlogKind};
use crate::olc::str_udup;
use crate::quest::real_quest;
use crate::spec::MobSpec;
use mud_data::types::*;

/// Every string is defaulted, so absent and empty both land as
/// "undefined".
fn copy_quest_strings(to: &mut Quest, from: &Quest) {
    for (dst, src) in [
        (&mut to.name, &from.name),
        (&mut to.desc, &from.desc),
        (&mut to.info, &from.info),
        (&mut to.done, &from.done),
        (&mut to.quit, &from.quit),
    ] {
        *dst = Some(str_udup(src.as_deref().unwrap_or(b"")));
    }
}

/// A clone, then the defaulting pass over the strings.
pub fn copy_quest(from: &Quest) -> Quest {
    let mut to = from.clone();
    copy_quest_strings(&mut to, from);
    to
}

/// add_quest. Returns the quest's rnum.
///
/// `func` is the scratch copy's `QST_FUNC` — what `qedit_setup_existing`
/// carried out of the table, or None for a new quest. Runtime assignment
/// is derived from the current NPC instead of this potentially stale copy.
pub fn add_quest(g: &mut Game, nqst: &Quest, _func: Option<MobSpec>) -> usize {
    let rznum = crate::dg::mobcmd::real_zone_by_thing(g, nqst.vnum as i32);

    let old_master = real_quest(g, nqst.vnum as i32).map(|r| g.world.quests[r].qm_vnum);
    let old_secondary = old_master.and_then(|qm| crate::quest::questmaster_secondary(g, qm));
    let secondary = crate::quest::questmaster_secondary(g, nqst.qm_vnum);
    let rnum = match real_quest(g, nqst.vnum as i32) {
        // The quest already exists, just update it.
        Some(rnum) => {
            g.world.quests[rnum] = copy_quest(nqst);
            g.quest_secondary[rnum] = secondary;
            rnum
        }
        None => {
            // total_quests++ / RECREATE, then walk down from the top
            // shifting rows up until the row below holds a smaller vnum.
            // Bubbling the new row down does the same rearranging.
            g.world.quests.push(copy_quest(nqst));
            g.quest_secondary.push(secondary);
            let mut rnum = g.world.quests.len() - 1;
            while rnum > 0 && nqst.vnum <= g.world.quests[rnum - 1].vnum {
                g.world.quests.swap(rnum, rnum - 1);
                g.quest_secondary.swap(rnum, rnum - 1);
                rnum -= 1;
            }
            rnum
        }
    };

    if let Some(old) = old_master.filter(|&qm| qm != nqst.qm_vnum) {
        crate::quest::update_questmaster_spec(g, old, old_secondary);
    }
    crate::quest::update_questmaster_spec(g, nqst.qm_vnum, secondary);

    // And make sure we save the updated quest information to disk.
    match rznum {
        Some(z) => {
            let number = g.world.zones[z].number;
            add_to_save_list(g, number, SL_QST);
        }
        None => g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: GenOLC: Cannot determine quest zone.",
        ),
    }

    rnum
}

pub fn delete_quest(g: &mut Game, rnum: usize) -> bool {
    // Reading QST_MASTER(rnum) above this check would index the table
    // at 65535 for a quest that was never added (real_quest -> NOTHING),
    // before deciding not to delete anything.
    if rnum >= g.world.quests.len() {
        return false;
    }
    let qm = g.world.quests[rnum].qm_vnum;
    let vnum = g.world.quests[rnum].vnum;
    let rznum = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32);
    let name = g.world.quests[rnum]
        .name
        .as_ref()
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_else(|| "(null)".to_string());
    g.log(format!("GenOLC: delete_quest: Deleting quest #{} ({}).", vnum, name));

    // Make a note of the quest master's secondary spec proc.
    let tempfunc = crate::quest::questmaster_secondary(g, qm);

    g.world.quests.remove(rnum);
    g.quest_secondary.remove(rnum);

    match rznum {
        Some(z) => {
            let number = g.world.zones[z].number;
            add_to_save_list(g, number, SL_QST);
        }
        None => g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: GenOLC: Cannot determine quest zone.",
        ),
    }

    crate::quest::update_questmaster_spec(g, qm, tempfunc);
    true
}

pub fn save_quests(g: &mut Game, zone_num: Option<usize>) -> bool {
    let top = g.world.zones.len().saturating_sub(1);
    let Some(zone_num) = zone_num.filter(|&z| z < g.world.zones.len()) else {
        g.log(format!(
            "SYSERR: GenOLC: save_quests: Invalid zone number {} passed! (0-{})",
            NOWHERE, top
        ));
        return false;
    };
    let vznum = g.world.zones[zone_num].number;
    // Zone rnum 0 reports 0 rather than its bot.
    let bot = if zone_num == 0 { 0 } else { g.world.zones[zone_num].bot };
    let ztop = g.world.zones[zone_num].top;
    g.log(format!(
        "GenOLC: save_quests: Saving quests in zone #{} ({}-{}).",
        vznum, bot, ztop
    ));

    let num_quests = (bot..=ztop).filter(|&v| real_quest(g, v as i32).is_some()).count();

    // No player-visible message on a failed write.
    if write_world_file(g, zone_num, SL_QST).is_none() {
        return false;
    }
    if num_quests > 0 {
        crate::olc::genzon::create_world_index(g, vznum as i32, "qst");
    }
    if in_save_list(g, vznum, SL_QST) {
        remove_from_save_list(g, vznum, SL_QST);
    }
    true
}
