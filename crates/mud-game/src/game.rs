//! The `Game` struct — one owner for all mutable server state: world data,
//! entity arenas, descriptor table, clocks, RNG, config, and logging.
//! All mutation flows through `&mut Game` (PLAN.md §3.2).

use std::collections::VecDeque;

use mud_data::ids::{Arena, CharId, CharTag, ObjId, ObjTag};
use mud_data::rng::CircleRng;
use mud_data::types::{Idx, RoomRnum, ZoneRnum};
use mud_net::descriptor::Descriptors;

use crate::ch::Char;
use crate::config::Config;
use crate::gametime::{MudTime, Weather};
use crate::obj::Obj;
use crate::text::Texts;

/// Runtime per-room state parallel to `world.rooms` (people/contents are
/// head-insert lists: index 0 = most recently added).
#[derive(Debug, Default, Clone)]
pub struct RoomRt {
    pub people: Vec<CharId>,
    pub contents: Vec<ObjId>,
    pub light: i32,
    /// DG script container (room_data.script).
    pub script: Option<Box<crate::dg::ScriptData>>,
}

/// Runtime per-zone state (struct zone_data's mutable half).
#[derive(Debug, Default, Clone)]
pub struct ZoneRt {
    pub age: i32,
}

pub const ZO_DEAD: i32 = 999;

/// One player-index row.
#[derive(Debug, Clone)]
pub struct PlayerIndexElement {
    /// Stored lowercase.
    pub name: Vec<u8>,
    pub id: i64,
    pub level: i32,
    pub flags: i32,
    pub last: i64,
}

pub const PINDEX_DELETED: i32 = 1 << 0;
pub const PINDEX_NODELETE: i32 = 1 << 1;
pub const PINDEX_SELFDELETE: i32 = 1 << 2;
pub const PINDEX_NOWIZLIST: i32 = 1 << 3;

/// The event queue payloads. mud_events and DG waits share one queue and
/// fire in insertion order per pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// get_protocols: greet + banner at connect+1.5s.
    Protocols { desc: usize },
    /// eWHIRLWIND: the whirlwind attack's per-round event.
    Whirlwind { ch: CharId },
    /// eSPL_DARKNESS: the room darkness countdown.
    SplDarkness { room: RoomRnum },
    /// DG `wait` continuation. Fires only if the trigger instance still
    /// exists and its armed wait_event matches `event_id`; anything else
    /// counts as cancelled.
    TrigWait { go: crate::dg::GoId, iid: u64, event_id: u64 },
}

/// GROUP_* flags.
pub const GROUP_OPEN: i32 = 1 << 0;
pub const GROUP_ANON: i32 = 1 << 1;
pub const GROUP_NPC: i32 = 1 << 2;

/// A player group. Kept in an append-ordered list with stable u64 ids;
/// members are append-ordered too.
#[derive(Debug, Clone)]
pub struct Group {
    pub id: u64,
    pub leader: Option<CharId>,
    pub members: Vec<CharId>,
    pub group_flags: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct MudEvent {
    pub fire_at: u64,
    /// Insertion sequence. queue_enq walks back from the bucket tail only
    /// while `i->key < key`, so an equal-keyed event is inserted BEFORE the
    /// existing ones: same-pulse events fire newest-first, not FIFO.
    pub seq: u64,
    pub kind: EventKind,
}

/// An entity that owns a lazily created mud_event list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventOwner {
    Char(CharId),
    Room(RoomRnum),
}

/// struct happyhour.
#[derive(Debug, Clone, Copy, Default)]
pub struct HappyHour {
    pub qp_rate: i32,
    pub exp_rate: i32,
    pub gold_rate: i32,
    pub ticks_left: i32,
}

pub struct Game {
    pub world: mud_world::model::World,
    pub rooms: Vec<RoomRt>,
    pub zones_rt: Vec<ZoneRt>,
    pub reset_q: VecDeque<ZoneRnum>,

