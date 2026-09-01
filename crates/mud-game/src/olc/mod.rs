//! The OLC framework every editor sits on: the per-descriptor [`OlcData`],
//! the colour globals, `cleanup_olc`, zone permissions,
//! and the nanny/string-editor dispatch tables that route a descriptor in
//! an OLC state to its parser.
//!
//! Two shapes are worth calling out because they are observable:
//!
//! * `nrm`/`grn`/`cyn`/`yel` are **global**, not per-descriptor state.
//! `get_char_colors` overwrites them for whoever called last, and several
//! menu writers never call it — so with two builders in OLC at once, one
//! builder's menu can be painted in the other's colour preference.
//! `Game::olc_colors` keeps that behavior; before the first
//! `get_char_colors` they are unset, which `%s` prints as `(null)`.
//! * Link loss inside any of the fourteen editor states runs `cleanup_olc`,
//! so `PLR_WRITING` never survives on the body.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;
use mud_world::model::{MobProto, ObjProto, Quest, Room, Shop, Trigger, Zone};

use crate::act::wizstat::{AEDIT_PERMISSION, ALL_PERMISSION, HEDIT_PERMISSION};
use crate::act::BStr;
use crate::comm::{self, act, cc, send_to_char, C_NRM, KCYN, KGRN, KNRM, KYEL, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::text::HelpEntry;

pub mod aedit;
pub mod archive;
pub mod cedit;
pub mod copy;
pub mod deflate;
pub mod export;
pub mod genmob;
pub mod genobj;
pub mod genqst;
pub mod genshp;
pub mod genwld;
pub mod genzon;
pub mod hedit;
pub mod list;
pub mod medit;
pub mod msgedit;
pub mod oedit;
pub mod prefedit;
pub mod qedit;
pub mod redit;
pub mod sedit;
pub mod tedit;
pub mod trigedit;
pub mod zedit;

// ---------------------------------------------------------------------------
// OLC limits and type tags
// ---------------------------------------------------------------------------

pub const MAX_ROOM_NAME: usize = 150;
pub const MAX_MOB_NAME: usize = 100;
pub const MAX_OBJ_NAME: usize = 100;
pub const MAX_ROOM_DESC: usize = 2048;
pub const MAX_EXIT_DESC: usize = 256;
pub const MAX_EXTRA_DESC: usize = 512;
pub const MAX_MOB_DESC: usize = 1024;
pub const MAX_OBJ_DESC: usize = 512;
pub const MAX_DUPLICATES: i32 = 100;

pub const OASIS_WLD: i32 = 0;
pub const OASIS_MOB: i32 = 1;
pub const OASIS_OBJ: i32 = 2;
pub const OASIS_ZON: i32 = 3;
pub const OASIS_EXI: i32 = 4;
pub const OASIS_CFG: i32 = 5;

/// Cleanup types.
pub const CLEANUP_ALL: u8 = 1;
pub const CLEANUP_STRUCTS: u8 = 2;
pub const CLEANUP_CONFIG: u8 = 3;

pub const STRING_TERMINATOR: u8 = b'~';

// ---------------------------------------------------------------------------
// The OLC colour globals
// ---------------------------------------------------------------------------

/// The four OLC colour strings. `None` is unset, which prints as
/// `(null)`.
#[derive(Debug, Clone, Copy)]
pub struct OlcColors {
    pub nrm: Option<&'static [u8]>,
    pub grn: Option<&'static [u8]>,
    pub cyn: Option<&'static [u8]>,
    pub yel: Option<&'static [u8]>,
}

impl Default for OlcColors {
    fn default() -> Self {
        Self { nrm: None, grn: None, cyn: None, yel: None }
    }
}

impl OlcColors {
    /// The four colours as &str, for the menus built with `format!`. Every
    /// colour string is ASCII, so this is lossless.
    pub fn nrm_s(&self) -> &str {
        std::str::from_utf8(self.nrm()).unwrap_or("")
    }
    pub fn grn_s(&self) -> &str {
        std::str::from_utf8(self.grn()).unwrap_or("")
    }
    pub fn cyn_s(&self) -> &str {
        std::str::from_utf8(self.cyn()).unwrap_or("")
    }
    pub fn yel_s(&self) -> &str {
        std::str::from_utf8(self.yel()).unwrap_or("")
    }

    pub fn nrm(&self) -> &[u8] {
        self.nrm.unwrap_or(b"(null)")
    }
    pub fn grn(&self) -> &[u8] {
        self.grn.unwrap_or(b"(null)")
    }
    pub fn cyn(&self) -> &[u8] {
        self.cyn.unwrap_or(b"(null)")
    }
    pub fn yel(&self) -> &[u8] {
        self.yel.unwrap_or(b"(null)")
    }
}

/// get_char_colors: set the colour globals for this character's colour
/// level. Deliberately global — see the module docs.
pub fn get_char_colors(g: &mut Game, chid: CharId) {
    g.olc_colors = OlcColors {
        nrm: Some(cc(g, chid, C_NRM, KNRM)),
        grn: Some(cc(g, chid, C_NRM, KGRN)),
        cyn: Some(cc(g, chid, C_NRM, KCYN)),
        yel: Some(cc(g, chid, C_NRM, KYEL)),
    };
}

pub fn clear_screen(g: &mut Game, di: usize) {
    let cls = g
        .descriptors
        .get(di)
        .and_then(|d| d.character)
        .and_then(|c| g.try_ch(c))
        .is_some_and(|c| c.prf(flags::PRF_CLS));
    if cls {
        comm::write_to_desc(g, di, b"\x1b[H\x1b[J");
    }
}

// ---------------------------------------------------------------------------
// Editor state
// ---------------------------------------------------------------------------

/// Where the line editor writes when a descriptor in an OLC state hands the
/// input stream to `string_add`. The field is named rather than pointed
/// at, and the buffer is copied back in `string_cleanup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrTarget {
    RoomDesc,
    /// An exit's description. Which direction is in `value`.
    ExitDesc,
    /// A room extra description. Which one is in `desc`.
    ExtraDesc,
    /// A mobile's description.
    MobDesc,
    /// An object's action description.
    ObjActDesc,
    /// An object extra description. Which one is in `desc`.
    ObjExtraDesc,
    /// The quest's info, done and quit text.
    QuestInfo,
    QuestDone,
    QuestQuit,
    /// A help entry's body.
    HelpEntry,
    /// cedit's three block fields, all on the operation menu.
    CeditMenu,
    CeditWelcMessg,
    CeditStartMessg,
}

