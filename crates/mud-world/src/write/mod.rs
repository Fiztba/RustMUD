//! World-file writers, one submodule per format. Output byte-matches the
//! the reference server's own canonical saves.
//!
//! Each writer exposes two entry points: `write_file`, the real save, and
//! `write_file_fmt`, which takes a [`VnumFmt`] so `export` can reuse the
//! same code instead of carrying a second, drifting copy.

pub mod mob;
pub mod obj;
pub mod qst;
pub mod shp;
pub mod trg;
pub mod wld;
pub mod zon;

use mud_data::types::{Idx, NOWHERE};

use crate::model::Zone;

/// How a writer renders the vnums it emits.
///
/// A second, hand-copied set of writers for `export` would differ only in
/// the vnum column, and such copies drift: an export .obj writer that never
/// gained the `timer` field, rooms that drop EX_HIDDEN from the door flag,
/// a .zon that `%100`s a cross-zone reference into a colliding
/// in-zone one, and its info file describes exits the rooms don't have.
/// Threading this formatter through the real writers instead makes an
/// export the same code path as a save, so it cannot drift again.
///
/// `Qq` is the scheme qq.info documents: every occurrence of the zone
/// number becomes `QQ`, and the recipient replaces it with theirs.
/// `Renumber` (`export <zone> <target>`) rewrites in-zone vnums into the
/// target zone's range instead, so the files drop straight in.
///
/// **Both forms mark a reference that leaves the zone `ZZnn`, never a real
/// vnum.** `ZZ` fails closed — `setup_dir`, `load_zones` and the shop
/// reader all refuse a vnum they cannot scan — whereas a live vnum the
/// destination happens not to have boots
/// fine and leaves a dead exit that `look <dir>` still describes.
///
/// The one format that does NOT mark is `.shp`: a shop legitimately stocks
/// other zones' goods — 377 of the shipped
/// products, 18 of the rooms and 19 of the keepers point outside their own
/// zone — so marking them would refuse to boot over something normal.
/// Those are handled the old way: lists drop the entry
/// ([`VnumFmt::in_zone`]),
/// and the keeper is forced into the zone's own numbering
/// ([`VnumFmt::push_zone_slot`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VnumFmt {
    /// A real save: every vnum is its own decimal number.
    Plain,
    /// `export <zone>`: `QQnn` in the zone, `ZZnn` out of it.
    Qq { bot: i64, top: i64 },
    /// `export <zone> <target>`: an in-zone vnum becomes
    /// `new_bot + (v - bot)`; out-of-zone stays `ZZnn`.
    Renumber { bot: i64, top: i64, new_bot: i64, new_number: i64 },
}

impl VnumFmt {
    /// `export <zone>` over this zone's vnum window.
    pub fn qq(zone: &Zone) -> Self {
        VnumFmt::Qq { bot: i64::from(zone.bot), top: i64::from(zone.top) }
    }

    /// `export <zone> <target>`. The window slides to `target * 100`, the
    /// grid all 189 shipped zones sit on (`zedit new` doesn't enforce it —
    /// see [`VnumFmt::spans_over_100`]).
    pub fn renumber(zone: &Zone, new_number: Idx) -> Self {
        VnumFmt::Renumber {
            bot: i64::from(zone.bot),
            top: i64::from(zone.top),
            new_bot: i64::from(new_number) * 100,
            new_number: i64::from(new_number),
        }
    }

    pub fn is_plain(&self) -> bool {
        matches!(self, VnumFmt::Plain)
    }

    /// The zone number a renumbering export is writing into.
    pub fn new_number(&self) -> Option<i64> {
        match self {
            VnumFmt::Renumber { new_number, .. } => Some(*new_number),
            _ => None,
        }
    }

    /// True when the zone is wider than the 100-vnum grid both export
    /// forms assume: `QQ%02d` collides two vnums onto one marker, and a
    /// renumber spills past the target zone into the one above it.
    pub fn spans_over_100(&self) -> bool {
        match self {
            VnumFmt::Plain => false,
            VnumFmt::Qq { bot, top } | VnumFmt::Renumber { bot, top, .. } => top - bot >= 100,
        }
    }

