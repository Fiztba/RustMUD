//! Spell/skill numbering, names, and the spell_info table. The dispatch
//! routines arrive with stage 5; the data is complete now because stage 4
//! needs names, per-class min levels (guild practice / list_skills) and
//! wear-off messages (affect_update).

pub const TOP_SPELL_DEFINE: i32 = 299;
pub const MAX_SPELLS: i32 = 130;

// Spell numbers.
pub const SPELL_ARMOR: i32 = 1;
pub const SPELL_TELEPORT: i32 = 2;
pub const SPELL_BLESS: i32 = 3;
pub const SPELL_BLINDNESS: i32 = 4;
pub const SPELL_BURNING_HANDS: i32 = 5;
pub const SPELL_CALL_LIGHTNING: i32 = 6;
pub const SPELL_CHARM: i32 = 7;
pub const SPELL_CHILL_TOUCH: i32 = 8;
pub const SPELL_CLONE: i32 = 9;
pub const SPELL_COLOR_SPRAY: i32 = 10;
pub const SPELL_CONTROL_WEATHER: i32 = 11;
pub const SPELL_CREATE_FOOD: i32 = 12;
pub const SPELL_CREATE_WATER: i32 = 13;
pub const SPELL_CURE_BLIND: i32 = 14;
pub const SPELL_CURE_CRITIC: i32 = 15;
pub const SPELL_CURE_LIGHT: i32 = 16;
pub const SPELL_CURSE: i32 = 17;
pub const SPELL_DETECT_ALIGN: i32 = 18;
pub const SPELL_DETECT_INVIS: i32 = 19;
pub const SPELL_DETECT_MAGIC: i32 = 20;
pub const SPELL_DETECT_POISON: i32 = 21;
pub const SPELL_DISPEL_EVIL: i32 = 22;
pub const SPELL_EARTHQUAKE: i32 = 23;
pub const SPELL_ENCHANT_WEAPON: i32 = 24;
pub const SPELL_ENERGY_DRAIN: i32 = 25;
pub const SPELL_FIREBALL: i32 = 26;
pub const SPELL_HARM: i32 = 27;
pub const SPELL_HEAL: i32 = 28;
pub const SPELL_INVISIBLE: i32 = 29;
pub const SPELL_LIGHTNING_BOLT: i32 = 30;
pub const SPELL_LOCATE_OBJECT: i32 = 31;
pub const SPELL_MAGIC_MISSILE: i32 = 32;
pub const SPELL_POISON: i32 = 33;
pub const SPELL_PROT_FROM_EVIL: i32 = 34;
pub const SPELL_REMOVE_CURSE: i32 = 35;
pub const SPELL_SANCTUARY: i32 = 36;
pub const SPELL_SHOCKING_GRASP: i32 = 37;
pub const SPELL_SLEEP: i32 = 38;
pub const SPELL_STRENGTH: i32 = 39;
pub const SPELL_SUMMON: i32 = 40;
pub const SPELL_VENTRILOQUATE: i32 = 41;
pub const SPELL_WORD_OF_RECALL: i32 = 42;
pub const SPELL_REMOVE_POISON: i32 = 43;
pub const SPELL_SENSE_LIFE: i32 = 44;
pub const SPELL_ANIMATE_DEAD: i32 = 45;
pub const SPELL_DISPEL_GOOD: i32 = 46;
pub const SPELL_GROUP_ARMOR: i32 = 47;
pub const SPELL_GROUP_HEAL: i32 = 48;
pub const SPELL_GROUP_RECALL: i32 = 49;
pub const SPELL_INFRAVISION: i32 = 50;
pub const SPELL_WATERWALK: i32 = 51;
pub const SPELL_IDENTIFY: i32 = 52;
pub const SPELL_FLY: i32 = 53;
pub const SPELL_DARKNESS: i32 = 54;
pub const SPELL_DG_AFFECT: i32 = 298;

