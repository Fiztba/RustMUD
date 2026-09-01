//! The object editor.
//!
//! Two oddities are kept deliberately: the apply menu treats answers 0 *and*
//! 1 as "clear this slot" (`(number = atoi(arg)) == 0 || (number = atoi(arg)) == 1`),
//! so apply #1 can never be set from the menu; and `value` doubles as
//! the apply slot index while the apply submenu is open, which is also the
//! editor's "something changed" flag.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{
    APPLY_TYPES, CONTAINER_BITS, DRINKS, EXTRA_BITS, ITEM_TYPES, WEAR_BITS,
};
use mud_data::types::*;
use mud_world::model::{ExtraDesc, ObjProto};

use crate::act::informative::{column_list, sprintbitarray};
use crate::act::wizstat::sprinttype;
use crate::act::BStr;
use crate::comm::{act, send_editor_help, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::{atoi, pers};
use crate::interpreter::{is_number, two_arguments};
use crate::olc::genobj::{add_object, delete_object, save_objects};
use crate::olc::trigedit::{dg_olc_script_copy, dg_script_menu, SCRIPT_MAIN_MENU};
use crate::olc::{
    can_edit_zone, clear_screen, genolc_checkstring, get_char_colors, send_cannot_edit, str_udup,
    OlcData, StrTarget, CLEANUP_ALL,
};

/// Submodes of OEDIT connectedness.
pub const OEDIT_MAIN_MENU: i32 = 1;
pub const OEDIT_KEYWORD: i32 = 2;
pub const OEDIT_SHORTDESC: i32 = 3;
pub const OEDIT_LONGDESC: i32 = 4;
pub const OEDIT_ACTDESC: i32 = 5;
pub const OEDIT_TYPE: i32 = 6;
pub const OEDIT_EXTRAS: i32 = 7;
pub const OEDIT_WEAR: i32 = 8;
pub const OEDIT_WEIGHT: i32 = 9;
pub const OEDIT_COST: i32 = 10;
pub const OEDIT_COSTPERDAY: i32 = 11;
pub const OEDIT_TIMER: i32 = 12;
pub const OEDIT_VALUE_1: i32 = 13;
pub const OEDIT_VALUE_2: i32 = 14;
pub const OEDIT_VALUE_3: i32 = 15;
pub const OEDIT_VALUE_4: i32 = 16;
pub const OEDIT_APPLY: i32 = 17;
pub const OEDIT_APPLYMOD: i32 = 18;
pub const OEDIT_EXTRADESC_KEY: i32 = 19;
pub const OEDIT_CONFIRM_SAVEDB: i32 = 20;
pub const OEDIT_CONFIRM_SAVESTRING: i32 = 21;
pub const OEDIT_PROMPT_APPLY: i32 = 22;
pub const OEDIT_EXTRADESC_DESCRIPTION: i32 = 23;
pub const OEDIT_EXTRADESC_MENU: i32 = 24;
pub const OEDIT_LEVEL: i32 = 25;
pub const OEDIT_PERM: i32 = 26;
pub const OEDIT_DELETE: i32 = 27;
pub const OEDIT_COPY: i32 = 28;

/// The limits oedit enforces.
const MAX_OBJ_WEIGHT: i32 = 1_000_000;
const MAX_OBJ_COST: i32 = 2_000_000;
const MAX_OBJ_RENT: i32 = 2_000_000;
const MAX_OBJ_TIMER: i32 = 1_071_000;
const MAX_CONTAINER_SIZE: i32 = 10_000;
const MAX_WEAPON_SDICE: i32 = 50;
const MAX_WEAPON_NDICE: i32 = 50;
const MAX_PEOPLE: i32 = 10;
/// NUM_SPELLS / NUM_LIQ_TYPES.
const NUM_SPELLS: i32 = 54;
const NUM_LIQ_TYPES: i32 = 16;

fn limit(v: i32, low: i32, high: i32) -> i32 {
    high.min(v.max(low))
}

pub fn do_oasis_oedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let (buf1, buf2, _) = two_arguments(argument);
    let mut number: i32 = NOWHERE as i32;
    let mut save = false;

    if buf1.is_empty() {
        send_to_char(g, chid, b"Specify an object VNUM to edit.\r\n");
        return;
    } else if !buf1[0].is_ascii_digit() {
        if crate::text::cmp_ci(b"save", &buf1) != std::cmp::Ordering::Equal {
            send_to_char(g, chid, b"Yikes!  Stop that, someone will get hurt!\r\n");
            return;
        }
        save = true;
        if is_number(&buf2) {
            number = atoi(&buf2);
        } else {
            let olc_zone = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
            if olc_zone > 0 {
                number = match g.world.real_zone(olc_zone as Idx) {
                    None => NOWHERE as i32,
                    // The zone below is resolved with real_zone, so
                    // this has to be the zone NUMBER. Handing it
                    // a vnum here would stop any
                    // argument-less save from ever resolving.
                    Some(zlok) => g.world.zones[zlok as usize].number as i32,
                };
            }
        }
        if number == NOWHERE as i32 {
            send_to_char(g, chid, b"Save which zone?\r\n");
            return;
        }
    }

    if number == NOWHERE as i32 {
        number = atoi(&buf1);
    }
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That object VNUM can't exist.\r\n");
        return;
    }

    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Oedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = match g.descriptors.get(other).and_then(|d| d.character) {
            Some(c) => pers(g, chid, c),
            None => b"someone".to_vec(),
        };
        let mut msg = b"That object is currently being edited by ".to_vec();
        msg.extend_from_slice(&who);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_IMMORT,
            true,
            "SYSERR: do_oasis: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }
    let mut olc = OlcData::new();

    let znum = if save {
        g.world.real_zone(number as Idx).map(|z| z as i32)
    } else {
        crate::dg::mobcmd::real_zone_by_thing(g, number).map(|z| z as i32)
    };
    let Some(znum) = znum else {
        send_to_char(g, chid, b"Sorry, there is no zone for that number!\r\n");
        return;
    };
    olc.zone_num = znum;

    if !can_edit_zone(g, chid, znum) {
        let zvnum = g.world.zones[znum as usize].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }

    if save {
        let zvnum = g.world.zones[znum as usize].number;
        send_to_char(
            g,
            chid,
            format!("Saving all objects in zone {}.\r\n", zvnum).as_bytes(),
        );
        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
        let msg = format!("OLC: {} saves object info for zone {}.", name, zvnum);
        g.mudlog(MudlogKind::Cmp, level, true, &msg);
        save_objects(g, Some(znum as usize));
        return;
    }

    olc.number = number;

    match g.world.real_object(number as Idx) {
        Some(real_num) => oedit_setup_existing(g, &mut olc, real_num as usize),
        None => oedit_setup_new(&mut olc),
    }

    oedit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Oedit;
    }
    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum as usize].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!("OLC: {} starts editing zone {} allowed zone {}", name, zvnum, allowed);
    g.mudlog(MudlogKind::Cmp, level, true, &msg);
}

