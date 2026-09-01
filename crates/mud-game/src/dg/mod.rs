//! DG Scripts engine (stage 6). Core data model, UID lookup, attachment
//! lifecycle, and script_log.
//!
//! Entity references inside the engine are `GoId` (re-resolved every step,
//! never held across a world mutation); scripts hang off Char/Obj/RoomRt as
//! `Option<Box<ScriptData>>`. Trigger instances carry a stable `iid` so a
//! suspended wait can find its trigger again after the world moves.

pub mod comm;
pub mod commands;
pub mod driver;
pub mod expr;
pub mod misc;
pub mod mobcmd;
pub mod objcmd;
pub mod triggers;
pub mod variables;
pub mod wldcmd;

use mud_data::ids::{CharId, ObjId};
use mud_data::types::{Idx, RoomRnum, NOWHERE};

use crate::game::Game;

pub type BStr = Vec<u8>;

pub const MOB_TRIGGER: i32 = 0;
pub const OBJ_TRIGGER: i32 = 1;
pub const WLD_TRIGGER: i32 = 2;

pub const DG_CASTER_PROXY: i32 = 1;
pub const DG_SPELL_LEVEL: u8 = 25;

pub const UID_CHAR: u8 = b'}';

pub const MOB_ID_BASE: i64 = 10_000_000;
pub const ROOM_ID_BASE: i64 = 20_000_000;
pub const OBJ_ID_BASE: i64 = 30_000_000;

pub const MAX_SCRIPT_DEPTH: i32 = 10;
pub const SCRIPT_ERROR_CODE: i32 = -9_999_999;

pub const TRIG_NEW: i32 = 0;
pub const TRIG_RESTART: i32 = 1;

// Mob trigger types.
pub const MTRIG_GLOBAL: u32 = 1 << 0;
pub const MTRIG_RANDOM: u32 = 1 << 1;
pub const MTRIG_COMMAND: u32 = 1 << 2;
pub const MTRIG_SPEECH: u32 = 1 << 3;
pub const MTRIG_ACT: u32 = 1 << 4;
pub const MTRIG_DEATH: u32 = 1 << 5;
pub const MTRIG_GREET: u32 = 1 << 6;
pub const MTRIG_GREET_ALL: u32 = 1 << 7;
pub const MTRIG_ENTRY: u32 = 1 << 8;
pub const MTRIG_RECEIVE: u32 = 1 << 9;
pub const MTRIG_FIGHT: u32 = 1 << 10;
pub const MTRIG_HITPRCNT: u32 = 1 << 11;
pub const MTRIG_BRIBE: u32 = 1 << 12;
pub const MTRIG_LOAD: u32 = 1 << 13;
pub const MTRIG_MEMORY: u32 = 1 << 14;
pub const MTRIG_CAST: u32 = 1 << 15;
pub const MTRIG_LEAVE: u32 = 1 << 16;
pub const MTRIG_DOOR: u32 = 1 << 17;
pub const MTRIG_DAMAGE: u32 = 1 << 18;
pub const MTRIG_TIME: u32 = 1 << 19;

// Obj trigger types.
pub const OTRIG_GLOBAL: u32 = 1 << 0;
pub const OTRIG_RANDOM: u32 = 1 << 1;
pub const OTRIG_COMMAND: u32 = 1 << 2;
pub const OTRIG_TIMER: u32 = 1 << 5;
pub const OTRIG_GET: u32 = 1 << 6;
pub const OTRIG_DROP: u32 = 1 << 7;
pub const OTRIG_GIVE: u32 = 1 << 8;
pub const OTRIG_WEAR: u32 = 1 << 9;
pub const OTRIG_REMOVE: u32 = 1 << 11;
pub const OTRIG_LOAD: u32 = 1 << 13;
pub const OTRIG_CAST: u32 = 1 << 15;
pub const OTRIG_LEAVE: u32 = 1 << 16;
pub const OTRIG_CONSUME: u32 = 1 << 18;
pub const OTRIG_TIME: u32 = 1 << 19;

// Wld trigger types.
pub const WTRIG_GLOBAL: u32 = 1 << 0;
pub const WTRIG_RANDOM: u32 = 1 << 1;
pub const WTRIG_COMMAND: u32 = 1 << 2;
pub const WTRIG_SPEECH: u32 = 1 << 3;
pub const WTRIG_RESET: u32 = 1 << 5;
pub const WTRIG_ENTER: u32 = 1 << 6;
pub const WTRIG_DROP: u32 = 1 << 7;
pub const WTRIG_CAST: u32 = 1 << 15;
pub const WTRIG_LEAVE: u32 = 1 << 16;
pub const WTRIG_DOOR: u32 = 1 << 17;
pub const WTRIG_LOGIN: u32 = 1 << 18;
pub const WTRIG_TIME: u32 = 1 << 19;

// Obj command trigger location bits.
pub const OCMD_EQUIP: i32 = 1 << 0;
pub const OCMD_INVEN: i32 = 1 << 1;
pub const OCMD_ROOM: i32 = 1 << 2;

