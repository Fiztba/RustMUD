//! Shopkeeper spec proc, pricing, keeper inventory sorting, and the
//! buy/sell/value/list/identify handlers. Keeper dialogue goes through the
//! real do_say/do_tell/do_echo/do_action commands.

use mud_data::flags;
use mud_data::ids::{CharId, ObjId};
use mud_data::tables;
use mud_data::types::*;

use crate::act::item::an;
use crate::comm::{self, act, cc, send_to_char, C_SPR, KNRM, KYEL};
use crate::game::Game;
use crate::handler::{
    self, can_carry_n, can_carry_w, can_see_obj, extract_obj, get_number, get_obj_in_list_vis,
    isname, obj_from_char, obj_name, obj_short, obj_to_char, obj_weight,
};
use crate::interpreter::{is_number, one_argument, SCMD_EMOTE};
use crate::limits::{decrease_gold, increase_gold};

// Shop flags and trade bits.
pub const WILL_START_FIGHT: u32 = 1 << 0;
pub const WILL_BANK_MONEY: u32 = 1 << 1;
pub const HAS_UNLIMITED_CASH: u32 = 1 << 2;

pub const TRADE_NOGOOD: i32 = 1 << 0;
pub const TRADE_NOEVIL: i32 = 1 << 1;
pub const TRADE_NONEUTRAL: i32 = 1 << 2;
pub const TRADE_NOMAGIC_USER: i32 = 1 << 3;
pub const TRADE_NOCLERIC: i32 = 1 << 4;
pub const TRADE_NOTHIEF: i32 = 1 << 5;
pub const TRADE_NOWARRIOR: i32 = 1 << 6;

pub const MIN_OUTSIDE_BANK: i32 = 5000;
pub const MAX_OUTSIDE_BANK: i32 = 15000;

const MSG_NOT_OPEN_YET: &[u8] = b"Come back later!";
const MSG_NOT_REOPEN_YET: &[u8] = b"Sorry, we have closed, but come back later.";
const MSG_CLOSED_FOR_DAY: &[u8] = b"Sorry, come back tomorrow.";
const MSG_NO_STEAL_HERE: &[u8] = b"$n is a bloody thief!";
const MSG_NO_SEE_CHAR: &[u8] = b"I don't trade with someone I can't see!";
const MSG_NO_SELL_ALIGN: &[u8] = b"Get out of here before I call the guards!";
const MSG_NO_SELL_CLASS: &[u8] = b"We don't serve your kind here!";
const MSG_NO_USED_WANDSTAFF: &[u8] = b"I don't buy used up wands or staves!";

/// Runtime shop state parallel to world.shops: the mutable half of the
/// shop record plus boot-resolved references.
#[derive(Debug, Clone, Default)]
pub struct ShopRt {
    /// Keeper mob rnum (resolved at boot; NOBODY when missing).
    pub keeper: Idx,
    /// Producing obj rnums; unresolvable vnums are dropped at boot.
    pub producing: Vec<Idx>,
    pub bank: i32,
    pub sort: i32,
    /// Displaced spec of the keeper mob (SHOP_FUNC) — called first.
    pub func: Option<crate::spec::MobSpec>,
}

/// Cached command numbers (assign_the_shopkeepers).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShopCmds {
    pub say: usize,
    pub tell: usize,
    pub emote: usize,
    pub slap: usize,
    pub puke: usize,
}

/// assign_shop_command_indices: take the five command numbers the shopkeeper
/// hands to do_action and do_say, plus the one the questmaster uses.
///
/// These are indices into the merged command table, and boot is not the only
/// time that table is built: aedit rebuilds it on every social save. A social
/// added, deleted or renamed anywhere before "slap" in sort order moves every
/// index after it, and a stale one still names *a* command, just the wrong
/// one -- so nothing reports an error and a caught thief gets handed the
/// baker's slippers instead of a slap until the next reboot. Retaken from
/// create_command_list for that reason, as well as from here.
pub fn assign_shop_command_indices(g: &mut Game) {
    g.shop_cmds = ShopCmds {
        say: crate::interpreter::find_command(g, b"say").unwrap_or(0),
        tell: crate::interpreter::find_command(g, b"tell").unwrap_or(0),
        emote: crate::interpreter::find_command(g, b"emote").unwrap_or(0),
        slap: crate::interpreter::find_command(g, b"slap").unwrap_or(0),
        puke: crate::interpreter::find_command(g, b"puke").unwrap_or(0),
    };
}

/// assign_the_shopkeepers: resolve keepers/producing and
/// install the spec, preserving a pre-existing different func as SHOP_FUNC.
pub fn assign_the_shopkeepers(g: &mut Game) {
    assign_shop_command_indices(g);
    for idx in 0..g.world.shops.len() {
        let (keeper_vnum, producing_vnums) = {
            let s = &g.world.shops[idx];
            (s.keeper_vnum, s.producing.clone())
        };
        let keeper = if keeper_vnum < 0 {
            NOBODY
        } else {
            g.world.real_mobile(keeper_vnum as Idx).unwrap_or(NOBODY)
        };
        let producing: Vec<Idx> = producing_vnums
            .iter()
            .filter_map(|&v| if v < 0 { None } else { g.world.real_object(v as Idx) })
            .collect();
        g.shops_rt[idx].keeper = keeper;
        g.shops_rt[idx].producing = producing;
        if keeper == NOBODY {
            continue;
        }
        // Preserve a pre-existing different func as the secondary.
        if let Some(prior) = g.mob_specs[keeper as usize] {
            if prior != crate::spec::MobSpec::ShopKeeper {
                g.shops_rt[idx].func = Some(prior);
            }
        }
        g.mob_specs[keeper as usize] = Some(crate::spec::MobSpec::ShopKeeper);
    }
}

// ---- keeper dialogue helpers ----

fn keeper_tell(g: &mut Game, keeper: CharId, to_name: &[u8], msg: &[u8]) {
    let mut buf = to_name.to_vec();
    buf.push(b' ');
    buf.extend_from_slice(msg);
    let cmd = g.shop_cmds.tell;
    crate::act::comm::do_tell(g, keeper, &buf, cmd, 0);
}

fn keeper_say(g: &mut Game, keeper: CharId, msg: &[u8]) {
    // cmd_tell is passed here (a harmless quirk; do_say ignores it).
    let cmd = g.shop_cmds.tell;
    crate::act::comm::do_say(g, keeper, msg, cmd, 0);
}

