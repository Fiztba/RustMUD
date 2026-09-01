//! The manual (ASPELL) spells: create water, recall, teleport,
//! summon, locate object, charm, identify, enchant weapon, detect poison.
//! (control weather is registered but has no effect: casting it costs mana
//! and says the spell, and nothing more.)

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::spells::SPELL_CHARM;
use mud_data::types::*;

use crate::act::movement::{add_follower, circle_follow, stop_follower, zone_flagged};
use crate::ch::Affect;
use crate::comm::{self, act, send_to_char};
use crate::game::{Game, MudlogKind};
use crate::handler::{char_from_room, char_to_room, obj_name, obj_short, pers};

const LIQ_WATER: i32 = 0;
const LIQ_SLIME: i32 = 9;

fn room_zone(g: &Game, room: RoomRnum) -> ZoneRnum {
    g.world.rooms[room as usize].zone
}

fn room_flagged(g: &Game, room: RoomRnum, bit: usize) -> bool {
    room != NOWHERE && g.world.rooms[room as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0
}

pub fn spell_create_water(g: &mut Game, _level: i32, chid: CharId, _victim: Option<CharId>, obj: Option<ObjId>) {
    let Some(oid) = obj else { return };
    if g.try_obj(oid).is_none() {
        return;
    }

    if g.obj(oid).type_flag == flags::ITEM_DRINKCON {
        if g.obj(oid).values[2] != LIQ_WATER && g.obj(oid).values[1] != 0 {
            crate::act::item::name_from_drinkcon(g, oid);
            g.obj_mut(oid).values[2] = LIQ_SLIME;
            crate::act::item::name_to_drinkcon(g, oid, LIQ_SLIME);
        } else {
            let water = (g.obj(oid).values[0] - g.obj(oid).values[1]).max(0);
            if water > 0 {
                if g.obj(oid).values[1] >= 0 {
                    crate::act::item::name_from_drinkcon(g, oid);
                }
                g.obj_mut(oid).values[2] = LIQ_WATER;
                g.obj_mut(oid).values[1] += water;
                crate::act::item::name_to_drinkcon(g, oid, LIQ_WATER);
                crate::act::item::weight_change_object(g, oid, water);
                act(g, b"$p is filled.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
            }
        }
    }
}

/// spell_recall — word of recall.
pub fn spell_recall(g: &mut Game, _level: i32, chid: CharId, victim: Option<CharId>, _obj: Option<ObjId>) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() || g.ch(victim).is_npc() {
        return;
    }

    if zone_flagged(g, room_zone(g, g.ch(victim).in_room), flags::ZONE_NOASTRAL) {
        // No trailing newline here.
        send_to_char(g, chid, b"A bright flash prevents your spell from working!");
        return;
    }

    act(g, b"$n disappears.", true, Some(victim), None, None, comm::TO_ROOM);
    char_from_room(g, victim);
    let dest = g.r_mortal_start_room;
    char_to_room(g, victim, dest);
    act(g, b"$n appears in the middle of the room.", true, Some(victim), None, None, comm::TO_ROOM);
    crate::act::informative::look_at_room(g, victim, false);
    crate::dg::triggers::entry_memory_mtrigger(g, victim);
    crate::dg::triggers::greet_mtrigger(g, victim, -1);
    crate::dg::triggers::greet_memory_mtrigger(g, victim);
}

pub fn spell_teleport(g: &mut Game, _level: i32, chid: CharId, victim: Option<CharId>, _obj: Option<ObjId>) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() || g.ch(victim).is_npc() {
        return;
    }

    if zone_flagged(g, room_zone(g, g.ch(victim).in_room), flags::ZONE_NOASTRAL) {
        send_to_char(g, chid, b"A bright flash prevents your spell from working!");
        return;
    }

    let top_of_world = (g.world.rooms.len() - 1) as i32;
    let to_room = loop {
        let r = g.rng.rand_number(0, top_of_world) as RoomRnum;
        let bad = room_flagged(g, r, flags::ROOM_PRIVATE)
            || room_flagged(g, r, flags::ROOM_DEATH)
            || room_flagged(g, r, flags::ROOM_GODROOM)
            || zone_flagged(g, room_zone(g, r), flags::ZONE_CLOSED)
            || zone_flagged(g, room_zone(g, r), flags::ZONE_NOASTRAL);
        if !bad {
            break r;
        }
    };

    act(g, b"$n slowly fades out of existence and is gone.", false, Some(victim), None, None, comm::TO_ROOM);
    char_from_room(g, victim);
    char_to_room(g, victim, to_room);
    act(g, b"$n slowly fades into existence.", false, Some(victim), None, None, comm::TO_ROOM);
    crate::act::informative::look_at_room(g, victim, false);
    crate::dg::triggers::entry_memory_mtrigger(g, victim);
    crate::dg::triggers::greet_mtrigger(g, victim, -1);
    crate::dg::triggers::greet_memory_mtrigger(g, victim);
}

