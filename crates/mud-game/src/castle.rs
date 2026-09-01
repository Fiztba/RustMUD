//! King's Castle (zone 150) special procedures by Pjotr/Sapowox.
//! The RNG draw order is load-bearing: the unconditional draws (training
//! master, Jerry, Peter) and the banzaii/get_victim draw gating are part of
//! observable. King Welmar's fry_victim performs its rolls
//! and acts; the actual cast_spell calls land at stage 5 (scripts avoid
//! castle fights).

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::spells::TYPE_UNDEFINED;
use mud_data::types::*;

use crate::comm::{act, send_to_char, TO_CHAR, TO_NOTVICT, TO_ROOM, TO_VICT};
use crate::game::Game;
use crate::interpreter::{SCMD_CLOSE, SCMD_LOCK, SCMD_OPEN, SCMD_UNLOCK};
use crate::spec::MobSpec;

const Z_KINGS_C: Idx = 150;

fn castle_zone(g: &Game) -> Option<usize> {
    g.world.zones.iter().position(|z| z.number == Z_KINGS_C)
}

/// castle_virtual: zone bot + offset.
fn castle_virtual(g: &Game, offset: Idx) -> Option<Idx> {
    castle_zone(g).map(|z| g.world.zones[z].bot + offset)
}

fn castle_real_room(g: &Game, roomoffset: Idx) -> Option<RoomRnum> {
    let vnum = castle_virtual(g, roomoffset)?;
    g.world.real_room(vnum)
}

pub fn assign_kings_castle(g: &mut Game) {
    let assign = |g: &mut Game, offset: Idx, spec: MobSpec| {
        let Some(vnum) = castle_virtual(g, offset) else { return };
        match g.world.real_mobile(vnum) {
            Some(rnum) => g.mob_specs[rnum as usize] = Some(spec),
            None => {
                if !g.mini_mud {
                    g.log(format!("SYSERR: assign_kings_castle(): can't find mob #{}.", vnum));
                }
            }
        }
    };
    assign(g, 0, MobSpec::CastleGuard); // Gwydion
    assign(g, 1, MobSpec::KingWelmar); // Our dear friend, the King
    assign(g, 3, MobSpec::CastleGuard); // Jim
    assign(g, 4, MobSpec::CastleGuard); // Brian
    assign(g, 5, MobSpec::CastleGuard); // Mick
    assign(g, 6, MobSpec::CastleGuard); // Matt
    assign(g, 7, MobSpec::CastleGuard); // Jochem
    assign(g, 8, MobSpec::CastleGuard); // Anne
    assign(g, 9, MobSpec::CastleGuard); // Andrew
    assign(g, 10, MobSpec::CastleGuard); // Bertram
    assign(g, 11, MobSpec::CastleGuard); // Jeanette
    assign(g, 12, MobSpec::Peter); // Peter
    assign(g, 13, MobSpec::TrainingMaster); // The training master
    assign(g, 16, MobSpec::James); // James the Butler
    assign(g, 17, MobSpec::Cleaning); // Ze Cleaning Fomen
    assign(g, 20, MobSpec::Tim); // Tim, Tom's twin
    assign(g, 21, MobSpec::Tom); // Tom, Tim's twin
    assign(g, 24, MobSpec::DicknDavid); // Dick, guard of the Treasury
    assign(g, 25, MobSpec::DicknDavid); // David, Dicks brother
    assign(g, 26, MobSpec::Jerry); // Jerry, the Gambler
    assign(g, 27, MobSpec::CastleGuard); // Michael
    assign(g, 28, MobSpec::CastleGuard); // Hans
    assign(g, 29, MobSpec::CastleGuard); // Boris
}

fn mob_vnum_of(g: &Game, chid: CharId) -> Option<Idx> {
    let rnum = g.ch(chid).mob_rnum;
    if rnum == NOBODY {
        None
    } else {
        Some(g.world.mob_protos[rnum as usize].vnum)
    }
}

