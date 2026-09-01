//! say, gsay, tell/reply, whisper/ask, page, the gen_comm channels, qsay
//! and emote, plus channel history recording.

use mud_data::flags::{self};
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::act::informative::{add_history, HIST_AUCTION, HIST_GOSSIP, HIST_GRATS, HIST_HOLLER, HIST_SAY, HIST_SHOUT, HIST_TELL};
use crate::act::BStr;
use crate::comm::{self, act, act_full, cc, send_to_char, ActArg, C_CMP, C_NRM, KGRN, KMAG, KNRM, KRED, KYEL};
use crate::game::{Game, MudlogKind};
use crate::handler::{get_char_room_vis, get_char_world_vis, get_player_vis, pers};
use crate::interpreter::{half_chop, skip_spaces};
use crate::text::parse_at;

/// legal_communication: '@' must not forge MXP tags.
fn legal_communication(arg: &[u8]) -> bool {
    let mut i = 0;
    while i < arg.len() {
        if arg[i] == b'@' {
            match arg.get(i + 1) {
                Some(&b'(') | Some(&b')') | Some(&b'<') | Some(&b'>') => return false,
                _ => {}
            }
        }
        i += 1;
    }
    true
}

fn room_soundproof(g: &Game, chid: CharId) -> bool {
    let room = g.ch(chid).in_room;
    room != NOWHERE
        && g.world.rooms[room as usize].room_flags[flags::ROOM_SOUNDPROOF / 32]
            & (1 << (flags::ROOM_SOUNDPROOF % 32))
            != 0
}

pub fn do_say(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Yes, but WHAT do you want to say?\r\n");
        // The trigger check sits outside the else, so an empty say still
        // runs the speech triggers (only '*' arglists can match "").
        crate::dg::triggers::speech_mtrigger(g, chid, b"");
        crate::dg::triggers::speech_wtrigger(g, chid, b"");
        return;
    }
    let mut speech = argument.to_vec();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }
    let mut msg = b"$n\tn says, '".to_vec();
    msg.extend_from_slice(&speech);
    msg.push(b'\'');
    // Room echo, recorded per awake receiver.
    let room = g.ch(chid).in_room;
    let people = g.rooms[room as usize].people.clone();
    for to in people {
        if to == chid {
            continue;
        }
        let Some(t) = g.try_ch(to) else { continue };
        if t.desc.is_none() && !t.is_npc() {
            continue;
        }
        if !t.awake() {
            continue;
        }
        if let Some(rendered) =
            act_full(g, &msg, true, Some(chid), None, ActArg::Char(to), comm::TO_VICT | comm::DG_NO_TRIG)
        {
            add_history(g, to, &rendered, HIST_SAY);
        }
    }
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        let mut mymsg = b"You say, '".to_vec();
        mymsg.extend_from_slice(&speech);
        mymsg.push(b'\'');
        if let Some(rendered) = act_full(g, &mymsg, false, Some(chid), None, ActArg::None, comm::TO_CHAR | comm::DG_NO_TRIG)
        {
            add_history(g, chid, &rendered, HIST_SAY);
        }
    }
    // Trigger check, after the say text is shown. parse_at has already
    // run, so triggers see the parsed text.
    crate::dg::triggers::speech_mtrigger(g, chid, &speech);
    crate::dg::triggers::speech_wtrigger(g, chid, &speech);
}

/// do_gsay. The green wrapping is keyed to the SPEAKER's
/// colour level — CCGRN(ch, ..) is evaluated once, as a format argument —
/// and the body carries two leading CCGRN codes.
pub fn do_gsay(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let argument = skip_spaces(argument);
    let Some(gid) = g.ch(chid).group else {
        send_to_char(g, chid, b"But you are not a member of a group!\r\n");
        return;
    };
    if argument.is_empty() {
        send_to_char(g, chid, b"Yes, but WHAT do you want to group-say?\r\n");
        return;
    }
    let mut speech = argument.to_vec();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }
    let grn = cc(g, chid, C_NRM, KGRN);
    let nrm = cc(g, chid, C_NRM, KNRM);
    let mut body = grn.to_vec();
    body.extend_from_slice(grn);
    body.extend_from_slice(g.ch(chid).get_name());
    body.extend_from_slice(b" says, '");
    body.extend_from_slice(&speech);
    body.push(b'\'');
    body.extend_from_slice(nrm);
    body.extend_from_slice(b"\r\n");
    comm::send_to_group(g, Some(chid), gid, &body);

    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        let mut mymsg = grn.to_vec();
        mymsg.extend_from_slice(b"You group-say, '");
        mymsg.extend_from_slice(&speech);
        mymsg.push(b'\'');
        mymsg.extend_from_slice(nrm);
        mymsg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &mymsg);
    }
}

