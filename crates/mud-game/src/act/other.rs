//! quit, save, not_here, gen_tog toggles, title, visible,
//! display/prompt, plus do_alias and do_echo/emote.

use mud_data::flags::{self};
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::BStr;
use crate::ch::{Alias, ALIAS_COMPLEX, ALIAS_SIMPLE};
use crate::comm::{self, act, send_to_char};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::handler::is_abbrev;
use crate::interpreter::{any_one_arg, delete_doubledollar, skip_spaces};
use crate::text::parse_at;

pub fn do_quit(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_QUIT;
    if g.ch(chid).is_npc() || g.ch(chid).desc.is_none() {
        return;
    }
    let level = g.ch(chid).level;
    if subcmd != SCMD_QUIT && level < LVL_IMMORT {
        send_to_char(g, chid, b"You have to type quit--no less, to quit!\r\n");
        return;
    }
    if g.ch(chid).fighting.is_some() {
        send_to_char(g, chid, b"No way!  You're fighting for your life!\r\n");
        return;
    }
    if g.ch(chid).position < POS_STUNNED {
        send_to_char(g, chid, b"You die before your time...\r\n");
        // die arrives with combat (stage 4); extraction still happens.
        crate::handler::extract_char(g, chid);
        return;
    }
    act(g, b"$n has left the game.", true, Some(chid), None, None, comm::TO_ROOM);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let invis = g.ch(chid).invis_lev();
    g.mudlog(
        MudlogKind::Nrm,
        (LVL_IMMORT as i16).max(invis) as u8,
        true,
        &format!("{} has quit the game.", name),
    );
    // quest_timeout: stage 7.
    send_to_char(g, chid, b"Goodbye, friend.. Come back soon!\r\n");

    // Free rent stores everything; otherwise the gear hits the floor in
    // extract_char_final and the crash file is deleted behind it.
    if g.config.free_rent {
        crate::objsave::crash_rentsave(g, chid, 0);
    }
    let room = g.ch(chid).in_room;
    if room != NOWHERE {
        let vnum = g.world.rooms[room as usize].vnum;
        g.ch_mut(chid).ps_mut().load_room = vnum;
    }
    crate::handler::extract_char(g, chid); // Char is saved before extracting.
}

pub fn do_save(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).desc.is_none() {
        return;
    }
    let name = g.ch(chid).get_name().to_vec();
    let mut msg = b"Saving ".to_vec();
    msg.extend_from_slice(&name);
    msg.extend_from_slice(b".\r\n");
    send_to_char(g, chid, &msg);
    crate::players_glue::save_char(g, chid);
    crate::objsave::crash_crashsave(g, chid);
    let room = g.ch(chid).in_room;
    if room != NOWHERE {
        let vnum = g.world.rooms[room as usize].vnum;
        if crate::handler::room_flagged(g, room, flags::ROOM_HOUSE_CRASH) {
            crate::house::house_crashsave(g, vnum as i32);
        }
        g.ch_mut(chid).ps_mut().load_room = vnum;
    }
}

pub fn do_not_here(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    send_to_char(g, chid, b"Sorry, but you cannot do that here!\r\n");
}

pub fn do_visible(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).level >= LVL_IMMORT {
        // perform_immort_vis.
        if g.ch(chid).invis_lev() == 0
            && !g.ch(chid).aff(flags::AFF_HIDE)
            && !g.ch(chid).aff(flags::AFF_INVISIBLE)
        {
            send_to_char(g, chid, b"You are already fully visible.\r\n");
            return;
        }
        g.ch_mut(chid).ps_mut().invis_level = 0;
        appear(g, chid);
        send_to_char(g, chid, b"You are now fully visible.\r\n");
        return;
    }
    if g.ch(chid).aff(flags::AFF_INVISIBLE) {
        appear(g, chid);
        send_to_char(g, chid, b"You break the spell of invisibility.\r\n");
    } else {
        send_to_char(g, chid, b"You are already visible.\r\n");
    }
}

pub fn appear(g: &mut Game, chid: CharId) {
    if crate::handler::affected_by_spell(g, chid, mud_data::spells::SPELL_INVISIBLE as i16) {
        crate::handler::affect_from_char(g, chid, mud_data::spells::SPELL_INVISIBLE as i16);
    }
    g.ch_mut(chid).affected_by.remove(flags::AFF_INVISIBLE);
    g.ch_mut(chid).affected_by.remove(flags::AFF_HIDE);
    if g.ch(chid).level < LVL_IMMORT {
        act(g, b"$n slowly fades into existence.", false, Some(chid), None, None, comm::TO_ROOM);
    } else {
        act(
            g,
            b"You feel a strange presence as $n appears, seemingly from nowhere.",
            false,
            Some(chid),
            None,
            None,
            comm::TO_ROOM,
        );
    }
}

pub fn do_sneak(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    use mud_data::spells::SKILL_SNEAK;
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_SNEAK) == 0 {
        send_to_char(g, chid, b"You have no idea how to do that.\r\n");
        return;
    }
    send_to_char(g, chid, b"Okay, you'll try to move silently for a while.\r\n");
    if g.ch(chid).aff(flags::AFF_SNEAK) {
        crate::handler::affect_from_char(g, chid, SKILL_SNEAK as i16);
    }

    let percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let dex = g.ch(chid).aff_abils.dex.clamp(0, 25) as usize;
    if percent > g.ch(chid).get_skill(SKILL_SNEAK) + mud_data::tables::DEX_APP_SKILL[dex].3 {
        return;
    }

    let mut af = crate::ch::Affect { spell: SKILL_SNEAK as i16, ..Default::default() };
    af.duration = g.ch(chid).level as i16;
    af.bitvector.set(flags::AFF_SNEAK);
    crate::handler::affect_to_char(g, chid, af);
}

/// do_hide: a raw AFF bit, no affect entry.
pub fn do_hide(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    use mud_data::spells::SKILL_HIDE;
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_HIDE) == 0 {
        send_to_char(g, chid, b"You have no idea how to do that.\r\n");
        return;
    }
    send_to_char(g, chid, b"You attempt to hide yourself.\r\n");

    if g.ch(chid).aff(flags::AFF_HIDE) {
        g.ch_mut(chid).affected_by.remove(flags::AFF_HIDE);
    }

    let percent = g.rng.rand_number(1, 101); // 101% is a complete failure
    let dex = g.ch(chid).aff_abils.dex.clamp(0, 25) as usize;
    if percent > g.ch(chid).get_skill(SKILL_HIDE) + mud_data::tables::DEX_APP_SKILL[dex].4 {
        return;
    }
    g.ch_mut(chid).affected_by.set(flags::AFF_HIDE);
}