fn member_of_staff(g: &Game, chid: CharId) -> bool {
    if !g.ch(chid).is_npc() {
        return false;
    }
    let Some(ch_num) = mob_vnum_of(g, chid) else { return false };
    let cv = |o: Idx| castle_virtual(g, o).unwrap_or(Idx::MAX);
    ch_num == cv(1)
        || (ch_num > cv(2) && ch_num < cv(15))
        || (ch_num > cv(15) && ch_num < cv(18))
        || (ch_num > cv(18) && ch_num < cv(30))
}

fn member_of_royal_guard(g: &Game, chid: CharId) -> bool {
    if !g.ch(chid).is_npc() {
        return false;
    }
    let Some(ch_num) = mob_vnum_of(g, chid) else { return false };
    let cv = |o: Idx| castle_virtual(g, o).unwrap_or(Idx::MAX);
    ch_num == cv(3)
        || ch_num == cv(6)
        || (ch_num > cv(7) && ch_num < cv(12))
        || (ch_num > cv(23) && ch_num < cv(26))
}

/// find_npc_by_name: prefix match on short_descr.
fn find_npc_by_name(g: &Game, at: CharId, name: &[u8]) -> Option<CharId> {
    let room = g.ch(at).in_room;
    if room == NOWHERE {
        return None;
    }
    for &c in &g.rooms[room as usize].people {
        let Some(cc) = g.try_ch(c) else { continue };
        if cc.is_npc()
            && cc
                .short_descr
                .as_deref()
                .is_some_and(|s| s.len() >= name.len() && &s[..name.len()] == name)
        {
            return Some(c);
        }
    }
    None
}

fn find_guard(g: &Game, at: CharId) -> Option<CharId> {
    let room = g.ch(at).in_room;
    if room == NOWHERE {
        return None;
    }
    for &c in &g.rooms[room as usize].people {
        let Some(cc) = g.try_ch(c) else { continue };
        if cc.fighting.is_none() && member_of_royal_guard(g, c) {
            return Some(c);
        }
    }
    None
}

/// get_victim: random room char fighting castle staff.
fn get_victim(g: &mut Game, at: CharId) -> Option<CharId> {
    let room = g.ch(at).in_room;
    if room == NOWHERE {
        return None;
    }
    let people = g.rooms[room as usize].people.clone();
    let mut num_bad_guys = 0;
    for &c in &people {
        let Some(cc) = g.try_ch(c) else { continue };
        if let Some(f) = cc.fighting {
            if g.try_ch(f).is_some() && member_of_staff(g, f) {
                num_bad_guys += 1;
            }
        }
    }
    if num_bad_guys == 0 {
        return None;
    }
    let victim = g.rng.rand_number(0, num_bad_guys); // we give them a chance
    if victim == 0 {
        return None;
    }
    let mut count = 0;
    for &c in &people {
        let Some(cc) = g.try_ch(c) else { continue };
        let Some(f) = cc.fighting else { continue };
        if g.try_ch(f).is_none() || !member_of_staff(g, f) {
            continue;
        }
        count += 1;
        if count == victim {
            return Some(c);
        }
    }
    None
}

fn banzaii(g: &mut Game, chid: CharId) -> bool {
    if !g.ch(chid).awake() || g.ch(chid).position == POS_FIGHTING {
        return false;
    }
    let Some(opponent) = get_victim(g, chid) else { return false };
    act(g, b"$n roars: 'Protect the Kingdom of Great King Welmar!  BANZAIIII!!!'", false, Some(chid), None, None, TO_ROOM);
    crate::fight::hit(g, chid, opponent, TYPE_UNDEFINED);
    true
}