fn oedit_setup_new(olc: &mut OlcData) {
    let mut obj = ObjProto {
        vnum: NOTHING,
        name: Some(b"unfinished object".to_vec()),
        description: Some(b"An unfinished object is lying here.".to_vec()),
        short_description: Some(b"an unfinished object".to_vec()),
        ..Default::default()
    };
    let bit = flags::ITEM_WEAR_TAKE;
    obj.wear_flags[bit / 32] |= 1 << (bit % 32);
    olc.obj = Some(Box::new(obj));
    olc.obj_rnum = NOTHING;
    olc.value = 0;
    olc.item_type = crate::dg::OBJ_TRIGGER;
    olc.script = None;
}

pub fn oedit_setup_existing(g: &mut Game, olc: &mut OlcData, real_num: usize) {
    let proto = g.world.obj_protos[real_num].clone();
    olc.obj_rnum = real_num as Idx;
    olc.obj = Some(Box::new(proto));
    olc.value = 0;
    olc.item_type = crate::dg::OBJ_TRIGGER;
    dg_olc_script_copy(olc);
    if let Some(o) = olc.obj.as_mut() {
        o.proto_script.clear();
    }
}

pub fn oedit_save_internally(g: &mut Game, _di: usize, olc: &mut OlcData) {
    let is_new = g.world.real_object(olc.number as Idx).is_none();

    let obj = olc.obj.as_ref().expect("oedit without an object").as_ref().clone();
    let Some(robj_num) = add_object(g, &obj, olc.number as Idx) else {
        g.log("oedit_save_internally: add_object failed.".to_string());
        return;
    };
    olc.obj_rnum = robj_num;

    let script = olc.script.clone().unwrap_or_default();
    g.world.obj_protos[robj_num as usize].proto_script = script.clone();

    // The objects currently in-game.
    for id in g.object_list.clone() {
        if g.objs.get(id).map(|o| o.item_number) != Some(robj_num) {
            continue;
        }
        if g.objs.get(id).is_some_and(|o| o.script.is_some()) {
            crate::dg::extract_script(g, crate::dg::GoId::Obj(id));
        }
        if let Some(o) = g.objs.get_mut(id) {
            o.proto_script = script.clone();
        }
        crate::dg::assign_triggers(g, crate::dg::GoId::Obj(id));
    }

    if !is_new {
        return;
    }

    // Produce lists in shops being edited.
    let others: Vec<usize> = g.descriptors.order.clone();
    for dsc in others.iter().copied() {
        if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Sedit) {
            continue;
        }
        let Some(other) = g.olc.get_mut(&dsc) else { continue };
        if let Some(shop) = other.shop.as_mut() {
            for p in shop.producing.iter_mut() {
                if *p >= robj_num as i32 {
                    *p += 1;
                }
            }
        }
    }
    // And zedit sessions.
    for dsc in others {
        if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Zedit) {
            continue;
        }
        let Some(other) = g.olc.get_mut(&dsc) else { continue };
        let Some(zone) = other.zone.as_mut() else { continue };
        for cmd in zone.cmds.iter_mut() {
            match cmd.command {
                b'P' => {
                    if cmd.arg3 >= robj_num as i32 {
                        cmd.arg3 += 1;
                    }
                    if cmd.arg1 >= robj_num as i32 {
                        cmd.arg1 += 1;
                    }
                }
                b'E' | b'G' | b'O' => {
                    if cmd.arg1 >= robj_num as i32 {
                        cmd.arg1 += 1;
                    }
                }
                b'R' => {
                    if cmd.arg2 >= robj_num as i32 {
                        cmd.arg2 += 1;
                    }
                }
                _ => {}
            }
        }
    }
}

fn oedit_save_to_disk(g: &mut Game, zone_num: Option<usize>) -> bool {
    save_objects(g, zone_num)
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn colors(g: &mut Game, di: usize) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
}

/// sprintbit over a single 32-bit value.
fn sprintbit(bits: i32, names: &[&str]) -> BStr {
    let mut out: BStr = Vec::new();
    let bits = bits as u32;
    for (i, name) in names.iter().enumerate() {
        if i >= 32 {
            break;
        }
        if bits & (1 << i) != 0 {
            out.extend_from_slice(name.as_bytes());
            out.push(b' ');
        }
    }
    if out.is_empty() {
        out.extend_from_slice(b"NOBITS ");
    }
    out
}