/// Format one of the seven shop message templates: %s = name, %d = amount,
/// %% = literal %. A missing (validation-dropped) template logs and yields
/// an empty message rather than formatting a missing string (F2 class).
fn shop_msg(g: &mut Game, shop_idx: usize, which: &str, name: &[u8], amount: i32) -> Vec<u8> {
    let template = {
        let s = &g.world.shops[shop_idx];
        match which {
            "no_such_item1" => s.no_such_item1.clone(),
            "no_such_item2" => s.no_such_item2.clone(),
            "do_not_buy" => s.do_not_buy.clone(),
            "missing_cash1" => s.missing_cash1.clone(),
            "missing_cash2" => s.missing_cash2.clone(),
            "message_buy" => s.message_buy.clone(),
            "message_sell" => s.message_sell.clone(),
            _ => None,
        }
    };
    let Some(template) = template else {
        g.log(format!(
            "SYSERR: shop #{} has no valid '{}' message.",
            g.world.shops[shop_idx].vnum, which
        ));
        return Vec::new();
    };
    let mut out = Vec::with_capacity(template.len() + name.len() + 12);
    let mut i = 0;
    while i < template.len() {
        if template[i] == b'%' && i + 1 < template.len() {
            match template[i + 1] {
                b's' => {
                    out.extend_from_slice(name);
                    i += 2;
                }
                b'd' => {
                    out.extend_from_slice(format!("{}", amount).as_bytes());
                    i += 2;
                }
                b'%' => {
                    out.push(b'%');
                    i += 2;
                }
                _ => {
                    out.push(template[i]);
                    i += 1;
                }
            }
        } else {
            out.push(template[i]);
            i += 1;
        }
    }
    out
}

// ---- gates ----

/// IS_GOD for shops.
fn shop_is_god(g: &Game, chid: CharId) -> bool {
    let ch = g.ch(chid);
    !ch.is_npc() && ch.level >= LVL_GOD
}

fn is_ok_char(g: &mut Game, keeper: CharId, chid: CharId, shop_idx: usize) -> bool {
    if !handler::can_see(g, keeper, chid) {
        keeper_say(g, keeper, MSG_NO_SEE_CHAR);
        return false;
    }
    if shop_is_god(g, chid) {
        return true;
    }
    let name = g.ch(chid).get_name().to_vec();
    let with_who = g.world.shops[shop_idx].with_who;
    let align = g.ch(chid).alignment;
    let is_good = align >= 350;
    let is_evil = align <= -350;
    let is_neutral = !is_good && !is_evil;
    if (is_good && with_who & TRADE_NOGOOD != 0)
        || (is_evil && with_who & TRADE_NOEVIL != 0)
        || (is_neutral && with_who & TRADE_NONEUTRAL != 0)
    {
        keeper_tell(g, keeper, &name, MSG_NO_SELL_ALIGN);
        return false;
    }
    if g.ch(chid).is_npc() {
        return true;
    }
    let class = g.ch(chid).class;
    if (class == CLASS_MAGIC_USER && with_who & TRADE_NOMAGIC_USER != 0)
        || (class == CLASS_CLERIC && with_who & TRADE_NOCLERIC != 0)
        || (class == CLASS_THIEF && with_who & TRADE_NOTHIEF != 0)
        || (class == CLASS_WARRIOR && with_who & TRADE_NOWARRIOR != 0)
    {
        keeper_tell(g, keeper, &name, MSG_NO_SELL_CLASS);
        return false;
    }
    true
}

fn is_open(g: &mut Game, keeper: CharId, shop_idx: usize, msg: bool) -> bool {
    let hours = g.time_info.hours as i32;
    let s = &g.world.shops[shop_idx];
    let buf: &[u8] = if s.open1 > hours {
        MSG_NOT_OPEN_YET
    } else if s.close1 < hours {
        if s.open2 > hours {
            MSG_NOT_REOPEN_YET
        } else if s.close2 < hours {
            MSG_CLOSED_FOR_DAY
        } else {
            b""
        }
    } else {
        b""
    };
    if buf.is_empty() {
        return true;
    }
    if msg {
        keeper_say(g, keeper, buf);
    }
    false
}

fn is_ok(g: &mut Game, keeper: CharId, chid: CharId, shop_idx: usize) -> bool {
    if is_open(g, keeper, shop_idx, true) {
        is_ok_char(g, keeper, chid, shop_idx)
    } else {
        false
    }
}

// BUY-keyword expression language ----

const OPER_OPEN_PAREN: i32 = 0;
const OPER_CLOSE_PAREN: i32 = 1;
const OPER_OR: i32 = 2;
const OPER_AND: i32 = 3;
const OPER_NOT: i32 = 4;
const MAX_OPER: i32 = 4;

fn find_oper_num(token: u8) -> Option<i32> {
    const OPERATOR_STR: [&[u8]; 5] = [b"[({", b"])}", b"|+", b"&*", b"^'"];
    (0..=MAX_OPER).find(|&i| OPERATOR_STR[i as usize].contains(&token))
}

fn evaluate_operation(g: &mut Game, ops: &mut Vec<i32>, vals: &mut Vec<i32>) {
    let pop = |g: &mut Game, st: &mut Vec<i32>| -> i32 {
        match st.pop() {
            Some(v) => v,
            None => {
                g.log(format!("SYSERR: Illegal expression {} in shop keyword list.", st.len()));
                0
            }
        }
    };
    let oper = pop(g, ops);
    if oper == OPER_NOT {
        let v = pop(g, vals);
        vals.push((v == 0) as i32);
    } else {
        let val1 = pop(g, vals);
        let val2 = pop(g, vals);
        if oper == OPER_AND {
            vals.push((val1 != 0 && val2 != 0) as i32);
        } else if oper == OPER_OR {
            vals.push((val1 != 0 || val2 != 0) as i32);
        }
    }
}

