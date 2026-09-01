//! Flag bit numbers and the 128-bit flag-set type.
//!
//! Most flag families are stored as `int[4]` bit arrays indexed by bit
//! number; a few legacy
//! families (exit info, container values, generic_find) are plain `1<<n`
//! masks. `FlagSet` mirrors the array form exactly — including its
//! serialization as four space-separated ints in pfiles and world files.

/// 128-bit flag set stored as 4×u32, matching the on-disk `int[4]` layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlagSet(pub [u32; 4]);

impl FlagSet {
    pub const EMPTY: FlagSet = FlagSet([0; 4]);

    pub fn from_words(words: [u32; 4]) -> Self {
        Self(words)
    }

    /// Single-word constructor for legacy single-int sources.
    pub fn from_single(word: u32) -> Self {
        Self([word, 0, 0, 0])
    }

    pub fn is_set(&self, bit: usize) -> bool {
        self.0[bit / 32] & (1 << (bit % 32)) != 0
    }

    pub fn set(&mut self, bit: usize) {
        self.0[bit / 32] |= 1 << (bit % 32);
    }

    pub fn remove(&mut self, bit: usize) {
        self.0[bit / 32] &= !(1 << (bit % 32));
    }

    pub fn toggle(&mut self, bit: usize) {
        self.0[bit / 32] ^= 1 << (bit % 32);
    }

    pub fn is_empty(&self) -> bool {
        self.0 == [0; 4]
    }
}

// Room flags
pub const ROOM_DARK: usize = 0;
pub const ROOM_DEATH: usize = 1;
pub const ROOM_NOMOB: usize = 2;
pub const ROOM_INDOORS: usize = 3;
pub const ROOM_PEACEFUL: usize = 4;
pub const ROOM_SOUNDPROOF: usize = 5;
pub const ROOM_NOTRACK: usize = 6;
pub const ROOM_NOMAGIC: usize = 7;
pub const ROOM_TUNNEL: usize = 8;
pub const ROOM_PRIVATE: usize = 9;
pub const ROOM_GODROOM: usize = 10;
pub const ROOM_HOUSE: usize = 11;
pub const ROOM_HOUSE_CRASH: usize = 12;
pub const ROOM_ATRIUM: usize = 13;
pub const ROOM_OLC: usize = 14;
pub const ROOM_BFS_MARK: usize = 15;
pub const ROOM_WORLDMAP: usize = 16;
pub const NUM_ROOM_FLAGS: usize = 17;

// Zone flags
pub const ZONE_CLOSED: usize = 0;
pub const ZONE_NOIMMORT: usize = 1;
pub const ZONE_QUEST: usize = 2;
pub const ZONE_GRID: usize = 3;
pub const ZONE_NOBUILD: usize = 4;
pub const ZONE_NOASTRAL: usize = 5;
pub const ZONE_WORLDMAP: usize = 6;
pub const NUM_ZONE_FLAGS: usize = 7;

// Exit info — legacy mask family
pub const EX_ISDOOR: u16 = 1 << 0;
pub const EX_CLOSED: u16 = 1 << 1;
pub const EX_LOCKED: u16 = 1 << 2;
pub const EX_PICKPROOF: u16 = 1 << 3;
pub const EX_HIDDEN: u16 = 1 << 4;

// Sector types — values, not bits.
pub const SECT_INSIDE: i32 = 0;
pub const SECT_CITY: i32 = 1;
pub const SECT_FIELD: i32 = 2;
pub const SECT_FOREST: i32 = 3;
pub const SECT_HILLS: i32 = 4;
pub const SECT_MOUNTAIN: i32 = 5;
pub const SECT_WATER_SWIM: i32 = 6;
pub const SECT_WATER_NOSWIM: i32 = 7;
pub const SECT_FLYING: i32 = 8;
pub const SECT_UNDERWATER: i32 = 9;
pub const NUM_ROOM_SECTORS: usize = 10;