// Obj consume trigger commands.
pub const OCMD_EAT: i32 = 1;
pub const OCMD_DRINK: i32 = 2;
pub const OCMD_QUAFF: i32 = 3;

pub const DG_ALLOW_GODS: i32 = 1 << 0;

pub const SPELL_DG_AFFECT: i16 = 298;

/// One variable binding (struct trig_var_data).
#[derive(Debug, Clone)]
pub struct TrigVar {
    pub name: BStr,
    pub value: BStr,
    pub context: i64,
}

/// A trigger instance (struct trig_data). The command list stays on the
/// prototype (world.triggers[nr].cmdlist); `curr_state` is a line index.
#[derive(Debug, Clone)]
pub struct TrigInstance {
    /// Stable instance id — the trigger's identity across a wait.
    pub iid: u64,
    /// rnum into world.triggers.
    pub nr: Idx,
    pub attach_type: i32,
    pub name: BStr,
    pub trigger_type: u32,
    pub narg: i32,
    pub arglist: BStr,
    /// Resume line index for wait (curr_state); usize::MAX means none.
    pub curr_state: usize,
    pub depth: i32,
    pub loops: i32,
    /// Pending wait: the event id armed for this trigger (event_id in
    /// EventKind::TrigWait). None = no wait.
    pub wait_event: Option<u64>,
    pub var_list: Vec<TrigVar>,
}

/// The per-entity script container (struct script_data).
#[derive(Debug, Clone, Default)]
pub struct ScriptData {
    pub types: u32,
    pub trig_list: Vec<TrigInstance>,
    pub global_vars: Vec<TrigVar>,
    pub context: i64,
}

/// mremember record (struct script_memory).
#[derive(Debug, Clone)]
pub struct ScriptMem {
    pub id: i64,
    pub cmd: Option<BStr>,
}

/// Owner of a running script; always re-resolved, never a borrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoId {
    Char(CharId),
    Obj(ObjId),
    Room(RoomRnum),
}

impl GoId {
    pub fn kind(&self) -> i32 {
        match self {
            GoId::Char(_) => MOB_TRIGGER,
            GoId::Obj(_) => OBJ_TRIGGER,
            GoId::Room(_) => WLD_TRIGGER,
        }
    }
}

/// The (owner, running trigger) pair threaded through the interpreter —
/// Carries (go, sc, trig, type); everything re-resolves from this.
#[derive(Debug, Clone, Copy)]
pub struct DgCtx {
    pub go: GoId,
    pub iid: u64,
}

/// Shared per-prototype line state: `loops` and `original` live on the
/// SHARED prototype list, so every instance of a trigger vnum — and
/// successive runs — interfere with each other (§1.3).
#[derive(Debug, Clone, Copy, Default)]
pub struct LineState {
    pub loops: i32,
    /// The while-line index this 'done' points back at (cl->original).
    pub original: Option<usize>,
}

/// What a UID resolves to in the lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UidEntry {
    Char(CharId),
    Obj(ObjId),
}

impl Game {
    /// SCRIPT(go) accessor.
    pub fn script_of(&self, go: GoId) -> Option<&ScriptData> {
        match go {
            GoId::Char(id) => self.try_ch(id)?.script.as_deref(),
            GoId::Obj(id) => self.try_obj(id)?.script.as_deref(),
            GoId::Room(r) => self.rooms.get(r as usize)?.script.as_deref(),
        }
    }

    pub fn script_of_mut(&mut self, go: GoId) -> Option<&mut ScriptData> {
        match go {
            GoId::Char(id) => self.chars.get_mut(id)?.script.as_deref_mut(),
            GoId::Obj(id) => self.objs.get_mut(id)?.script.as_deref_mut(),
            GoId::Room(r) => self.rooms.get_mut(r as usize)?.script.as_deref_mut(),
        }
    }

    /// Ensure a script container exists; the entity must
    /// be live.
    pub fn ensure_script(&mut self, go: GoId) -> &mut ScriptData {
        let slot = match go {
            GoId::Char(id) => &mut self.chars.get_mut(id).expect("stale CharId").script,
            GoId::Obj(id) => &mut self.objs.get_mut(id).expect("stale ObjId").script,
            GoId::Room(r) => &mut self.rooms[r as usize].script,
        };
        slot.get_or_insert_with(Default::default)
    }

    pub fn go_alive(&self, go: GoId) -> bool {
        match go {
            GoId::Char(id) => self.try_ch(id).is_some(),
            GoId::Obj(id) => self.try_obj(id).is_some(),
            GoId::Room(r) => (r as usize) < self.rooms.len(),
        }
    }

    /// SCRIPT_CHECK(go, type).
    pub fn script_check(&self, go: GoId, bit: u32) -> bool {
        self.script_of(go).is_some_and(|sc| sc.types & bit != 0)
    }