fn evaluate_expression(g: &mut Game, oid: ObjId, expr: Option<&[u8]>) -> bool {
    let Some(expr) = expr else { return true };
    if expr.is_empty() {
        return true;
    }
    let expr = expr.to_vec();
    let mut ops: Vec<i32> = Vec::new();
    let mut vals: Vec<i32> = Vec::new();
    let mut ptr = 0usize;
    while ptr < expr.len() {
        if expr[ptr].is_ascii_whitespace() {
            ptr += 1;
            continue;
        }
        match find_oper_num(expr[ptr]) {
            None => {
                let end = ptr;
                while ptr < expr.len() && !expr[ptr].is_ascii_whitespace() && find_oper_num(expr[ptr]).is_none() {
                    ptr += 1;
                }
                let name = &expr[end..ptr];
                let mut pushed = false;
                for (eindex, bit) in tables::EXTRA_BITS.iter().enumerate() {
                    if name.eq_ignore_ascii_case(bit.as_bytes()) {
                        vals.push(g.obj(oid).extra_flags.is_set(eindex) as i32);
                        pushed = true;
                        break;
                    }
                }
                if !pushed {
                    let v = isname(name, obj_name(g, oid));
                    vals.push(v as i32);
                }
            }
            Some(temp) => {
                if temp != OPER_OPEN_PAREN {
                    while ops.last().copied().unwrap_or(-1) > temp {
                        evaluate_operation(g, &mut ops, &mut vals);
                    }
                }
                if temp == OPER_CLOSE_PAREN {
                    if ops.pop() != Some(OPER_OPEN_PAREN) {
                        g.log("SYSERR: Illegal parenthesis in shop keyword expression.".to_string());
                        return false;
                    }
                } else {
                    ops.push(temp);
                }
                ptr += 1;
            }
        }
    }
    while ops.last().copied().unwrap_or(-1) != -1 && !ops.is_empty() {
        evaluate_operation(g, &mut ops, &mut vals);
    }
    let temp = match vals.pop() {
        Some(v) => v,
        None => {
            g.log("SYSERR: Illegal expression 0 in shop keyword list.".to_string());
            0
        }
    };
    if !vals.is_empty() {
        g.log("SYSERR: Extra operands left on shop keyword expression stack.".to_string());
        return false;
    }
    temp != 0
}

// ---- trade rules ----

#[derive(PartialEq)]
enum TradeResult {
    Ok,
    NoVal,
    NotOk,
    Dead,
}

fn trade_with(g: &mut Game, oid: ObjId, shop_idx: usize) -> TradeResult {
    if g.obj(oid).cost < 1 {
        return TradeResult::NoVal;
    }
    if g.obj(oid).obj_flagged(flags::ITEM_NOSELL) {
        return TradeResult::NotOk;
    }
    let types: Vec<(i32, Option<Vec<u8>>)> = g.world.shops[shop_idx]
        .type_list
        .iter()
        .map(|t| (t.type_, t.keywords.clone()))
        .collect();
    for (btype, keywords) in types {
        if btype == g.obj(oid).type_flag {
            let tf = g.obj(oid).type_flag;
            if g.obj(oid).values[2] == 0 && (tf == flags::ITEM_WAND || tf == flags::ITEM_STAFF) {
                return TradeResult::Dead;
            } else if evaluate_expression(g, oid, keywords.as_deref()) {
                return TradeResult::Ok;
            }
        }
    }
    TradeResult::NotOk
}

/// same_obj between two instances.
pub fn same_obj(g: &Game, a: ObjId, b: ObjId) -> bool {
    let (o1, o2) = (g.obj(a), g.obj(b));
    if o1.item_number != o2.item_number || o1.cost != o2.cost {
        return false;
    }
    for i in 0..MAX_OBJ_AFFECT {
        if o1.affected[i].location != o2.affected[i].location
            || o1.affected[i].modifier != o2.affected[i].modifier
        {
            return false;
        }
    }
    true
}

/// same_obj against a prototype (shop_producing's proto comparison).
fn same_as_proto(g: &Game, oid: ObjId, rnum: Idx) -> bool {
    let o = g.obj(oid);
    if o.item_number != rnum {
        return false;
    }
    let Some(proto) = g.world.obj_protos.get(rnum as usize) else {
        return false;
    };
    if o.cost != proto.cost {
        return false;
    }
    for i in 0..MAX_OBJ_AFFECT {
        if o.affected[i].location != proto.affected[i].location
            || o.affected[i].modifier != proto.affected[i].modifier
        {
            return false;
        }
    }
    true
}

fn shop_producing(g: &Game, oid: ObjId, shop_idx: usize) -> bool {
    if g.obj(oid).item_number == NOTHING {
        return false;
    }
    g.shops_rt[shop_idx].producing.iter().any(|&r| same_as_proto(g, oid, r))
}

/// transaction_amt: returns (quantity, remaining item spec).
fn transaction_amt(arg: &[u8]) -> (i32, Vec<u8>) {
    let (buf, buywhat) = one_argument(arg);
    if !buywhat.is_empty() && !buf.is_empty() && is_number(&buf) {
        // Strip "N " off the front of arg.
        let rest = crate::interpreter::skip_spaces(buywhat).to_vec();
        return (handler::atoi(&buf), rest);
    }
    (1, arg.to_vec())
}

fn times_message(g: &Game, oid: Option<ObjId>, name: &[u8], num: i32) -> Vec<u8> {
    let mut buf = match oid {
        Some(o) => obj_short(g, o).to_vec(),
        None => {
            let ptr = match name.iter().position(|&c| c == b'.') {
                Some(p) => &name[p + 1..],
                None => name,
            };
            let mut b = an(ptr).to_vec();
            b.push(b' ');
            b.extend_from_slice(ptr);
            b
        }
    };
    if num > 1 {
        buf.extend_from_slice(format!(" (x {})", num).as_bytes());
    }
    buf
}

/// get_slide_obj_vis: visible match skipping items
/// the same as the previous match — adjacent-run dedup.
fn get_slide_obj_vis(g: &Game, chid: CharId, name: &[u8], list: &[ObjId]) -> Option<ObjId> {
    let (number, tmp) = get_number(name);
    if number == 0 {
        return None;
    }
    let mut last_match: Option<ObjId> = None;
    let mut j = 1;
    for &i in list {
        if j > number {
            break;
        }
        if isname(&tmp, obj_name(g, i)) && can_see_obj(g, chid, i) {
            let same_as_last = last_match.is_some_and(|lm| same_obj(g, lm, i));
            if !same_as_last {
                if j == number {
                    return Some(i);
                }
                last_match = Some(i);
                j += 1;
            }
        }
    }
    None
}

/// get_hash_obj_vis: the "#n"/index lookup.
fn get_hash_obj_vis(g: &Game, chid: CharId, name: &[u8], list: &[ObjId]) -> Option<ObjId> {
    let qindex = if is_number(name) {
        handler::atoi(name)
    } else if name.len() > 1 && is_number(&name[1..]) {
        handler::atoi(&name[1..])
    } else {
        return None;
    };
    let mut qindex = qindex;
    let mut last_obj: Option<ObjId> = None;
    for &loop_ in list {
        if can_see_obj(g, chid, loop_) && g.obj(loop_).cost > 0 {
            let same_as_last = last_obj.is_some_and(|lo| same_obj(g, lo, loop_));
            if !same_as_last {
                qindex -= 1;
                if qindex == 0 {
                    return Some(loop_);
                }
                last_obj = Some(loop_);
            }
        }
    }
    None
}

