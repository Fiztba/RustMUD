//! .qst parser: the record loop and parse_quest.
//!
//! Five tilde strings (name/desc/info/done/quit), then three numeric lines
//! whose field counts are enforced exactly (7, 7, 3 -- anything else is
//! fatal), then lines are consumed until one starts with 'S'. The u16
//! Vnum fields are stored as u16, so -1 becomes 65535 for target/prereq/
//! prev/next/obj_reward, the questmaster keeps its vnum only when
//! real_mobile finds it (else NOBODY), and vnum-typed values truncate.

use super::trg::{scan_after_hash, scan_int, scan_word};
use crate::lex::{Reader, asciiflag_conv};
use crate::model::{Quest, World};

/// The .qst record loop — the same shape as the .trg loop.
pub fn parse_file(world: &mut World, data: &[u8], filename: &str) -> Result<(), String> {
    let mut r = Reader::new(data);
    let mut nr: i32 = -1;
    loop {
        let line = match r.get_line() {
            Some(l) => l,
            None => {
                return Err(if nr == -1 {
                    format!("qst file {filename} is empty!")
                } else {
                    format!(
                        "Format error in {filename} after qst #{nr}: \
                         expecting a new qst, but file ended! \
                         (maybe the file is not terminated with '$'?)"
                    )
                });
            }
        };
        if line.first() == Some(&b'$') {
            return Ok(());
        }
        if line.first() == Some(&b'#') {
            let last = nr;
            nr = match scan_after_hash(&line) {
                Some(v) => v,
                None => return Err(format!("Format error after qst #{last}")),
            };
            if nr >= 99999 {
                return Ok(());
            }
            parse_quest(world, &mut r, nr)?;
        } else {
            return Err(format!(
                "Format error in qst file {filename} near qst #{nr}: \
                 offending line: '{}'",
                String::from_utf8_lossy(&line)
            ));
        }
    }
}