// Skill numbers.
pub const SKILL_BACKSTAB: i32 = 131;
pub const SKILL_BASH: i32 = 132;
pub const SKILL_HIDE: i32 = 133;
pub const SKILL_KICK: i32 = 134;
pub const SKILL_PICK_LOCK: i32 = 135;
pub const SKILL_WHIRLWIND: i32 = 136;
pub const SKILL_RESCUE: i32 = 137;
pub const SKILL_SNEAK: i32 = 138;
pub const SKILL_STEAL: i32 = 139;
pub const SKILL_TRACK: i32 = 140;
pub const SKILL_BANDAGE: i32 = 141;

// Attack/weapon types.
pub const TYPE_HIT: i32 = 300;
pub const TYPE_STING: i32 = 301;
pub const TYPE_WHIP: i32 = 302;
pub const TYPE_SLASH: i32 = 303;
pub const TYPE_BITE: i32 = 304;
pub const TYPE_BLUDGEON: i32 = 305;
pub const TYPE_CRUSH: i32 = 306;
pub const TYPE_POUND: i32 = 307;
pub const TYPE_CLAW: i32 = 308;
pub const TYPE_MAUL: i32 = 309;
pub const TYPE_THRASH: i32 = 310;
pub const TYPE_PIERCE: i32 = 311;
pub const TYPE_BLAST: i32 = 312;
pub const TYPE_PUNCH: i32 = 313;
pub const TYPE_STAB: i32 = 314;
pub const TYPE_SUFFERING: i32 = 399;
pub const TYPE_UNDEFINED: i32 = -1;

// Cast types.
pub const CAST_UNDEFINED: i32 = -1;
pub const CAST_SPELL: i32 = 0;
pub const CAST_POTION: i32 = 1;
pub const CAST_WAND: i32 = 2;
pub const CAST_STAFF: i32 = 3;
pub const CAST_SCROLL: i32 = 4;

/// DEFAULT_STAFF_LVL / DEFAULT_WAND_LVL: the comment says
/// 14, the code says 12 — 12 is what runs.
pub const DEFAULT_STAFF_LVL: i32 = 12;
pub const DEFAULT_WAND_LVL: i32 = 12;

// Saving throw types.
pub const SAVING_PARA: i32 = 0;
pub const SAVING_ROD: i32 = 1;
pub const SAVING_PETRI: i32 = 2;
pub const SAVING_BREATH: i32 = 3;
pub const SAVING_SPELL: i32 = 4;

// Routine bits.
pub const MAG_DAMAGE: i32 = 1 << 0;
pub const MAG_AFFECTS: i32 = 1 << 1;
pub const MAG_UNAFFECTS: i32 = 1 << 2;
pub const MAG_POINTS: i32 = 1 << 3;
pub const MAG_ALTER_OBJS: i32 = 1 << 4;
pub const MAG_GROUPS: i32 = 1 << 5;
pub const MAG_MASSES: i32 = 1 << 6;
pub const MAG_AREAS: i32 = 1 << 7;
pub const MAG_SUMMONS: i32 = 1 << 8;
pub const MAG_CREATIONS: i32 = 1 << 9;
pub const MAG_MANUAL: i32 = 1 << 10;
pub const MAG_ROOMS: i32 = 1 << 11;

// Target bits.
pub const TAR_IGNORE: i32 = 1 << 0;
pub const TAR_CHAR_ROOM: i32 = 1 << 1;
pub const TAR_CHAR_WORLD: i32 = 1 << 2;
pub const TAR_FIGHT_SELF: i32 = 1 << 3;
pub const TAR_FIGHT_VICT: i32 = 1 << 4;
pub const TAR_SELF_ONLY: i32 = 1 << 5;
pub const TAR_NOT_SELF: i32 = 1 << 6;
pub const TAR_OBJ_INV: i32 = 1 << 7;
pub const TAR_OBJ_ROOM: i32 = 1 << 8;
pub const TAR_OBJ_WORLD: i32 = 1 << 9;
pub const TAR_OBJ_EQUIP: i32 = 1 << 10;