fn get_purchase_obj(
    g: &mut Game,
    chid: CharId,
    arg: &[u8],
    keeper: CharId,
    shop_idx: usize,
    msg: bool,
) -> Option<ObjId> {
    let (name, _) = one_argument(arg);
    loop {
        let list = g.ch(keeper).carrying.clone();
        let obj = if name.first() == Some(&b'#') || is_number(&name) {
            get_hash_obj_vis(g, chid, &name, &list)
        } else {
            get_slide_obj_vis(g, chid, &name, &list)
        };
        let Some(obj) = obj else {
            if msg {
                let chname = g.ch(chid).get_name().to_vec();
                let buf = shop_msg(g, shop_idx, "no_such_item1", &chname, 0);
                keeper_tell_raw(g, keeper, &buf);
            }
            return None;
        };
        if g.obj(obj).cost <= 0 {
            extract_obj(g, obj);
            continue;
        }
        return Some(obj);
    }
}

/// Keeper tell where the message already includes the target name.
fn keeper_tell_raw(g: &mut Game, keeper: CharId, buf: &[u8]) {
    let cmd = g.shop_cmds.tell;
    crate::act::comm::do_tell(g, keeper, buf, cmd, 0);
}

fn buy_price(g: &Game, oid: ObjId, shop_idx: usize, keeper: CharId, buyer: CharId) -> i32 {
    let cost = g.obj(oid).cost as f32;
    let profit = g.world.shops[shop_idx].profit_buy;
    let cha_k = g.ch(keeper).aff_abils.cha as f32;
    let cha_b = g.ch(buyer).aff_abils.cha as f32;
    (cost * profit * (1.0 + (cha_k - cha_b) / 70.0f32)) as i32
}

fn sell_price(g: &Game, oid: ObjId, shop_idx: usize, keeper: CharId, seller: CharId) -> i32 {
    let cost = g.obj(oid).cost as f32;
    let cha_k = g.ch(keeper).aff_abils.cha as f32;
    let cha_s = g.ch(seller).aff_abils.cha as f32;
    let mut sell_mod = g.world.shops[shop_idx].profit_sell * (1.0 - (cha_k - cha_s) / 70.0f32);
    let buy_mod = g.world.shops[shop_idx].profit_buy * (1.0 + (cha_k - cha_s) / 70.0f32);
    if sell_mod > buy_mod {
        sell_mod = buy_mod;
    }
    (cost * sell_mod) as i32
}

// ---- keeper inventory bookkeeping ----

/// slide_obj: insert next to an identical object, or merge
/// into infinite stock (extraction) when the shop produces it.
fn slide_obj(g: &mut Game, oid: ObjId, keeper: CharId, shop_idx: usize) {
    if g.shops_rt[shop_idx].sort < g.ch(keeper).carry_items as i32 {
        sort_keeper_objs(g, keeper, shop_idx);
    }
    if shop_producing(g, oid, shop_idx) {
        extract_obj(g, oid);
        return;
    }
    g.shops_rt[shop_idx].sort += 1;
    obj_to_char(g, oid, keeper);
    // Reposition: after the first identical object below the head.
    let carrying = g.ch(keeper).carrying.clone();
    for (i, &other) in carrying.iter().enumerate().skip(1) {
        if same_obj(g, oid, other) {
            let ch = g.ch_mut(keeper);
            ch.carrying.remove(0);
            ch.carrying.insert(i, oid);
            return;
        }
    }
    // No match: stays at the head.
}

fn sort_keeper_objs(g: &mut Game, keeper: CharId, shop_idx: usize) {
    let mut list: Vec<ObjId> = Vec::new();
    while g.shops_rt[shop_idx].sort < g.ch(keeper).carry_items as i32 {
        let Some(&head) = g.ch(keeper).carrying.first() else { break };
        obj_from_char(g, head);
        list.insert(0, head);
    }
    for temp in list {
        let produced = shop_producing(g, temp, shop_idx);
        let rnum = g.obj(temp).item_number;
        let already_stocked = g
            .ch(keeper)
            .carrying
            .iter()
            .any(|&o| g.obj(o).item_number == rnum);
        if produced && !already_stocked {
            obj_to_char(g, temp, keeper);
            g.shops_rt[shop_idx].sort += 1;
        } else {
            slide_obj(g, temp, keeper, shop_idx);
        }
    }
}

// ---- the five transaction handlers ----

