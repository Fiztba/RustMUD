//! Say_spell obfuscation, do_cast parsing/targeting,
//! cast_spell gates, call_magic dispatch, and mag_objectmagic (the magic-item
//! entry point). The spell_info table itself lives in mud_data::spells.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::spells::{
    self, skill_name, spell_info, CAST_POTION, CAST_SCROLL, CAST_SPELL, CAST_STAFF, CAST_WAND,
    DEFAULT_STAFF_LVL, DEFAULT_WAND_LVL, MAG_AFFECTS, MAG_ALTER_OBJS, MAG_AREAS, MAG_CREATIONS,
    MAG_DAMAGE, MAG_GROUPS, MAG_MANUAL, MAG_MASSES, MAG_POINTS, MAG_ROOMS, MAG_SUMMONS,
    MAG_UNAFFECTS, MAX_SPELLS, SAVING_BREATH, SAVING_ROD, SAVING_SPELL, TAR_CHAR_ROOM,
    TAR_CHAR_WORLD, TAR_FIGHT_SELF, TAR_FIGHT_VICT, TAR_IGNORE, TAR_NOT_SELF, TAR_OBJ_EQUIP,
    TAR_OBJ_INV, TAR_OBJ_ROOM, TAR_OBJ_WORLD, TAR_SELF_ONLY, TOP_SPELL_DEFINE,
};
use mud_data::types::*;

use crate::comm::{self, act, send_to_char};
use crate::game::Game;
use crate::handler::{
    generic_find, get_char_room_vis_counted, get_char_world_vis, get_number,
    get_obj_in_list_vis_counted, get_obj_vis_counted, isname, obj_name, FIND_CHAR_ROOM,
    FIND_OBJ_EQUIP, FIND_OBJ_INV, FIND_OBJ_ROOM,
};
use crate::interpreter::{one_argument, skip_spaces};

/// The syllable table; first match wins, table order
/// is load-bearing ("ar"+"mor" → "abrazak").
const SYLS: &[(&[u8], &[u8])] = &[
    (b" ", b" "),
    (b"ar", b"abra"),
    (b"ate", b"i"),
    (b"cau", b"kada"),
    (b"blind", b"nose"),
    (b"bur", b"mosa"),
    (b"cu", b"judi"),
    (b"de", b"oculo"),
    (b"dis", b"mar"),
    (b"ect", b"kamina"),
    (b"en", b"uns"),
    (b"gro", b"cra"),
    (b"light", b"dies"),
    (b"lo", b"hi"),
    (b"magi", b"kari"),
    (b"mon", b"bar"),
    (b"mor", b"zak"),
    (b"move", b"sido"),
    (b"ness", b"lacri"),
    (b"ning", b"illa"),
    (b"per", b"duda"),
    (b"ra", b"gru"),
    (b"re", b"candus"),
    (b"son", b"sabru"),
    (b"tect", b"infra"),
    (b"tri", b"cula"),
    (b"ven", b"nofo"),
    (b"word of", b"inset"),
    (b"a", b"i"),
    (b"b", b"v"),
    (b"c", b"q"),
    (b"d", b"m"),
    (b"e", b"o"),
    (b"f", b"y"),
    (b"g", b"t"),
    (b"h", b"p"),
    (b"i", b"u"),
    (b"j", b"y"),
    (b"k", b"t"),
    (b"l", b"r"),
    (b"m", b"w"),
    (b"n", b"b"),
    (b"o", b"a"),
    (b"p", b"s"),
    (b"q", b"d"),
    (b"r", b"f"),
    (b"s", b"g"),
    (b"t", b"h"),
    (b"u", b"e"),
    (b"v", b"z"),
    (b"w", b"x"),
    (b"x", b"n"),
    (b"y", b"l"),
    (b"z", b"k"),
];

pub fn mag_manacost(g: &Game, chid: CharId, spellnum: i32) -> i32 {
    let info = spell_info(spellnum);
    let class = g.ch(chid).class.clamp(0, 3) as usize;
    let level = g.ch(chid).level as i32;
    (info.mana_max - (info.mana_change * (level - info.min_level[class]))).max(info.mana_min)
}