pub fn do_steal(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use mud_data::spells::{SKILL_STEAL, TYPE_UNDEFINED};
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_STEAL) == 0 {
        send_to_char(g, chid, b"You have no idea how to do that.\r\n");
        return;
    }
    {
        let room = g.ch(chid).in_room;
        if room != NOWHERE
            && g.world.rooms[room as usize].room_flags[flags::ROOM_PEACEFUL / 32]
                & (1 << (flags::ROOM_PEACEFUL % 32))
                != 0
        {
            send_to_char(g, chid, b"This room just has such a peaceful, easy feeling...\r\n");
            return;
        }
    }

    let (obj_name_arg, vict_name, _) = crate::interpreter::two_arguments(argument);

    let Some(vict) = crate::handler::get_char_room_vis(g, chid, &vict_name, None) else {
        send_to_char(g, chid, b"Steal what from who?\r\n");
        return;
    };
    if vict == chid {
        send_to_char(g, chid, b"Come on now, that's rather stupid!\r\n");
        return;
    }

    // Check if player stealing is allowed.
    let mut pcsteal = false;
    if !g.ch(vict).is_npc() {
        if g.config.pt_setting == 0 {
            send_to_char(g, chid, b"Stealing from players is not allowed.\r\n");
            return;
        }
        pcsteal = g.config.pt_setting == 1;
    }

    // 101% is a complete failure.
    let dex = g.ch(chid).aff_abils.dex.clamp(0, 25) as usize;
    let mut percent = g.rng.rand_number(1, 101) - mud_data::tables::DEX_APP_SKILL[dex].0;

    if g.ch(vict).position < POS_SLEEPING {
        percent = -1; // ALWAYS SUCCESS, unless heavy object.
    }
    if !g.ch(vict).awake() {
        // Easier to steal from sleeping people.
        percent -= 50;
    }
    // No stealing from Imm's or Shopkeepers.
    let vict_is_keeper = g.ch(vict).is_npc()
        && g.ch(vict).mob_rnum != NOBODY
        && g.mob_specs.get(g.ch(vict).mob_rnum as usize).copied().flatten()
            == Some(crate::spec::MobSpec::ShopKeeper);
    if g.ch(vict).level as i32 >= LVL_IMMORT as i32 || vict_is_keeper {
        percent = 101; // Failure
    }

    let mut ohoh = false;
    let skill = g.ch(chid).get_skill(SKILL_STEAL);

    if obj_name_arg != b"coins" && obj_name_arg != b"gold" {
        let carrying = g.ch(vict).carrying.clone();
        let inv_obj = crate::handler::get_obj_in_list_vis(g, chid, &obj_name_arg, None, &carrying);

        if inv_obj.is_none() {
            let mut eq_obj = None;
            let mut eq_pos = 0;
            for pos in 0..NUM_WEARS {
                if let Some(o) = g.ch(vict).equipment[pos] {
                    if crate::handler::isname(&obj_name_arg, crate::handler::obj_name(g, o))
                        && crate::handler::can_see_obj(g, chid, o)
                    {
                        eq_obj = Some(o);
                        eq_pos = pos;
                        break;
                    }
                }
            }
            let Some(eqo) = eq_obj else {
                act(g, b"$E hasn't got that item.", false, Some(chid), None, Some(vict), comm::TO_CHAR);
                return;
            };
            // It is equipment.
            if g.ch(vict).position > POS_STUNNED {
                send_to_char(g, chid, b"Steal the equipment now?  Impossible!\r\n");
                return;
            }
            if crate::dg::triggers::give_otrigger(g, eqo, vict, chid) == 0
                || crate::dg::triggers::receive_mtrigger(g, chid, vict, eqo) == 0
            {
                send_to_char(g, chid, b"Impossible!\r\n");
                return;
            }
            if g.try_obj(eqo).is_none() {
                return;
            }
            act(g, b"You unequip $p and steal it.", false, Some(chid), Some(eqo), None, comm::TO_CHAR);
            act(g, b"$n steals $p from $N.", false, Some(chid), Some(eqo), Some(vict), comm::TO_NOTVICT);
            if let Some(o) = crate::handler::unequip_char(g, vict, eq_pos) {
                crate::handler::obj_to_char(g, o, chid);
            }
        } else {
            let obj = inv_obj.unwrap();
            percent += g.obj(obj).weight; // Make heavy harder.

            if percent > skill {
                ohoh = true;
                send_to_char(g, chid, b"Oops..\r\n");
                // Player got caught and stealing is limited via cedit.
                if pcsteal && !g.ch(chid).plr(flags::PLR_THIEF) {
                    g.ch_mut(chid).act.set(flags::PLR_THIEF);
                }
                act(g, b"$n tried to steal something from you!", false, Some(chid), None, Some(vict), comm::TO_VICT);
                act(g, b"$n tries to steal something from $N.", true, Some(chid), None, Some(vict), comm::TO_NOTVICT);
            } else {
                // Steal the item.
                if (g.ch(chid).carry_items as i32) + 1 < crate::handler::can_carry_n(g.ch(chid)) {
                    if crate::dg::triggers::give_otrigger(g, obj, vict, chid) == 0
                        || crate::dg::triggers::receive_mtrigger(g, chid, vict, obj) == 0
                    {
                        send_to_char(g, chid, b"Impossible!\r\n");
                        return;
                    }
                    if g.try_obj(obj).is_none() {
                        return;
                    }
                    if g.ch(chid).carry_weight + g.obj(obj).weight < crate::handler::can_carry_w(g.ch(chid)) {
                        crate::handler::obj_from_char(g, obj);
                        crate::handler::obj_to_char(g, obj, chid);
                        send_to_char(g, chid, b"Got it!\r\n");
                    }
                } else {
                    send_to_char(g, chid, b"You cannot carry that much.\r\n");
                }
            }
        }
    } else {
        // Steal some coins.
        if g.ch(vict).awake() && percent > skill {
            ohoh = true;
            // Player got caught and stealing is limited via cedit.
            if pcsteal && !g.ch(chid).plr(flags::PLR_THIEF) {
                g.ch_mut(chid).act.set(flags::PLR_THIEF);
            }
            send_to_char(g, chid, b"Oops..\r\n");
            act(g, b"You discover that $n has $s hands in your wallet.", false, Some(chid), None, Some(vict), comm::TO_VICT);
            act(g, b"$n tries to steal gold from $N.", true, Some(chid), None, Some(vict), comm::TO_NOTVICT);
        } else {
            // Steal some gold coins.
            let mut gold = (g.ch(vict).points.gold * g.rng.rand_number(1, 10)) / 100;
            gold = gold.min(1782);
            if gold > 0 {
                crate::limits::increase_gold(g, chid, gold);
                crate::limits::decrease_gold(g, vict, gold);
                if gold > 1 {
                    let msg = format!("Bingo!  You got {} gold coins.\r\n", gold);
                    send_to_char(g, chid, msg.as_bytes());
                } else {
                    send_to_char(g, chid, b"You manage to swipe a solitary gold coin.\r\n");
                }
            } else {
                send_to_char(g, chid, b"You couldn't get any gold...\r\n");
            }
        }
    }

    if ohoh && g.ch(vict).is_npc() && g.ch(vict).awake() {
        crate::fight::hit(g, vict, chid, TYPE_UNDEFINED);
    }
}