fn is_tell_ok(g: &mut Game, chid: CharId, vict: CharId) -> bool {
    if chid == vict {
        send_to_char(g, chid, b"You try to tell yourself something.\r\n");
    } else if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOTELL) {
        send_to_char(g, chid, b"You can't tell other people while you have notell on.\r\n");
    } else if g.ch(chid).level < LVL_GOD && room_soundproof(g, chid) {
        send_to_char(g, chid, b"The walls seem to absorb your words.\r\n");
    } else if !g.ch(vict).is_npc() && g.ch(vict).desc.is_none() {
        act(g, b"$E's linkless at the moment.", false, Some(chid), None, Some(vict), comm::TO_CHAR | comm::TO_SLEEP);
    } else if g.ch(vict).plr(flags::PLR_WRITING) {
        act(g, b"$E's writing a message right now; try again later.", false, Some(chid), None, Some(vict), comm::TO_CHAR | comm::TO_SLEEP);
    } else if (!g.ch(vict).is_npc() && g.ch(vict).prf(flags::PRF_NOTELL))
        || (g.ch(vict).level < LVL_GOD && {
            let room = g.ch(vict).in_room;
            room != NOWHERE
                && g.world.rooms[room as usize].room_flags[flags::ROOM_SOUNDPROOF / 32]
                    & (1 << (flags::ROOM_SOUNDPROOF % 32))
                    != 0
        })
    {
        act(g, b"$E can't hear you.", false, Some(chid), None, Some(vict), comm::TO_CHAR | comm::TO_SLEEP);
    } else {
        return true;
    }
    false
}

fn perform_tell(g: &mut Game, chid: CharId, vict: CharId, arg: &[u8]) {
    let red_vict = cc(g, vict, C_NRM, KRED).to_vec();
    let nrm_vict = cc(g, vict, C_NRM, KNRM).to_vec();
    let mut msg = red_vict.clone();
    msg.extend_from_slice(b"$n tells you, '");
    msg.extend_from_slice(arg);
    msg.push(b'\'');
    msg.extend_from_slice(&nrm_vict);
    if let Some(rendered) =
        act_full(g, &msg, false, Some(chid), None, ActArg::Char(vict), comm::TO_VICT | comm::TO_SLEEP)
    {
        add_history(g, vict, &rendered, HIST_TELL);
    }

    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        let red = cc(g, chid, C_NRM, KRED).to_vec();
        let nrm = cc(g, chid, C_NRM, KNRM).to_vec();
        let mut msg = red;
        msg.extend_from_slice(b"You tell $N, '");
        msg.extend_from_slice(arg);
        msg.push(b'\'');
        msg.extend_from_slice(&nrm);
        if let Some(rendered) =
            act_full(g, &msg, false, Some(chid), None, ActArg::Char(vict), comm::TO_CHAR | comm::TO_SLEEP)
        {
            add_history(g, chid, &rendered, HIST_TELL);
        }
    }
    if !g.ch(vict).is_npc() && !g.ch(chid).is_npc() {
        let id = g.ch(chid).idnum;
        g.ch_mut(vict).ps_mut().last_tell = id;
    }
}

pub fn do_tell(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, buf2) = half_chop(argument);
    if buf.is_empty() || buf2.is_empty() {
        send_to_char(g, chid, b"Who do you wish to tell what??\r\n");
        return;
    }
    let vict = if g.ch(chid).level < LVL_IMMORT {
        get_player_vis(g, chid, &buf, false)
    } else {
        get_char_world_vis(g, chid, &buf, None)
    };
    let Some(vict) = vict else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if !is_tell_ok(g, chid, vict) {
        return;
    }
    let mut speech = buf2.clone();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }
    perform_tell(g, chid, vict, &speech);
}