/// One builder's editor session. Only the fields the active editor uses are
/// populated; the rest stay `None`.
#[derive(Debug, Default)]
pub struct OlcData {
    /// Which submenu is parsing input.
    pub mode: i32,
    /// The *real* zone number being edited (or one of the
    /// AEDIT/HEDIT pseudo-zones).
    pub zone_num: i32,
    /// Vnum of the subject.
    pub number: i32,
    /// "Has changed" flag, and scratch space -- redit keeps its direction here.
    pub value: i32,
    /// hedit: the help-table generation this editor's index was taken
    /// against, and the keyword of the row it opened. The keyword is
    /// captured before the builder can edit the keyword field, so a save
    /// can find the row again if the table moved underneath.
    pub help_version: u64,
    pub help_key: Option<BStr>,
    pub help_text: Option<BStr>,
    /// Scratch text for tedit, aedit and trigedit.
    pub storage: Option<BStr>,
    pub mob: Option<Box<MobProto>>,
    pub room: Option<Box<Room>>,
    pub obj: Option<Box<ObjProto>>,
    pub zone: Option<Box<Zone>>,
    pub shop: Option<Box<Shop>>,
    pub config: Option<Box<crate::config::Config>>,
    pub quest: Option<Box<Quest>>,
    /// Index of the extra description under edit, within the
    /// owning room's/object's list.
    pub desc: Option<usize>,
    pub action: Option<Box<crate::social::Social>>,
    pub trig: Option<Box<Trigger>>,
    pub ibt: Option<Box<crate::ibt::Ibt>>,
    pub help: Option<Box<HelpEntry>>,
    /// Which submenu the script-assignment editor is in.
    pub script_mode: i32,
    pub trigger_position: i32,
    /// MOB_TRIGGER, OBJ_TRIGGER or WLD_TRIGGER.
    pub item_type: i32,
    /// The trigger list being assigned in [r|o|m]edit. `None`
    /// means no list, which the menus render as "Not Set.".
    pub script: Option<Vec<Idx>>,
    /// Which field the line editor is filling.
    pub str_target: Option<StrTarget>,
    /// The scratch copy's rnum and shop keeper. The prototypes have no
    /// rnum field of their own, so they are held here.
    pub mob_rnum: Idx,
    pub obj_rnum: Idx,
    pub shop_keeper: Idx,
    /// The rest of the shop record, which lives in `ShopRt`
    /// (`S_BANK`/`S_SORT`/`S_FUNC`). copy_shop carries all three, so the
    /// scratch shop has to as well even though no sedit screen edits bank
    /// or sort.
    pub shop_bank: i32,
    pub shop_sort: i32,
    pub shop_func: Option<crate::spec::MobSpec>,
    /// `QST_FUNC` on the scratch quest — the questmaster's displaced spec
    /// proc, carried by `copy_quest`.
    pub quest_func: Option<crate::spec::MobSpec>,
    /// OLC_MSG_LIST / OLC_MSG: msgedit's working copy of a message
    /// slot, and the index of the slot being edited.
    pub msg_list: Option<Box<crate::fight::FightMessageList>>,
    pub msg_index: usize,
    /// msgedit's "quit after saving" flag. It is per-descriptor here; a
    /// single shared flag would leak between builders (B64).
    pub msg_quit: bool,
    /// OLC_PREFS: prefedit's working copy of a player's toggles,
    /// and who they belong to.
    pub prefs: Option<Box<prefedit::PrefsScratch>>,
    /// The scratch trigger's rnum; the `Trigger` prototype has no rnum
    /// field of its own. `NOTHING` while the trigger under edit is new.
    pub trig_rnum: Idx,
    /// The room's live light count, carried across the edit and written
    /// back on save. It lives here because RoomRt owns the runtime half.
    pub room_light: i32,
    /// zedit's "the command list has changed" flag. The static `Zone` has
    /// no spare field for it, since the reset clock lives in `zones_rt`.
    /// Its partner, "the header has changed", is `Zone::number` on the
    /// scratch copy.
    pub zone_age: i32,
}