// Player flags
pub const PLR_KILLER: usize = 0;
pub const PLR_THIEF: usize = 1;
pub const PLR_FROZEN: usize = 2;
pub const PLR_DONTSET: usize = 3;
pub const PLR_WRITING: usize = 4;
pub const PLR_MAILING: usize = 5;
pub const PLR_CRASH: usize = 6;
pub const PLR_SITEOK: usize = 7;
pub const PLR_NOSHOUT: usize = 8;
pub const PLR_NOTITLE: usize = 9;
pub const PLR_DELETED: usize = 10;
pub const PLR_LOADROOM: usize = 11;
pub const PLR_NOWIZLIST: usize = 12;
pub const PLR_NODELETE: usize = 13;
pub const PLR_INVSTART: usize = 14;
pub const PLR_CRYO: usize = 15;
pub const PLR_NOTDEADYET: usize = 16;
pub const PLR_BUG: usize = 17;
pub const PLR_IDEA: usize = 18;
pub const PLR_TYPO: usize = 19;

// Mobile flags
pub const MOB_SPEC: usize = 0;
pub const MOB_SENTINEL: usize = 1;
pub const MOB_SCAVENGER: usize = 2;
pub const MOB_ISNPC: usize = 3;
pub const MOB_AWARE: usize = 4;
pub const MOB_AGGRESSIVE: usize = 5;
pub const MOB_STAY_ZONE: usize = 6;
pub const MOB_WIMPY: usize = 7;
pub const MOB_AGGR_EVIL: usize = 8;
pub const MOB_AGGR_GOOD: usize = 9;
pub const MOB_AGGR_NEUTRAL: usize = 10;
pub const MOB_MEMORY: usize = 11;
pub const MOB_HELPER: usize = 12;
pub const MOB_NOCHARM: usize = 13;
pub const MOB_NOSUMMON: usize = 14;
pub const MOB_NOSLEEP: usize = 15;
pub const MOB_NOBASH: usize = 16;
pub const MOB_NOBLIND: usize = 17;
pub const MOB_NOKILL: usize = 18;
pub const MOB_NOTDEADYET: usize = 19;
pub const NUM_MOB_FLAGS: usize = 19; // excludes the NOTDEADYET tombstone, as in C

// Preference flags
pub const PRF_BRIEF: usize = 0;
pub const PRF_COMPACT: usize = 1;
pub const PRF_NOSHOUT: usize = 2;
pub const PRF_NOTELL: usize = 3;
pub const PRF_DISPHP: usize = 4;
pub const PRF_DISPMANA: usize = 5;
pub const PRF_DISPMOVE: usize = 6;
pub const PRF_AUTOEXIT: usize = 7;
pub const PRF_NOHASSLE: usize = 8;
pub const PRF_QUEST: usize = 9;
pub const PRF_SUMMONABLE: usize = 10;
pub const PRF_NOREPEAT: usize = 11;
pub const PRF_HOLYLIGHT: usize = 12;
pub const PRF_COLOR_1: usize = 13;
pub const PRF_COLOR_2: usize = 14;
pub const PRF_NOWIZ: usize = 15;
pub const PRF_LOG1: usize = 16;
pub const PRF_LOG2: usize = 17;
pub const PRF_NOAUCT: usize = 18;
pub const PRF_NOGOSS: usize = 19;
pub const PRF_NOGRATZ: usize = 20;
pub const PRF_SHOWVNUMS: usize = 21;
pub const PRF_DISPAUTO: usize = 22;
pub const PRF_CLS: usize = 23;
pub const PRF_BUILDWALK: usize = 24;
pub const PRF_AFK: usize = 25;
pub const PRF_AUTOLOOT: usize = 26;
pub const PRF_AUTOGOLD: usize = 27;
pub const PRF_AUTOSPLIT: usize = 28;
pub const PRF_AUTOSAC: usize = 29;
pub const PRF_AUTOASSIST: usize = 30;
pub const PRF_AUTOMAP: usize = 31;
pub const PRF_AUTOKEY: usize = 32;
pub const PRF_AUTODOOR: usize = 33;
pub const PRF_ZONERESETS: usize = 34;
pub const PRF_VERBOSE: usize = 35;
pub const NUM_PRF_FLAGS: usize = 36;