pub const UNUSED_SPELLNAME: &str = "!UNUSED!";

/// struct spell_info_type as data.
#[derive(Debug, Clone, Copy)]
pub struct SpellInfo {
    pub mana_max: i32,
    pub mana_min: i32,
    pub mana_change: i32,
    /// min_level per class [Mu, Cl, Th, Wa].
    pub min_level: [i32; 4],
    pub min_position: u8,
    pub targets: i32,
    pub violent: bool,
    pub routines: i32,
    pub name: &'static str,
    pub wear_off_msg: Option<&'static str>,
}

const LVL_IMMORT: i32 = 31;
const LVL_IMPL: i32 = 34;
const POS_SITTING: u8 = 6;
const POS_FIGHTING: u8 = 7;
const POS_STANDING: u8 = 8;

const UNUSED: SpellInfo = SpellInfo {
    mana_max: 0,
    mana_min: 0,
    mana_change: 0,
    min_level: [LVL_IMPL + 1; 4],
    min_position: 0,
    targets: 0,
    violent: false,
    routines: 0,
    name: UNUSED_SPELLNAME,
    wear_off_msg: None,
};

struct SpellTable([SpellInfo; TOP_SPELL_DEFINE as usize + 1]);

impl SpellTable {
    /// spello.
    #[allow(clippy::too_many_arguments)]
    fn spello(
        &mut self,
        spl: i32,
        name: &'static str,
        max_mana: i32,
        min_mana: i32,
        mana_change: i32,
        minpos: u8,
        targets: i32,
        violent: bool,
        routines: i32,
        wearoff: Option<&'static str>,
    ) {
        let e = &mut self.0[spl as usize];
        e.min_level = [LVL_IMMORT; 4];
        e.mana_max = max_mana;
        e.mana_min = min_mana;
        e.mana_change = mana_change;
        e.min_position = minpos;
        e.targets = targets;
        e.violent = violent;
        e.routines = routines;
        e.name = name;
        e.wear_off_msg = wearoff;
    }

    fn skillo(&mut self, skill: i32, name: &'static str) {
        self.spello(skill, name, 0, 0, 0, 0, 0, false, 0, None);
    }

    fn spell_level(&mut self, spell: i32, class: usize, level: i32) {
        self.0[spell as usize].min_level[class] = level;
    }
}

const CLASS_MAGIC_USER: usize = 0;
const CLASS_CLERIC: usize = 1;
const CLASS_THIEF: usize = 2;
const CLASS_WARRIOR: usize = 3;

