//! Special procedures: the special dispatch chain,
//! the spec registries for mobiles, objects and rooms, and the
//! stage-3 procs — bank, dump, pet shops. Shop keepers live in
//! shop.rs; guild/mayor/castle and friends arrive with stages 4-5.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::comm::{self, act, send_to_char};
use crate::game::Game;
use crate::handler::{char_to_room, extract_obj};
use crate::interpreter::two_arguments;
use crate::limits::{decrease_bank, decrease_gold, increase_bank, increase_gold};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobSpec {
    ShopKeeper,
    Guild,
    Mayor,
    // King's Castle (zone 150).
    CastleGuard,
    KingWelmar,
    Peter,
    TrainingMaster,
    James,
    Cleaning,
    Tim,
    Tom,
    DicknDavid,
    Jerry,
    Receptionist,
    Cryogenicist,
    Postmaster,
    QuestMaster,
}

/// Path-walker state for the mayor and King Welmar. Held per-mob rather
/// than globally (B5); indistinguishable while each has a single instance,
/// as the shipped world does.
#[derive(Debug, Clone, Copy, Default)]
pub struct PathState {
    pub moving: bool,
    /// Which path (mayor: 0 open / 1 close; king: 0 bedroom / 1 throne /
    /// 2 monolog).
    pub path: u8,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjSpec {
    Bank,
    GenBoard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomSpec {
    Dump,
    PetShop,
}

pub fn call_mob_spec(g: &mut Game, spec: MobSpec, chid: CharId, mob: CharId, cmd: usize, arg: &[u8]) -> bool {
    match spec {
        MobSpec::ShopKeeper => crate::shop::shop_keeper(g, chid, mob, cmd, arg),
        MobSpec::Guild => guild(g, chid, mob, cmd, arg),
        MobSpec::Mayor => mayor(g, chid, mob, cmd, arg),
        MobSpec::CastleGuard => crate::castle::castle_guard(g, chid, mob, cmd, arg),
        MobSpec::KingWelmar => crate::castle::king_welmar(g, chid, mob, cmd, arg),
        MobSpec::Peter => crate::castle::peter(g, chid, mob, cmd, arg),
        MobSpec::TrainingMaster => crate::castle::training_master(g, chid, mob, cmd, arg),
        MobSpec::James => crate::castle::james(g, chid, mob, cmd, arg),
        MobSpec::Cleaning => crate::castle::cleaning(g, chid, mob, cmd, arg),
        MobSpec::Tim => crate::castle::tim(g, chid, mob, cmd, arg),
        MobSpec::Tom => crate::castle::tom(g, chid, mob, cmd, arg),
        MobSpec::DicknDavid => crate::castle::dick_n_david(g, chid, mob, cmd, arg),
        MobSpec::Jerry => crate::castle::jerry(g, chid, mob, cmd, arg),
        MobSpec::Postmaster => crate::mail::postmaster(g, chid, mob, cmd, arg),
        MobSpec::QuestMaster => crate::quest::questmaster(g, chid, mob, cmd, arg),
        MobSpec::Receptionist => {
            crate::objsave::gen_receptionist(g, chid, mob, cmd, arg, crate::objsave::RENT_FACTOR)
        }
        MobSpec::Cryogenicist => {
            crate::objsave::gen_receptionist(g, chid, mob, cmd, arg, crate::objsave::CRYO_FACTOR)
        }
    }
}

fn call_obj_spec(
    g: &mut Game,
    spec: ObjSpec,
    chid: CharId,
    oid: mud_data::ids::ObjId,
    cmd: usize,
    arg: &[u8],
) -> bool {
    match spec {
        ObjSpec::Bank => bank(g, chid, cmd, arg),
        ObjSpec::GenBoard => crate::boards::gen_board(g, chid, oid, cmd, arg),
    }
}

fn call_room_spec(g: &mut Game, spec: RoomSpec, chid: CharId, cmd: usize, arg: &[u8]) -> bool {
    match spec {
        RoomSpec::Dump => dump(g, chid, cmd, arg),
        RoomSpec::PetShop => pet_shops(g, chid, cmd, arg),
    }
}

/// special: room func → worn eq → inventory →
/// room mobs (skipping NOTDEADYET) → room objects; first TRUE consumes.
pub fn special(g: &mut Game, chid: CharId, cmd: usize, arg: &[u8]) -> bool {
    let room = g.ch(chid).in_room;
    if room == NOWHERE {
        return false;
    }

    if let Some(spec) = g.room_specs[room as usize] {
        if call_room_spec(g, spec, chid, cmd, arg) {
            return true;
        }
    }

    for j in 0..NUM_WEARS {
        let Some(oid) = g.ch(chid).equipment[j] else { continue };
        if let Some(spec) = obj_spec(g, oid) {
            if call_obj_spec(g, spec, chid, oid, cmd, arg) {
                return true;
            }
        }
    }

    let carrying = g.ch(chid).carrying.clone();
    for oid in carrying {
        if !g.try_obj_alive(oid) {
            continue;
        }
        if let Some(spec) = obj_spec(g, oid) {
            if call_obj_spec(g, spec, chid, oid, cmd, arg) {
                return true;
            }
        }
    }

    let people = g.rooms[room as usize].people.clone();
    for k in people {
        let Some(kc) = g.try_ch(k) else { continue };
        if kc.mob_flagged(flags::MOB_NOTDEADYET) {
            continue;
        }
        let spec = mob_spec(g, k);
        if let Some(spec) = spec {
            if call_mob_spec(g, spec, chid, k, cmd, arg) {
                return true;
            }
        }
    }

    let contents = g.rooms[room as usize].contents.clone();
    for oid in contents {
        if !g.try_obj_alive(oid) {
            continue;
        }
        if let Some(spec) = obj_spec(g, oid) {
            if call_obj_spec(g, spec, chid, oid, cmd, arg) {
                return true;
            }
        }
    }

    false
}

fn obj_spec(g: &Game, oid: mud_data::ids::ObjId) -> Option<ObjSpec> {
    let rnum = g.obj(oid).item_number;
    if rnum == NOTHING {
        return None;
    }
    g.obj_specs.get(rnum as usize).copied().flatten()
}

fn mob_spec(g: &Game, chid: CharId) -> Option<MobSpec> {
    let ch = g.ch(chid);
    if !ch.is_npc() {
        return None;
    }
    let rnum = ch.mob_rnum;
    if rnum == NOBODY {
        return None;
    }
    g.mob_specs.get(rnum as usize).copied().flatten()
}

// ---- assignment ----

/// assign_mobiles: install the special procedures that are still C code.
///
/// Guildguards, snake, thief, magic user, puff, fido, janitor and cityguard
/// behaviour all comes from DG triggers on the mobs themselves, so none of
/// them is assigned here and none has a proc in this crate.
pub fn assign_mobiles(g: &mut Game) {
    crate::castle::assign_kings_castle(g);

    assignmob(g, 3095, MobSpec::Cryogenicist);

    const GUILD_MOBS: [i32; 38] = [
        120, 121, 122, 123, 2556, 2559, 2562, 2564, 2800, 3020, 3021, 3022, 3023, 5400, 5401,
        5402, 5403, 11518, 25720, 25721, 25722, 25723, 25726, 25732, 27572, 27573, 27574, 27575,
        27721, 29204, 29227, 31601, 31603, 31605, 31607, 31609, 31611, 31639,
    ];
    for vnum in GUILD_MOBS {
        assignmob(g, vnum, MobSpec::Guild);
    }
    // Two more guild rows sit numerically among the 316xx block
    // (31641); kept in file order here.
    assignmob(g, 31641, MobSpec::Guild);

    assignmob(g, 3105, MobSpec::Mayor);

    for vnum in [110, 1201, 3010, 10412, 10719, 25710, 27164, 30128, 31510] {
        assignmob(g, vnum, MobSpec::Postmaster);
    }

    for vnum in [1200, 3005, 5404, 27713, 27730] {
        assignmob(g, vnum, MobSpec::Receptionist);
    }
}

fn assignmob(g: &mut Game, vnum: i32, spec: MobSpec) {
    match g.world.real_mobile(vnum as Idx) {
        Some(rnum) => g.mob_specs[rnum as usize] = Some(spec),
        None => {
            if !g.mini_mud {
                g.log(format!("SYSERR: Attempt to assign spec to non-existant mob #{}", vnum));
            }
        }
    }
}

/// guild — the practice teacher.
fn guild(g: &mut Game, chid: CharId, _mob: CharId, cmd: usize, argument: &[u8]) -> bool {
    if g.ch(chid).is_npc() || cmd == 0 || g.commands[cmd].command != b"practice" {
        return false;
    }
    let argument = crate::interpreter::skip_spaces(argument);

    if argument.is_empty() {
        crate::act::informative::list_skills(g, chid);
        return true;
    }
    if g.ch(chid).ps().practices <= 0 {
        send_to_char(g, chid, b"You do not seem to be able to practice now.\r\n");
        return true;
    }

    let class = g.ch(chid).class.clamp(0, 3) as usize;
    let prac = mud_data::tables::PRAC_PARAMS;
    let splskl: &[u8] = if prac[3][class] == 0 { b"spell" } else { b"skill" };

    let skill_num = find_skill_num(argument);
    let level_ok = skill_num.is_some_and(|n| {
        g.ch(chid).level as i32
            >= mud_data::spells::spell_info(n).min_level[class]
    });
    let Some(skill_num) = skill_num.filter(|&n| n >= 1 && level_ok) else {
        let mut out = b"You do not know of that ".to_vec();
        out.extend_from_slice(splskl);
        out.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &out);
        return true;
    };
    let learned = prac[0][class];
    if g.ch(chid).get_skill(skill_num) >= learned {
        send_to_char(g, chid, b"You are already learned in that area.\r\n");
        return true;
    }
    send_to_char(g, chid, b"You practice for a while...\r\n");
    g.ch_mut(chid).ps_mut().practices -= 1;

    let gain =
        crate::limits::practice_gain_percent(class as i32, g.ch(chid).aff_abils.intel as i32);
    let percent = g.ch(chid).get_skill(skill_num) + gain;
    g.ch_mut(chid).set_skill(skill_num, learned.min(percent));

    if g.ch(chid).get_skill(skill_num) >= learned {
        send_to_char(g, chid, b"You are now learned in that area.\r\n");
    }
    true
}

/// find_skill_num: abbreviation match, then
/// word-by-word abbreviation match.
pub fn find_skill_num(name: &[u8]) -> Option<i32> {
    for skindex in 1..=mud_data::spells::TOP_SPELL_DEFINE {
        let info_name = mud_data::spells::spell_info(skindex).name.as_bytes();
        if crate::handler::is_abbrev(name, info_name) {
            return Some(skindex);
        }
        // Word-by-word abbreviation ("cure light" ~ "cur lig").
        let mut ok = true;
        let mut first_iter = info_name.split(|&c| c == b' ').filter(|w| !w.is_empty());
        let mut second_iter = name.split(|&c| c == b' ').filter(|w| !w.is_empty());
        let mut first = first_iter.next();
        let mut second = second_iter.next();
        while let (Some(f), Some(s)) = (first, second) {
            if !crate::handler::is_abbrev(s, f) {
                ok = false;
                break;
            }
            first = first_iter.next();
            second = second_iter.next();
        }
        if ok && second.is_none() {
            return Some(skindex);
        }
    }
    None
}

/// mayor — mob 3105, the Midgaard gate walker.
fn mayor(g: &mut Game, _chid: CharId, me: CharId, cmd: usize, _arg: &[u8]) -> bool {
    const OPEN_PATH: &[u8] = b"W3a3003b33000c111d0d111Oe333333Oe22c222112212111a1S.";
    const CLOSE_PATH: &[u8] = b"W3a3003b33000c111d0d111CE333333CE22c222112212111a1S.";

    let ch = me;
    if !g.mob_paths.get(&ch).map(|s| s.moving).unwrap_or(false) {
        let hours = g.time_info.hours;
        let new_path = if hours == 6 {
            Some(0u8)
        } else if hours == 20 {
            Some(1u8)
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
    if cmd != 0 || !moving || pos < POS_SLEEPING || pos == POS_FIGHTING {
        return false;
    }

    let path: &[u8] = if path_no == 0 { OPEN_PATH } else { CLOSE_PATH };
    let step = path.get(index).copied().unwrap_or(b'.');
    match step {
        b'0'..=b'3' => {
            crate::act::movement::perform_move(g, ch, (step - b'0') as i32, true);
        }
        b'W' => {
            g.ch_mut(ch).position = mud_data::types::POS_STANDING;
            act(g, b"$n awakens and groans loudly.", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'S' => {
            g.ch_mut(ch).position = mud_data::types::POS_SLEEPING;
            act(g, b"$n lies down and instantly falls asleep.", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'a' => {
            act(g, b"$n says 'Hello Honey!'", false, Some(ch), None, None, comm::TO_ROOM);
            act(g, b"$n smirks.", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'b' => {
            act(g, b"$n says 'What a view!  I must get something done about that dump!'", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'c' => {
            act(g, b"$n says 'Vandals!  Youngsters nowadays have no respect for anything!'", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'd' => {
            act(g, b"$n says 'Good day, citizens!'", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'e' => {
            act(g, b"$n says 'I hereby declare the bazaar open!'", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'E' => {
            act(g, b"$n says 'I hereby declare Midgaard closed!'", false, Some(ch), None, None, comm::TO_ROOM);
        }
        b'O' => {
            crate::act::movement::do_gen_door(g, ch, b"gate", 0, crate::interpreter::SCMD_UNLOCK);
            crate::act::movement::do_gen_door(g, ch, b"gate", 0, crate::interpreter::SCMD_OPEN);
        }
        b'C' => {
            crate::act::movement::do_gen_door(g, ch, b"gate", 0, crate::interpreter::SCMD_CLOSE);
            crate::act::movement::do_gen_door(g, ch, b"gate", 0, crate::interpreter::SCMD_LOCK);
        }
        b'.' => {
            g.mob_paths.entry(ch).or_default().moving = false;
        }
        _ => {}
    }
    g.mob_paths.entry(ch).or_default().index += 1;
    false
}

/// assign_objects: boards (stage 7) and banks.
pub fn assign_objects(g: &mut Game) {
    const BANK_OBJS: [i32; 9] = [115, 334, 336, 3034, 3036, 3907, 10640, 10751, 25758];
    for vnum in BANK_OBJS {
        if let Some(rnum) = g.world.real_object(vnum as Idx) {
            g.obj_specs[rnum as usize] = Some(ObjSpec::Bank);
        }
    }
    const BOARD_OBJS: [i32; 7] = [1226, 1227, 1228, 3096, 3097, 3098, 3099];
    for vnum in BOARD_OBJS {
        if let Some(rnum) = g.world.real_object(vnum as Idx) {
            g.obj_specs[rnum as usize] = Some(ObjSpec::GenBoard);
        }
    }
}

pub fn assign_rooms(g: &mut Game) {
    const PET_SHOP_ROOMS: [i32; 7] = [3031, 10738, 23281, 25722, 27155, 27616, 31523];
    for vnum in PET_SHOP_ROOMS {
        if let Some(rnum) = g.real_room(vnum) {
            g.room_specs[rnum as usize] = Some(RoomSpec::PetShop);
        }
    }
    if g.config.dts_are_dumps {
        for i in 0..g.world.rooms.len() {
            if g.world.rooms[i].room_flags[0] & (1 << flags::ROOM_DEATH) != 0 {
                g.room_specs[i] = Some(RoomSpec::Dump);
            }
        }
    }
}

// ---- the procs ----

fn dump(g: &mut Game, chid: CharId, cmd: usize, arg: &[u8]) -> bool {
    let room = g.ch(chid).in_room;

    while let Some(&k) = g.rooms[room as usize].contents.first() {
        act(g, b"$p vanishes in a puff of smoke!", false, None, Some(k), None, comm::TO_ROOM);
        extract_obj(g, k);
    }

    if g.commands[cmd].command != b"drop" {
        return false;
    }

    crate::act::item::do_drop(g, chid, arg, cmd, crate::interpreter::SCMD_DROP);

    let mut value = 0;
    while let Some(&k) = g.rooms[room as usize].contents.first() {
        act(g, b"$p vanishes in a puff of smoke!", false, None, Some(k), None, comm::TO_ROOM);
        value += 1.max(50.min(g.obj(k).cost / 10));
        extract_obj(g, k);
    }

    if value != 0 {
        send_to_char(g, chid, b"You are awarded for outstanding performance.\r\n");
        act(g, b"$n has been awarded for being a good citizen.", true, Some(chid), None, None, comm::TO_ROOM);

        if g.ch(chid).level < 3 {
            // gain_exp's level-up handling arrives with stage 4; the tiny
            // dump reward can't cross a threshold that matters before then.
            g.ch_mut(chid).points.exp += value;
        } else {
            increase_gold(g, chid, value);
        }
    }
    true
}

/// get_char_room: name search in a specific room, NOT
/// visibility-gated (the pet shop uses it).
fn get_char_room(g: &Game, name: &[u8], room: RoomRnum) -> Option<CharId> {
    let (mut number, name) = crate::handler::get_number(name);
    if number == 0 {
        return None;
    }
    let mut last = None;
    for &i in &g.rooms[room as usize].people {
        if crate::handler::isname(&name, g.ch(i).name.as_deref().unwrap_or(b"")) {
            if number == crate::handler::FIND_INDEX_LAST {
                last = Some(i);
                continue;
            }
            number -= 1;
            if number == 0 {
                return Some(i);
            }
        }
    }
    last
}

/// pet_shops. Pets live in room rnum + 1 ("Gross.").
fn pet_shops(g: &mut Game, chid: CharId, cmd: usize, arg: &[u8]) -> bool {
    let pet_room = g.ch(chid).in_room + 1;
    let cmd_name = g.commands[cmd].command.clone();

    if cmd_name == b"list" {
        send_to_char(g, chid, b"Available pets are:\r\n");
        let people = g.rooms[pet_room as usize].people.clone();
        for pet in people {
            // No, you can't have the Implementor as a pet if he's in there.
            if !g.ch(pet).is_npc() {
                continue;
            }
            let price = g.ch(pet).level as i32 * 300;
            let mut line = format!("{:>8} - ", price).into_bytes();
            line.extend_from_slice(g.ch(pet).get_name());
            line.extend_from_slice(b"\r\n");
            send_to_char(g, chid, &line);
        }
        true
    } else if cmd_name == b"buy" {
        let (buf, pet_name, _) = two_arguments(arg);

        let found = get_char_room(g, &buf, pet_room);
        let Some(proto_pet) = found.filter(|&p| g.ch(p).is_npc()) else {
            send_to_char(g, chid, b"There is no such pet!\r\n");
            return true;
        };
        let price = g.ch(proto_pet).level as i32 * 300;
        if g.ch(chid).points.gold < price {
            send_to_char(g, chid, b"You don't have enough gold!\r\n");
            return true;
        }
        decrease_gold(g, chid, price);

        let rnum = g.ch(proto_pet).mob_rnum;
        let Some(pet) = crate::db::read_mobile(g, rnum) else {
            return true;
        };
        g.ch_mut(pet).points.exp = 0;
        g.ch_mut(pet).affected_by.set(flags::AFF_CHARM);

        if !pet_name.is_empty() {
            let mut name = g.ch(pet).name.clone().unwrap_or_default();
            name.push(b' ');
            name.extend_from_slice(&pet_name);
            g.ch_mut(pet).name = Some(name);

            let mut desc = g.ch(pet).long_descr.clone().unwrap_or_default();
            desc.extend_from_slice(b"A small sign on a chain around the neck says 'My name is ");
            desc.extend_from_slice(&pet_name);
            desc.extend_from_slice(b"'\r\n");
            g.ch_mut(pet).long_descr = Some(desc);
        }
        let here = g.ch(chid).in_room;
        char_to_room(g, pet, here);
        crate::act::movement::add_follower(g, pet, chid);

        // Be certain that pets can't get/carry/use/wield/wear items.
        {
            let p = g.ch_mut(pet);
            p.carry_weight = 1000;
            p.carry_items = 100;
        }

        send_to_char(g, chid, b"May you enjoy your pet.\r\n");
        act(g, b"$n buys $N as a pet.", false, Some(chid), None, Some(pet), comm::TO_ROOM);
        true
    } else {
        // All commands except list and buy.
        false
    }
}

/// bank — object proc.
fn bank(g: &mut Game, chid: CharId, cmd: usize, arg: &[u8]) -> bool {
    let cmd_name = g.commands[cmd].command.clone();
    if cmd_name == b"balance" {
        let bank = g.ch(chid).points.bank_gold;
        if bank > 0 {
            send_to_char(g, chid, format!("Your current balance is {} coins.\r\n", bank).as_bytes());
        } else {
            send_to_char(g, chid, b"You currently have no money deposited.\r\n");
        }
        true
    } else if cmd_name == b"deposit" {
        let amount = crate::handler::atoi(arg);
        if amount <= 0 {
            send_to_char(g, chid, b"How much do you want to deposit?\r\n");
            return true;
        }
        if g.ch(chid).points.gold < amount {
            send_to_char(g, chid, b"You don't have that many coins!\r\n");
            return true;
        }
        decrease_gold(g, chid, amount);
        increase_bank(g, chid, amount);
        send_to_char(g, chid, format!("You deposit {} coins.\r\n", amount).as_bytes());
        act(g, b"$n makes a bank transaction.", true, Some(chid), None, None, comm::TO_ROOM);
        true
    } else if cmd_name == b"withdraw" {
        let amount = crate::handler::atoi(arg);
        if amount <= 0 {
            send_to_char(g, chid, b"How much do you want to withdraw?\r\n");
            return true;
        }
        if g.ch(chid).points.bank_gold < amount {
            send_to_char(g, chid, b"You don't have that many coins deposited!\r\n");
            return true;
        }
        increase_gold(g, chid, amount);
        decrease_bank(g, chid, amount);
        send_to_char(g, chid, format!("You withdraw {} coins.\r\n", amount).as_bytes());
        act(g, b"$n makes a bank transaction.", true, Some(chid), None, None, comm::TO_ROOM);
        true
    } else {
        false
    }
}