fn shopping_buy(g: &mut Game, arg: &[u8], chid: CharId, keeper: CharId, shop_idx: usize) {
    if !is_ok(g, keeper, chid, shop_idx) {
        return;
    }
    if g.shops_rt[shop_idx].sort < g.ch(keeper).carry_items as i32 {
        sort_keeper_objs(g, keeper, shop_idx);
    }
    let chname = g.ch(chid).get_name().to_vec();
    let (buynum, arg) = transaction_amt(arg);
    if buynum < 0 {
        let mut buf = chname.clone();
        buf.extend_from_slice(b" A negative amount?  Try selling me something.");
        keeper_tell_raw(g, keeper, &buf);
        return;
    }
    if arg.is_empty() || buynum == 0 {
        let mut buf = chname.clone();
        buf.extend_from_slice(b" What do you want to buy??");
        keeper_tell_raw(g, keeper, &buf);
        return;
    }
    let Some(obj) = get_purchase_obj(g, chid, &arg, keeper, shop_idx, true) else {
        return;
    };

    let is_god = shop_is_god(g, chid);
    let quest_item = g.obj(obj).obj_flagged(flags::ITEM_QUEST);
    if quest_item {
        if g.obj(obj).cost > g.ch(chid).ps().questpoints && !is_god {
            let mut buf = chname.clone();
            buf.extend_from_slice(b" You haven't earned enough quest points for such an item.");
            keeper_tell_raw(g, keeper, &buf);
            return;
        }
    } else if buy_price(g, obj, shop_idx, keeper, chid) > g.ch(chid).points.gold && !is_god {
        let buf = shop_msg(g, shop_idx, "missing_cash2", &chname, 0);
        keeper_tell_raw(g, keeper, &buf);

        match g.world.shops[shop_idx].temper1 {
            0 => {
                let (cmd, name) = (g.shop_cmds.puke, chname.clone());
                crate::act::social::do_action(g, keeper, &name, cmd, 0);
                return;
            }
            1 => {
                let cmd = g.shop_cmds.emote;
                crate::act::other::do_echo(g, keeper, b"smokes on his joint.", cmd, SCMD_EMOTE);
                return;
            }
            _ => return,
        }
    }

    {
        let ch = g.ch(chid);
        if ch.is_npc() || !ch.prf(flags::PRF_NOHASSLE) {
            if ch.carry_items as i32 + 1 > can_carry_n(ch) {
                let f = handler::fname(obj_name(g, obj));
                let mut msg = f;
                msg.extend_from_slice(b": You can't carry any more items.\r\n");
                send_to_char(g, chid, &msg);
                return;
            }
            if ch.carry_weight + obj_weight(g, obj) > can_carry_w(ch) {
                let f = handler::fname(obj_name(g, obj));
                let mut msg = f;
                msg.extend_from_slice(b": You can't carry that much weight.\r\n");
                send_to_char(g, chid, &msg);
                return;
            }
        }
    }

    let mut bought = 0;
    let mut goldamt = 0;
    let mut last_obj: Option<ObjId> = None;
    let mut cur: Option<ObjId> = Some(obj);
    loop {
        let Some(o) = cur else { break };
        let ch = g.ch(chid);
        let funds_ok = if quest_item {
            ch.ps().questpoints >= g.obj(o).cost || is_god
        } else {
            ch.points.gold >= buy_price(g, o, shop_idx, keeper, chid) || is_god
        };
        if !(funds_ok
            && (ch.carry_items as i32) < can_carry_n(ch)
            && bought < buynum
            && ch.carry_weight + obj_weight(g, o) <= can_carry_w(ch))
        {
            break;
        }
        bought += 1;
        let bought_obj = if shop_producing(g, o, shop_idx) {
            crate::db::read_object(g, g.obj(o).item_number).unwrap_or(o)
        } else {
            obj_from_char(g, o);
            g.shops_rt[shop_idx].sort -= 1;
            o
        };
        obj_to_char(g, bought_obj, chid);

        if quest_item {
            let cost = g.obj(bought_obj).cost;
            goldamt += cost;
            if !is_god {
                g.ch_mut(chid).ps_mut().questpoints -= cost;
            }
        } else {
            let charged = buy_price(g, bought_obj, shop_idx, keeper, chid);
            goldamt += charged;
            if !is_god {
                decrease_gold(g, chid, charged);
            }
        }

        last_obj = Some(bought_obj);
        cur = get_purchase_obj(g, chid, &arg, keeper, shop_idx, false);
        if let (Some(c), Some(l)) = (cur, last_obj) {
            if !same_obj(g, c, l) {
                break;
            }
        } else if cur.is_none() {
            break;
        }
    }

    if bought < buynum {
        let same = match (cur, last_obj) {
            (Some(c), Some(l)) => same_obj(g, c, l),
            _ => false,
        };
        let ch = g.ch(chid);
        let mut buf = chname.clone();
        if cur.is_none() || !same {
            buf.extend_from_slice(format!(" I only have {} to sell you.", bought).as_bytes());
        } else if !quest_item && ch.points.gold < buy_price(g, cur.unwrap(), shop_idx, keeper, chid) {
            buf.extend_from_slice(format!(" You can only afford {}.", bought).as_bytes());
        } else if quest_item && ch.ps().questpoints < g.obj(cur.unwrap()).cost {
            buf.extend_from_slice(
                format!(" You only had sufficient quest points for {}.", bought).as_bytes(),
            );
        } else if ch.carry_items as i32 >= can_carry_n(ch) {
            buf.extend_from_slice(format!(" You can only hold {}.", bought).as_bytes());
        } else if ch.carry_weight + obj_weight(g, cur.unwrap()) > can_carry_w(ch) {
            buf.extend_from_slice(format!(" You can only carry {}.", bought).as_bytes());
        } else {
            buf.extend_from_slice(format!(" Something screwy only gave you {}.", bought).as_bytes());
        }
        keeper_tell_raw(g, keeper, &buf);
    }
    // The keeper is paid whenever the BOUGHT items were not quest purchases.
    // Keying this (and the quest tell below) on the post-loop lookahead
    // `obj` would swallow the payment when the buyer took
    // the last item in stock. `quest_item` is captured from the first
    // purchased object, so there is no empty case to guard here.
    let (pay, quest_tell) = (!quest_item, quest_item);
    if !is_god && pay {
        increase_gold(g, keeper, goldamt);
        if g.world.shops[shop_idx].bitvector & WILL_BANK_MONEY != 0
            && g.ch(keeper).points.gold > MAX_OUTSIDE_BANK
        {
            let excess = g.ch(keeper).points.gold - MAX_OUTSIDE_BANK;
            g.shops_rt[shop_idx].bank += excess;
            g.ch_mut(keeper).points.gold = MAX_OUTSIDE_BANK;
        }
    }
    let head = g.ch(chid).carrying.first().copied();
    let tempstr = times_message(g, head, b"", bought);

    let mut tempbuf = b"$n buys ".to_vec();
    tempbuf.extend_from_slice(&tempstr);
    tempbuf.push(b'.');
    act(g, &tempbuf, false, Some(chid), None, None, comm::TO_ROOM);

    let tellbuf = if quest_tell {
        let mut b = chname.clone();
        b.extend_from_slice(format!(" That has cost you {} quest points.", goldamt).as_bytes());
        b
    } else {
        shop_msg(g, shop_idx, "message_buy", &chname, goldamt)
    };
    keeper_tell_raw(g, keeper, &tellbuf);

    let mut msg = b"You now have ".to_vec();
    msg.extend_from_slice(&tempstr);
    msg.extend_from_slice(b".\r\n");
    send_to_char(g, chid, &msg);
}

fn get_selling_obj(
    g: &mut Game,
    chid: CharId,
    name: &[u8],
    keeper: CharId,
    shop_idx: usize,
    msg: bool,
) -> Option<ObjId> {
    let carrying = g.ch(chid).carrying.clone();
    let Some(obj) = get_obj_in_list_vis(g, chid, name, None, &carrying) else {
        if msg {
            let chname = g.ch(chid).get_name().to_vec();
            let buf = shop_msg(g, shop_idx, "no_such_item2", &chname, 0);
            keeper_tell_raw(g, keeper, &buf);
        }
        return None;
    };
    let result = trade_with(g, obj, shop_idx);
    if result == TradeResult::Ok {
        return Some(obj);
    }
    if !msg {
        return None;
    }
    let chname = g.ch(chid).get_name().to_vec();
    let buf = match result {
        TradeResult::NoVal => {
            let mut b = chname.clone();
            b.extend_from_slice(b" You've got to be kidding, that thing is worthless!");
            b
        }
        TradeResult::NotOk => shop_msg(g, shop_idx, "do_not_buy", &chname, 0),
        TradeResult::Dead => {
            let mut b = chname.clone();
            b.push(b' ');
            b.extend_from_slice(MSG_NO_USED_WANDSTAFF);
            b
        }
        TradeResult::Ok => unreachable!(),
    };
    keeper_tell_raw(g, keeper, &buf);
    None
}