    /// Find a trigger instance by iid on an owner.
    pub fn trig(&self, go: GoId, iid: u64) -> Option<&TrigInstance> {
        self.script_of(go)?.trig_list.iter().find(|t| t.iid == iid)
    }

    pub fn trig_mut(&mut self, go: GoId, iid: u64) -> Option<&mut TrigInstance> {
        self.script_of_mut(go)?.trig_list.iter_mut().find(|t| t.iid == iid)
    }
}

/// char_script_id: lazily assign + insert in the table.
pub fn char_script_id(g: &mut Game, chid: CharId) -> i64 {
    let cur = g.ch(chid).script_id;
    if cur != 0 {
        return cur;
    }
    let id = g.max_mob_id;
    g.max_mob_id += 1;
    g.ch_mut(chid).script_id = id;
    add_to_lookup_table(g, id, UidEntry::Char(chid));
    if g.max_mob_id >= ROOM_ID_BASE {
        g.mudlog(
            crate::game::MudlogKind::Cmp,
            mud_data::types::LVL_BUILDER,
            true,
            "SYSERR: Script IDs for mobiles have exceeded the limit -- reboot to fix this",
        );
    }
    id
}

pub fn obj_script_id(g: &mut Game, oid: ObjId) -> i64 {
    let cur = g.obj(oid).script_id;
    if cur != 0 {
        return cur;
    }
    let id = g.max_obj_id;
    g.max_obj_id += 1;
    g.obj_mut(oid).script_id = id;
    add_to_lookup_table(g, id, UidEntry::Obj(oid));
    id
}

/// room_script_id macro: vnum + ROOM_ID_BASE (deterministic, not in table).
pub fn room_script_id(g: &Game, room: RoomRnum) -> i64 {
    g.world.rooms[room as usize].vnum as i64 + ROOM_ID_BASE
}

pub fn add_to_lookup_table(g: &mut Game, uid: i64, e: UidEntry) {
    if let Some(old) = g.dg_lookup.insert(uid, e) {
        if old != e {
            g.log(format!("add_to_lookup updating existing value for uid={}", uid));
        }
    }
}

pub fn remove_from_lookup_table(g: &mut Game, uid: i64) {
    if uid == 0 {
        return;
    }
    if g.dg_lookup.remove(&uid).is_none() {
        g.log(format!("remove_from_lookup. UID {} not found.", uid));
    }
}

/// find_char(n): refuses room-range and up; failed table
/// lookups are logged.
pub fn find_char(g: &mut Game, n: i64) -> Option<CharId> {
    if n >= ROOM_ID_BASE {
        return None;
    }
    match g.dg_lookup.get(&n) {
        Some(UidEntry::Char(id)) => {
            let id = *id;
            g.try_ch(id).map(|_| id)
        }
        Some(UidEntry::Obj(_)) => None,
        None => {
            g.log(format!(
                "find_char_by_uid_in_lookup_table : No entity with number {} in lookup table",
                n
            ));
            None
        }
    }
}

/// find_obj(n): refuses below OBJ_ID_BASE.
pub fn find_obj(g: &mut Game, n: i64) -> Option<ObjId> {
    if n < OBJ_ID_BASE {
        return None;
    }
    match g.dg_lookup.get(&n) {
        Some(UidEntry::Obj(id)) => {
            let id = *id;
            g.try_obj(id).map(|_| id)
        }
        Some(UidEntry::Char(_)) => None,
        None => {
            g.log(format!(
                "find_obj_by_uid_in_lookup_table : No entity with number {} in lookup table",
                n
            ));
            None
        }
    }
}

/// has_obj_by_uid_in_lookup_table: bare existence check, no log.
pub fn has_obj_by_uid_in_lookup_table(g: &Game, n: i64) -> bool {
    g.dg_lookup.contains_key(&n)
}

/// find_room(n): arithmetic + real_room.
pub fn find_room(g: &Game, n: i64) -> Option<RoomRnum> {
    let n = n - ROOM_ID_BASE;
    if n < 0 {
        return None;
    }
    g.real_room(n as i32)
}

/// atoi: leading whitespace, optional sign, digits; i64 to match a long
/// (uids overflow i32).
pub fn atoi64(s: &[u8]) -> i64 {
    let mut i = 0;
    while s.get(i).is_some_and(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')) {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    while let Some(&c) = s.get(i) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i64);
        i += 1;
    }
    if neg { v.wrapping_neg() } else { v }
}

/// atoi truncated to int, matching the 32-bit stores DG uses for narg etc.
pub fn atoi32(s: &[u8]) -> i32 {
    atoi64(s) as i32
}

/// add_var. Rejects names containing '.'; same-name
/// replacement when stored context is 0 or equals the given one; else a new
/// binding is PUSHED AT THE HEAD with the given context.
pub fn add_var(list: &mut Vec<TrigVar>, name: &[u8], value: &[u8], context: i64) {
    if name.contains(&b'.') {
        // The caller has no &mut Game to log through, and the parsers
        // strip dots, so this cannot arise in normal play — dropped.
        return;
    }
    // Stops at the FIRST name match; only that binding's context decides
    // between replace-in-place and push-new-at-head.
    if let Some(v) = list.iter_mut().find(|v| crate::handler::eq_ci(&v.name, name)) {
        if v.context == 0 || v.context == context {
            v.value = value.to_vec();
            return;
        }
    }
    list.insert(0, TrigVar { name: name.to_vec(), value: value.to_vec(), context });
}

