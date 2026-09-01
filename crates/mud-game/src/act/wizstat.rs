//! `stat` and `vstat` (1364-1425) — the immortal
//! inspector for rooms, objects, characters, zones and triggers.

use mud_data::flags::{ITEM_ARMOR, ITEM_CONTAINER, ITEM_DRINKCON, ITEM_FOOD, ITEM_FOUNTAIN, ITEM_FURNITURE, ITEM_KEY, ITEM_LIGHT, ITEM_MONEY, ITEM_NOTE, ITEM_POTION, ITEM_SCROLL, ITEM_STAFF, ITEM_WAND, ITEM_WEAPON};
use mud_data::ids::{CharId, ObjId};
use mud_data::tables::{
    self, AFFECTED_BITS, APPLY_TYPES, CONNECTED_TYPES, CONTAINER_BITS, DIRS, DRINKS,
    EQUIPMENT_TYPES, EXIT_BITS, EXTRA_BITS, GENDERS, ITEM_TYPES, POSITION_TYPES, ROOM_BITS,
    SECTOR_TYPES, WEAR_BITS,
};
use mud_data::types::*;

use crate::act::informative::sprintbitarray;
use crate::act::{pad_right, BStr};
use crate::comm::{cc, send_to_char, C_NRM, KCYN, KGRN, KNRM, KYEL};
use crate::game::Game;
use crate::handler::{atoi, can_see, can_see_obj, get_number, is_abbrev, pers};
use crate::interpreter::{half_chop, is_number, one_argument};
use crate::quest::sprintbit;
use crate::spec::{MobSpec, ObjSpec, RoomSpec};

/// GET_OLC_ZONE sentinels.
pub const AEDIT_PERMISSION: i32 = 999;
pub const HEDIT_PERMISSION: i32 = 888;
pub const ALL_PERMISSION: i32 = 666;

pub fn sprinttype(type_: i32, names: &[&str]) -> BStr {
    match usize::try_from(type_).ok().and_then(|i| names.get(i)) {
        Some(n) => n.as_bytes().to_vec(),
        None => b"UNDEFINED".to_vec(),
    }
}

/// get_spec_func_name. Procs outside the table — every King's-Castle
/// routine — have no name, which goes through `%s` as "(null)".
pub fn mob_spec_name(spec: MobSpec) -> &'static [u8] {
    match spec {
        MobSpec::Mayor => b"Mayor",
        MobSpec::Postmaster => b"Postmaster",
        MobSpec::Receptionist => b"Receptionist",
        MobSpec::Cryogenicist => b"Cryogenicist",
        MobSpec::Guild => b"Guildmaster",
        MobSpec::QuestMaster => b"Questmaster",
        MobSpec::ShopKeeper => b"Shopkeeper",
        _ => b"(null)",
    }
}

pub fn obj_spec_name(spec: ObjSpec) -> &'static [u8] {
    match spec {
        ObjSpec::GenBoard => b"Bulletin Board",
        ObjSpec::Bank => b"Bank",
    }
}

pub fn room_spec_name(spec: RoomSpec) -> &'static [u8] {
    match spec {
        RoomSpec::PetShop => b"Pet Shop",
        RoomSpec::Dump => b"Dump",
    }
}

