//! The trigger editor and the script-assignment menu that
//! redit/oedit/medit hand off to with `S`.
//!
//! This file starts with the assignment half (`dg_script_menu` /
//! `dg_script_edit_parse` / `dg_olc_script_copy`); trigedit proper follows.
//!
//! The insert-by-position walk is off by one — `--pos` is evaluated before
//! the list pointer advances, so "position 2" lands the trigger *third*.
//! Deletion has no such skew. Both behaviors are deliberate.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::tables::{OTRIG_TYPES, TRIG_TYPES, WTRIG_TYPES};
use mud_data::types::*;
use mud_world::model::Trigger;

use crate::act::BStr;
use crate::comm::{act, send_to_char, string_write, write_to_desc, TO_ROOM};
use crate::game::{Game, MudlogKind};
use crate::handler::atoi;
use crate::olc::{
    can_edit_zone, clear_screen, get_char_colors, send_cannot_edit, OlcData, CLEANUP_ALL,
};

pub const NUM_TRIG_TYPE_FLAGS: usize = 21;

/// Submodes of TRIGEDIT connectedness.
pub const TRIGEDIT_MAIN_MENU: i32 = 0;
pub const TRIGEDIT_TRIGTYPE: i32 = 1;
pub const TRIGEDIT_CONFIRM_SAVESTRING: i32 = 2;
pub const TRIGEDIT_NAME: i32 = 3;
pub const TRIGEDIT_INTENDED: i32 = 4;
pub const TRIGEDIT_TYPES: i32 = 5;
pub const TRIGEDIT_COMMANDS: i32 = 6;
pub const TRIGEDIT_NARG: i32 = 7;
pub const TRIGEDIT_ARGUMENT: i32 = 8;
pub const TRIGEDIT_COPY: i32 = 9;
pub const TRIGEDIT_CONFIRM_DELETE: i32 = 10;

/// "arbitrary > highest possible room number".
pub const OLC_SCRIPT_EDIT: i32 = 82766;
pub const SCRIPT_MAIN_MENU: i32 = 0;
pub const SCRIPT_NEW_TRIGGER: i32 = 1;
pub const SCRIPT_DEL_TRIGGER: i32 = 2;

/// dg_olc_script_copy: lift the edited thing's proto
/// script into OLC_SCRIPT. An empty list means no script.
pub fn dg_olc_script_copy(olc: &mut OlcData) {
    let orig = match olc.item_type {
        crate::dg::MOB_TRIGGER => olc.mob.as_ref().map(|m| m.proto_script.clone()),
        crate::dg::OBJ_TRIGGER => olc.obj.as_ref().map(|o| o.proto_script.clone()),
        _ => olc.room.as_ref().map(|r| r.proto_script.clone()),
    }
    .unwrap_or_default();
    olc.script = if orig.is_empty() { None } else { Some(orig) };
}

pub fn dg_script_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    // Make sure our input parser gets used.
    olc.mode = OLC_SCRIPT_EDIT;
    olc.script_mode = SCRIPT_MAIN_MENU;

    clear_screen(g, di);
    write_to_desc(g, di, b"     Triggers Attached:\r\n");

    let c = g.olc_colors;
    let list = olc.script.clone().unwrap_or_default();
    let item_type = olc.item_type;
    for (i, vnum) in list.iter().enumerate() {
        let (name, attach) = match g.world.real_trigger(*vnum) {
            Some(r) => {
                let t = &g.world.triggers[r as usize];
                (t.name.clone().unwrap_or_default(), t.attach_type)
            }
            // Unreachable: a menu-built list never holds an unknown vnum.
            None => (b"(null)".to_vec(), item_type),
        };
        let mut out: BStr = Vec::new();
        out.extend_from_slice(format!("     {:2}) [", i + 1).as_bytes());
        out.extend_from_slice(c.cyn());
        out.extend_from_slice(format!("{}", vnum).as_bytes());
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(b"] ");
        out.extend_from_slice(c.cyn());
        out.extend_from_slice(&name);
        out.extend_from_slice(c.nrm());
        write_to_desc(g, di, &out);
        if attach != item_type {
            let mut out: BStr = b"   ".to_vec();
            out.extend_from_slice(c.grn());
            out.extend_from_slice(b"** Mis-matched Trigger Type **");
            out.extend_from_slice(c.nrm());
            out.extend_from_slice(b"\r\n");
            write_to_desc(g, di, &out);
        } else {
            write_to_desc(g, di, b"\r\n");
        }
    }
    if list.is_empty() {
        write_to_desc(g, di, b"     <none>\r\n");
    }

    let mut out: BStr = b"\r\n".to_vec();
    for (key, label) in [
        (&b"N"[..], &b")  Attach trigger\r\n"[..]),
        (b"X", b")  Detach trigger\r\n"),
        (b"Q", b")  Quit\r\n\r\n"),
    ] {
        out.extend_from_slice(b" ");
        out.extend_from_slice(c.grn());
        out.extend_from_slice(key);
        out.extend_from_slice(c.nrm());
        out.extend_from_slice(label);
    }
    out.extend_from_slice(b"     Enter choice :");
    write_to_desc(g, di, &out);
}

/// Parse the "position, vnum" answer. Returns (count, position, vnum).
fn scan_pos_vnum(arg: &[u8]) -> (i32, i32, i32) {
    let mut p = 0usize;
    let mut count = 0;
    let mut vals = [0i32; 2];
    for slot in 0..2 {
        if slot == 1 {
            // Leading whitespace is skipped, but the comma is required.
            while p < arg.len() && arg[p].is_ascii_whitespace() {
                p += 1;
            }
            if p >= arg.len() || arg[p] != b',' {
                break;
            }
            p += 1;
        }
        while p < arg.len() && arg[p].is_ascii_whitespace() {
            p += 1;
        }
        let start = p;
        if p < arg.len() && (arg[p] == b'-' || arg[p] == b'+') {
            p += 1;
        }
        let digits = p;
        while p < arg.len() && arg[p].is_ascii_digit() {
            p += 1;
        }
        if p == digits {
            break;
        }
        vals[slot] = crate::handler::atoi(&arg[start..p]);
        count += 1;
    }
    (count, vals[0], vals[1])
}