fn do_npc_rescue(g: &mut Game, hero: CharId, victim: CharId) -> bool {
    let room = g.ch(hero).in_room;
    let bad_guy = g.rooms[room as usize]
        .people
        .iter()
        .copied()
        .find(|&c| g.try_ch(c).is_some_and(|cc| cc.fighting == Some(victim)));
    // NO WAY I'll rescue the one I'm fighting!
    let Some(bad_guy) = bad_guy else { return false };
    if bad_guy == hero {
        return false;
    }

    act(g, b"You bravely rescue $N.\r\n", false, Some(hero), None, Some(victim), TO_CHAR);
    act(g, b"You are rescued by $N, your loyal friend!\r\n", false, Some(victim), None, Some(hero), TO_CHAR);
    act(g, b"$n heroically rescues $N.", false, Some(hero), None, Some(victim), TO_NOTVICT);

    if g.ch(bad_guy).fighting.is_some() {
        crate::fight::stop_fighting(g, bad_guy);
    }
    if g.ch(hero).fighting.is_some() {
        crate::fight::stop_fighting(g, hero);
    }
    crate::fight::set_fighting(g, hero, bad_guy);
    crate::fight::set_fighting(g, bad_guy, hero);
    true
}

/// block_way. `prohibited_direction` is pre-increment: passing 1 blocks
/// command index 2 = "east".
fn block_way(g: &mut Game, chid: CharId, cmd: usize, in_room_vnum: Idx, prohibited_direction: usize) -> bool {
    if cmd != prohibited_direction + 1 {
        return false;
    }
    if g.ch(chid)
        .short_descr
        .as_deref()
        .is_some_and(|s| s.starts_with(b"King Welmar"))
    {
        return false;
    }
    if g.ch(chid).in_room != g.world.real_room(in_room_vnum).unwrap_or(NOWHERE) {
        return false;
    }
    if !member_of_staff(g, chid) {
        act(g, b"The guard roars at $n and pushes $m back.", false, Some(chid), None, None, TO_ROOM);
    }
    send_to_char(g, chid, b"The guard roars: 'Entrance is Prohibited!', and pushes you back.\r\n");
    true
}

fn is_trash(g: &Game, oid: mud_data::ids::ObjId) -> bool {
    if !g.obj(oid).can_wear(flags::ITEM_WEAR_TAKE) {
        return false;
    }
    g.obj(oid).type_flag == flags::ITEM_DRINKCON || g.obj(oid).cost <= 10
}

/// fry_victim: King Welmar's per-round nastiness.
fn fry_victim(g: &mut Game, chid: CharId) {
    use mud_data::spells::{SPELL_COLOR_SPRAY, SPELL_FIREBALL, SPELL_HARM, SPELL_HEAL};
    if g.ch(chid).points.mana < 10 {
        return;
    }
    let Some(tch) = get_victim(g, chid) else { return };

    match g.rng.rand_number(0, 8) {
        1..=3 => {
            send_to_char(g, chid, b"You raise your hand in a dramatical gesture.\r\n");
            act(g, b"$n raises $s hand in a dramatical gesture.", true, Some(chid), None, None, TO_ROOM);
            crate::spell_parser::cast_spell(g, chid, Some(tch), None, SPELL_COLOR_SPRAY);
        }
        4..=5 => {
            send_to_char(g, chid, b"You concentrate and mumble to yourself.\r\n");
            act(g, b"$n concentrates, and mumbles to $mself.", true, Some(chid), None, None, TO_ROOM);
            crate::spell_parser::cast_spell(g, chid, Some(tch), None, SPELL_HARM);
        }
        6..=7 => {
            act(g, b"You look deeply into the eyes of $N.", true, Some(chid), None, Some(tch), TO_CHAR);
            act(g, b"$n looks deeply into the eyes of $N.", true, Some(chid), None, Some(tch), TO_NOTVICT);
            act(g, b"You see an ill-boding flame in the eye of $n.", true, Some(chid), None, Some(tch), TO_VICT);
            crate::spell_parser::cast_spell(g, chid, Some(tch), None, SPELL_FIREBALL);
        }
        _ => {
            if g.rng.rand_number(0, 1) == 0 {
                crate::spell_parser::cast_spell(g, chid, Some(chid), None, SPELL_HEAL);
            }
        }
    }
    if g.try_ch(chid).is_some() {
        g.ch_mut(chid).points.mana -= 10;
    }
}