/// remove_var: first name match regardless of context.
pub fn remove_var(list: &mut Vec<TrigVar>, name: &[u8]) -> bool {
    if let Some(pos) = list.iter().position(|v| crate::handler::eq_ci(&v.name, name)) {
        list.remove(pos);
        true
    } else {
        false
    }
}

/// read_trigger(rnum) + trig_data_copy: instance from prototype.
pub fn read_trigger(g: &mut Game, rnum: Idx) -> Option<TrigInstance> {
    let proto = g.world.triggers.get(rnum as usize)?;
    let t = TrigInstance {
        iid: {
            g.next_trig_iid += 1;
            g.next_trig_iid
        },
        nr: rnum,
        attach_type: proto.attach_type,
        name: proto.name.clone().unwrap_or_default(),
        trigger_type: proto.trigger_type,
        narg: proto.narg,
        arglist: proto.arglist.clone().unwrap_or_default(),
        curr_state: usize::MAX,
        depth: 0,
        loops: 0,
        wait_event: None,
        var_list: Vec::new(),
    };
    if let Some(c) = g.trig_counts.get_mut(rnum as usize) {
        *c += 1;
    }
    Some(t)
}

/// add_trigger: loc -1 append, 0 prepend, else after
/// position loc (clamped to tail).
pub fn add_trigger_at(sc: &mut ScriptData, t: TrigInstance, loc: i32) {
    let bits = t.trigger_type;
    if loc == 0 {
        sc.trig_list.insert(0, t);
    } else if loc < 0 {
        sc.trig_list.push(t);
    } else {
        // Walk n=loc while there is a next, inserting after it.
        let pos = (loc as usize).min(sc.trig_list.len().saturating_sub(1));
        if sc.trig_list.is_empty() {
            sc.trig_list.push(t);
        } else {
            sc.trig_list.insert(pos + 1, t);
        }
    }
    sc.types |= bits;
}

/// extract_trigger bookkeeping that needs Game (index count, wait cancel).
/// The instance itself is dropped by the caller removing it from the list.
pub fn extract_trigger_book(g: &mut Game, t: &TrigInstance) {
    if t.wait_event.is_some() {
        // event_cancel: the queued TrigWait dies. Our events re-validate
        // wait_event ids at fire time, so dropping the instance suffices;
        // remove the queue entry to keep the queue small.
        let ev = t.wait_event;
        g.events.retain(|e| match e.kind {
            crate::game::EventKind::TrigWait { event_id, .. } => Some(event_id) != ev,
            _ => true,
        });
    }
    if let Some(c) = g.trig_counts.get_mut(t.nr as usize) {
        *c -= 1;
    }
}

/// remove_trigger. `name` = number (position OR vnum),
/// keyword, or N.keyword. Returns true if one was removed.
pub fn remove_trigger(g: &mut Game, go: GoId, name: &[u8]) -> bool {
    let Some(sc) = g.script_of(go) else { return false };

    let dot = name.iter().position(|&b| b == b'.');
    let string = dot.is_some() || !name.first().is_some_and(|b| b.is_ascii_digit());
    let (mut num, sname): (i32, &[u8]) = if string {
        match dot {
            Some(p) => (atoi32(&name[..p]), &name[p + 1..]),
            None => (0, name),
        }
    } else {
        (atoi32(name), name)
    };
    let _ = &mut num;

    let mut found: Option<usize> = None;
    let mut n = 0;
    for (idx, t) in sc.trig_list.iter().enumerate() {
        if string {
            if crate::handler::isname(sname, &t.name) {
                n += 1;
                if n >= num {
                    found = Some(idx);
                    break;
                }
            }
        } else {
            n += 1;
            if n >= num {
                found = Some(idx);
                break;
            }
            let vnum = g.world.triggers[t.nr as usize].vnum as i32;
            if vnum == num {
                found = Some(idx);
                break;
            }
        }
    }

    let Some(idx) = found else { return false };
    let sc = g.script_of_mut(go).unwrap();
    let t = sc.trig_list.remove(idx);
    sc.types = sc.trig_list.iter().fold(0, |acc, t| acc | t.trigger_type);
    extract_trigger_book(g, &t);
    true
}

/// extract_script: drop the whole container.
pub fn extract_script(g: &mut Game, go: GoId) {
    let slot = match go {
        GoId::Char(id) => match g.chars.get_mut(id) {
            Some(c) => &mut c.script,
            None => return,
        },
        GoId::Obj(id) => match g.objs.get_mut(id) {
            Some(o) => &mut o.script,
            None => return,
        },
        GoId::Room(r) => match g.rooms.get_mut(r as usize) {
            Some(rt) => &mut rt.script,
            None => return,
        },
    };
    let Some(sc) = slot.take() else { return };
    for t in &sc.trig_list {
        extract_trigger_book(g, t);
    }
}