fn oedit_disp_container_flags_menu(g: &mut Game, di: usize, olc: &OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let bits = sprintbit(olc.obj.as_ref().unwrap().values[1], &CONTAINER_BITS);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    for (n, label) in [
        (&b"1"[..], &b") CLOSEABLE\r\n"[..]),
        (b"2", b") PICKPROOF\r\n"),
        (b"3", b") CLOSED\r\n"),
        (b"4", b") LOCKED\r\n"),
    ] {
        out.extend_from_slice(c.grn());
        out.extend_from_slice(n);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
    }
    out.extend_from_slice(b"Container flags: ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter flag, 0 to quit : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_extradesc_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let idx = olc.desc.unwrap_or(0);
    let (keyword, description, has_next) = {
        let obj = olc.obj.as_ref().unwrap();
        let xd = obj.ex_descriptions.get(idx);
        (
            xd.and_then(|x| x.keyword.clone()).filter(|k| !k.is_empty()),
            xd.and_then(|x| x.description.clone()).filter(|d| !d.is_empty()),
            idx + 1 < obj.ex_descriptions.len(),
        )
    };
    let c = g.olc_colors;
    let mut out: BStr = b"Extra desc menu\r\n".to_vec();
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"1");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Keywords: ");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(keyword.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"2");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Description:\r\n");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(description.as_deref().unwrap_or(b"<NONE>"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"3");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Goto next description: ");
    out.extend_from_slice(if has_next { b"Set." } else { b"Not set." });
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"0");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Quit\r\nEnter choice : ");
    write_to_desc(g, di, &out);
    olc.mode = OEDIT_EXTRADESC_MENU;
}

fn oedit_disp_prompt_apply_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let obj = olc.obj.as_ref().unwrap().as_ref().clone();
    let mut out: BStr = Vec::new();
    for counter in 0..MAX_OBJ_AFFECT {
        if obj.affected[counter].modifier != 0 {
            let apply_buf = sprinttype(obj.affected[counter].location, &APPLY_TYPES);
            out.extend_from_slice(
                format!(" {}{}{}) {:+} to ", c.grn_s(), counter + 1, c.nrm_s(), obj.affected[counter].modifier)
                    .as_bytes(),
            );
            out.extend_from_slice(&apply_buf);
            out.extend_from_slice(b"\r\n");
        } else {
            out.extend_from_slice(
                format!(" {}{}{}) None.\r\n", c.grn_s(), counter + 1, c.nrm_s()).as_bytes(),
            );
        }
    }
    out.extend_from_slice(b"\r\nEnter affection to modify (0 to quit) : ");
    write_to_desc(g, di, &out);
    olc.mode = OEDIT_PROMPT_APPLY;
}

fn oedit_liquid_type(g: &mut Game, di: usize, olc: &mut OlcData) {
    colors(g, di);
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let names: Vec<BStr> = DRINKS.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &names, true);
    }
    let c = g.olc_colors;
    let mut out: BStr = b"\r\n".to_vec();
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"Enter drink type : ");
    write_to_desc(g, di, &out);
    olc.mode = OEDIT_VALUE_3;
}

fn oedit_disp_apply_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    colors(g, di);
    clear_screen(g, di);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        let names: Vec<BStr> = APPLY_TYPES.iter().map(|s| s.as_bytes().to_vec()).collect();
        column_list(g, chid, 0, &names, true);
    }
    write_to_desc(g, di, b"\r\nEnter apply type (0 is no apply) : ");
    olc.mode = OEDIT_APPLY;
}

fn oedit_disp_weapon_menu(g: &mut Game, di: usize) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for (counter, (singular, _)) in crate::fight::ATTACK_HIT_TEXT.iter().enumerate() {
        columns += 1;
        let name: String = String::from_utf8_lossy(singular).chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20} {}",
                c.grn_s(),
                counter,
                c.nrm_s(),
                name,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(b"\r\nEnter weapon type : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_spells_menu(g: &mut Game, di: usize) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for counter in 1..=NUM_SPELLS {
        columns += 1;
        let name: String = mud_data::spells::spell_info(counter).name.chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {}{:<20} {}",
                c.grn_s(),
                counter,
                c.nrm_s(),
                c.yel_s(),
                name,
                if columns % 3 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    let mut tail: BStr = b"\r\n".to_vec();
    tail.extend_from_slice(c.nrm());
    tail.extend_from_slice(b"Enter spell choice (-1 for none) : ");
    out.extend_from_slice(&tail);
    write_to_desc(g, di, &out);
}

fn item_type(olc: &OlcData) -> i32 {
    olc.obj.as_ref().unwrap().type_flag
}

fn oedit_disp_val1_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    olc.mode = OEDIT_VALUE_1;
    match item_type(olc) {
        flags::ITEM_LIGHT => oedit_disp_val3_menu(g, di, olc),
        flags::ITEM_SCROLL | flags::ITEM_WAND | flags::ITEM_STAFF | flags::ITEM_POTION => {
            write_to_desc(g, di, b"Spell level : ")
        }
        flags::ITEM_WEAPON => write_to_desc(g, di, b"Modifier to Hitroll : "),
        flags::ITEM_ARMOR => write_to_desc(g, di, b"Apply to AC : "),
        flags::ITEM_CONTAINER => {
            write_to_desc(g, di, b"Max weight to contain (-1 for unlimited) : ")
        }
        flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN => {
            write_to_desc(g, di, b"Max drink units (-1 for unlimited) : ")
        }
        flags::ITEM_FOOD => write_to_desc(g, di, b"Hours to fill stomach : "),
        flags::ITEM_MONEY => write_to_desc(g, di, b"Number of gold coins : "),
        flags::ITEM_FURNITURE => write_to_desc(g, di, b"Number of people it can hold : "),
        flags::ITEM_NOTE
        | flags::ITEM_OTHER
        | flags::ITEM_WORN
        | flags::ITEM_TREASURE
        | flags::ITEM_TRASH
        | flags::ITEM_KEY
        | flags::ITEM_PEN
        | flags::ITEM_BOAT
        | flags::ITEM_FREE
        | flags::ITEM_FREE2 => oedit_disp_menu(g, di, olc),
        _ => g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: OLC: Reached default case in oedit_disp_val1_menu()!",
        ),
    }
}

