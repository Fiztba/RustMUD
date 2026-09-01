//! In-memory world model for Stage 1: parsed prototypes and zone data.
//!
//! Field shapes match what the parse routines *store*, because the writers
//! emit from those stored values — e.g. a mob file's THAC0 becomes
//! `hitroll = 20 - thac0` in memory and is written back as `20 - hitroll`.
//!
//! rnums are file-load order. Lookup, however, goes through vnum-keyed maps
//! rather than a binary search over an assumed-sorted array, so a file whose
//! records are out of order still resolves -- 301.wld ships two such rooms.

use std::collections::HashMap;

use mud_data::types::{Idx, RoomRnum, ZoneRnum};

pub type BStr = Vec<u8>;

#[derive(Debug, Clone, Default)]
pub struct ExtraDesc {
    pub keyword: Option<BStr>,
    pub description: Option<BStr>,
}

#[derive(Debug, Clone)]
pub struct Exit {
    pub general_description: Option<BStr>,
    pub keyword: Option<BStr>,
    /// EX_* bits as stored after door-flag mapping (0..=4 -> bit combos).
    pub exit_info: u16,
    /// Key object vnum; NOTHING for none (file -1 or 65535).
    pub key: Idx,
    /// Raw to_room vnum as parsed (file value; 0 and -1 both mean "none").
    pub to_room_vnum: i32,
    /// Resolved room rnum after renum_world; NOWHERE if unresolvable.
    pub to_room: RoomRnum,
}

#[derive(Debug, Clone, Default)]
pub struct Room {
    pub vnum: Idx,
    pub zone: ZoneRnum,
    pub sector_type: i32,
    pub room_flags: [u32; 4],
    pub name: Option<BStr>,
    pub description: Option<BStr>,
    pub ex_descriptions: Vec<ExtraDesc>,
    pub dir_option: [Option<Box<Exit>>; 10],
    /// Trigger vnums attached via room T lines (proto_script).
    pub proto_script: Vec<Idx>,
}