/// dg_script_edit_parse. Returns false only for the
/// `q` exit, which hands control back to the calling editor.
pub fn dg_script_edit_parse(g: &mut Game, di: usize, olc: &mut OlcData, arg: &[u8]) -> bool {
    match olc.script_mode {
        SCRIPT_MAIN_MENU => {
            match arg.first().copied().map(|c| c.to_ascii_lowercase()) {
                Some(b'q') => return false,
                Some(b'n') => {
                    write_to_desc(g, di, b"\r\nPlease enter position, vnum   (ex: 1, 200):");
                    olc.script_mode = SCRIPT_NEW_TRIGGER;
                }
                Some(b'x') => {
                    write_to_desc(g, di, b"     Which entry should be deleted?  0 to abort :");
                    olc.script_mode = SCRIPT_DEL_TRIGGER;
                }
                _ => dg_script_menu(g, di, olc),
            }
            return true;
        }

        SCRIPT_NEW_TRIGGER => {
            let (count, mut pos, mut vnum) = scan_pos_vnum(arg);
            if count < 2 {
                vnum = if count == 1 { pos } else { -1 };
            }
            if count == 1 {
                pos = 999;
            }
            if pos > 0 && vnum != 0 {
                if g.world.real_trigger(vnum as u16).is_none() || !(0..=65535).contains(&vnum) {
                    write_to_desc(
                        g,
                        di,
                        b"Invalid Trigger VNUM!\r\nPlease enter position, vnum   (ex: 1, 200):",
                    );
                    return true;
                }
                let mut list = olc.script.clone().unwrap_or_default();
                if pos == 1 || list.is_empty() {
                    list.insert(0, vnum as Idx);
                } else {
                    let mut idx = 0usize;
                    let mut p = pos;
                    while idx + 1 < list.len() && {
                        p -= 1;
                        p != 0
                    } {
                        idx += 1;
                    }
                    list.insert(idx + 1, vnum as Idx);
                }
                olc.script = Some(list);
                olc.value += 1;
            }
        }

        SCRIPT_DEL_TRIGGER => {
            let pos = crate::handler::atoi(arg);
            if pos > 0 {
                let mut list = olc.script.clone().unwrap_or_default();
                if pos == 1 && !list.is_empty() {
                    olc.value += 1;
                    list.remove(0);
                } else {
                    let mut p = pos - 1;
                    let mut cur: Option<usize> = if list.is_empty() { None } else { Some(0) };
                    loop {
                        p -= 1;
                        if p == 0 || cur.is_none() {
                            break;
                        }
                        cur = match cur {
                            Some(i) if i + 1 < list.len() => Some(i + 1),
                            _ => None,
                        };
                    }
                    if let Some(i) = cur {
                        if i + 1 < list.len() {
                            olc.value += 1;
                            list.remove(i + 1);
                        }
                    }
                }
                olc.script = if list.is_empty() { None } else { Some(list) };
            }
        }

        _ => {}
    }

    dg_script_menu(g, di, olc);
    true
}

// ---------------------------------------------------------------------------
// trigedit proper
// ---------------------------------------------------------------------------

pub fn do_oasis_trigedit(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    // No building as a mob or while being forced.
    let Some(di) = g.ch(chid).desc else { return };
    if g.ch(chid).is_npc() || g.descriptors.get(di).map(|d| d.state) != Some(ConState::Playing) {
        return;
    }

    let argument = crate::interpreter::skip_spaces(argument);
    if argument.is_empty() || !argument[0].is_ascii_digit() {
        send_to_char(g, chid, b"Specify a trigger VNUM to edit.\r\n");
        return;
    }
    let number = atoi(argument);
    if number < 0 || number > u16::MAX as i32 {
        send_to_char(g, chid, b"That trigger VNUM can't exist.\r\n");
        return;
    }

    // Check that it isn't already being edited. Unlike redit's scan this one
    // names the builder outright, so an invisible one is still named.
    for other in g.descriptors.order.clone() {
        if g.descriptors.get(other).map(|d| d.state) != Some(ConState::Trigedit) {
            continue;
        }
        if crate::olc::olc_of(g, other).map(|o| o.number) != Some(number) {
            continue;
        }
        let who = g
            .descriptors
            .get(other)
            .and_then(|d| d.character)
            .map(|c| g.ch(c).get_name().to_vec())
            .unwrap_or_default();
        let mut msg = b"That trigger is currently being edited by ".to_vec();
        msg.extend_from_slice(&who);
        msg.extend_from_slice(b".\r\n");
        send_to_char(g, chid, &msg);
        return;
    }

    if g.olc.contains_key(&di) {
        g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: do_oasis_trigedit: Player already had olc structure.",
        );
        g.olc.remove(&di);
    }

    let mut olc = OlcData::new();

    let Some(znum) = crate::dg::mobcmd::real_zone_by_thing(g, number) else {
        send_to_char(g, chid, b"Sorry, there is no zone for that number!\r\n");
        return;
    };
    olc.zone_num = znum as i32;

    if !can_edit_zone(g, chid, znum as i32) {
        let zvnum = g.world.zones[znum].number as i32;
        send_cannot_edit(g, chid, zvnum);
        return;
    }
    olc.number = number;

    match g.world.real_trigger(number as Idx) {
        Some(real_num) => trigedit_setup_existing(g, &mut olc, real_num as usize),
        None => trigedit_setup_new(&mut olc),
    }

    trigedit_disp_menu(g, di, &mut olc);
    g.olc.insert(di, olc);
    if let Some(d) = g.descriptors.get_mut(di) {
        d.state = ConState::Trigedit;
    }

    act(g, b"$n starts using OLC.", true, Some(chid), None, None, TO_ROOM);
    g.ch_mut(chid).act.set(flags::PLR_WRITING);

    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
    let level = (LVL_IMMORT as i16).max(g.ch(chid).invis_lev()) as u8;
    let zvnum = g.world.zones[znum].number;
    let allowed = g.ch(chid).player_specials.as_ref().map_or(0, |ps| ps.olc_zone);
    let msg = format!(
        "OLC: {} starts editing zone {} [trigger](allowed zone {})",
        name, zvnum, allowed
    );
    g.mudlog(MudlogKind::Cmp, level, true, &msg);
}

fn trigedit_setup_new(olc: &mut OlcData) {
    let trig = Trigger {
        vnum: NOTHING,
        name: Some(b"new trigger".to_vec()),
        attach_type: crate::dg::MOB_TRIGGER,
        trigger_type: crate::dg::MTRIG_GREET,
        narg: 100,
        arglist: None,
        cmdlist: Vec::new(),
    };
    olc.trig_rnum = NOWHERE;
    // The cmdlist lives as one flat string until the trigger is saved.
    olc.storage = Some(b"%echo% This trigger commandlist is not complete!\r\n".to_vec());
    olc.trig = Some(Box::new(trig));
    olc.value = 0; // Has-changed flag.
}

pub fn trigedit_setup_existing(g: &Game, olc: &mut OlcData, rtrg_num: usize) {
    let trig = g.world.triggers[rtrg_num].clone();
    // Flatten the command list for the text editor; trigedit_save turns it
    // back into lines.
    let mut storage: BStr = Vec::new();
    for cmd in &trig.cmdlist {
        storage.extend_from_slice(cmd);
        storage.extend_from_slice(b"\r\n");
    }
    olc.trig_rnum = rtrg_num as Idx;
    olc.storage = Some(storage);
    olc.trig = Some(Box::new(trig));
    olc.value = 0;
}

