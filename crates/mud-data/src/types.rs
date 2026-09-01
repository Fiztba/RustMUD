//! Core scalar types and limit constants.
//!
//! Index types are unsigned shorts: vnums range 0..=65534 and 65535 is the
//! shared "none" sentinel for rooms, objects, mobs, and flags alike. World
//! files encode these numbers directly, so the widths are part of the wire
//! format.

/// Index type for all virtual/real numbers.
pub type Idx = u16;

pub type RoomVnum = Idx;
pub type ObjVnum = Idx;
pub type MobVnum = Idx;
pub type ZoneVnum = Idx;
pub type ShopVnum = Idx;
pub type TrigVnum = Idx;
pub type QstVnum = Idx;

pub type RoomRnum = Idx;
pub type ObjRnum = Idx;
pub type MobRnum = Idx;
pub type ZoneRnum = Idx;
pub type ShopRnum = Idx;
pub type TrigRnum = Idx;
pub type QstRnum = Idx;

pub const NOWHERE: Idx = Idx::MAX;
pub const NOTHING: Idx = Idx::MAX;
pub const NOBODY: Idx = Idx::MAX;
pub const NOFLAG: Idx = Idx::MAX;

// Directions
pub const NORTH: usize = 0;
pub const EAST: usize = 1;
pub const SOUTH: usize = 2;
pub const WEST: usize = 3;
pub const UP: usize = 4;
pub const DOWN: usize = 5;
pub const NORTHWEST: usize = 6;
pub const NORTHEAST: usize = 7;
pub const SOUTHEAST: usize = 8;
pub const SOUTHWEST: usize = 9;
pub const NUM_OF_DIRS: usize = 10;

// Immortal levels
pub const LVL_IMPL: u8 = 34;
pub const LVL_GRGOD: u8 = 33;
pub const LVL_GOD: u8 = 32;
pub const LVL_IMMORT: u8 = 31;
pub const LVL_BUILDER: u8 = LVL_IMMORT;

// Positions
pub const POS_DEAD: u8 = 0;
pub const POS_MORTALLYW: u8 = 1;
pub const POS_INCAP: u8 = 2;
pub const POS_STUNNED: u8 = 3;
pub const POS_SLEEPING: u8 = 4;
pub const POS_RESTING: u8 = 5;
pub const POS_SITTING: u8 = 6;
pub const POS_FIGHTING: u8 = 7;
pub const POS_STANDING: u8 = 8;
pub const NUM_POSITIONS: usize = 9;

// Sex
pub const SEX_NEUTRAL: u8 = 0;
pub const SEX_MALE: u8 = 1;
pub const SEX_FEMALE: u8 = 2;
pub const NUM_GENDERS: usize = 3;

// PC classes. CLASS_UNDEFINED is -1.
pub const CLASS_UNDEFINED: i8 = -1;
pub const CLASS_MAGIC_USER: i8 = 0;
pub const CLASS_CLERIC: i8 = 1;
pub const CLASS_THIEF: i8 = 2;
pub const CLASS_WARRIOR: i8 = 3;
pub const NUM_CLASSES: usize = 4;

// Equipment wear positions
pub const WEAR_LIGHT: usize = 0;
pub const WEAR_FINGER_R: usize = 1;
pub const WEAR_FINGER_L: usize = 2;
pub const WEAR_NECK_1: usize = 3;
pub const WEAR_NECK_2: usize = 4;
pub const WEAR_BODY: usize = 5;
pub const WEAR_HEAD: usize = 6;
pub const WEAR_LEGS: usize = 7;
pub const WEAR_FEET: usize = 8;
pub const WEAR_HANDS: usize = 9;
pub const WEAR_ARMS: usize = 10;
pub const WEAR_SHIELD: usize = 11;
pub const WEAR_ABOUT: usize = 12;
pub const WEAR_WAIST: usize = 13;
pub const WEAR_WRIST_R: usize = 14;
pub const WEAR_WRIST_L: usize = 15;
pub const WEAR_WIELD: usize = 16;
pub const WEAR_HOLD: usize = 17;
pub const NUM_WEARS: usize = 18;

// Pulse engine. One pulse = 100ms; constants are in
// pulses. PULSE_DG_SCRIPT lives with the script engine.
pub const PASSES_PER_SEC: u64 = 10;
pub const PULSE_ZONE: u64 = 10 * PASSES_PER_SEC;
pub const PULSE_MOBILE: u64 = 10 * PASSES_PER_SEC;
pub const PULSE_VIOLENCE: u64 = 2 * PASSES_PER_SEC;
pub const PULSE_AUTOSAVE: u64 = 60 * PASSES_PER_SEC;
pub const PULSE_IDLEPWD: u64 = 15 * PASSES_PER_SEC;
pub const PULSE_SANITY: u64 = 30 * PASSES_PER_SEC;
pub const PULSE_USAGE: u64 = 5 * 60 * PASSES_PER_SEC;
pub const PULSE_TIMESAVE: u64 = 30 * 60 * PASSES_PER_SEC;
pub const PULSE_DG_SCRIPT: u64 = 13 * PASSES_PER_SEC;