/// The pure scan behind obfuscate_spell: left-to-right first-match against
/// SYLS with a 200-byte output cap; returns the garbled text plus any log
/// lines (overflow / unmatched byte).
fn obfuscate_core(unobfuscated: &[u8]) -> (Vec<u8>, Vec<String>) {
    let mut out: Vec<u8> = Vec::new();
    let mut warnings = Vec::new();
    let mut maxlen: i64 = 200;
    let mut ofs = 0usize;
    while ofs < unobfuscated.len() {
        let mut matched = false;
        for &(org, news) in SYLS {
            if unobfuscated[ofs..].starts_with(org) {
                if (news.len() as i64) < maxlen {
                    out.extend_from_slice(news);
                    maxlen -= news.len() as i64;
                } else {
                    warnings.push(format!(
                        "No room in obfuscated version of '{}' (currently obfuscated to '{}') to add syllable '{}'.",
                        String::from_utf8_lossy(unobfuscated),
                        String::from_utf8_lossy(&out),
                        String::from_utf8_lossy(news)
                    ));
                }
                ofs += org.len();
                matched = true;
                break;
            }
        }
        if !matched {
            warnings.push(format!(
                "No entry in syllable table for substring of '{}' starting at '{}'.",
                String::from_utf8_lossy(unobfuscated),
                String::from_utf8_lossy(&unobfuscated[ofs..])
            ));
            ofs += 1;
        }
    }
    (out, warnings)
}

pub fn obfuscate_spell(g: &mut Game, unobfuscated: &[u8]) -> Vec<u8> {
    let (out, warnings) = obfuscate_core(unobfuscated);
    for w in warnings {
        g.log(w);
    }
    out
}

/// say_spell: observers of the caster's class hear
/// the real name, everyone else the obfuscated one. Only descriptors see it
/// (mobs are skipped by the `!i->desc` gate).
fn say_spell(g: &mut Game, chid: CharId, spellnum: i32, tch: Option<CharId>, tobj: Option<ObjId>) {
    let spell = skill_name(spellnum).as_bytes().to_vec();
    let obfuscated = obfuscate_spell(g, &spell);

    let room = g.ch(chid).in_room;
    let format: &[u8] = if tch.is_some_and(|t| g.try_ch(t).is_some_and(|c| c.in_room == room)) {
        if tch == Some(chid) {
            b"$n closes $s eyes and utters the words, '%s'."
        } else {
            b"$n stares at $N and utters the words, '%s'."
        }
    } else if tobj.is_some_and(|o| {
        let ob = g.obj(o);
        ob.in_room == room || ob.carried_by == Some(chid)
    }) {
        b"$n stares at $p and utters the words, '%s'."
    } else {
        b"$n utters the words, '%s'."
    };

    let fill = |words: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(format.len() + words.len());
        let mut i = 0;
        while i < format.len() {
            if format[i] == b'%' && format.get(i + 1) == Some(&b's') {
                out.extend_from_slice(words);
                i += 2;
            } else {
                out.push(format[i]);
                i += 1;
            }
        }
        out
    };
    let buf_original = fill(&spell);
    let buf_obfuscated = fill(&obfuscated);

    let caster_class = g.ch(chid).class;
    let people = g.rooms[room as usize].people.clone();
    let varg = match tch {
        Some(t) => comm::ActArg::Char(t),
        None => comm::ActArg::None,
    };
    for i in people {
        if i == chid || Some(i) == tch {
            continue;
        }
        let Some(ic) = g.try_ch(i) else { continue };
        if ic.desc.is_none() || !ic.awake() {
            continue;
        }
        if ic.class == caster_class {
            comm::perform_act(g, &buf_original, Some(chid), tobj, varg, i);
        } else {
            comm::perform_act(g, &buf_obfuscated, Some(chid), tobj, varg, i);
        }
    }

    if let Some(t) = tch {
        if t != chid && g.try_ch(t).is_some_and(|c| c.in_room == g.ch(chid).in_room) {
            let words = if g.ch(t).class == caster_class { &spell } else { &obfuscated };
            let mut msg = b"$n stares at you and utters the words, '".to_vec();
            msg.extend_from_slice(words);
            msg.extend_from_slice(b"'.");
            act(g, &msg, false, Some(chid), None, Some(t), comm::TO_VICT);
        }
    }
}

