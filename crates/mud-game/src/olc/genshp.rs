//! The shop half of the OLC generic layer.
//!
//! A shop's file fields and its runtime ones (`bank`, `sort`, `func`, and
//! the resolved keeper/product rnums) are split across two structures:
//! `mud_world::model::Shop` is the file data and `shop::ShopRt` the runtime
//! half, so `add_shop` has to land in both tables at the same index.
//!
//! Two representation notes:
//!
//! * `Shop::producing` holds object **vnums**; `ShopRt::producing` carries
//! the rnums for the runtime paths. The writer prints the vnum either way.
//! * `Shop::in_rooms` holds room vnums, which is why every display of
//! `S_ROOM` calls `real_room`.

use mud_world::model::Shop;

use crate::act::BStr;
use crate::db::{
    add_to_save_list, in_save_list, remove_from_save_list, write_world_file, SL_MOB, SL_SHP,
};
use crate::game::{Game, MudlogKind};
use crate::shop::ShopRt;
use mud_data::types::*;

/// real_shop: binary search over the shop table.
pub fn real_shop(g: &Game, vnum: i32) -> Option<usize> {
    let mut bot: i64 = 0;
    let mut top: i64 = g.world.shops.len() as i64 - 1;
    while bot <= top {
        let mid = ((bot + top) / 2) as usize;
        let num = g.world.shops[mid].vnum as i32;
        if num == vnum {
            return Some(mid);
        }
        if num > vnum {
            top = mid as i64 - 1;
        } else {
            bot = mid as i64 + 1;
        }
    }
    None
}

/// modify_shop_string: every keeper message has to carry
/// the `%s` the shopkeeper's name is substituted into, so one is prepended
/// when the builder's text does not start with a `%`.
pub fn modify_shop_string(new_s: &[u8]) -> BStr {
    if new_s.first() != Some(&b'%') {
        let mut out: BStr = b"%s ".to_vec();
        out.extend_from_slice(new_s);
        out
    } else {
        new_s.to_vec()
    }
}

/// The runtime half of a shop under edit — the fields that live in
/// `ShopRt`. `bank` and `sort` ride along because `copy_shop` carries them,
/// though no sedit screen touches either.
#[derive(Debug, Clone, Default)]
pub struct ShopRtScratch {
    pub keeper: Idx,
    pub bank: i32,
    pub sort: i32,
    pub func: Option<crate::spec::MobSpec>,
}

impl ShopRtScratch {
    pub fn from_rt(rt: &ShopRt) -> Self {
        Self { keeper: rt.keeper, bank: rt.bank, sort: rt.sort, func: rt.func }
    }

    /// A fresh shop: no keeper, everything else zeroed.
    pub fn new_shop() -> Self {
        Self { keeper: NOBODY, bank: 0, sort: 0, func: None }
    }
}

/// The product rnums `ShopRt` carries, resolved from the file-side vnums.
/// Unresolvable entries drop, as they do at boot.
fn resolve_products(g: &Game, shop: &Shop) -> Vec<Idx> {
    shop.producing
        .iter()
        .filter_map(|&v| {
            if !(0..=u16::MAX as i32).contains(&v) {
                return None;
            }
            g.world.real_object(v as Idx)
        })
        .collect()
}

/// add_shop: update in place when the vnum already
/// exists, otherwise insert in vnum order. Either way the zone is flagged
/// for saving — and if the shop's vnum belongs to no zone, that is a SYSERR
/// and the shop still lands in the table.
pub fn add_shop(g: &mut Game, shop: &Shop, rt: &ShopRtScratch) -> usize {
    let rznum = crate::dg::mobcmd::real_zone_by_thing(g, shop.vnum as i32);

    let flag_zone = |g: &mut Game| match rznum {
        Some(z) => {
            let number = g.world.zones[z].number;
            add_to_save_list(g, number, SL_SHP);
        }
        None => {
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: GenOLC: Cannot determine shop zone.",
            );
        }
    };

    if let Some(rshop) = real_shop(g, shop.vnum as i32) {
        let products = resolve_products(g, shop);
        g.world.shops[rshop] = shop.clone();
        let existing = &mut g.shops_rt[rshop];
        existing.keeper = rt.keeper;
        existing.bank = rt.bank;
        existing.sort = rt.sort;
        existing.func = rt.func;
        existing.producing = products;
        flag_zone(g);
        return rshop;
    }

    // Walk down from the top shifting entries up until the new vnum is
    // greater than its neighbour below; that lands it in vnum order.
    let rshop = g
        .world
        .shops
        .iter()
        .position(|s| s.vnum > shop.vnum)
        .unwrap_or(g.world.shops.len());
    let products = resolve_products(g, shop);
    g.world.shops.insert(rshop, shop.clone());
    g.shops_rt.insert(
        rshop,
        ShopRt {
            keeper: rt.keeper,
            producing: products,
            bank: rt.bank,
            sort: rt.sort,
            func: rt.func,
        },
    );
    flag_zone(g);
    rshop
}