// Mud calendar. 75 real seconds per mud hour; 35-day months;
// 17-month years.
/// Real-world spans — rent expiry and played-time math.
pub const SECS_PER_REAL_MIN: i64 = 60;
pub const SECS_PER_REAL_HOUR: i64 = 60 * SECS_PER_REAL_MIN;
pub const SECS_PER_REAL_DAY: i64 = 24 * SECS_PER_REAL_HOUR;

pub const SECS_PER_MUD_HOUR: u64 = 75;
pub const SECS_PER_MUD_DAY: u64 = 24 * SECS_PER_MUD_HOUR;
pub const SECS_PER_MUD_MONTH: u64 = 35 * SECS_PER_MUD_DAY;
pub const SECS_PER_MUD_YEAR: u64 = 17 * SECS_PER_MUD_MONTH;

// String/buffer limits. These are byte limits and are
// player-visible via truncation behavior — do not "fix".
pub const MAX_SOCK_BUF: usize = 24 * 1024;
pub const MAX_PROMPT_LENGTH: usize = 96;
pub const GARBAGE_SPACE: usize = 32;
pub const SMALL_BUFSIZE: usize = 1024;
pub const LARGE_BUFSIZE: usize = MAX_SOCK_BUF - GARBAGE_SPACE - MAX_PROMPT_LENGTH;
pub const MAX_STRING_LENGTH: usize = 49152;
pub const MAX_INPUT_LENGTH: usize = 512;
pub const MAX_RAW_INPUT_LENGTH: usize = 12 * 1024;
pub const MAX_MESSAGES: usize = 60;
pub const MAX_NAME_LENGTH: usize = 20;
pub const MAX_PWD_LENGTH: usize = 30;
pub const MAX_TITLE_LENGTH: usize = 80;
pub const HOST_LENGTH: usize = 40;
pub const BANNED_SITE_LENGTH: usize = 50;
pub const PLR_DESC_LENGTH: usize = 4096;
pub const MAX_SKILLS: usize = 200;
pub const MAX_AFFECT: usize = 32;
pub const MAX_OBJ_AFFECT: usize = 6;
pub const MAX_NOTE_LENGTH: usize = 4000;
pub const MAX_HELP_KEYWORDS: usize = 256;
pub const MAX_COMPLETED_QUESTS: usize = 1024;
pub const MAX_CMD_LENGTH: usize = 16384;
pub const MAX_BAG_ROWS: usize = 5;

pub const MAX_GOLD: i32 = 2_140_000_000;
pub const MAX_BANK: i32 = 2_140_000_000;

/// Spell/skill numbers. Stage 5 brings the full table; these are
/// referenced earlier (poisoned food/drink, pick lock).
pub const SPELL_POISON: i16 = 33;
pub const SKILL_PICK_LOCK: i16 = 135;

/// Connection states. Values are stable — they gate
/// nanny dispatch and appear in `users` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConState {
    Playing = 0,
    Close = 1,
    GetName = 2,
    NameCnfrm = 3,
    Password = 4,
    Newpasswd = 5,
    Cnfpasswd = 6,
    Qsex = 7,
    Qclass = 8,
    Rmotd = 9,
    Menu = 10,
    PlrDesc = 11,
    ChpwdGetold = 12,
    ChpwdGetnew = 13,
    ChpwdVrfy = 14,
    Delcnf1 = 15,
    Delcnf2 = 16,
    Disconnect = 17,
    Oedit = 18,
    Redit = 19,
    Zedit = 20,
    Medit = 21,
    Sedit = 22,
    Tedit = 23,
    Cedit = 24,
    Aedit = 25,
    Trigedit = 26,
    Hedit = 27,
    Qedit = 28,
    Prefedit = 29,
    Ibtedit = 30,
    Msgedit = 31,
    GetProtocol = 32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinels_are_u16_max() {
        assert_eq!(NOWHERE, 65535);
        assert_eq!(LARGE_BUFSIZE, 24448);
    }

    #[test]
    fn calendar_math() {
        assert_eq!(SECS_PER_MUD_DAY, 1800);
        assert_eq!(SECS_PER_MUD_YEAR, 75 * 24 * 35 * 17);
    }
}