    pub chars: Arena<Char, CharTag>,
    pub objs: Arena<Obj, ObjTag>,
    /// Global lists, newest-first. Iteration order is observable. Deques,
    /// not Vecs: the C prepends to a linked list in O(1), and with a
    /// million mobs loading at boot a Vec::insert(0, ..) per load is
    /// quadratic.
    pub character_list: std::collections::VecDeque<CharId>,
    pub object_list: std::collections::VecDeque<ObjId>,
    /// Live instance counts per prototype.
    pub mob_counts: Vec<i32>,
    pub obj_counts: Vec<i32>,

    pub descriptors: Descriptors,
    pub events: Vec<MudEvent>,

    pub time_info: MudTime,
    pub weather: Weather,
    pub beginning_of_time: i64,
    pub rng: CircleRng,
    /// Wall clock, refreshed each pulse (deterministic in tests).
    pub now: i64,
    pub boot_time: i64,
    pub pulse: u64,

    pub player_table: Vec<PlayerIndexElement>,
    pub top_idnum: i64,

    /// zone_update's function-static minute counter.
    pub zone_timer: u64,
    /// Local-time offset for %H:%M history stamps.
    pub tz_offset_secs: i64,
    /// lib/misc/xnames substrings (read_invalid_list).
    pub invalid_names: Vec<Vec<u8>>,
    /// ban_list — newest first.
    pub ban_list: Vec<crate::ban::BanEntry>,
    /// save_list: (zone vnum, SL_* type) pairs pending a disk write.
    /// Newest first: add_to_save_list prepends.
    pub save_list: Vec<(Idx, i32)>,
    /// r_mortal_start_room / r_immort_start_room / r_frozen_start_room
    /// resolved once at boot, and *shifted* by add_room /
    /// zeroed by delete_room, so a deleted start room stays The Void until
    /// the next reboot even if the vnum comes back.
    pub r_mortal_start_room: RoomRnum,
    pub r_immort_start_room: RoomRnum,
    pub r_frozen_start_room: RoomRnum,
    /// The OLC editor state of each descriptor in an OLC state, keyed by
    /// descriptor index.
    pub olc: std::collections::HashMap<usize, Box<crate::olc::OlcData>>,
    /// The OLC colour strings `nrm`/`grn`/`cyn`/`yel` — shared by every
    /// builder in OLC at once.
    pub olc_colors: crate::olc::OlcColors,
    /// Set by do_copyover; the server binary performs the socket handoff
    /// and replaces the process, so the call never returns.
    pub copyover: Option<crate::copyover::CopyoverPlan>,
    /// Entities that have had a `create_list` for their event queue
    /// counted by `show stats`, freed with the owner.
    pub event_lists: std::collections::HashSet<EventOwner>,
    /// heartbeat's function-static mins_since_crashsave.
    pub autosave_minutes: i32,

    pub config: Config,
    pub texts: Texts,
    pub lib_dir: std::path::PathBuf,

    /// Characters marked for extraction this pulse (extractions_pending).
    pub extractions_pending: i32,

    pub circle_shutdown: bool,
    pub circle_reboot: i32,
    pub circle_restrict: u8,
    pub mini_mud: bool,
    pub no_rent_check: bool,
    pub no_specials: bool,
    /// no_mail: set when the boot-time scan fails.
    pub no_mail: bool,

    /// DG UID lookup table. Bucketing is an implementation detail; the
    /// id→entity map is the behavior.
    pub dg_lookup: std::collections::HashMap<i64, crate::dg::UidEntry>,
    /// max_mob_id / max_obj_id: rolling script-id counters.
    pub max_mob_id: i64,
    pub max_obj_id: i64,
    /// Trigger-instance id counter — the identity a wait resumes against.
    pub next_trig_iid: u64,
    /// Unique ids for TrigWait events (cancel-vs-snapshot validation).
    pub next_dg_event_id: u64,
    /// Event insertion counter (see MudEvent::seq).
    pub next_event_seq: u64,
    /// Live attach counts per trigger rnum.
    pub trig_counts: Vec<i32>,
    /// SHARED per-prototype line state, keyed by (trigger rnum, line index).
    /// Every running instance of a trigger sees the same counters; see
    /// `dg::LineState`.
    pub trig_line_state: std::collections::HashMap<(Idx, usize), crate::dg::LineState>,
    /// script_driver's static recursion depth.
    pub dg_script_depth: i32,
    /// dg_owner_purged.
    pub dg_owner_purged: bool,
    /// dg_act_check: whether the LAST act call had triggers enabled.
    /// Direct perform_act callers observe the stale value.
    pub dg_act_check: bool,