/// extract_script_mem: free the mremember list.
pub fn extract_script_mem(g: &mut Game, chid: CharId) {
    if let Some(ch) = g.chars.get_mut(chid) {
        ch.script_mem.clear();
    }
}

/// copy_proto_script + assign_triggers for a fresh mob/obj instance
/// instantiate every proto_script vnum, appending.
pub fn assign_triggers(g: &mut Game, go: GoId) {
    let protos: Vec<Idx> = match go {
        GoId::Char(id) => g.ch(id).proto_script.clone(),
        GoId::Obj(id) => g.obj(id).proto_script.clone(),
        GoId::Room(r) => g.world.rooms[r as usize].proto_script.clone(),
    };
    for vnum in protos {
        let Some(&rnum) = g.world.trig_map.get(&vnum) else {
            let what = match go {
                GoId::Char(id) => format!(
                    "mob #{}",
                    g.world.mob_protos.get(g.ch(id).mob_rnum as usize).map_or(0, |p| p.vnum)
                ),
                GoId::Obj(id) => format!(
                    "obj #{}",
                    g.world.obj_protos.get(g.obj(id).item_number as usize).map_or(0, |p| p.vnum)
                ),
                GoId::Room(r) => format!("room #{}", g.world.rooms[r as usize].vnum),
            };
            g.mudlog(
                crate::game::MudlogKind::Brf,
                mud_data::types::LVL_BUILDER,
                true,
                &format!("SYSERR: trigger #{} non-existant, for {}", vnum, what),
            );
            continue;
        };
        if let Some(t) = read_trigger(g, rnum) {
            add_trigger_at(g.ensure_script(go), t, -1);
        }
    }
}

/// Boot-time room script attach. The world parser only records
/// proto_script, so the instances are built here, in room order and then
/// T-line order.
pub fn boot_room_scripts(g: &mut Game) {
    for r in 0..g.world.rooms.len() {
        if g.world.rooms[r].proto_script.is_empty() {
            continue;
        }
        assign_triggers(g, GoId::Room(r as RoomRnum));
    }
}

/// script_log: syslog line + broadcast to imms with
/// syslog on (level >= LVL_BUILDER, PRF_LOG1/2 sum >= NRM).
pub fn script_log(g: &mut Game, msg: &str) {
    g.log(format!("SCRIPT ERROR: {}", msg));
    let mut targets = Vec::new();
    for &di in &g.descriptors.order {
        let Some(d) = g.descriptors.get(di) else { continue };
        if d.state != mud_data::types::ConState::Playing {
            continue;
        }
        let Some(chid) = d.character else { continue };
        let Some(ch) = g.try_ch(chid) else { continue };
        if ch.is_npc() {
            continue;
        }
        if ch.level < mud_data::types::LVL_BUILDER {
            continue;
        }
        if ch.plr(mud_data::flags::PLR_WRITING) {
            continue;
        }
        let tp = (ch.prf(mud_data::flags::PRF_LOG1) as i32)
            + 2 * (ch.prf(mud_data::flags::PRF_LOG2) as i32);
        if tp < 2 {
            continue;
        }
        targets.push(chid);
    }
    for chid in targets {
        let green = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KGRN).to_vec();
        let nrm = crate::comm::cc(g, chid, crate::comm::C_NRM, crate::comm::KNRM).to_vec();
        let mut line = Vec::with_capacity(msg.len() + 16);
        line.extend_from_slice(&green);
        line.extend_from_slice(b"[ ");
        line.extend_from_slice(msg.as_bytes());
        line.extend_from_slice(b" ]");
        line.extend_from_slice(&nrm);
        line.extend_from_slice(b"\r\n");
        crate::comm::send_to_char_color(g, chid, &line);
    }
}

/// mob_log: `Mob (<short>, VNum <n>):: msg`.
pub fn mob_log(g: &mut Game, chid: CharId, msg: &str) {
    let short = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let vnum = mob_vnum(g, chid);
    script_log(g, &format!("Mob ({}, VNum {}):: {}", short, vnum, msg));
}

/// obj_log: `Obj (<short>, VNum <n>):: msg`.
pub fn obj_log(g: &mut Game, oid: ObjId, msg: &str) {
    let short = String::from_utf8_lossy(crate::handler::obj_short(g, oid)).into_owned();
    let vnum = obj_vnum(g, oid);
    script_log(g, &format!("Obj ({}, VNum {}):: {}", short, vnum, msg));
}

/// wld_log: `Room <vnum>:: msg`.
pub fn wld_log(g: &mut Game, room: RoomRnum, msg: &str) {
    let vnum = g.world.rooms[room as usize].vnum;
    script_log(g, &format!("Room {} :: {}", vnum, msg));
}