fn oedit_disp_val2_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    olc.mode = OEDIT_VALUE_2;
    match item_type(olc) {
        flags::ITEM_SCROLL | flags::ITEM_POTION => oedit_disp_spells_menu(g, di),
        flags::ITEM_WAND | flags::ITEM_STAFF => {
            write_to_desc(g, di, b"Max number of charges : ")
        }
        flags::ITEM_WEAPON => write_to_desc(g, di, b"Number of damage dice : "),
        flags::ITEM_FOOD => oedit_disp_val4_menu(g, di, olc),
        flags::ITEM_CONTAINER => oedit_disp_container_flags_menu(g, di, olc),
        flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN => {
            write_to_desc(g, di, b"Initial drink units : ")
        }
        _ => oedit_disp_menu(g, di, olc),
    }
}

fn oedit_disp_val3_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    olc.mode = OEDIT_VALUE_3;
    match item_type(olc) {
        flags::ITEM_LIGHT => {
            write_to_desc(g, di, b"Number of hours (0 = burnt, -1 is infinite) : ")
        }
        flags::ITEM_SCROLL | flags::ITEM_POTION => oedit_disp_spells_menu(g, di),
        flags::ITEM_WAND | flags::ITEM_STAFF => {
            write_to_desc(g, di, b"Number of charges remaining : ")
        }
        flags::ITEM_WEAPON => write_to_desc(g, di, b"Size of damage dice : "),
        flags::ITEM_CONTAINER => {
            write_to_desc(g, di, b"Vnum of key to open container (-1 for no key) : ")
        }
        flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN => oedit_liquid_type(g, di, olc),
        _ => oedit_disp_menu(g, di, olc),
    }
}

fn oedit_disp_val4_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    olc.mode = OEDIT_VALUE_4;
    match item_type(olc) {
        flags::ITEM_SCROLL | flags::ITEM_POTION | flags::ITEM_WAND | flags::ITEM_STAFF => {
            oedit_disp_spells_menu(g, di)
        }
        flags::ITEM_WEAPON => oedit_disp_weapon_menu(g, di),
        flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN | flags::ITEM_FOOD => {
            write_to_desc(g, di, b"Poisoned (0 = not poison) : ")
        }
        _ => oedit_disp_menu(g, di, olc),
    }
}

fn oedit_disp_type_menu(g: &mut Game, di: usize) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for (counter, name) in ITEM_TYPES.iter().enumerate().take(flags::NUM_ITEM_TYPES) {
        columns += 1;
        let n: String = name.chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20} {}",
                c.grn_s(),
                counter,
                c.nrm_s(),
                n,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(b"\r\nEnter object type : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_extra_menu(g: &mut Game, di: usize, olc: &OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for (counter, name) in EXTRA_BITS.iter().enumerate().take(flags::NUM_ITEM_FLAGS) {
        columns += 1;
        let n: String = name.chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20} {}",
                c.grn_s(),
                counter + 1,
                c.nrm_s(),
                n,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(&olc.obj.as_ref().unwrap().extra_flags, &EXTRA_BITS, &mut bits);
    out.extend_from_slice(b"\r\nObject flags: ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter object extra flag (0 to quit) : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_perm_menu(g: &mut Game, di: usize, olc: &OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for counter in 1..flags::NUM_AFF_FLAGS {
        columns += 1;
        let n: String = mud_data::tables::AFFECTED_BITS[counter].chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20} {}",
                c.grn_s(),
                counter,
                c.nrm_s(),
                n,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(
        &olc.obj.as_ref().unwrap().perm_affects,
        &mud_data::tables::AFFECTED_BITS,
        &mut bits,
    );
    out.extend_from_slice(b"\r\nObject permanent flags: ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter object perm flag (0 to quit) : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_wear_menu(g: &mut Game, di: usize, olc: &OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    let mut columns = 0;
    for (counter, name) in WEAR_BITS.iter().enumerate().take(flags::NUM_ITEM_WEARS) {
        columns += 1;
        let n: String = name.chars().take(20).collect();
        out.extend_from_slice(
            format!(
                "{}{:2}{}) {:<20} {}",
                c.grn_s(),
                counter + 1,
                c.nrm_s(),
                n,
                if columns % 2 == 0 { "\r\n" } else { "" }
            )
            .as_bytes(),
        );
    }
    let mut bits: BStr = Vec::new();
    sprintbitarray(&olc.obj.as_ref().unwrap().wear_flags, &WEAR_BITS, &mut bits);
    out.extend_from_slice(b"\r\nWear flags: ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter wear flag, 0 to quit : ");
    write_to_desc(g, di, &out);
}