    /// Log sink: lines are timestamped and written by the binary's logger.
    pub log_lines: Vec<String>,

    /// Boot-time syslog echo (mudlog also mirrors to connected imms).
    pub socials: Vec<crate::social::Social>,
    pub commands: Vec<crate::interpreter::CommandEntry>,
    pub help_table: Vec<crate::text::HelpEntry>,
    /// Bumped every time the help table is rebuilt or replaced. hedit records
    /// it when it opens and compares on the way out: an index taken against
    /// a table that has since moved names whatever now sits in that slot,
    /// which is somebody else's entry.
    pub help_table_version: u64,

    /// Special-procedure registries, indexed by prototype rnum or room rnum.
    pub mob_specs: Vec<Option<crate::spec::MobSpec>>,
    pub obj_specs: Vec<Option<crate::spec::ObjSpec>>,
    pub room_specs: Vec<Option<crate::spec::RoomSpec>>,
    /// Runtime shop state parallel to world.shops.
    pub shops_rt: Vec<crate::shop::ShopRt>,
    pub shop_cmds: crate::shop::ShopCmds,

    /// combat_list: head-first, prepend on set_fighting.
    pub combat_list: Vec<CharId>,
    /// next_combat_list — the perform_violence cursor, patched by
    /// stop_fighting.
    pub next_combat: Option<CharId>,
    /// The combat message lists, from lib/misc/messages.
    pub fight_messages: Vec<crate::fight::FightMessageList>,
    /// Per-mob mayor/King-Welmar path state (B5 fixed: per-mob rather than
    /// file-static; identical in the shipped world, which has one of each).
    pub mob_paths: std::collections::HashMap<CharId, crate::spec::PathState>,

    /// group_list: append-ordered live groups.
    pub groups: Vec<Group>,
    pub next_group_id: u64,
    /// cast_arg2: first word of the last cast target string.
    pub cast_arg2: Vec<u8>,
    /// house_control[] — live house records, boot order.
    pub houses: Vec<crate::house::HouseControl>,
    /// The bulletin boards.
    pub boards: crate::boards::BoardState,
    /// The Idea/Bug/Typo lists.
    pub ibt: crate::ibt::IbtLists,
    /// happy_data.
    pub happy: HappyHour,
    /// recent_list — newest first, memory only.
    pub recent_list: Vec<crate::llog::RecentPlayer>,
    /// next_tick: seconds until the next mud-hour tick.
    pub next_tick: i32,
    /// QST_FUNC: a questmaster's pre-existing spec, kept as secondary. It
    /// has to ride along with the row whenever `add_quest`/`delete_quest`
    /// shift the table — hence a Vec parallel to `world.quests`, not a map
    /// keyed by an rnum that moves.
    pub quest_secondary: Vec<Option<crate::spec::MobSpec>>,
}

impl Game {
    pub fn ch(&self, id: CharId) -> &Char {
        self.chars.get(id).expect("stale CharId")
    }

    pub fn ch_mut(&mut self, id: CharId) -> &mut Char {
        self.chars.get_mut(id).expect("stale CharId")
    }

    pub fn try_ch(&self, id: CharId) -> Option<&Char> {
        self.chars.get(id)
    }

    pub fn obj(&self, id: ObjId) -> &Obj {
        self.objs.get(id).expect("stale ObjId")
    }

    pub fn obj_mut(&mut self, id: ObjId) -> &mut Obj {
        self.objs.get_mut(id).expect("stale ObjId")
    }

    pub fn try_obj(&self, id: ObjId) -> Option<&Obj> {
        self.objs.get(id)
    }

    /// Loop guard: an ObjId snapshot may go stale mid-iteration (extraction).
    pub fn try_obj_alive(&self, id: ObjId) -> bool {
        self.objs.get(id).is_some()
    }

    /// GROUP(ch) — the group the character belongs to, if still live.
    pub fn group_of(&self, chid: CharId) -> Option<&Group> {
        let gid = self.try_ch(chid)?.group?;
        self.groups.iter().find(|gr| gr.id == gid)
    }