#[derive(Debug, Clone, Default)]
pub struct MobProto {
    pub vnum: Idx,
    pub keywords: Option<BStr>,
    pub short_descr: Option<BStr>,
    pub long_descr: Option<BStr>,
    pub ddescription: Option<BStr>,
    pub act: [u32; 4],
    pub affected_by: [u32; 4],
    pub alignment: i32,
    /// 'S'/'E' type letter from the flags line (writer always emits 'E').
    pub level: i32,
    /// Stored as 20 - THAC0(file).
    pub hitroll: i32,
    /// Stored as 10 * AC(file).
    pub armor: i32,
    /// Hit points: file XdY+Z stored as hit=X, mana=Y, move=Z (proto abuse).
    pub hit: i32,
    pub mana: i32,
    pub mov: i32,
    /// Damage dice XdY+Z: damnodice, damsizedice, damroll.
    pub damnodice: i32,
    pub damsizedice: i32,
    pub damroll: i32,
    pub gold: i32,
    pub exp: i32,
    pub position: i32,
    pub default_pos: i32,
    pub sex: i32,
    /// E-spec extensions; None = default (writer omits).
    pub bare_hand_attack: Option<i32>,
    pub str_: Option<i32>,
    pub str_add: Option<i32>,
    pub intel: Option<i32>,
    pub wis: Option<i32>,
    pub dex: Option<i32>,
    pub con: Option<i32>,
    pub cha: Option<i32>,
    pub saving_para: Option<i32>,
    pub saving_rod: Option<i32>,
    pub saving_petri: Option<i32>,
    pub saving_breath: Option<i32>,
    pub saving_spell: Option<i32>,
    pub proto_script: Vec<Idx>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ObjAffect {
    pub location: i32,
    pub modifier: i32,
}

#[derive(Debug, Clone, Default)]
pub struct ObjProto {
    pub vnum: Idx,
    pub name: Option<BStr>,
    pub short_description: Option<BStr>,
    pub description: Option<BStr>,
    pub action_description: Option<BStr>,
    pub type_flag: i32,
    pub extra_flags: [u32; 4],
    pub wear_flags: [u32; 4],
    pub perm_affects: [u32; 4],
    pub values: [i32; 4],
    pub weight: i32,
    pub cost: i32,
    pub cost_per_day: i32,
    pub level: i32,
    pub timer: i32,
    pub affected: [ObjAffect; 6],
    pub ex_descriptions: Vec<ExtraDesc>,
    pub proto_script: Vec<Idx>,
}

#[derive(Debug, Clone, Default)]
pub struct ZoneCommand {
    pub command: u8,
    pub if_flag: i32,
    pub arg1: i32,
    pub arg2: i32,
    pub arg3: i32,
    pub arg4: i32,
    pub arg5: i32,
    /// V-command strings: variable name, value.
    pub sarg1: Option<BStr>,
    pub sarg2: Option<BStr>,
    /// Physical line number in the .zon file, for error reporting.
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Zone {
    pub number: Idx,
    pub builders: Option<BStr>,
    pub name: Option<BStr>,
    pub bot: Idx,
    pub top: Idx,
    pub lifespan: i32,
    pub reset_mode: i32,
    pub zone_flags: [u32; 4],
    pub min_level: i32,
    pub max_level: i32,
    pub cmds: Vec<ZoneCommand>,
}

#[derive(Debug, Clone, Default)]
pub struct Trigger {
    pub vnum: Idx,
    pub name: Option<BStr>,
    pub attach_type: i32,
    pub trigger_type: u32,
    pub narg: i32,
    pub arglist: Option<BStr>,
    /// Script body lines as loaded; blank lines are dropped.
    pub cmdlist: Vec<BStr>,
}

#[derive(Debug, Clone, Default)]
pub struct ShopBuyData {
    pub type_: i32,
    pub keywords: Option<BStr>,
}

#[derive(Debug, Clone, Default)]
pub struct Shop {
    pub vnum: Idx,
    pub producing: Vec<i32>,
    pub profit_buy: f32,
    pub profit_sell: f32,
    pub type_list: Vec<ShopBuyData>,
    pub no_such_item1: Option<BStr>,
    pub no_such_item2: Option<BStr>,
    pub missing_cash1: Option<BStr>,
    pub missing_cash2: Option<BStr>,
    pub do_not_buy: Option<BStr>,
    pub message_buy: Option<BStr>,
    pub message_sell: Option<BStr>,
    pub temper1: i32,
    pub bitvector: u32,
    /// Keeper mob vnum as in file; the resolved rnum is kept alongside it.
    pub keeper_vnum: i32,
    pub with_who: i32,
    pub in_rooms: Vec<i32>,
    pub open1: i32,
    pub close1: i32,
    pub open2: i32,
    pub close2: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Quest {
    pub vnum: Idx,
    pub qm_vnum: i32,
    pub flags: u32,
    pub type_: i32,
    pub name: Option<BStr>,
    pub desc: Option<BStr>,
    pub info: Option<BStr>,
    pub done: Option<BStr>,
    pub quit: Option<BStr>,
    pub value: i32,
    pub penalty: i32,
    pub min_level: i32,
    pub max_level: i32,
    pub target: i32,
    pub prereq: i32,
    pub obj_in: i32,
    pub obj_out: i32,
    pub time: i32,
    pub gold_reward: i32,
    pub exp_reward: i32,
    pub obj_reward: i32,
    pub prev_quest: i32,
    pub next_quest: i32,
}

/// The loaded world: arrays in file-load order (rnum parity with C) plus
/// order-robust vnum maps for lookup.
#[derive(Debug, Default)]
pub struct World {
    pub zones: Vec<Zone>,
    pub rooms: Vec<Room>,
    pub mob_protos: Vec<MobProto>,
    pub obj_protos: Vec<ObjProto>,
    pub triggers: Vec<Trigger>,
    pub shops: Vec<Shop>,
    pub quests: Vec<Quest>,
    pub room_map: HashMap<Idx, RoomRnum>,
    pub mob_map: HashMap<Idx, Idx>,
    pub obj_map: HashMap<Idx, Idx>,
    pub trig_map: HashMap<Idx, Idx>,
    /// SYSERR lines a parser produced while correcting a malformed record.
    /// The parsers have no logger to hand, so these are collected here and
    /// drained by `boot_world`.
    pub load_warnings: Vec<String>,
}

impl World {
    pub fn real_room(&self, vnum: Idx) -> Option<RoomRnum> {
        self.room_map.get(&vnum).copied()
    }
    pub fn real_mobile(&self, vnum: Idx) -> Option<Idx> {
        self.mob_map.get(&vnum).copied()
    }
    pub fn real_object(&self, vnum: Idx) -> Option<Idx> {
        self.obj_map.get(&vnum).copied()
    }
    pub fn real_trigger(&self, vnum: Idx) -> Option<Idx> {
        self.trig_map.get(&vnum).copied()
    }
    pub fn real_zone(&self, vnum: Idx) -> Option<ZoneRnum> {
        self.zones.iter().position(|z| z.number == vnum).map(|i| i as ZoneRnum)
    }
}
