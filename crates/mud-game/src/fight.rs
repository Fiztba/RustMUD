//! The combat engine: fight-state list, perform_violence, hit,
//! damage (the exact 20-step flow), damage messages (lib/misc/messages),
//! death (die → raw_kill → make_corpse), and kill experience.
//!
//! DG trigger hooks (fight/hitprcnt/damage/death mtriggers) are stage-6
//! stubs with their default outcomes; groups activate at stage 5.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::spells::*;
use mud_data::types::*;

use crate::comm::{act, act_full, cc, send_to_char, send_to_room, ActArg, C_CMP, C_SPR, KNRM, KRED, KYEL, TO_CHAR, TO_NOTVICT, TO_ROOM, TO_SLEEP, TO_VICT};
use crate::game::{Game, MudlogKind};
use crate::handler::{affect_from_char, affect_remove_all, char_from_room, char_to_room, extract_char, obj_to_obj, obj_to_room, unequip_char};

pub const ATTACK_HIT_TEXT: [(&[u8], &[u8]); 15] = [
    (b"hit", b"hits"),
    (b"sting", b"stings"),
    (b"whip", b"whips"),
    (b"slash", b"slashes"),
    (b"bite", b"bites"),
    (b"bludgeon", b"bludgeons"),
    (b"crush", b"crushes"),
    (b"pound", b"pounds"),
    (b"claw", b"claws"),
    (b"maul", b"mauls"),
    (b"thrash", b"thrashes"),
    (b"pierce", b"pierces"),
    (b"blast", b"blasts"),
    (b"punch", b"punches"),
    (b"stab", b"stabs"),
];

pub const MAX_MESSAGES: usize = 60;

#[derive(Debug, Clone, Default)]
pub struct MsgTriple {
    pub attacker: Option<Vec<u8>>,
    pub victim: Option<Vec<u8>>,
    pub room: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct MessageType {
    pub die: MsgTriple,
    pub miss: MsgTriple,
    pub hit: MsgTriple,
    pub god: MsgTriple,
}

/// One message slot: every M record for an attack type. The loader
/// prepends, so msg[0] is the last block in file order.
#[derive(Debug, Clone, Default)]
pub struct FightMessageList {
    pub a_type: i32,
    pub number_of_attacks: i32,
    pub msg: Vec<MessageType>,
}

/// load_messages over lib/misc/messages. Slots fill in
/// first-free/matching order; records prepend within a slot.
pub fn load_messages(lib: &std::path::Path) -> Result<Vec<FightMessageList>, String> {
    let path = lib.join("misc").join("messages");
    let data = std::fs::read(&path)
        .map_err(|e| format!("SYSERR: Error reading combat message file {}: {}", path.display(), e))?;
    let mut lines = data.split(|&c| c == b'\n');

    let mut table: Vec<FightMessageList> = vec![FightMessageList::default(); MAX_MESSAGES];

    // fread_action: '#' first char → NULL; truncate at \r/\n; parse_at.
    fn fread_action(line: Option<&[u8]>) -> Result<Option<Vec<u8>>, String> {
        let line = line.ok_or("SYSERR: fread_action: unexpected EOF")?;
        if line.first() == Some(&b'#') {
            return Ok(None);
        }
        let mut buf: Vec<u8> = line.split(|&c| c == b'\r').next().unwrap_or(b"").to_vec();
        crate::text::parse_at(&mut buf);
        Ok(Some(buf))
    }

    while let Some(raw) = lines.next() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.first() != Some(&b'M') {
            continue;
        }
        let type_line = lines.next().ok_or("SYSERR: messages file ends after M")?;
        let type_: i32 = String::from_utf8_lossy(type_line)
            .trim()
            .parse()
            .unwrap_or(0);
        let slot = table
            .iter()
            .position(|e| e.a_type == type_ || e.a_type == 0)
            .ok_or("SYSERR: Too many combat messages.  Increase MAX_MESSAGES and recompile.")?;
        table[slot].a_type = type_;
        table[slot].number_of_attacks += 1;
        let mut m = MessageType::default();
        m.die.attacker = fread_action(lines.next())?;
        m.die.victim = fread_action(lines.next())?;
        m.die.room = fread_action(lines.next())?;
        m.miss.attacker = fread_action(lines.next())?;
        m.miss.victim = fread_action(lines.next())?;
        m.miss.room = fread_action(lines.next())?;
        m.hit.attacker = fread_action(lines.next())?;
        m.hit.victim = fread_action(lines.next())?;
        m.hit.room = fread_action(lines.next())?;
        m.god.attacker = fread_action(lines.next())?;
        m.god.victim = fread_action(lines.next())?;
        m.god.room = fread_action(lines.next())?;
        table[slot].msg.insert(0, m);
    }
    Ok(table)
}

#[inline]
pub fn is_weapon_type(t: i32) -> bool {
    (TYPE_HIT..TYPE_SUFFERING).contains(&t)
}

/// STRENGTH_APPLY_INDEX over an id (handler has the &Char form).
pub fn strength_apply_index(g: &Game, chid: CharId) -> usize {
    crate::handler::strength_apply_index(g.ch(chid))
}

pub fn update_pos(g: &mut Game, chid: CharId) {
    let (hp, pos) = {
        let ch = g.ch(chid);
        (ch.points.hit, ch.position)
    };
    let new_pos = if hp > 0 && pos > POS_STUNNED {
        return;
    } else if hp > 0 {
        POS_STANDING
    } else if hp <= -11 {
        POS_DEAD
    } else if hp <= -6 {
        POS_MORTALLYW
    } else if hp <= -3 {
        POS_INCAP
    } else {
        POS_STUNNED
    };
    g.ch_mut(chid).position = new_pos;
}