const SUMMON_FAIL: &[u8] = b"You failed.\r\n";

pub fn spell_summon(g: &mut Game, level: i32, chid: CharId, victim: Option<CharId>, _obj: Option<ObjId>) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() {
        return;
    }

    if g.ch(victim).level as i32 > (LVL_IMMORT as i32 - 1).min(level + 3) {
        send_to_char(g, chid, SUMMON_FAIL);
        return;
    }

    if zone_flagged(g, room_zone(g, g.ch(victim).in_room), flags::ZONE_NOASTRAL)
        || zone_flagged(g, room_zone(g, g.ch(chid).in_room), flags::ZONE_NOASTRAL)
    {
        send_to_char(g, chid, b"A bright flash prevents your spell from working!");
        return;
    }

    if g.config.pk_setting == 0 {
        if g.ch(victim).mob_flagged(flags::MOB_AGGRESSIVE) {
            act(
                g,
                b"As the words escape your lips and $N travels\r\nthrough time and space towards you, you realize that $E is\r\naggressive and might harm you, so you wisely send $M back.",
                false,
                Some(chid),
                None,
                Some(victim),
                comm::TO_CHAR,
            );
            return;
        }
        if !g.ch(victim).is_npc()
            && !g.ch(victim).prf(flags::PRF_SUMMONABLE)
            && !g.ch(victim).plr(flags::PLR_KILLER)
        {
            let caster_name = g.ch(chid).get_name().to_vec();
            let room = g.ch(chid).in_room;
            let room_name = g.world.rooms[room as usize].name.clone().unwrap_or_default();
            let mut msg = caster_name.clone();
            msg.extend_from_slice(b" just tried to summon you to: ");
            msg.extend_from_slice(&room_name);
            msg.extend_from_slice(b".\r\nThis failed because you have summon protection on.\r\nType NOSUMMON to allow other players to summon you.\r\n");
            send_to_char(g, victim, &msg);

            let vict_name = g.ch(victim).get_name().to_vec();
            let mut msg = b"You failed because ".to_vec();
            msg.extend_from_slice(&vict_name);
            msg.extend_from_slice(b" has summon protection on.\r\n");
            send_to_char(g, chid, &msg);

            let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()).max(g.ch(victim).invis_lev()) as u8;
            let log_msg = format!(
                "{} failed summoning {} to {}.",
                String::from_utf8_lossy(&caster_name),
                String::from_utf8_lossy(&vict_name),
                String::from_utf8_lossy(&room_name)
            );
            g.mudlog(MudlogKind::Brf, lvl, true, &log_msg);
            return;
        }
    }

    if g.ch(victim).mob_flagged(flags::MOB_NOSUMMON)
        || (g.ch(victim).is_npc() && crate::magic::mag_savingthrow(g, victim, mud_data::spells::SAVING_SPELL, 0))
    {
        send_to_char(g, chid, SUMMON_FAIL);
        return;
    }

    act(g, b"$n disappears suddenly.", true, Some(victim), None, None, comm::TO_ROOM);
    char_from_room(g, victim);
    let dest = g.ch(chid).in_room;
    char_to_room(g, victim, dest);
    act(g, b"$n arrives suddenly.", true, Some(victim), None, None, comm::TO_ROOM);
    act(g, b"$n has summoned you!", false, Some(chid), None, Some(victim), comm::TO_VICT);
    crate::act::informative::look_at_room(g, victim, false);
    crate::dg::triggers::entry_memory_mtrigger(g, victim);
    crate::dg::triggers::greet_mtrigger(g, victim, -1);
    crate::dg::triggers::greet_memory_mtrigger(g, victim);
}