pub fn king_welmar(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    const MONOLOG: [&[u8]; 4] = [
        b"$n proclaims 'Primus in regnis Geticis coronam'.",
        b"$n proclaims 'regiam gessi, subiique regis'.",
        b"$n proclaims 'munus et mores colui sereno'.",
        b"$n proclaims 'principe dignos'.",
    ];
    const BEDROOM_PATH: &[u8] = b"s33004o1c1S.";
    const THRONE_PATH: &[u8] = b"W3o3cG52211rg.";
    const MONOLOG_PATH: &[u8] = b"ABCDPPPP.";

    let ch = me;
    if !g.mob_paths.get(&ch).map(|s| s.moving).unwrap_or(false) {
        let hours = g.time_info.hours;
        let in_room = g.ch(ch).in_room;
        let new_path = if hours == 8 && Some(in_room) == castle_real_room(g, 51) {
            Some(1)
        } else if hours == 21 && Some(in_room) == castle_real_room(g, 17) {
            Some(0)
        } else if hours == 12 && Some(in_room) == castle_real_room(g, 17) {
            Some(2)
        } else {
            None
        };
        if let Some(p) = new_path {
            let state = g.mob_paths.entry(ch).or_default();
            state.moving = true;
            state.path = p;
            state.index = 0;
        }
    }
    let (moving, path_no, index) = {
        let state = g.mob_paths.entry(ch).or_default();
        (state.moving, state.path, state.index)
    };
    let pos = g.ch(ch).position;
    if cmd != 0 || pos < POS_SLEEPING || (pos == POS_SLEEPING && !moving) {
        return false;
    }

    if pos == POS_FIGHTING {
        fry_victim(g, ch);
        return false;
    } else if banzaii(g, ch) {
        return false;
    }

    if !moving {
        return false;
    }

    let path: &[u8] = match path_no {
        0 => BEDROOM_PATH,
        1 => THRONE_PATH,
        _ => MONOLOG_PATH,
    };
    let step = path.get(index).copied().unwrap_or(b'.');
    match step {
        b'0'..=b'5' => {
            crate::act::movement::perform_move(g, ch, (step - b'0') as i32, true);
        }
        b'A'..=b'D' => {
            act(g, MONOLOG[(step - b'A') as usize], false, Some(ch), None, None, TO_ROOM);
        }
        b'P' => {}
        b'W' => {
            g.ch_mut(ch).position = POS_STANDING;
            act(g, b"$n awakens and stands up.", false, Some(ch), None, None, TO_ROOM);
        }
        b'S' => {
            g.ch_mut(ch).position = POS_SLEEPING;
            act(g, b"$n lies down on $s beautiful bed and instantly falls asleep.", false, Some(ch), None, None, TO_ROOM);
        }
        b'r' => {
            g.ch_mut(ch).position = POS_SITTING;
            act(g, b"$n sits down on $s great throne.", false, Some(ch), None, None, TO_ROOM);
        }
        b's' => {
            g.ch_mut(ch).position = POS_STANDING;
            act(g, b"$n stands up.", false, Some(ch), None, None, TO_ROOM);
        }
        b'G' => {
            act(g, b"$n says 'Good morning, trusted friends.'", false, Some(ch), None, None, TO_ROOM);
        }
        b'g' => {
            act(g, b"$n says 'Good morning, dear subjects.'", false, Some(ch), None, None, TO_ROOM);
        }
        b'o' => {
            crate::act::movement::do_gen_door(g, ch, b"door", 0, SCMD_UNLOCK);
            crate::act::movement::do_gen_door(g, ch, b"door", 0, SCMD_OPEN);
        }
        b'c' => {
            crate::act::movement::do_gen_door(g, ch, b"door", 0, SCMD_CLOSE);
            crate::act::movement::do_gen_door(g, ch, b"door", 0, SCMD_LOCK);
        }
        b'.' => {
            g.mob_paths.entry(ch).or_default().moving = false;
        }
        _ => {}
    }
    g.mob_paths.entry(ch).or_default().index += 1;
    false
}