/// Trigger-prefixed script_log used by driver builtins:
/// `Trigger: <name>, VNum <n>. <msg>`.
pub fn trig_log(g: &mut Game, go: GoId, iid: u64, msg: &str) {
    let (name, vnum) = match g.trig(go, iid) {
        Some(t) => (
            String::from_utf8_lossy(&t.name).into_owned(),
            g.world.triggers[t.nr as usize].vnum as i32,
        ),
        None => ("<unknown>".into(), 0),
    };
    script_log(g, &format!("Trigger: {}, VNum {}. {}", name, vnum, msg));
}

pub fn trig_vnum(g: &Game, go: GoId, iid: u64) -> i32 {
    g.trig(go, iid).map_or(0, |t| g.world.triggers[t.nr as usize].vnum as i32)
}

/// GET_MOB_VNUM: with CIRCLE_UNSIGNED_INDEX, a PC (or a mob
/// with no rnum) reports NOBODY = 65535 — NOT -1. Scripts compare against
/// the promoted unsigned value (the findmob PC idiom is 65535).
pub fn mob_vnum(g: &Game, chid: CharId) -> i32 {
    let ch = g.ch(chid);
    if ch.is_npc() && ch.mob_rnum != mud_data::types::NOBODY {
        g.world
            .mob_protos
            .get(ch.mob_rnum as usize)
            .map_or(mud_data::types::NOBODY as i32, |p| p.vnum as i32)
    } else {
        mud_data::types::NOBODY as i32
    }
}

/// GET_OBJ_VNUM: unique objects (corpses!) report
/// NOTHING = 65535 — the shipped fido script tests %item.vnum(65535)%.
pub fn obj_vnum(g: &Game, oid: ObjId) -> i32 {
    let o = g.obj(oid);
    if o.item_number != mud_data::types::NOTHING {
        g.world
            .obj_protos
            .get(o.item_number as usize)
            .map_or(mud_data::types::NOTHING as i32, |p| p.vnum as i32)
    } else {
        mud_data::types::NOTHING as i32
    }
}

/// obj_room: room, carrier's room, wearer's room, or the
/// container chain.
pub fn obj_room(g: &Game, oid: ObjId) -> RoomRnum {
    let o = g.obj(oid);
    if o.in_room != NOWHERE {
        o.in_room
    } else if let Some(c) = o.carried_by {
        g.ch(c).in_room
    } else if let Some(w) = o.worn_by {
        g.ch(w).in_room
    } else if let Some(inside) = o.in_obj {
        obj_room(g, inside)
    } else {
        NOWHERE
    }
}

/// The shared while/done line state for a prototype line (lazily zeroed).
pub fn line_state(g: &mut Game, nr: Idx, line: usize) -> &mut LineState {
    g.trig_line_state.entry((nr, line)).or_default()
}

// ---- §2.2 name/UID target resolution ----

fn is_uid(name: &[u8]) -> bool {
    name.first() == Some(&UID_CHAR)
}

/// get_char (world-wide; UID global).
pub fn get_char(g: &mut Game, name: &[u8]) -> Option<CharId> {
    if is_uid(name) {
        let id = find_char(g, atoi64(&name[1..]))?;
        misc::valid_dg_target(g, id, DG_ALLOW_GODS).then_some(id)
    } else {
        let list = g.character_list.clone();
        list.into_iter().find(|&id| {
            g.try_ch(id).is_some_and(|ch| {
                crate::handler::isname(name, ch.name.as_deref().unwrap_or(b""))
            }) && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
        })
    }
}

/// get_char_near_obj: people in the obj's room.
pub fn get_char_near_obj(g: &mut Game, oid: ObjId, name: &[u8]) -> Option<CharId> {
    if is_uid(name) {
        let id = find_char(g, atoi64(&name[1..]))?;
        misc::valid_dg_target(g, id, DG_ALLOW_GODS).then_some(id)
    } else {
        let room = obj_room(g, oid);
        if room == NOWHERE {
            return None;
        }
        let people = g.rooms[room as usize].people.clone();
        people.into_iter().find(|&id| {
            g.try_ch(id).is_some_and(|ch| {
                crate::handler::isname(name, ch.name.as_deref().unwrap_or(b""))
            }) && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
        })
    }
}

/// get_char_in_room.
pub fn get_char_in_room(g: &mut Game, room: RoomRnum, name: &[u8]) -> Option<CharId> {
    if is_uid(name) {
        let id = find_char(g, atoi64(&name[1..]))?;
        misc::valid_dg_target(g, id, DG_ALLOW_GODS).then_some(id)
    } else {
        let people = g.rooms[room as usize].people.clone();
        people.into_iter().find(|&id| {
            g.try_ch(id).is_some_and(|ch| {
                crate::handler::isname(name, ch.name.as_deref().unwrap_or(b""))
            }) && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
        })
    }
}