/// do_use: quaff/recite/use.
pub fn do_use(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, subcmd: i32) {
    use crate::interpreter::{SCMD_QUAFF, SCMD_RECITE, SCMD_USE};
    let (arg, buf) = crate::interpreter::half_chop(argument);
    if arg.is_empty() {
        let cmd_name = g.commands[cmd].command.clone();
        let mut msg = b"What do you want to ".to_vec();
        msg.extend_from_slice(&cmd_name);
        msg.extend_from_slice(b"?\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let held = g.ch(chid).equipment[WEAR_HOLD];
    let mut mag_item = held.filter(|&o| crate::handler::isname(&arg, crate::handler::obj_name(g, o)));

    if mag_item.is_none() {
        match subcmd {
            SCMD_RECITE | SCMD_QUAFF => {
                let carrying = g.ch(chid).carrying.clone();
                mag_item = crate::handler::get_obj_in_list_vis(g, chid, &arg, None, &carrying);
                if mag_item.is_none() {
                    let an: &[u8] =
                        if arg.first().is_some_and(|c| b"aeiouAEIOU".contains(c)) { b"an" } else { b"a" };
                    let mut msg = b"You don't seem to have ".to_vec();
                    msg.extend_from_slice(an);
                    msg.push(b' ');
                    msg.extend_from_slice(&arg);
                    msg.extend_from_slice(b".\r\n");
                    send_to_char(g, chid, &msg);
                    return;
                }
            }
            SCMD_USE => {
                let an: &[u8] =
                    if arg.first().is_some_and(|c| b"aeiouAEIOU".contains(c)) { b"an" } else { b"a" };
                let mut msg = b"You don't seem to be holding ".to_vec();
                msg.extend_from_slice(an);
                msg.push(b' ');
                msg.extend_from_slice(&arg);
                msg.extend_from_slice(b".\r\n");
                send_to_char(g, chid, &msg);
                return;
            }
            _ => {
                g.log(format!("SYSERR: Unknown subcmd {} passed to do_use.", subcmd));
                return;
            }
        }
    }
    let mag_item = mag_item.unwrap();

    match subcmd {
        SCMD_QUAFF => {
            if g.obj(mag_item).type_flag != flags::ITEM_POTION {
                send_to_char(g, chid, b"You can only quaff potions.\r\n");
                return;
            }
        }
        SCMD_RECITE => {
            if g.obj(mag_item).type_flag != flags::ITEM_SCROLL {
                send_to_char(g, chid, b"You can only recite scrolls.\r\n");
                return;
            }
        }
        SCMD_USE => {
            if g.obj(mag_item).type_flag != flags::ITEM_WAND
                && g.obj(mag_item).type_flag != flags::ITEM_STAFF
            {
                send_to_char(g, chid, b"You can't seem to figure out how to use it.\r\n");
                return;
            }
        }
        _ => {}
    }

    crate::spell_parser::mag_objectmagic(g, chid, mag_item, &buf);
}

/// count_color_chars: `\t\t` counts 1, `\tX` counts 2.
pub fn count_color_chars(s: &[u8]) -> usize {
    let mut num = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        while i < s.len() && s[i] == b'\t' {
            if s.get(i + 1) == Some(&b'\t') {
                num += 1;
            } else {
                num += 2;
            }
            i += 2;
        }
        i += 1;
    }
    num
}

fn print_group(g: &mut Game, chid: CharId) {
    use crate::comm::{cc, C_NRM, KBGRN, KGRN, KNRM};
    send_to_char(g, chid, b"Your group consists of:\r\n");
    let Some(gr) = g.group_of(chid) else { return };
    let members = gr.members.clone();
    let leader = gr.leader;
    for k in members {
        let Some(kc) = g.try_ch(k) else { continue };
        let name = kc.get_name().to_vec();
        let (hit, max_hit, mana, max_mana, mov, max_move) = {
            let p = &kc.points;
            (p.hit, p.max_hit, p.mana, p.max_mana, p.mov, p.max_move)
        };
        let width = count_color_chars(&name) + 22;
        let mut line = name.clone();
        while line.len() < width {
            line.push(b' ');
        }
        line.extend_from_slice(b": ");
        line.extend_from_slice(cc(g, chid, C_NRM, if leader == Some(k) { KBGRN } else { KGRN }));
        line.extend_from_slice(
            format!(
                "[{:>4}/{:<4}]H [{:>4}/{:<4}]M [{:>4}/{:<4}]V",
                hit, max_hit, mana, max_mana, mov, max_move
            )
            .as_bytes(),
        );
        line.extend_from_slice(cc(g, chid, C_NRM, KNRM));
        line.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &line);
    }
}

fn display_group_list(g: &mut Game, chid: CharId) {
    use crate::comm::{cc, C_NRM, KGRN, KNRM, KRED};
    use crate::game::{GROUP_ANON, GROUP_NPC, GROUP_OPEN};
    let mut count = 0usize;
    let groups: Vec<(u64, i32, Option<CharId>, usize)> =
        g.groups.iter().map(|gr| (gr.id, gr.group_flags, gr.leader, gr.members.len())).collect();
    if !groups.is_empty() {
        send_to_char(
            g,
            chid,
            b"#   Group Leader     # of Members    In Zone\r\n---------------------------------------------------\r\n",
        );
        for (_gid, gflags, leader, size) in groups {
            if gflags & GROUP_NPC != 0 {
                continue;
            }
            let leader_alive = leader.and_then(|l| g.try_ch(l).map(|_| l));
            if let (Some(l), true) = (leader_alive, gflags & GROUP_ANON == 0) {
                count += 1;
                let name = g.ch(l).get_name().to_vec();
                let zone = {
                    let room = g.ch(l).in_room;
                    let z = g.world.rooms[room as usize].zone;
                    g.world.zones[z as usize].name.clone().unwrap_or_default()
                };
                let mut line = format!("{:<2}) ", count).into_bytes();
                line.extend_from_slice(cc(g, chid, C_NRM, if gflags & GROUP_OPEN != 0 { KGRN } else { KRED }));
                let mut padded = name.clone();
                while padded.len() < 12 {
                    padded.push(b' ');
                }
                line.extend_from_slice(&padded);
                line.extend_from_slice(format!("     {:<2}              ", size).as_bytes());
                line.extend_from_slice(&zone);
                line.extend_from_slice(cc(g, chid, C_NRM, KNRM));
                line.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &line);
            } else {
                count += 1;
                let line = format!("{:<2}) Hidden\r\n", count).into_bytes();
                send_to_char(g, chid, &line);
            }
        }
    }
    if count > 0 {
        let mut out = b"\r\n".to_vec();
        out.extend_from_slice(comm::cc(g, chid, comm::C_NRM, comm::KGRN));
        out.extend_from_slice(b"Seeking Members");
        out.extend_from_slice(comm::cc(g, chid, comm::C_NRM, comm::KNRM));
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(comm::cc(g, chid, comm::C_NRM, comm::KRED));
        out.extend_from_slice(b"Closed");
        out.extend_from_slice(comm::cc(g, chid, comm::C_NRM, comm::KNRM));
        out.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &out);
    } else {
        send_to_char(g, chid, b"\r\nCurrently no groups formed.\r\n");
    }
}