/// isname_obj: case-insensitive whole-word-start match
/// against the alias list (substring must start the list or follow a space).
fn isname_obj(search: &[u8], list: &[u8]) -> bool {
    if search.is_empty() {
        // strstr(list, "") == list, i.e. "found at start".
        return true;
    }
    let searchname = search.to_ascii_lowercase();
    let namelist = list.to_ascii_lowercase();

    let found_pos = namelist
        .windows(searchname.len())
        .position(|w| w == searchname.as_slice());
    let Some(found_pos) = found_pos else { return false };

    if namelist.starts_with(searchname.as_slice()) {
        return true;
    }
    found_pos > 0 && namelist[found_pos - 1] == b' '
}

#[cfg(test)]
mod tests {
    use super::isname_obj;

    #[test]
    fn isname_obj_first_strstr_hit_decides() {
        // Whole word at the start.
        assert!(isname_obj(b"ring", b"ring gold"));
        // Whole word after a space.
        assert!(isname_obj(b"ring", b"gold ring"));
        // Case-insensitive.
        assert!(isname_obj(b"RING", b"gold ring"));
        // Embedded substring only → no.
        assert!(!isname_obj(b"ring", b"shimmering"));
        // The quirk: strstr's FIRST hit is inside "shimmering", not the
        // stand-alone word later — the earlier embedded hit shadows it.
        assert!(!isname_obj(b"ring", b"shimmering ring"));
        assert!(!isname_obj(b"ring", b"boring"));
    }
}

/// spell_locate_object: search term is cast_arg2; up to
/// Half the caster level in results, walking the global (newest-first)
/// object list.
pub fn spell_locate_object(g: &mut Game, _level: i32, chid: CharId, _victim: Option<CharId>, obj: Option<ObjId>) {
    if obj.is_none() {
        send_to_char(g, chid, b"You sense nothing.\r\n");
        return;
    }

    let name = g.cast_arg2.clone();
    let mut j = g.ch(chid).level as i32 / 2;

    let object_list = g.object_list.clone();
    for oid in object_list {
        if j <= 0 {
            break;
        }
        if g.try_obj(oid).is_none() {
            continue;
        }
        if !isname_obj(&name, obj_name(g, oid)) {
            continue;
        }

        let shortd = obj_short(g, oid).to_vec();
        let mut line = shortd.clone();
        if let Some(first) = line.first_mut() {
            *first = first.to_ascii_uppercase();
        }

        let o_carried = g.obj(oid).carried_by;
        let o_room = g.obj(oid).in_room;
        let o_in = g.obj(oid).in_obj;
        let o_worn = g.obj(oid).worn_by;
        if let Some(carrier) = o_carried {
            line.extend_from_slice(b" is being carried by ");
            line.extend_from_slice(&pers(g, chid, carrier));
            line.extend_from_slice(b".\r\n");
        } else if o_room != NOWHERE {
            line.extend_from_slice(b" is in ");
            line.extend_from_slice(g.world.rooms[o_room as usize].name.as_deref().unwrap_or(b""));
            line.extend_from_slice(b".\r\n");
        } else if let Some(container) = o_in {
            line.extend_from_slice(b" is in ");
            line.extend_from_slice(&obj_short(g, container).to_vec());
            line.extend_from_slice(b".\r\n");
        } else if let Some(wearer) = o_worn {
            line.extend_from_slice(b" is being worn by ");
            line.extend_from_slice(&pers(g, chid, wearer));
            line.extend_from_slice(b".\r\n");
        } else {
            line.extend_from_slice(b"'s location is uncertain.\r\n");
        }
        send_to_char(g, chid, &line);
        j -= 1;
    }
}