/// Build the spell table, then apply per-class minimum levels.
fn build_table() -> SpellTable {
    let mut t = SpellTable([UNUSED; TOP_SPELL_DEFINE as usize + 1]);

    t.spello(SPELL_ANIMATE_DEAD, "animate dead", 35, 10, 3, POS_STANDING, TAR_OBJ_ROOM, false, MAG_SUMMONS, None);
    t.spello(SPELL_ARMOR, "armor", 30, 15, 3, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_AFFECTS, Some("You feel less protected."));
    t.spello(SPELL_BLESS, "bless", 35, 5, 3, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV, false, MAG_AFFECTS | MAG_ALTER_OBJS, Some("You feel less righteous."));
    t.spello(SPELL_BLINDNESS, "blindness", 35, 25, 1, POS_STANDING, TAR_CHAR_ROOM | TAR_NOT_SELF, false, MAG_AFFECTS, Some("You feel a cloak of blindness dissolve."));
    t.spello(SPELL_BURNING_HANDS, "burning hands", 30, 10, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_CALL_LIGHTNING, "call lightning", 40, 25, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_CHARM, "charm person", 75, 50, 2, POS_FIGHTING, TAR_CHAR_ROOM | TAR_NOT_SELF, true, MAG_MANUAL, Some("You feel more self-confident."));
    t.spello(SPELL_CHILL_TOUCH, "chill touch", 30, 10, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE | MAG_AFFECTS, Some("You feel your strength return."));
    t.spello(SPELL_CLONE, "clone", 80, 65, 5, POS_STANDING, TAR_IGNORE, false, MAG_SUMMONS, None);
    t.spello(SPELL_COLOR_SPRAY, "color spray", 30, 15, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_CONTROL_WEATHER, "control weather", 75, 25, 5, POS_STANDING, TAR_IGNORE, false, MAG_MANUAL, None);
    t.spello(SPELL_CREATE_FOOD, "create food", 30, 5, 4, POS_STANDING, TAR_IGNORE, false, MAG_CREATIONS, None);
    t.spello(SPELL_CREATE_WATER, "create water", 30, 5, 4, POS_STANDING, TAR_OBJ_INV | TAR_OBJ_EQUIP, false, MAG_MANUAL, None);
    t.spello(SPELL_CURE_BLIND, "cure blind", 30, 5, 2, POS_STANDING, TAR_CHAR_ROOM, false, MAG_UNAFFECTS, None);
    t.spello(SPELL_CURE_CRITIC, "cure critic", 30, 10, 2, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_POINTS, None);
    t.spello(SPELL_CURE_LIGHT, "cure light", 30, 10, 2, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_POINTS, None);
    t.spello(SPELL_CURSE, "curse", 80, 50, 2, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV, true, MAG_AFFECTS | MAG_ALTER_OBJS, Some("You feel more optimistic."));
    t.spello(SPELL_DARKNESS, "darkness", 30, 5, 4, POS_STANDING, TAR_IGNORE, false, MAG_ROOMS, None);
    t.spello(SPELL_DETECT_ALIGN, "detect alignment", 20, 10, 2, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("You feel less aware."));
    t.spello(SPELL_DETECT_INVIS, "detect invisibility", 20, 10, 2, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("Your eyes stop tingling."));
    t.spello(SPELL_DETECT_MAGIC, "detect magic", 20, 10, 2, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("The detect magic wears off."));
    t.spello(SPELL_DETECT_POISON, "detect poison", 15, 5, 1, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV | TAR_OBJ_ROOM, false, MAG_MANUAL, Some("The detect poison wears off."));
    t.spello(SPELL_DISPEL_EVIL, "dispel evil", 40, 25, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_DISPEL_GOOD, "dispel good", 40, 25, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_EARTHQUAKE, "earthquake", 40, 25, 3, POS_FIGHTING, TAR_IGNORE, true, MAG_AREAS, None);
    t.spello(SPELL_ENCHANT_WEAPON, "enchant weapon", 150, 100, 10, POS_STANDING, TAR_OBJ_INV, false, MAG_MANUAL, None);
    t.spello(SPELL_ENERGY_DRAIN, "energy drain", 40, 25, 1, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE | MAG_MANUAL, None);
    t.spello(SPELL_GROUP_ARMOR, "group armor", 50, 30, 2, POS_STANDING, TAR_IGNORE, false, MAG_GROUPS, None);
    t.spello(SPELL_FIREBALL, "fireball", 40, 30, 2, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_FLY, "fly", 40, 20, 2, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_AFFECTS, Some("You drift slowly to the ground."));
    t.spello(SPELL_GROUP_HEAL, "group heal", 80, 60, 5, POS_STANDING, TAR_IGNORE, false, MAG_GROUPS, None);
    t.spello(SPELL_HARM, "harm", 75, 45, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_HEAL, "heal", 60, 40, 3, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_POINTS | MAG_UNAFFECTS, None);
    t.spello(SPELL_INFRAVISION, "infravision", 25, 10, 1, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("Your night vision seems to fade."));
    t.spello(SPELL_INVISIBLE, "invisibility", 35, 25, 1, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV | TAR_OBJ_ROOM, false, MAG_AFFECTS | MAG_ALTER_OBJS, Some("You feel yourself exposed."));
    t.spello(SPELL_LIGHTNING_BOLT, "lightning bolt", 30, 15, 1, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_LOCATE_OBJECT, "locate object", 25, 20, 1, POS_STANDING, TAR_OBJ_WORLD, false, MAG_MANUAL, None);
    t.spello(SPELL_MAGIC_MISSILE, "magic missile", 25, 10, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_POISON, "poison", 50, 20, 3, POS_STANDING, TAR_CHAR_ROOM | TAR_NOT_SELF | TAR_OBJ_INV, true, MAG_AFFECTS | MAG_ALTER_OBJS, Some("You feel less sick."));
    t.spello(SPELL_PROT_FROM_EVIL, "protection from evil", 40, 10, 3, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("You feel less protected."));
    t.spello(SPELL_REMOVE_CURSE, "remove curse", 45, 25, 5, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV | TAR_OBJ_EQUIP, false, MAG_UNAFFECTS | MAG_ALTER_OBJS, None);
    t.spello(SPELL_REMOVE_POISON, "remove poison", 40, 8, 4, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV | TAR_OBJ_ROOM, false, MAG_UNAFFECTS | MAG_ALTER_OBJS, None);
    t.spello(SPELL_SANCTUARY, "sanctuary", 110, 85, 5, POS_STANDING, TAR_CHAR_ROOM, false, MAG_AFFECTS, Some("The white aura around your body fades."));
    t.spello(SPELL_SENSE_LIFE, "sense life", 20, 10, 2, POS_STANDING, TAR_CHAR_ROOM | TAR_SELF_ONLY, false, MAG_AFFECTS, Some("You feel less aware of your surroundings."));
    t.spello(SPELL_SHOCKING_GRASP, "shocking grasp", 30, 15, 3, POS_FIGHTING, TAR_CHAR_ROOM | TAR_FIGHT_VICT, true, MAG_DAMAGE, None);
    t.spello(SPELL_SLEEP, "sleep", 40, 25, 5, POS_STANDING, TAR_CHAR_ROOM, true, MAG_AFFECTS, Some("You feel less tired."));
    t.spello(SPELL_STRENGTH, "strength", 35, 30, 1, POS_STANDING, TAR_CHAR_ROOM, false, MAG_AFFECTS, Some("You feel weaker."));
    t.spello(SPELL_SUMMON, "summon", 75, 50, 3, POS_STANDING, TAR_CHAR_WORLD | TAR_NOT_SELF, false, MAG_MANUAL, None);
    t.spello(SPELL_TELEPORT, "teleport", 75, 50, 3, POS_STANDING, TAR_CHAR_ROOM, false, MAG_MANUAL, None);
    t.spello(SPELL_WATERWALK, "waterwalk", 40, 20, 2, POS_STANDING, TAR_CHAR_ROOM, false, MAG_AFFECTS, Some("Your feet seem less buoyant."));
    t.spello(SPELL_WORD_OF_RECALL, "word of recall", 20, 10, 2, POS_FIGHTING, TAR_CHAR_ROOM, false, MAG_MANUAL, None);
    // Identify is registered castable here. A leftover non-castable
    // registration further down (0 mana, min position 0) would otherwise
    // overwrite this line.
    t.spello(SPELL_IDENTIFY, "identify", 50, 25, 5, POS_STANDING, TAR_CHAR_ROOM | TAR_OBJ_INV | TAR_OBJ_ROOM, false, MAG_MANUAL, None);
    t.spello(SPELL_DG_AFFECT, "Script-inflicted", 0, 0, 0, POS_SITTING, TAR_IGNORE, true, 0, None);

    t.skillo(SKILL_BACKSTAB, "backstab");
    t.skillo(SKILL_BASH, "bash");
    t.skillo(SKILL_HIDE, "hide");
    t.skillo(SKILL_KICK, "kick");
    t.skillo(SKILL_PICK_LOCK, "pick lock");
    t.skillo(SKILL_RESCUE, "rescue");
    t.skillo(SKILL_SNEAK, "sneak");
    t.skillo(SKILL_STEAL, "steal");
    t.skillo(SKILL_TRACK, "track");
    t.skillo(SKILL_WHIRLWIND, "whirlwind");
    t.skillo(SKILL_BANDAGE, "bandage");

    // init_spell_levels — runs after mag_assign_spells at boot.
    // MAGES
    t.spell_level(SPELL_MAGIC_MISSILE, CLASS_MAGIC_USER, 1);
    t.spell_level(SPELL_DETECT_INVIS, CLASS_MAGIC_USER, 2);
    t.spell_level(SPELL_DETECT_MAGIC, CLASS_MAGIC_USER, 2);
    t.spell_level(SPELL_CHILL_TOUCH, CLASS_MAGIC_USER, 3);
    t.spell_level(SPELL_INFRAVISION, CLASS_MAGIC_USER, 3);
    t.spell_level(SPELL_INVISIBLE, CLASS_MAGIC_USER, 4);
    t.spell_level(SPELL_ARMOR, CLASS_MAGIC_USER, 4);
    t.spell_level(SPELL_BURNING_HANDS, CLASS_MAGIC_USER, 5);
    t.spell_level(SPELL_LOCATE_OBJECT, CLASS_MAGIC_USER, 6);
    t.spell_level(SPELL_STRENGTH, CLASS_MAGIC_USER, 6);
    t.spell_level(SPELL_SHOCKING_GRASP, CLASS_MAGIC_USER, 7);
    t.spell_level(SPELL_SLEEP, CLASS_MAGIC_USER, 8);
    t.spell_level(SPELL_LIGHTNING_BOLT, CLASS_MAGIC_USER, 9);
    t.spell_level(SPELL_BLINDNESS, CLASS_MAGIC_USER, 9);
    t.spell_level(SPELL_DETECT_POISON, CLASS_MAGIC_USER, 10);
    t.spell_level(SPELL_COLOR_SPRAY, CLASS_MAGIC_USER, 11);
    t.spell_level(SPELL_ENERGY_DRAIN, CLASS_MAGIC_USER, 13);
    t.spell_level(SPELL_CURSE, CLASS_MAGIC_USER, 14);
    t.spell_level(SPELL_POISON, CLASS_MAGIC_USER, 14);
    t.spell_level(SPELL_FIREBALL, CLASS_MAGIC_USER, 15);
    t.spell_level(SPELL_CHARM, CLASS_MAGIC_USER, 16);
    t.spell_level(SPELL_IDENTIFY, CLASS_MAGIC_USER, 20);
    t.spell_level(SPELL_FLY, CLASS_MAGIC_USER, 22);
    t.spell_level(SPELL_ENCHANT_WEAPON, CLASS_MAGIC_USER, 26);
    t.spell_level(SPELL_CLONE, CLASS_MAGIC_USER, 30);
    // CLERICS
    t.spell_level(SPELL_CURE_LIGHT, CLASS_CLERIC, 1);
    t.spell_level(SPELL_ARMOR, CLASS_CLERIC, 1);
    t.spell_level(SPELL_CREATE_FOOD, CLASS_CLERIC, 2);
    t.spell_level(SPELL_CREATE_WATER, CLASS_CLERIC, 2);
    t.spell_level(SPELL_DETECT_POISON, CLASS_CLERIC, 3);
    t.spell_level(SPELL_DETECT_ALIGN, CLASS_CLERIC, 4);
    t.spell_level(SPELL_CURE_BLIND, CLASS_CLERIC, 4);
    t.spell_level(SPELL_BLESS, CLASS_CLERIC, 5);
    t.spell_level(SPELL_DETECT_INVIS, CLASS_CLERIC, 6);
    t.spell_level(SPELL_BLINDNESS, CLASS_CLERIC, 6);
    t.spell_level(SPELL_INFRAVISION, CLASS_CLERIC, 7);
    t.spell_level(SPELL_PROT_FROM_EVIL, CLASS_CLERIC, 8);
    t.spell_level(SPELL_POISON, CLASS_CLERIC, 8);
    t.spell_level(SPELL_GROUP_ARMOR, CLASS_CLERIC, 9);
    t.spell_level(SPELL_CURE_CRITIC, CLASS_CLERIC, 9);
    t.spell_level(SPELL_SUMMON, CLASS_CLERIC, 10);
    t.spell_level(SPELL_REMOVE_POISON, CLASS_CLERIC, 10);
    t.spell_level(SPELL_IDENTIFY, CLASS_CLERIC, 11);
    t.spell_level(SPELL_WORD_OF_RECALL, CLASS_CLERIC, 12);
    t.spell_level(SPELL_DARKNESS, CLASS_CLERIC, 12);
    t.spell_level(SPELL_EARTHQUAKE, CLASS_CLERIC, 12);
    t.spell_level(SPELL_DISPEL_EVIL, CLASS_CLERIC, 14);
    t.spell_level(SPELL_DISPEL_GOOD, CLASS_CLERIC, 14);
    t.spell_level(SPELL_SANCTUARY, CLASS_CLERIC, 15);
    t.spell_level(SPELL_CALL_LIGHTNING, CLASS_CLERIC, 15);
    t.spell_level(SPELL_HEAL, CLASS_CLERIC, 16);
    t.spell_level(SPELL_CONTROL_WEATHER, CLASS_CLERIC, 17);
    t.spell_level(SPELL_SENSE_LIFE, CLASS_CLERIC, 18);
    t.spell_level(SPELL_HARM, CLASS_CLERIC, 19);
    t.spell_level(SPELL_GROUP_HEAL, CLASS_CLERIC, 22);
    t.spell_level(SPELL_REMOVE_CURSE, CLASS_CLERIC, 26);
    // THIEVES
    t.spell_level(SKILL_SNEAK, CLASS_THIEF, 1);
    t.spell_level(SKILL_PICK_LOCK, CLASS_THIEF, 2);
    t.spell_level(SKILL_BACKSTAB, CLASS_THIEF, 3);
    t.spell_level(SKILL_STEAL, CLASS_THIEF, 4);
    t.spell_level(SKILL_HIDE, CLASS_THIEF, 5);
    t.spell_level(SKILL_TRACK, CLASS_THIEF, 6);
    // WARRIORS
    t.spell_level(SKILL_KICK, CLASS_WARRIOR, 1);
    t.spell_level(SKILL_RESCUE, CLASS_WARRIOR, 3);
    t.spell_level(SKILL_BANDAGE, CLASS_WARRIOR, 7);
    t.spell_level(SKILL_TRACK, CLASS_WARRIOR, 9);
    t.spell_level(SKILL_BASH, CLASS_WARRIOR, 12);
    t.spell_level(SKILL_WHIRLWIND, CLASS_WARRIOR, 16);

    t
}

static TABLE: std::sync::OnceLock<Box<[SpellInfo; 300]>> = std::sync::OnceLock::new();

/// The spell_info table.
pub fn spell_info_table() -> &'static [SpellInfo; 300] {
    TABLE.get_or_init(|| Box::new(build_table().0))
}

/// Entry for `num`. Out-of-range and reserved numbers give the unused entry.
pub fn spell_info(num: i32) -> &'static SpellInfo {
    if (0..=TOP_SPELL_DEFINE).contains(&num) {
        &spell_info_table()[num as usize]
    } else {
        &UNUSED
    }
}

pub fn skill_name(num: i32) -> &'static str {
    if num > 0 && num <= TOP_SPELL_DEFINE {
        spell_info(num).name
    } else if num == -1 {
        "UNUSED"
    } else {
        "UNDEFINED"
    }
}

/// The registered (non-!UNUSED!) spell/skill names — kept for callers that
/// list name/number pairs.
pub fn spell_names() -> Vec<(i32, &'static str)> {
    (1..=TOP_SPELL_DEFINE)
        .filter_map(|n| {
            let e = spell_info(n);
            if e.name != UNUSED_SPELLNAME { Some((n, e.name)) } else { None }
        })
        .collect()
}
