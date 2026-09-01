//! Runtime object data. Instances shallow-copy their
//! prototype; string fields here are `None` when the prototype's text is in
//! effect (the string is shared and freed conditionally — same observable
//! behavior, honest ownership).

use mud_data::flags::FlagSet;
use mud_data::types::{Idx, RoomRnum, NOWHERE};
use mud_world::model::ExtraDesc;

use mud_data::ids::{CharId, ObjId};

pub type BStr = Vec<u8>;

#[derive(Debug, Clone, Default)]
pub struct Obj {
    /// Prototype rnum; NOTHING for unique objects.
    pub item_number: Idx,
    /// Room the object is in; NOWHERE when carried/worn/contained.
    pub in_room: RoomRnum,

    pub values: [i32; 4],
    pub type_flag: i32,
    /// Same wear/extra/perm bit layout as the prototype.
    pub wear_flags: FlagSet,
    pub extra_flags: FlagSet,
    pub perm_affects: FlagSet,
    pub weight: i32,
    pub cost: i32,
    pub cost_per_day: i32,
    pub level: i32,
    pub timer: i32,

    pub affected: [mud_world::model::ObjAffect; mud_data::types::MAX_OBJ_AFFECT],

    /// String overrides; None ⇒ prototype text applies (see Game::obj_name etc.).
    pub name: Option<BStr>,
    pub short_description: Option<BStr>,
    pub description: Option<BStr>,
    pub action_description: Option<BStr>,
    pub ex_descriptions: Option<Vec<ExtraDesc>>,

    pub carried_by: Option<CharId>,
    pub worn_by: Option<CharId>,
    pub worn_on: i16,
    pub in_obj: Option<ObjId>,
    pub contains: Vec<ObjId>,
    /// OBJ_SAT_IN_BY (furniture occupant chain head).
    pub sat_in_by: Option<CharId>,

    pub proto_script: Vec<Idx>,
    pub script_id: i64,
    /// DG script container.
    pub script: Option<Box<crate::dg::ScriptData>>,
}

impl Obj {
    pub fn obj_flagged(&self, bit: usize) -> bool {
        self.extra_flags.is_set(bit)
    }

    pub fn can_wear(&self, bit: usize) -> bool {
        self.wear_flags.is_set(bit)
    }
}

pub fn default_worn_on() -> i16 {
    -1
}

/// Fresh empty object as `create_obj` makes one: NOTHING proto,
/// NOWHERE room, worn_on -1.
pub fn create_obj() -> Obj {
    Obj {
        item_number: mud_data::types::NOTHING,
        in_room: NOWHERE,
        worn_on: -1,
        ..Default::default()
    }
}
