//! do_write — jotting a note on a piece of paper.
//!
//! Lives in its own module because it is the one editor client outside the
//! login menu that writes straight into a game object; boards, mail and IBT
//! all intercept the `write` command earlier (spec-proc / subcmd) and route
//! through `playing_string_cleanup`.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::types::*;

use crate::comm::{act, send_editor_help, send_to_char, string_write, TO_CHAR, TO_ROOM};
use crate::game::Game;
use crate::handler::{can_see_obj, get_obj_in_list_vis};
use crate::interpreter::two_arguments;

pub const MAX_NOTE_LENGTH: usize = 4000;

/// AN — note that only the five vowels are tested, not 'y'.
fn an(s: &[u8]) -> &'static [u8] {
    match s.first() {
        Some(c) if b"aeiouAEIOU".contains(c) => b"an",
        _ => b"a",
    }
}

pub fn do_write(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (papername, penname, _) = two_arguments(argument);
    if g.ch(chid).desc.is_none() {
        return;
    }
    if papername.is_empty() {
        send_to_char(
            g,
            chid,
            b"Write?  With what?  ON what?  What are you trying to do?!?\r\n",
        );
        return;
    }

    let carrying = g.ch(chid).carrying.clone();
    let mut paper: Option<ObjId>;
    let mut pen: Option<ObjId>;

    if !penname.is_empty() {
        paper = get_obj_in_list_vis(g, chid, &papername, None, &carrying);
        if paper.is_none() {
            let mut m = b"You have no ".to_vec();
            m.extend_from_slice(&papername);
            m.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &m);
            return;
        }
        pen = get_obj_in_list_vis(g, chid, &penname, None, &carrying);
        if pen.is_none() {
            let mut m = b"You have no ".to_vec();
            m.extend_from_slice(&penname);
            m.extend_from_slice(b".\r\n");
            send_to_char(g, chid, &m);
            return;
        }
    } else {
        // One argument: work out which half of the pair it is.
        paper = get_obj_in_list_vis(g, chid, &papername, None, &carrying);
        let Some(found) = paper else {
            let mut m = b"There is no ".to_vec();
            m.extend_from_slice(&papername);
            m.extend_from_slice(b" in your inventory.\r\n");
            send_to_char(g, chid, &m);
            return;
        };
        pen = None;
        let t = g.obj(found).type_flag;
        if t == flags::ITEM_PEN {
            pen = Some(found);
            paper = None;
        } else if t != flags::ITEM_NOTE {
            send_to_char(g, chid, b"That thing has nothing to do with writing.\r\n");
            return;
        }

        let Some(held) = g.ch(chid).equipment[WEAR_HOLD] else {
            let mut m = b"You can't write with ".to_vec();
            m.extend_from_slice(an(&papername));
            m.push(b' ');
            m.extend_from_slice(&papername);
            m.extend_from_slice(b" alone.\r\n");
            send_to_char(g, chid, &m);
            return;
        };
        if !can_see_obj(g, chid, held) {
            send_to_char(g, chid, b"The stuff in your hand is invisible!  Yeech!!\r\n");
            return;
        }
        if pen.is_some() {
            paper = Some(held);
        } else {
            pen = Some(held);
        }
    }

    let (Some(paper), Some(pen)) = (paper, pen) else { return };
    if g.obj(pen).type_flag != flags::ITEM_PEN {
        act(g, b"$p is no good for writing with.", false, Some(chid), Some(pen), None, TO_CHAR);
        return;
    }
    if g.obj(paper).type_flag != flags::ITEM_NOTE {
        act(g, b"You can't write on $p.", false, Some(chid), Some(paper), None, TO_CHAR);
        return;
    }

    // Something on it already: shown, and kept as the abort text.
    let backstr = g.obj(paper).action_description.clone();
    if let Some(text) = backstr.clone() {
        send_to_char(g, chid, b"There's something written on it already:\r\n");
        send_to_char(g, chid, &text);
    }

    act(g, b"$n begins to jot down a note.", true, Some(chid), None, None, TO_ROOM);
    send_editor_help(g, chid);
    string_write(g, chid, MAX_NOTE_LENGTH, 0, backstr);
    if let Some(di) = g.ch(chid).desc {
        if let Some(d) = g.descriptors.get_mut(di) {
            if let Some(s) = d.editing.as_mut() {
                s.note_obj = Some(paper);
            }
        }
    }
}