fn list_zone_commands_room(g: &mut Game, chid: CharId, rvnum: i32) {
    let zrnum = crate::dg::mobcmd::real_zone_by_thing(g, rvnum);
    let rrnum = g.real_room(rvnum);
    let (Some(zrnum), Some(rrnum)) = (zrnum, rrnum) else {
        send_to_char(g, chid, b"No zone information available.\r\n");
        return;
    };
    let (cyn, yel, nrm) = (
        cc(g, chid, C_NRM, KCYN).to_vec(),
        cc(g, chid, C_NRM, KYEL).to_vec(),
        cc(g, chid, C_NRM, KNRM).to_vec(),
    );
    let mut out = b"Zone commands in this room:".to_vec();
    out.extend_from_slice(&yel);
    out.extend_from_slice(b"\r\n");

    let mut count = 0;
    let mut cmd_room: i32 = NOWHERE as i32;
    for i in 0..g.world.zones[zrnum].cmds.len() {
        let zc = g.world.zones[zrnum].cmds[i].clone();
        match zc.command {
            b'M' | b'O' | b'T' | b'V' => cmd_room = zc.arg3,
            b'D' | b'R' => cmd_room = zc.arg1,
            _ => {}
        }
        if cmd_room != rrnum as i32 {
            continue;
        }
        count += 1;
        let then: &[u8] = if zc.if_flag != 0 { b" then " } else { b"" };
        out.extend_from_slice(then);
        match zc.command {
            b'M' => {
                let p = &g.world.mob_protos[zc.arg1 as usize];
                out.extend_from_slice(b"Load ");
                out.extend_from_slice(p.short_descr.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(p.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(format!("], Max : {}\r\n", zc.arg2).as_bytes());
            }
            b'G' | b'O' => {
                let p = &g.world.obj_protos[zc.arg1 as usize];
                out.extend_from_slice(if zc.command == b'G' { b"Give it " } else { b"Load " });
                out.extend_from_slice(p.short_description.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(p.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(format!("], Max : {}\r\n", zc.arg2).as_bytes());
            }
            b'E' => {
                let p = &g.world.obj_protos[zc.arg1 as usize];
                out.extend_from_slice(b"Equip with ");
                out.extend_from_slice(p.short_description.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(p.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(b"], ");
                out.extend_from_slice(
                    EQUIPMENT_TYPES.get(zc.arg3 as usize).copied().unwrap_or("").as_bytes(),
                );
                out.extend_from_slice(format!(", Max : {}\r\n", zc.arg2).as_bytes());
            }
            b'P' => {
                let inner = &g.world.obj_protos[zc.arg1 as usize];
                let outer = &g.world.obj_protos[zc.arg3 as usize];
                out.extend_from_slice(b"Put ");
                out.extend_from_slice(inner.short_description.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(inner.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(b"] in ");
                out.extend_from_slice(outer.short_description.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(outer.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(format!("], Max : {}\r\n", zc.arg2).as_bytes());
            }
            b'R' => {
                let p = &g.world.obj_protos[zc.arg2 as usize];
                out.extend_from_slice(b"Remove ");
                out.extend_from_slice(p.short_description.as_deref().unwrap_or(b""));
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(p.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(b"] from room.\r\n");
            }
            b'D' => {
                out.extend_from_slice(b"Set door ");
                out.extend_from_slice(DIRS.get(zc.arg2 as usize).copied().unwrap_or("").as_bytes());
                out.extend_from_slice(b" as ");
                out.extend_from_slice(match zc.arg3 {
                    0 => &b"open"[..],
                    1 => &b"closed"[..],
                    _ => &b"locked"[..],
                });
                out.extend_from_slice(b".\r\n");
            }
            b'T' => {
                let t = &g.world.triggers[zc.arg2 as usize];
                out.extend_from_slice(b"Attach trigger ");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(t.name.as_deref().unwrap_or(b""));
                out.extend_from_slice(&yel);
                out.extend_from_slice(b" [");
                out.extend_from_slice(&cyn);
                out.extend_from_slice(t.vnum.to_string().as_bytes());
                out.extend_from_slice(&yel);
                out.extend_from_slice(b"] to ");
                out.extend_from_slice(attach_word(zc.arg1));
                out.extend_from_slice(b"\r\n");
            }
            b'V' => {
                out.extend_from_slice(b"Assign global ");
                out.extend_from_slice(zc.sarg1.as_deref().unwrap_or(b""));
                out.extend_from_slice(format!(":{} to ", zc.arg2).as_bytes());
                out.extend_from_slice(attach_word(zc.arg1));
                out.extend_from_slice(b" = ");
                out.extend_from_slice(zc.sarg2.as_deref().unwrap_or(b""));
                out.extend_from_slice(b"\r\n");
            }
            _ => out.extend_from_slice(b"<Unknown Command>\r\n"),
        }
    }
    out.extend_from_slice(&nrm);
    if count == 0 {
        out.extend_from_slice(b"None!\r\n");
    }
    send_to_char(g, chid, &out);
}

fn attach_word(kind: i32) -> &'static [u8] {
    match kind {
        crate::dg::MOB_TRIGGER => b"mobile",
        crate::dg::OBJ_TRIGGER => b"object",
        crate::dg::WLD_TRIGGER => b"room",
        _ => b"????",
    }
}

pub fn do_stat_room(g: &mut Game, chid: CharId, rm: RoomRnum) {
    let (cyn, grn, yel, nrm) = (
        cc(g, chid, C_NRM, KCYN).to_vec(),
        cc(g, chid, C_NRM, KGRN).to_vec(),
        cc(g, chid, C_NRM, KYEL).to_vec(),
        cc(g, chid, C_NRM, KNRM).to_vec(),
    );
    let r = rm as usize;

    let mut out = b"Room name: ".to_vec();
    out.extend_from_slice(&cyn);
    out.extend_from_slice(g.world.rooms[r].name.as_deref().unwrap_or(b""));
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n");

    let zone_num = g.world.zones[g.world.rooms[r].zone as usize].number;
    let vnum = g.world.rooms[r].vnum as i32;
    let sect = sprinttype(g.world.rooms[r].sector_type, &SECTOR_TYPES);
    let script_id = crate::dg::room_script_id(g, rm);
    out.extend_from_slice(format!("Zone: [{:3}], VNum: [", zone_num).as_bytes());
    out.extend_from_slice(&grn);
    out.extend_from_slice(format!("{:5}", vnum).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(format!("], RNum: [{:5}], IDNum: [{:5}], Type: ", rm, script_id).as_bytes());
    out.extend_from_slice(&sect);
    out.extend_from_slice(b"\r\n");

    let mut flags_buf = Vec::new();
    sprintbitarray(&g.world.rooms[r].room_flags, &ROOM_BITS, &mut flags_buf);
    out.extend_from_slice(b"SpecProc: ");
    match g.room_specs[r] {
        None => out.extend_from_slice(b"None"),
        Some(s) => out.extend_from_slice(room_spec_name(s)),
    }
    out.extend_from_slice(b", Flags: ");
    out.extend_from_slice(&flags_buf);
    out.extend_from_slice(b"\r\n");

    out.extend_from_slice(b"Description:\r\n");
    match g.world.rooms[r].description.as_deref() {
        Some(d) if !d.is_empty() => out.extend_from_slice(d),
        _ => out.extend_from_slice(b"  None.\r\n"),
    }

    if !g.world.rooms[r].ex_descriptions.is_empty() {
        out.extend_from_slice(b"Extra descs:");
        out.extend_from_slice(&cyn);
        for ed in &g.world.rooms[r].ex_descriptions {
            out.extend_from_slice(b" [");
            out.extend_from_slice(ed.keyword.as_deref().unwrap_or(b""));
            out.push(b']');
        }
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"Chars present:");
    out.extend_from_slice(&yel);
    send_to_char(g, chid, &out);

    let people = g.rooms[r].people.clone();
    let mut column = 14usize;
    let mut found = 0;
    for (i, &k) in people.iter().enumerate() {
        if g.try_ch(k).is_none() || !can_see(g, chid, k) {
            continue;
        }
        let kind: &[u8] = if !g.ch(k).is_npc() {
            b"PC"
        } else if g.ch(k).mob_rnum == NOBODY {
            b"NPC"
        } else {
            b"MOB"
        };
        let mut chunk: BStr = if found > 0 { b",".to_vec() } else { Vec::new() };
        chunk.push(b' ');
        chunk.extend_from_slice(g.ch(k).get_name());
        chunk.push(b'(');
        chunk.extend_from_slice(kind);
        chunk.push(b')');
        found += 1;
        column += chunk.len();
        send_to_char(g, chid, &chunk);
        if column >= 62 {
            let more = people[i + 1..].iter().any(|&n| g.try_ch(n).is_some());
            send_to_char(g, chid, if more { b",\r\n" } else { b"\r\n" });
            found = 0;
            column = 0;
        }
    }
    send_to_char(g, chid, &nrm);
    // A5: the list has no line terminator of its own, having relied on the
    // wrap firing on the last entry — which only the broken column
    // arithmetic guaranteed. With real widths it has to end the line, or
    // "Contents:" runs onto the end of it.
    if column != 0 {
        send_to_char(g, chid, b"\r\n");
    }

    let contents = g.rooms[r].contents.clone();
    if !contents.is_empty() {
        let mut out = b"Contents:".to_vec();
        out.extend_from_slice(&grn);
        send_to_char(g, chid, &out);
        let mut column = 9usize;
        let mut found = 0;
        for (i, &j) in contents.iter().enumerate() {
            if !g.try_obj_alive(j) || !can_see_obj(g, chid, j) {
                continue;
            }
            let mut chunk: BStr = if found > 0 { b",".to_vec() } else { Vec::new() };
            chunk.push(b' ');
            chunk.extend_from_slice(crate::handler::obj_short(g, j));
            found += 1;
            column += chunk.len();
        send_to_char(g, chid, &chunk);
            if column >= 62 {
                let more = contents[i + 1..].iter().any(|&n| g.try_obj_alive(n));
                send_to_char(g, chid, if more { b",\r\n" } else { b"\r\n" });
                found = 0;
                column = 0;
            }
        }
        send_to_char(g, chid, &nrm);
        if column != 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }

    for i in 0..crate::fight::dir_count(g) {
        let Some(exit) = g.world.rooms[r].dir_option[i].as_deref() else { continue };
        let (to_room, key, keyword, info, gen_desc) = (
            exit.to_room,
            exit.key,
            exit.keyword.clone(),
            exit.exit_info,
            exit.general_description.clone(),
        );
        let mut buf1: BStr = Vec::new();
        if to_room == NOWHERE {
            buf1.push(b' ');
            buf1.extend_from_slice(&cyn);
            buf1.extend_from_slice(b"NONE");
            buf1.extend_from_slice(&nrm);
        } else {
            buf1.extend_from_slice(&cyn);
            buf1.extend_from_slice(
                format!("{:5}", g.world.rooms[to_room as usize].vnum).as_bytes(),
            );
            buf1.extend_from_slice(&nrm);
        }
        let bits = sprintbit(info as i64, &EXIT_BITS);
        let mut out = b"Exit ".to_vec();
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&pad_right(DIRS[i].as_bytes(), 5));
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b":  To: [");
        out.extend_from_slice(&buf1);
        out.extend_from_slice(
            format!("], Key: [{:5}], Keywords: ", if key as u32 == NOTHING as u32 { -1i32 } else { key as i32 })
                .as_bytes(),
        );
        out.extend_from_slice(keyword.as_deref().unwrap_or(b"None"));
        out.extend_from_slice(b", Type: ");
        out.extend_from_slice(&bits);
        out.extend_from_slice(b"\r\n");
        match gen_desc {
            Some(d) if !d.is_empty() => out.extend_from_slice(&d),
            _ => out.extend_from_slice(b"  No exit description.\r\n"),
        }
        send_to_char(g, chid, &out);
    }

    do_sstat(g, chid, crate::dg::GoId::Room(rm));
    list_zone_commands_room(g, chid, vnum);
}

fn do_sstat(g: &mut Game, chid: CharId, go: crate::dg::GoId) {
    send_to_char(g, chid, b"Triggers:\r\n");
    if g.script_of(go).is_none() {
        send_to_char(g, chid, b"  None.\r\n");
        return;
    }
    crate::dg::commands::script_stat(g, chid, go);
}

pub fn do_stat_object(g: &mut Game, chid: CharId, j: ObjId) {
    let (cyn, grn, yel, nrm) = (
        cc(g, chid, C_NRM, KCYN).to_vec(),
        cc(g, chid, C_NRM, KGRN).to_vec(),
        cc(g, chid, C_NRM, KYEL).to_vec(),
        cc(g, chid, C_NRM, KNRM).to_vec(),
    );

    let short = g.obj(j).short_description.clone();
    let mut out = b"Name: '".to_vec();
    out.extend_from_slice(&yel);
    out.extend_from_slice(match short.as_deref() {
        Some(s) => s,
        None => proto_short(g, j).unwrap_or(b"<None>"),
    });
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"', Keywords: ");
    out.extend_from_slice(crate::handler::obj_name(g, j));
    out.extend_from_slice(b"\r\n");

    let rnum = g.obj(j).item_number;
    let vnum = if rnum == NOTHING { NOTHING as i32 } else { g.world.obj_protos[rnum as usize].vnum as i32 };
    let type_str = sprinttype(g.obj(j).type_flag, &ITEM_TYPES);
    let script_id = crate::dg::obj_script_id(g, j);
    out.extend_from_slice(b"VNum: [");
    out.extend_from_slice(&grn);
    out.extend_from_slice(format!("{:5}", vnum).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(
        format!("], RNum: [{:5}], Idnum: [{:5}], Type: ", rnum as i32, script_id).as_bytes(),
    );
    out.extend_from_slice(&type_str);
    out.extend_from_slice(b", SpecProc: ");
    match g.obj_specs.get(rnum as usize).copied().flatten() {
        Some(s) => out.extend_from_slice(obj_spec_name(s)),
        None => out.extend_from_slice(b"None"),
    }
    out.extend_from_slice(b"\r\n");

    let desc = g.obj(j).description.clone().or_else(|| proto_desc(g, j));
    out.extend_from_slice(b"L-Desc: '");
    out.extend_from_slice(&yel);
    out.extend_from_slice(desc.as_deref().unwrap_or(b"<None>"));
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"'\r\n");

    let adesc = crate::handler::obj_action_desc(g, j).map(|d| d.to_vec());
    out.extend_from_slice(b"A-Desc: '");
    out.extend_from_slice(&yel);
    out.extend_from_slice(adesc.as_deref().unwrap_or(b"<None>"));
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"'\r\n");

    let exds = obj_exdescs(g, j);
    if !exds.is_empty() {
        out.extend_from_slice(b"Extra descs:");
        out.extend_from_slice(&cyn);
        for kw in &exds {
            out.extend_from_slice(b" [");
            out.extend_from_slice(kw);
            out.push(b']');
        }
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
    }

    let mut buf = Vec::new();
    sprintbitarray(&g.obj(j).wear_flags.0, &WEAR_BITS, &mut buf);
    out.extend_from_slice(b"Can be worn on: ");
    out.extend_from_slice(&buf);
    out.extend_from_slice(b"\r\n");

    let mut buf = Vec::new();
    sprintbitarray(&g.obj(j).perm_affects.0, &AFFECTED_BITS, &mut buf);
    out.extend_from_slice(b"Set char bits : ");
    out.extend_from_slice(&buf);
    out.extend_from_slice(b"\r\n");

    let mut buf = Vec::new();
    sprintbitarray(&g.obj(j).extra_flags.0, &EXTRA_BITS, &mut buf);
    out.extend_from_slice(b"Extra flags   : ");
    out.extend_from_slice(&buf);
    out.extend_from_slice(b"\r\n");

    let o = g.obj(j);
    out.extend_from_slice(
        format!(
            "Weight: {}, Value: {}, Cost/day: {}, Timer: {}, Min level: {}\r\n",
            o.weight, o.cost, o.cost_per_day, o.timer, o.level
        )
        .as_bytes(),
    );

    let in_room = o.in_room;
    let room_vnum = if in_room == NOWHERE { NOWHERE as i32 } else { g.world.rooms[in_room as usize].vnum as i32 };
    out.extend_from_slice(format!("In room: {} (", room_vnum).as_bytes());
    if in_room == NOWHERE {
        out.extend_from_slice(b"Nowhere");
    } else {
        out.extend_from_slice(g.world.rooms[in_room as usize].name.as_deref().unwrap_or(b""));
    }
    out.extend_from_slice(b"), ");

    out.extend_from_slice(b"In object: ");
    match g.obj(j).in_obj {
        Some(o2) if g.try_obj_alive(o2) => out.extend_from_slice(crate::handler::obj_short(g, o2)),
        _ => out.extend_from_slice(b"None"),
    }
    out.extend_from_slice(b", Carried by: ");
    match g.obj(j).carried_by.filter(|&c| g.try_ch(c).is_some()) {
        Some(c) => out.extend_from_slice(g.ch(c).get_name()),
        None => out.extend_from_slice(b"Nobody"),
    }
    out.extend_from_slice(b", Worn by: ");
    match g.obj(j).worn_by.filter(|&c| g.try_ch(c).is_some()) {
        Some(c) => out.extend_from_slice(g.ch(c).get_name()),
        None => out.extend_from_slice(b"Nobody"),
    }
    out.extend_from_slice(b"\r\n");

    let v = g.obj(j).values;
    match g.obj(j).type_flag {
        ITEM_LIGHT => {
            if v[2] == -1 {
                out.extend_from_slice(b"Hours left: Infinite\r\n");
            } else {
                out.extend_from_slice(format!("Hours left: [{}]\r\n", v[2]).as_bytes());
            }
        }
        ITEM_SCROLL | ITEM_POTION => {
            out.extend_from_slice(
                format!(
                    "Spells: (Level {}) {}, {}, {}\r\n",
                    v[0],
                    mud_data::spells::skill_name(v[1]),
                    mud_data::spells::skill_name(v[2]),
                    mud_data::spells::skill_name(v[3])
                )
                .as_bytes(),
            );
        }
        ITEM_WAND | ITEM_STAFF => {
            out.extend_from_slice(
                format!(
                    "Spell: {} at level {}, {} (of {}) charges remaining\r\n",
                    mud_data::spells::skill_name(v[3]),
                    v[0],
                    v[2],
                    v[1]
                )
                .as_bytes(),
            );
        }
        ITEM_WEAPON => {
            let avg = ((v[2] + 1) as f64 / 2.0) * v[1] as f64;
            let msg = crate::fight::ATTACK_HIT_TEXT
                .get(v[3].clamp(0, 14) as usize)
                .map(|t| t.0)
                .unwrap_or(b"");
            out.extend_from_slice(
                format!("Todam: {}d{}, Avg Damage: {:.1}. Message type: ", v[1], v[2], avg)
                    .as_bytes(),
            );
            out.extend_from_slice(msg);
            out.extend_from_slice(b"\r\n");
        }
        ITEM_ARMOR => {
            out.extend_from_slice(format!("AC-apply: [{}]\r\n", v[0]).as_bytes());
        }
        ITEM_CONTAINER => {
            let bits = sprintbit(v[1] as i64, &CONTAINER_BITS);
            out.extend_from_slice(format!("Weight capacity: {}, Lock Type: ", v[0]).as_bytes());
            out.extend_from_slice(&bits);
            out.extend_from_slice(
                format!(", Key Num: {}, Corpse: {}\r\n", v[2], yesno(v[3] != 0)).as_bytes(),
            );
        }
        ITEM_DRINKCON | ITEM_FOUNTAIN => {
            let liq = sprinttype(v[2], &DRINKS);
            out.extend_from_slice(
                format!("Capacity: {}, Contains: {}, Poisoned: {}, Liquid: ", v[0], v[1], yesno(v[3] != 0))
                    .as_bytes(),
            );
            out.extend_from_slice(&liq);
            out.extend_from_slice(b"\r\n");
        }
        ITEM_NOTE => out.extend_from_slice(format!("Tongue: {}\r\n", v[0]).as_bytes()),
        ITEM_KEY => {}
        ITEM_FOOD => out.extend_from_slice(
            format!("Makes full: {}, Poisoned: {}\r\n", v[0], yesno(v[3] != 0)).as_bytes(),
        ),
        ITEM_MONEY => out.extend_from_slice(format!("Coins: {}\r\n", v[0]).as_bytes()),
        ITEM_FURNITURE => {
            out.extend_from_slice(
                format!("Can hold: [{}] Num. of People in: [{}]\r\n", v[0], v[1]).as_bytes(),
            );
            out.extend_from_slice(b"Holding : ");
            // OBJ_SAT_IN_BY heads a NEXT_SITTING chain; furniture here
            // holds a single occupant, which is the whole chain in
            // practice.
            if let Some(t) = g.obj(j).sat_in_by.filter(|&t| g.try_ch(t).is_some()) {
                out.extend_from_slice(g.ch(t).get_name());
                out.push(b' ');
            }
            out.extend_from_slice(b"\r\n");
        }
        _ => out.extend_from_slice(
            format!("Values 0-3: [{}] [{}] [{}] [{}]\r\n", v[0], v[1], v[2], v[3]).as_bytes(),
        ),
    }

    let contains = g.obj(j).contains.clone();
    if !contains.is_empty() {
        out.extend_from_slice(b"\r\nContents:");
        out.extend_from_slice(&grn);
        send_to_char(g, chid, &out);
        out = Vec::new();
        let mut column = 9usize;
        let mut found = 0;
        for (i, &j2) in contains.iter().enumerate() {
            if !g.try_obj_alive(j2) {
                continue;
            }
            let mut chunk: BStr = if found > 0 { b",".to_vec() } else { Vec::new() };
            chunk.push(b' ');
            chunk.extend_from_slice(crate::handler::obj_short(g, j2));
            found += 1;
            column += chunk.len();
        send_to_char(g, chid, &chunk);
            if column >= 62 {
                let more = contains[i + 1..].iter().any(|&n| g.try_obj_alive(n));
                send_to_char(g, chid, if more { b",\r\n" } else { b"\r\n" });
                found = 0;
                column = 0;
            }
        }
        send_to_char(g, chid, &nrm);
        if column != 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }

    out.extend_from_slice(b"Affections:");
    let mut found = 0;
    for i in 0..MAX_OBJ_AFFECT {
        let a = g.obj(j).affected[i];
        if a.modifier == 0 {
            continue;
        }
        let name = sprinttype(a.location, &APPLY_TYPES);
        if found > 0 {
            out.push(b',');
        }
        out.extend_from_slice(format!(" {:+} to ", a.modifier).as_bytes());
        out.extend_from_slice(&name);
        found += 1;
    }
    if found == 0 {
        out.extend_from_slice(b" None");
    }
    out.extend_from_slice(b"\r\n");
    send_to_char(g, chid, &out);

    do_sstat(g, chid, crate::dg::GoId::Obj(j));
}

fn yesno(b: bool) -> &'static str {
    if b {
        "YES"
    } else {
        "NO"
    }
}

fn proto_short(g: &Game, j: ObjId) -> Option<&[u8]> {
    let rnum = g.obj(j).item_number;
    if rnum == NOTHING {
        return None;
    }
    g.world.obj_protos[rnum as usize].short_description.as_deref()
}

fn proto_desc(g: &Game, j: ObjId) -> Option<BStr> {
    let rnum = g.obj(j).item_number;
    if rnum == NOTHING {
        return None;
    }
    g.world.obj_protos[rnum as usize].description.clone()
}

fn obj_exdescs(g: &Game, j: ObjId) -> Vec<BStr> {
    if let Some(list) = g.obj(j).ex_descriptions.as_ref() {
        return list.iter().map(|e| e.keyword.clone().unwrap_or_default()).collect();
    }
    let rnum = g.obj(j).item_number;
    if rnum == NOTHING {
        return Vec::new();
    }
    g.world.obj_protos[rnum as usize]
        .ex_descriptions
        .iter()
        .map(|e| e.keyword.clone().unwrap_or_default())
        .collect()
}

pub fn do_stat_character(g: &mut Game, chid: CharId, k: CharId) {
    let (cyn, grn, yel, nrm) = (
        cc(g, chid, C_NRM, KCYN).to_vec(),
        cc(g, chid, C_NRM, KGRN).to_vec(),
        cc(g, chid, C_NRM, KYEL).to_vec(),
        cc(g, chid, C_NRM, KNRM).to_vec(),
    );
    let is_npc = g.ch(k).is_npc();
    let is_mob = is_npc && g.ch(k).mob_rnum != NOBODY;

    let sex = sprinttype(g.ch(k).sex as i32, &GENDERS);
    let kind: &[u8] = if !is_npc {
        b"PC"
    } else if !is_mob {
        b"NPC"
    } else {
        b"MOB"
    };
    let idnum = if is_npc { crate::dg::char_script_id(g, k) } else { g.ch(k).idnum };
    let in_room = g.ch(k).in_room;
    let room_vnum = if in_room == NOWHERE {
        NOWHERE as i32
    } else {
        g.world.rooms[in_room as usize].vnum as i32
    };
    let loadroom = if is_npc {
        NOWHERE as i32
    } else {
        g.ch(k).ps().load_room as i32
    };

    let mut out = sex.clone();
    out.push(b' ');
    out.extend_from_slice(kind);
    out.extend_from_slice(b" '");
    out.extend_from_slice(g.ch(k).get_name());
    out.extend_from_slice(
        format!(
            "'  IDNum: [{:5}], In room [{:5}], Loadroom : [{:5}]\r\n",
            idnum, room_vnum, loadroom
        )
        .as_bytes(),
    );

    if is_mob {
        let rnum = g.ch(k).mob_rnum;
        let vnum = g.world.mob_protos[rnum as usize].vnum as i32;
        out.extend_from_slice(b"Keyword: ");
        out.extend_from_slice(g.ch(k).name.as_deref().unwrap_or(b""));
        out.extend_from_slice(format!(", VNum: [{:5}], RNum: [{:5}]\r\n", vnum, rnum).as_bytes());
        out.extend_from_slice(b"L-Des: ");
        match g.ch(k).long_descr.as_deref() {
            Some(d) if !d.is_empty() => out.extend_from_slice(d),
            _ => out.extend_from_slice(b"<None>\r\n"),
        }
    } else {
        out.extend_from_slice(b"Title: ");
        out.extend_from_slice(g.ch(k).title.as_deref().unwrap_or(b"<None>"));
        out.extend_from_slice(b"\r\n");
    }

    out.extend_from_slice(b"D-Des: ");
    match g.ch(k).description.as_deref() {
        Some(d) if !d.is_empty() => out.extend_from_slice(d),
        _ => out.extend_from_slice(b"<None>\r\n"),
    }

    let class = sprinttype(g.ch(k).class as i32, &crate::act::wizset::PC_CLASS_TYPES_STR);
    out.extend_from_slice(if is_npc { &b"Mobile"[..] } else { b"Class: " });
    if !is_npc {
        out.extend_from_slice(&class);
    }
    out.extend_from_slice(b", Lev: [");
    out.extend_from_slice(&yel);
    out.extend_from_slice(format!("{:2}", g.ch(k).level).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"], XP: [");
    out.extend_from_slice(&yel);
    out.extend_from_slice(format!("{:7}", g.ch(k).points.exp).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(format!("], Align: [{:4}]\r\n", g.ch(k).alignment).as_bytes());

    if !is_npc {
        let tz = g.tz_offset_secs;
        let buf1 = crate::act::wizard::strftime_date(g.ch(k).time.birth, tz);
        let buf2 = crate::act::wizard::strftime_date(g.ch(k).time.logon, tz);
        out.extend_from_slice(
            format!("Created: [{}], Last Logon: [{}]\r\n", buf1, buf2).as_bytes(),
        );
        let played = g.ch(k).time.played;
        let age = crate::gametime::age(g.ch(k).time.birth, g.now).year;
        // The wisdom and intelligence tables are only the opening figures;
        // the class bounds both. Reporting the raw entries told an immortal a
        // warrior learned 35% a session when he learns 12%, so these are the
        // numbers play actually uses.
        let class = g.ch(k).class as i32;
        let per_level = crate::limits::practices_per_level(class, g.ch(k).aff_abils.wis as i32);
        let per_prac = crate::limits::practice_gain_percent(class, g.ch(k).aff_abils.intel as i32);
        out.extend_from_slice(
            format!(
                "Played: [{}h {}m], Age: [{}], Prac: [{}] (+{}/lvl, {}%/prac)",
                played / 3600,
                (played % 3600) / 60,
                age,
                g.ch(k).ps().practices,
                per_level,
                per_prac
            )
            .as_bytes(),
        );
        if g.ch(k).level >= LVL_BUILDER {
            let olc = g.ch(k).ps().olc_zone;
            let label: BStr = match olc {
                AEDIT_PERMISSION => b"Aedit".to_vec(),
                HEDIT_PERMISSION => b"Hedit".to_vec(),
                ALL_PERMISSION => b"All".to_vec(),
                x if x == NOWHERE as i32 => b"OFF".to_vec(),
                x => x.to_string().into_bytes(),
            };
            out.extend_from_slice(b", OLC[");
            out.extend_from_slice(&cyn);
            out.extend_from_slice(&label);
            out.extend_from_slice(&nrm);
            out.push(b']');
        }
        out.extend_from_slice(b"\r\n");
    }

    let a = g.ch(k).aff_abils;
    let stat = |v: i8, extra: Option<i8>| -> BStr {
        match extra {
            Some(e) => format!("{}/{}", v, e).into_bytes(),
            None => v.to_string().into_bytes(),
        }
    };
    let push_stat = |out: &mut BStr, label: &[u8], val: BStr| {
        out.extend_from_slice(label);
        out.extend_from_slice(b": [");
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&val);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"]  ");
    };
    push_stat(&mut out, b"Str", stat(a.str_, Some(a.str_add)));
    push_stat(&mut out, b"Int", stat(a.intel, None));
    push_stat(&mut out, b"Wis", stat(a.wis, None));
    push_stat(&mut out, b"Dex", stat(a.dex, None));
    push_stat(&mut out, b"Con", stat(a.con, None));
    out.extend_from_slice(b"Cha: [");
    out.extend_from_slice(&cyn);
    out.extend_from_slice(a.cha.to_string().as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"]\r\n");

    let (hg, mg, vg) = (
        crate::limits::hit_gain(g, k),
        crate::limits::mana_gain(g, k),
        crate::limits::move_gain(g, k),
    );
    let p = g.ch(k).points;
    out.extend_from_slice(b"Hit p.:[");
    out.extend_from_slice(&grn);
    out.extend_from_slice(format!("{}/{}+{}", p.hit, p.max_hit, hg).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"]  Mana p.:[");
    out.extend_from_slice(&grn);
    out.extend_from_slice(format!("{}/{}+{}", p.mana, p.max_mana, mg).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"]  Move p.:[");
    out.extend_from_slice(&grn);
    out.extend_from_slice(format!("{}/{}+{}", p.mov, p.max_move, vg).as_bytes());
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"]\r\n");

    out.extend_from_slice(
        format!(
            "Gold: [{:9}], Bank: [{:9}] (Total: {}), ",
            p.gold,
            p.bank_gold,
            p.gold + p.bank_gold
        )
        .as_bytes(),
    );
    if is_npc {
        // Only PCs get a terminator from the Screen row that follows, so
        // an NPC's AC row has to end itself or it runs on.
        out.extend_from_slice(b"\r\n");
    } else {
        let (w, l) = (g.ch(k).ps().screen_width, g.ch(k).ps().page_length);
        out.extend_from_slice(b"Screen ");
        out.extend_from_slice(&cyn);
        out.push(b'[');
        out.extend_from_slice(&yel);
        out.extend_from_slice(w.to_string().as_bytes());
        out.extend_from_slice(&nrm);
        out.push(b'x');
        out.extend_from_slice(&yel);
        out.extend_from_slice(l.to_string().as_bytes());
        out.extend_from_slice(&cyn);
        out.push(b']');
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
    }

    let dex = g.ch(k).aff_abils.dex.clamp(0, 25) as usize;
    let sv = g.ch(k).apply_saving_throw;
    out.extend_from_slice(
        format!(
            "AC: [{}{:+}/10], Hitroll: [{:2}], Damroll: [{:2}], Saving throws: [{}/{}/{}/{}/{}]\r\n",
            p.armor,
            tables::DEX_APP[dex].2,
            p.hitroll,
            p.damroll,
            sv[0],
            sv[1],
            sv[2],
            sv[3],
            sv[4]
        )
        .as_bytes(),
    );

    let pos = sprinttype(g.ch(k).position as i32, &POSITION_TYPES);
    out.extend_from_slice(b"Pos: ");
    out.extend_from_slice(&pos);
    out.extend_from_slice(b", Fighting: ");
    match g.ch(k).fighting.filter(|&c| g.try_ch(c).is_some()) {
        Some(c) => out.extend_from_slice(g.ch(c).get_name()),
        None => out.extend_from_slice(b"Nobody"),
    }
    if is_npc {
        let at = g.ch(k).mob_specials.attack_type.clamp(0, 14) as usize;
        out.extend_from_slice(b", Attack type: ");
        out.extend_from_slice(crate::fight::ATTACK_HIT_TEXT[at].0);
    }
    if let Some(di) = g.ch(k).desc {
        if let Some(d) = g.descriptors.get(di) {
            let st = sprinttype(d.state as i32, &CONNECTED_TYPES);
            out.extend_from_slice(b", Connected: ");
            out.extend_from_slice(&st);
        }
    }

    if is_npc {
        let dp = sprinttype(g.ch(k).mob_specials.default_pos as i32, &POSITION_TYPES);
        out.extend_from_slice(b", Default position: ");
        out.extend_from_slice(&dp);
        out.extend_from_slice(b"\r\n");
        let mut buf = Vec::new();
        sprintbitarray(&g.ch(k).act.0, &tables::ACTION_BITS, &mut buf);
        out.extend_from_slice(b"NPC flags: ");
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&buf);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
    } else {
        out.extend_from_slice(format!(", Idle Timer (in tics) [{}]\r\n", g.ch(k).timer).as_bytes());
        let mut buf = Vec::new();
        sprintbitarray(&g.ch(k).act.0, &tables::PLAYER_BITS, &mut buf);
        out.extend_from_slice(b"PLR: ");
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&buf);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
        let mut buf = Vec::new();
        sprintbitarray(&g.ch(k).ps().pref.0, &tables::PREFERENCE_BITS, &mut buf);
        out.extend_from_slice(b"PRF: ");
        out.extend_from_slice(&grn);
        out.extend_from_slice(&buf);
        out.extend_from_slice(&nrm);
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(
            format!(
                "Quest Points: [{:9}] Quests Completed: [{:5}]\r\n",
                g.ch(k).ps().questpoints,
                g.ch(k).ps().num_completed_quests
            )
            .as_bytes(),
        );
        if g.ch(k).ps().current_quest != NOTHING {
            out.extend_from_slice(
                format!(
                    "Current Quest: [{:5}] Time Left: [{:5}]\r\n",
                    g.ch(k).ps().current_quest,
                    g.ch(k).ps().quest_time
                )
                .as_bytes(),
            );
        }
    }

    if is_mob {
        let rnum = g.ch(k).mob_rnum as usize;
        out.extend_from_slice(b"Mob Spec-Proc: ");
        match g.mob_specs.get(rnum).copied().flatten() {
            Some(s) => out.extend_from_slice(mob_spec_name(s)),
            None => out.extend_from_slice(b"None"),
        }
        out.extend_from_slice(
            format!(
                ", NPC Bare Hand Dam: {}d{}\r\n",
                g.ch(k).mob_specials.damnodice,
                g.ch(k).mob_specials.damsizedice
            )
            .as_bytes(),
        );
    }

    let inv = g.ch(k).carrying.len();
    let eq = (0..NUM_WEARS).filter(|&i| g.ch(k).equipment[i].is_some()).count();
    out.extend_from_slice(
        format!(
            "Carried: weight: {}, items: {}; Items in: inventory: {}, eq: {}\r\n",
            g.ch(k).carry_weight,
            g.ch(k).carry_items,
            inv,
            eq
        )
        .as_bytes(),
    );

    if !is_npc {
        out.extend_from_slice(
            format!(
                "Hunger: {}, Thirst: {}, Drunk: {}\r\n",
                g.ch(k).ps().conditions[crate::ch::HUNGER],
                g.ch(k).ps().conditions[crate::ch::THIRST],
                g.ch(k).ps().conditions[crate::ch::DRUNK]
            )
            .as_bytes(),
        );
    }

    let mut line = b"Master is: ".to_vec();
    match g.ch(k).master.filter(|&c| g.try_ch(c).is_some()) {
        Some(m) => line.extend_from_slice(g.ch(m).get_name()),
        None => line.extend_from_slice(b"<none>"),
    }
    line.extend_from_slice(b", Followers are:");
    out.extend_from_slice(&line);
    send_to_char(g, chid, &out);
    out = Vec::new();
    let mut column = line.len();
    let followers = g.ch(k).followers.clone();
    if followers.is_empty() {
        send_to_char(g, chid, b" <none>\r\n");
    } else {
        let mut found = 0;
        for (i, &fol) in followers.iter().enumerate() {
            if g.try_ch(fol).is_none() {
                continue;
            }
            let mut chunk: BStr = if found > 0 { b",".to_vec() } else { Vec::new() };
            chunk.push(b' ');
            chunk.extend_from_slice(&pers(g, chid, fol));
            found += 1;
            column += chunk.len();
        send_to_char(g, chid, &chunk);
            if column >= 62 {
                let more = followers[i + 1..].iter().any(|&n| g.try_ch(n).is_some());
                send_to_char(g, chid, if more { b",\r\n" } else { b"\r\n" });
                found = 0;
                column = 0;
            }
        }
        if column != 0 {
            send_to_char(g, chid, b"\r\n");
        }
    }

    let mut buf = Vec::new();
    sprintbitarray(&g.ch(k).affected_by.0, &AFFECTED_BITS, &mut buf);
    out.extend_from_slice(b"AFF: ");
    out.extend_from_slice(&yel);
    out.extend_from_slice(&buf);
    out.extend_from_slice(&nrm);
    out.extend_from_slice(b"\r\n");

    for aff in g.ch(k).affected.clone() {
        out.extend_from_slice(format!("SPL: ({:3}hr) ", aff.duration + 1).as_bytes());
        out.extend_from_slice(&cyn);
        out.extend_from_slice(&pad_right(
            mud_data::spells::skill_name(aff.spell as i32).as_bytes(),
            21,
        ));
        out.extend_from_slice(&nrm);
        out.push(b' ');
        if aff.modifier != 0 {
            out.extend_from_slice(
                format!(
                    "{:+} to {}",
                    aff.modifier,
                    APPLY_TYPES.get(aff.location as usize).copied().unwrap_or("")
                )
                .as_bytes(),
            );
        }
        if !aff.bitvector.is_empty() {
            if aff.modifier != 0 {
                out.extend_from_slice(b", ");
            }
            for i in 1..AFFECTED_BITS.len() {
                if aff.bitvector.is_set(i) {
                    out.extend_from_slice(format!("sets {}, ", AFFECTED_BITS[i]).as_bytes());
                }
            }
        }
        out.extend_from_slice(b"\r\n");
    }

    if !is_npc && g.ch(k).level >= LVL_IMMORT {
        let qyel = cc(g, chid, crate::comm::C_SPR, KYEL).to_vec();
        let qcyn = cc(g, chid, crate::comm::C_SPR, KCYN).to_vec();
        let qnrm = cc(g, chid, crate::comm::C_SPR, KNRM).to_vec();
        for (label, msg, dflt) in [
            (&b"POOFIN:  "[..], g.ch(k).ps().poofin.clone(), &b"appears with an ear-splitting bang."[..]),
            (&b"POOFOUT: "[..], g.ch(k).ps().poofout.clone(), &b"disappears in a puff of smoke."[..]),
        ] {
            out.extend_from_slice(&qyel);
            out.extend_from_slice(label);
            out.extend_from_slice(&qcyn);
            out.extend_from_slice(g.ch(k).get_name());
            out.push(b' ');
            out.extend_from_slice(msg.as_deref().unwrap_or(dflt));
            out.extend_from_slice(&qnrm);
            out.extend_from_slice(b"\r\n");
        }
    }
    send_to_char(g, chid, &out);

    do_sstat(g, chid, crate::dg::GoId::Char(k));

    let mem = g.ch(k).script_mem.clone();
    if !mem.is_empty() {
        send_to_char(g, chid, b"Script memory:\r\n  Remember             Command\r\n");
        for m in mem {
            let mc = crate::dg::find_char(g, m.id);
            let mut out = b"  ".to_vec();
            match mc {
                None => out.extend_from_slice(b"** Corrupted!\r\n"),
                Some(c) => {
                    out.extend_from_slice(&crate::act::pad_right_trunc(g.ch(c).get_name(), 20));
                    match m.cmd.as_deref() {
                        Some(cmd) => out.extend_from_slice(cmd),
                        None => out.extend_from_slice(b" <default>"),
                    }
                    out.extend_from_slice(b"\r\n");
                }
            }
            send_to_char(g, chid, &out);
        }
    }

    if !is_npc {
        let vars = g
            .ch(k)
            .script
            .as_ref()
            .map(|s| s.global_vars.clone())
            .unwrap_or_default();
        if !vars.is_empty() {
            send_to_char(g, chid, b"Global Variables:\r\n");
            for tv in vars {
                let mut out = b"    ".to_vec();
                out.extend_from_slice(&crate::act::pad_left(&tv.name, 10));
                out.extend_from_slice(b":  ");
                if tv.value.first() == Some(&crate::dg::UID_CHAR) {
                    out.extend_from_slice(b"[UID]: ");
                    let uname = crate::dg::commands::find_uid_name(g, &tv.value);
                    out.extend_from_slice(&uname);
                } else {
                    out.extend_from_slice(&tv.value);
                }
                out.extend_from_slice(b"\r\n");
                send_to_char(g, chid, &out);
            }
        }
    }
}

pub fn do_stat(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf1, buf2) = half_chop(argument);
    if buf1.is_empty() {
        send_to_char(g, chid, b"Stats on who or what or where?\r\n");
        return;
    }
    if is_abbrev(&buf1, b"room") {
        let room = if buf2.is_empty() {
            g.ch(chid).in_room
        } else {
            match g.real_room(atoi(&buf2)) {
                Some(r) => r,
                None => {
                    send_to_char(g, chid, b"That is not a valid room.\r\n");
                    return;
                }
            }
        };
        do_stat_room(g, chid, room);
    } else if is_abbrev(&buf1, b"mob") {
        if buf2.is_empty() {
            send_to_char(g, chid, b"Stats on which mobile?\r\n");
        } else if let Some(v) = crate::handler::get_char_world_vis(g, chid, &buf2, None) {
            do_stat_character(g, chid, v);
        } else {
            send_to_char(g, chid, b"No such mobile around.\r\n");
        }
    } else if is_abbrev(&buf1, b"player") {
        if buf2.is_empty() {
            send_to_char(g, chid, b"Stats on which player?\r\n");
        } else if let Some(v) = crate::handler::get_player_vis(g, chid, &buf2, false) {
            do_stat_character(g, chid, v);
        } else {
            send_to_char(g, chid, b"No such player around.\r\n");
        }
    } else if is_abbrev(&buf1, b"file") {
        if buf2.is_empty() {
            send_to_char(g, chid, b"Stats on which player?\r\n");
        } else if let Some(v) = crate::handler::get_player_vis(g, chid, &buf2, false) {
            do_stat_character(g, chid, v);
        } else if let Some(v) = crate::players_glue::load_char_offline(g, &buf2) {
            crate::handler::char_to_room(g, v, 0);
            if g.ch(v).level > g.ch(chid).level {
                send_to_char(g, chid, b"Sorry, you can't do that.\r\n");
            } else {
                do_stat_character(g, chid, v);
            }
            crate::handler::extract_char_final(g, v);
        } else {
            send_to_char(g, chid, b"There is no such player.\r\n");
        }
    } else if is_abbrev(&buf1, b"object") {
        if buf2.is_empty() {
            send_to_char(g, chid, b"Stats on which object?\r\n");
        } else {
            let mut n = 1;
            if let Some(o) = crate::handler::get_obj_vis_counted(g, chid, &buf2, &mut n) {
                do_stat_object(g, chid, o);
            } else {
                send_to_char(g, chid, b"No such object around.\r\n");
            }
        }
    } else if is_abbrev(&buf1, b"zone") {
        let vnum = if buf2.is_empty() {
            g.world.zones[g.world.rooms[g.ch(chid).in_room as usize].zone as usize].number as i32
        } else {
            atoi(&buf2)
        };
        crate::act::wizshow::print_zone(g, chid, vnum);
    } else {
        let (number, name) = get_number(&buf1);
        let mut n = number;
        if let Some(pos) = crate::handler::get_obj_pos_in_equip_vis_counted(g, chid, &name, &mut n) {
            let o = g.ch(chid).equipment[pos].unwrap();
            do_stat_object(g, chid, o);
            return;
        }
        let carrying = g.ch(chid).carrying.clone();
        if let Some(o) =
            crate::handler::get_obj_in_list_vis_counted(g, chid, &name, &mut n, &carrying)
        {
            do_stat_object(g, chid, o);
            return;
        }
        if let Some(v) = crate::handler::get_char_room_vis_counted(g, chid, &name, &mut n) {
            do_stat_character(g, chid, v);
            return;
        }
        let room = g.ch(chid).in_room;
        let contents = g.rooms[room as usize].contents.clone();
        if let Some(o) =
            crate::handler::get_obj_in_list_vis_counted(g, chid, &name, &mut n, &contents)
        {
            do_stat_object(g, chid, o);
            return;
        }
        if let Some(v) = crate::handler::get_char_world_vis(g, chid, &name, Some(n)) {
            do_stat_character(g, chid, v);
            return;
        }
        if let Some(o) = crate::handler::get_obj_vis_counted(g, chid, &name, &mut n) {
            do_stat_object(g, chid, o);
            return;
        }
        send_to_char(g, chid, b"Nothing around by that name.\r\n");
    }
}

pub fn do_vstat(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    let (buf, buf2, _) = crate::interpreter::two_arguments(argument);
    if buf.is_empty() || buf2.is_empty() || !buf2[0].is_ascii_digit() {
        send_to_char(g, chid, b"Usage: vstat { o | m | r | t | s | z } <number>\r\n");
        return;
    }
    if !is_number(&buf2) {
        send_to_char(g, chid, b"That's not a valid number.\r\n");
        return;
    }
    let num = atoi(&buf2);
    match buf[0].to_ascii_lowercase() {
        b'm' => {
            let Some(r_num) = g.world.real_mobile(num as Idx) else {
                send_to_char(g, chid, b"There is no monster with that number.\r\n");
                return;
            };
            let Some(mob) = crate::db::read_mobile(g, r_num) else { return };
            crate::handler::char_to_room(g, mob, 0);
            do_stat_character(g, chid, mob);
            crate::handler::extract_char(g, mob);
        }
        b'o' => {
            let Some(r_num) = g.world.real_object(num as Idx) else {
                send_to_char(g, chid, b"There is no object with that number.\r\n");
                return;
            };
            let Some(obj) = crate::db::read_object(g, r_num) else { return };
            do_stat_object(g, chid, obj);
            crate::handler::extract_obj(g, obj);
        }
        b'r' => {
            let arg = format!("room {}", num).into_bytes();
            do_stat(g, chid, &arg, 0, 0);
        }
        b'z' => {
            let arg = format!("zone {}", num).into_bytes();
            do_stat(g, chid, &arg, 0, 0);
        }
        b't' => {
            let arg = num.to_string().into_bytes();
            crate::dg::commands::do_tstat(g, chid, &arg, 0, 0);
        }
        b's' => {
            let arg = format!("shops {}", num).into_bytes();
            crate::act::wizshow::do_show(g, chid, &arg, 0, 0);
        }
        _ => send_to_char(g, chid, b"Syntax: vstat { r | m | o | z | t | s } <number>\r\n"),
    }
}

/// The lone `one_argument` re-export the module needs for `stat` parsing.
#[allow(unused)]
fn _unused(a: &[u8]) -> BStr {
    one_argument(a).0
}