fn trigedit_disp_menu(g: &mut Game, di: usize, olc: &mut OlcData) {
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    let trig = olc.trig.as_ref().unwrap().as_ref().clone();
    let (attach_type, trgtypes): (&[u8], BStr) = if trig.attach_type == crate::dg::OBJ_TRIGGER {
        (b"Objects", crate::quest::sprintbit(trig.trigger_type as i64, &OTRIG_TYPES))
    } else if trig.attach_type == crate::dg::WLD_TRIGGER {
        (b"Rooms", crate::quest::sprintbit(trig.trigger_type as i64, &WTRIG_TYPES))
    } else {
        (b"Mobiles", crate::quest::sprintbit(trig.trigger_type as i64, &TRIG_TYPES))
    };

    clear_screen(g, di);
    let c = g.olc_colors;
    let mut out: BStr = Vec::new();
    out.extend_from_slice(
        format!("Trigger Editor [{}{}{}]\r\n\r\n", c.grn_s(), olc.number, c.nrm_s()).as_bytes(),
    );
    out.extend_from_slice(format!("{}1){} Name         : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(trig.name.as_deref().unwrap_or(b"(null)"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}2){} Intended for : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(attach_type);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}3){} Trigger types: {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(&trgtypes);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!("{}4){} Numeric Arg  : {}{}\r\n", c.grn_s(), c.nrm_s(), c.yel_s(), trig.narg)
            .as_bytes(),
    );
    out.extend_from_slice(format!("{}5){} Arguments    : {}", c.grn_s(), c.nrm_s(), c.yel_s()).as_bytes());
    out.extend_from_slice(trig.arglist.as_deref().unwrap_or(b""));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("{}6){} Commands:\r\n{}", c.grn_s(), c.nrm_s(), c.cyn_s()).as_bytes());
    out.extend_from_slice(olc.storage.as_deref().unwrap_or(b"(null)"));
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(
        format!(
            "{}W{}) Copy Trigger\r\n{}X{}) Delete Trigger\r\n{}Q){} Quit\r\nEnter Choice :",
            c.grn_s(),
            c.nrm_s(),
            c.grn_s(),
            c.nrm_s(),
            c.grn_s(),
            c.nrm_s()
        )
        .as_bytes(),
    );
    write_to_desc(g, di, &out);
    olc.mode = TRIGEDIT_MAIN_MENU;
}

/// The trigger-type table for an attach type, and with it the number of
/// flags that table actually names.
///
/// Walking all three with `i < NUM_TRIG_TYPE_FLAGS` uses 21, which
/// is right for `trig_types` and one too many for
/// `otrig_types`/`wtrig_types`, which name 20. The type menu for an object
/// or room trigger would then print `types[20]`, the `"\n"` sentinel, as a
/// 21st entry: a raw line break padded to twenty columns, mid-menu. The
/// toggle guard has the
/// same bound, so a builder can also set the nameless bit 20 on an object or
/// room trigger, where nothing will ever read it. Both are bounded by the
/// table itself.
fn trig_type_table(attach_type: i32) -> &'static [&'static str] {
    if attach_type == crate::dg::WLD_TRIGGER {
        &WTRIG_TYPES
    } else if attach_type == crate::dg::OBJ_TRIGGER {
        &OTRIG_TYPES
    } else {
        &TRIG_TYPES
    }
}

/// trigedit_disp_types. Note it never sets the mode -- the
/// caller does, and the TRIGEDIT_TYPES parser re-displays without touching it.
fn trigedit_disp_types(g: &mut Game, di: usize, olc: &OlcData) {
    let trig = olc.trig.as_ref().unwrap();
    let types = trig_type_table(trig.attach_type);
    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
        get_char_colors(g, chid);
    }
    clear_screen(g, di);

    let c = g.olc_colors;
    let mut columns = 0;
    for (i, name) in types.iter().enumerate() {
        columns += 1;
        let mut n = name.as_bytes().to_vec();
        n.truncate(20);
        while n.len() < 20 {
            n.push(b' ');
        }
        let mut line: BStr =
            format!("{}{:2}{}) ", c.grn_s(), i + 1, c.nrm_s()).into_bytes();
        line.extend_from_slice(&n);
        line.extend_from_slice(b"  ");
        if columns % 2 == 0 {
            line.extend_from_slice(b"\r\n");
        }
        write_to_desc(g, di, &line);
    }
    let bits = crate::quest::sprintbit(trig.trigger_type as i64, types);
    let mut out: BStr = b"\r\nCurrent types : ".to_vec();
    out.extend_from_slice(c.cyn());
    out.extend_from_slice(&bits);
    out.extend_from_slice(c.nrm());
    out.extend_from_slice(b"\r\nEnter type (0 to quit) : ");
    write_to_desc(g, di, &out);
}

// ---------------------------------------------------------------------------
// Script syntax highlighting (by Victor Almeida)
// ---------------------------------------------------------------------------

/// str_replace: replace every occurrence, scanning forward
/// from just past each replacement so a replacement containing the needle
/// cannot loop forever.
fn str_replace(string: &[u8], substr: &[u8], replacement: &[u8]) -> BStr {
    if substr.is_empty() {
        return string.to_vec();
    }
    let mut out: BStr = Vec::with_capacity(string.len());
    let mut head = 0usize;
    while head < string.len() {
        if string[head..].starts_with(substr) {
            out.extend_from_slice(replacement);
            head += substr.len();
        } else {
            out.push(string[head]);
            head += 1;
        }
    }
    out.extend_from_slice(&string[head.min(string.len())..]);
    out
}

/// syntax_color_replacement[SYNTAX_TERMS][2]. Order is
/// load-bearing: each pass runs over the output of the last, which is what
/// the four "corrective" entries near the end exist to clean up.
const SYNTAX_COLOR_REPLACEMENT: [(&[u8], &[u8]); 49] = [
    // script logic (10)
    (b"if", b"\tcif\tn"),
    (b"elseif", b"\tcelseif\tn"),
    (b"else", b"\tcelse\tn"),
    (b"end", b"\tcend\tn"),
    (b"switch", b"\tcswitch\tn"),
    (b"case", b"\tccase\tY"),
    (b"default", b"\tcdefault\tn"),
    (b"break", b"\tcbreak\tn"),
    (b"while", b"\tcwhile\tn"),
    (b"done", b"\tcdone\tn"),
    // commands (15)
    (b"eval ", b"\tceval\tY "),
    (b"nop ", b"\tcnop\tY "),
    (b"extract ", b"\tcextract\tY "),
    (b"dg_letter ", b"\tcdg_letter\tY "),
    (b"makeuid ", b"\tcmakeuid\tY "),
    (b"dg_cast ", b"\tcdg_cast\tY "),
    (b"dg_affect ", b"\tcdg_affect\tY "),
    (b"global ", b"\tcglobal\tY "),
    (b"context ", b"\tccontext\tY "),
    (b"remote ", b"\tcremot\tce\tY "),
    (b"rdelete ", b"\tcrdelete\tY "),
    (b"set ", b"\tcset\tY "),
    (b"unset ", b"\tcunset\tY "),
    (b"attach ", b"\tcattach\tY "),
    (b"detach ", b"\tcdetach\tY "),
    // stopping (3)
    (b"wait", b"\trwait"),
    (b"return", b"\trreturn"),
    (b"halt", b"\trhalt"),
    // operands (12)
    (b"||", b"\tc||\tY"),
    (b"&&", b"\tc&&\tY"),
    (b"==", b"\tc==\tY"),
    (b"!=", b"\tc!=\tY"),
    (b"<=", b"\tc<=\tY"),
    (b">=", b"\tc>=\tY"),
    (b"< ", b"\tc< \tY"),
    (b"> ", b"\tc> \tY"),
    (b"/=", b"\tc/=\tY"),
    (b"!", b"\tc!\tn"),
    (b"(", b"\tc(\tY"),
    (b")", b"\tc)\tn"),
    // corrective (4)
    (b"\tc!\tn=", b"\tc!=\tY"),
    (b"%s\tcend\tn%", b"\tm%\tosend%\tn"),
    (b"%\tc)", b"\tm%\tc)"),
    (b")\tn%", b")\tm%"),
    // variables (5)
    (b"% ", b"\tm%\tn "),
    (b"%,", b"\tm%\tn,"),
    (b"%.", b"\tm%\tn."),
    (b"%:", b"\tm%\tn:"),
    (b"%", b"\tm%\to"),
];