fn parse_quest(world: &mut World, r: &mut Reader, nr: i32) -> Result<(), String> {
    let ctx = format!("quest vnum {nr}");

    let name = r.fread_string(&ctx)?;
    let desc = r.fread_string(&ctx)?;
    let info = r.fread_string(&ctx)?;
    let done = r.fread_string(&ctx)?;
    let quit = r.fread_string(&ctx)?;

    // All seven fields must be present.
    let line = r
        .get_line()
        .ok_or_else(|| format!("Format error in numeric line (expected 7, got EOF), {ctx}"))?;
    let mut i = 0;
    let t0 = scan_int(&line, &mut i);
    let t1 = t0.and_then(|_| scan_int(&line, &mut i));
    let f1 = t1.and_then(|_| scan_word(&line, &mut i));
    let t2 = f1.as_ref().and_then(|_| scan_int(&line, &mut i));
    let t3 = t2.and_then(|_| scan_int(&line, &mut i));
    let t4 = t3.and_then(|_| scan_int(&line, &mut i));
    let t5 = t4.and_then(|_| scan_int(&line, &mut i));
    let (Some(t0), Some(t1), Some(f1), Some(t2), Some(t3), Some(t4), Some(t5)) =
        (t0, t1, f1, t2, t3, t4, t5)
    else {
        return Err(format!(
            "Format error in numeric line (expected 7), {}: '{}'",
            ctx,
            String::from_utf8_lossy(&line)
        ));
    };

    let type_ = t0;
    // qm = (real_mobile(t[1]) == NOBODY) ? NOBODY: t[1] — both the lookup
    // argument and the stored mob_vnum are u16-truncated.
    let qm_vnum = if world.mob_map.contains_key(&(t1 as u16)) {
        (t1 as u16) as i32
    } else {
        65535
    };
    let flags = asciiflag_conv(&f1);
    // target is a plain int: only -1 becomes NOTHING, other values kept.
    let target = if t2 == -1 { 65535 } else { t2 };
    // prev/next (qst_vnum) and prereq (obj_vnum) are u16 stores.
    let prev_quest = if t3 == -1 { 65535 } else { (t3 as u16) as i32 };
    let next_quest = if t4 == -1 { 65535 } else { (t4 as u16) as i32 };
    let prereq = if t5 == -1 { 65535 } else { (t5 as u16) as i32 };

    // Seven raw ints into value[0..6]: points, penalty, min level, max
    // level, time limit, return-mob (obj_in), quantity (obj_out).
    let line = r
        .get_line()
        .ok_or_else(|| format!("Format error in numeric line (expected 7, got EOF), {ctx}"))?;
    let mut i = 0;
    let mut v = [0i32; 7];
    for (j, slot) in v.iter_mut().enumerate() {
        *slot = scan_int(&line, &mut i).ok_or_else(|| {
            format!(
                "Format error in numeric line (expected 7, got {}), {}: '{}'",
                j,
                ctx,
                String::from_utf8_lossy(&line)
            )
        })?;
    }

    let line = r.get_line().ok_or_else(|| {
        format!("Format error in numeric (rewards) line (expected 3, got EOF), {ctx}")
    })?;
    let mut i = 0;
    let g0 = scan_int(&line, &mut i);
    let g1 = g0.and_then(|_| scan_int(&line, &mut i));
    let g2 = g1.and_then(|_| scan_int(&line, &mut i));
    let (Some(gold_reward), Some(exp_reward), Some(obj)) = (g0, g1, g2) else {
        return Err(format!(
            "Format error in numeric (rewards) line (expected 3), {}: '{}'",
            ctx,
            String::from_utf8_lossy(&line)
        ));
    };
    // obj_reward is an obj_vnum (u16): -1 becomes NOTHING = 65535, which
    // the writer prints back raw.
    let obj_reward = if obj == -1 { 65535 } else { (obj as u16) as i32 };

    // Consume lines until one starts with 'S'; everything else (even '$')
    // is silently skipped. EOF here is fatal.
    loop {
        let line = r.get_line().ok_or_else(|| format!("Format error in {ctx}"))?;
        if line.first() == Some(&b'S') {
            break;
        }
    }

    world.quests.push(Quest {
        vnum: nr as u16,
        qm_vnum,
        flags,
        type_,
        name,
        desc,
        info,
        done,
        quit,
        value: v[0],
        penalty: v[1],
        min_level: v[2],
        max_level: v[3],
        target,
        prereq,
        obj_in: v[5],
        obj_out: v[6],
        time: v[4],
        gold_reward,
        exp_reward,
        obj_reward,
        prev_quest,
        next_quest,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &[u8] = b"#100\nKill the Mice!~\nmice~\n   Help me.\n~\nWell done!\n~\nYou quit.\n~\n3 179 0 194 -1 -1 -1\n0 0 1 34 60 -1 3\n10 0 65535\nS\n$~\n";

    #[test]
    fn stock_record_with_known_qm() {
        let mut w = World::default();
        w.mob_map.insert(179, 0);
        parse_file(&mut w, STOCK, "1.qst").expect("parse");
        assert_eq!(w.quests.len(), 1);
        let q = &w.quests[0];
        assert_eq!(q.vnum, 100);
        assert_eq!(q.name.as_deref(), Some(&b"Kill the Mice!"[..]));
        assert_eq!(q.desc.as_deref(), Some(&b"mice"[..]));
        assert_eq!(q.info.as_deref(), Some(&b"   Help me.\r\n"[..]));
        assert_eq!(q.type_, 3);
        assert_eq!(q.qm_vnum, 179);
        assert_eq!(q.flags, 0);
        assert_eq!(q.target, 194);
        assert_eq!(q.prev_quest, 65535);
        assert_eq!(q.next_quest, 65535);
        assert_eq!(q.prereq, 65535);
        assert_eq!(
            (q.value, q.penalty, q.min_level, q.max_level, q.time, q.obj_in, q.obj_out),
            (0, 0, 1, 34, 60, -1, 3)
        );
        assert_eq!((q.gold_reward, q.exp_reward, q.obj_reward), (10, 0, 65535));
    }

    #[test]
    fn unknown_questmaster_becomes_nobody() {
        let mut w = World::default();
        parse_file(&mut w, STOCK, "1.qst").expect("parse");
        assert_eq!(w.quests[0].qm_vnum, 65535);
    }

    #[test]
    fn minus_one_reward_stores_nothing() {
        let mut w = World::default();
        let data = b"#5\nN~\nd~\ni~\no~\nq~\n0 1 ab -1 2 3 4\n1 2 3 4 5 6 7\n8 9 -1\nS\n$~\n";
        parse_file(&mut w, data, "t.qst").expect("parse");
        let q = &w.quests[0];
        assert_eq!(q.flags, 0b11); // 'ab'
        assert_eq!(q.target, 65535); // -1 => NOTHING
        assert_eq!((q.prev_quest, q.next_quest, q.prereq), (2, 3, 4));
        assert_eq!(q.obj_reward, 65535);
        assert_eq!((q.value, q.penalty, q.min_level, q.max_level), (1, 2, 3, 4));
        assert_eq!((q.time, q.obj_in, q.obj_out), (5, 6, 7));
    }

    #[test]
    fn junk_before_s_is_skipped_and_wrong_counts_fatal() {
        let mut w = World::default();
        let ok = b"#1\nN~\nd~\ni~\no~\nq~\n0 1 0 2 3 4 5\n1 2 3 4 5 6 7\n1 2 3\nX\nY 9\nS extra\n$~\n";
        parse_file(&mut w, ok, "t.qst").expect("junk lines before S are legal");
        assert_eq!(w.quests.len(), 1);

        let short = b"#1\nN~\nd~\ni~\no~\nq~\n0 1 0 2 3 4\n1 2 3 4 5 6 7\n1 2 3\nS\n$~\n";
        assert!(parse_file(&mut World::default(), short, "t.qst").is_err());

        let bad_rewards = b"#1\nN~\nd~\ni~\no~\nq~\n0 1 0 2 3 4 5\n1 2 3 4 5 6 7\n1 2\nS\n$~\n";
        assert!(parse_file(&mut World::default(), bad_rewards, "t.qst").is_err());
    }

    #[test]
    fn empty_strings_are_none_and_empty_file_variants() {
        let mut w = World::default();
        let data = b"#2\n~\n~\n~\n~\n~\n0 1 0 -1 -1 -1 -1\n0 0 0 0 0 0 0\n0 0 0\nS\n$~\n";
        parse_file(&mut w, data, "t.qst").expect("parse");
        let q = &w.quests[0];
        assert_eq!(q.name, None);
        assert_eq!(q.quit, None);
        assert_eq!(q.obj_reward, 0);

        // "$~" alone parses to zero quests; a zero-byte file is fatal.
        assert!(parse_file(&mut World::default(), b"$~\r\n", "t.qst").is_ok());
        assert!(parse_file(&mut World::default(), b"", "t.qst").is_err());
    }
}