/// spell_charm — gate order exactly.
pub fn spell_charm(g: &mut Game, level: i32, chid: CharId, victim: Option<CharId>, _obj: Option<ObjId>) {
    let Some(victim) = victim else { return };
    if g.try_ch(victim).is_none() {
        return;
    }

    if victim == chid {
        send_to_char(g, chid, b"You like yourself even better!\r\n");
    } else if !g.ch(victim).is_npc() && !g.ch(victim).prf(flags::PRF_SUMMONABLE) {
        send_to_char(g, chid, b"You fail because SUMMON protection is on!\r\n");
    } else if g.ch(victim).aff(flags::AFF_SANCTUARY) {
        send_to_char(g, chid, b"Your victim is protected by sanctuary!\r\n");
    } else if g.ch(victim).mob_flagged(flags::MOB_NOCHARM) {
        send_to_char(g, chid, b"Your victim resists!\r\n");
    } else if g.ch(chid).aff(flags::AFF_CHARM) {
        send_to_char(g, chid, b"You can't have any followers of your own!\r\n");
    } else if g.ch(victim).aff(flags::AFF_CHARM) || level < g.ch(victim).level as i32 {
        send_to_char(g, chid, b"You fail.\r\n");
    } else if g.config.pk_setting == 0 && !g.ch(victim).is_npc() {
        // player charming another player - no legal reason for this
        send_to_char(g, chid, b"You fail - shouldn't be doing it anyway.\r\n");
    } else if circle_follow(g, victim, chid) {
        send_to_char(g, chid, b"Sorry, following in circles is not allowed.\r\n");
    } else if crate::magic::mag_savingthrow(g, victim, mud_data::spells::SAVING_PARA, 0) {
        send_to_char(g, chid, b"Your victim resists!\r\n");
    } else {
        if g.ch(victim).master.is_some() {
            stop_follower(g, victim);
        }
        add_follower(g, victim, chid);

        let mut af = Affect { spell: SPELL_CHARM as i16, ..Default::default() };
        let mut duration = 24i32 * 2;
        let cha = g.ch(chid).aff_abils.cha as i32;
        if cha != 0 {
            duration *= cha;
        }
        let int = g.ch(victim).aff_abils.intel as i32;
        if int != 0 {
            duration /= int;
        }
        af.duration = duration as i16;
        af.bitvector.set(flags::AFF_CHARM);
        crate::handler::affect_to_char(g, victim, af);

        act(g, b"Isn't $n just such a nice fellow?", false, Some(chid), None, Some(victim), comm::TO_VICT);
        if g.ch(victim).is_npc() {
            g.ch_mut(victim).act.remove(flags::MOB_SPEC);
        }
    }
}

/// sprinttype: list lookup with "UNDEFINED" out-of-range.
fn sprinttype(idx: i32, names: &[&str]) -> Vec<u8> {
    if idx >= 0 && (idx as usize) < names.len() {
        names[idx as usize].as_bytes().to_vec()
    } else {
        b"UNDEFINED".to_vec()
    }
}