pub fn check_killer(g: &mut Game, chid: CharId, vict: CharId) {
    {
        let v = g.ch(vict);
        if v.plr(flags::PLR_KILLER) || v.plr(flags::PLR_THIEF) {
            return;
        }
        let c = g.ch(chid);
        if c.plr(flags::PLR_KILLER) || c.is_npc() || v.is_npc() || chid == vict {
            return;
        }
    }
    g.ch_mut(chid).act.set(flags::PLR_KILLER);
    send_to_char(g, chid, b"If you want to be a PLAYER KILLER, so be it...\r\n");
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vname = String::from_utf8_lossy(g.ch(vict).get_name()).into_owned();
    let room = g.ch(vict).in_room;
    let rname = String::from_utf8_lossy(
        g.world.rooms[room as usize].name.as_deref().unwrap_or(b""),
    )
    .into_owned();
    let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()).max(g.ch(vict).invis_lev()) as u8;
    g.mudlog(
        MudlogKind::Brf,
        lvl,
        true,
        &format!("PC Killer bit set on {} for initiating attack on {} at {}.", name, vname, rname),
    );
}

/// pk_allowed. true = the fight may proceed.
pub fn pk_allowed(g: &mut Game, chid: CharId, vict: CharId) -> bool {
    if g.ch(chid).is_npc() || g.ch(vict).is_npc() {
        return true;
    }
    if g.config.pk_setting == 0 {
        return false;
    }
    if g.config.pk_setting == 1 {
        check_killer(g, chid, vict);
    }
    true
}

/// set_fighting. Prepends to combat_list.
pub fn set_fighting(g: &mut Game, chid: CharId, vict: CharId) {
    if chid == vict {
        return;
    }
    if g.ch(chid).fighting.is_some() {
        g.log("SYSERR: set_fighting: already fighting (core_dump point)".to_string());
        return;
    }
    if !pk_allowed(g, chid, vict) {
        send_to_char(g, chid, b"Player killing is not permitted.\r\n");
        return;
    }
    g.combat_list.insert(0, chid);
    if g.ch(chid).aff(flags::AFF_SLEEP) {
        affect_from_char(g, chid, SPELL_SLEEP as i16);
    }
    let ch = g.ch_mut(chid);
    ch.fighting = Some(vict);
    ch.position = POS_FIGHTING;
}

/// stop_fighting. Patches the perform_violence cursor.
pub fn stop_fighting(g: &mut Game, chid: CharId) {
    if g.next_combat == Some(chid) {
        let pos = g.combat_list.iter().position(|&c| c == chid);
        g.next_combat = pos.and_then(|p| g.combat_list.get(p + 1)).copied();
    }
    g.combat_list.retain(|&c| c != chid);
    let ch = g.ch_mut(chid);
    ch.fighting = None;
    ch.position = POS_STANDING;
    update_pos(g, chid);
}

/// make_corpse. The corpse presets body-only weight; its contents account
/// honestly through the corpse carve-out in obj_to_obj/obj_from_obj.
fn make_corpse(g: &mut Game, chid: CharId) {
    let name = g.ch(chid).get_name().to_vec();
    let mut obj = crate::obj::create_obj();
    obj.name = Some(b"corpse".to_vec());
    let mut d = b"The corpse of ".to_vec();
    d.extend_from_slice(&name);
    d.extend_from_slice(b" is lying here.");
    obj.description = Some(d);
    let mut s = b"the corpse of ".to_vec();
    s.extend_from_slice(&name);
    obj.short_description = Some(s);
    obj.type_flag = flags::ITEM_CONTAINER;
    obj.wear_flags.set(flags::ITEM_WEAR_TAKE);
    obj.extra_flags.set(flags::ITEM_NODONATE);
    obj.values[0] = 0; // You can't store stuff in a corpse
    obj.values[3] = 1; // corpse identifier
    obj.cost_per_day = 100000;
    // Body weight alone; contents account honestly below.
    obj.weight = g.ch(chid).weight as i32;
    obj.timer = if g.ch(chid).is_npc() {
        g.config.max_npc_corpse_time
    } else {
        g.config.max_pc_corpse_time
    };
    let oid = g.objs.insert(obj);
    g.object_list.push_front(oid);

    // Transfer inventory wholesale (corpse->contains = ch->carrying;
    // object_list_new_owner clears carried_by).
    let carried: Vec<ObjId> = std::mem::take(&mut g.ch_mut(chid).carrying);
    for &c in &carried {
        g.obj_mut(c).carried_by = None;
        g.obj_mut(c).in_obj = Some(oid);
        // Honest accounting: the wholesale move bypasses obj_to_obj.
        let w = crate::handler::obj_weight(g, c);
        g.obj_mut(oid).weight += w;
    }
    g.obj_mut(oid).contains = carried;

    // Equipment: remove_otrigger fires per slot, result ignored
    for i in 0..NUM_WEARS {
        if let Some(eq0) = g.ch(chid).equipment[i] {
            crate::dg::triggers::remove_otrigger(g, eq0, chid);
            if g.ch(chid).equipment[i].is_none() || g.try_obj(eq0).is_none() {
                continue;
            }
            if let Some(eq) = unequip_char(g, chid, i) {
                obj_to_obj(g, eq, oid);
            }
        }
    }

    // Gold → money object, except link-dead PCs (anti-dupe: gold evaporates).
    let gold = g.ch(chid).points.gold;
    if gold > 0 {
        if g.ch(chid).is_npc() || g.ch(chid).desc.is_some() {
            if let Some(money) = crate::handler::create_money(g, gold) {
                obj_to_obj(g, money, oid);
            }
        }
        g.ch_mut(chid).points.gold = 0;
    }
    {
        let ch = g.ch_mut(chid);
        ch.carry_items = 0;
        ch.carry_weight = 0;
    }
    let room = g.ch(chid).in_room;
    obj_to_room(g, oid, room);
}