/// The command names, coloured after the syntax terms above.
const COMMAND_COLOR_REPLACEMENT: [(&[u8], &[u8]); 35] = [
    // Mob specific commands (25)
    (b"mlog", b"\tcmlog\tn"),
    (b"masound", b"\tcmasound\tn"),
    (b"mkill", b"\tcmkill\tn"),
    (b"mjunk", b"\tcmjunk\tn"),
    (b"mdamage", b"\tcmdamage\tn"),
    (b"mdoor", b"\tcmdoor\tn"),
    (b"mecho", b"\tcmecho\tn"),
    (b"mrecho", b"\tcmrecho\tn"),
    (b"mechoaround", b"\tcmechoaround\tn"),
    (b"msend", b"\tcmsend\tn"),
    (b"mload", b"\tcmload\tn"),
    (b"mpurge", b"\tcmpurge\tn"),
    (b"mgoto", b"\tcmgoto\tn"),
    (b"mteleport", b"\tcmteleport\tn"),
    (b"mforce", b"\tcmforce\tn"),
    (b"mhunt", b"\tcmhunt\tn"),
    (b"mremember", b"\tcmremember\tn"),
    (b"mforget", b"\tcmforget\tn"),
    (b"mtransform", b"\tcmtransform\tn"),
    (b"mzoneecho", b"\tcmzoneecho\tn"),
    (b"mfollow", b"\tcmfollow\tn"),
    (b"mquest", b"\tcmquest\tn"),
    (b"malign", b"\tcmalign\tn"),
    (b"mcast", b"\tcmcast\tn"),
    (b"mdismiss", b"\tcmdismiss\tn"),
    // common commands (10)
    (b"drop ", b"\tcdrop \tn"),
    (b"emote ", b"\tcemote \tn"),
    (b"give ", b"\tcgive \tn"),
    (b"say ", b"\tcsay \tn"),
    (b"tell ", b"\tctell \tn"),
    (b"unlock ", b"\tcunlock \tn"),
    (b"lock ", b"\tclock \tn"),
    (b"open ", b"\tcopen \tn"),
    (b"close ", b"\tcclose \tn"),
    (b"junk ", b"\tcjunk \tn"),
];

/// script_syntax_highlighting.
///
/// Runs of line endings collapse, so blank lines never reach the output. A
/// line whose first
/// non-space byte is `*` is a comment: it gets `\tg` and none of the keyword
/// passes. Everything else runs the syntax table, then the command table,
/// then every social in the command list — an unanchored replace, so a social
/// name inside a longer word is coloured too.
fn script_syntax_highlighting(g: &mut Game, di: usize, string: &[u8]) {
    let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) else { return };
    let socials: Vec<BStr> = g
        .commands
        .iter()
        .filter(|c| matches!(c.handler, crate::interpreter::Handler::Action))
        .map(|c| c.command.clone())
        .collect();

    let mut buffer: BStr = Vec::new();
    for tok in string.split(|c| *c == b'\r' || *c == b'\n') {
        if tok.is_empty() {
            continue; // empty tokens are skipped
        }
        let mut line = tok.to_vec();
        let mut comment = false;
        for i in 0..=line.len() {
            let byte = line.get(i).copied();
            match byte {
                Some(b' ') => continue,
                Some(b'*') => {
                    line = str_replace(&line, b"*", b"\tg*");
                    comment = true;
                }
                _ => comment = false,
            }
            break;
        }

        if !comment {
            for (needle, rep) in SYNTAX_COLOR_REPLACEMENT {
                line = str_replace(&line, needle, rep);
            }
            for (needle, rep) in COMMAND_COLOR_REPLACEMENT {
                line = str_replace(&line, needle, rep);
            }
            for social in &socials {
                let mut rep: BStr = b"\tc".to_vec();
                rep.extend_from_slice(social);
                rep.extend_from_slice(b"\tn");
                line = str_replace(&line, social, &rep);
            }
        }

        let room = MAX_STRING_LENGTH.saturating_sub(buffer.len()).saturating_sub(1);
        let take = line.len().min(room);
        buffer.extend_from_slice(&line[..take]);
        let room = MAX_STRING_LENGTH.saturating_sub(buffer.len()).saturating_sub(1);
        let tail = b"\tn\r\n";
        buffer.extend_from_slice(&tail[..tail.len().min(room)]);
    }
    crate::act::informative::page_string(g, chid, &buffer);
}

// ---------------------------------------------------------------------------
// trigedit_parse
// ---------------------------------------------------------------------------

