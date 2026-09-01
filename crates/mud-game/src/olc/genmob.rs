//! The mobile half of the generic OLC library.
//!
//! Editing an existing prototype re-points every live instance's strings at
//! the new prototype — the bytes are copied, not aliased — so a medit
//! save is visible on mobs already walking around. Inserting shifts every
//! mob rnum: live mobs' `nr`, zone 'M' commands, and shopkeepers.

use mud_data::types::*;
use mud_world::model::MobProto;

use crate::db::{
    add_to_save_list, in_save_list, remove_from_save_list, write_world_file, SL_MOB, SL_ZON,
};
use crate::game::{Game, MudlogKind};

/// check_mobile_strings, in field order: an empty string is logged and
/// replaced with "An undefined string.\n". Works on a detached prototype,
/// which is what both callers have — `save_mobiles` (the table entry) and
/// `copy_mobile` (the OLC copy).
pub fn check_mobile_strings_of(g: &mut Game, mob: &mut MobProto, vnum: Idx) {
    for (slot, desc) in [
        (0usize, "long description"),
        (1, "detailed description"),
        (2, "alias list"),
        (3, "short description"),
    ] {
        let s = match slot {
            0 => &mut mob.long_descr,
            1 => &mut mob.ddescription,
            2 => &mut mob.keywords,
            _ => &mut mob.short_descr,
        };
        if s.as_ref().is_none_or(|v| v.is_empty()) {
            *s = Some(b"An undefined string.\n".to_vec());
            let msg = format!("GenOLC: Mob #{} has an invalid {}.", vnum, desc);
            g.mudlog(MudlogKind::Brf, LVL_GOD, true, &msg);
        }
    }
}

/// The same, applied in place to a prototype in the table.
pub fn check_mobile_strings(g: &mut Game, rmob: usize) {
    let mut proto = g.world.mob_protos[rmob].clone();
    let vnum = proto.vnum;
    check_mobile_strings_of(g, &mut proto, vnum);
    g.world.mob_protos[rmob] = proto;
}

/// update_mobile_strings: live instances take the
/// prototype's strings. Only fields the prototype actually has are copied —
/// mob prototypes never carry a title, so live titles survive.

/// add_mobile. Returns the rnum, or `None` for NOBODY.
pub fn add_mobile(g: &mut Game, mob: &MobProto, vnum: Idx) -> Option<Idx> {
    if let Some(rnum) = g.world.real_mobile(vnum) {
        let mut copy = mob.clone();
        copy.vnum = vnum;
        check_mobile_strings_of(g, &mut copy, vnum);
        g.world.mob_protos[rnum as usize] = copy;
        // Mobs already in the world are left alone. A mob keeps the name
        // and descriptions it was read with: a builder editing a mobile
        // must not rewrite one a player is already fighting. The edit
        // reaches the next mob read from this prototype.
        //
        // The C re-points those strings instead, because a live mob shares
        // the prototype's pointers and `copy_mobile` frees them -- leaving
        // them dangling otherwise. It detaches them now (B92). Nothing to
        // detach here: every string on a mob is already its own `Vec`.
        if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32) {
            let zvnum = g.world.zones[z].number;
            add_to_save_list(g, zvnum, SL_MOB);
        }
        g.log(format!("GenOLC: add_mobile: Updated existing mobile #{}.", vnum));
        return Some(rnum);
    }

    let old_len = g.world.mob_protos.len();
    let mut found: usize = 0;
    for i in (1..=old_len).rev() {
        if vnum > g.world.mob_protos[i - 1].vnum {
            found = i;
            break;
        }
    }
    let mut copy = mob.clone();
    copy.vnum = vnum;
    g.world.mob_protos.insert(found, copy);
    g.mob_counts.insert(found, 0);
    g.mob_specs.insert(found, None);
    for v in g.world.mob_map.values_mut() {
        if *v as usize >= found {
            *v += 1;
        }
    }
    g.world.mob_map.insert(vnum, found as Idx);

    g.log(format!("GenOLC: add_mobile: Added mobile {} at index #{}.", vnum, found));

    // Live mobile rnums.
    for id in g.character_list.clone() {
        if let Some(c) = g.chars.get_mut(id) {
            if c.mob_rnum != NOBODY && c.mob_rnum as usize >= found {
                c.mob_rnum += 1;
            }
        }
    }
    // Zone 'M' commands, every zone.
    for zi in 0..g.world.zones.len() {
        for cmd in g.world.zones[zi].cmds.iter_mut() {
            if cmd.command == b'M' && cmd.arg1 >= found as i32 {
                cmd.arg1 += 1;
            }
        }
    }
    // Shop keepers.
    for s in g.shops_rt.iter_mut() {
        if s.keeper != NOBODY && s.keeper as usize >= found {
            s.keeper += 1;
        }
    }

    if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32) {
        let zvnum = g.world.zones[z].number;
        add_to_save_list(g, zvnum, SL_MOB);
    }
    Some(found as Idx)
}