impl OlcData {
    /// A fresh session, every numeric field at zero.
    pub fn new() -> Box<Self> {
        Box::new(Self::default())
    }
}

/// Read-only peek at another descriptor's OLC data.
pub fn olc_of(g: &Game, di: usize) -> Option<&OlcData> {
    g.olc.get(&di).map(|b| &**b)
}

// ---------------------------------------------------------------------------
// cleanup_olc
// ---------------------------------------------------------------------------

/// Free the editor state and put the descriptor back into the playing state.
///
/// Takes the data by value, so dropping it releases everything the editor
/// owned. Only the observable half — the act, the mudlog and the state
/// change — is spelled out here.
pub fn cleanup_olc(g: &mut Game, di: usize, olc: Box<OlcData>, cleanup_type: u8) {
    let state = g.descriptors.get(di).map(|d| d.state);
    let chid = g.descriptors.get(di).and_then(|d| d.character);

    if olc.room.is_some() && !matches!(cleanup_type, CLEANUP_ALL | CLEANUP_STRUCTS | CLEANUP_CONFIG)
    {
        // "default: /* The caller has screwed up. */"
        g.log("SYSERR: cleanup_olc: Unknown type!".to_string());
    }

    let zone_num = olc.zone_num;
    drop(olc);
    g.olc.remove(&di);

    // Restore descriptor playing status.
    if let Some(chid) = chid {
        if g.try_ch(chid).is_some() {
            g.ch_mut(chid).act.remove(flags::PLR_WRITING);
        }
        // Only a descriptor that actually entered an editor announces
        // leaving one. `dig` and buildwalk borrow an OlcData so that
        // `redit_save_internally` can do the insertion for them without ever
        // touching the state, so neither the room message nor the zone log
        // line is emitted for a session that never started.
        let announced = state != Some(ConState::Playing);
        if g.try_ch(chid).is_some() && announced {
            act(g, b"$n stops using OLC.", true, Some(chid), None, None, TO_ROOM);

            let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
            let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
            if cleanup_type == CLEANUP_CONFIG {
                let msg = format!("OLC: {} stops editing the game configuration", name);
                g.mudlog(MudlogKind::Brf, level, true, &msg);
            } else if state == Some(ConState::Tedit) {
                let msg = format!("OLC: {} stops editing text files.", name);
                g.mudlog(MudlogKind::Brf, level, true, &msg);
            } else if state == Some(ConState::Hedit) {
                let msg = format!("OLC: {} stops editing help files.", name);
                g.mudlog(MudlogKind::Cmp, level, true, &msg);
            // Four editors reach this line holding something other than a
            // zone, and it reads whatever they left as one.
            //
            // aedit keeps a SOCIAL index there -- 0 to 491 on the shipped world
            // against a zone table of 189 -- so more than half of all socials
            // are out of range, on every save and every quit. The lookup below
            // is checked, so what came out was the raw index, which is its own
            // kind of
            // wrong. msgedit and prefedit never write the field at all and so
            // report zone 0; ibtedit DOES write it, as a dirty flag of 0 or 1,
            // and so reports zone 1 once anything has changed. In range, and
            // never once true.
            //
            // tedit and hedit above already say what they were editing rather
            // than pretending to a zone. So do these -- in this function's own
            // words, not an echo of each editor's entry line: prefedit logs
            // nothing on the way in, and msgedit and ibtedit name the
            // individual record, which is not what is being left.
            } else if let Some(what) = match state {
                Some(ConState::Aedit) => Some(&b"actions"[..]),
                Some(ConState::Msgedit) => Some(&b"messages"[..]),
                Some(ConState::Prefedit) => Some(&b"preferences"[..]),
                Some(ConState::Ibtedit) => Some(&b"ideas, bugs and typos"[..]),
                _ => None,
            } {
                let msg = format!(
                    "OLC: {} stops editing {}.",
                    name,
                    String::from_utf8_lossy(what)
                );
                g.mudlog(MudlogKind::Cmp, level, true, &msg);
            } else if let Some(z) = g.world.zones.get(zone_num as usize) {
                let zvnum = z.number as i32;
                let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
                let msg = format!(
                    "OLC: {} stops editing zone {} allowed zone {}",
                    name, zvnum, allowed
                );
                g.mudlog(MudlogKind::Cmp, level, true, &msg);
            } else {
                // Nothing reaches this. Every state left sets zone_num from
                // real_zone/real_zone_by_thing and refuses to open on NOWHERE,
                // and the callers that arrive still in Playing are covered by
                // the `announced` guard above -- which is what closes aedit's
                // early cleanup, made before the state is set and with zone_num
                // one past the social table.
                //
                // It stays so the next editor to keep something else in that
                // field gets a line naming itself rather than a zone it is not
                // editing, and it names the builder like every other branch
                // here rather than leaving a bare number to trace.
                let who = state
                    .and_then(|s| mud_data::tables::CONNECTED_TYPES.get(s as usize).copied())
                    .unwrap_or("UNDEFINED");
                let msg = format!(
                    "SYSERR: cleanup_olc: {} left {} in OLC_ZNUM while in {}, \
                     which is not a zone.",
                    name, zone_num, who
                );
                g.mudlog(MudlogKind::Brf, level, true, &msg);
            }
        }
        if announced {
            if let Some(d) = g.descriptors.get_mut(di) {
                d.state = ConState::Playing;
            }
        }
    }
}