pub fn do_reply(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        return;
    }
    let argument = skip_spaces(argument);
    let last = g.ch(chid).ps().last_tell;
    if last == crate::ch::NOBODY_TELL {
        send_to_char(g, chid, b"You have nobody to reply to!\r\n");
        return;
    }
    if argument.is_empty() {
        send_to_char(g, chid, b"What is your reply?\r\n");
        return;
    }
    // Scan character_list for the idnum, skipping NPCs.
    let target = g
        .character_list
        .iter()
        .copied()
        .find(|c| g.try_ch(*c).is_some_and(|ch| !ch.is_npc() && ch.idnum == last));
    let Some(tch) = target else {
        send_to_char(g, chid, b"That player is no longer here.\r\n");
        return;
    };
    if is_tell_ok(g, chid, tch) {
        let mut speech = argument.to_vec();
        if g.config.special_in_comm && legal_communication(&speech) {
            parse_at(&mut speech);
        }
        perform_tell(g, chid, tch, &speech);
    }
}

/// do_spec_comm — whisper/ask.
pub fn do_spec_comm(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_WHISPER;
    let (action_sing, action_plur, action_others): (&[u8], &[u8], &[u8]) = if subcmd == SCMD_WHISPER {
        (b"whisper to", b"whispers to", b"$n whispers something to $N.")
    } else {
        (b"ask", b"asks", b"$n asks $N a question.")
    };
    let (buf, buf2) = half_chop(argument);
    if buf.is_empty() || buf2.is_empty() {
        let mut msg = b"Whom do you want to ".to_vec();
        msg.extend_from_slice(action_sing);
        msg.extend_from_slice(b".. and what??\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let Some(vict) = get_char_room_vis(g, chid, &buf, None) else {
        let msg = g.config.noperson.clone();
        send_to_char(g, chid, &msg);
        return;
    };
    if vict == chid {
        send_to_char(g, chid, b"You can't get your mouth close enough to your ear...\r\n");
        return;
    }
    let mut speech = buf2.clone();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }
    let mut msg = b"$n ".to_vec();
    msg.extend_from_slice(action_plur);
    msg.extend_from_slice(b" you, '");
    msg.extend_from_slice(&speech);
    msg.push(b'\'');
    act(g, &msg, false, Some(chid), None, Some(vict), comm::TO_VICT);
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        let mut msg = b"You ".to_vec();
        msg.extend_from_slice(action_sing);
        msg.push(b' ');
        msg.extend_from_slice(&pers(g, chid, vict));
        msg.extend_from_slice(b", '");
        msg.extend_from_slice(&speech);
        msg.extend_from_slice(b"'\r\n");
        send_to_char(g, chid, &msg);
    }
    act(g, action_others, false, Some(chid), None, Some(vict), comm::TO_NOTVICT);
}

pub fn do_page(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() {
        send_to_char(g, chid, b"Monsters can't page.. go away.\r\n");
        return;
    }
    let (arg, arg2) = half_chop(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Whom do you wish to page?\r\n");
        return;
    }
    let mut buf = b"\x07\x07*$n* ".to_vec();
    buf.extend_from_slice(&arg2);
    if arg == b"all" {
        if g.ch(chid).level > LVL_GOD {
            for di in g.descriptors.indices() {
                let Some(d) = g.descriptors.get(di) else { continue };
                if d.state != ConState::Playing {
                    continue;
                }
                let Some(to) = d.character else { continue };
                act(g, &buf, false, Some(chid), None, Some(to), comm::TO_VICT);
            }
        } else {
            send_to_char(g, chid, b"You will never be godly enough to do that!\r\n");
        }
        return;
    }
    if let Some(vict) = get_char_world_vis(g, chid, &arg, None) {
        act(g, &buf, false, Some(chid), None, Some(vict), comm::TO_VICT);
        if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
            let ok = g.config.ok.clone();
            send_to_char(g, chid, &ok);
        } else {
            act(g, &buf, false, Some(chid), None, Some(vict), comm::TO_CHAR);
        }
    } else {
        send_to_char(g, chid, b"There is no such person in the game!\r\n");
    }
}