pub fn trigedit_parse(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    arg: &[u8],
) -> Option<Box<OlcData>> {
    let first = arg.first().copied().unwrap_or(0).to_ascii_lowercase();
    match olc.mode {
        TRIGEDIT_MAIN_MENU => {
            match first {
                b'q' => {
                    if olc.value != 0 {
                        // Anything been changed?
                        if olc.trig.as_ref().is_some_and(|t| t.trigger_type == 0) {
                            write_to_desc(
                                g,
                                di,
                                b"Invalid Trigger Type! Answer a to abort quit!\r\n",
                            );
                        }
                        write_to_desc(g, di, b"Do you wish to save your changes? : ");
                        olc.mode = TRIGEDIT_CONFIRM_SAVESTRING;
                    } else {
                        crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                        return None;
                    }
                }
                b'1' => {
                    olc.mode = TRIGEDIT_NAME;
                    write_to_desc(g, di, b"Name: ");
                }
                b'2' => {
                    olc.mode = TRIGEDIT_INTENDED;
                    write_to_desc(g, di, b"0: Mobiles, 1: Objects, 2: Rooms: ");
                }
                b'3' => {
                    olc.mode = TRIGEDIT_TYPES;
                    trigedit_disp_types(g, di, &olc);
                }
                b'4' => {
                    olc.mode = TRIGEDIT_NARG;
                    write_to_desc(g, di, b"Numeric argument: ");
                }
                b'5' => {
                    olc.mode = TRIGEDIT_ARGUMENT;
                    write_to_desc(g, di, b"Argument: ");
                }
                b'6' => {
                    olc.mode = TRIGEDIT_COMMANDS;
                    write_to_desc(g, di, b"Enter trigger commands: (/s saves /h for help)\r\n\r\n");
                    // backstr is the copy /a restores; set it only when
                    // there was something to show.
                    let storage = olc.storage.clone();
                    if let Some(text) = storage.as_deref() {
                        clear_screen(g, di);
                        script_syntax_highlighting(g, di, text);
                    }
                    if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                        string_write(g, chid, MAX_CMD_LENGTH, 0, storage);
                    }
                    olc.value = 1;
                }
                b'w' => {
                    write_to_desc(g, di, b"Copy what trigger? ");
                    olc.mode = TRIGEDIT_COPY;
                }
                b'x' => {
                    if g.world.real_trigger(olc.number as Idx).is_none() {
                        write_to_desc(
                            g,
                            di,
                            b"That trigger has never been saved -- quit without saving instead.\r\n",
                        );
                        trigedit_disp_menu(g, di, &mut olc);
                        return Some(olc);
                    }
                    write_to_desc(g, di, b"Are you sure you want to delete this trigger? ");
                    olc.mode = TRIGEDIT_CONFIRM_DELETE;
                }
                _ => {
                    trigedit_disp_menu(g, di, &mut olc);
                }
            }
            return Some(olc);
        }

        TRIGEDIT_CONFIRM_DELETE => {
            match first {
                b'y' => {
                    // Resolve by VNUM, the way trigedit_save does. olc.trig_rnum
                    // is not trustworthy here: another builder saving a trigger
                    // renumbers it underneath this editor, and for a trigger
                    // that has never been saved it is NOWHERE to begin with.
                    let drnum = g.world.real_trigger(olc.number as Idx);
                    let dzone = crate::dg::mobcmd::real_zone_by_thing(g, olc.number);

                    if let Some(drnum) = drnum {
                        if delete_trigger(g, drnum) {
                            let invis = g
                                .descriptors
                                .get(di)
                                .and_then(|d| d.character)
                                .map_or(0, |c| g.ch(c).invis_lev());
                            let written =
                                dzone.is_some_and(|z| trigedit_write_zone(g, z, invis));
                            if written {
                                write_to_desc(g, di, b"Trigger deleted.\r\n");
                            } else {
                                // Not "a reboot will bring it back": by the time
                                // the write is attempted the prototypes that
                                // referenced this trigger have already been saved
                                // without it, so a reboot returns the trigger on
                                // its own. That ordering is deliberate -- the
                                // other way round strands dangling references
                                // instead -- but the builder should be told which
                                // of the two they have.
                                write_to_desc(
                                    g,
                                    di,
                                    b"The trigger is gone from memory, but its own file could not be \
                                      written. A reboot brings the trigger back, though not the things \
                                      it was attached to -- those have already been saved without it. \
                                      See the syslog.\r\n",
                                );
                            }
                            if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                                let name =
                                    String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                                let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                                let msg =
                                    format!("OLC: {} deletes trigger {}", name, olc.number);
                                g.mudlog(MudlogKind::Cmp, level, true, &msg);
                            }
                            crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                            return None;
                        }
                    }
                    // Nothing was deleted, so nothing is thrown away either --
                    // cleanup_olc here would discard the builder's unsaved work.
                    write_to_desc(g, di, b"Could not delete that trigger.\r\n");
                    trigedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                b'n' => {
                    trigedit_disp_menu(g, di, &mut olc);
                    return Some(olc);
                }
                _ => {
                    write_to_desc(g, di, b"Invalid choice!\r\n");
                    write_to_desc(g, di, b"Delete this trigger? : ");
                    return Some(olc);
                }
            }
        }

        TRIGEDIT_CONFIRM_SAVESTRING => match first {
            b'y' => {
                trigedit_save(g, di, &mut olc);
                if let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) {
                    let name = String::from_utf8_lossy(g.ch(chid).get_name()).into_owned();
                    let level = (LVL_BUILDER as i16).max(g.ch(chid).invis_lev()) as u8;
                    let msg = format!("OLC: {} edits trigger {}", name, olc.number);
                    g.mudlog(MudlogKind::Cmp, level, true, &msg);
                }
                crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                return None;
            }
            b'n' => {
                crate::olc::cleanup_olc(g, di, olc, CLEANUP_ALL);
                return None;
            }
            // 'a' aborts the quit and falls through to the menu redisplay.
            b'a' => {}
            _ => {
                write_to_desc(g, di, b"Invalid choice!\r\n");
                write_to_desc(g, di, b"Do you wish to save your changes? : ");
                return Some(olc);
            }
        },

        TRIGEDIT_NAME => {
            let mut text = arg.to_vec();
            mud_net::editor::smash_tilde(&mut text);
            let name = if text.is_empty() { b"undefined".to_vec() } else { text };
            if let Some(t) = olc.trig.as_mut() {
                t.name = Some(name);
            }
            olc.value += 1;
        }

        TRIGEDIT_INTENDED => {
            // The guard `>= MOB_TRIGGER || <= WLD_TRIGGER` is true for
            // every integer, so any number lands in attach_type.
            let v = atoi(arg);
            if v >= crate::dg::MOB_TRIGGER || v <= crate::dg::WLD_TRIGGER {
                if let Some(t) = olc.trig.as_mut() {
                    t.attach_type = v;
                }
            }
            olc.value += 1;
        }

        TRIGEDIT_NARG => {
            if let Some(t) = olc.trig.as_mut() {
                t.narg = 100.min(atoi(arg).max(0));
            }
            olc.value += 1;
        }

        TRIGEDIT_ARGUMENT => {
            let mut text = arg.to_vec();
            mud_net::editor::smash_tilde(&mut text);
            if let Some(t) = olc.trig.as_mut() {
                t.arglist = if text.is_empty() { None } else { Some(text) };
            }
            olc.value += 1;
        }

        TRIGEDIT_TYPES => {
            let i = atoi(arg);
            if i != 0 {
                let n = olc.trig.as_ref().map_or(0, |t| trig_type_table(t.attach_type).len());
                if (0..=n as i32).contains(&i) {
                    if let Some(t) = olc.trig.as_mut() {
                        t.trigger_type ^= 1u32 << (i - 1);
                    }
                }
                olc.value += 1;
                trigedit_disp_types(g, di, &olc);
                return Some(olc);
            }
            // Falls through to the main-menu redisplay below.
        }

        TRIGEDIT_COPY => {
            match g.world.real_trigger(atoi(arg) as Idx) {
                Some(i) => trigedit_setup_existing(g, &mut olc, i as usize),
                None => write_to_desc(g, di, b"That trigger does not exist.\r\n"),
            }
        }

        TRIGEDIT_COMMANDS => {}

        _ => {}
    }

    olc.mode = TRIGEDIT_MAIN_MENU;
    trigedit_disp_menu(g, di, &mut olc);
    Some(olc)
}

pub fn trigedit_string_cleanup(
    g: &mut Game,
    di: usize,
    mut olc: Box<OlcData>,
    text: Option<BStr>,
    _saved: bool,
) -> Option<Box<OlcData>> {
    if olc.mode == TRIGEDIT_COMMANDS {
        olc.storage = text;
        trigedit_disp_menu(g, di, &mut olc);
    }
    Some(olc)
}

// ---------------------------------------------------------------------------
// trigedit_save
// ---------------------------------------------------------------------------

/// Split OLC_STORAGE back into command lines. Empty runs are dropped, and an
/// empty buffer becomes the one-line placeholder.
fn recompile_cmdlist(storage: Option<&[u8]>) -> Vec<BStr> {
    let Some(s) = storage else {
        return vec![b"* No Script".to_vec()];
    };
    let lines: Vec<BStr> = s
        .split(|c| *c == b'\n' || *c == b'\r')
        .filter(|l| !l.is_empty())
        .map(|l| l.to_vec())
        .collect();
    if lines.is_empty() {
        vec![b"* No script".to_vec()]
    } else {
        lines
    }
}