/// The close_socket half: every OLC state gets a cleanup here, so a link
/// loss is never silent (see B29 in the module docs).
pub fn cleanup_olc_on_close(g: &mut Game, di: usize) {
    let state = g.descriptors.get(di).map(|d| d.state);
    let cleans = matches!(
        state,
        Some(
            ConState::Oedit
                | ConState::Redit
                | ConState::Zedit
                | ConState::Medit
                | ConState::Sedit
                | ConState::Tedit
                | ConState::Trigedit
                | ConState::Aedit
                | ConState::Hedit
                | ConState::Qedit
                | ConState::Msgedit
                // These three were once missing here, so link loss
                // inside one of them skipped cleanup entirely and left
                // PLR_WRITING set.
                | ConState::Cedit
                | ConState::Prefedit
                | ConState::Ibtedit
        )
    );
    if cleans {
        if let Some(olc) = g.olc.remove(&di) {
            // cedit apart. cleanup_olc picks its log line off the cleanup
            // type, so CLEANUP_ALL has it announce a zone it was never
            // editing -- and cedit's own three exits all pass CLEANUP_CONFIG,
            // so a builder who drops the link is logged the same as one who
            // quits. The structure is dropped either way.
            let kind = if state == Some(ConState::Cedit) { CLEANUP_CONFIG } else { CLEANUP_ALL };
            cleanup_olc(g, di, olc, kind);
        }
    } else {
        // Drop the structure.
        g.olc.remove(&di);
    }
}

