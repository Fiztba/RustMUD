//! Runtime character data, shared by PCs and NPCs.
//! Field shapes follow the on-disk and wire formats; cross-references are
//! generational IDs instead of pointers.

use mud_data::flags::FlagSet;
use mud_data::types::{Idx, RoomRnum, MAX_SKILLS, NOWHERE};

use mud_data::ids::{CharId, ObjId};

pub type BStr = Vec<u8>;

/// Condition indices (DRUNK 0, HUNGER 1, THIRST 2 — note
/// order is DRUNK, HUNGER, THIRST for GET_COND).
pub const DRUNK: usize = 0;
pub const HUNGER: usize = 1;
pub const THIRST: usize = 2;

/// One spell affect on a character (`affected_type`).
#[derive(Debug, Clone, Default)]
pub struct Affect {
    pub spell: i16,
    pub duration: i16,
    pub modifier: i8,
    pub location: u8,
    pub bitvector: FlagSet,
}

/// Ability scores (`char_ability_data`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Abilities {
    pub str_: i8,
    pub str_add: i8,
    pub intel: i8,
    pub wis: i8,
    pub dex: i8,
    pub con: i8,
    pub cha: i8,
}

/// Points (`char_point_data`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Points {
    pub mana: i32,
    pub max_mana: i32,
    pub hit: i32,
    pub max_hit: i32,
    pub mov: i32,
    pub max_move: i32,
    pub armor: i32,
    pub gold: i32,
    pub bank_gold: i32,
    pub exp: i32,
    pub hitroll: i8,
    pub damroll: i8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TimeData {
    pub birth: i64,
    pub logon: i64,
    /// Seconds of play time accumulated at last logon refresh.
    pub played: i32,
}

/// One saved alias (`alias_data`). The replacement keeps its leading
/// space (study 02 §9.1).
#[derive(Debug, Clone)]
pub struct Alias {
    pub alias: BStr,
    pub replacement: BStr,
    pub type_: i32,
}

pub const ALIAS_SIMPLE: i32 = 0;
pub const ALIAS_COMPLEX: i32 = 1;

/// PC-only data (`player_special_data` and its `..._saved`).
#[derive(Debug, Clone)]
pub struct PlayerSpecials {
    pub skills: [i8; MAX_SKILLS + 1],
    pub wimp_level: i32,
    pub freeze_level: i8,
    pub invis_level: i16,
    /// Loadroom as a vnum; NOWHERE = none.
    pub load_room: Idx,
    pub pref: FlagSet,
    pub bad_pws: u8,
    /// DRUNK/HUNGER/THIRST, -1 = not subject.
    pub conditions: [i16; 3],
    /// Practice sessions ("spells_to_learn").
    pub practices: i32,
    pub olc_zone: i32,
    /// GET_BUILDWALK_SECTOR: the sector new buildwalk
    /// rooms get. Not persisted to the pfile.
    pub buildwalk_sector: i32,
    pub page_length: i32,
    pub screen_width: i32,
    pub questpoints: i32,
    pub num_completed_quests: i32,
    pub completed_quests: Vec<Idx>,
    pub current_quest: Idx,
    pub quest_time: i32,
    pub quest_counter: i32,
    pub last_motd: i64,
    pub last_news: i64,
    pub poofin: Option<BStr>,
    pub poofout: Option<BStr>,
    pub aliases: Vec<Alias>,
    pub last_tell: i64,
    pub host: Option<BStr>,
    /// Per-channel communication history (comm_hist[NUM_HIST]).
    pub comm_hist: [Vec<BStr>; 9],
}

impl Default for PlayerSpecials {
    fn default() -> Self {
        Self {
            skills: [0; MAX_SKILLS + 1],
            wimp_level: 0,
            freeze_level: 0,
            invis_level: 0,
            load_room: 0,
            pref: FlagSet::EMPTY,
            bad_pws: 0,
            conditions: [0; 3],
            practices: 0,
            // A fresh char's olc_zone starts at 0; the NOWHERE
            // default is a pfile-load default, applied there.
            olc_zone: 0,
            buildwalk_sector: 0,
            page_length: 22,
            screen_width: 80,
            questpoints: 0,
            num_completed_quests: 0,
            completed_quests: Vec::new(),
            current_quest: mud_data::types::NOTHING,
            quest_time: -1,
            quest_counter: 0,
            last_motd: 0,
            last_news: 0,
            poofin: None,
            poofout: None,
            aliases: Vec::new(),
            last_tell: NOBODY_TELL,
            host: None,
            comm_hist: Default::default(),
        }
    }
}