pub fn trigedit_save(g: &mut Game, di: usize, olc: &mut OlcData) {
    let mut trig = olc.trig.as_ref().unwrap().as_ref().clone();
    trig.cmdlist = recompile_cmdlist(olc.storage.as_deref());
    trig.vnum = olc.number as Idx;

    match g.world.real_trigger(olc.number as Idx) {
        Some(rnum) => {
            let rnum = rnum as usize;
            g.world.triggers[rnum] = trig.clone();
            // The while/done state lives in a side table keyed by
            // (rnum, line), so it has to be dropped with the prototype.
            g.trig_line_state.retain(|(nr, _), _| *nr != rnum as Idx);
            // Go through the mud and replace every live copy.
            refresh_live_triggers(g, rnum as Idx, &trig);
        }
        None => {
            // A new trigger: find its slot, shift everything above it.
            let rnum = g
                .world
                .triggers
                .iter()
                .position(|t| t.vnum as i32 > olc.number)
                .unwrap_or(g.world.triggers.len());
            g.world.triggers.insert(rnum, trig);
            g.trig_counts.insert(rnum, 0);
            g.world.trig_map.clear();
            for (i, t) in g.world.triggers.iter().enumerate() {
                g.world.trig_map.insert(t.vnum, i as Idx);
            }
            // Line state is keyed by rnum, so it has to shift with the
            // trigger table.
            let shifted: std::collections::HashMap<(Idx, usize), crate::dg::LineState> = g
                .trig_line_state
                .drain()
                .map(|((nr, line), st)| {
                    (if nr as usize >= rnum { (nr + 1, line) } else { (nr, line) }, st)
                })
                .collect();
            g.trig_line_state = shifted;
            olc.trig_rnum = rnum as Idx;

            // Fix every live trigger and every other builder's copy.
            shift_live_trig_rnums(g, rnum as Idx);
            for dsc in g.descriptors.order.clone() {
                if dsc == di
                    || g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Trigedit)
                {
                    continue;
                }
                if let Some(other) = g.olc.get_mut(&dsc) {
                    if other.trig_rnum != NOWHERE && other.trig_rnum >= rnum as Idx {
                        other.trig_rnum += 1;
                    }
                }
            }

            // Zone reset 'T' commands hold an rnum as well: renum_zone_table
            // turns the file's vnum into one at boot, and an insert moves the
            // trigger they name. Leaving them alone makes the reset attach
            // whichever trigger slid into the slot, and the next zedit save
            // then writes that wrong vnum out to the .zon file.
            let nw = NOWHERE as i32;
            for zone in g.world.zones.iter_mut() {
                for cmd in zone.cmds.iter_mut() {
                    if cmd.command == b'T' && cmd.arg2 != nw && cmd.arg2 >= rnum as i32 {
                        cmd.arg2 += 1;
                    }
                }
            }
            // And every open zedit's copy of them. zedit_setup takes whole
            // reset commands out of the zone table, so the same rnum exists
            // twice over; repairing only the live one leaves the editor to
            // write its stale copy back on an ordinary save.
            for dsc in g.descriptors.order.clone() {
                if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Zedit) {
                    continue;
                }
                let Some(other) = g.olc.get_mut(&dsc) else { continue };
                let Some(zone) = other.zone.as_mut() else { continue };
                for cmd in zone.cmds.iter_mut() {
                    if cmd.command == b'T' && cmd.arg2 != nw && cmd.arg2 >= rnum as i32 {
                        cmd.arg2 += 1;
                    }
                }
            }
        }
    }

    // The trigger goes to disk NOW rather than waiting for the builder: if it
    // were lost after being assigned to something, the next reboot would
    // SYSERR in a way that is hard to trace back.
    let invis = g
        .descriptors
        .get(di)
        .and_then(|d| d.character)
        .map_or(0, |c| g.ch(c).invis_lev());
    if !trigedit_write_zone(g, olc.zone_num as usize, invis) {
        return;
    }
    write_to_desc(g, di, b"Trigger saved to disk.\r\n");
}

/// Write one zone's triggers out, and refresh the index.
///
/// Lifted out of [`trigedit_save`] so anything that changes a zone's triggers
/// can persist them the same way -- the delete does. Copying the writer
/// instead is how this codebase's ancestor ended up with two sets of world
/// writers that drifted apart.
fn trigedit_write_zone(g: &mut Game, zrnum: usize, invis_lev: i16) -> bool {
    let level = (LVL_GOD as i16).max(invis_lev) as u8;
    let zone = g.world.zones[zrnum].number;
    let body = mud_world::write::trg::write_file(&g.world, zrnum as u16);
    let dir = g.lib_dir.join("world").join("trg");
    let newname = dir.join(format!("{}.new", zone));
    let oldname = dir.join(format!("{}.trg", zone));

    if std::fs::write(&newname, &body).is_err() {
        let msg = format!("SYSERR: OLC: Can't open trig file \"{}\"", newname.display());
        g.mudlog(MudlogKind::Brf, level, true, &msg);
        // A partial .new left lying around is read by nothing, but it is also
        // one boot away from being mistaken for a good file by a human.
        let _ = std::fs::remove_file(&newname);
        return false;
    }

    // Try the replace before removing the target. Where rename overwrites, a
    // failure leaves the old file exactly where it was; removing first means a
    // failed rename has already thrown the zone's triggers away with nothing
    // put back. Platforms that cannot overwrite still get their remove, on the
    // retry.
    if std::fs::rename(&newname, &oldname).is_err() {
        let _ = std::fs::remove_file(&oldname);
        if let Err(e) = std::fs::rename(&newname, &oldname) {
            let msg = format!(
                "SYSERR: OLC: Can't rename \"{}\" to \"{}\": {}",
                newname.display(),
                oldname.display(),
                e
            );
            g.mudlog(MudlogKind::Brf, level, true, &msg);
            return false;
        }
    }

    crate::olc::genzon::create_world_index(g, zone as i32, "trg");
    true
}

/// The `trigger_list` walk of trigedit_save: every live instance of this
/// prototype takes the new name/arglist/cmdlist, loses any pending wait and
/// its variables, and restarts at the top.
fn refresh_live_triggers(g: &mut Game, rnum: Idx, proto: &Trigger) {
    for go in live_script_owners(g) {
        let mut cancel: Vec<u64> = Vec::new();
        {
            let Some(sc) = g.script_of_mut(go) else { continue };
            for t in sc.trig_list.iter_mut() {
                if t.nr != rnum {
                    continue;
                }
                t.name = proto.name.clone().unwrap_or_default();
                t.arglist = proto.arglist.clone().unwrap_or_default();
                if let Some(ev) = t.wait_event.take() {
                    cancel.push(ev);
                }
                t.var_list.clear();
                t.curr_state = 0;
                t.trigger_type = proto.trigger_type;
                t.attach_type = proto.attach_type;
                t.narg = proto.narg;
                t.depth = 0;
            }
        }
        // event_cancel: the queued TrigWait dies with the reload.
        g.events.retain(|e| match e.kind {
            crate::game::EventKind::TrigWait { event_id, .. } => !cancel.contains(&event_id),
            _ => true,
        });
    }
}