fn oedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    colors(g, di);
    clear_screen(g, di);
    let obj = olc.obj.as_ref().unwrap().as_ref().clone();
    let c = g.olc_colors;

    let buf1 = sprinttype(obj.type_flag, &ITEM_TYPES);
    let mut buf2: BStr = Vec::new();
    sprintbitarray(&obj.extra_flags, &EXTRA_BITS, &mut buf2);

    let nonempty = |s: &Option<BStr>, dflt: &'static [u8]| -> BStr {
        match s {
            Some(v) if !v.is_empty() => v.clone(),
            _ => dflt.to_vec(),
        }
    };

    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!("-- Item number : [{}{}{}]\r\n", c.cyn_s(), olc.number, c.nrm_s()).as_bytes(),
    );
    for (n, label, value) in [
        (&b"1"[..], &b") Keywords : "[..], nonempty(&obj.name, b"undefined")),
        (b"2", b") S-Desc   : ", nonempty(&obj.short_description, b"undefined")),
        (b"3", b") L-Desc   :-\r\n", nonempty(&obj.description, b"undefined")),
    ] {
        out.extend_from_slice(c.grn());
        out.extend_from_slice(n);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
        out.extend_from_slice(c.yel());
        out.extend_from_slice(&value);
        out.extend_from_slice(b"\r\n");
    }
    // The A-Desc line has no trailing \r\n of its own — the default text
    // carries one and a set description is expected to end with one.
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"4");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") A-Desc   :-\r\n");
    out.extend_from_slice(c.yel());
    out.extend_from_slice(&nonempty(&obj.action_description, b"Not Set.\r\n"));
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"5");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Type        : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&buf1);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"6");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Extra flags : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&buf2);
    out.extend_from_slice(b"\r\n");
    write_to_desc(g, di, &out);

    let mut buf1: BStr = Vec::new();
    sprintbitarray(&obj.wear_flags, &WEAR_BITS, &mut buf1);
    let mut buf2: BStr = Vec::new();
    sprintbitarray(&obj.perm_affects, &mud_data::tables::AFFECTED_BITS, &mut buf2);

    let mut out: BStr = Vec::new();
    out.extend_from_slice(c.grn());
    out.extend_from_slice(b"7");
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b") Wear flags  : ");
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&buf1);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}8{}) Weight      : {}{}\r\n{}9{}) Cost        : {}{}\r\n{}A{}) Cost/Day    : {}{}\r\n{}B{}) Timer       : {}{}\r\n{}C{}) Values      : {}{} {} {} {}\r\n",
            c.grn_s(), c.nrm_s(), c.cyn_s(), obj.weight,
            c.grn_s(), c.nrm_s(), c.cyn_s(), obj.cost,
            c.grn_s(), c.nrm_s(), c.cyn_s(), obj.cost_per_day,
            c.grn_s(), c.nrm_s(), c.cyn_s(), obj.timer,
            c.grn_s(), c.nrm_s(), c.cyn_s(),
            obj.values[0], obj.values[1], obj.values[2], obj.values[3]
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{}D{}) Applies menu\r\n{}E{}) Extra descriptions menu: {}{}{}\r\n{}M{}) Min Level   : {}{}\r\n{}P{}) Perm Affects: {}",
            c.grn_s(), c.nrm_s(),
            c.grn_s(), c.nrm_s(), c.cyn_s(),
            if obj.ex_descriptions.is_empty() { "Not Set." } else { "Set." },
            c.grn_s(),
            c.grn_s(), c.nrm_s(), c.cyn_s(), obj.level,
            c.grn_s(), c.nrm_s(), c.cyn_s()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&buf2);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}S{}) Script      : {}{}\r\n{}W{}) Copy object\r\n{}X{}) Delete object\r\n{}Q{}) Quit\r\nEnter choice : ",
            c.grn_s(), c.nrm_s(), c.cyn_s(),
            if olc.script.is_some() { "Set." } else { "Not Set." },
            c.grn_s(), c.nrm_s(),
            c.grn_s(), c.nrm_s(),
            c.grn_s(), c.nrm_s()
        )
        .as_bytes(),
    );
    write_to_desc(g, di, &out);

    olc.mode = OEDIT_MAIN_MENU;
}

// ---------------------------------------------------------------------------
// The main loop
// ---------------------------------------------------------------------------