    pub fn group(&self, gid: u64) -> Option<&Group> {
        self.groups.iter().find(|gr| gr.id == gid)
    }

    pub fn group_mut(&mut self, gid: u64) -> Option<&mut Group> {
        self.groups.iter_mut().find(|gr| gr.id == gid)
    }

    /// `global_lists->iSize` for `show stats`. `create_list`
    /// registers every list it makes except the very first, so the live count
    /// is: group_list + world_events (both made at boot), one per descriptor
    /// (`newd->events`), one per group (`members`), plus the lazily created
    /// per-character and per-room event lists.
    pub fn list_count(&self) -> usize {
        2 + self.descriptors.order.len() + self.groups.len() + self.event_lists.len()
    }

    /// basic_mud_log line (timestamping happens at the sink).
    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        mud_data::rng::rng_trace_note(&msg);
        self.log_lines.push(msg);
    }

    /// mudlog: log file + broadcast to qualified imms.
    /// type: OFF/BRF/NRM/CMP → PRF_LOG1/2 levels.
    pub fn mudlog(&mut self, kind: MudlogKind, level: u8, file: bool, msg: &str) {
        if file {
            self.log(msg.to_string());
        }
        if kind == MudlogKind::Off {
            return;
        }
        let mut targets = Vec::new();
        for &di in &self.descriptors.order {
            let Some(d) = self.descriptors.get(di) else { continue };
            if d.state != mud_data::types::ConState::Playing {
                continue;
            }
            let Some(chid) = d.character else { continue };
            let Some(ch) = self.chars.get(chid) else { continue };
            if ch.is_npc() || ch.level < level {
                continue;
            }
            if ch.plr(mud_data::flags::PLR_WRITING) {
                continue;
            }
            let log1 = ch.prf(mud_data::flags::PRF_LOG1) as i32;
            let log2 = ch.prf(mud_data::flags::PRF_LOG2) as i32;
            let tp = (log1 + 2 * log2) as i32;
            if tp < kind as i32 {
                continue;
            }
            targets.push(chid);
        }
        // sends the whole thing in one call, wrapped in the
        // recipient's green: `send_to_char(ch, "%s%s%s", CCGRN(ch, C_NRM),
        // buf, CCNRM(ch, C_NRM))`. Dropping the wrapper left every wiznet
        // line plain for an immortal with colour on.
        let line = format!("[ {} ]\r\n", msg);
        for chid in targets {
            let grn = crate::comm::cc(self, chid, crate::comm::C_NRM, crate::comm::KGRN);
            let nrm = crate::comm::cc(self, chid, crate::comm::C_NRM, crate::comm::KNRM);
            let mut out = Vec::with_capacity(grn.len() + line.len() + nrm.len());
            out.extend_from_slice(grn);
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(nrm);
            crate::comm::send_to_char_color(self, chid, &out);
        }
    }

    pub fn room_rt(&self, r: RoomRnum) -> &RoomRt {
        &self.rooms[r as usize]
    }

    pub fn room_rt_mut(&mut self, r: RoomRnum) -> &mut RoomRt {
        &mut self.rooms[r as usize]
    }

    /// real_room helper, keyed by u16 vnum.
    pub fn real_room(&self, vnum: i32) -> Option<RoomRnum> {
        if !(0..65535).contains(&vnum) {
            return None;
        }
        self.world.real_room(vnum as Idx)
    }

    /// Schedule a mud event (event_create: `when` clamped to >= 1).
    pub fn queue_event(&mut self, fire_in_pulses: u64, kind: EventKind) {
        self.next_event_seq += 1;
        let seq = self.next_event_seq;
        // attach_mud_event creates the owner's event
        // list on first use; `show stats` counts it.
        match kind {
            EventKind::Whirlwind { ch } => {
                self.event_lists.insert(EventOwner::Char(ch));
            }
            EventKind::SplDarkness { room } => {
                self.event_lists.insert(EventOwner::Room(room));
            }
            _ => {}
        }
        self.events.push(MudEvent { fire_at: self.pulse + fire_in_pulses.max(1), seq, kind });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MudlogKind {
    Off = 0,
    Brf = 1,
    Nrm = 2,
    Cmp = 3,
}