/// save_shops. Only writes the index when the zone
/// actually holds a shop, and clears the zone's save-list entry.
pub fn save_shops(g: &mut Game, zone_num: Option<usize>) -> bool {
    let top = g.world.zones.len().saturating_sub(1);
    let Some(zone_num) = zone_num.filter(|&z| z < g.world.zones.len()) else {
        g.log(format!(
            "SYSERR: GenOLC: save_shops: Invalid real zone number {}. (0-{})",
            NOWHERE, top
        ));
        return false;
    };
    let vznum = g.world.zones[zone_num].number;
    let (bot, ztop) = (g.world.zones[zone_num].bot, g.world.zones[zone_num].top);
    let num_shops = (bot..=ztop).filter(|&v| real_shop(g, v as i32).is_some()).count();

    if write_world_file(g, zone_num, SL_SHP).is_none() {
        g.mudlog(MudlogKind::Brf, LVL_GOD, true, "SYSERR: OLC: Cannot open shop file!");
        return false;
    }
    if num_shops > 0 {
        crate::olc::genzon::create_world_index(g, vznum as i32, "shp");
    }
    if in_save_list(g, vznum, SL_SHP) {
        remove_from_save_list(g, vznum, SL_SHP);
    }
    true
}

/// How many shops still name this mobile as their keeper.
fn shops_kept_by(g: &Game, keeper: Idx) -> usize {
    g.shops_rt.iter().filter(|rt| rt.keeper == keeper).count()
}

/// Take MOB_SPEC off a mobile that has nothing left to dispatch.
///
/// Three places, because the flag lives in three: the prototype, every copy
/// already walking around, and the .mob file the next copy is read from --
/// which is the only one of the three that survives a reboot. Left set over a
/// mobile with no proc, `mobile_activity` logs "Attempting to call
/// non-existing mob function" for every copy that spawns, and strips the bit
/// from the LIVE mobile only, so the prototype hands it out again on the next
/// respawn and the file hands it out again after every reboot.
///
/// The file is written here only when the keeper lives in the zone the caller
/// is already writing. Keepers do not have to live in their shop's zone, and
/// reaching across to write one the builder may not own would also flush
/// whatever else that zone had pending; anywhere else it is queued. The
/// in-memory correction happens either way.
///
/// Returns true if the bit came off, so the caller can say which zone is now
/// waiting on a save.
fn release_spec_mobile(g: &mut Game, keeper: Idx, caller_zone: Option<usize>) -> bool {
    const WORD: usize = mud_data::flags::MOB_SPEC / 32;
    const BIT: u32 = 1 << (mud_data::flags::MOB_SPEC % 32);

    // Only when there is genuinely nothing left to call.
    if g.mob_specs.get(keeper as usize).copied().flatten().is_some() {
        return false;
    }
    match g.world.mob_protos.get_mut(keeper as usize) {
        Some(proto) if proto.act[WORD] & BIT != 0 => proto.act[WORD] &= !BIT,
        _ => return false,
    }

    for chid in g.character_list.clone() {
        let Some(ch) = g.chars.get_mut(chid) else { continue };
        if ch.is_npc() && ch.mob_rnum == keeper {
            ch.act.remove(mud_data::flags::MOB_SPEC);
        }
    }

    let kvnum = g.world.mob_protos[keeper as usize].vnum as i32;
    if let Some(kzone) = crate::dg::mobcmd::real_zone_by_thing(g, kvnum) {
        let number = g.world.zones[kzone].number;
        add_to_save_list(g, number, SL_MOB);
        if Some(kzone) == caller_zone {
            crate::olc::genmob::save_mobiles(g, Some(kzone));
        }
    }
    true
}