pub fn training_master(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    let ch = me;
    if !g.ch(ch).awake() || g.ch(ch).position == POS_FIGHTING {
        return false;
    }
    if cmd != 0 {
        return false;
    }
    if banzaii(g, ch) || g.rng.rand_number(0, 2) != 0 {
        return false;
    }
    let Some(mut pupil1) = find_npc_by_name(g, ch, b"Brian") else { return false };
    let Some(mut pupil2) = find_npc_by_name(g, ch, b"Mick") else { return false };
    if g.ch(pupil1).fighting.is_some() || g.ch(pupil2).fighting.is_some() {
        return false;
    }
    if g.rng.rand_number(0, 1) != 0 {
        std::mem::swap(&mut pupil1, &mut pupil2);
    }
    match g.rng.rand_number(0, 7) {
        0 => {
            act(g, b"$n hits $N on $s head with a powerful blow.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"You hit $N on $s head with a powerful blow.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"$n hits you on your head with a powerful blow.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        1 => {
            act(g, b"$n hits $N in $s chest with a thrust.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"You manage to thrust $N in the chest.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"$n manages to thrust you in your chest.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        2 => {
            send_to_char(g, ch, b"You command your pupils to bow.\r\n");
            act(g, b"$n commands $s pupils to bow.", false, Some(ch), None, None, TO_ROOM);
            act(g, b"$n bows before $N.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"$N bows before $n.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"You bow before $N, who returns your gesture.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"You bow before $n, who returns your gesture.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        3 => {
            act(g, b"$N yells at $n, as he fumbles and drops $s sword.", false, Some(pupil1), None, Some(ch), TO_NOTVICT);
            act(g, b"$n quickly picks up $s weapon.", false, Some(pupil1), None, None, TO_ROOM);
            act(g, b"$N yells at you, as you fumble, losing your weapon.", false, Some(pupil1), None, Some(ch), TO_CHAR);
            send_to_char(g, pupil1, b"You quickly pick up your weapon again.\r\n");
            act(g, b"You yell at $n, as he fumbles, losing $s weapon.", false, Some(pupil1), None, Some(ch), TO_VICT);
        }
        4 => {
            act(g, b"$N tricks $n, and slashes him across the back.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"$N tricks you, and slashes you across your back.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"You trick $n, and quickly slash him across $s back.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        5 => {
            act(g, b"$n lunges a blow at $N but $N parries skillfully.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"You lunge a blow at $N but $E parries skillfully.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"$n lunges a blow at you, but you skillfully parry it.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        6 => {
            act(g, b"$n clumsily tries to kick $N, but misses.", false, Some(pupil1), None, Some(pupil2), TO_NOTVICT);
            act(g, b"You clumsily miss $N with your poor excuse for a kick.", false, Some(pupil1), None, Some(pupil2), TO_CHAR);
            act(g, b"$n fails an unusually clumsy attempt at kicking you.", false, Some(pupil1), None, Some(pupil2), TO_VICT);
        }
        _ => {
            send_to_char(g, ch, b"You show your pupils an advanced technique.\r\n");
            act(g, b"$n shows $s pupils an advanced technique.", false, Some(ch), None, None, TO_ROOM);
        }
    }
    false
}

fn castle_twin_proc(g: &mut Game, chid: CharId, me: CharId, cmd: usize, ctlnum: Idx, twinname: &[u8]) -> bool {
    let ch = me;
    if !g.ch(ch).awake() {
        return false;
    }
    if cmd != 0 {
        let room_vnum = castle_virtual(g, ctlnum).unwrap_or(Idx::MAX);
        return block_way(g, chid, cmd, room_vnum, 1);
    }
    if let Some(king) = find_npc_by_name(g, ch, b"King Welmar") {
        if g.ch(ch).master.is_none() {
            crate::act::movement::do_follow(g, ch, b"King Welmar", 0, 0);
        }
        if g.ch(king).fighting.is_some() {
            do_npc_rescue(g, ch, king);
        }
    }
    if let Some(twin) = find_npc_by_name(g, ch, twinname) {
        if g.ch(twin).fighting.is_some() && 2 * g.ch(twin).points.hit < g.ch(ch).points.hit {
            do_npc_rescue(g, ch, twin);
        }
    }
    if g.ch(ch).position != POS_FIGHTING {
        banzaii(g, ch);
    }
    false
}

pub fn tom(g: &mut Game, chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    castle_twin_proc(g, chid, me, cmd, 48, b"Tim")
}

pub fn tim(g: &mut Game, chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    castle_twin_proc(g, chid, me, cmd, 49, b"Tom")
}

/// castle_cleaner, shared by James (gripes) and cleaning.
fn castle_cleaner(g: &mut Game, me: CharId, cmd: usize, gripe: bool) -> bool {
    let ch = me;
    if cmd != 0 || !g.ch(ch).awake() || g.ch(ch).position == POS_FIGHTING {
        return false;
    }
    let room = g.ch(ch).in_room;
    if room == NOWHERE {
        return false;
    }
    let contents = g.rooms[room as usize].contents.clone();
    for oid in contents {
        if !g.try_obj_alive(oid) || !is_trash(g, oid) {
            continue;
        }
        if gripe {
            act(g, b"$n says: 'My oh my!  I ought to fire that lazy cleaning woman!'", false, Some(ch), None, None, TO_ROOM);
            act(g, b"$n picks up a piece of trash.", false, Some(ch), None, None, TO_ROOM);
        }
        crate::handler::obj_from_room(g, oid);
        crate::handler::obj_to_char(g, oid, ch);
        return true;
    }
    false
}

pub fn james(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    castle_cleaner(g, me, cmd, true)
}

pub fn cleaning(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    castle_cleaner(g, me, cmd, false)
}

pub fn castle_guard(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    if cmd != 0 || !g.ch(me).awake() || g.ch(me).position == POS_FIGHTING {
        return false;
    }
    banzaii(g, me)
}

pub fn dick_n_david(g: &mut Game, chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    if !g.ch(me).awake() {
        return false;
    }
    if cmd == 0 && g.ch(me).position != POS_FIGHTING {
        banzaii(g, me);
    }
    let room_vnum = castle_virtual(g, 36).unwrap_or(Idx::MAX);
    block_way(g, chid, cmd, room_vnum, 1)
}

pub fn peter(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    let ch = me;
    if cmd != 0 || !g.ch(ch).awake() || g.ch(ch).position == POS_FIGHTING {
        return false;
    }
    if banzaii(g, ch) {
        return false;
    }
    if g.rng.rand_number(0, 3) == 0 {
        if let Some(guard) = find_guard(g, ch) {
            match g.rng.rand_number(0, 5) {
                0 => {
                    act(g, b"$N comes sharply into attention as $n inspects $M.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"$N comes sharply into attention as you inspect $M.", false, Some(ch), None, Some(guard), TO_CHAR);
                    act(g, b"You go sharply into attention as $n inspects you.", false, Some(ch), None, Some(guard), TO_VICT);
                }
                1 => {
                    act(g, b"$N looks very small, as $n roars at $M.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"$N looks very small as you roar at $M.", false, Some(ch), None, Some(guard), TO_CHAR);
                    act(g, b"You feel very small as $N roars at you.", false, Some(ch), None, Some(guard), TO_VICT);
                }
                2 => {
                    act(g, b"$n gives $N some Royal directions.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"You give $N some Royal directions.", false, Some(ch), None, Some(guard), TO_CHAR);
                    act(g, b"$n gives you some Royal directions.", false, Some(ch), None, Some(guard), TO_VICT);
                }
                3 => {
                    act(g, b"$n looks at you.", false, Some(ch), None, Some(guard), TO_VICT);
                    act(g, b"$n looks at $N.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"$n growls: 'Those boots need polishing!'", false, Some(ch), None, Some(guard), TO_ROOM);
                    act(g, b"You growl at $N.", false, Some(ch), None, Some(guard), TO_CHAR);
                }
                4 => {
                    act(g, b"$n looks at you.", false, Some(ch), None, Some(guard), TO_VICT);
                    act(g, b"$n looks at $N.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"$n growls: 'Straighten that collar!'", false, Some(ch), None, Some(guard), TO_ROOM);
                    act(g, b"You growl at $N.", false, Some(ch), None, Some(guard), TO_CHAR);
                }
                _ => {
                    act(g, b"$n looks at you.", false, Some(ch), None, Some(guard), TO_VICT);
                    act(g, b"$n looks at $N.", false, Some(ch), None, Some(guard), TO_NOTVICT);
                    act(g, b"$n growls: 'That chain mail looks rusty!  CLEAN IT !!!'", false, Some(ch), None, Some(guard), TO_ROOM);
                    act(g, b"You growl at $N.", false, Some(ch), None, Some(guard), TO_CHAR);
                }
            }
        }
    }
    false
}

pub fn jerry(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    let ch = me;
    if !g.ch(ch).awake() || g.ch(ch).position == POS_FIGHTING {
        return false;
    }
    if cmd != 0 {
        return false;
    }
    if banzaii(g, ch) || g.rng.rand_number(0, 2) != 0 {
        return false;
    }
    let mut gambler1 = ch;
    let Some(mut gambler2) = find_npc_by_name(g, ch, b"Michael") else { return false };
    if g.ch(gambler1).fighting.is_some() || g.ch(gambler2).fighting.is_some() {
        return false;
    }
    if g.rng.rand_number(0, 1) != 0 {
        std::mem::swap(&mut gambler1, &mut gambler2);
    }
    match g.rng.rand_number(0, 5) {
        0 => {
            act(g, b"$n rolls the dice and cheers loudly at the result.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You roll the dice and cheer. GREAT!", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n cheers loudly as $e rolls the dice.", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
        1 => {
            act(g, b"$n curses the Goddess of Luck roundly as he sees $N's roll.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You curse the Goddess of Luck as $N rolls.", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n swears angrily. You are in luck!", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
        2 => {
            act(g, b"$n sighs loudly and gives $N some gold.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You sigh loudly at the pain of having to give $N some gold.", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n sighs loudly as $e gives you your rightful win.", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
        3 => {
            act(g, b"$n smiles remorsefully as $N's roll tops $s.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You smile sadly as you see that $N beats you. Again.", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n smiles remorsefully as your roll tops $s.", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
        4 => {
            act(g, b"$n excitedly follows the dice with $s eyes.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You excitedly follow the dice with your eyes.", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n excitedly follows the dice with $s eyes.", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
        _ => {
            act(g, b"$n says 'Well, my luck has to change soon', as he shakes the dice.", false, Some(gambler1), None, Some(gambler2), TO_NOTVICT);
            act(g, b"You say 'Well, my luck has to change soon' and shake the dice.", false, Some(gambler1), None, Some(gambler2), TO_CHAR);
            act(g, b"$n says 'Well, my luck has to change soon', as he shakes the dice.", false, Some(gambler1), None, Some(gambler2), TO_VICT);
        }
    }
    false
}