/// call_magic — the heart of the magic system.
pub fn call_magic(
    g: &mut Game,
    caster: CharId,
    cvict: Option<CharId>,
    ovict: Option<ObjId>,
    spellnum: i32,
    level: i32,
    casttype: i32,
) -> i32 {
    if spellnum < 1 || spellnum > TOP_SPELL_DEFINE {
        return 0;
    }

    // Cast triggers: wld, then obj, then mob.
    if crate::dg::triggers::cast_wtrigger(g, caster, cvict, ovict, spellnum) == 0 {
        return 0;
    }
    if let Some(ov) = ovict {
        if crate::dg::triggers::cast_otrigger(g, caster, ov, spellnum) == 0 {
            return 0;
        }
    }
    if let Some(cv) = cvict {
        if crate::dg::triggers::cast_mtrigger(g, caster, cv, spellnum) == 0 {
            return 0;
        }
    }

    let room = g.ch(caster).in_room;
    let room_flag = |g: &Game, bit: usize| {
        room != NOWHERE
            && g.world.rooms[room as usize].room_flags[bit / 32] & (1 << (bit % 32)) != 0
    };
    let info = *spell_info(spellnum);

    if room_flag(g, flags::ROOM_NOMAGIC) {
        send_to_char(g, caster, b"Your magic fizzles out and dies.\r\n");
        act(g, b"$n's magic fizzles out and dies.", false, Some(caster), None, None, comm::TO_ROOM);
        return 0;
    }
    if room_flag(g, flags::ROOM_PEACEFUL) && (info.violent || info.routines & MAG_DAMAGE != 0) {
        send_to_char(
            g,
            caster,
            b"A flash of white light fills the room, dispelling your violent magic!\r\n",
        );
        act(
            g,
            b"White light from no particular source suddenly fills the room, then vanishes.",
            false,
            Some(caster),
            None,
            None,
            comm::TO_ROOM,
        );
        return 0;
    }
    if cvict.is_some_and(|v| g.try_ch(v).is_some_and(|c| c.mob_flagged(flags::MOB_NOKILL))) {
        send_to_char(g, caster, b"This mob is protected.\r\n");
        return 0;
    }

    let savetype = match casttype {
        CAST_STAFF | CAST_SCROLL | CAST_POTION | CAST_WAND => SAVING_ROD,
        CAST_SPELL => SAVING_SPELL,
        _ => SAVING_BREATH,
    };

    if info.routines & MAG_DAMAGE != 0
        && crate::magic::mag_damage(g, level, caster, cvict, spellnum, savetype) == -1
    {
        return -1; // Successful and target died, don't cast again.
    }
    if info.routines & MAG_AFFECTS != 0 {
        crate::magic::mag_affects(g, level, caster, cvict, spellnum, savetype);
    }
    if info.routines & MAG_UNAFFECTS != 0 {
        crate::magic::mag_unaffects(g, level, caster, cvict, spellnum, savetype);
    }
    if info.routines & MAG_POINTS != 0 {
        crate::magic::mag_points(g, level, caster, cvict, spellnum, savetype);
    }
    if info.routines & MAG_ALTER_OBJS != 0 {
        crate::magic::mag_alter_objs(g, level, caster, ovict, spellnum, savetype);
    }
    if info.routines & MAG_GROUPS != 0 {
        crate::magic::mag_groups(g, level, caster, spellnum, savetype);
    }
    if info.routines & MAG_MASSES != 0 {
        crate::magic::mag_masses(g, level, caster, spellnum, savetype);
    }
    if info.routines & MAG_AREAS != 0 {
        crate::magic::mag_areas(g, level, caster, spellnum, savetype);
    }
    if info.routines & MAG_SUMMONS != 0 {
        crate::magic::mag_summons(g, level, caster, ovict, spellnum, savetype);
    }
    if info.routines & MAG_CREATIONS != 0 {
        crate::magic::mag_creations(g, level, caster, spellnum);
    }
    if info.routines & MAG_ROOMS != 0 {
        crate::magic::mag_rooms(g, level, caster, spellnum);
    }
    if info.routines & MAG_MANUAL != 0 {
        match spellnum {
            spells::SPELL_CHARM => crate::spells::spell_charm(g, level, caster, cvict, ovict),
            spells::SPELL_CREATE_WATER => crate::spells::spell_create_water(g, level, caster, cvict, ovict),
            spells::SPELL_DETECT_POISON => crate::spells::spell_detect_poison(g, level, caster, cvict, ovict),
            spells::SPELL_ENCHANT_WEAPON => crate::spells::spell_enchant_weapon(g, level, caster, cvict, ovict),
            spells::SPELL_IDENTIFY => crate::spells::spell_identify(g, level, caster, cvict, ovict),
            spells::SPELL_LOCATE_OBJECT => crate::spells::spell_locate_object(g, level, caster, cvict, ovict),
            spells::SPELL_SUMMON => crate::spells::spell_summon(g, level, caster, cvict, ovict),
            spells::SPELL_WORD_OF_RECALL => crate::spells::spell_recall(g, level, caster, cvict, ovict),
            spells::SPELL_TELEPORT => crate::spells::spell_teleport(g, level, caster, cvict, ovict),
            _ => {}
        }
    }

    1
}

