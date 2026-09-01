//! BFS pathfinding (find_first_step), the track skill, and
//! hunt_victim. Rooms are marked with ROOM_BFS_MARK, and every mark is
//! cleared on
//! each call; a local visited set is observably identical.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::spells::{SKILL_TRACK, TYPE_UNDEFINED};
use mud_data::types::*;

use crate::comm::send_to_char;
use crate::fight::dir_count;
use crate::game::Game;
use crate::interpreter::one_argument;

pub const BFS_ERROR: i32 = -1;
pub const BFS_ALREADY_THERE: i32 = -2;
pub const BFS_NO_PATH: i32 = -3;

fn valid_edge(g: &Game, marked: &[bool], x: RoomRnum, y: usize) -> bool {
    let Some(e) = g.world.rooms[x as usize].dir_option[y].as_deref() else {
        return false;
    };
    if e.to_room == NOWHERE {
        return false;
    }
    if !g.config.track_through_doors && e.exit_info & flags::EX_CLOSED != 0 {
        return false;
    }
    let to = e.to_room as usize;
    if g.world.rooms[to].room_flags[0] & (1 << flags::ROOM_NOTRACK) != 0 || marked[to] {
        return false;
    }
    true
}

pub fn find_first_step(g: &mut Game, src: RoomRnum, target: RoomRnum) -> i32 {
    if src == NOWHERE
        || target == NOWHERE
        || src as usize >= g.world.rooms.len()
        || target as usize >= g.world.rooms.len()
    {
        g.log(format!("SYSERR: Illegal value {} or {} passed to find_first_step. (graph.c)", src, target));
        return BFS_ERROR;
    }
    if src == target {
        return BFS_ALREADY_THERE;
    }

    let mut marked = vec![false; g.world.rooms.len()];
    marked[src as usize] = true;

    let mut queue: std::collections::VecDeque<(RoomRnum, i32)> = Default::default();
    for dir in 0..dir_count(g) {
        if valid_edge(g, &marked, src, dir) {
            let to = g.world.rooms[src as usize].dir_option[dir].as_deref().unwrap().to_room;
            marked[to as usize] = true;
            queue.push_back((to, dir as i32));
        }
    }

    while let Some(&(room, first_dir)) = queue.front() {
        if room == target {
            return first_dir;
        }
        for dir in 0..dir_count(g) {
            if valid_edge(g, &marked, room, dir) {
                let to = g.world.rooms[room as usize].dir_option[dir].as_deref().unwrap().to_room;
                marked[to as usize] = true;
                queue.push_back((to, first_dir));
            }
        }
        queue.pop_front();
    }
    BFS_NO_PATH
}

pub fn do_track(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if g.ch(chid).is_npc() || g.ch(chid).get_skill(SKILL_TRACK) == 0 {
        send_to_char(g, chid, b"You have no idea how.\r\n");
        return;
    }
    let (arg, _) = one_argument(argument);
    if arg.is_empty() {
        send_to_char(g, chid, b"Whom are you trying to track?\r\n");
        return;
    }
    let Some(vict) = crate::handler::get_char_world_vis(g, chid, &arg, None) else {
        send_to_char(g, chid, b"No one is around by that name.\r\n");
        return;
    };
    if g.ch(vict).aff(flags::AFF_NOTRACK) {
        send_to_char(g, chid, b"You sense no trail.\r\n");
        return;
    }

    // 101 is a complete failure, no matter what the proficiency.
    if g.rng.rand_number(0, 101) >= g.ch(chid).get_skill(SKILL_TRACK) {
        let mut tries = 10;
        let room = g.ch(chid).in_room;
        let mut dir;
        loop {
            dir = g.rng.rand_number(0, dir_count(g) as i32 - 1) as usize;
            tries -= 1;
            if crate::fight::can_go(g, room, dir).is_some() || tries == 0 {
                break;
            }
        }
        let mut out = b"You sense a trail ".to_vec();
        out.extend_from_slice(mud_data::tables::DIRS[dir].as_bytes());
        out.extend_from_slice(b" from here!\r\n");
        send_to_char(g, chid, &out);
        return;
    }

    let src = g.ch(chid).in_room;
    let dst = g.ch(vict).in_room;
    let dir = find_first_step(g, src, dst);
    match dir {
        BFS_ERROR => send_to_char(g, chid, b"Hmm.. something seems to be wrong.\r\n"),
        BFS_ALREADY_THERE => send_to_char(g, chid, b"You're already in the same room!!\r\n"),
        BFS_NO_PATH => {
            let hmhr: &[u8] = match g.ch(vict).sex {
                SEX_MALE => b"him",
                SEX_FEMALE => b"her",
                _ => b"it",
            };
            let mut out = b"You can't sense a trail to ".to_vec();
            out.extend_from_slice(hmhr);
            out.extend_from_slice(b" from here.\r\n");
            send_to_char(g, chid, &out);
        }
        d => {
            let mut out = b"You sense a trail ".to_vec();
            out.extend_from_slice(mud_data::tables::DIRS[d as usize].as_bytes());
            out.extend_from_slice(b" from here!\r\n");
            send_to_char(g, chid, &out);
        }
    }
}

/// hunt_victim. HUNTING is set only by the DG mhunt
/// command (stage 6), but the machinery runs from mobile_activity now.
pub fn hunt_victim(g: &mut Game, chid: CharId) {
    let Some(prey) = g.ch(chid).hunting else { return };
    if g.ch(chid).fighting.is_some() {
        return;
    }

    // Make sure the char still exists.
    let found = g.character_list.iter().any(|&c| c == prey) && g.try_ch(prey).is_some();
    if !found {
        crate::act::comm::do_say(g, chid, b"Damn!  My prey is gone!!", 0, 0);
        g.ch_mut(chid).hunting = None;
        return;
    }

    let src = g.ch(chid).in_room;
    let dst = g.ch(prey).in_room;
    let dir = find_first_step(g, src, dst);
    if dir < 0 {
        let hmhr: &[u8] = match g.ch(prey).sex {
            SEX_MALE => b"him",
            SEX_FEMALE => b"her",
            _ => b"it",
        };
        let mut buf = b"Damn!  I lost ".to_vec();
        buf.extend_from_slice(hmhr);
        buf.push(b'!');
        crate::act::comm::do_say(g, chid, &buf, 0, 0);
        g.ch_mut(chid).hunting = None;
    } else {
        crate::act::movement::perform_move(g, chid, dir, true);
        if g.try_ch(chid).is_some()
            && g.try_ch(prey).is_some()
            && g.ch(chid).in_room == g.ch(prey).in_room
        {
            crate::fight::hit(g, chid, prey, TYPE_UNDEFINED);
        }
    }
}