/// spell_identify.
pub fn spell_identify(g: &mut Game, _level: i32, chid: CharId, victim: Option<CharId>, obj: Option<ObjId>) {
    use mud_data::tables::{AFFECTED_BITS, APPLY_TYPES, EXTRA_BITS, ITEM_TYPES};

    if let Some(oid) = obj.filter(|&o| g.try_obj(o).is_some()) {
        let typename = sprinttype(g.obj(oid).type_flag, &ITEM_TYPES);
        let mut out = b"You feel informed:\r\nObject '".to_vec();
        out.extend_from_slice(&obj_short(g, oid).to_vec());
        out.extend_from_slice(b"', Item type: ");
        out.extend_from_slice(&typename);
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);

        {
            // `if (GET_OBJ_AFFECT(obj))` tests an int[4] ARRAY — always
            // true — so the abilities line always prints, "NOBITS" included.
            let mut bits = Vec::new();
            crate::act::informative::sprintbitarray(&g.obj(oid).perm_affects.0, &AFFECTED_BITS, &mut bits);
            let mut out = b"Item will give you following abilities:  ".to_vec();
            out.extend_from_slice(&bits);
            out.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &out);
        }

        let mut bits = Vec::new();
        crate::act::informative::sprintbitarray(&g.obj(oid).extra_flags.0, &EXTRA_BITS, &mut bits);
        let mut out = b"Item is: ".to_vec();
        out.extend_from_slice(&bits);
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);

        let o = g.obj(oid);
        let msg = format!(
            "Weight: {}, Value: {}, Rent: {}, Min. level: {}\r\n",
            o.weight, o.cost, o.cost_per_day, o.level
        );
        send_to_char(g, chid, msg.as_bytes());

        let t = g.obj(oid).type_flag;
        match t {
            flags::ITEM_SCROLL | flags::ITEM_POTION => {
                let mut names = Vec::new();
                for i in 1..=3 {
                    let v = g.obj(oid).values[i];
                    if v >= 1 {
                        names.push(b' ');
                        names.extend_from_slice(mud_data::spells::skill_name(v).as_bytes());
                    }
                }
                let mut out = b"This ".to_vec();
                out.extend_from_slice(&sprinttype(t, &ITEM_TYPES));
                out.extend_from_slice(b" casts: ");
                out.extend_from_slice(&names);
                out.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &out);
            }
            flags::ITEM_WAND | flags::ITEM_STAFF => {
                let o = g.obj(oid);
                let msg = format!(
                    "This {} casts: {}\r\nIt has {} maximum charge{} and {} remaining.\r\n",
                    String::from_utf8_lossy(&sprinttype(t, &ITEM_TYPES)),
                    mud_data::spells::skill_name(o.values[3]),
                    o.values[1],
                    if o.values[1] == 1 { "" } else { "s" },
                    o.values[2]
                );
                send_to_char(g, chid, msg.as_bytes());
            }
            flags::ITEM_WEAPON => {
                let o = g.obj(oid);
                let msg = format!(
                    "Damage Dice is '{}D{}' for an average per-round damage of {:.1}.\r\n",
                    o.values[1],
                    o.values[2],
                    ((o.values[2] + 1) as f64 / 2.0) * o.values[1] as f64
                );
                send_to_char(g, chid, msg.as_bytes());
            }
            flags::ITEM_ARMOR => {
                let msg = format!("AC-apply is {}\r\n", g.obj(oid).values[0]);
                send_to_char(g, chid, msg.as_bytes());
            }
            _ => {}
        }

        let mut found = false;
        for i in 0..MAX_OBJ_AFFECT {
            let a = g.obj(oid).affected[i];
            if a.location != flags::APPLY_NONE && a.modifier != 0 {
                if !found {
                    send_to_char(g, chid, b"Can affect you as :\r\n");
                    found = true;
                }
                let loc = sprinttype(a.location, &APPLY_TYPES);
                let mut out = b"   Affects: ".to_vec();
                out.extend_from_slice(&loc);
                out.extend_from_slice(format!(" By {}\r\n", a.modifier).as_bytes());
                send_to_char(g, chid, &out);
            }
        }
    } else if let Some(victim) = victim.filter(|&v| g.try_ch(v).is_some()) {
        let name = g.ch(victim).get_name().to_vec();
        let mut out = b"Name: ".to_vec();
        out.extend_from_slice(&name);
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);

        if !g.ch(victim).is_npc() {
            let a = crate::gametime::age(g.ch(victim).time.birth, g.now);
            let mut out = name.clone();
            out.extend_from_slice(
                format!(
                    " is {} years, {} months, {} days and {} hours old.\r\n",
                    a.year, a.month, a.day, a.hours
                )
                .as_bytes(),
            );
            send_to_char(g, chid, &out);
        }
        let (height, weight) = (g.ch(victim).height, g.ch(victim).weight);
        send_to_char(g, chid, format!("Height {} cm, Weight {} pounds\r\n", height, weight).as_bytes());
        let (level, hit, mana) = {
            let v = g.ch(victim);
            (v.level, v.points.hit, v.points.mana)
        };
        send_to_char(g, chid, format!("Level: {}, Hits: {}, Mana: {}\r\n", level, hit, mana).as_bytes());
        let ac = crate::act::informative::compute_armor_class(g, victim);
        let (hitroll, damroll) = (g.ch(victim).points.hitroll, g.ch(victim).points.damroll);
        send_to_char(g, chid, format!("AC: {}, Hitroll: {}, Damroll: {}\r\n", ac, hitroll, damroll).as_bytes());
        let ab = g.ch(victim).aff_abils;
        send_to_char(
            g,
            chid,
            format!(
                "Str: {}/{}, Int: {}, Wis: {}, Dex: {}, Con: {}, Cha: {}\r\n",
                ab.str_, ab.str_add, ab.intel, ab.wis, ab.dex, ab.con, ab.cha
            )
            .as_bytes(),
        );
    }
}