fn change_alignment(g: &mut Game, chid: CharId, victim: CharId) {
    let va = g.ch(victim).alignment;
    let ca = g.ch(chid).alignment;
    g.ch_mut(chid).alignment += (-va - ca) / 16;
}

pub fn death_cry(g: &mut Game, chid: CharId) {
    act(g, b"Your blood freezes as you hear $n's death cry.", false, Some(chid), None, None, TO_ROOM);
    let room = g.ch(chid).in_room;
    for door in 0..dir_count(g) {
        if let Some(to) = can_go(g, room, door) {
            send_to_room(g, to, b"Your blood freezes as you hear someone's death cry.\r\n");
        }
    }
}

pub fn dir_count(g: &Game) -> usize {
    if g.config.diagonal_dirs { 10 } else { 6 }
}

/// CAN_GO from a room: exit exists, leads somewhere, door not closed.
pub fn can_go(g: &Game, room: RoomRnum, door: usize) -> Option<RoomRnum> {
    if room == NOWHERE {
        return None;
    }
    let e = g.world.rooms[room as usize].dir_option[door].as_deref()?;
    if e.to_room == NOWHERE || e.exit_info & flags::EX_CLOSED != 0 {
        return None;
    }
    Some(e.to_room)
}

pub fn raw_kill(g: &mut Game, chid: CharId, killer: Option<CharId>) {
    if g.ch(chid).fighting.is_some() {
        stop_fighting(g, chid);
    }
    affect_remove_all(g, chid);
    // To make ordinary commands work in scripts. welcor
    g.ch_mut(chid).position = POS_STANDING;

    if let Some(k) = killer {
        if death_mtrigger(g, chid, k) {
            death_cry(g, chid);
        }
    } else {
        death_cry(g, chid);
    }

    // autoquest_trigger_check AQ_MOB_KILL: every group
    // member in the victim's room OR zone gets the credit; a soloist just
    // gets it directly.
    if let Some(k) = killer {
        let gid = g.try_ch(k).and_then(|c| c.group);
        match gid {
            Some(gid) => {
                let members =
                    g.groups.iter().find(|gr| gr.id == gid).map(|gr| gr.members.clone());
                let victim_room = g.ch(chid).in_room;
                let victim_zone = if victim_room == NOWHERE {
                    NOWHERE
                } else {
                    g.world.rooms[victim_room as usize].zone
                };
                for i in members.unwrap_or_default() {
                    let Some(ic) = g.try_ch(i) else { continue };
                    let iroom = ic.in_room;
                    let same = iroom == victim_room
                        || (iroom != NOWHERE
                            && victim_room != NOWHERE
                            && g.world.rooms[iroom as usize].zone == victim_zone);
                    if same {
                        crate::quest::autoquest_trigger_check(
                            g,
                            i,
                            Some(chid),
                            None,
                            crate::quest::AQ_MOB_KILL,
                        );
                    }
                }
            }
            None => crate::quest::autoquest_trigger_check(
                g,
                k,
                Some(chid),
                None,
                crate::quest::AQ_MOB_KILL,
            ),
        }
    }


    // Alert Group if Applicable. The dying member is the
    // excluded sender.
    if let Some(gid) = g.ch(chid).group {
        let mut body = g.ch(chid).get_name().to_vec();
        body.extend_from_slice(b" has died.\r\n");
        crate::comm::send_to_group(g, Some(chid), gid, &body);
    }

    update_pos(g, chid);
    make_corpse(g, chid);
    extract_char(g, chid);

    if let Some(k) = killer {
        if g.try_ch(k).is_some() {
            crate::quest::autoquest_trigger_check(g, k, None, None, crate::quest::AQ_MOB_SAVE);
            crate::quest::autoquest_trigger_check(g, k, None, None, crate::quest::AQ_ROOM_CLEAR);
        }
    }
}

pub fn die(g: &mut Game, chid: CharId, killer: Option<CharId>) {
    let exp = g.ch(chid).points.exp;
    crate::limits::gain_exp(g, chid, -(exp / 2));
    if !g.ch(chid).is_npc() {
        let ch = g.ch_mut(chid);
        ch.act.remove(flags::PLR_KILLER);
        ch.act.remove(flags::PLR_THIEF);
    }
    raw_kill(g, chid, killer);
}

fn perform_group_gain(g: &mut Game, chid: CharId, base: i32, victim: CharId) {
    let mut share = g.config.max_exp_gain.min(base.max(1));
    if crate::act::other::is_happyhour(g) && g.happy.exp_rate > 0 {
        // "This only reports the correct amount - the calc is done in
        // gain_exp".
        let hap = share + (share as f32 * (g.happy.exp_rate as f32 / 100.0)) as i32;
        share = g.config.max_exp_gain.min(hap.max(1));
    }
    if share > 1 {
        send_to_char(
            g,
            chid,
            format!("You receive your share of experience -- {} points.\r\n", share).as_bytes(),
        );
    } else {
        send_to_char(g, chid, b"You receive your share of experience -- one measly little point!\r\n");
    }
    crate::limits::gain_exp(g, chid, share);
    change_alignment(g, chid, victim);
}