    /// Does this vnum belong to the zone being written? There are two
    /// equivalent tests — `world[rnum].zone == zrnum` for exits, and
    /// `vnum < bot || vnum > top` for shop lists — which agree, since a
    /// room takes its zone from that same window.
    ///
    /// Always true for a real save, so a writer that drops out-of-zone
    /// entries on export (`.shp` products and rooms) keeps all of them
    /// when saving.
    pub fn in_zone(&self, v: i64) -> bool {
        match self {
            VnumFmt::Plain => true,
            VnumFmt::Qq { bot, top } | VnumFmt::Renumber { bot, top, .. } => {
                (*bot..=*top).contains(&v)
            }
        }
    }

    /// Emit one vnum, in whichever scheme is in force.
    ///
    /// The NOWHERE/NOTHING/NOBODY sentinel (65535) passes through
    /// untouched: a few fields store it raw on disk (a quest's unset
    /// `obj_reward` is the common case), and it is not a vnum to reattach.
    pub fn push_vnum(&self, out: &mut Vec<u8>, v: i64) {
        if self.is_plain() || v == i64::from(NOWHERE) {
            push_int(out, v);
            return;
        }
        if !self.in_zone(v) {
            out.extend_from_slice(b"ZZ");
            push_marker(out, v);
            return;
        }
        match self {
            VnumFmt::Qq { .. } => {
                out.extend_from_slice(b"QQ");
                push_marker(out, v);
            }
            VnumFmt::Renumber { bot, new_bot, .. } => push_int(out, new_bot + (v - bot)),
            VnumFmt::Plain => unreachable!("handled above"),
        }
    }

    /// Emit a vnum in the exported zone's own numbering whether or not it
    /// belongs there — the `%100` treatment, kept for the one field that
    /// has no other option: a shop's keeper. It is mandatory, so it can't
    /// be dropped the way an out-of-zone product is, and marking it `ZZ`
    /// would stop 19 shipped zones from exporting a bootable .shp at all.
    ///
    /// The cost: a keeper from another zone silently becomes
    /// whichever mob holds that slot in the recipient's copy. The info
    /// file names them, since nothing in the file itself can.
    pub fn push_zone_slot(&self, out: &mut Vec<u8>, v: i64) {
        match self {
            VnumFmt::Plain => push_int(out, v),
            VnumFmt::Qq { .. } => {
                out.extend_from_slice(b"QQ");
                push_marker(out, v);
            }
            VnumFmt::Renumber { new_bot, .. } => push_int(out, new_bot + v.rem_euclid(100)),
        }
    }

    /// The zone's own number: the `#30` of a.zon header, the first field
    /// of a room's flag line. The QQ scheme writes the bare marker, which
    /// is why `#QQ` has no digits after it.
    pub fn push_zone_number(&self, out: &mut Vec<u8>, n: i64) {
        match self {
            VnumFmt::Plain => push_int(out, n),
            VnumFmt::Qq { .. } => out.extend_from_slice(b"QQ"),
            VnumFmt::Renumber { new_number, .. } => push_int(out, *new_number),
        }
    }
}

/// The `%02d` half of a QQnn/ZZnn marker.
fn push_marker(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(format!("{:02}", v.rem_euclid(100)).as_bytes());
}

#[cfg(test)]
mod vnum_fmt_tests {
    use super::*;

    fn zone(bot: Idx, top: Idx) -> Zone {
        Zone { number: bot / 100, bot, top, ..Default::default() }
    }

    fn rendered(fmt: VnumFmt, v: i64) -> String {
        let mut out = Vec::new();
        fmt.push_vnum(&mut out, v);
        String::from_utf8(out).unwrap()
    }