/// get_char_by_obj: carrier, wearer, then whole world.
pub fn get_char_by_obj(g: &mut Game, oid: ObjId, name: &[u8]) -> Option<CharId> {
    if is_uid(name) {
        let id = find_char(g, atoi64(&name[1..]))?;
        return misc::valid_dg_target(g, id, DG_ALLOW_GODS).then_some(id);
    }
    let o = g.obj(oid);
    let (carrier, wearer) = (o.carried_by, o.worn_by);
    if let Some(c) = carrier {
        if g.try_ch(c).is_some_and(|ch| crate::handler::isname(name, ch.name.as_deref().unwrap_or(b"")))
            && misc::valid_dg_target(g, c, DG_ALLOW_GODS)
        {
            return Some(c);
        }
    }
    if let Some(w) = wearer {
        if g.try_ch(w).is_some_and(|ch| crate::handler::isname(name, ch.name.as_deref().unwrap_or(b"")))
            && misc::valid_dg_target(g, w, DG_ALLOW_GODS)
        {
            return Some(w);
        }
    }
    let list = g.character_list.clone();
    list.into_iter().find(|&id| {
        g.try_ch(id)
            .is_some_and(|ch| crate::handler::isname(name, ch.name.as_deref().unwrap_or(b"")))
            && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
    })
}

/// get_char_by_room: room's people, then whole world.
pub fn get_char_by_room(g: &mut Game, room: RoomRnum, name: &[u8]) -> Option<CharId> {
    if is_uid(name) {
        let id = find_char(g, atoi64(&name[1..]))?;
        return misc::valid_dg_target(g, id, DG_ALLOW_GODS).then_some(id);
    }
    let people = g.rooms[room as usize].people.clone();
    for id in people {
        if g.try_ch(id).is_some_and(|ch| crate::handler::isname(name, ch.name.as_deref().unwrap_or(b"")))
            && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
        {
            return Some(id);
        }
    }
    let list = g.character_list.clone();
    list.into_iter().find(|&id| {
        g.try_ch(id)
            .is_some_and(|ch| crate::handler::isname(name, ch.name.as_deref().unwrap_or(b"")))
            && misc::valid_dg_target(g, id, DG_ALLOW_GODS)
    })
}

/// get_obj_in_list: UID or keyword against a list snapshot.
pub fn get_obj_in_list(g: &Game, name: &[u8], list: &[ObjId]) -> Option<ObjId> {
    if is_uid(name) {
        let id = atoi64(&name[1..]);
        list.iter().copied().find(|&o| g.try_obj(o).is_some_and(|ob| ob.script_id == id && id != 0))
    } else {
        list.iter()
            .copied()
            .find(|&o| g.try_obj(o).is_some_and(|_| crate::handler::isname(name, crate::handler::obj_name(g, o))))
    }
}

/// get_object_in_equip: UID, bare vnum, or keyword over worn eq.
pub fn get_object_in_equip(g: &Game, chid: CharId, name: &[u8]) -> Option<ObjId> {
    let eq = g.ch(chid).equipment;
    if is_uid(name) {
        let id = atoi64(&name[1..]);
        for slot in eq.iter().flatten() {
            if g.try_obj(*slot).is_some_and(|o| o.script_id == id && id != 0) {
                return Some(*slot);
            }
        }
        return None;
    }
    if name.iter().all(|b| b.is_ascii_digit()) && !name.is_empty() {
        let vnum = atoi32(name);
        for slot in eq.iter().flatten() {
            if obj_vnum(g, *slot) == vnum {
                return Some(*slot);
            }
        }
        return None;
    }
    // get_number(&tmp) + isname over positions.
    let (number, tmpname) = crate::handler::get_number(name);
    if number == 0 {
        return None;
    }
    let mut j = 0;
    for slot in eq.iter().flatten() {
        if crate::handler::isname(&tmpname, crate::handler::obj_name(g, *slot)) {
            j += 1;
            if j == number {
                return Some(*slot);
            }
        }
    }
    None
}