/// group_gain: members in the killer's room split
/// round-up thirds; no level-difference bonus for grouped kills.
fn group_gain(g: &mut Game, chid: CharId, victim: CharId) {
    let Some(gr) = g.group_of(chid) else { return };
    let members = gr.members.clone();
    let room = g.ch(chid).in_room;
    let tot_members = members
        .iter()
        .filter(|&&k| g.try_ch(k).is_some_and(|c| c.in_room == room))
        .count() as i32;

    // Round up to the nearest tot_members.
    let mut tot_gain = (g.ch(victim).points.exp / 3) + tot_members - 1;
    if !g.ch(victim).is_npc() {
        tot_gain = tot_gain.min(g.config.max_exp_loss * 2 / 3);
    }
    let base = if tot_members >= 1 { (tot_gain / tot_members).max(1) } else { 0 };

    for k in members {
        if g.try_ch(k).is_some_and(|c| c.in_room == room) {
            perform_group_gain(g, k, base, victim);
        }
    }
}

fn solo_gain(g: &mut Game, chid: CharId, victim: CharId) {
    let mut exp = g.config.max_exp_gain.min(g.ch(victim).points.exp / 3);

    let level_diff = g.ch(victim).level as i32 - g.ch(chid).level as i32;
    if g.ch(chid).is_npc() {
        exp += 0.max((exp * 4.min(level_diff)) / 8);
    } else {
        exp += 0.max((exp * 8.min(level_diff)) / 8);
    }
    exp = exp.max(1);

    if crate::act::other::is_happyhour(g) && g.happy.exp_rate > 0 {
        // Reporting only — gain_exp applies the bonus to the credit
        let happy_exp = exp + (exp as f32 * (g.happy.exp_rate as f32 / 100.0)) as i32;
        exp = happy_exp.max(1);
    }

    if exp > 1 {
        send_to_char(g, chid, format!("You receive {} experience points.\r\n", exp).as_bytes());
    } else {
        send_to_char(g, chid, b"You receive one lousy experience point.\r\n");
    }
    crate::limits::gain_exp(g, chid, exp);
    change_alignment(g, chid, victim);
}

/// replace_string: #w singular / #W plural.
fn replace_string(s: &[u8], singular: &[u8], plural: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' && i + 1 < s.len() {
            match s[i + 1] {
                b'W' => out.extend_from_slice(plural),
                b'w' => out.extend_from_slice(singular),
                _ => out.push(b'#'),
            }
            i += 2;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

fn dam_message(g: &mut Game, dam: i32, chid: CharId, victim: CharId, w_type: i32) {
    struct DamWeapon {
        to_room: &'static [u8],
        to_char: &'static [u8],
        to_victim: &'static [u8],
    }
    const DAM_WEAPONS: [DamWeapon; 9] = [
        DamWeapon {
            to_room: b"$n tries to #w $N, but misses.",
            to_char: b"You try to #w $N, but miss.",
            to_victim: b"$n tries to #w you, but misses.",
        },
        DamWeapon {
            to_room: b"$n tickles $N as $e #W $M.",
            to_char: b"You tickle $N as you #w $M.",
            to_victim: b"$n tickles you as $e #W you.",
        },
        DamWeapon {
            to_room: b"$n barely #W $N.",
            to_char: b"You barely #w $N.",
            to_victim: b"$n barely #W you.",
        },
        DamWeapon {
            to_room: b"$n #W $N.",
            to_char: b"You #w $N.",
            to_victim: b"$n #W you.",
        },
        DamWeapon {
            to_room: b"$n #W $N hard.",
            to_char: b"You #w $N hard.",
            to_victim: b"$n #W you hard.",
        },
        DamWeapon {
            to_room: b"$n #W $N very hard.",
            to_char: b"You #w $N very hard.",
            to_victim: b"$n #W you very hard.",
        },
        DamWeapon {
            to_room: b"$n #W $N extremely hard.",
            to_char: b"You #w $N extremely hard.",
            to_victim: b"$n #W you extremely hard.",
        },
        DamWeapon {
            to_room: b"$n massacres $N to small fragments with $s #w.",
            to_char: b"You massacre $N to small fragments with your #w.",
            to_victim: b"$n massacres you to small fragments with $s #w.",
        },
        DamWeapon {
            to_room: b"$n OBLITERATES $N with $s deadly #w!!",
            to_char: b"You OBLITERATE $N with your deadly #w!!",
            to_victim: b"$n OBLITERATES you with $s deadly #w!!",
        },
    ];

    let w = (w_type - TYPE_HIT) as usize;
    let (singular, plural) = ATTACK_HIT_TEXT[w];

    let msgnum = match dam {
        0 => 0,
        1..=2 => 1,
        3..=4 => 2,
        5..=6 => 3,
        7..=10 => 4,
        11..=14 => 5,
        15..=19 => 6,
        20..=23 => 7,
        _ => 8,
    };

    let buf = replace_string(DAM_WEAPONS[msgnum].to_room, singular, plural);
    act(g, &buf, false, Some(chid), None, Some(victim), TO_NOTVICT);

    if g.ch(chid).level >= LVL_IMMORT {
        send_to_char(g, chid, format!("({}) ", dam).as_bytes());
    }
    let buf = replace_string(DAM_WEAPONS[msgnum].to_char, singular, plural);
    act(g, &buf, false, Some(chid), None, Some(victim), TO_CHAR);
    let n = cc(g, chid, C_CMP, KNRM);
    send_to_char(g, chid, n);

    if g.ch(victim).level >= LVL_IMMORT {
        send_to_char(g, victim, format!("\tR({})", dam).as_bytes());
    }
    let buf = replace_string(DAM_WEAPONS[msgnum].to_victim, singular, plural);
    act(g, &buf, false, Some(chid), None, Some(victim), TO_VICT | TO_SLEEP);
    let n = cc(g, victim, C_CMP, KNRM);
    send_to_char(g, victim, n);
}

/// skill_message. Returns true if an entry existed.
pub fn skill_message(g: &mut Game, dam: i32, chid: CharId, vict: CharId, attacktype: i32) -> bool {
    let weap = g.ch(chid).equipment[WEAR_WIELD];
    let Some(slot) = g.fight_messages.iter().position(|e| e.a_type == attacktype) else {
        return false;
    };
    let count = g.fight_messages[slot].number_of_attacks;
    let nr = g.rng.dice(1, count) as usize;
    // Walk nr-1 links from the head (prepend order).
    let idx = (nr - 1).min(g.fight_messages[slot].msg.len().saturating_sub(1));
    let msg = g.fight_messages[slot].msg[idx].clone();

    let vict_is_god = !g.ch(vict).is_npc() && g.ch(vict).level >= LVL_IMPL;
    if vict_is_god {
        if let Some(m) = &msg.god.attacker {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_CHAR);
        }
        if let Some(m) = &msg.god.victim {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_VICT);
        }
        if let Some(m) = &msg.god.room {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_NOTVICT);
        }
    } else if dam != 0 {
        let triple = if g.ch(vict).position == POS_DEAD { &msg.die } else { &msg.hit };
        if let Some(m) = &triple.attacker {
            let y = cc(g, chid, C_CMP, KYEL);
            send_to_char(g, chid, y);
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_CHAR);
            let n = cc(g, chid, C_CMP, KNRM);
            send_to_char(g, chid, n);
        }
        let r = cc(g, vict, C_CMP, KRED);
        send_to_char(g, vict, r);
        if let Some(m) = &triple.victim {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_VICT | TO_SLEEP);
        }
        let n = cc(g, vict, C_CMP, KNRM);
        send_to_char(g, vict, n);
        if let Some(m) = &triple.room {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_NOTVICT);
        }
    } else if chid != vict {
        if let Some(m) = &msg.miss.attacker {
            let y = cc(g, chid, C_CMP, KYEL);
            send_to_char(g, chid, y);
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_CHAR);
            let n = cc(g, chid, C_CMP, KNRM);
            send_to_char(g, chid, n);
        }
        let r = cc(g, vict, C_CMP, KRED);
        send_to_char(g, vict, r);
        if let Some(m) = &msg.miss.victim {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_VICT | TO_SLEEP);
        }
        let n = cc(g, vict, C_CMP, KNRM);
        send_to_char(g, vict, n);
        if let Some(m) = &msg.miss.room {
            act_full(g, m, false, Some(chid), weap, ActArg::Char(vict), TO_NOTVICT);
        }
    }
    true
}