/// do_group — Vatiken's Group System: Version 1.1.
pub fn do_group(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::game::{GROUP_ANON, GROUP_OPEN};
    use crate::handler::{create_group, is_abbrev, join_group, leave_group};
    let (buf, rest) = crate::interpreter::one_argument(argument);

    if buf.is_empty() {
        if g.ch(chid).group.is_some() {
            print_group(g, chid);
        } else {
            send_to_char(g, chid, b"You must specify a group option, or type HELP GROUP for more info.\r\n");
        }
        return;
    }

    if is_abbrev(&buf, b"new") {
        if g.ch(chid).group.is_some() {
            send_to_char(g, chid, b"You are already in a group.\r\n");
        } else {
            create_group(g, chid);
        }
    } else if is_abbrev(&buf, b"list") {
        display_group_list(g, chid);
    } else if is_abbrev(&buf, b"join") {
        let name = skip_spaces(rest);
        let Some(vict) = crate::handler::get_char_room_vis(g, chid, name, None) else {
            send_to_char(g, chid, b"Join who?\r\n");
            return;
        };
        if vict == chid {
            send_to_char(g, chid, b"That would be one lonely grouping.\r\n");
            return;
        }
        if g.ch(chid).group.is_some() {
            send_to_char(g, chid, b"But you are already part of a group.\r\n");
            return;
        }
        let Some(vgid) = g.ch(vict).group else {
            act(g, b"$E$u is not part of a group!", false, Some(chid), None, Some(vict), comm::TO_CHAR);
            return;
        };
        if g.group(vgid).is_some_and(|gr| gr.group_flags & GROUP_OPEN == 0) {
            send_to_char(g, chid, b"That group isn't accepting members.\r\n");
            return;
        }
        join_group(g, chid, vgid);
    } else if is_abbrev(&buf, b"kick") {
        let name = skip_spaces(rest);
        let Some(vict) = crate::handler::get_char_room_vis(g, chid, name, None) else {
            send_to_char(g, chid, b"Kick out who?\r\n");
            return;
        };
        if vict == chid {
            send_to_char(g, chid, b"There are easier ways to leave the group.\r\n");
            return;
        }
        let Some(gid) = g.ch(chid).group else {
            send_to_char(g, chid, b"But you are not part of a group.\r\n");
            return;
        };
        if g.group(gid).is_some_and(|gr| gr.leader != Some(chid)) {
            send_to_char(g, chid, b"Only the group's leader can kick members out.\r\n");
            return;
        }
        if g.ch(vict).group != Some(gid) {
            act(g, b"$E$u is not a member of your group!", false, Some(chid), None, Some(vict), comm::TO_CHAR);
            return;
        }
        let mut msg = b"You have kicked ".to_vec();
        msg.extend_from_slice(g.ch(vict).get_name());
        msg.extend_from_slice(b" out of the group.\r\n");
        send_to_char(g, chid, &msg);
        send_to_char(g, vict, b"You have been kicked out of the group.\r\n");
        leave_group(g, vict);
    } else if is_abbrev(&buf, b"regroup") {
        let Some(gid) = g.ch(chid).group else {
            send_to_char(g, chid, b"But you aren't part of a group!\r\n");
            return;
        };
        let vict = g.group(gid).and_then(|gr| gr.leader);
        if vict == Some(chid) {
            send_to_char(g, chid, b"You are the group leader and cannot re-group.\r\n");
        } else {
            leave_group(g, chid);
            if let Some(vgid) = vict.and_then(|v| g.try_ch(v)).and_then(|c| c.group) {
                join_group(g, chid, vgid);
            }
        }
    } else if is_abbrev(&buf, b"leave") {
        if g.ch(chid).group.is_none() {
            send_to_char(g, chid, b"But you aren't part of a group!\r\n");
            return;
        }
        leave_group(g, chid);
    } else if is_abbrev(&buf, b"option") {
        let opt = skip_spaces(rest);
        let Some(gid) = g.ch(chid).group else {
            send_to_char(g, chid, b"But you aren't part of a group!\r\n");
            return;
        };
        if g.group(gid).is_some_and(|gr| gr.leader != Some(chid)) {
            send_to_char(g, chid, b"Only the group leader can adjust the group flags.\r\n");
            return;
        }
        if is_abbrev(opt, b"open") {
            let now_open = {
                let gr = g.group_mut(gid).unwrap();
                gr.group_flags ^= GROUP_OPEN;
                gr.group_flags & GROUP_OPEN != 0
            };
            let msg = format!(
                "The group is now {} to new members.\r\n",
                if now_open { "open" } else { "closed" }
            );
            send_to_char(g, chid, msg.as_bytes());
        } else if is_abbrev(opt, b"anonymous") {
            let now_anon = {
                let gr = g.group_mut(gid).unwrap();
                gr.group_flags ^= GROUP_ANON;
                gr.group_flags & GROUP_ANON != 0
            };
            let msg = format!(
                "The group location is now {} to other players.\r\n",
                if now_anon { "invisible" } else { "visible" }
            );
            send_to_char(g, chid, msg.as_bytes());
        } else {
            send_to_char(g, chid, b"The flag options are: Open, Anonymous\r\n");
        }
    } else {
        send_to_char(g, chid, b"You must specify a group option, or type HELP GROUP for more info.\r\n");
    }
}