fn shopping_sell(g: &mut Game, arg: &[u8], chid: CharId, keeper: CharId, shop_idx: usize) {
    if !is_ok(g, keeper, chid, shop_idx) {
        return;
    }
    let chname = g.ch(chid).get_name().to_vec();
    let (sellnum, arg) = transaction_amt(arg);
    if sellnum < 0 {
        let mut buf = chname.clone();
        buf.extend_from_slice(b" A negative amount?  Try buying something.");
        keeper_tell_raw(g, keeper, &buf);
        return;
    }
    if arg.is_empty() || sellnum == 0 {
        let mut buf = chname.clone();
        buf.extend_from_slice(b" What do you want to sell??");
        keeper_tell_raw(g, keeper, &buf);
        return;
    }
    let (name, _) = one_argument(&arg);
    let Some(first) = get_selling_obj(g, chid, &name, keeper, shop_idx, true) else {
        return;
    };
    let unlimited_cash = g.world.shops[shop_idx].bitvector & HAS_UNLIMITED_CASH != 0;
    if !unlimited_cash
        && g.ch(keeper).points.gold + g.shops_rt[shop_idx].bank
            < sell_price(g, first, shop_idx, keeper, chid)
    {
        let buf = shop_msg(g, shop_idx, "missing_cash1", &chname, 0);
        keeper_tell_raw(g, keeper, &buf);
        return;
    }

    let mut sold = 0;
    let mut goldamt = 0;
    let mut cur = Some(first);
    while let Some(o) = cur {
        if !(unlimited_cash
            || g.ch(keeper).points.gold + g.shops_rt[shop_idx].bank >= sell_price(g, o, shop_idx, keeper, chid))
            || sold >= sellnum
        {
            break;
        }
        let charged = sell_price(g, o, shop_idx, keeper, chid);
        goldamt += charged;
        if !unlimited_cash {
            decrease_gold(g, keeper, charged);
        }
        sold += 1;
        obj_from_char(g, o);
        slide_obj(g, o, keeper, shop_idx); // Seems we don't use return value.
        cur = get_selling_obj(g, chid, &name, keeper, shop_idx, false);
    }

    if sold < sellnum {
        let mut buf = chname.clone();
        if cur.is_none() {
            buf.extend_from_slice(format!(" You only have {} of those.", sold).as_bytes());
        } else if g.ch(keeper).points.gold + g.shops_rt[shop_idx].bank
            < sell_price(g, cur.unwrap(), shop_idx, keeper, chid)
        {
            buf.extend_from_slice(format!(" I can only afford to buy {} of those.", sold).as_bytes());
        } else {
            buf.extend_from_slice(format!(" Something really screwy made me buy {}.", sold).as_bytes());
        }
        keeper_tell_raw(g, keeper, &buf);
    }
    increase_gold(g, chid, goldamt);

    let tempstr = times_message(g, None, &name, sold);
    let mut tempbuf = b"$n sells ".to_vec();
    tempbuf.extend_from_slice(&tempstr);
    tempbuf.push(b'.');
    act(g, &tempbuf, false, Some(chid), None, None, comm::TO_ROOM);

    let buf = shop_msg(g, shop_idx, "message_sell", &chname, goldamt);
    keeper_tell_raw(g, keeper, &buf);

    let mut msg = b"The shopkeeper now has ".to_vec();
    msg.extend_from_slice(&tempstr);
    msg.extend_from_slice(b".\r\n");
    send_to_char(g, chid, &msg);

    if g.ch(keeper).points.gold < MIN_OUTSIDE_BANK {
        let refill = (MAX_OUTSIDE_BANK - g.ch(keeper).points.gold).min(g.shops_rt[shop_idx].bank);
        g.shops_rt[shop_idx].bank -= refill;
        increase_gold(g, keeper, refill);
    }
}

fn shopping_value(g: &mut Game, arg: &[u8], chid: CharId, keeper: CharId, shop_idx: usize) {
    if !is_ok(g, keeper, chid, shop_idx) {
        return;
    }
    let chname = g.ch(chid).get_name().to_vec();
    if arg.is_empty() {
        let mut buf = chname.clone();
        buf.extend_from_slice(b" What do you want me to evaluate??");
        keeper_tell_raw(g, keeper, &buf);
        return;
    }
    let (name, _) = one_argument(arg);
    let Some(obj) = get_selling_obj(g, chid, &name, keeper, shop_idx, true) else {
        return;
    };
    let price = sell_price(g, obj, shop_idx, keeper, chid);
    let mut buf = chname.clone();
    buf.extend_from_slice(format!(" I'll give you {} gold coins for that!", price).as_bytes());
    keeper_tell_raw(g, keeper, &buf);
}