// ---- DG trigger hooks (stage 6) ----

fn fight_mtrigger(g: &mut Game, chid: CharId) {
    crate::dg::triggers::fight_mtrigger(g, chid);
}
fn hitprcnt_mtrigger(g: &mut Game, chid: CharId) {
    crate::dg::triggers::hitprcnt_mtrigger(g, chid);
}
fn damage_mtrigger(g: &mut Game, chid: CharId, vict: CharId, dam: i32, attacktype: i32) -> i32 {
    crate::dg::triggers::damage_mtrigger(g, chid, vict, dam, attacktype)
}
fn death_mtrigger(g: &mut Game, chid: CharId, killer: CharId) -> bool {
    crate::dg::triggers::death_mtrigger(g, chid, Some(killer)) != 0
}

/// damage. Returns -1 victim died, 0 no damage, else dam.
pub fn damage(g: &mut Game, chid: CharId, victim: CharId, mut dam: i32, attacktype: i32) -> i32 {
    // 1. Corpse guard.
    if g.ch(victim).position <= POS_DEAD {
        if g.ch(victim).plr(flags::PLR_NOTDEADYET) || g.ch(victim).mob_flagged(flags::MOB_NOTDEADYET) {
            return -1;
        }
        let vname = String::from_utf8_lossy(g.ch(victim).get_name()).into_owned();
        let cname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let room = g.ch(victim).in_room;
        let vnum = if room != NOWHERE { g.world.rooms[room as usize].vnum as i32 } else { -1 };
        g.log(format!("SYSERR: Attempt to damage corpse '{}' in room #{} by '{}'.", vname, vnum, cname));
        die(g, victim, Some(chid));
        return -1;
    }

    // 2. PK gate.
    if !pk_allowed(g, chid, victim) {
        send_to_char(g, chid, b"Player killing is not permitted.\r\n");
        return 0;
    }

    // 3. Peaceful room (the DG caster proxy, mob vnum 1, bypasses).
    let proxy_rnum = g.world.real_mobile(1);
    let ch_rnum = g.ch(chid).mob_rnum;
    let room = g.ch(chid).in_room;
    if Some(ch_rnum) != proxy_rnum
        && chid != victim
        && room != NOWHERE
        && g.world.rooms[room as usize].room_flags[0] & (1 << flags::ROOM_PEACEFUL) != 0
    {
        send_to_char(g, chid, b"This room just has such a peaceful, easy feeling...\r\n");
        return 0;
    }

    // 4. Shopkeeper / NOKILL protection.
    if !crate::shop::ok_damage_shopkeeper(g, chid, victim) || g.ch(victim).mob_flagged(flags::MOB_NOKILL) {
        send_to_char(g, chid, b"This mob is protected.\r\n");
        return 0;
    }

    // 5. Immortal victim.
    if !g.ch(victim).is_npc() && g.ch(victim).level >= LVL_IMMORT && g.ch(victim).prf(flags::PRF_NOHASSLE) {
        dam = 0;
    }

    // 6. DAMAGE mtrigger (stage 6; -1 aborts).
    dam = damage_mtrigger(g, chid, victim, dam, attacktype);
    if dam == -1 {
        return 0;
    }

    // 7. Auto-engage.
    if victim != chid {
        if g.ch(chid).position > POS_STUNNED && g.ch(chid).fighting.is_none() {
            set_fighting(g, chid, victim);
        }
        if g.ch(victim).position > POS_STUNNED && g.ch(victim).fighting.is_none() {
            set_fighting(g, victim, chid);
            if g.ch(victim).mob_flagged(flags::MOB_MEMORY) && !g.ch(chid).is_npc() {
                crate::mobact::remember(g, victim, chid);
            }
        }
    }

    // 8. Pet betrayal.
    if g.ch(victim).master == Some(chid) {
        crate::act::movement::stop_follower(g, victim);
    }

    // 9. Reveal attacker.
    if g.ch(chid).aff(flags::AFF_INVISIBLE) || g.ch(chid).aff(flags::AFF_HIDE) {
        crate::act::other::appear(g, chid);
    }

    // 10. Sanctuary.
    if g.ch(victim).aff(flags::AFF_SANCTUARY) && dam >= 2 {
        dam /= 2;
    }

    // 11. Cap 100, floor 0; subtract.
    // The per-round ceiling is a balance decision rather than a storage
    // artifact, and it does not govern `%damage%`: script_damage writes hit
    // points directly and never enters this function. What the ceiling keeps
    // is the
    // widening that stops a large script blow wrapping positive and
    // healing its victim to full.
    dam = dam.min(100).max(0);
    g.ch_mut(victim).points.hit -= dam;

    // 13. Exp for the hit.
    if chid != victim {
        let vl = g.ch(victim).level as i32;
        crate::limits::gain_exp(g, chid, vl * dam);
    }

    update_pos(g, victim);

    // 15. Message selection.
    if !is_weapon_type(attacktype) {
        skill_message(g, dam, chid, victim, attacktype);
    } else if g.ch(victim).position == POS_DEAD || dam == 0 {
        if !skill_message(g, dam, chid, victim, attacktype) {
            dam_message(g, dam, chid, victim, attacktype);
        }
    } else {
        dam_message(g, dam, chid, victim, attacktype);
    }

    // 16. Position report to victim.
    match g.ch(victim).position {
        POS_MORTALLYW => {
            act(g, b"$n is mortally wounded, and will die soon, if not aided.", true, Some(victim), None, None, TO_ROOM);
            send_to_char(g, victim, b"You are mortally wounded, and will die soon, if not aided.\r\n");
        }
        POS_INCAP => {
            act(g, b"$n is incapacitated and will slowly die, if not aided.", true, Some(victim), None, None, TO_ROOM);
            send_to_char(g, victim, b"You are incapacitated and will slowly die, if not aided.\r\n");
        }
        POS_STUNNED => {
            act(g, b"$n is stunned, but will probably regain consciousness again.", true, Some(victim), None, None, TO_ROOM);
            send_to_char(g, victim, b"You're stunned, but will probably regain consciousness again.\r\n");
        }
        POS_DEAD => {
            act(g, b"$n is dead!  R.I.P.", false, Some(victim), None, None, TO_ROOM);
            send_to_char(g, victim, b"You are dead!  Sorry...\r\n");
        }
        _ => {
            // >= POSITION SLEEPING
            let max_hit = g.ch(victim).points.max_hit;
            if dam > max_hit / 4 {
                send_to_char(g, victim, b"That really did HURT!\r\n");
            }
            if g.ch(victim).points.hit < max_hit / 4 {
                let mut out: Vec<u8> = Vec::new();
                out.extend_from_slice(cc(g, victim, C_SPR, KRED));
                out.extend_from_slice(b"You wish that your wounds would stop BLEEDING so much!");
                out.extend_from_slice(cc(g, victim, C_SPR, KNRM));
                out.extend_from_slice(b"\r\n");
                send_to_char(g, victim, &out);
                if chid != victim && g.ch(victim).mob_flagged(flags::MOB_WIMPY) {
                    crate::act::offensive::do_flee(g, victim, b"", 0, 0);
                }
            }
            let wimp = if g.ch(victim).is_npc() { 0 } else { g.ch(victim).ps().wimp_level };
            let hp = g.ch(victim).points.hit;
            if !g.ch(victim).is_npc() && wimp != 0 && victim != chid && hp < wimp && hp > 0 {
                send_to_char(g, victim, b"You wimp out, and attempt to flee!\r\n");
                crate::act::offensive::do_flee(g, victim, b"", 0, 0);
            }
        }
    }

    // 17. Linkdead rescue.
    if !g.ch(victim).is_npc() && g.ch(victim).desc.is_none() && g.ch(victim).position > POS_STUNNED {
        crate::act::offensive::do_flee(g, victim, b"", 0, 0);
        if g.ch(victim).fighting.is_none() {
            act(g, b"$n is rescued by divine forces.", false, Some(victim), None, None, TO_ROOM);
            let was = g.ch(victim).in_room;
            g.ch_mut(victim).was_in_room = was;
            char_from_room(g, victim);
            char_to_room(g, victim, 0);
        }
    }

    // 18. Stunned or worse stops fighting.
    if g.ch(victim).position <= POS_STUNNED && g.ch(victim).fighting.is_some() {
        stop_fighting(g, victim);
    }

    // 19. Death block.
    if g.ch(victim).position == POS_DEAD {
        if chid != victim && (g.ch(victim).is_npc() || g.ch(victim).desc.is_some()) {
            if g.ch(chid).group.is_some() {
                group_gain(g, chid, victim);
            } else {
                solo_gain(g, chid, victim);
            }
        }

        if !g.ch(victim).is_npc() {
            let vname = String::from_utf8_lossy(g.ch(victim).get_name()).into_owned();
            let cname = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let room = g.ch(victim).in_room;
            let rname = String::from_utf8_lossy(
                g.world.rooms[room as usize].name.as_deref().unwrap_or(b""),
            )
            .into_owned();
            let lvl = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()).max(g.ch(victim).invis_lev()) as u8;
            g.mudlog(MudlogKind::Brf, lvl, true, &format!("{} killed by {} at {}", vname, cname, rname));
            if g.ch(chid).mob_flagged(flags::MOB_MEMORY) {
                crate::mobact::forget(g, chid, victim);
            }
        }
        let mut local_gold = 0;
        if g.ch(victim).is_npc() {
            // GET_GOLD can't be read off the corpse later, so the happy-hour
            // top-up goes onto the victim first and rides into the corpse
            // Note the rate is applied as a bare
            // percentage of the pile, not 100+rate.
            if crate::act::other::is_happyhour(g) && g.happy.gold_rate > 0 {
                let gold = g.ch(victim).points.gold;
                let happy_gold =
                    ((gold as f32 * (g.happy.gold_rate as f32 / 100.0)) as i32).max(0);
                crate::limits::increase_gold(g, victim, happy_gold);
            }
            local_gold = g.ch(victim).points.gold;
        }

        die(g, victim, Some(chid));

        let ch_alive = g.try_ch(chid).is_some();
        if ch_alive
            && g.ch(chid).group.is_some()
            && local_gold > 0
            && g.ch(chid).prf(flags::PRF_AUTOSPLIT)
        {
            // grab the coins and split the stored amount.
            let (_, _, corpse_obj) =
                crate::handler::generic_find(g, chid, b"corpse", crate::handler::FIND_OBJ_ROOM);
            if corpse_obj.is_some() {
                crate::act::item::do_get(g, chid, b"all.coin last.corpse", 0, 0);
                let amt = format!("{}", local_gold);
                crate::act::other::do_split(g, chid, amt.as_bytes(), 0, 0);
            }
        } else if ch_alive && !g.ch(chid).is_npc() && chid != victim && g.ch(chid).prf(flags::PRF_AUTOGOLD) {
            crate::act::item::do_get(g, chid, b"all.coin last.corpse", 0, 0);
        }
        if ch_alive && !g.ch(chid).is_npc() && chid != victim && g.ch(chid).prf(flags::PRF_AUTOLOOT) {
            // last.corpse, not corpse: obj_to_room appends, so a plain name
            // finds the oldest corpse in the room, not the one just made.
            crate::act::item::do_get(g, chid, b"all last.corpse", 0, 0);
        }
        if g.try_ch(victim).map(|v| v.is_npc()).unwrap_or(true) && ch_alive && !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_AUTOSAC) {
            crate::act::item::do_sac(g, chid, b"last.corpse", 0, 0);
        }
        return -1;
    }
    dam
}