// ---------------------------------------------------------------------------
// Zone permissions
// ---------------------------------------------------------------------------

/// can_edit_zone. `rnum` is a real zone number, or the
/// AEDIT/HEDIT pseudo-zone.
pub fn can_edit_zone(g: &Game, chid: CharId, rnum: i32) -> bool {
    let ch = g.ch(chid);
    if ch.desc.is_none() || ch.is_npc() || rnum == NOWHERE as i32 {
        return false;
    }
    let pseudo = rnum == HEDIT_PERMISSION || rnum == AEDIT_PERMISSION;
    if !pseudo
        && g.world
            .zones
            .get(rnum as usize)
            .is_some_and(|_| crate::act::wizard::zone_flagged(g, rnum as usize, flags::ZONE_NOBUILD))
    {
        return false;
    }
    let olc_zone = ch.player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    if olc_zone == ALL_PERMISSION {
        return true;
    }
    if olc_zone == HEDIT_PERMISSION && rnum == HEDIT_PERMISSION {
        return true;
    }
    if olc_zone == AEDIT_PERMISSION && rnum == AEDIT_PERMISSION {
        return true;
    }
    if ch.level >= LVL_GRGOD {
        return true;
    }
    if !pseudo {
        let builders = g
            .world
            .zones
            .get(rnum as usize)
            .and_then(|z| z.builders.clone())
            .unwrap_or_default();
        if crate::handler::is_name(ch.get_name(), &builders) {
            return true;
        }
    }
    if olc_zone == NOWHERE as i32 {
        return false;
    }
    if ch.level < LVL_BUILDER {
        return false;
    }
    if g.world.real_zone(olc_zone as Idx) == Some(rnum as Idx) {
        return true;
    }
    false
}

pub fn send_cannot_edit(g: &mut Game, chid: CharId, zone: i32) {
    let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let buf = if olc_zone != NOWHERE as i32 {
        send_to_char(
            g,
            chid,
            format!(
                "You do not have permission to edit zone {}.  Try zone {}.\r\n",
                zone, olc_zone
            )
            .as_bytes(),
        );
        format!("OLC: {} tried to edit zone {} (allowed zone {}).", name, zone, olc_zone)
    } else {
        send_to_char(
            g,
            chid,
            format!("You do not have permission to edit zone {}.\r\n", zone).as_bytes(),
        );
        format!("OLC: {} tried to edit zone {}.", name, zone)
    };
    g.mudlog(MudlogKind::Brf, LVL_IMPL, true, &buf);
}

// ---------------------------------------------------------------------------
// Small OLC helpers
// ---------------------------------------------------------------------------