fn count_color_chars(s: &[u8]) -> usize {
    let mut num = 0;
    let mut i = 0;
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

fn list_object(
    g: &mut Game,
    oid: ObjId,
    cnt: i32,
    aindex: i32,
    shop_idx: usize,
    keeper: CharId,
    chid: CharId,
) -> Vec<u8> {
    let quantity = if shop_producing(g, oid, shop_idx) {
        "Unlimited".to_string()
    } else {
        format!("{}", cnt)
    };
    let short = obj_short(g, oid).to_vec();
    let mut itemname: Vec<u8> = match g.obj(oid).type_flag {
        t if t == flags::ITEM_DRINKCON => {
            if g.obj(oid).values[1] != 0 {
                let liq = g.obj(oid).values[2].clamp(0, 15) as usize;
                let mut b = short.clone();
                b.extend_from_slice(b" of ");
                b.extend_from_slice(tables::DRINKS[liq].as_bytes());
                b
            } else {
                short.clone()
            }
        }
        t if t == flags::ITEM_WAND || t == flags::ITEM_STAFF => {
            let mut b = short.clone();
            if g.obj(oid).values[2] < g.obj(oid).values[1] {
                b.extend_from_slice(b" (partially used)");
            }
            b
        }
        _ => short.clone(),
    };
    comm::cap(&mut itemname);

    // " %2d) %9s %-*s %6d%s\r\n" with * = count_color_chars + 48.
    let width = count_color_chars(&itemname) + 48;
    let price = buy_price(g, oid, shop_idx, keeper, chid);
    let qp = if g.obj(oid).obj_flagged(flags::ITEM_QUEST) { " qp" } else { "" };
    let mut line = format!(" {:>2})  {:>9}   ", aindex, quantity).into_bytes();
    let mut padded = itemname.clone();
    while padded.len() < width {
        padded.push(b' ');
    }
    line.extend_from_slice(&padded);
    line.extend_from_slice(format!(" {:>6}{}\r\n", price, qp).as_bytes());
    line
}

fn shopping_list(g: &mut Game, arg: &[u8], chid: CharId, keeper: CharId, shop_idx: usize) {
    if !is_ok(g, keeper, chid, shop_idx) {
        return;
    }
    if g.shops_rt[shop_idx].sort < g.ch(keeper).carry_items as i32 {
        sort_keeper_objs(g, keeper, shop_idx);
    }
    let (name, _) = one_argument(arg);

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(
        b" ##   Available   Item                                               Cost\r\n\
----------------------------------------------------------------------------\r\n",
    );
    let mut last_obj: Option<ObjId> = None;
    let mut cnt = 0;
    let mut lindex = 0;
    let mut found = false;
    let mut has_quest = false;
    let carrying = g.ch(keeper).carrying.clone();
    for obj in carrying {
        if can_see_obj(g, chid, obj) && g.obj(obj).cost > 0 {
            match last_obj {
                None => {
                    last_obj = Some(obj);
                    cnt = 1;
                }
                Some(lo) if same_obj(g, lo, obj) => cnt += 1,
                Some(lo) => {
                    lindex += 1;
                    if name.is_empty() || isname(&name, obj_name(g, lo)) {
                        let line = list_object(g, lo, cnt, lindex, shop_idx, keeper, chid);
                        buf.extend_from_slice(&line);
                        found = true;
                        if g.obj(lo).obj_flagged(flags::ITEM_QUEST) {
                            has_quest = true;
                        }
                    }
                    cnt = 1;
                    last_obj = Some(obj);
                }
            }
        }
    }
    lindex += 1;
    match last_obj {
        None => send_to_char(g, chid, b"Currently, there is nothing for sale.\r\n"),
        Some(_) if !name.is_empty() && !found => {
            send_to_char(g, chid, b"Presently, none of those are for sale.\r\n")
        }
        Some(lo) => {
            if name.is_empty() || isname(&name, obj_name(g, lo)) {
                let line = list_object(g, lo, cnt, lindex, shop_idx, keeper, chid);
                buf.extend_from_slice(&line);
                if g.obj(lo).obj_flagged(flags::ITEM_QUEST) {
                    has_quest = true;
                }
            }
            crate::act::informative::page_string(g, chid, &buf);
            if has_quest {
                send_to_char(g, chid, b"Items flagged \"qp\" require quest points to purchase.\r\n");
            }
        }
    }
}

fn shopping_identify(g: &mut Game, arg: &[u8], chid: CharId, keeper: CharId, shop_idx: usize) -> bool {
    if !is_ok(g, keeper, chid, shop_idx) {
        return false;
    }
    if g.shops_rt[shop_idx].sort < g.ch(keeper).carry_items as i32 {
        sort_keeper_objs(g, keeper, shop_idx);
    }
    if arg.is_empty() {
        let chname = g.ch(chid).get_name().to_vec();
        let mut buf = chname;
        buf.extend_from_slice(b" What do you want to identify??");
        keeper_tell_raw(g, keeper, &buf);
        return true;
    }
    let Some(obj) = get_purchase_obj(g, chid, arg, keeper, shop_idx, true) else {
        return false;
    };

    {
        let mut msg = b"Name: ".to_vec();
        let short = obj_short(g, obj);
        msg.extend_from_slice(if short.is_empty() { b"<None>" } else { short });
        msg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &msg);
    }
    {
        let t = g.obj(obj).type_flag;
        let tname = tables::ITEM_TYPES.get(t as usize).copied().unwrap_or("UNDEFINED");
        send_to_char(g, chid, format!("Type: {}\r\n", tname).as_bytes());
    }
    {
        let weight = obj_weight(g, obj);
        let sellp = sell_price(g, obj, shop_idx, keeper, chid);
        let buyp = buy_price(g, obj, shop_idx, keeper, chid);
        let qyel = cc(g, chid, C_SPR, KYEL);
        let qnrm = cc(g, chid, C_SPR, KNRM);
        let mut msg = format!("Weight: {}, Cost to Sell: ", weight).into_bytes();
        msg.extend_from_slice(qyel);
        msg.extend_from_slice(format!("{}", sellp).as_bytes());
        msg.extend_from_slice(qnrm);
        msg.extend_from_slice(b", Cost to Buy: ");
        msg.extend_from_slice(qyel);
        msg.extend_from_slice(format!("{}", buyp).as_bytes());
        msg.extend_from_slice(qnrm);
        msg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &msg);
    }
    {
        let mut flagbuf: Vec<u8> = Vec::new();
        let wear_words = g.obj(obj).wear_flags.0;
        crate::act::informative::sprintbitarray(&wear_words, &tables::WEAR_BITS, &mut flagbuf);
        let mut msg = b"Can be worn on: ".to_vec();
        msg.extend_from_slice(&flagbuf);
        msg.extend_from_slice(b"\r\n");
        send_to_char(g, chid, &msg);
    }

    let vals = g.obj(obj).values;
    let t = g.obj(obj).type_flag;
    if t == flags::ITEM_LIGHT {
        if vals[2] == -1 {
            send_to_char(g, chid, b"Hours Remaining: (Infinite)\r\n");
        } else if vals[2] == 0 {
            send_to_char(g, chid, b"Hours Remaining: None!\r\n");
        } else {
            send_to_char(g, chid, format!("Hours Remaining: {}\r\n", vals[2]).as_bytes());
        }
    } else if t == flags::ITEM_SCROLL || t == flags::ITEM_POTION {
        send_to_char(
            g,
            chid,
            format!(
                "Spells: {}, {}, {}\r\n",
                mud_data::spells::skill_name(vals[1]),
                mud_data::spells::skill_name(vals[2]),
                mud_data::spells::skill_name(vals[3])
            )
            .as_bytes(),
        );
    } else if t == flags::ITEM_WAND || t == flags::ITEM_STAFF {
        send_to_char(g, chid, format!("Spell: {}\r\n", mud_data::spells::skill_name(vals[3])).as_bytes());
        send_to_char(g, chid, format!("Charges: {}/{}\r\n", vals[2], vals[1]).as_bytes());
    } else if t == flags::ITEM_WEAPON {
        let avg = ((vals[2] + 1) as f64 / 2.0) * vals[1] as f64;
        send_to_char(
            g,
            chid,
            format!(
                "Damage Dice is '{}D{}' for an average per-round damage of {:.1}.\r\n",
                vals[1], vals[2], avg
            )
            .as_bytes(),
        );
    } else if t == flags::ITEM_ARMOR {
        if vals[1] == 0 {
            send_to_char(g, chid, format!("AC-apply: [{}]\r\n", vals[0]).as_bytes());
        } else {
            send_to_char(g, chid, format!("AC-apply: [{}] - This item has magical affects.\r\n", vals[0]).as_bytes());
        }
    } else if t == flags::ITEM_CONTAINER {
        send_to_char(g, chid, format!("Capacity: {}/{}\r\n", obj_weight(g, obj), vals[0]).as_bytes());
    } else if t == flags::ITEM_DRINKCON || t == flags::ITEM_FOUNTAIN {
        send_to_char(g, chid, format!("Drinks: {}/{}\r\n", vals[1], vals[0]).as_bytes());
    } else if t == flags::ITEM_WORN {
        if vals[1] > 0 {
            send_to_char(g, chid, b"This item has magical affects.\r\n");
        } else {
            send_to_char(g, chid, b"\r\n");
        }
    } else {
        send_to_char(g, chid, b"\r\n");
    }

    let mut found = 0;
    let mut msg = b"Affections:".to_vec();
    for i in 0..MAX_OBJ_AFFECT {
        let a = g.obj(obj).affected[i];
        if a.modifier != 0 {
            let loc = tables::APPLY_TYPES.get(a.location as usize).copied().unwrap_or("UNDEFINED");
            msg.extend_from_slice(
                format!("{} {:+} to {}", if found > 0 { "," } else { "" }, a.modifier, loc).as_bytes(),
            );
            found += 1;
        }
    }
    if found == 0 {
        msg.extend_from_slice(b" None");
    }
    msg.extend_from_slice(b"\r\nExtra Flags: ");
    let mut flagbuf: Vec<u8> = Vec::new();
    let extra_words = g.obj(obj).extra_flags.0;
    crate::act::informative::sprintbitarray(&extra_words, &tables::EXTRA_BITS, &mut flagbuf);
    msg.extend_from_slice(&flagbuf);
    msg.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &msg);

    true
}