pub fn spell_enchant_weapon(g: &mut Game, level: i32, chid: CharId, _victim: Option<CharId>, obj: Option<ObjId>) {
    let Some(oid) = obj else { return };
    if g.try_obj(oid).is_none() {
        return;
    }

    // Either already enchanted or not a weapon.
    if g.obj(oid).type_flag != flags::ITEM_WEAPON || g.obj(oid).extra_flags.is_set(flags::ITEM_MAGIC) {
        return;
    }
    // Make sure no other affections.
    for i in 0..MAX_OBJ_AFFECT {
        if g.obj(oid).affected[i].location != flags::APPLY_NONE {
            return;
        }
    }

    g.obj_mut(oid).extra_flags.set(flags::ITEM_MAGIC);

    {
        let o = g.obj_mut(oid);
        o.affected[0].location = flags::APPLY_HITROLL;
        o.affected[0].modifier = 1 + if level >= 18 { 1 } else { 0 };
        o.affected[1].location = flags::APPLY_DAMROLL;
        o.affected[1].modifier = 1 + if level >= 20 { 1 } else { 0 };
    }

    let align = g.ch(chid).alignment;
    if align >= 350 {
        g.obj_mut(oid).extra_flags.set(flags::ITEM_ANTI_EVIL);
        act(g, b"$p glows blue.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
    } else if align <= -350 {
        g.obj_mut(oid).extra_flags.set(flags::ITEM_ANTI_GOOD);
        act(g, b"$p glows red.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
    } else {
        act(g, b"$p glows yellow.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
    }
}

pub fn spell_detect_poison(g: &mut Game, _level: i32, chid: CharId, victim: Option<CharId>, obj: Option<ObjId>) {
    if let Some(victim) = victim.filter(|&v| g.try_ch(v).is_some()) {
        if victim == chid {
            if g.ch(victim).aff(flags::AFF_POISON) {
                send_to_char(g, chid, b"You can sense poison in your blood.\r\n");
            } else {
                send_to_char(g, chid, b"You feel healthy.\r\n");
            }
        } else if g.ch(victim).aff(flags::AFF_POISON) {
            act(g, b"You sense that $E is poisoned.", false, Some(chid), None, Some(victim), comm::TO_CHAR);
        } else {
            act(g, b"You sense that $E is healthy.", false, Some(chid), None, Some(victim), comm::TO_CHAR);
        }
    }

    if let Some(oid) = obj.filter(|&o| g.try_obj(o).is_some()) {
        match g.obj(oid).type_flag {
            flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN | flags::ITEM_FOOD => {
                if g.obj(oid).values[3] != 0 {
                    act(g, b"You sense that $p has been contaminated.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
                } else {
                    act(g, b"You sense that $p is safe for consumption.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
                }
            }
            _ => {
                send_to_char(g, chid, b"You sense that it should not be consumed.\r\n");
            }
        }
    }
}