/// last_tell starts at NOBODY (65535); tells compare against idnums.
pub const NOBODY_TELL: i64 = 65535;

/// NPC-only data (`mob_special_data`).
#[derive(Debug, Clone, Default)]
pub struct MobSpecials {
    pub attack_type: i32,
    pub default_pos: u8,
    pub damnodice: i8,
    pub damsizedice: i8,
    /// PCs remembered by MOB_MEMORY mobs (idnums; stage 4).
    pub memory: Vec<i64>,
    /// The AFF bits this mob was read with, copied off the prototype at
    /// `read_mobile` and carried on the mob from then on.
    ///
    /// Asked of the mob rather than of `mob_protos[mob_rnum]`, because the
    /// two drift: `mtransform` replaces the body and then restores the old
    /// rnum, so a transformed mob wears one prototype's flags under another's
    /// number, and medit writes an edited prototype back under mobs already
    /// spawned from it. Reading the prototype would call a flag innate that
    /// the mob was never made with -- adding sanctuary to a dog would give it
    /// to a bear that used to be a dog -- and the guard below would then
    /// refuse to drop it. Empty for players, who have no mob file.
    pub innate_aff: FlagSet,
}

/// The runtime character.
#[derive(Debug, Clone)]
pub struct Char {
    /// Prototype rnum; NOBODY (u16::MAX) for PCs.
    pub mob_rnum: Idx,
    /// PC: player name (capitalized). NPC: keyword list.
    pub name: Option<BStr>,
    /// NPC only.
    pub short_descr: Option<BStr>,
    /// NPC room line.
    pub long_descr: Option<BStr>,
    pub description: Option<BStr>,
    pub title: Option<BStr>,
    pub passwd: BStr,
    pub sex: u8,
    pub class: i8,
    pub level: u8,
    pub time: TimeData,
    pub weight: u8,
    pub height: u8,

    pub real_abils: Abilities,
    pub aff_abils: Abilities,
    pub points: Points,

    // char_special_data
    pub fighting: Option<CharId>,
    pub hunting: Option<CharId>,
    pub position: u8,
    pub carry_weight: i32,
    pub carry_items: u8,
    pub timer: i32,
    pub alignment: i32,
    pub idnum: i64,
    /// GET_PFILEPOS: the player-index slot, -1 until the character is
    /// indexed. save_char refuses to write a character without one
    /// is how `set file` gates its write-back.
    pub pfilepos: i32,
    /// ch->pref: the per-login unique number the last-log keys
    /// its rows on. Rolled at character creation, not persisted.
    pub punique: i32,
    /// PLR_* for PCs / MOB_* for NPCs.
    pub act: FlagSet,
    pub affected_by: FlagSet,
    pub apply_saving_throw: [i16; 5],

    pub player_specials: Option<Box<PlayerSpecials>>,
    pub mob_specials: MobSpecials,

    pub affected: Vec<Affect>,
    pub equipment: [Option<ObjId>; mud_data::types::NUM_WEARS],

    pub in_room: RoomRnum,
    pub was_in_room: RoomRnum,
    pub wait: i32,

    pub carrying: Vec<ObjId>,
    pub desc: Option<usize>,
    pub followers: Vec<CharId>,
    pub master: Option<CharId>,
    /// GROUP(ch): id into Game::groups (Vatiken group system).
    pub group: Option<u64>,
    /// Trigger vnums attached at runtime (zone T commands / proto_script).
    pub proto_script: Vec<Idx>,
    /// DG script id (players: idnum; mobs: assigned from a rolling counter).
    pub script_id: i64,
    /// DG script container.
    pub script: Option<Box<crate::dg::ScriptData>>,
    /// mremember records.
    pub script_mem: Vec<crate::dg::ScriptMem>,
}