/// ok_shop_room — room by VNUM.
pub fn ok_shop_room(g: &Game, shop_idx: usize, room_vnum: i32) -> bool {
    g.world.shops[shop_idx].in_rooms.iter().any(|&r| r == room_vnum)
}

/// shop_keeper — the installed mob spec.
/// ok_damage_shopkeeper: un-charmed keepers of shops
/// without SHOP_KILL_CHARS can't be damaged — they tell the attacker off and
/// slap them.
pub fn ok_damage_shopkeeper(g: &mut Game, chid: CharId, victim: CharId) -> bool {
    // Only prototype mobs whose spec is shop_keeper qualify.
    let rnum = g.ch(victim).mob_rnum;
    if !g.ch(victim).is_npc()
        || rnum == mud_data::types::NOBODY
        || g.mob_specs.get(rnum as usize).copied().flatten() != Some(crate::spec::MobSpec::ShopKeeper)
    {
        return true;
    }
    // Prevent "invincible" shopkeepers if they're charmed.
    if g.ch(victim).aff(mud_data::flags::AFF_CHARM) {
        return true;
    }
    let keeper_vnum = g.world.mob_protos[rnum as usize].vnum as i32;
    let shop = g
        .world
        .shops
        .iter()
        .position(|s| s.keeper_vnum == keeper_vnum && s.bitvector & WILL_START_FIGHT == 0);
    if shop.is_some() {
        let mut buf = g.ch(chid).get_name().to_vec();
        buf.extend_from_slice(b" Get out of here before I call the guards!");
        crate::act::comm::do_tell(g, victim, &buf, 0, 0);

        let name = g.ch(chid).get_name().to_vec();
        let slap = g.shop_cmds.slap;
        crate::act::social::do_action(g, victim, &name, slap, 0);
        return false;
    }
    true
}

pub fn shop_keeper(g: &mut Game, chid: CharId, keeper: CharId, cmd: usize, arg: &[u8]) -> bool {
    let keeper_rnum = g.ch(keeper).mob_rnum;
    let Some(shop_idx) = (0..g.shops_rt.len()).find(|&i| g.shops_rt[i].keeper == keeper_rnum) else {
        return false;
    };

    // Secondary SHOP_FUNC first.
    if let Some(func) = g.shops_rt[shop_idx].func {
        if crate::spec::call_mob_spec(g, func, chid, keeper, cmd, arg) {
            return true;
        }
    }

    if keeper == chid {
        if cmd != 0 {
            g.shops_rt[shop_idx].sort = 0; // Safety in case "drop all".
        }
        return false;
    }
    let room = g.ch(chid).in_room;
    let room_vnum = g.world.rooms[room as usize].vnum as i32;
    if !ok_shop_room(g, shop_idx, room_vnum) {
        return false;
    }
    if !g.ch(keeper).awake() {
        return false;
    }

    let cmd_name = g.commands[cmd].command.clone();
    if cmd_name == b"steal" {
        let mut argm = b"$N shouts '".to_vec();
        argm.extend_from_slice(MSG_NO_STEAL_HERE);
        argm.push(b'\'');
        act(g, &argm, false, Some(chid), None, Some(keeper), comm::TO_CHAR);

        let name = g.ch(chid).get_name().to_vec();
        let slap = g.shop_cmds.slap;
        crate::act::social::do_action(g, keeper, &name, slap, 0);
        return true;
    }

    if cmd_name == b"buy" {
        shopping_buy(g, arg, chid, keeper, shop_idx);
        true
    } else if cmd_name == b"sell" {
        shopping_sell(g, arg, chid, keeper, shop_idx);
        true
    } else if cmd_name == b"value" {
        shopping_value(g, arg, chid, keeper, shop_idx);
        true
    } else if cmd_name == b"list" {
        shopping_list(g, arg, chid, keeper, shop_idx);
        true
    } else if cmd_name == b"identify" {
        shopping_identify(g, arg, chid, keeper, shop_idx)
    } else {
        false
    }
}