    fn zone_number(fmt: VnumFmt, n: i64) -> String {
        let mut out = Vec::new();
        fmt.push_zone_number(&mut out, n);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn plain_writes_every_vnum_as_itself() {
        let f = VnumFmt::Plain;
        assert_eq!(rendered(f, 3001), "3001");
        assert_eq!(rendered(f, 1204), "1204");
        assert_eq!(zone_number(f, 30), "30");
        assert!(f.is_plain());
        assert_eq!(f.new_number(), None);
    }

    #[test]
    fn qq_marks_the_zone_and_zz_marks_what_leaves_it() {
        let f = VnumFmt::qq(&zone(3000, 3099));
        assert_eq!(rendered(f, 3000), "QQ00");
        assert_eq!(rendered(f, 3004), "QQ04");
        assert_eq!(rendered(f, 3099), "QQ99");
        // One past each end of the window is someone else's vnum.
        assert_eq!(rendered(f, 2999), "ZZ99");
        assert_eq!(rendered(f, 3100), "ZZ00");
        assert_eq!(rendered(f, 1204), "ZZ04");
        // "#QQ" has no digits — the zone number is the thing replaced.
        assert_eq!(zone_number(f, 30), "QQ");
        assert!(!f.is_plain());
        assert_eq!(f.new_number(), None);
    }

    #[test]
    fn renumber_slides_the_window_but_still_zzs_the_rest() {
        let f = VnumFmt::renumber(&zone(57700, 57799), 400);
        assert_eq!(rendered(f, 57700), "40000");
        assert_eq!(rendered(f, 57742), "40042");
        assert_eq!(rendered(f, 57799), "40099");
        assert_eq!(rendered(f, 57800), "ZZ00");
        assert_eq!(zone_number(f, 577), "400");
        assert_eq!(f.new_number(), Some(400));
    }

    #[test]
    fn a_zone_that_does_not_sit_on_the_grid_still_renumbers_from_its_bottom() {
        // zedit new doesn't enforce bot == number * 100.
        let f = VnumFmt::renumber(&zone(3050, 3099), 400);
        assert_eq!(rendered(f, 3050), "40000");
        assert_eq!(rendered(f, 3099), "40049");
    }

    #[test]
    fn the_nothing_sentinel_is_never_a_marker() {
        // 65535 reaches a writer as itself (a quest's unset obj_reward);
        // ZZ35 would tell the recipient to reattach a room that isn't one.
        for f in [VnumFmt::qq(&zone(3000, 3099)), VnumFmt::renumber(&zone(3000, 3099), 400)] {
            assert_eq!(rendered(f, 65535), "65535");
        }
    }

    #[test]
    fn a_zone_wider_than_the_grid_is_reported() {
        assert!(!VnumFmt::qq(&zone(3000, 3099)).spans_over_100());
        assert!(VnumFmt::qq(&zone(3000, 3100)).spans_over_100());
        assert!(VnumFmt::renumber(&zone(3000, 3199), 400).spans_over_100());
        assert!(!VnumFmt::Plain.spans_over_100());
    }
}

/// Bits 0-25 -> 'a'-'z', 26-31 -> 'A'-'F'; empty
/// bitvector renders as "0".
pub fn sprintascii(bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..32u32 {
        if bits & (1 << i) != 0 {
            out.push(if i < 26 { b'a' + i as u8 } else { b'A' + (i - 26) as u8 });
        }
    }
    if out.is_empty() {
        out.push(b'0');
    }
    out
}

/// "%d" formatting for i32.
pub fn push_int(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(itoa(v).as_bytes());
}

fn itoa(v: i64) -> String {
    v.to_string()
}

/// Emit a tilde-terminated string field: the stored bytes (already \r\n
/// normalized in memory) are written with "\n" line endings and tabs
/// converted back to '@' color codes, exactly as genolc's
/// convert_from_tabs + strip_cr pipeline does.
pub fn push_tilde_string(out: &mut Vec<u8>, s: &Option<Vec<u8>>) {
    if let Some(s) = s {
        for &b in s {
            match b {
                b'\r' => {}
                b'\t' => out.push(b'@'),
                other => out.push(other),
            }
        }
    }
    out.extend_from_slice(b"~\n");
}