pub fn oedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let mut arg = arg.to_vec();

    match olc.mode {
        OEDIT_CONFIRM_SAVESTRING => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    oedit_save_internally(g, di, &mut olc);
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                        let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                        let msg = format!("OLC: {} edits obj {}", name, olc.number);
                        g.mudlog(MudlogKind::Cmp, level, true, &msg);
                    }
                    if g.config.auto_save_olc {
                        let zone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);
                        if oedit_save_to_disk(g, zone) {
                            write_to_desc(g, di, b"Object saved to disk.\r\n");
                        } else {
                            write_to_desc(g, di, &crate::olc::save_failed("the object"));
                        }
                    } else {
                        write_to_desc(g, di, b"Object saved to memory.\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    let script = olc.script.take();
                    if let (Some(o), Some(script)) = (olc.obj.as_mut(), script) {
                        o.proto_script = script;
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'a') | Some(b'A') => {
                    oedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    write_to_desc(g, di, b"Do you wish to save your changes? : \r\n");
                    return Some(olc);
                }
            }
        }

        OEDIT_MAIN_MENU => {
            match arg.first().copied() {
                Some(b'q') | Some(b'Q') => {
                    if olc.value != 0 {
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = OEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                }
                Some(b'1') => {
                    write_to_desc(g, di, b"Enter keywords : ");
                    olc.mode = OEDIT_KEYWORD;
                }
                Some(b'2') => {
                    write_to_desc(g, di, b"Enter short desc : ");
                    olc.mode = OEDIT_SHORTDESC;
                }
                Some(b'3') => {
                    write_to_desc(g, di, b"Enter long desc :-\r\n| ");
                    olc.mode = OEDIT_LONGDESC;
                }
                Some(b'4') => {
                    olc.mode = OEDIT_ACTDESC;
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter action description:\r\n\r\n");
                    let old = olc.obj.as_ref().unwrap().action_description.clone();
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, crate::boards::MAX_MESSAGE_LENGTH, 0, old);
                    }
                    olc.str_target = Some(StrTarget::ObjActDesc);
                    olc.value = 1;
                }
                Some(b'5') => {
                    oedit_disp_type_menu(g, di);
                    olc.mode = OEDIT_TYPE;
                }
                Some(b'6') => {
                    oedit_disp_extra_menu(g, di, &olc);
                    olc.mode = OEDIT_EXTRAS;
                }
                Some(b'7') => {
                    oedit_disp_wear_menu(g, di, &olc);
                    olc.mode = OEDIT_WEAR;
                }
                Some(b'8') => {
                    write_to_desc(g, di, b"Enter weight : ");
                    olc.mode = OEDIT_WEIGHT;
                }
                Some(b'9') => {
                    write_to_desc(g, di, b"Enter cost : ");
                    olc.mode = OEDIT_COST;
                }
                Some(b'a') | Some(b'A') => {
                    write_to_desc(g, di, b"Enter cost per day : ");
                    olc.mode = OEDIT_COSTPERDAY;
                }
                Some(b'b') | Some(b'B') => {
                    write_to_desc(g, di, b"Enter timer : ");
                    olc.mode = OEDIT_TIMER;
                }
                Some(b'c') | Some(b'C') => {
                    let o = olc.obj.as_mut().unwrap();
                    o.values = [0; 4];
                    olc.value = 1;
                    oedit_disp_val1_menu(g, di, &mut olc);
                }
                Some(b'd') | Some(b'D') => oedit_disp_prompt_apply_menu(g, di, &mut olc),
                Some(b'e') | Some(b'E') => {
                    let o = olc.obj.as_mut().unwrap();
                    if o.ex_descriptions.is_empty() {
                        o.ex_descriptions.push(ExtraDesc::default());
                    }
                    olc.desc = Some(0);
                    oedit_disp_extradesc_menu(g, di, &mut olc);
                }
                Some(b'm') | Some(b'M') => {
                    write_to_desc(g, di, b"Enter new minimum level: ");
                    olc.mode = OEDIT_LEVEL;
                }
                Some(b'p') | Some(b'P') => {
                    oedit_disp_perm_menu(g, di, &olc);
                    olc.mode = OEDIT_PERM;
                }
                Some(b's') | Some(b'S') => {
                    olc.script_mode = SCRIPT_MAIN_MENU;
                    dg_script_menu(g, di, &mut olc);
                }
                Some(b'w') | Some(b'W') => {
                    write_to_desc(g, di, b"Copy what object? ");
                    olc.mode = OEDIT_COPY;
                }
                Some(b'x') | Some(b'X') => {
                    write_to_desc(g, di, b"Are you sure you want to delete this object? ");
                    olc.mode = OEDIT_DELETE;
                }
                _ => oedit_disp_menu(g, di, &mut olc),
            }
            return Some(olc);
        }

        crate::olc::trigedit::OLC_SCRIPT_EDIT => {
            if crate::olc::trigedit::dg_script_edit_parse(g, di, &mut olc, &arg) {
                return Some(olc);
            }
        }

        OEDIT_KEYWORD => {
            if genolc_checkstring(&mut arg) {
                olc.obj.as_mut().unwrap().name = Some(str_udup(&arg));
            }
        }
        OEDIT_SHORTDESC => {
            if genolc_checkstring(&mut arg) {
                olc.obj.as_mut().unwrap().short_description = Some(str_udup(&arg));
            }
        }
        OEDIT_LONGDESC => {
            if genolc_checkstring(&mut arg) {
                olc.obj.as_mut().unwrap().description = Some(str_udup(&arg));
            }
        }

        OEDIT_TYPE => {
            let number = atoi(&arg);
            if number < 0 || number >= flags::NUM_ITEM_TYPES as i32 {
                write_to_desc(g, di, b"Invalid choice, try again : ");
                return Some(olc);
            }
            let o = olc.obj.as_mut().unwrap();
            o.type_flag = number;
            o.values = [0; 4];
        }

        OEDIT_EXTRAS => {
            let number = atoi(&arg);
            if number < 0 || number > flags::NUM_ITEM_FLAGS as i32 {
                oedit_disp_extra_menu(g, di, &olc);
                return Some(olc);
            } else if number != 0 {
                let bit = (number - 1) as usize;
                let o = olc.obj.as_mut().unwrap();
                o.extra_flags[bit / 32] ^= 1 << (bit % 32);
                oedit_disp_extra_menu(g, di, &olc);
                return Some(olc);
            }
        }

        OEDIT_WEAR => {
            let number = atoi(&arg);
            if number < 0 || number > flags::NUM_ITEM_WEARS as i32 {
                write_to_desc(g, di, b"That's not a valid choice!\r\n");
                oedit_disp_wear_menu(g, di, &olc);
                return Some(olc);
            } else if number != 0 {
                let bit = (number - 1) as usize;
                let o = olc.obj.as_mut().unwrap();
                o.wear_flags[bit / 32] ^= 1 << (bit % 32);
                oedit_disp_wear_menu(g, di, &olc);
                return Some(olc);
            }
        }

        OEDIT_WEIGHT => {
            olc.obj.as_mut().unwrap().weight = limit(atoi(&arg), 0, MAX_OBJ_WEIGHT);
        }
        OEDIT_COST => {
            olc.obj.as_mut().unwrap().cost = limit(atoi(&arg), 0, MAX_OBJ_COST);
        }
        OEDIT_COSTPERDAY => {
            olc.obj.as_mut().unwrap().cost_per_day = limit(atoi(&arg), 0, MAX_OBJ_RENT);
        }
        OEDIT_TIMER => {
            olc.obj.as_mut().unwrap().timer = limit(atoi(&arg), 0, MAX_OBJ_TIMER);
        }
        OEDIT_LEVEL => {
            olc.obj.as_mut().unwrap().level = limit(atoi(&arg), 0, LVL_IMPL as i32);
        }

        OEDIT_PERM => {
            let number = atoi(&arg);
            if number != 0 {
                if number > 0 && number < flags::NUM_AFF_FLAGS as i32 {
                    // Setting AFF_CHARM on objects like this is dangerous.
                    if number != flags::AFF_CHARM as i32 {
                        let bit = number as usize;
                        let o = olc.obj.as_mut().unwrap();
                        o.perm_affects[bit / 32] ^= 1 << (bit % 32);
                    }
                }
                oedit_disp_perm_menu(g, di, &olc);
                return Some(olc);
            }
        }

        OEDIT_VALUE_1 => {
            let number = atoi(&arg);
            match item_type(&olc) {
                flags::ITEM_FURNITURE => {
                    if number < 0 || number > MAX_PEOPLE {
                        oedit_disp_val1_menu(g, di, &mut olc);
                    } else {
                        olc.obj.as_mut().unwrap().values[0] = number;
                        oedit_disp_val2_menu(g, di, &mut olc);
                    }
                    // Falls out of the switch and calls val2 again.
                    oedit_disp_val2_menu(g, di, &mut olc);
                    return Some(olc);
                }
                flags::ITEM_WEAPON => {
                    olc.obj.as_mut().unwrap().values[0] = number.max(-50).min(50);
                }
                flags::ITEM_CONTAINER => {
                    olc.obj.as_mut().unwrap().values[0] = limit(number, -1, MAX_CONTAINER_SIZE);
                }
                _ => olc.obj.as_mut().unwrap().values[0] = number,
            }
            oedit_disp_val2_menu(g, di, &mut olc);
            return Some(olc);
        }

        OEDIT_VALUE_2 => {
            let number = atoi(&arg);
            match item_type(&olc) {
                flags::ITEM_SCROLL | flags::ITEM_POTION => {
                    olc.obj.as_mut().unwrap().values[1] = if number == 0 || number == -1 {
                        -1
                    } else {
                        limit(number, 1, NUM_SPELLS)
                    };
                    oedit_disp_val3_menu(g, di, &mut olc);
                }
                flags::ITEM_CONTAINER => {
                    if number < 0 || number > 4 {
                        oedit_disp_container_flags_menu(g, di, &olc);
                    } else if number != 0 {
                        let o = olc.obj.as_mut().unwrap();
                        o.values[1] ^= 1 << (number - 1);
                        olc.value = 1;
                        oedit_disp_val2_menu(g, di, &mut olc);
                    } else {
                        oedit_disp_val3_menu(g, di, &mut olc);
                    }
                }
                flags::ITEM_WEAPON => {
                    olc.obj.as_mut().unwrap().values[1] = limit(number, 1, MAX_WEAPON_NDICE);
                    oedit_disp_val3_menu(g, di, &mut olc);
                }
                _ => {
                    olc.obj.as_mut().unwrap().values[1] = number;
                    oedit_disp_val3_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        OEDIT_VALUE_3 => {
            let mut number = atoi(&arg);
            let (min_val, max_val) = match item_type(&olc) {
                flags::ITEM_SCROLL | flags::ITEM_POTION => {
                    if number == 0 || number == -1 {
                        olc.obj.as_mut().unwrap().values[2] = -1;
                        oedit_disp_val4_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    (1, NUM_SPELLS)
                }
                flags::ITEM_WEAPON => (1, MAX_WEAPON_SDICE),
                flags::ITEM_WAND | flags::ITEM_STAFF => (0, 20),
                flags::ITEM_DRINKCON | flags::ITEM_FOUNTAIN => {
                    number -= 1;
                    (0, NUM_LIQ_TYPES - 1)
                }
                flags::ITEM_KEY => (0, 65099),
                _ => (-65000, 65000),
            };
            olc.obj.as_mut().unwrap().values[2] = limit(number, min_val, max_val);
            oedit_disp_val4_menu(g, di, &mut olc);
            return Some(olc);
        }

        OEDIT_VALUE_4 => {
            let number = atoi(&arg);
            let (min_val, max_val) = match item_type(&olc) {
                flags::ITEM_SCROLL | flags::ITEM_POTION => {
                    if number == 0 || number == -1 {
                        olc.obj.as_mut().unwrap().values[3] = -1;
                        oedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    (1, NUM_SPELLS)
                }
                flags::ITEM_WAND | flags::ITEM_STAFF => (1, NUM_SPELLS),
                flags::ITEM_WEAPON => (0, crate::fight::ATTACK_HIT_TEXT.len() as i32 - 1),
                _ => (-65000, 65000),
            };
            olc.obj.as_mut().unwrap().values[3] = limit(number, min_val, max_val);
        }

        OEDIT_PROMPT_APPLY => {
            let number = atoi(&arg);
            if number != 0 {
                if number < 0 || number > MAX_OBJ_AFFECT as i32 {
                    oedit_disp_prompt_apply_menu(g, di, &mut olc);
                    return Some(olc);
                }
                olc.value = number - 1;
                olc.mode = OEDIT_APPLY;
                oedit_disp_apply_menu(g, di, &mut olc);
                return Some(olc);
            }
        }

        OEDIT_APPLY => {
            let number = atoi(&arg);
            if number == 0 || number == 1 {
                let slot = olc.value.clamp(0, MAX_OBJ_AFFECT as i32 - 1) as usize;
                let o = olc.obj.as_mut().unwrap();
                o.affected[slot].location = 0;
                o.affected[slot].modifier = 0;
                oedit_disp_prompt_apply_menu(g, di, &mut olc);
            } else if number < 0 || number > flags::NUM_APPLIES as i32 {
                oedit_disp_apply_menu(g, di, &mut olc);
            } else {
                // Builders may not stack the same apply twice.
                let level = g
                    .descriptors
                    .get(di)
                    .and_then(|d| d.character)
                    .map(|c| g.ch(c).level)
                    .unwrap_or(0);
                if level < LVL_IMPL {
                    let o = olc.obj.as_ref().unwrap();
                    if o.affected.iter().any(|a| a.location == number) {
                        write_to_desc(g, di, b"Object already has that apply.");
                        return Some(olc);
                    }
                }
                let slot = olc.value.clamp(0, MAX_OBJ_AFFECT as i32 - 1) as usize;
                olc.obj.as_mut().unwrap().affected[slot].location = number - 1;
                write_to_desc(g, di, b"Modifier : ");
                olc.mode = OEDIT_APPLYMOD;
            }
            return Some(olc);
        }

        OEDIT_APPLYMOD => {
            let slot = olc.value.clamp(0, MAX_OBJ_AFFECT as i32 - 1) as usize;
            olc.obj.as_mut().unwrap().affected[slot].modifier = atoi(&arg);
            oedit_disp_prompt_apply_menu(g, di, &mut olc);
            return Some(olc);
        }

        OEDIT_EXTRADESC_KEY => {
            if genolc_checkstring(&mut arg) {
                let idx = olc.desc.unwrap_or(0);
                if let Some(xd) = olc.obj.as_mut().unwrap().ex_descriptions.get_mut(idx) {
                    xd.keyword = Some(str_udup(&arg));
                }
            }
            oedit_disp_extradesc_menu(g, di, &mut olc);
            return Some(olc);
        }

        OEDIT_EXTRADESC_MENU => {
            let number = atoi(&arg);
            match number {
                0 => {
                    let idx = olc.desc.unwrap_or(0);
                    let o = olc.obj.as_mut().unwrap();
                    let incomplete = o
                        .ex_descriptions
                        .get(idx)
                        .map(|x| x.keyword.is_none() || x.description.is_none())
                        .unwrap_or(false);
                    if incomplete {
                        o.ex_descriptions.remove(idx);
                        olc.desc = None;
                    }
                }
                1 => {
                    olc.mode = OEDIT_EXTRADESC_KEY;
                    write_to_desc(g, di, b"Enter keywords, separated by spaces :-\r\n| ");
                    return Some(olc);
                }
                2 => {
                    olc.mode = OEDIT_EXTRADESC_DESCRIPTION;
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        send_editor_help(g, chid);
                    }
                    write_to_desc(g, di, b"Enter the extra description:\r\n\r\n");
                    let idx = olc.desc.unwrap_or(0);
                    let old = olc
                        .obj
                        .as_ref()
                        .unwrap()
                        .ex_descriptions
                        .get(idx)
                        .and_then(|x| x.description.clone());
                    if let Some(text) = &old {
                        write_to_desc(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, crate::boards::MAX_MESSAGE_LENGTH, 0, old);
                    }
                    olc.str_target = Some(StrTarget::ObjExtraDesc);
                    olc.value = 1;
                    return Some(olc);
                }
                3 => {
                    let idx = olc.desc.unwrap_or(0);
                    let o = olc.obj.as_mut().unwrap();
                    let complete = o
                        .ex_descriptions
                        .get(idx)
                        .map(|x| x.keyword.is_some() && x.description.is_some())
                        .unwrap_or(false);
                    if complete {
                        if idx + 1 < o.ex_descriptions.len() {
                            olc.desc = Some(idx + 1);
                        } else {
                            o.ex_descriptions.push(ExtraDesc::default());
                            olc.desc = Some(o.ex_descriptions.len() - 1);
                        }
                    }
                    // Deliberate fall-through into the default arm.
                    oedit_disp_extradesc_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    oedit_disp_extradesc_menu(g, di, &mut olc);
                    return Some(olc);
                }
            }
        }

        OEDIT_COPY => {
            match g.world.real_object(atoi(&arg).clamp(0, u16::MAX as i32) as Idx) {
                Some(number) => oedit_setup_existing(g, &mut olc, number as usize),
                None => write_to_desc(g, di, b"That object does not exist.\r\n"),
            }
        }

        OEDIT_DELETE => {
            match arg.first().copied() {
                Some(b'y') | Some(b'Y') => {
                    let rnum = olc.obj_rnum;
                    if delete_object(g, rnum).is_some() {
                        write_to_desc(g, di, b"Object deleted.\r\n");
                        // Same toggle the save path honours.
                        if g.config.auto_save_olc {
                            crate::db::save_all(g);
                        }
                    } else {
                        write_to_desc(g, di, b"Couldn't delete the object!\r\n");
                    }
                    crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                    return None;
                }
                Some(b'n') | Some(b'N') => {
                    oedit_disp_menu(g, di, &mut olc);
                    olc.mode = OEDIT_MAIN_MENU;
                }
                _ => write_to_desc(g, di, b"Please answer 'Y' or 'N': "),
            }
            return Some(olc);
        }

        _ => {
            g.mudlog(
                MudlogKind::Brf,
                LVL_BUILDER,
                true,
                "SYSERR: OLC: Reached default case in oedit_parse()!",
            );
            write_to_desc(g, di, b"Oops...\r\n");
        }
    }

    olc.value = 1;
    oedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

pub fn oedit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    match olc.str_target.take() {
        Some(StrTarget::ObjActDesc) => {
            olc.obj.as_mut().unwrap().action_description = text;
        }
        Some(StrTarget::ObjExtraDesc) => {
            let idx = olc.desc.unwrap_or(0);
            if let Some(xd) = olc.obj.as_mut().unwrap().ex_descriptions.get_mut(idx) {
                xd.description = text;
            }
        }
        _ => {}
    }
    match olc.mode {
        OEDIT_ACTDESC => oedit_disp_menu(g, di, &mut olc),
        OEDIT_EXTRADESC_DESCRIPTION => oedit_disp_extradesc_menu(g, di, &mut olc),
        _ => {}
    }
    Some(olc)
}