/// do_gen_comm — channels.
pub fn do_gen_comm(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, subcmd: i32) {
    use crate::interpreter::{SCMD_AUCTION, SCMD_GEMOTE, SCMD_GOSSIP, SCMD_GRATZ, SCMD_HOLLER, SCMD_SHOUT};

    // com_msgs[subcmd]: [mute-punished msg, name, muted-on-channel msg, color].
    let (mute_msg, com_name, muted_msg, color): (&[u8], &[u8], &[u8], &'static [u8]) = match subcmd {
        SCMD_HOLLER => (b"You cannot holler!!\r\n", b"holler", b"", KYEL),
        SCMD_SHOUT => (b"You cannot shout!!\r\n", b"shout", b"Turn off your noshout flag first!\r\n", KYEL),
        SCMD_GOSSIP => (b"You cannot gossip!!\r\n", b"gossip", b"You aren't even on the channel!\r\n", KYEL),
        SCMD_AUCTION => (b"You cannot auction!!\r\n", b"auction", b"You aren't even on the channel!\r\n", KMAG),
        SCMD_GRATZ => (b"You cannot congratulate!\r\n", b"congrat", b"You aren't even on the channel!\r\n", KGRN),
        SCMD_GEMOTE => (b"You cannot gossip your emotions!\r\n", b"gemote", b"", KYEL),
        _ => return,
    };
    let channel_flag = match subcmd {
        SCMD_SHOUT => Some(flags::PRF_NOSHOUT),
        SCMD_GOSSIP | SCMD_GEMOTE => Some(flags::PRF_NOGOSS),
        SCMD_AUCTION => Some(flags::PRF_NOAUCT),
        SCMD_GRATZ => Some(flags::PRF_NOGRATZ),
        _ => None,
    };
    let hist_type = match subcmd {
        SCMD_HOLLER => HIST_HOLLER,
        SCMD_SHOUT => HIST_SHOUT,
        SCMD_GOSSIP | SCMD_GEMOTE => HIST_GOSSIP,
        SCMD_AUCTION => HIST_AUCTION,
        SCMD_GRATZ => HIST_GRATS,
        _ => HIST_GOSSIP,
    };

    if g.ch(chid).plr(flags::PLR_NOSHOUT) {
        send_to_char(g, chid, mute_msg);
        return;
    }
    if room_soundproof(g, chid) && g.ch(chid).level < LVL_GOD {
        send_to_char(g, chid, b"The walls seem to absorb your words.\r\n");
        return;
    }
    if subcmd == SCMD_GEMOTE {
        do_gmote(g, chid, argument);
        return;
    }
    let level_can_shout = g.config.level_can_shout;
    if (g.ch(chid).level as i32) < level_can_shout {
        let mut msg = format!("You must be at least level {} before you can ", level_can_shout).into_bytes();
        msg.extend_from_slice(com_name);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    if let Some(flag) = channel_flag {
        if !g.ch(chid).is_npc() && g.ch(chid).prf(flag) {
            send_to_char(g, chid, muted_msg);
            return;
        }
    }
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        // Both %s are the channel name. (ERRATA: study 07 §8.6 says
        // "name twice"; it is the channel.)
        let cn = String::from_utf8_lossy(com_name).into_owned();
        let msg = format!("Yes, {}, fine, {} we must, but WHAT???\r\n", cn, cn);
        send_to_char(g, chid, msg.as_bytes());
        return;
    }
    if subcmd == SCMD_HOLLER {
        let cost = g.config.holler_move_cost;
        if g.ch(chid).points.mov < cost {
            send_to_char(g, chid, b"You're too exhausted to holler.\r\n");
            return;
        }
        g.ch_mut(chid).points.mov -= cost;
    }
    let mut speech = argument.to_vec();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }

    // Sender echo.
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else {
        let color_on = if g.ch(chid).color_lev() >= C_CMP { color } else { b"" };
        let mut msg = color_on.to_vec();
        msg.extend_from_slice(b"You ");
        msg.extend_from_slice(com_name);
        msg.extend_from_slice(b", '");
        msg.extend_from_slice(&speech);
        msg.extend_from_slice(color_on);
        msg.push(b'\'');
        msg.extend_from_slice(cc(g, chid, C_CMP, KNRM));
        if let Some(rendered) = act_full(g, &msg, false, Some(chid), None, ActArg::None, comm::TO_CHAR | comm::TO_SLEEP) {
            add_history(g, chid, &rendered, hist_type);
        }
    }

    // Delivery.
    let my_zone = {
        let room = g.ch(chid).in_room;
        g.world.rooms[room as usize].zone
    };
    let sender_is_pc = !g.ch(chid).is_npc();
    let sender_level = g.ch(chid).level;
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(to) = d.character else { continue };
        if to == chid {
            continue;
        }
        let Some(t) = g.try_ch(to) else { continue };
        if sender_is_pc {
            if let Some(flag) = channel_flag {
                if t.prf(flag) {
                    continue;
                }
            }
            if t.plr(flags::PLR_WRITING) {
                continue;
            }
        }
        let t_room = t.in_room;
        if t_room != NOWHERE
            && g.world.rooms[t_room as usize].room_flags[flags::ROOM_SOUNDPROOF / 32]
                & (1 << (flags::ROOM_SOUNDPROOF % 32))
                != 0
            && sender_level < LVL_GOD
        {
            continue;
        }
        if subcmd == SCMD_SHOUT
            && ((t_room != NOWHERE && g.world.rooms[t_room as usize].zone != my_zone) || !t.awake())
        {
            continue;
        }
        let color_on = if g.ch(to).color_lev() >= C_NRM { color } else { b"" };
        let mut msg = color_on.to_vec();
        msg.extend_from_slice(b"$n ");
        msg.extend_from_slice(com_name);
        msg.extend_from_slice(b"s, '");
        msg.extend_from_slice(&speech);
        msg.push(b'\'');
        msg.extend_from_slice(if color_on.is_empty() { b"" } else { KNRM });
        if let Some(rendered) =
            act_full(g, &msg, false, Some(chid), None, ActArg::Char(to), comm::TO_VICT | comm::TO_SLEEP)
        {
            add_history(g, to, &rendered, hist_type);
        }
    }
}