pub fn compute_thaco(g: &Game, chid: CharId, _victim: CharId) -> i32 {
    let ch = g.ch(chid);
    let mut calc_thaco = if !ch.is_npc() {
        mud_data::tables::thaco(ch.class as i32, ch.level as i32)
    } else {
        20
    };
    calc_thaco -= mud_data::tables::STR_APP[strength_apply_index(g, chid)].0;
    calc_thaco -= g.ch(chid).points.hitroll as i32;
    calc_thaco -= ((g.ch(chid).aff_abils.intel as f64 - 13.0) / 1.5) as i32;
    calc_thaco -= ((g.ch(chid).aff_abils.wis as f64 - 13.0) / 1.5) as i32;
    calc_thaco
}

pub fn hit(g: &mut Game, chid: CharId, victim: CharId, type_: i32) {
    if g.try_ch(chid).is_none() || g.try_ch(victim).is_none() {
        return;
    }

    fight_mtrigger(g, chid);

    if g.ch(chid).in_room != g.ch(victim).in_room {
        if g.ch(chid).fighting == Some(victim) {
            stop_fighting(g, chid);
        }
        return;
    }

    let wielded = g.ch(chid).equipment[WEAR_WIELD]
        .filter(|&w| g.obj(w).type_flag == flags::ITEM_WEAPON);
    let w_type = if let Some(w) = wielded {
        g.obj(w).values[3] + TYPE_HIT
    } else if g.ch(chid).is_npc() && g.ch(chid).mob_specials.attack_type != 0 {
        g.ch(chid).mob_specials.attack_type + TYPE_HIT
    } else {
        TYPE_HIT
    };

    let calc_thaco = compute_thaco(g, chid, victim);
    let victim_ac = crate::act::informative::compute_armor_class(g, victim) / 10;
    let diceroll = g.rng.rand_number(1, 20);

    if g.config.debug_mode >= 2 {
        send_to_char(
            g,
            chid,
            format!(
                "\t1Debug:\r\n   \t2Thaco: \t3{}\r\n   \t2AC: \t3{}\r\n   \t2Diceroll: \t3{}\tn\r\n",
                calc_thaco, victim_ac, diceroll
            )
            .as_bytes(),
        );
    }

    let landed = if diceroll == 20 || !g.ch(victim).awake() {
        true
    } else if diceroll == 1 {
        false
    } else {
        calc_thaco - diceroll <= victim_ac
    };

    if !landed {
        let t = if type_ == SKILL_BACKSTAB { SKILL_BACKSTAB } else { w_type };
        damage(g, chid, victim, 0, t);
    } else {
        let mut dam = mud_data::tables::STR_APP[strength_apply_index(g, chid)].1;
        dam += g.ch(chid).points.damroll as i32;

        if let Some(w) = wielded {
            let (n, s) = (g.obj(w).values[1], g.obj(w).values[2]);
            dam += g.rng.dice(n, s);
        } else if g.ch(chid).is_npc() {
            let (n, s) = (g.ch(chid).mob_specials.damnodice as i32, g.ch(chid).mob_specials.damsizedice as i32);
            dam += g.rng.dice(n, s);
        } else {
            dam += g.rng.rand_number(0, 2);
        }

        let vpos = g.ch(victim).position as i32;
        if vpos < POS_FIGHTING as i32 {
            dam *= 1 + (POS_FIGHTING as i32 - vpos) / 3;
        }
        dam = dam.max(1);

        if type_ == SKILL_BACKSTAB {
            let mult = mud_data::tables::BACKSTAB_MULT[(g.ch(chid).level as usize).min(34)];
            damage(g, chid, victim, dam * mult, SKILL_BACKSTAB);
        } else {
            damage(g, chid, victim, dam, w_type);
        }
    }

    if g.try_ch(victim).is_some() {
        hitprcnt_mtrigger(g, victim);
    }
}