pub fn get_obj_near_obj(g: &mut Game, oid: ObjId, name: &[u8]) -> Option<ObjId> {
    if crate::handler::eq_ci(name, b"self") || crate::handler::eq_ci(name, b"me") {
        return Some(oid);
    }
    let contains = g.obj(oid).contains.clone();
    if !contains.is_empty() {
        if let Some(i) = get_obj_in_list(g, name, &contains) {
            return Some(i);
        }
    }
    let (in_obj, worn_by, carried_by) = {
        let o = g.obj(oid);
        (o.in_obj, o.worn_by, o.carried_by)
    };
    if let Some(container) = in_obj {
        if is_uid(name) {
            let id = atoi64(&name[1..]);
            if id == g.obj(container).script_id {
                return Some(container);
            }
        } else if crate::handler::isname(name, crate::handler::obj_name(g, container)) {
            return Some(container);
        }
    } else if let Some(w) = worn_by {
        if let Some(i) = get_object_in_equip(g, w, name) {
            return Some(i);
        }
    } else if let Some(c) = carried_by {
        let carrying = g.ch(c).carrying.clone();
        if let Some(i) = get_obj_in_list(g, name, &carrying) {
            return Some(i);
        }
    } else {
        let rm = obj_room(g, oid);
        if rm != NOWHERE {
            let contents = g.rooms[rm as usize].contents.clone();
            if let Some(i) = get_obj_in_list(g, name, &contents) {
                return Some(i);
            }
            let people = g.rooms[rm as usize].people.clone();
            for ch in people {
                if let Some(i) = get_object_in_equip(g, ch, name) {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// get_obj (world-wide).
pub fn get_obj(g: &mut Game, name: &[u8]) -> Option<ObjId> {
    if is_uid(name) {
        find_obj(g, atoi64(&name[1..]))
    } else {
        let list = g.object_list.clone();
        list.into_iter().find(|&o| {
            g.try_obj(o).is_some() && crate::handler::isname(name, crate::handler::obj_name(g, o))
        })
    }
}

/// get_room: UID or vnum.
pub fn get_room(g: &Game, name: &[u8]) -> Option<RoomRnum> {
    if is_uid(name) {
        find_room(g, atoi64(&name[1..]))
    } else {
        g.real_room(atoi32(name))
    }
}

pub fn get_obj_by_obj(g: &mut Game, oid: ObjId, name: &[u8]) -> Option<ObjId> {
    if is_uid(name) {
        return find_obj(g, atoi64(&name[1..]));
    }
    if crate::handler::eq_ci(name, b"self") || crate::handler::eq_ci(name, b"me") {
        return Some(oid);
    }
    let contains = g.obj(oid).contains.clone();
    if !contains.is_empty() {
        if let Some(i) = get_obj_in_list(g, name, &contains) {
            return Some(i);
        }
    }
    let (in_obj, worn_by, carried_by) = {
        let o = g.obj(oid);
        (o.in_obj, o.worn_by, o.carried_by)
    };
    if let Some(container) = in_obj {
        if crate::handler::isname(name, crate::handler::obj_name(g, container)) {
            return Some(container);
        }
    }
    if let Some(w) = worn_by {
        if let Some(i) = get_object_in_equip(g, w, name) {
            return Some(i);
        }
    }
    if let Some(c) = carried_by {
        let carrying = g.ch(c).carrying.clone();
        if let Some(i) = get_obj_in_list(g, name, &carrying) {
            return Some(i);
        }
    }
    let rm = obj_room(g, oid);
    if rm != NOWHERE {
        let contents = g.rooms[rm as usize].contents.clone();
        if let Some(i) = get_obj_in_list(g, name, &contents) {
            return Some(i);
        }
    }
    get_obj(g, name)
}

/// get_obj_in_room: room contents only, by script_id or keyword.
pub fn get_obj_in_room(g: &Game, room: RoomRnum, name: &[u8]) -> Option<ObjId> {
    let contents = &g.rooms[room as usize].contents;
    if is_uid(name) {
        let id = atoi64(&name[1..]);
        contents.iter().copied().find(|&o| g.try_obj(o).is_some_and(|ob| ob.script_id == id && id != 0))
    } else {
        contents
            .iter()
            .copied()
            .find(|&o| g.try_obj(o).is_some() && crate::handler::isname(name, crate::handler::obj_name(g, o)))
    }
}

/// get_obj_by_room: room contents, then world.
pub fn get_obj_by_room(g: &mut Game, room: RoomRnum, name: &[u8]) -> Option<ObjId> {
    if is_uid(name) {
        return find_obj(g, atoi64(&name[1..]));
    }
    let contents = g.rooms[room as usize].contents.clone();
    for o in contents {
        if g.try_obj(o).is_some() && crate::handler::isname(name, crate::handler::obj_name(g, o)) {
            return Some(o);
        }
    }
    let list = g.object_list.clone();
    list.into_iter()
        .find(|&o| g.try_obj(o).is_some() && crate::handler::isname(name, crate::handler::obj_name(g, o)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_var_context_rules() {
        let mut list = Vec::new();
        add_var(&mut list, b"x", b"1", 0);
        add_var(&mut list, b"x", b"2", 0);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].value, b"2");
        // Nonzero context replaces a context-0 binding's value in place.
        add_var(&mut list, b"x", b"3", 42);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].value, b"3");
        assert_eq!(list[0].context, 0);
        // A different-context binding coexists when stored context nonzero.
        list[0].context = 7;
        add_var(&mut list, b"x", b"4", 42);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].value, b"4"); // pushed at head
        assert_eq!(list[0].context, 42);
        // Dotted names rejected.
        add_var(&mut list, b"a.b", b"v", 0);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn atoi64_stops_at_first_non_digit() {
        assert_eq!(atoi64(b"  42abc"), 42);
        assert_eq!(atoi64(b"-7"), -7);
        assert_eq!(atoi64(b"+9"), 9);
        assert_eq!(atoi64(b"abc"), 0);
        assert_eq!(atoi64(b""), 0);
        assert_eq!(atoi64(b"20003001"), 20003001);
    }
}