/// do_gmote, via a TO_GMOTE act.
fn do_gmote(g: &mut Game, chid: CharId, argument: &[u8]) {
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        send_to_char(g, chid, b"Gemote? Yes? Gemote what?\r\n");
        return;
    }
    let mut msg = b"Gemote: $n ".to_vec();
    msg.extend_from_slice(argument);
    act_full(g, &msg, false, Some(chid), None, ActArg::None, comm::TO_GMOTE);
    // Sender echo through the same TO_GMOTE loop — the sender is included.
}

/// do_qcomm — qsay/qecho.
pub fn do_qcomm(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, subcmd: i32) {
    use crate::interpreter::SCMD_QECHO;
    if !g.ch(chid).prf(flags::PRF_QUEST) {
        send_to_char(g, chid, b"You aren't even part of the quest!\r\n");
        return;
    }
    let argument = skip_spaces(argument);
    if argument.is_empty() {
        let cmdname = g.commands[cmd].command.clone();
        let mut msg: BStr = Vec::new();
        let mut cap = cmdname.clone();
        if let Some(c) = cap.first_mut() {
            *c = c.to_ascii_uppercase();
        }
        msg.extend_from_slice(&cap);
        msg.extend_from_slice(b"?  Yes, fine, ");
        msg.extend_from_slice(&cmdname);
        msg.extend_from_slice(b" we must, but WHAT??\r\n");
        send_to_char(g, chid, &msg);
        return;
    }
    let mut speech = argument.to_vec();
    if g.config.special_in_comm && legal_communication(&speech) {
        parse_at(&mut speech);
    }
    if subcmd == SCMD_QECHO {
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        g.mudlog(MudlogKind::Cmp, LVL_GOD, true, &format!("(GC) {} qechoed: {}", name, String::from_utf8_lossy(&speech)));
    }
    // Sender.
    if !g.ch(chid).is_npc() && g.ch(chid).prf(flags::PRF_NOREPEAT) {
        let ok = g.config.ok.clone();
        send_to_char(g, chid, &ok);
    } else if subcmd == SCMD_QECHO {
        let mut msg = speech.clone();
        msg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &msg);
    } else {
        let mut msg = b"You quest-say, '".to_vec();
        msg.extend_from_slice(&speech);
        msg.extend_from_slice(b"'\r\n");
        send_to_char(g, chid, &msg);
    }
    // Receivers.
    for di in g.descriptors.indices() {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != ConState::Playing {
            continue;
        }
        let Some(to) = d.character else { continue };
        if to == chid {
            continue;
        }
        let Some(t) = g.try_ch(to) else { continue };
        if !t.prf(flags::PRF_QUEST) {
            continue;
        }
        if subcmd == SCMD_QECHO {
            let mut msg = speech.clone();
            msg.extend_from_slice(b"\r\n");
            send_to_char(g, to, &msg);
        } else {
            let mut msg = b"$n quest-says, '".to_vec();
            msg.extend_from_slice(&speech);
            msg.push(b'\'');
            act(g, &msg, false, Some(chid), None, Some(to), comm::TO_VICT | comm::TO_SLEEP);
        }
    }
}