// Affect flags
pub const AFF_DONTUSE: usize = 0;
pub const AFF_BLIND: usize = 1;
pub const AFF_INVISIBLE: usize = 2;
pub const AFF_DETECT_ALIGN: usize = 3;
pub const AFF_DETECT_INVIS: usize = 4;
pub const AFF_DETECT_MAGIC: usize = 5;
pub const AFF_SENSE_LIFE: usize = 6;
pub const AFF_WATERWALK: usize = 7;
pub const AFF_SANCTUARY: usize = 8;
pub const AFF_GROUP: usize = 9; // AFF_UNUSED in structs.h; display name still "GROUP"
pub const AFF_CURSE: usize = 10;
pub const AFF_INFRAVISION: usize = 11;
pub const AFF_POISON: usize = 12;
pub const AFF_PROTECT_EVIL: usize = 13;
pub const AFF_PROTECT_GOOD: usize = 14;
pub const AFF_SLEEP: usize = 15;
pub const AFF_NOTRACK: usize = 16;
pub const AFF_FLYING: usize = 17;
pub const AFF_SCUBA: usize = 18;
pub const AFF_SNEAK: usize = 19;
pub const AFF_HIDE: usize = 20;
pub const AFF_FREE: usize = 21;
pub const AFF_CHARM: usize = 22;
pub const NUM_AFF_FLAGS: usize = 23;

// Item types — values.
/// No type, or one we do not know — `item_types[0]`, "UNDEFINED".
pub const ITEM_UNDEFINED: i32 = 0;
pub const ITEM_LIGHT: i32 = 1;
pub const ITEM_SCROLL: i32 = 2;
pub const ITEM_WAND: i32 = 3;
pub const ITEM_STAFF: i32 = 4;
pub const ITEM_WEAPON: i32 = 5;
pub const ITEM_FURNITURE: i32 = 6;
pub const ITEM_FREE: i32 = 7;
pub const ITEM_TREASURE: i32 = 8;
pub const ITEM_ARMOR: i32 = 9;
pub const ITEM_POTION: i32 = 10;
pub const ITEM_WORN: i32 = 11;
pub const ITEM_OTHER: i32 = 12;
pub const ITEM_TRASH: i32 = 13;
pub const ITEM_FREE2: i32 = 14;
pub const ITEM_CONTAINER: i32 = 15;
pub const ITEM_NOTE: i32 = 16;
pub const ITEM_DRINKCON: i32 = 17;
pub const ITEM_KEY: i32 = 18;
pub const ITEM_FOOD: i32 = 19;
pub const ITEM_MONEY: i32 = 20;
pub const ITEM_PEN: i32 = 21;
pub const ITEM_BOAT: i32 = 22;
pub const ITEM_FOUNTAIN: i32 = 23;
pub const NUM_ITEM_TYPES: usize = 24;

// Wear flags
pub const ITEM_WEAR_TAKE: usize = 0;
pub const ITEM_WEAR_FINGER: usize = 1;
pub const ITEM_WEAR_NECK: usize = 2;
pub const ITEM_WEAR_BODY: usize = 3;
pub const ITEM_WEAR_HEAD: usize = 4;
pub const ITEM_WEAR_LEGS: usize = 5;
pub const ITEM_WEAR_FEET: usize = 6;
pub const ITEM_WEAR_HANDS: usize = 7;
pub const ITEM_WEAR_ARMS: usize = 8;
pub const ITEM_WEAR_SHIELD: usize = 9;
pub const ITEM_WEAR_ABOUT: usize = 10;
pub const ITEM_WEAR_WAIST: usize = 11;
pub const ITEM_WEAR_WRIST: usize = 12;
pub const ITEM_WEAR_WIELD: usize = 13;
pub const ITEM_WEAR_HOLD: usize = 14;
pub const NUM_ITEM_WEARS: usize = 15;