/// perform_violence. The cursor is patched by stop_fighting mid-walk.
pub fn perform_violence(g: &mut Game) {
    let mut cur = g.combat_list.first().copied();
    while let Some(chid) = cur {
        // next_combat_list = ch->next_fighting.
        let pos = g.combat_list.iter().position(|&c| c == chid);
        g.next_combat = pos.and_then(|p| g.combat_list.get(p + 1)).copied();

        'body: {
            if g.try_ch(chid).is_none() {
                break 'body;
            }
            let fighting = g.ch(chid).fighting;
            let same_room = fighting
                .and_then(|f| g.try_ch(f))
                .is_some_and(|f| f.in_room == g.ch(chid).in_room);
            if fighting.is_none() || !same_room {
                stop_fighting(g, chid);
                break 'body;
            }

            if g.ch(chid).is_npc() {
                if g.ch(chid).wait > 0 {
                    g.ch_mut(chid).wait -= PULSE_VIOLENCE as i32;
                    break 'body;
                }
                g.ch_mut(chid).wait = 0;
                if g.ch(chid).position < POS_FIGHTING {
                    g.ch_mut(chid).position = POS_FIGHTING;
                    act(g, b"$n scrambles to $s feet!", true, Some(chid), None, None, TO_ROOM);
                }
            }

            if g.ch(chid).position < POS_FIGHTING {
                send_to_char(g, chid, b"You can't fight while sitting!!\r\n");
                break 'body;
            }

            // Group auto-assist: standing, unengaged
            // members in the room join via do_assist BY NAME. NPC members
            // always assist; PCs need PRF_AUTOASSIST.
            if let Some(gr) = g.group_of(chid) {
                if !gr.members.is_empty() {
                    let members = gr.members.clone();
                    let room = g.ch(chid).in_room;
                    let name = g.ch(chid).get_name().to_vec();
                    for tch in members {
                        if tch == chid {
                            continue;
                        }
                        let Some(t) = g.try_ch(tch) else { continue };
                        if !t.is_npc() && !t.prf(flags::PRF_AUTOASSIST) {
                            continue;
                        }
                        if t.in_room != room {
                            continue;
                        }
                        if t.fighting.is_some() {
                            continue;
                        }
                        if t.position != POS_STANDING {
                            continue;
                        }
                        if !crate::handler::can_see(g, tch, chid) {
                            continue;
                        }
                        crate::act::offensive::do_assist(g, tch, &name, 0, 0);
                    }
                }
            }

            // The swing is unconditional after the assist loop — even when
            // the
            // assists just killed the victim (the corpse-pending target still
            // absorbs the to-hit roll; damage then returns -1 silently).
            let Some(vict) = g.ch(chid).fighting else { break 'body };
            hit(g, chid, vict, TYPE_UNDEFINED);

            // Combat spec procs act each round — unlike mobact, this call
            // is NOT gated on no_specials.
            if g.try_ch(chid).is_some()
                && g.ch(chid).mob_flagged(flags::MOB_SPEC)
                && !g.ch(chid).mob_flagged(flags::MOB_NOTDEADYET)
            {
                let rnum = g.ch(chid).mob_rnum;
                if rnum != NOBODY {
                    if let Some(spec) = g.mob_specs.get(rnum as usize).copied().flatten() {
                        crate::spec::call_mob_spec(g, spec, chid, chid, 0, b"");
                    }
                }
            }
        }

        cur = g.next_combat;
    }
    g.next_combat = None;
}