/// extract_mobile_all: every live instance of this vnum.
fn extract_mobile_all(g: &mut Game, vnum: Idx) {
    for id in g.character_list.clone() {
        let is_target = g
            .chars
            .get(id)
            .map(|c| {
                c.mob_rnum != NOBODY
                    && g.world
                        .mob_protos
                        .get(c.mob_rnum as usize)
                        .map(|p| p.vnum)
                        == Some(vnum)
            })
            .unwrap_or(false);
        if is_target {
            crate::handler::extract_char(g, id);
        }
    }
}

/// delete_mobile. Returns the rnum deleted.
pub fn delete_mobile(g: &mut Game, refpt: Idx) -> Option<Idx> {
    if refpt == NOBODY || refpt as usize >= g.world.mob_protos.len() {
        g.log(format!("SYSERR: GenOLC: delete_mobile: Invalid rnum {}.", refpt));
        return None;
    }
    let vnum = g.world.mob_protos[refpt as usize].vnum;

    extract_mobile_all(g, vnum);
    // Calling extract_char on the *prototype* would touch something
    // that is not in
    // character_list — the count went up for a character
    // extract_pending_chars could never find, logging "Couldn't find 1
    // extractions as counted." once per deletion. The table entry is
    // overwritten by the shift below regardless.

    g.world.mob_protos.remove(refpt as usize);
    g.mob_counts.remove(refpt as usize);
    g.mob_specs.remove(refpt as usize);
    g.world.mob_map.remove(&vnum);
    for v in g.world.mob_map.values_mut() {
        if *v > refpt {
            *v -= 1;
        }
    }

    // Live mobile rnums.
    for id in g.character_list.clone() {
        if let Some(c) = g.chars.get_mut(id) {
            if c.mob_rnum >= refpt && c.mob_rnum != NOBODY {
                c.mob_rnum -= 1;
            }
        }
    }
    // Zone 'M' commands: the ones loading this mob are removed outright.
    // Advance past the shifted-in command after a delete. B26: the
    // zones whose tables change here need writing back out too.
    let mut touched: Vec<Idx> = Vec::new();
    for zi in 0..g.world.zones.len() {
        let mut ci = 0usize;
        let mut zone_touched = false;
        while ci < g.world.zones[zi].cmds.len() {
            let cmd = &mut g.world.zones[zi].cmds[ci];
            if cmd.command == b'M' {
                if cmd.arg1 == refpt as i32 {
                    g.world.zones[zi].cmds.remove(ci);
                    zone_touched = true;
                } else if cmd.arg1 > refpt as i32 {
                    cmd.arg1 -= 1;
                    zone_touched = true;
                }
            }
            ci += 1;
        }
        if zone_touched {
            touched.push(g.world.zones[zi].number);
        }
    }
    for zvnum in touched {
        add_to_save_list(g, zvnum, SL_ZON);
    }
    // Shop keepers.
    for s in g.shops_rt.iter_mut() {
        if s.keeper >= refpt && s.keeper != NOBODY {
            s.keeper -= 1;
        }
    }

    // Flag rather than write — medit's delete branch decides whether
    // this goes to disk now, honouring the OLC autosave toggle.
    if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32) {
        let zvnum = g.world.zones[z].number;
        add_to_save_list(g, zvnum, SL_MOB);
    }

    Some(refpt)
}

pub fn save_mobiles(g: &mut Game, rznum: Option<usize>) -> bool {
    let top = g.world.zones.len().saturating_sub(1);
    let Some(rznum) = rznum.filter(|&z| z < g.world.zones.len()) else {
        g.log(format!(
            "SYSERR: GenOLC: save_mobiles: Invalid real zone number {}. (0-{})",
            NOWHERE, top
        ));
        return false;
    };
    let vznum = g.world.zones[rznum].number;

    // check_mobile_strings runs over every mob in the window before the
    // file is rendered.
    let (bot, ztop) = (g.world.zones[rznum].bot, g.world.zones[rznum].top);
    for i in bot..=ztop {
        if let Some(rmob) = g.world.real_mobile(i) {
            check_mobile_strings(g, rmob as usize);
        }
        if i == u16::MAX {
            break;
        }
    }

    let Some(written) = write_world_file(g, rznum, SL_MOB) else {
        g.mudlog(
            MudlogKind::Brf,
            LVL_GOD,
            true,
            "SYSERR: GenOLC: Cannot open mob file for writing.",
        );
        return false;
    };
    if in_save_list(g, vznum, SL_MOB) {
        remove_from_save_list(g, vznum, SL_MOB);
    }
    g.log(format!(
        "GenOLC: 'world/mob/{}.mob' saved, {} bytes written.",
        vznum, written
    ));
    true
}