impl Default for Char {
    fn default() -> Self {
        Self {
            mob_rnum: mud_data::types::NOBODY,
            name: None,
            short_descr: None,
            long_descr: None,
            description: None,
            title: None,
            passwd: Vec::new(),
            sex: 0,
            class: 0,
            level: 0,
            time: TimeData::default(),
            weight: 0,
            height: 0,
            real_abils: Abilities::default(),
            aff_abils: Abilities::default(),
            points: Points::default(),
            fighting: None,
            hunting: None,
            position: mud_data::types::POS_STANDING,
            carry_weight: 0,
            carry_items: 0,
            timer: 0,
            alignment: 0,
            idnum: 0,
            pfilepos: -1,
            punique: 0,
            act: FlagSet::EMPTY,
            affected_by: FlagSet::EMPTY,
            apply_saving_throw: [0; 5],
            player_specials: None,
            mob_specials: MobSpecials::default(),
            affected: Vec::new(),
            equipment: [None; mud_data::types::NUM_WEARS],
            in_room: NOWHERE,
            was_in_room: NOWHERE,
            wait: 0,
            carrying: Vec::new(),
            desc: None,
            followers: Vec::new(),
            master: None,
            group: None,
            proto_script: Vec::new(),
            script_id: 0,
            script: None,
            script_mem: Vec::new(),
        }
    }
}

impl Char {
    pub fn is_npc(&self) -> bool {
        self.act.is_set(mud_data::flags::MOB_ISNPC)
    }

    /// PC name, or NPC short description.
    pub fn get_name(&self) -> &[u8] {
        if self.is_npc() {
            self.short_descr.as_deref().unwrap_or(b"<None>")
        } else {
            self.name.as_deref().unwrap_or(b"<None>")
        }
    }

    pub fn prf(&self, bit: usize) -> bool {
        !self.is_npc()
            && self.player_specials.as_ref().is_some_and(|ps| ps.pref.is_set(bit))
    }

    pub fn plr(&self, bit: usize) -> bool {
        !self.is_npc() && self.act.is_set(bit)
    }

    pub fn mob_flagged(&self, bit: usize) -> bool {
        self.is_npc() && self.act.is_set(bit)
    }

    pub fn aff(&self, bit: usize) -> bool {
        self.affected_by.is_set(bit)
    }

    pub fn awake(&self) -> bool {
        self.position > mud_data::types::POS_SLEEPING
    }

    /// GET_INVIS_LEV (0 for NPCs).
    pub fn invis_lev(&self) -> i16 {
        self.player_specials.as_ref().map_or(0, |ps| ps.invis_level)
    }

    pub fn ps(&self) -> &PlayerSpecials {
        self.player_specials.as_ref().expect("player_specials on NPC")
    }

    pub fn ps_mut(&mut self) -> &mut PlayerSpecials {
        self.player_specials.as_mut().expect("player_specials on NPC")
    }

    /// GET_SKILL: NPCs share a zeroed dummy — always 0.
    pub fn get_skill(&self, num: i32) -> i32 {
        if self.is_npc() {
            return 0;
        }
        self.player_specials
            .as_ref()
            .and_then(|ps| ps.skills.get(num as usize))
            .map(|&v| v as i32)
            .unwrap_or(0)
    }

    pub fn set_skill(&mut self, num: i32, val: i32) {
        if let Some(ps) = self.player_specials.as_mut() {
            if let Some(slot) = ps.skills.get_mut(num as usize) {
                *slot = val as i8;
            }
        }
    }

    /// COLOR_LEV: PRF_COLOR_1 + 2*PRF_COLOR_2 (0 for NPCs).
    pub fn color_lev(&self) -> u8 {
        if self.is_npc() {
            return 0;
        }
        (self.prf(mud_data::flags::PRF_COLOR_1) as u8)
            + 2 * (self.prf(mud_data::flags::PRF_COLOR_2) as u8)
    }
}