/// mag_objectmagic: staves, wands, scrolls, potions.
pub fn mag_objectmagic(g: &mut Game, chid: CharId, oid: ObjId, argument: &[u8]) {
    let (arg, _) = one_argument(argument);

    let (k, mut tch, tobj) = generic_find(
        g,
        chid,
        &arg,
        FIND_CHAR_ROOM | FIND_OBJ_INV | FIND_OBJ_ROOM | FIND_OBJ_EQUIP,
    );

    let otype = g.obj(oid).type_flag;
    match otype {
        flags::ITEM_STAFF => {
            act(g, b"You tap $p three times on the ground.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
            let action_desc = g.obj(oid).action_description.clone();
            if let Some(ad) = action_desc.filter(|d| !d.is_empty()) {
                act(g, &ad, false, Some(chid), Some(oid), None, comm::TO_ROOM);
            } else {
                act(g, b"$n taps $p three times on the ground.", false, Some(chid), Some(oid), None, comm::TO_ROOM);
            }

            if g.obj(oid).values[2] <= 0 {
                send_to_char(g, chid, b"It seems powerless.\r\n");
                act(g, b"Nothing seems to happen.", false, Some(chid), Some(oid), None, comm::TO_ROOM);
            } else {
                g.obj_mut(oid).values[2] -= 1;
                g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
                let k_level = if g.obj(oid).values[0] != 0 { g.obj(oid).values[0] } else { DEFAULT_STAFF_LVL };
                let spellnum = g.obj(oid).values[3];
                let routines = spell_info(spellnum).routines;
                let room = g.ch(chid).in_room;
                if routines & (MAG_MASSES | MAG_AREAS) != 0 {
                    let mut i = g.rooms[room as usize].people.len() as i32;
                    while i > 0 {
                        i -= 1;
                        call_magic(g, chid, None, None, spellnum, k_level, CAST_STAFF);
                    }
                } else {
                    let people = g.rooms[room as usize].people.clone();
                    for t in people {
                        if t != chid && g.try_ch(t).is_some() {
                            call_magic(g, chid, Some(t), None, spellnum, k_level, CAST_STAFF);
                        }
                    }
                }
            }
        }
        flags::ITEM_WAND => {
            if k == FIND_CHAR_ROOM {
                if tch == Some(chid) {
                    act(g, b"You point $p at yourself.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
                    act(g, b"$n points $p at $mself.", false, Some(chid), Some(oid), None, comm::TO_ROOM);
                } else {
                    act(g, b"You point $p at $N.", false, Some(chid), Some(oid), tch, comm::TO_CHAR);
                    let action_desc = g.obj(oid).action_description.clone();
                    if let Some(ad) = action_desc.filter(|d| !d.is_empty()) {
                        act(g, &ad, false, Some(chid), Some(oid), tch, comm::TO_ROOM);
                    } else {
                        act(g, b"$n points $p at $N.", true, Some(chid), Some(oid), tch, comm::TO_ROOM);
                    }
                }
            } else if let Some(to) = tobj {
                act_obj2(g, b"You point $p at $P.", false, chid, oid, to, comm::TO_CHAR);
                let action_desc = g.obj(oid).action_description.clone();
                if let Some(ad) = action_desc.filter(|d| !d.is_empty()) {
                    act_obj2(g, &ad, false, chid, oid, to, comm::TO_ROOM);
                } else {
                    act_obj2(g, b"$n points $p at $P.", true, chid, oid, to, comm::TO_ROOM);
                }
            } else if spell_info(g.obj(oid).values[3]).routines & (MAG_AREAS | MAG_MASSES) != 0 {
                // Wands with area spells don't need to be pointed.
                act(g, b"You point $p outward.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
                act(g, b"$n points $p outward.", true, Some(chid), Some(oid), None, comm::TO_ROOM);
            } else {
                act(g, b"At what should $p be pointed?", false, Some(chid), Some(oid), None, comm::TO_CHAR);
                return;
            }

            if g.obj(oid).values[2] <= 0 {
                send_to_char(g, chid, b"It seems powerless.\r\n");
                act(g, b"Nothing seems to happen.", false, Some(chid), Some(oid), None, comm::TO_ROOM);
                return;
            }
            g.obj_mut(oid).values[2] -= 1;
            g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
            let spellnum = g.obj(oid).values[3];
            let lvl = if g.obj(oid).values[0] != 0 { g.obj(oid).values[0] } else { DEFAULT_WAND_LVL };
            call_magic(g, chid, tch, tobj, spellnum, lvl, CAST_WAND);
        }
        flags::ITEM_SCROLL => {
            if !arg.is_empty() {
                if k == 0 {
                    act(
                        g,
                        b"There is nothing to here to affect with $p.",
                        false,
                        Some(chid),
                        Some(oid),
                        None,
                        comm::TO_CHAR,
                    );
                    return;
                }
            } else {
                tch = Some(chid);
            }

            act(g, b"You recite $p which dissolves.", true, Some(chid), Some(oid), None, comm::TO_CHAR);
            let action_desc = g.obj(oid).action_description.clone();
            if let Some(ad) = action_desc.filter(|d| !d.is_empty()) {
                act(g, &ad, false, Some(chid), Some(oid), tch, comm::TO_ROOM);
            } else {
                act(g, b"$n recites $p.", false, Some(chid), Some(oid), None, comm::TO_ROOM);
            }

            g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
            let (v0, v1, v2, v3) = {
                let o = g.obj(oid);
                (o.values[0], o.values[1], o.values[2], o.values[3])
            };
            for spell in [v1, v2, v3] {
                if call_magic(g, chid, tch, tobj, spell, v0, CAST_SCROLL) <= 0 {
                    break;
                }
            }
            if g.try_obj(oid).is_some() {
                crate::handler::extract_obj(g, oid);
            }
        }
        flags::ITEM_POTION => {
            // tch is the quaffer, passed directly to the calls below.
            if crate::dg::triggers::consume_otrigger(g, oid, chid, crate::dg::OCMD_QUAFF) == 0 {
                return;
            }
            if g.try_obj(oid).is_none() {
                return;
            }

            act(g, b"You quaff $p.", false, Some(chid), Some(oid), None, comm::TO_CHAR);
            let action_desc = g.obj(oid).action_description.clone();
            if let Some(ad) = action_desc.filter(|d| !d.is_empty()) {
                act(g, &ad, false, Some(chid), Some(oid), None, comm::TO_ROOM);
            } else {
                act(g, b"$n quaffs $p.", true, Some(chid), Some(oid), None, comm::TO_ROOM);
            }

            g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
            let (v0, v1, v2, v3) = {
                let o = g.obj(oid);
                (o.values[0], o.values[1], o.values[2], o.values[3])
            };
            for spell in [v1, v2, v3] {
                if call_magic(g, chid, Some(chid), None, spell, v0, CAST_POTION) <= 0 {
                    break;
                }
            }
            if g.try_obj(oid).is_some() {
                crate::handler::extract_obj(g, oid);
            }
        }
        _ => {
            g.log(format!("SYSERR: Unknown object_type {} in mag_objectmagic.", otype));
        }
    }
}

/// act with obj + second obj ($P) — comm::act_full with ActArg::Obj.
fn act_obj2(g: &mut Game, msg: &[u8], hide: bool, chid: CharId, obj: ObjId, obj2: ObjId, to: i32) {
    comm::act_full(g, msg, hide, Some(chid), Some(obj), comm::ActArg::Obj(obj2), to);
}

/// cast_spell: position/self gates, "Okay.",
/// say_spell, then call_magic at the caster's level.
pub fn cast_spell(g: &mut Game, chid: CharId, tch: Option<CharId>, tobj: Option<ObjId>, spellnum: i32) -> i32 {
    if !(0..=TOP_SPELL_DEFINE).contains(&spellnum) {
        g.log(format!("SYSERR: cast_spell trying to call spellnum {}/{}.", spellnum, TOP_SPELL_DEFINE));
        return 0;
    }
    let info = *spell_info(spellnum);

    if (g.ch(chid).position as i32) < info.min_position as i32 {
        match g.ch(chid).position {
            POS_SLEEPING => send_to_char(g, chid, b"You dream about great magical powers.\r\n"),
            POS_RESTING => send_to_char(g, chid, b"You cannot concentrate while resting.\r\n"),
            POS_SITTING => send_to_char(g, chid, b"You can't do this sitting!\r\n"),
            POS_FIGHTING => send_to_char(g, chid, b"Impossible!  You can't concentrate enough!\r\n"),
            _ => send_to_char(g, chid, b"You can't do much of anything like this!\r\n"),
        }
        return 0;
    }
    // The comparison is by identity: a masterless charmed caster with no
    // target
    // (NULL == NULL) also refuses — Option::eq mirrors that exactly.
    if g.ch(chid).aff(flags::AFF_CHARM) && g.ch(chid).master == tch {
        send_to_char(g, chid, b"You are afraid you might hurt your master!\r\n");
        return 0;
    }
    if tch != Some(chid) && info.targets & TAR_SELF_ONLY != 0 {
        send_to_char(g, chid, b"You can only cast this spell upon yourself!\r\n");
        return 0;
    }
    if tch == Some(chid) && info.targets & TAR_NOT_SELF != 0 {
        send_to_char(g, chid, b"You cannot cast this spell upon yourself!\r\n");
        return 0;
    }
    if info.routines & MAG_GROUPS != 0 && g.ch(chid).group.is_none() {
        send_to_char(g, chid, b"You can't cast this spell if you're not in a group!\r\n");
        return 0;
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
    say_spell(g, chid, spellnum, tch, tobj);

    let level = g.ch(chid).level as i32;
    call_magic(g, chid, tch, tobj, spellnum, level, CAST_SPELL)
}

/// Split on single quotes, three times, as do_cast does: (before-first-quote,
/// quoted-name, after-closing-quote). Each Option mirrors a NULL return.
fn cast_tokens(argument: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>) {
    let mut i0 = 0usize;
    while i0 < argument.len() && argument[i0] == b'\'' {
        i0 += 1;
    }
    if i0 >= argument.len() {
        return (None, None, None);
    }
    let q1 = argument[i0..].iter().position(|&c| c == b'\'').map(|p| i0 + p).unwrap_or(argument.len());
    let tok1 = argument[i0..q1].to_vec();
    // The next split resumes after the quote that ended tok1, then
    // skips any leading delimiters.
    let mut j = if q1 < argument.len() { q1 + 1 } else { q1 };
    while j < argument.len() && argument[j] == b'\'' {
        j += 1;
    }
    if j >= argument.len() {
        return (Some(tok1), None, None);
    }
    let q2 = argument[j..].iter().position(|&c| c == b'\'').map(|p| j + p).unwrap_or(argument.len());
    let tok2 = argument[j..q2].to_vec();
    if q2 >= argument.len() {
        return (Some(tok1), Some(tok2), None);
    }
    let rest = &argument[q2 + 1..];
    if rest.is_empty() {
        (Some(tok1), Some(tok2), None)
    } else {
        (Some(tok1), Some(tok2), Some(rest.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obfuscation_matches_the_syllable_table() {
        // First-match table-order scan, computed by
        // hand against the syls[] table.
        assert_eq!(obfuscate_core(b"armor").0, b"abrazak".to_vec());
        assert_eq!(obfuscate_core(b"magic missile").0, b"kariq wugguro".to_vec());
        assert_eq!(obfuscate_core(b"cure light").0, b"judicandus dies".to_vec());
        assert_eq!(obfuscate_core(b"word of recall").0, b"inset candusqirr".to_vec());
        assert!(obfuscate_core(b"armor").1.is_empty());
    }

    #[test]
    fn cast_token_split_mirrors_strtok() {
        // cast 'armor' → argument " 'armor'"
        assert_eq!(
            cast_tokens(b" 'armor'"),
            (Some(b" ".to_vec()), Some(b"armor".to_vec()), None)
        );
        // cast 'cure light' me
        assert_eq!(
            cast_tokens(b" 'cure light' me"),
            (Some(b" ".to_vec()), Some(b"cure light".to_vec()), Some(b" me".to_vec()))
        );
        // No quotes at all → tok2 NULL ("holy magic symbols").
        assert_eq!(cast_tokens(b" armor"), (Some(b" armor".to_vec()), None, None));
        // Empty → tok1 NULL ("Cast what where?").
        assert_eq!(cast_tokens(b""), (None, None, None));
        // Unterminated quote still yields the name.
        assert_eq!(cast_tokens(b" 'armor"), (Some(b" ".to_vec()), Some(b"armor".to_vec()), None));
        // Doubled opening quote is skipped as a leading delimiter.
        assert_eq!(
            cast_tokens(b" ''fireball'"),
            (Some(b" ".to_vec()), Some(b"fireball".to_vec()), None)
        );
        // All-quotes input: every delimiter is skipped and nothing remains.
        assert_eq!(cast_tokens(b"''"), (None, None, None));
    }
}

pub fn do_cast(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }

    let (s0, s1, t_raw) = cast_tokens(argument);
    if s0.is_none() {
        send_to_char(g, chid, b"Cast what where?\r\n");
        return;
    }
    let Some(mut s) = s1 else {
        send_to_char(g, chid, b"Spell names must be enclosed in the Holy Magic Symbols: '\r\n");
        return;
    };
    s = skip_spaces(&s).to_vec();

    let spellnum = crate::spec::find_skill_num(&s).unwrap_or(-1);
    if spellnum < 1 || spellnum > MAX_SPELLS || s.is_empty() {
        send_to_char(g, chid, b"Cast what?!?\r\n");
        return;
    }
    let info = *spell_info(spellnum);
    let class = g.ch(chid).class.clamp(0, 3) as usize;
    if (g.ch(chid).level as i32) < info.min_level[class] {
        send_to_char(g, chid, b"You do not know that spell!\r\n");
        return;
    }
    if g.ch(chid).get_skill(spellnum) == 0 {
        send_to_char(g, chid, b"You are unfamiliar with that spell.\r\n");
        return;
    }

    // Find the target.
    let mut tch: Option<CharId> = None;
    let mut tobj: Option<ObjId> = None;
    let mut target = false;

    let mut t: Vec<u8> = Vec::new();
    if let Some(raw) = t_raw {
        // strlcpy(arg, t); one_argument(arg, t); skip_spaces(&t) —
        // the target token is the (lowercased) first word.
        let (word, _) = one_argument(&raw);
        t = word;
        g.cast_arg2 = t.clone();
    }

    if info.targets & TAR_IGNORE != 0 {
        target = true;
    } else if !t.is_empty() {
        let (mut number, stripped) = get_number(&t);
        t = stripped;
        if !target && info.targets & TAR_CHAR_ROOM != 0 {
            if let Some(v) = get_char_room_vis_counted(g, chid, &t, &mut number) {
                tch = Some(v);
                target = true;
            }
        }
        if !target && info.targets & TAR_CHAR_WORLD != 0 {
            if let Some(v) = get_char_world_vis(g, chid, &t, Some(number)) {
                tch = Some(v);
                target = true;
            }
        }
        if !target && info.targets & TAR_OBJ_INV != 0 {
            let carrying = g.ch(chid).carrying.clone();
            if let Some(o) = get_obj_in_list_vis_counted(g, chid, &t, &mut number, &carrying) {
                tobj = Some(o);
                target = true;
            }
        }
        if !target && info.targets & TAR_OBJ_EQUIP != 0 {
            // Equipment is scanned with plain isname, no countdown.
            for i in 0..NUM_WEARS {
                if target {
                    break;
                }
                if let Some(o) = g.ch(chid).equipment[i] {
                    if isname(&t, obj_name(g, o)) {
                        tobj = Some(o);
                        target = true;
                    }
                }
            }
        }
        if !target && info.targets & TAR_OBJ_ROOM != 0 {
            let room = g.ch(chid).in_room;
            let contents = g.rooms[room as usize].contents.clone();
            if let Some(o) = get_obj_in_list_vis_counted(g, chid, &t, &mut number, &contents) {
                tobj = Some(o);
                target = true;
            }
        }
        if !target && info.targets & TAR_OBJ_WORLD != 0 {
            if let Some(o) = get_obj_vis_counted(g, chid, &t, &mut number) {
                tobj = Some(o);
                target = true;
            }
        }
    } else {
        // Empty target string.
        if !target && info.targets & TAR_FIGHT_SELF != 0 && g.ch(chid).fighting.is_some() {
            tch = Some(chid);
            target = true;
        }
        if !target && info.targets & TAR_FIGHT_VICT != 0 {
            if let Some(f) = g.ch(chid).fighting {
                tch = Some(f);
                target = true;
            }
        }
        if !target && info.targets & TAR_CHAR_ROOM != 0 && !info.violent {
            tch = Some(chid);
            target = true;
        }
        if !target {
            let what = if info.targets & (TAR_OBJ_ROOM | TAR_OBJ_INV | TAR_OBJ_WORLD | TAR_OBJ_EQUIP) != 0 {
                "what"
            } else {
                "who"
            };
            let msg = format!("Upon {} should the spell be cast?\r\n", what);
            send_to_char(g, chid, msg.as_bytes());
            return;
        }
    }

    if target && tch == Some(chid) && info.violent {
        send_to_char(
            g,
            chid,
            b"You shouldn't cast that on yourself -- could be bad for your health!\r\n",
        );
        return;
    }
    if !target {
        send_to_char(g, chid, b"Cannot find the target of your spell!\r\n");
        return;
    }

    let mana = mag_manacost(g, chid, spellnum);
    if mana > 0 && g.ch(chid).points.mana < mana && (g.ch(chid).level as i32) < LVL_IMMORT as i32 {
        send_to_char(g, chid, b"You haven't the energy to cast that spell!\r\n");
        return;
    }

    // You throws the dice and you takes your chances.. 101% is total failure.
    if g.rng.rand_number(0, 101) > g.ch(chid).get_skill(spellnum) {
        g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
        let msged = tch.is_some_and(|t| crate::fight::skill_message(g, 0, chid, t, spellnum));
        if !msged {
            send_to_char(g, chid, b"You lost your concentration!\r\n");
        }
        if mana > 0 {
            let ch = g.ch_mut(chid);
            ch.points.mana = (ch.points.mana - mana / 2).clamp(0, ch.points.max_mana.max(0));
        }
        if info.violent && tch.is_some_and(|t| g.try_ch(t).is_some_and(|c| c.is_npc())) {
            crate::fight::hit(g, tch.unwrap(), chid, spells::TYPE_UNDEFINED);
        }
    } else if cast_spell(g, chid, tch, tobj, spellnum) != 0 {
        g.ch_mut(chid).wait = PULSE_VIOLENCE as i32;
        if mana > 0 {
            let ch = g.ch_mut(chid);
            ch.points.mana = (ch.points.mana - mana).clamp(0, ch.points.max_mana.max(0));
        }
    }
}