pub fn do_report(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(gid) = g.ch(chid).group else {
        send_to_char(g, chid, b"But you are not a member of any group!\r\n");
        return;
    };
    let name = g.ch(chid).get_name().to_vec();
    let p = g.ch(chid).points;
    let mut body = name;
    body.extend_from_slice(
        format!(
            " reports: {}/{}H, {}/{}M, {}/{}V\r\n",
            p.hit, p.max_hit, p.mana, p.max_mana, p.mov, p.max_move
        )
        .as_bytes(),
    );
    crate::comm::send_to_group(g, None, gid, &body);
}

pub fn do_split(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::limits::{decrease_gold, increase_gold};
    if g.ch(chid).is_npc() {
        return;
    }
    let (buf, _) = crate::interpreter::one_argument(argument);

    if crate::interpreter::is_number(&buf) {
        let amount = atoi(&buf);
        if amount <= 0 {
            send_to_char(g, chid, b"Sorry, you can't do that.\r\n");
            return;
        }
        if amount > g.ch(chid).points.gold {
            send_to_char(g, chid, b"You don't seem to have that much gold to split.\r\n");
            return;
        }

        let room = g.ch(chid).in_room;
        let mut num = 0i32;
        if let Some(gr) = g.group_of(chid) {
            let members = gr.members.clone();
            for k in members {
                if g.try_ch(k).is_some_and(|c| c.in_room == room && !c.is_npc()) {
                    num += 1;
                }
            }
        }

        let (share, rest) = if num > 0 && g.ch(chid).group.is_some() {
            (amount / num, amount % num)
        } else {
            send_to_char(g, chid, b"With whom do you wish to share your gold?\r\n");
            return;
        };

        decrease_gold(g, chid, share * (num - 1));

        let name = g.ch(chid).get_name().to_vec();
        let mut buf_msg = name.clone();
        buf_msg.extend_from_slice(format!(" splits {} coins; you receive {}.\r\n", amount, share).as_bytes());
        if rest > 0 {
            buf_msg.extend_from_slice(
                format!(
                    "{} coin{} {} not splitable, so ",
                    rest,
                    if rest == 1 { "" } else { "s" },
                    if rest == 1 { "was" } else { "were" }
                )
                .as_bytes(),
            );
            buf_msg.extend_from_slice(&name);
            buf_msg.extend_from_slice(b" keeps the money.\r\n");
        }

        if let Some(gr) = g.group_of(chid) {
            let members = gr.members.clone();
            for k in members {
                if k != chid && g.try_ch(k).is_some_and(|c| c.in_room == room && !c.is_npc()) {
                    increase_gold(g, k, share);
                    send_to_char(g, k, &buf_msg);
                }
            }
        }

        let msg = format!(
            "You split {} coins among {} members -- {} coins each.\r\n",
            amount, num, share
        );
        send_to_char(g, chid, msg.as_bytes());
        if rest > 0 {
            let msg = format!(
                "{} coin{} {} not splitable, so you keep the money.\r\n",
                rest,
                if rest == 1 { "" } else { "s" },
                if rest == 1 { "was" } else { "were" }
            );
            send_to_char(g, chid, msg.as_bytes());
        }
    } else {
        send_to_char(g, chid, b"How many coins do you wish to split with your group?\r\n");
    }
}

pub fn do_title(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let mut argument = skip_spaces(argument).to_vec();
    delete_doubledollar(&mut argument);
    parse_at(&mut argument);

    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"Your title is fine... go away.\r\n");
    } else if g.ch(chid).plr(flags::PLR_NOTITLE) {
        send_to_char(g, chid, b"You can't title yourself -- you shouldn't have abused it!\r\n");
    } else if argument.contains(&b'(') || argument.contains(&b')') {
        send_to_char(g, chid, b"Titles can't contain the ( or ) characters.\r\n");
    } else if argument.len() > MAX_TITLE_LENGTH {
        let msg = format!("Sorry, titles can't be longer than {} characters.\r\n", MAX_TITLE_LENGTH);
        send_to_char(g, chid, msg.as_bytes());
    } else {
        g.ch_mut(chid).title = Some(argument.clone());
        let name = g.ch(chid).name.clone().unwrap_or_default();
        let mut msg = b"Okay, you're now ".to_vec();
        msg.extend_from_slice(&name);
        if !argument.is_empty() {
            msg.push(b' ');
        }
        msg.extend_from_slice(&argument);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
    }
}

/// PRF_TOG_CHK helper: toggle, return new state.
fn prf_tog_chk(g: &mut Game, chid: CharId, flag: usize) -> bool {
    let ps = g.ch_mut(chid).ps_mut();
    ps.pref.toggle(flag);
    ps.pref.is_set(flag)
}