/// The rnum fixup after an insertion: `GET_TRIG_RNUM(t) += (t > rnum)`.
fn shift_live_trig_rnums(g: &mut Game, rnum: Idx) {
    for go in live_script_owners(g) {
        let Some(sc) = g.script_of_mut(go) else { continue };
        for t in sc.trig_list.iter_mut() {
            // >=, not >. An insert at `rnum` moves the previous occupant to
            // rnum+1 and corrects that prototype, but its live copies still
            // hold rnum -- and > skips exactly them, leaving them naming the
            // trigger that was just created.
            if t.nr != NOWHERE && t.nr >= rnum {
                t.nr += 1;
            }
        }
    }
}

/// Live triggers hang off whatever they are attached to, so walking them
/// all means walking every scripted entity.
fn live_script_owners(g: &Game) -> Vec<crate::dg::GoId> {
    let mut out = Vec::new();
    for id in g.character_list.iter() {
        if g.try_ch(*id).is_some_and(|c| c.script.is_some()) {
            out.push(crate::dg::GoId::Char(*id));
        }
    }
    for id in g.object_list.iter() {
        if g.try_obj(*id).is_some_and(|o| o.script.is_some()) {
            out.push(crate::dg::GoId::Obj(*id));
        }
    }
    for r in 0..g.rooms.len() {
        if g.rooms[r].script.is_some() {
            out.push(crate::dg::GoId::Room(r as RoomRnum));
        }
    }
    out
}

/// Remove every live copy of one trigger from a thing's script, and say how
/// many went. The live trigger knows its rnum but not its owner, so the
/// owners are what get walked.
fn trigedit_strip_live(g: &mut Game, go: crate::dg::GoId, rnum: Idx) -> usize {
    let gone: Vec<crate::dg::TrigInstance> = {
        let Some(sc) = g.script_of_mut(go) else { return 0 };
        let (gone, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut sc.trig_list).into_iter().partition(|t| t.nr == rnum);
        sc.trig_list = kept;
        if gone.is_empty() {
            return 0;
        }
        // Same bookkeeping remove_trigger does: an emptied or shortened list
        // must stop advertising types it no longer has.
        sc.types = sc.trig_list.iter().fold(0, |acc, t| acc | t.trigger_type);
        gone
    };
    for t in &gone {
        crate::dg::extract_trigger_book(g, t);
    }
    gone.len()
}

/// Drop one vnum from a prototype's default-trigger list. These hold VNUMS,
/// not rnums, which is why deleting a trigger needs no renumbering pass over
/// them the way `delete_object` does over its own references.
fn trigedit_strip_proto(list: &mut Vec<Idx>, vnum: Idx) -> bool {
    let before = list.len();
    list.retain(|&v| v != vnum);
    list.len() != before
}

/// Which of a zone's files a trigger delete makes stale.
const TRIGDEL_MOB: u8 = 1 << 0;
const TRIGDEL_OBJ: u8 = 1 << 1;
const TRIGDEL_WLD: u8 = 1 << 2;
const TRIGDEL_ZON: u8 = 1 << 3;