/// Told to the builder when a save did not reach the disk.
///
/// The editor's own copy is already in memory and the entry is still on
/// the save list, so `olc save` or the next autosave will try again --
/// which is what makes a failed write a warning rather than lost work.
/// Saying nothing is the part that loses work: the builder walks away
/// believing the write happened.
pub(crate) fn save_failed(what: &str) -> Vec<u8> {
    format!("Unable to save {what} to disk. Changes remain marked for saving.
")
        .into_bytes()
}

/// An empty or absent string becomes "undefined".
pub fn str_udup(s: &[u8]) -> BStr {
    if s.is_empty() {
        b"undefined".to_vec()
    } else {
        s.to_vec()
    }
}

/// genolc_checkstring: smash_tilde + parse_at in place. Always returns
/// TRUE.
pub fn genolc_checkstring(arg: &mut BStr) -> bool {
    mud_net::editor::smash_tilde(arg);
    mud_net::editor::parse_at(arg);
    true
}

/// split_argument: copy the leading run of non-space,
/// non-'=' characters into `tag`, then drop the separator run from the
/// front of `argument`.
pub fn split_argument(argument: &[u8]) -> (BStr, BStr) {
    let mut tag = Vec::new();
    let mut p = 0;
    while p < argument.len() {
        let c = argument[p];
        if c != b' ' && c != b'=' {
            tag.push(c);
            p += 1;
        } else if c == b'=' {
            break;
        } else {
            p += 1;
        }
    }
    while p < argument.len() && (argument[p] == b'=' || argument[p] == b' ') {
        p += 1;
    }
    (tag, argument[p..].to_vec())
}

/// atoidx: strtol, then anything negative or above
/// IDXTYPE_MAX collapses to NOWHERE. Callers store the result in an `int`,
/// so a negative argument arrives as 65535, not as -1.
pub fn atoidx(s: &[u8]) -> i32 {
    let mut p = 0;
    while p < s.len() && s[p].is_ascii_whitespace() {
        p += 1;
    }
    let neg = matches!(s.get(p), Some(b'-'));
    if matches!(s.get(p), Some(b'-') | Some(b'+')) {
        p += 1;
    }
    // strtol saturates at LONG_MAX/LONG_MIN and sets ERANGE; either way the
    // guard below rejects it, so an i64 that stops growing is enough.
    let mut n: i64 = 0;
    let mut digits = false;
    while p < s.len() && s[p].is_ascii_digit() {
        digits = true;
        n = (n * 10 + (s[p] - b'0') as i64).min(i64::from(u16::MAX) + 1);
        p += 1;
    }
    if !digits {
        // No conversion performed: strtol returns 0 and leaves errno clear.
        return 0;
    }
    if neg {
        n = -n;
    }
    if !(0..=i64::from(u16::MAX)).contains(&n) {
        return NOWHERE as i32;
    }
    n as i32
}

pub fn count_non_protocol_chars(s: &[u8]) -> i32 {
    let mut count = 0;
    let mut p = 0;
    while p < s.len() {
        let c = s[p];
        if c == b'\r' || c == b'\n' {
            p += 1;
            continue;
        }
        if c == b'@' || c == b'\t' {
            p += 1;
            let n = s.get(p).copied();
            match n {
                Some(b'[') => {
                    while p < s.len() && s[p] != b']' {
                        p += 1;
                    }
                    p += 1;
                }
                Some(b'<') | Some(b'>') | Some(b'(') | Some(b')') => p += 1,
                _ => p += 1,
            }
            continue;
        }
        count += 1;
        p += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// Dispatch: nanny's OLC table and string_add's
// cleanup table
// ---------------------------------------------------------------------------

/// The states in nanny's `olc_functions[]` table.
/// CON_TEDIT is deliberately absent: tedit lives entirely in the line
/// editor, so its input never reaches nanny.
pub fn nanny_olc_state(state: ConState) -> bool {
    matches!(
        state,
        ConState::Oedit
            | ConState::Zedit
            | ConState::Sedit
            | ConState::Medit
            | ConState::Redit
            | ConState::Cedit
            | ConState::Trigedit
            | ConState::Aedit
            | ConState::Hedit
            | ConState::Qedit
            | ConState::Prefedit
            | ConState::Ibtedit
            | ConState::Msgedit
    )
}

/// The OLC half of nanny: route a line to the editor parsing this state.
/// Returns false when the state is not an OLC state.
pub fn olc_parse(g: &mut Game, di: usize, arg: &[u8]) -> bool {
    let Some(state) = g.descriptors.get(di).map(|d| d.state) else { return false };
    if !nanny_olc_state(state) {
        return false;
    }
    // No editor can reach its state without an OLC structure, so a
    // missing one is simply a no-op here.
    let Some(olc) = g.olc.remove(&di) else { return true };
    let left = match state {
        ConState::Redit => redit::redit_parse(g, di, olc, arg),
        ConState::Medit => medit::medit_parse(g, di, olc, arg),
        ConState::Oedit => oedit::oedit_parse(g, di, olc, arg),
        ConState::Zedit => zedit::zedit_parse(g, di, olc, arg),
        ConState::Trigedit => trigedit::trigedit_parse(g, di, olc, arg),
        ConState::Sedit => sedit::sedit_parse(g, di, olc, arg),
        ConState::Qedit => qedit::qedit_parse(g, di, olc, arg),
        ConState::Aedit => aedit::aedit_parse(g, di, olc, arg),
        ConState::Hedit => hedit::hedit_parse(g, di, olc, arg),
        ConState::Msgedit => msgedit::msgedit_parse(g, di, olc, arg),
        ConState::Prefedit => prefedit::prefedit_parse(g, di, olc, arg),
        ConState::Cedit => cedit::cedit_parse(g, di, olc, arg),
        _ => {
            g.olc.insert(di, olc);
            return false;
        }
    };
    if let Some(olc) = left {
        g.olc.insert(di, olc);
    }
    true
}

/// The OLC half of string_add's cleanup dispatch: `action` is true for a
/// save (STRINGADD_SAVE), false for an abort.
pub fn string_cleanup(g: &mut Game, di: usize, text: Option<BStr>, saved: bool) -> bool {
    let Some(state) = g.descriptors.get(di).map(|d| d.state) else { return false };
    let Some(olc) = g.olc.remove(&di) else { return false };
    let left = match state {
        ConState::Redit => redit::redit_string_cleanup(g, di, olc, text, saved),
        ConState::Medit => medit::medit_string_cleanup(g, di, olc, text, saved),
        ConState::Oedit => oedit::oedit_string_cleanup(g, di, olc, text, saved),
        ConState::Trigedit => trigedit::trigedit_string_cleanup(g, di, olc, text, saved),
        ConState::Qedit => qedit::qedit_string_cleanup(g, di, olc, text, saved),
        ConState::Hedit => hedit::hedit_string_cleanup(g, di, olc, text, saved),
        ConState::Cedit => cedit::cedit_string_cleanup(g, di, olc, text, saved),
        // tedit never reaches nanny, but its save lands here.
        ConState::Tedit => tedit::tedit_string_cleanup(g, di, olc, text, saved),
        _ => {
            g.olc.insert(di, olc);
            return false;
        }
    };
    if let Some(olc) = left {
        g.olc.insert(di, olc);
    }
    true
}

// ---------------------------------------------------------------------------
// do_show_save_list — the `olc` command
// ---------------------------------------------------------------------------

/// The message column of the save_types[] table, indexed by SL_*.
pub fn save_type_message(t: i32) -> &'static str {
    match t {
        crate::db::SL_MOB => "mobile",
        crate::db::SL_OBJ => "object",
        crate::db::SL_SHP => "shop",
        crate::db::SL_WLD => "room",
        crate::db::SL_ZON => "zone",
        crate::db::SL_CFG => "config",
        crate::db::SL_QST => "quest",
        crate::db::SL_ACT => "social",
        crate::db::SL_HLP => "help",
        _ => "(null)",
    }
}

pub fn do_show_save_list(g: &mut Game, chid: CharId, _arg: &[u8], _cmd: usize, _subcmd: i32) {
    if g.save_list.is_empty() {
        send_to_char(g, chid, b"All world files are up to date.\r\n");
        return;
    }
    send_to_char(g, chid, b"The following files need saving:\r\n");
    for (zone, t) in g.save_list.clone() {
        if t != crate::db::SL_CFG {
            let line = format!(" - {} data for zone {}.\r\n", save_type_message(t), zone);
            send_to_char(g, chid, line.as_bytes());
        } else {
            send_to_char(g, chid, b" - Game configuration data.\r\n");
        }
    }
}