pub fn do_gen_tog(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::*;
    if g.ch(chid).is_npc() {
        return;
    }
    // (off_msg, on_msg) per SCMD,.
    let pair: Option<(usize, &[u8], &[u8])> = match subcmd {
        SCMD_NOSUMMON => Some((
            flags::PRF_SUMMONABLE,
            b"You are now safe from summoning by other players.\r\n",
            b"You may now be summoned by other players.\r\n",
        )),
        SCMD_NOHASSLE => Some((flags::PRF_NOHASSLE, b"Nohassle disabled.\r\n", b"Nohassle enabled.\r\n")),
        SCMD_BRIEF => Some((flags::PRF_BRIEF, b"Brief mode off.\r\n", b"Brief mode on.\r\n")),
        SCMD_COMPACT => Some((flags::PRF_COMPACT, b"Compact mode off.\r\n", b"Compact mode on.\r\n")),
        SCMD_NOTELL => Some((flags::PRF_NOTELL, b"You can now hear tells.\r\n", b"You are now deaf to tells.\r\n")),
        SCMD_NOAUCTION => Some((flags::PRF_NOAUCT, b"You can now hear auctions.\r\n", b"You are now deaf to auctions.\r\n")),
        SCMD_NOSHOUT => Some((flags::PRF_NOSHOUT, b"You can now hear shouts.\r\n", b"You are now deaf to shouts.\r\n")),
        SCMD_NOGOSSIP => Some((flags::PRF_NOGOSS, b"You can now hear gossip.\r\n", b"You are now deaf to gossip.\r\n")),
        SCMD_NOGRATZ => Some((
            flags::PRF_NOGRATZ,
            b"You can now hear the congratulation messages.\r\n",
            b"You are now deaf to the congratulation messages.\r\n",
        )),
        SCMD_NOWIZ => Some((
            flags::PRF_NOWIZ,
            b"You can now hear the Wiz-channel.\r\n",
            b"You are now deaf to the Wiz-channel.\r\n",
        )),
        SCMD_QUEST => Some((
            flags::PRF_QUEST,
            b"You are no longer part of the Quest.\r\n",
            b"Okay, you are part of the Quest!\r\n",
        )),
        SCMD_SHOWVNUMS => Some((
            flags::PRF_SHOWVNUMS,
            b"You will no longer see the room flags.\r\n",
            b"You will now see the room flags.\r\n",
        )),
        SCMD_NOREPEAT => Some((
            flags::PRF_NOREPEAT,
            b"You will now have your communication repeated.\r\n",
            b"You will no longer have your communication repeated.\r\n",
        )),
        SCMD_HOLYLIGHT => Some((flags::PRF_HOLYLIGHT, b"HolyLight mode off.\r\n", b"HolyLight mode on.\r\n")),
        SCMD_SLOWNS => {
            g.config.nameserver_is_slow = !g.config.nameserver_is_slow;
            let msg: &[u8] = if g.config.nameserver_is_slow {
                b"Nameserver_is_slow changed to YES; sitenames will no longer be resolved.\r\n"
            } else {
                b"Nameserver_is_slow changed to NO; IP addresses will now be resolved.\r\n"
            };
            send_to_char(g, chid, msg);
            return;
        }
        SCMD_AUTOEXIT => Some((flags::PRF_AUTOEXIT, b"Autoexits disabled.\r\n", b"Autoexits enabled.\r\n")),
        SCMD_TRACK => {
            g.config.track_through_doors = !g.config.track_through_doors;
            let msg: &[u8] = if g.config.track_through_doors {
                b"Will now track through doors.\r\n"
            } else {
                b"Will no longer track through doors.\r\n"
            };
            send_to_char(g, chid, msg);
            return;
        }
        SCMD_CLS => Some((
            flags::PRF_CLS,
            b"Will no longer clear screen in OLC.\r\n",
            b"Will now clear screen in OLC.\r\n",
        )),
        SCMD_BUILDWALK => {
            if g.ch(chid).level < LVL_BUILDER {
                send_to_char(g, chid, b"Builders only, sorry.\r\n");
                return;
            }
            let on = prf_tog_chk(g, chid, flags::PRF_BUILDWALK);
            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let level = g.ch(chid).level;
            let allowed = g.ch(chid).ps().olc_zone;
            if on {
                // The sector argument: any abbreviation of a sector name,
                // and anything unrecognised (or absent) falls back to 0.
                let (arg, _) = crate::interpreter::one_argument(argument);
                let mut i = 0usize;
                if !arg.is_empty() {
                    let names = &mud_data::tables::SECTOR_TYPES[..flags::NUM_ROOM_SECTORS];
                    match names.iter().position(|s| is_abbrev(&arg, s.as_bytes())) {
                        Some(p) => i = p,
                        None => i = 0,
                    }
                }
                g.ch_mut(chid).ps_mut().buildwalk_sector = i as i32;
                let msg = format!(
                    "Default sector type is {}\r\n",
                    mud_data::tables::SECTOR_TYPES[i]
                );
                send_to_char(g, chid, msg.as_bytes());
                g.mudlog(
                    MudlogKind::Cmp,
                    level,
                    true,
                    &format!("OLC: {} turned buildwalk on. Allowed zone {}", name, allowed),
                );
                send_to_char(g, chid, b"Buildwalk On.\r\n");
            } else {
                g.mudlog(
                    MudlogKind::Cmp,
                    level,
                    true,
                    &format!("OLC: {} turned buildwalk off. Allowed zone {}", name, allowed),
                );
                send_to_char(g, chid, b"Buildwalk Off.\r\n");
            }
            return;
        }
        SCMD_AFK => {
            let on = prf_tog_chk(g, chid, flags::PRF_AFK);
            if on {
                send_to_char(g, chid, b"AFK flag is now on.\r\n");
                act(g, b"$n has gone AFK.", true, Some(chid), None, None, comm::TO_ROOM);
            } else {
                send_to_char(g, chid, b"AFK flag is now off.\r\n");
                act(g, b"$n has come back from AFK.", true, Some(chid), None, None, comm::TO_ROOM);
                // Mail notice: stage 7.
            }
            return;
        }
        SCMD_AUTOLOOT => Some((flags::PRF_AUTOLOOT, b"Autoloot disabled.\r\n", b"Autoloot enabled.\r\n")),
        SCMD_AUTOGOLD => Some((flags::PRF_AUTOGOLD, b"Autogold disabled.\r\n", b"Autogold enabled.\r\n")),
        SCMD_AUTOSPLIT => Some((flags::PRF_AUTOSPLIT, b"Autosplit disabled.\r\n", b"Autosplit enabled.\r\n")),
        SCMD_AUTOSAC => Some((flags::PRF_AUTOSAC, b"Autosacrifice disabled.\r\n", b"Autosacrifice enabled.\r\n")),
        SCMD_AUTOASSIST => Some((flags::PRF_AUTOASSIST, b"Autoassist disabled.\r\n", b"Autoassist enabled.\r\n")),
        SCMD_AUTOMAP => Some((flags::PRF_AUTOMAP, b"Automap disabled.\r\n", b"Automap enabled.\r\n")),
        SCMD_AUTOKEY => Some((flags::PRF_AUTOKEY, b"Autokey disabled.\r\n", b"Autokey enabled.\r\n")),
        SCMD_AUTODOOR => Some((flags::PRF_AUTODOOR, b"Autodoor disabled.\r\n", b"Autodoor enabled.\r\n")),
        SCMD_ZONERESETS => Some((flags::PRF_ZONERESETS, b"ZoneResets disabled.\r\n", b"ZoneResets enabled.\r\n")),
        _ => {
            g.log("SYSERR: Unknown subcmd in do_gen_toggle.".to_string());
            return;
        }
    };
    if let Some((flag, off_msg, on_msg)) = pair {
        let on = prf_tog_chk(g, chid, flag);
        send_to_char(g, chid, if on { on_msg } else { off_msg });
    }
}

/// toggle wimpy — a special case of the toggle listing; no standalone
/// command.
pub fn gen_tog_wimpy(g: &mut Game, chid: CharId, value: &[u8]) {
    if value.is_empty() {
        let wimp = g.ch(chid).ps().wimp_level;
        if wimp != 0 {
            let msg = format!("Your current wimp level is {} hit points.\r\n", wimp);
            send_to_char(g, chid, msg.as_bytes());
        } else {
            send_to_char(g, chid, b"At the moment, you're not a wimp.  (sure, sure...)\r\n");
        }
        return;
    }
    if value.first().is_some_and(|c| c.is_ascii_digit()) || value.first() == Some(&b'-') {
        let wimp = atoi(value);
        if wimp == 0 {
            send_to_char(g, chid, b"Okay, you'll now tough out fights to the bitter end.");
            g.ch_mut(chid).ps_mut().wimp_level = 0;
        } else if wimp < 0 {
            send_to_char(g, chid, b"Heh, heh, heh.. we are jolly funny today, eh?\r\n");
        } else if wimp > g.ch(chid).points.max_hit {
            send_to_char(g, chid, b"That doesn't make much sense, now does it?\r\n");
        } else if wimp > g.ch(chid).points.max_hit / 2 {
            send_to_char(g, chid, b"You can't set your wimp level above half your hit points.\r\n");
        } else {
            let msg = format!("Okay, you'll wimp out if you drop below {} hit points.", wimp);
            send_to_char(g, chid, msg.as_bytes());
            g.ch_mut(chid).ps_mut().wimp_level = wimp;
        }
    } else {
        send_to_char(
            g,
            chid,
            b"Specify at how many hit points you want to wimp out at.  (0 to disable)\r\n",
        );
    }
}