// Extra object flags
pub const ITEM_GLOW: usize = 0;
pub const ITEM_HUM: usize = 1;
pub const ITEM_NORENT: usize = 2;
pub const ITEM_NODONATE: usize = 3;
pub const ITEM_NOINVIS: usize = 4;
pub const ITEM_INVISIBLE: usize = 5;
pub const ITEM_MAGIC: usize = 6;
pub const ITEM_NODROP: usize = 7;
pub const ITEM_BLESS: usize = 8;
pub const ITEM_ANTI_GOOD: usize = 9;
pub const ITEM_ANTI_EVIL: usize = 10;
pub const ITEM_ANTI_NEUTRAL: usize = 11;
pub const ITEM_ANTI_MAGIC_USER: usize = 12;
pub const ITEM_ANTI_CLERIC: usize = 13;
pub const ITEM_ANTI_THIEF: usize = 14;
pub const ITEM_ANTI_WARRIOR: usize = 15;
pub const ITEM_NOSELL: usize = 16;
pub const ITEM_QUEST: usize = 17;
pub const NUM_ITEM_FLAGS: usize = 18;

// Apply locations — values.
pub const APPLY_NONE: i32 = 0;
pub const APPLY_STR: i32 = 1;
pub const APPLY_DEX: i32 = 2;
pub const APPLY_INT: i32 = 3;
pub const APPLY_WIS: i32 = 4;
pub const APPLY_CON: i32 = 5;
pub const APPLY_CHA: i32 = 6;
pub const APPLY_CLASS: i32 = 7;
pub const APPLY_LEVEL: i32 = 8;
pub const APPLY_AGE: i32 = 9;
pub const APPLY_CHAR_WEIGHT: i32 = 10;
pub const APPLY_CHAR_HEIGHT: i32 = 11;
pub const APPLY_MANA: i32 = 12;
pub const APPLY_HIT: i32 = 13;
pub const APPLY_MOVE: i32 = 14;
pub const APPLY_GOLD: i32 = 15;
pub const APPLY_EXP: i32 = 16;
pub const APPLY_AC: i32 = 17;
pub const APPLY_HITROLL: i32 = 18;
pub const APPLY_DAMROLL: i32 = 19;
pub const APPLY_SAVING_PARA: i32 = 20;
pub const APPLY_SAVING_ROD: i32 = 21;
pub const APPLY_SAVING_PETRI: i32 = 22;
pub const APPLY_SAVING_BREATH: i32 = 23;
pub const APPLY_SAVING_SPELL: i32 = 24;
pub const NUM_APPLIES: usize = 25;

// Container flags — legacy mask family on value[1]
pub const CONT_CLOSEABLE: i32 = 1 << 0;
pub const CONT_PICKPROOF: i32 = 1 << 1;
pub const CONT_CLOSED: i32 = 1 << 2;
pub const CONT_LOCKED: i32 = 1 << 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flagset_bit_ops_cross_word_boundaries() {
        let mut f = FlagSet::EMPTY;
        f.set(PRF_VERBOSE); // bit 35 → word 1
        assert!(f.is_set(35));
        assert_eq!(f.0, [0, 1 << 3, 0, 0]);
        f.toggle(0);
        assert_eq!(f.0[0], 1);
        f.remove(35);
        assert!(f.is_empty() == false && !f.is_set(35));
    }

    #[test]
    fn counts_match_structs_h() {
        assert_eq!(NUM_PRF_FLAGS, 36);
        assert_eq!(NUM_AFF_FLAGS, 23);
        assert_eq!(NUM_MOB_FLAGS, 19);
        assert_eq!(NUM_ROOM_FLAGS, 17);
    }
}
