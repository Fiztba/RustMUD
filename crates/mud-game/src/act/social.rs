//! do_action, the social command handler.

use mud_data::ids::CharId;

use crate::comm::{self, act, send_to_char};
use crate::game::Game;
use crate::handler::get_char_room_vis;
use crate::interpreter::two_arguments;

pub fn do_action(g: &mut Game, chid: CharId, argument: &[u8], cmd: usize, _subcmd: i32) {
    let Some(soc_idx) = g.commands.get(cmd).and_then(|c| c.social) else {
        send_to_char(g, chid, b"That action is not supported.\r\n");
        return;
    };
    let action = g.socials[soc_idx].clone();

    let (buf, buf2, _) = two_arguments(argument);

    // Two-word form requires body-part messages.
    if !buf2.is_empty() && action.char_body_found.is_none() {
        send_to_char(g, chid, b"Sorry, this social does not support body parts.\r\n");
        return;
    }

    if buf.is_empty() {
        // No argument.
        if let Some(msg) = &action.char_no_arg {
            let mut m = msg.clone();
            m.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &m);
        }
        if let Some(msg) = &action.others_no_arg {
            act(g, msg, action.hide != 0, Some(chid), None, None, comm::TO_ROOM);
        }
        return;
    }

    let vict = get_char_room_vis(g, chid, &buf, None);
    let Some(vict) = vict else {
        // Object socials (char_obj_found) search inventory then room —
        // stage 3 items make these findable; the not-found path applies now.
        if let Some(msg) = &action.not_found {
            let mut m = msg.clone();
            m.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &m);
        } else {
            send_to_char(g, chid, b"I don't see anything by that name here.\r\n");
        }
        return;
    };

    if vict == chid {
        if let Some(msg) = &action.char_auto {
            let mut m = msg.clone();
            m.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &m);
        } else {
            send_to_char(g, chid, b"Erm, no.\r\n");
        }
        if let Some(msg) = &action.others_auto {
            act(g, msg, action.hide != 0, Some(chid), None, None, comm::TO_ROOM);
        }
        return;
    }

    if (g.ch(vict).position as i32) < action.min_victim_position {
        act(
            g,
            b"$N is not in a proper position for that.",
            false,
            Some(chid),
            None,
            Some(vict),
            comm::TO_CHAR | comm::TO_SLEEP,
        );
        return;
    }

    let body = !buf2.is_empty();
    let (char_msg, others_msg, vict_msg) = if body {
        (
            action.char_body_found.clone(),
            action.others_body_found.clone(),
            action.vict_body_found.clone(),
        )
    } else {
        (action.char_found.clone(), action.others_found.clone(), action.vict_found.clone())
    };
    // Body-part socials pass the part via $T (the vict_obj string); the
    // shipped socials.new uses $m/$M forms, so the simple triple works.
    if let Some(msg) = char_msg {
        act(g, &msg, false, Some(chid), None, Some(vict), comm::TO_CHAR | comm::TO_SLEEP);
    }
    if let Some(msg) = others_msg {
        act(g, &msg, action.hide != 0, Some(chid), None, Some(vict), comm::TO_NOTVICT);
    }
    if let Some(msg) = vict_msg {
        act(g, &msg, action.hide != 0, Some(chid), None, Some(vict), comm::TO_VICT);
    }
}