/// toggle pagelength.
pub fn gen_tog_pagelength(g: &mut Game, chid: CharId, value: &[u8]) {
    if value.first().is_some_and(|c| c.is_ascii_digit()) {
        let n = atoi(value);
        if (5..=255).contains(&n) {
            g.ch_mut(chid).ps_mut().page_length = n;
            let msg = format!("Okay, your page length is now set to {} lines.", n);
            send_to_char(g, chid, msg.as_bytes());
        } else {
            send_to_char(g, chid, b"Please specify a number of lines (5 - 255).");
        }
    } else {
        let n = g.ch(chid).ps().page_length;
        let msg = format!("Your current page length is set to {} lines.", n);
        send_to_char(g, chid, msg.as_bytes());
    }
}

/// toggle screenwidth.
pub fn gen_tog_screenwidth(g: &mut Game, chid: CharId, value: &[u8]) {
    if value.first().is_some_and(|c| c.is_ascii_digit()) {
        let n = atoi(value);
        if (40..=200).contains(&n) {
            g.ch_mut(chid).ps_mut().screen_width = n;
            let msg = format!("Okay, your screen width is now set to {} characters.", n);
            send_to_char(g, chid, msg.as_bytes());
        } else {
            send_to_char(g, chid, b"Please specify a number of characters (40 - 200).");
        }
    } else {
        let n = g.ch(chid).ps().screen_width;
        let msg = format!("Your current screen width is set to {} characters.", n);
        send_to_char(g, chid, msg.as_bytes());
    }
}

/// do_display — the `prompt`/`display` command.
pub fn do_display(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"Monsters don't need displays.  Go away.\r\n");
        return;
    }
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Usage: prompt { { H | M | V } | all | auto | none }\r\n");
        return;
    }
    if argument.eq_ignore_ascii_case(b"auto") {
        let on = prf_tog_chk(g, chid, flags::PRF_DISPAUTO);
        let msg = format!("Auto prompt {}abled.\r\n", if on { "en" } else { "dis" });
        send_to_char(g, chid, msg.as_bytes());
        return;
    }
    if argument.eq_ignore_ascii_case(b"on") || argument.eq_ignore_ascii_case(b"all") {
        let ps = g.ch_mut(chid).ps_mut();
        ps.pref.set(flags::PRF_DISPHP);
        ps.pref.set(flags::PRF_DISPMANA);
        ps.pref.set(flags::PRF_DISPMOVE);
    } else if argument.eq_ignore_ascii_case(b"off") || argument.eq_ignore_ascii_case(b"none") {
        let ps = g.ch_mut(chid).ps_mut();
        ps.pref.remove(flags::PRF_DISPHP);
        ps.pref.remove(flags::PRF_DISPMANA);
        ps.pref.remove(flags::PRF_DISPMOVE);
    } else {
        {
            let ps = g.ch_mut(chid).ps_mut();
            ps.pref.remove(flags::PRF_DISPHP);
            ps.pref.remove(flags::PRF_DISPMANA);
            ps.pref.remove(flags::PRF_DISPMOVE);
        }
        for c in argument.iter().map(|c| c.to_ascii_lowercase()) {
            match c {
                b'h' => g.ch_mut(chid).ps_mut().pref.set(flags::PRF_DISPHP),
                b'm' => g.ch_mut(chid).ps_mut().pref.set(flags::PRF_DISPMANA),
                b'v' => g.ch_mut(chid).ps_mut().pref.set(flags::PRF_DISPMOVE),
                _ => {
                    send_to_char(g, chid, b"Usage: prompt { { H | M | V } | all | auto | none }\r\n");
                    return;
                }
            }
        }
    }
    let ok = g.config.ok.clone();
    send_to_char(g, chid, &ok);
}

pub fn do_alias(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let (arg, repl_raw) = any_one_arg(argument);
    if arg.is_empty() {
        let mut out = b"Currently defined aliases:\r\n".to_vec();
        let aliases = g.ch(chid).ps().aliases.clone();
        if aliases.is_empty() {
            out.extend_from_slice(b" None.\r\n");
        } else {
            for a in &aliases {
                // "%-15s %s" + CRLF — the replacement keeps its own leading
                // space, so two spaces separate the columns.
                out.extend_from_slice(&crate::act::pad_right(&a.alias, 15));
                out.push(b' ');
                out.extend_from_slice(&a.replacement);
                out.extend_from_slice(b"\r\n");
            }
        }
        send_to_char(g, chid, &out);
        return;
    }
    // Remove any old alias with this name.
    let existed = {
        let ps = g.ch_mut(chid).ps_mut();
        let before = ps.aliases.len();
        ps.aliases.retain(|a| a.alias != arg);
        ps.aliases.len() != before
    };
    // repl_raw points just past the first word — leading space preserved.
    if skip_spaces(repl_raw).is_empty() {
        if existed {
            send_to_char(g, chid, b"Alias deleted.\r\n");
        } else {
            send_to_char(g, chid, b"No such alias.\r\n");
        }
        return;
    }
    if arg == b"alias" {
        send_to_char(g, chid, b"You can't alias 'alias'.\r\n");
        return;
    }
    let mut repl = repl_raw.to_vec();
    delete_doubledollar(&mut repl);
    let type_ = if repl.contains(&b';') || repl.contains(&b'$') { ALIAS_COMPLEX } else { ALIAS_SIMPLE };
    g.ch_mut(chid).ps_mut().aliases.insert(0, Alias { alias: arg.clone(), replacement: repl, type_ });
    crate::players_glue::save_char(g, chid);
    send_to_char(g, chid, b"Alias ready.\r\n");
}

/// do_echo — emote and imm echo.
pub fn do_echo(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_EMOTE;
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Yes.. but what?\r\n");
        return;
    }
    let buf: BStr = if subcmd == SCMD_EMOTE {
        let mut b = b"$n ".to_vec();
        b.extend_from_slice(argument);
        b
    } else {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let invis = g.ch(chid).invis_lev();
        g.mudlog(
            MudlogKind::Cmp,
            (LVL_BUILDER as i16).max(invis) as u8,
            true,
            &format!("(GC) {} echoed: {}", name, String::from_utf8_lossy(argument)),
        );
        argument.to_vec()
    };
    act(g, &buf, false, Some(chid), None, None, comm::TO_ROOM);
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        act(g, &buf, false, Some(chid), None, None, comm::TO_CHAR);
    }
}