/// Delete a trigger prototype outright: every live copy, every reference from
/// a mobile/object/room prototype, and the table slot itself.
///
/// The inverse of [`trigedit_save`]'s insert arm, and it has to undo exactly
/// what that arm does -- including the three side tables (`trig_map`,
/// `trig_counts`, `trig_line_state`). Missing one of those is silent
/// corruption rather than a compile error.
pub fn delete_trigger(g: &mut Game, rnum: Idx) -> bool {
    if rnum == NOWHERE || rnum as usize >= g.world.triggers.len() {
        return false;
    }

    // Which zone files this delete makes stale, by rnum. Marked as the
    // prototypes are stripped and written out at the end, once each.
    let mut dirty: Vec<u8> = vec![0; g.world.zones.len()];

    let vnum = g.world.triggers[rnum as usize].vnum;
    let zrnum = crate::dg::mobcmd::real_zone_by_thing(g, vnum as i32);

    let name = g.world.triggers[rnum as usize]
        .name
        .clone()
        .map_or_else(|| "unnamed".to_string(), |n| String::from_utf8_lossy(&n).into_owned());
    g.log(format!("GenOLC: delete_trigger: Deleting trigger #{} ({}).", vnum, name));

    // 1. Detach and free every attached copy, wherever it is running.
    let mut live = 0usize;
    for go in live_script_owners(g) {
        let n = trigedit_strip_live(g, go, rnum);
        if n == 0 {
            continue;
        }
        live += n;
        // A script whose last trigger has been removed is extracted here: an
        // empty trig_list is a state nothing else in the game produces, and
        // nothing downstream expects it.
        if g.script_of(go).is_some_and(|sc| sc.trig_list.is_empty()) {
            crate::dg::extract_script(g, go);
        }
    }

    // 2. Stop the prototypes handing it out again on the next load, and mark
    //    the zones that changed so the reference goes from the .mob/.obj/.wld
    //    files too. Stripping only the in-memory copy is not enough: on the
    //    next boot the file still says `T <vnum>` and dg_read_trigger logs
    //    "Trigger vnum #N asked for but non-existant!" for every one, on every
    //    reboot, forever.
    let mut refs = 0usize;
    for i in 0..g.world.mob_protos.len() {
        if trigedit_strip_proto(&mut g.world.mob_protos[i].proto_script, vnum) {
            refs += 1;
            let mvnum = g.world.mob_protos[i].vnum as i32;
            if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, mvnum) {
                let zvnum = g.world.zones[z].number;
                crate::db::add_to_save_list(g, zvnum, crate::db::SL_MOB);
                dirty[z] |= TRIGDEL_MOB;
            }
        }
    }
    for i in 0..g.world.obj_protos.len() {
        if trigedit_strip_proto(&mut g.world.obj_protos[i].proto_script, vnum) {
            refs += 1;
            let ovnum = g.world.obj_protos[i].vnum as i32;
            if let Some(z) = crate::dg::mobcmd::real_zone_by_thing(g, ovnum) {
                let zvnum = g.world.zones[z].number;
                crate::db::add_to_save_list(g, zvnum, crate::db::SL_OBJ);
                dirty[z] |= TRIGDEL_OBJ;
            }
        }
    }
    for r in 0..g.world.rooms.len() {
        if trigedit_strip_proto(&mut g.world.rooms[r].proto_script, vnum) {
            refs += 1;
            let z = g.world.rooms[r].zone as usize;
            let zvnum = g.world.zones[z].number;
            crate::db::add_to_save_list(g, zvnum, crate::db::SL_WLD);
            dirty[z] |= TRIGDEL_WLD;
        }
    }

    // And the lists being assembled inside an open r/o/medit script menu,
    // which are vnum lists like the prototypes'. The deleting builder's own
    // olc is not in this map -- it is held by the caller -- and a trigedit
    // session has no script list of its own, so nothing is missed.
    for dsc in g.descriptors.order.clone() {
        if let Some(other) = g.olc.get_mut(&dsc) {
            if let Some(list) = other.script.as_mut() {
                if trigedit_strip_proto(list, vnum) {
                    refs += 1;
                }
            }
        }
    }

    if live > 0 || refs > 0 {
        g.log(format!(
            "GenOLC: delete_trigger: detached {} live copy/copies and {} prototype reference(s).",
            live, refs
        ));
    }

    // 3. The prototype and its table slot, plus the three side tables keyed by
    //    rnum that have to move with it.
    g.world.triggers.remove(rnum as usize);
    if (rnum as usize) < g.trig_counts.len() {
        g.trig_counts.remove(rnum as usize);
    }
    g.world.trig_map.clear();
    for (i, t) in g.world.triggers.iter().enumerate() {
        g.world.trig_map.insert(t.vnum, i as Idx);
    }
    // The deleted trigger's own line state goes rather than shifting, or the
    // trigger that slides into the slot inherits its while/done bookkeeping.
    let shifted: std::collections::HashMap<(Idx, usize), crate::dg::LineState> = g
        .trig_line_state
        .drain()
        .filter(|((nr, _), _)| *nr != rnum)
        .map(|((nr, line), st)| (if nr > rnum { (nr - 1, line) } else { (nr, line) }, st))
        .collect();
    g.trig_line_state = shifted;

    // 4. Any live trigger above the hole is now pointing one slot too high.
    for go in live_script_owners(g) {
        let Some(sc) = g.script_of_mut(go) else { continue };
        for t in sc.trig_list.iter_mut() {
            // Strict `>`, the mirror of shift_live_trig_rnums' deliberate
            // `>=`. Every instance that named `rnum` itself is already gone.
            if t.nr != NOWHERE && t.nr > rnum {
                t.nr -= 1;
            }
        }
    }
    // Anyone else sitting in trigedit above the hole is now one slot high.
    for dsc in g.descriptors.order.clone() {
        if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Trigedit) {
            continue;
        }
        if let Some(other) = g.olc.get_mut(&dsc) {
            if other.trig_rnum != NOWHERE && other.trig_rnum > rnum {
                other.trig_rnum -= 1;
            }
        }
    }

    // 5. The zone resets. A 'T' naming the trigger that is going has nothing
    //    left to attach, so it goes too; the rest shift down. Removing one
    //    slides the next command into its slot, so the cursor must not
    //    advance or a second consecutive 'T' is skipped.
    let nw = NOWHERE as i32;
    for zon in 0..g.world.zones.len() {
        let mut cmd_no = 0usize;
        while cmd_no < g.world.zones[zon].cmds.len() {
            let (cmd, arg2) = {
                let c = &g.world.zones[zon].cmds[cmd_no];
                (c.command, c.arg2)
            };
            if cmd == b'T' && arg2 != nw {
                if arg2 == rnum as i32 {
                    crate::olc::genzon::delete_zone_command(
                        &mut g.world.zones[zon],
                        cmd_no as i32,
                    );
                    dirty[zon] |= TRIGDEL_ZON;
                    continue;
                } else if arg2 > rnum as i32 {
                    g.world.zones[zon].cmds[cmd_no].arg2 -= 1;
                    dirty[zon] |= TRIGDEL_ZON;
                }
            }
            cmd_no += 1;
        }
        if dirty[zon] & TRIGDEL_ZON != 0 {
            let zvnum = g.world.zones[zon].number;
            crate::db::add_to_save_list(g, zvnum, crate::db::SL_ZON);
        }
    }

    // And the zedit copies of those same commands. zedit_setup takes whole
    // reset commands out of the zone table, so a 'T' command's rnum exists
    // twice over -- once in the live table and once inside every open zedit.
    //
    // The snapshot is disabled in place rather than removed, which is what the
    // live table above does. It has to be: zedit holds a cursor into this
    // array across prompts (`olc.value` indexes `olc.zone.cmds`), and removing
    // an entry from under that cursor leaves it naming a different command --
    // or, at the end of the list, none.
    //
    // '*' is the mark renum_zone_table already puts on a reset command that
    // cannot resolve. reset_zone treats it as a no-op, the .zon writer drops
    // it, and zedit's menu stops reading the trigger table for it -- so the
    // zone still reaches disk without the reference, and every index into the
    // array stays where the editor left it.
    for dsc in g.descriptors.order.clone() {
        if g.descriptors.get(dsc).map(|d| d.state) != Some(ConState::Zedit) {
            continue;
        }
        let (dead, on_it) = {
            let Some(other) = g.olc.get_mut(&dsc) else { continue };
            // olc.value only names a command while one is being filled in; in
            // the menu it is a leftover.
            let editing = matches!(
                other.mode,
                crate::olc::zedit::ZEDIT_COMMAND_TYPE
                    | crate::olc::zedit::ZEDIT_IF_FLAG
                    | crate::olc::zedit::ZEDIT_ARG1
                    | crate::olc::zedit::ZEDIT_ARG2
                    | crate::olc::zedit::ZEDIT_ARG3
                    | crate::olc::zedit::ZEDIT_SARG1
                    | crate::olc::zedit::ZEDIT_SARG2
            );
            let cursor = other.value;
            let Some(zone) = other.zone.as_mut() else { continue };
            let mut dead = 0usize;
            let mut on_it = false;
            for (i, c) in zone.cmds.iter_mut().enumerate() {
                if c.command != b'T' || c.arg2 == nw {
                    continue;
                }
                if c.arg2 == rnum as i32 {
                    c.command = b'*';
                    dead += 1;
                    if editing && i as i32 == cursor {
                        on_it = true;
                    }
                } else if c.arg2 > rnum as i32 {
                    c.arg2 -= 1;
                }
            }
            (dead, on_it)
        };

        // Say so rather than letting a command quietly stop working under
        // someone who is looking straight at it.
        if dead > 0 {
            let msg = format!(
                "\r\nTrigger {} has been deleted; {} reset command{} in the zone you are \
                 editing no longer does anything.\r\n",
                vnum,
                dead,
                if dead == 1 { "" } else { "s" }
            );
            write_to_desc(g, dsc, msg.as_bytes());
        }
        // And if one of them is the command they are part-way through filling
        // in, take them off it. Every argument prompt switches on the command
        // byte and none of them has a case for '*'.
        if on_it {
            if let Some(other) = g.olc.get_mut(&dsc) {
                other.mode = crate::olc::zedit::ZEDIT_MAIN_MENU;
            }
            write_to_desc(
                g,
                dsc,
                b"That was the command you were editing, so you are back at the zone menu \
                  -- press return for it.\r\n",
            );
        }
    }

    // 6. Write the stale files now instead of only queueing them. The trigger
    //    has already left the .trg by the time the caller returns, so a server
    //    that dies before the next saveall comes back up with prototypes still
    //    naming a trigger that no longer exists -- and dg_read_trigger logs
    //    "asked for but non-existant" for every one of them, on every boot
    //    after that. Queueing alone inverts the order: the thing depended on
    //    goes first and its dependents follow only if someone saves. Each
    //    save_ call clears its own save-list entry, so nobody else's pending
    //    edits are flushed with them.
    for z in 0..dirty.len() {
        if dirty[z] & TRIGDEL_MOB != 0 {
            crate::olc::genmob::save_mobiles(g, Some(z));
        }
        if dirty[z] & TRIGDEL_OBJ != 0 {
            crate::olc::genobj::save_objects(g, Some(z));
        }
        if dirty[z] & TRIGDEL_WLD != 0 {
            crate::olc::genwld::save_rooms(g, Some(z));
        }
        if dirty[z] & TRIGDEL_ZON != 0 {
            crate::db::save_zone(g, z);
        }
    }

    if zrnum.is_none() {
        g.mudlog(
            MudlogKind::Brf,
            LVL_BUILDER,
            true,
            "SYSERR: GenOLC: delete_trigger: Cannot determine trigger zone.",
        );
    }

    true
}