/// Put both mobiles right after a shop's keeper has been changed.
///
/// The rule the boot establishes is that a mobile runs `shop_keeper` exactly
/// while some shop names it, and `assign_the_shopkeepers` displaces whatever
/// proc the mobile already had into the shop's `func` so it can be handed
/// back. Installing `shop_keeper` on the new mobile without releasing the old
/// one would leave it answering `list` and `buy` with "Sorry, but you cannot
/// do that here!" -- a shopkeeper for no shop.
///
/// `oldfunc` is the proc this shop was holding for its previous keeper, read
/// before `add_shop` overwrote the record. It is only the mobile's OWN proc
/// when this is the shop holding it: `assign_the_shopkeepers` stashes it in
/// the first shop that names the mobile and leaves the rest empty. So when the
/// mobile still keeps others, the proc has to move to one of them rather than
/// be dropped, or releasing shops in one order destroys it and the other order
/// does not.
///
/// MOB_SPEC is not SET on the incoming keeper: `special` dispatches on the
/// index entry and never reads the flag, and `assign_the_shopkeepers` does not
/// set it either, so setting it here would diverge from what a reboot gives.
/// It IS cleared off a mobile released with nothing left to dispatch.
///
/// Returns the mobile it released, or NOBODY.
pub fn reassign_shopkeeper(
    g: &mut Game,
    vnum: i32,
    oldkeeper: Idx,
    oldfunc: Option<crate::spec::MobSpec>,
) -> Idx {
    let Some(rshop) = real_shop(g, vnum) else { return NOBODY };
    let newkeeper = g.shops_rt[rshop].keeper;
    if newkeeper == oldkeeper {
        return NOBODY;
    }
    let shop_zone = crate::dg::mobcmd::real_zone_by_thing(g, vnum);
    let mut released = NOBODY;

    // The mobile that is no longer this shop's keeper.
    if oldkeeper != NOBODY && (oldkeeper as usize) < g.world.mob_protos.len() {
        if shops_kept_by(g, oldkeeper) == 0 {
            // Nothing names it now, so it stops being a shopkeeper and takes
            // its own proc back.
            if g.mob_specs.get(oldkeeper as usize).copied().flatten()
                == Some(crate::spec::MobSpec::ShopKeeper)
            {
                if let Some(slot) = g.mob_specs.get_mut(oldkeeper as usize) {
                    *slot = oldfunc;
                }
            }
            if release_spec_mobile(g, oldkeeper, shop_zone) {
                released = oldkeeper;
            }
        } else if let Some(f) = oldfunc {
            // It still keeps others, so it stays as it is -- but if the proc
            // was being held HERE it has to move somewhere that survives.
            if let Some(i) = (0..g.shops_rt.len())
                .find(|&i| g.shops_rt[i].keeper == oldkeeper && g.shops_rt[i].func.is_none())
            {
                g.shops_rt[i].func = Some(f);
            }
        }
    }

    // And the mobile that now is. func is written unconditionally: this record
    // still holds whatever add_shop copied out of the editor, which is the OLD
    // keeper's proc, and leaving it there makes the shop run an unrelated
    // mobile's spec proc.
    if newkeeper != NOBODY && (newkeeper as usize) < g.world.mob_protos.len() {
        let prev = g.mob_specs.get(newkeeper as usize).copied().flatten();
        g.shops_rt[rshop].func =
            if prev == Some(crate::spec::MobSpec::ShopKeeper) { None } else { prev };
        if let Some(slot) = g.mob_specs.get_mut(newkeeper as usize) {
            *slot = Some(crate::spec::MobSpec::ShopKeeper);
        }
    } else {
        g.shops_rt[rshop].func = None;
    }

    released
}

/// delete_shop: remove a shop outright.
///
/// The keeper is the whole of the difficulty. `assign_the_shopkeepers`
/// displaces the mobile's own spec proc into the shop record and points
/// `mob_specs` at ShopKeeper. It runs once, at boot, and stashes the proc in
/// only the FIRST shop it walks for a given keeper, so a mobile keeping two
/// shops has its proc in the lower-rnum one and none in the other. Simply
/// dropping it when other shops remain therefore destroys the only copy:
/// delete the shops in ascending order and the mobile's own proc is gone for
/// good. It is handed to a surviving shop instead.
pub fn delete_shop(g: &mut Game, rnum: usize) -> bool {
    if rnum >= g.world.shops.len() {
        return false;
    }
    let vnum = g.world.shops[rnum].vnum as i32;
    let rznum = crate::dg::mobcmd::real_zone_by_thing(g, vnum);
    let keeper = g.shops_rt[rnum].keeper;
    let tempfunc = g.shops_rt[rnum].func;

    g.log(format!("GenOLC: delete_shop: Deleting shop #{}.", vnum));

    g.world.shops.remove(rnum);
    g.shops_rt.remove(rnum);

    if keeper != NOBODY {
        if shops_kept_by(g, keeper) > 0 {
            // Somebody else still keeps a shop for this mobile, so it stays a
            // shopkeeper -- but the proc it had before it became one may have
            // been living in the record just removed. Give it to a survivor
            // that has none.
            if let Some(f) = tempfunc {
                if let Some(i) = (0..g.shops_rt.len())
                    .find(|&i| g.shops_rt[i].keeper == keeper && g.shops_rt[i].func.is_none())
                {
                    g.shops_rt[i].func = Some(f);
                }
            }
        } else {
            // Its last shop. Hand the proc back, but only if ShopKeeper is
            // still what is there to replace: a keeper reassigned since boot
            // should keep the reassignment rather than have it overwritten.
            if g.mob_specs.get(keeper as usize).copied().flatten()
                == Some(crate::spec::MobSpec::ShopKeeper)
            {
                if let Some(slot) = g.mob_specs.get_mut(keeper as usize) {
                    *slot = tempfunc;
                }
            }
            release_spec_mobile(g, keeper, rznum);
        }
    }

    match rznum {
        Some(z) => {
            let number = g.world.zones[z].number;
            add_to_save_list(g, number, SL_SHP);
        }
        None => g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: GenOLC: delete_shop: Cannot determine shop zone.",
        ),
    }

    true
}