/// do_practice moved to informative (list side); alias for the table.
pub use crate::act::informative::do_practice;

// ---------------------------------------------------------------------------
// Happy hour
// ---------------------------------------------------------------------------

pub fn is_happyhour(g: &Game) -> bool {
    let h = &g.happy;
    (h.exp_rate > 0 || h.gold_rate > 0 || h.qp_rate > 0) && h.ticks_left > 0
}

fn show_happyhour(g: &mut Game, chid: CharId) {
    use crate::comm::{cc, C_NRM, KNRM, KYEL};
    let level = g.ch(chid).level;
    if !is_happyhour(g) && level < LVL_GRGOD {
        send_to_char(g, chid, b"Sorry, there is currently no happy hour!\r\n");
        return;
    }
    let secs_left = if g.happy.ticks_left != 0 {
        (g.happy.ticks_left - 1) * SECS_PER_MUD_HOUR as i32 + g.next_tick
    } else {
        0
    };
    let (yel, nrm) = (cc(g, chid, C_NRM, KYEL).to_vec(), cc(g, chid, C_NRM, KNRM).to_vec());
    let rate_line = |v: &mut Vec<u8>, rate: i32, tail: &[u8]| {
        v.extend_from_slice(&yel);
        v.extend_from_slice(format!("+{}%", rate).as_bytes());
        v.extend_from_slice(&nrm);
        v.extend_from_slice(tail);
    };
    let mut out = b"tbaMUD Happy Hour!\r\n------------------\r\n".to_vec();
    if g.happy.exp_rate > 0 || level >= LVL_GOD {
        rate_line(&mut out, g.happy.exp_rate, b" to Experience per kill\r\n");
    }
    if g.happy.gold_rate > 0 || level >= LVL_GOD {
        rate_line(&mut out, g.happy.gold_rate, b" to Gold gained per kill\r\n");
    }
    if g.happy.qp_rate > 0 || level >= LVL_GOD {
        rate_line(&mut out, g.happy.qp_rate, b" to Questpoints per quest\r\n");
    }
    out.extend_from_slice(b"Time Remaining: ");
    for (val, label) in [
        (secs_left / 3600, &b" hours "[..]),
        ((secs_left % 3600) / 60, &b" mins "[..]),
        (secs_left % 60, &b" secs"[..]),
    ] {
        out.extend_from_slice(&yel);
        out.extend_from_slice(val.to_string().as_bytes());
        out.extend_from_slice(&nrm);
        out.extend_from_slice(label);
    }
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);
}

pub fn do_happyhour(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    use crate::comm::{cc, C_NRM, KNRM, KYEL};
    if g.ch(chid).level < LVL_GOD {
        show_happyhour(g, chid);
        return;
    }
    let (arg, val, _) = crate::interpreter::two_arguments(argument);
    let num = crate::handler::atoi(&val).clamp(0, 1000);

    if is_abbrev(&arg, b"experience") {
        g.happy.exp_rate = num;
        let m = format!("Happy Hour Exp rate set to +{}%\r\n", num);
        send_to_char(g, chid, m.as_bytes());
    } else if is_abbrev(&arg, b"gold") || is_abbrev(&arg, b"coins") {
        g.happy.gold_rate = num;
        let m = format!("Happy Hour Gold rate set to +{}%\r\n", num);
        send_to_char(g, chid, m.as_bytes());
    } else if is_abbrev(&arg, b"time") || is_abbrev(&arg, b"ticks") {
        if g.happy.ticks_left != 0 && num == 0 {
            crate::comm::game_info(g, b"Happyhour has been stopped!");
        } else if g.happy.ticks_left == 0 && num != 0 {
            crate::comm::game_info(g, b"A Happyhour has started!");
        }
        g.happy.ticks_left = num;
        let secs = num * SECS_PER_MUD_HOUR as i32;
        let m = format!(
            "Happy Hour Time set to {} ticks ({} hours {} mins and {} secs)\r\n",
            num,
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        );
        send_to_char(g, chid, m.as_bytes());
    } else if is_abbrev(&arg, b"qp") || is_abbrev(&arg, b"questpoints") {
        g.happy.qp_rate = num;
        let m = format!("Happy Hour Questpoints rate set to +{}%\r\n", num);
        send_to_char(g, chid, m.as_bytes());
    } else if is_abbrev(&arg, b"show") {
        show_happyhour(g, chid);
    } else if is_abbrev(&arg, b"default") {
        g.happy.exp_rate = 100;
        g.happy.gold_rate = 50;
        g.happy.qp_rate = 50;
        g.happy.ticks_left = 48;
        crate::comm::game_info(g, b"A Happyhour has started!");
    } else {
        let (yel, nrm) = (cc(g, chid, C_NRM, KYEL).to_vec(), cc(g, chid, C_NRM, KNRM).to_vec());
        let mut out = Vec::new();
        let mut row = |lead: &[u8], body: &[u8], comment: &[u8]| {
            out.extend_from_slice(lead);
            out.extend_from_slice(&yel);
            out.extend_from_slice(body);
            out.extend_from_slice(&nrm);
            out.extend_from_slice(comment);
            out.extend_from_slice(b"\r\n");
        };
        row(b"Usage: ", b"happyhour              ", b"- show usage (this info)");
        row(b"       ", b"happyhour show         ", b"- display current settings (what mortals see)");
        row(b"       ", b"happyhour time <ticks> ", b"- set happyhour time and start timer");
        row(b"       ", b"happyhour qp <num>     ", b"- set qp percentage gain");
        row(b"       ", b"happyhour exp <num>    ", b"- set exp percentage gain");
        row(b"       ", b"happyhour gold <num>   ", b"- set gold percentage gain");
        out.extend_from_slice(
            b"       \tyhappyhour default      \tw- sets a default setting for happyhour\r\n\r\n",
        );
        out.extend_from_slice(b"Configure the happyhour settings and start a happyhour.\r\n");
        out.extend_from_slice(
            format!("Currently 1 hour IRL = {} ticks\r\n", 3600 / SECS_PER_MUD_HOUR).as_bytes(),
        );
        out.extend_from_slice(
            b"If no number is specified, 0 (off) is assumed.\r\nThe command \tyhappyhour time\tn will therefore stop the happyhour timer.\r\n",
        );
        send_to_char(g, chid, &out);
    }
}
