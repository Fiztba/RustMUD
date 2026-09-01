//! ASCII player files: save_char/load_char, the player index, and
//! get_filename bucketing, with the standard pfile defaults pre-applied.
//!
//! Study spec: docs/study/04-player-persistence.md (§1, §2, §3, §4, §13).
//!
//! This is pure file-format code over a neutral DTO ([`PlayerFile`]); the
//! game layer maps it to its runtime Char (and owns runtime-only behavior:
//! affect_total, immortal skill/condition overrides, the script-players
//! trigger attachment, and the alias/affect list-head insertion that
//! reverses order on each save/load cycle — the DTO preserves file order so
//! that save -> load -> save is byte-identical).
//!
//! Three behaviors are pinned here, all reachable only from a pfile the
//! writer never produces: a Skil/Affs/Qest/Alis/Vars block that hits EOF
//! before its terminator stops rather than looping; the index reader is
//! single-pass and stops at `~`, so trailing lines cannot confuse it; and
//! numeric fields are kept at DTO width (i32/i64) rather than narrowed to
//! the on-disk field widths (byte/ush_int).

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use mud_data::types::MAX_NAME_LENGTH;

use crate::lex::{Reader, asciiflag_conv, atol};
use crate::write::sprintascii;

/// conditions and skills are only saved below this level.
const LVL_IMMORT: i32 = 31;
/// save_char's tmp_aff array bounds the affect list.
const MAX_AFFECT: usize = 32;
/// bound for the legacy 5-token affect bit number.
const NUM_AFF_FLAGS: i32 = 23;
/// The NOTHING/NOWHERE sentinel, as formatted through %d.
const NOTHING: i32 = 65535;

/// Everything the ASCII pfile can carry.
#[derive(Debug, Clone)]
pub struct PlayerFile {
    pub name: Option<Vec<u8>>,        // Name:
    pub passwd: Vec<u8>,              // Pass: (≤30 bytes; C array is never NULL, so always written)
    pub title: Option<Vec<u8>>,       // Titl:
    pub description: Option<Vec<u8>>, // Desc: (multi-line, \r\n per line, @→\t applied on LOAD)
    pub poofin: Option<Vec<u8>>,
    pub poofout: Option<Vec<u8>>,
    pub sex: i32,
    pub class: i32,
    pub level: i32,
    pub idnum: i64,
    pub birth: i64,
    pub played: i32,
    pub last_logon: i64,
    pub last_motd: i64,
    pub last_news: i64,
    pub host: Option<Vec<u8>>,
    pub height: i32,
    pub weight: i32,
    pub alignment: i32,
    pub plr_flags: [u32; 4],
    pub aff_flags: [u32; 4],
    pub prf_flags: [u32; 4],
    pub saving_throws: [i32; 5], // Thr1..Thr5
    pub wimpy: i32,
    pub freeze_level: i32,
    pub invis_level: i32,
    pub load_room: i32, // Room: (default 0, NOT NOWHERE — quirk §13.6)
    pub bad_pws: i32,
    pub practices: i32,
    pub hunger: i32, // Hung:/Thir:/Drnk: (C GET_COND order: DRUNK=0, HUNGER=1, THIRST=2)
    pub thirst: i32,
    pub drunk: i32,
    pub hit: i32,
    pub max_hit: i32,
    pub mana: i32,
    pub max_mana: i32,
    pub mov: i32,
    pub max_move: i32,
    pub str_: i32,
    pub str_add: i32,
    pub intel: i32,
    pub wis: i32,
    pub dex: i32,
    pub con: i32,
    pub cha: i32,
    pub ac: i32,
    pub gold: i32,
    pub bank: i32,
    pub exp: i32,
    pub hitroll: i32,
    pub damroll: i32,
    pub olc_zone: i32,
    pub page_length: i32,
    pub screen_width: i32,
    pub questpoints: i32,
    pub quest_counter: i32,
    pub current_quest: i32,
    pub completed_quests: Vec<u16>, // Qest:
    pub triggers: Vec<u16>,         // Trig: lines
    pub skills: Vec<(i32, i32)>,    // Skil: pairs
    pub affects: Vec<PfAffect>,     // Affs:
    pub aliases: Vec<PfAlias>,      // Alis:
    pub vars: Vec<PfVar>,           // Vars:
}

/// One Affs: row (load_affects).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PfAffect {
    pub spell: i32,
    pub duration: i32,
    pub modifier: i32,
    pub location: i32,
    pub bitvector: [u32; 4],
}

/// One alias. The replacement KEEPS its leading space — an in-memory
/// invariant the writer relies on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PfAlias {
    pub alias: Vec<u8>,
    pub replacement: Vec<u8>,
    pub type_: i32,
}

/// One DG global variable. The reader lowercases `name` on load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PfVar {
    pub name: Vec<u8>,
    pub context: i64,
    pub value: Vec<u8>,
}

impl Default for PlayerFile {
    /// everything 0/absent except PFDEF_OLC = NOWHERE
    /// (65535), PFDEF_PAGELENGTH = 22, PFDEF_SCREENWIDTH = 80 and
    /// PFDEF_CURRQUEST = NOTHING (65535). PFDEF_LOADROOM is 0 (§13.6).
    fn default() -> Self {
        PlayerFile {
            name: None,
            passwd: Vec::new(),
            title: None,
            description: None,
            poofin: None,
            poofout: None,
            sex: 0,
            class: 0,
            level: 0,
            idnum: 0,
            birth: 0,
            played: 0,
            last_logon: 0,
            last_motd: 0,
            last_news: 0,
            host: None,
            height: 0,
            weight: 0,
            alignment: 0,
            plr_flags: [0; 4],
            aff_flags: [0; 4],
            prf_flags: [0; 4],
            saving_throws: [0; 5],
            wimpy: 0,
            freeze_level: 0,
            invis_level: 0,
            load_room: 0,
            bad_pws: 0,
            practices: 0,
            hunger: 0,
            thirst: 0,
            drunk: 0,
            hit: 0,
            max_hit: 0,
            mana: 0,
            max_mana: 0,
            mov: 0,
            max_move: 0,
            str_: 0,
            str_add: 0,
            intel: 0,
            wis: 0,
            dex: 0,
            con: 0,
            cha: 0,
            ac: 0,
            gold: 0,
            bank: 0,
            exp: 0,
            hitroll: 0,
            damroll: 0,
            olc_zone: NOTHING,
            page_length: 22,
            screen_width: 80,
            questpoints: 0,
            quest_counter: 0,
            current_quest: NOTHING,
            completed_quests: Vec::new(),
            triggers: Vec::new(),
            skills: Vec::new(),
            affects: Vec::new(),
            aliases: Vec::new(),
            vars: Vec::new(),
        }
    }
}

/// file modes, in the same spirit (CRASH/ETEXT/
/// SCRIPT_VARS/PLR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Plr,
    Objs,
    Vars,
    Text,
}

/// Only 'A'-'Z' are folded; every other byte is untouched.
fn lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() { b + (b'a' - b'A') } else { b }
}

/// get_filename. The whole name is lowercased, the bucket
/// is chosen from its first letter, and the relative path is
/// `<prefix>/<bucket>/<name>.<suffix>` (e.g. "plrfiles/A-E/bob.plr").
/// Empty names return None.
pub fn get_filename(kind: FileKind, name: &[u8]) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let lowered: Vec<u8> = name.iter().map(|&b| lower(b)).collect();
    let (prefix, suffix) = match kind {
        FileKind::Objs => ("plrobjs", "objs"),
        FileKind::Text => ("plrtext", "text"),
        FileKind::Vars => ("plrvars", "mem"),
        FileKind::Plr => ("plrfiles", "plr"),
    };
    let middle = match lowered[0] {
        b'a'..=b'e' => "A-E",
        b'f'..=b'j' => "F-J",
        b'k'..=b'o' => "K-O",
        b'p'..=b't' => "P-T",
        b'u'..=b'z' => "U-Z",
        _ => "ZZZ",
    };
    let name = String::from_utf8_lossy(&lowered);
    Some(PathBuf::from(format!("{prefix}/{middle}/{name}.{suffix}")))
}

/* ---------------------------------------------------------------- */
/* scanf-shaped lexical helpers                                     */
/* ---------------------------------------------------------------- */

/// True for ASCII space, tab, newline, vertical tab, form feed and return.
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Parse a leading integer, wrapping at 32 bits.
fn atoi(s: &[u8]) -> i32 {
    atol(s) as i32
}

/// Read one integer: whitespace, optional sign, at least one digit. On
/// failure `pos` is left where it was and None comes back.
fn scan_int(line: &[u8], pos: &mut usize) -> Option<i32> {
    scan_long(line, pos).map(|v| v as i32)
}

/// As [`scan_int`], widened to i64.
fn scan_long(line: &[u8], pos: &mut usize) -> Option<i64> {
    let mut i = *pos;
    while i < line.len() && is_ws(line[i]) {
        i += 1;
    }
    let start = i;
    let mut j = i;
    if j < line.len() && (line[j] == b'+' || line[j] == b'-') {
        j += 1;
    }
    let digits_start = j;
    while j < line.len() && line[j].is_ascii_digit() {
        j += 1;
    }
    if j == digits_start {
        return None;
    }
    *pos = j;
    Some(atol(&line[start..j]))
}

/// Read one token: skip whitespace, then take the non-whitespace run.
fn scan_str(line: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let mut i = *pos;
    while i < line.len() && is_ws(line[i]) {
        i += 1;
    }
    if i >= line.len() {
        return None;
    }
    let start = i;
    while i < line.len() && !is_ws(line[i]) {
        i += 1;
    }
    *pos = i;
    Some(line[start..i].to_vec())
}

/// Fill `out` with integers, left to right, stopping at the first one that
/// does not parse. Slots that are not reached KEEP their previous values:
/// the array persists across list lines, which the callers rely on.
/// Returns how many slots were written.
fn scan_ints_into(line: &[u8], out: &mut [i32]) -> usize {
    let mut pos = 0usize;
    let mut count = 0usize;
    for slot in out.iter_mut() {
        match scan_int(line, &mut pos) {
            Some(v) => {
                *slot = v;
                count += 1;
            }
            None => break,
        }
    }
    count
}

/// Up to `max` whitespace-delimited tokens.
fn scan_tokens(line: &[u8], max: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while out.len() < max {
        match scan_str(line, &mut pos) {
            Some(t) => out.push(t),
            None => break,
        }
    }
    out
}

// tag_argument lives in lex.rs (objsave's record parser needs it too). A
// line shorter than 4 bytes yields a short tag that matches nothing.
use crate::lex::tag_argument;

/// Parse `current/max`. The second value exists only if the `/` directly
/// follows the first number -- whitespace before it ends the parse -- and a
/// missing max reads as 0.
fn load_hmvs(line: &[u8]) -> (i32, i32) {
    let mut num = 0i32;
    let mut num2 = 0i32;
    let mut pos = 0usize;
    if let Some(v) = scan_int(line, &mut pos) {
        num = v;
        if line.get(pos) == Some(&b'/') {
            pos += 1;
            if let Some(v2) = scan_int(line, &mut pos) {
                num2 = v2;
            }
        }
    }
    (num, num2)
}

/// Whitespace EXCEPT '\t' — tab is the color escape and is
/// deliberately not skipped.
fn skip_spaces(line: &[u8], pos: &mut usize) {
    while *pos < line.len() && line[*pos] != b'\t' && is_ws(line[*pos]) {
        *pos += 1;
    }
}

/// any_one_arg: skip_spaces, then copy the token
/// LOWERed until whitespace. (Yes — pfile var names are lowercased on load.)
fn any_one_arg(line: &[u8], pos: &mut usize) -> Vec<u8> {
    skip_spaces(line, pos);
    let mut out = Vec::new();
    while *pos < line.len() && !is_ws(line[*pos]) {
        out.push(lower(line[*pos]));
        *pos += 1;
    }
    out
}

/* ---------------------------------------------------------------- */
/* load_char                                                        */
/* ---------------------------------------------------------------- */

fn err_name(pf: &PlayerFile) -> String {
    match &pf.name {
        Some(n) => String::from_utf8_lossy(n).into_owned(),
        None => "(null)".to_owned(), // glibc %s of NULL
    }
}

/// load_affects. 8-token rows are the current format;
/// 5-token rows are the legacy 32-bit form whose 5th value is a single AFF
/// bit NUMBER (set only when 0 < n < NUM_AFF_FLAGS); any other token count
/// logs a SYSERR — but the affect is still added (with whatever
/// spell/duration/modifier/location the previous line left behind, and a
/// zero bitvector). Terminates on a leading 0.
fn load_affects(r: &mut Reader, pf: &mut PlayerFile, errors: &mut Vec<String>) {
    let mut nums = [0i32; 8];
    loop {
        let Some(line) = r.get_line_sized(513) else { break }; // C spins on EOF; we stop
        let n_vars = scan_ints_into(&line, &mut nums);
        if nums[0] > 0 {
            let mut bitvector = [0u32; 4];
            if n_vars == 8 {
                for (slot, &v) in bitvector.iter_mut().zip(&nums[4..8]) {
                    *slot = v as u32;
                }
            } else if n_vars == 5 {
                let bit = nums[4];
                if bit > 0 && bit < NUM_AFF_FLAGS {
                    bitvector[(bit / 32) as usize] |= 1u32 << (bit % 32);
                }
            } else {
                errors.push(format!(
                    "SYSERR: Invalid affects in pfile ({}), expecting 5 or 8 values",
                    err_name(pf)
                ));
            }
            pf.affects.push(PfAffect {
                spell: nums[0],
                duration: nums[1],
                modifier: nums[2],
                location: nums[3],
                bitvector,
            });
        }
        if nums[0] == 0 {
            break;
        }
    }
}

/// load_skills: "<skillnum> <value>" pairs until a
/// leading 0 ("0 0" terminator). Unassigned locals persist across lines.
fn load_skills(r: &mut Reader, pf: &mut PlayerFile) {
    let mut nums = [0i32; 2];
    loop {
        let Some(line) = r.get_line_sized(513) else { break };
        scan_ints_into(&line, &mut nums);
        if nums[0] != 0 {
            pf.skills.push((nums[0], nums[1]));
        } else {
            break;
        }
    }
}

/// load_quests: one vnum per line until 65535 (NOTHING).
/// The cast to ush_int is preserved (`num as u16`).
fn load_quests(r: &mut Reader, pf: &mut PlayerFile) {
    let mut num = NOTHING;
    loop {
        let Some(line) = r.get_line_sized(513) else { break };
        let mut slot = [num];
        scan_ints_into(&line, &mut slot);
        num = slot[0];
        if num != NOTHING {
            pf.completed_quests.push(num as u16);
        } else {
            break;
        }
    }
}

/// read_aliases_ascii. Per alias, three get_line lines:
/// alias (one leading space stripped if present), replacement (a space is
/// prepended, then dropped again if the line already began with one — so
/// the in-memory replacement always keeps a leading space), type. A count
/// of exactly 0 clears the alias list. File order is preserved; see the
/// module docs.
fn read_aliases_ascii(r: &mut Reader, pf: &mut PlayerFile, count: i32) {
    if count == 0 {
        pf.aliases.clear();
        return;
    }
    let mut i = 0;
    while i < count {
        let Some(abuf) = r.get_line_sized(513) else { break };
        let Some(rline) = r.get_line_sized(512) else { break };
        let Some(tbuf) = r.get_line_sized(512) else { break };
        let mut rbuf = Vec::with_capacity(rline.len() + 1);
        rbuf.push(b' ');
        rbuf.extend_from_slice(&rline);
        if !abuf.is_empty() && rbuf.len() > 1 && !tbuf.is_empty() {
            let alias = if abuf[0] == b' ' { abuf[1..].to_vec() } else { abuf };
            let replacement = if rbuf[1] == b' ' { rbuf[1..].to_vec() } else { rbuf };
            pf.aliases.push(PfAlias { alias, replacement, type_: atoi(&tbuf) });
        }
        i += 1;
    }
}

/// read_saved_vars_ascii: `count` get_line lines of
/// "<name> <context> <value...>". name and context are read with
/// any_one_arg (LOWERCASED), then skip_spaces — so the value is the rest
/// of the line with leading whitespace (except tab) removed; it may be
/// empty and may contain spaces.
fn read_saved_vars_ascii(r: &mut Reader, pf: &mut PlayerFile, count: i32) {
    let mut i = 0;
    while i < count {
        let Some(line) = r.get_line_sized(1024) else { break };
        let mut pos = 0usize;
        let name = any_one_arg(&line, &mut pos);
        let context_str = any_one_arg(&line, &mut pos);
        skip_spaces(&line, &mut pos);
        pf.vars.push(PfVar {
            name,
            context: atol(&context_str),
            value: line[pos..].to_vec(),
        });
        i += 1;
    }
}

/// Act/Aff/Pref line: 4 asciiflag words, or the legacy single-token form
/// filling word 0 only. `f1` is a scratch buffer shared by all three
/// tags: the fallback for a short line is asciiflag_conv of
/// the WHOLE line for Act/Aff but of stale `f1` for Pref (
/// quirk §13.3).
fn load_flag_line(line: &[u8], flags: &mut [u32; 4], f1: &mut Vec<u8>, f1_fallback: bool) {
    let toks = scan_tokens(line, 4);
    if let Some(t) = toks.first() {
        *f1 = t.clone();
    }
    if toks.len() == 4 {
        for (slot, tok) in flags.iter_mut().zip(&toks) {
            *slot = asciiflag_conv(tok);
        }
    } else if f1_fallback {
        flags[0] = asciiflag_conv(f1);
    } else {
        flags[0] = asciiflag_conv(line);
    }
}

/// load_char, as pure parsing: read
/// `<lib>/plrfiles/<bucket>/<name>.plr` and return the [`PlayerFile`] plus
/// any SYSERR lines the load produced. None when the file is missing
/// (or the name is empty). The pfile defaults are pre-applied, so
/// absent tags leave defaults. Unknown scalar tags are silently ignored,
/// one line consumed.
pub fn load_char(lib: &Path, name: &[u8]) -> Option<(PlayerFile, Vec<String>)> {
    let rel = get_filename(FileKind::Plr, name)?;
    let data = std::fs::read(lib.join(rel)).ok()?;

    let mut pf = PlayerFile::default();
    let mut errors: Vec<String> = Vec::new();
    let mut r = Reader::new(&data);
    // Scratch buffer shared across Act/Aff/Pref lines (quirk §13.3).
    let mut f1: Vec<u8> = Vec::new();
    // Only the first Vars: tag creates the script. A second one returns
    // WITHOUT consuming its payload lines, which then parse as tags.
    let mut script_created = false;

    while let Some(raw) = r.get_line_sized(513) {
        let (tag, line) = tag_argument(&raw);
        match tag.as_slice() {
            b"Ac  " => pf.ac = atoi(&line),
            b"Act " => load_flag_line(&line, &mut pf.plr_flags, &mut f1, false),
            b"Aff " => load_flag_line(&line, &mut pf.aff_flags, &mut f1, false),
            b"Affs" => load_affects(&mut r, &mut pf, &mut errors),
            b"Alin" => pf.alignment = atoi(&line),
            b"Alis" => read_aliases_ascii(&mut r, &mut pf, atoi(&line)),

            b"Badp" => pf.bad_pws = atoi(&line),
            b"Bank" => pf.bank = atoi(&line),
            b"Brth" => pf.birth = atol(&line),

            b"Cha " => pf.cha = atoi(&line),
            b"Clas" => pf.class = atoi(&line),
            b"Con " => pf.con = atoi(&line),

            b"Desc" => match r.fread_string(&err_name(&pf)) {
                Ok(d) => pf.description = d,
                Err(e) => {
                    // Record the SYSERR and stop parsing.
                    errors.push(format!("SYSERR: {e}"));
                    break;
                }
            },
            b"Dex " => pf.dex = atoi(&line),
            b"Drnk" => pf.drunk = atoi(&line),
            b"Drol" => pf.damroll = atoi(&line),

            b"Exp " => pf.exp = atoi(&line),
            b"Frez" => pf.freeze_level = atoi(&line),
            b"Gold" => pf.gold = atoi(&line),

            b"Hit " => (pf.hit, pf.max_hit) = load_hmvs(&line),
            b"Hite" => pf.height = atoi(&line),
            b"Host" => pf.host = Some(line),
            b"Hrol" => pf.hitroll = atoi(&line),
            b"Hung" => pf.hunger = atoi(&line),

            b"Id  " => pf.idnum = atol(&line),
            b"Int " => pf.intel = atoi(&line),
            b"Invs" => pf.invis_level = atoi(&line),

            b"Last" => pf.last_logon = atol(&line),
            b"Lern" => pf.practices = atoi(&line),
            b"Levl" => pf.level = atoi(&line),
            b"Lmot" => pf.last_motd = atoi(&line) as i64,
            b"Lnew" => pf.last_news = atoi(&line) as i64,

            b"Mana" => (pf.mana, pf.max_mana) = load_hmvs(&line),
            b"Move" => (pf.mov, pf.max_move) = load_hmvs(&line),

            b"Name" => {
                // bounds this with strlcpy into a
                // char[MAX_NAME_LENGTH + 1] before the strdup. Nothing
                // overruns on this side -- the field owns its bytes -- but
                // without the same truncation the two disagree on a pfile
                // whose Name line is longer than a name can be, which
                // get_line lets it be by about 250 bytes.
                let mut name = line;
                name.truncate(MAX_NAME_LENGTH);
                pf.name = Some(name);
            }
            b"Olc " => pf.olc_zone = atoi(&line),

            b"Page" => pf.page_length = atoi(&line),
            b"Pass" => pf.passwd = line,
            b"Plyd" => pf.played = atoi(&line),
            b"PfIn" => pf.poofin = Some(line),
            b"PfOt" => pf.poofout = Some(line),
            b"Pref" => load_flag_line(&line, &mut pf.prf_flags, &mut f1, true),

            // Qpnt is the backward-compatibility alias.
            b"Qstp" | b"Qpnt" => pf.questpoints = atoi(&line),
            b"Qcur" => pf.current_quest = atoi(&line),
            b"Qcnt" => pf.quest_counter = atoi(&line),
            b"Qest" => load_quests(&mut r, &mut pf),

            b"Room" => pf.load_room = atoi(&line),

            b"Sex " => pf.sex = atoi(&line),
            b"ScrW" => pf.screen_width = atoi(&line),
            b"Skil" => load_skills(&mut r, &mut pf),
            b"Str " => (pf.str_, pf.str_add) = load_hmvs(&line),

            b"Thir" => pf.thirst = atoi(&line),
            b"Thr1" => pf.saving_throws[0] = atoi(&line),
            b"Thr2" => pf.saving_throws[1] = atoi(&line),
            b"Thr3" => pf.saving_throws[2] = atoi(&line),
            b"Thr4" => pf.saving_throws[3] = atoi(&line),
            b"Thr5" => pf.saving_throws[4] = atoi(&line),
            b"Titl" => pf.title = Some(line),
            // This layer always records trigger vnums; whether they are
            // attached is the game layer's business.
            b"Trig" => pf.triggers.push(atoi(&line) as u16),

            b"Vars" => {
                let count = atoi(&line);
                if !script_created {
                    script_created = true;
                    read_saved_vars_ascii(&mut r, &mut pf, count);
                }
            }

            b"Wate" => pf.weight = atoi(&line),
            b"Wimp" => pf.wimpy = atoi(&line),
            b"Wis " => pf.wis = atoi(&line),

            _ => {} // silently swallowed, one line consumed (quirk §13.4)
        }
    }

    Some((pf, errors))
}

/* ---------------------------------------------------------------- */
/* save_char                                                        */
/* ---------------------------------------------------------------- */

/// "Tttt: value\n" — 4-byte space-padded tag, ": ", value, LF.
fn put_str(out: &mut Vec<u8>, tag: &[u8; 4], value: &[u8]) {
    out.extend_from_slice(tag);
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.push(b'\n');
}

fn put_int(out: &mut Vec<u8>, tag: &[u8; 4], v: i64) {
    put_str(out, tag, v.to_string().as_bytes());
}

/// "Tag: cur/max\n" (Hit/Mana/Move/Str).
fn put_pair(out: &mut Vec<u8>, tag: &[u8; 4], a: i32, b: i32) {
    put_str(out, tag, format!("{a}/{b}").as_bytes());
}

/// Four sprintascii words, single-space separated.
fn put_flags(out: &mut Vec<u8>, tag: &[u8; 4], flags: &[u32; 4]) {
    let mut value = Vec::new();
    for (i, &w) in flags.iter().enumerate() {
        if i > 0 {
            value.push(b' ');
        }
        value.extend_from_slice(&sprintascii(w));
    }
    put_str(out, tag, &value);
}

/// save_char, rendering only (the runtime dance of
/// unequipping/de-affecting is the game layer's job — the DTO already
/// holds raw values). Fixed tag order, omit-when-default per row, LF line
/// endings.
pub fn save_char(pf: &PlayerFile) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();

    if let Some(name) = &pf.name {
        put_str(&mut out, b"Name", name);
    }
    // The password field is never absent, so it is always written.
    put_str(&mut out, b"Pass", &pf.passwd);
    if let Some(title) = &pf.title {
        put_str(&mut out, b"Titl", title);
    }
    if let Some(desc) = &pf.description {
        if !desc.is_empty() {
            // "Desc:\n<strip_cr'd text>~\n": strip_cr
            // removes every '\r'; '\t' color codes are written RAW (the
            // writer does not re-escape to '@' — quirk §13.19).
            out.extend_from_slice(b"Desc:\n");
            out.extend(desc.iter().copied().filter(|&b| b != b'\r'));
            out.extend_from_slice(b"~\n");
        }
    }
    if let Some(poofin) = &pf.poofin {
        put_str(&mut out, b"PfIn", poofin);
    }
    if let Some(poofout) = &pf.poofout {
        put_str(&mut out, b"PfOt", poofout);
    }
    if pf.sex != 0 {
        put_int(&mut out, b"Sex ", pf.sex.into());
    }
    if pf.class != 0 {
        put_int(&mut out, b"Clas", pf.class.into());
    }
    if pf.level != 0 {
        put_int(&mut out, b"Levl", pf.level.into());
    }

    put_int(&mut out, b"Id  ", pf.idnum);
    put_int(&mut out, b"Brth", pf.birth);
    put_int(&mut out, b"Plyd", pf.played.into());
    put_int(&mut out, b"Last", pf.last_logon);

    // Lmot/Lnew are time_t but printed through an (int) cast.
    if pf.last_motd != 0 {
        put_int(&mut out, b"Lmot", (pf.last_motd as i32).into());
    }
    if pf.last_news != 0 {
        put_int(&mut out, b"Lnew", (pf.last_news as i32).into());
    }

    if let Some(host) = &pf.host {
        put_str(&mut out, b"Host", host);
    }
    if pf.height != 0 {
        put_int(&mut out, b"Hite", pf.height.into());
    }
    if pf.weight != 0 {
        put_int(&mut out, b"Wate", pf.weight.into());
    }
    if pf.alignment != 0 {
        put_int(&mut out, b"Alin", pf.alignment.into());
    }

    put_flags(&mut out, b"Act ", &pf.plr_flags);
    put_flags(&mut out, b"Aff ", &pf.aff_flags);
    put_flags(&mut out, b"Pref", &pf.prf_flags);

    const THR_TAGS: [&[u8; 4]; 5] = [b"Thr1", b"Thr2", b"Thr3", b"Thr4", b"Thr5"];
    for (tag, &save) in THR_TAGS.iter().zip(&pf.saving_throws) {
        if save != 0 {
            put_int(&mut out, tag, save.into());
        }
    }

    if pf.wimpy != 0 {
        put_int(&mut out, b"Wimp", pf.wimpy.into());
    }
    if pf.freeze_level != 0 {
        put_int(&mut out, b"Frez", pf.freeze_level.into());
    }
    if pf.invis_level != 0 {
        put_int(&mut out, b"Invs", pf.invis_level.into());
    }
    // PFDEF_LOADROOM is 0, so NOWHERE (65535) IS written (quirk §13.6).
    if pf.load_room != 0 {
        put_int(&mut out, b"Room", pf.load_room.into());
    }

    if pf.bad_pws != 0 {
        put_int(&mut out, b"Badp", pf.bad_pws.into());
    }
    if pf.practices != 0 {
        put_int(&mut out, b"Lern", pf.practices.into());
    }

    // Conditions: written when != 0 AND level < LVL_IMMORT.
    // (-1 = "off" IS written for mortals; the imm -1s are suppressed by level.)
    if pf.hunger != 0 && pf.level < LVL_IMMORT {
        put_int(&mut out, b"Hung", pf.hunger.into());
    }
    if pf.thirst != 0 && pf.level < LVL_IMMORT {
        put_int(&mut out, b"Thir", pf.thirst.into());
    }
    if pf.drunk != 0 && pf.level < LVL_IMMORT {
        put_int(&mut out, b"Drnk", pf.drunk.into());
    }

    if pf.hit != 0 || pf.max_hit != 0 {
        put_pair(&mut out, b"Hit ", pf.hit, pf.max_hit);
    }
    if pf.mana != 0 || pf.max_mana != 0 {
        put_pair(&mut out, b"Mana", pf.mana, pf.max_mana);
    }
    if pf.mov != 0 || pf.max_move != 0 {
        put_pair(&mut out, b"Move", pf.mov, pf.max_move);
    }

    if pf.str_ != 0 || pf.str_add != 0 {
        put_pair(&mut out, b"Str ", pf.str_, pf.str_add);
    }

    if pf.intel != 0 {
        put_int(&mut out, b"Int ", pf.intel.into());
    }
    if pf.wis != 0 {
        put_int(&mut out, b"Wis ", pf.wis.into());
    }
    if pf.dex != 0 {
        put_int(&mut out, b"Dex ", pf.dex.into());
    }
    if pf.con != 0 {
        put_int(&mut out, b"Con ", pf.con.into());
    }
    if pf.cha != 0 {
        put_int(&mut out, b"Cha ", pf.cha.into());
    }

    if pf.ac != 0 {
        put_int(&mut out, b"Ac  ", pf.ac.into());
    }
    if pf.gold != 0 {
        put_int(&mut out, b"Gold", pf.gold.into());
    }
    if pf.bank != 0 {
        put_int(&mut out, b"Bank", pf.bank.into());
    }
    if pf.exp != 0 {
        put_int(&mut out, b"Exp ", pf.exp.into());
    }
    if pf.hitroll != 0 {
        put_int(&mut out, b"Hrol", pf.hitroll.into());
    }
    if pf.damroll != 0 {
        put_int(&mut out, b"Drol", pf.damroll.into());
    }
    if pf.olc_zone != NOTHING {
        put_int(&mut out, b"Olc ", pf.olc_zone.into());
    }
    if pf.page_length != 22 {
        put_int(&mut out, b"Page", pf.page_length.into());
    }
    if pf.screen_width != 80 {
        put_int(&mut out, b"ScrW", pf.screen_width.into());
    }
    if pf.questpoints != 0 {
        put_int(&mut out, b"Qstp", pf.questpoints.into());
    }
    if pf.quest_counter != 0 {
        put_int(&mut out, b"Qcnt", pf.quest_counter.into());
    }
    if !pf.completed_quests.is_empty() {
        out.extend_from_slice(b"Qest:\n");
        for &vnum in &pf.completed_quests {
            out.extend_from_slice(vnum.to_string().as_bytes());
            out.push(b'\n');
        }
        out.extend_from_slice(b"65535\n"); // NOTHING terminator
    }
    if pf.current_quest != NOTHING {
        put_int(&mut out, b"Qcur", pf.current_quest.into());
    }

    // One Trig: line per attached trigger, written whenever any exist --
    // even when the loader would go on to drop them.
    for &vnum in &pf.triggers {
        put_int(&mut out, b"Trig", vnum.into());
    }

    // Skil: block only for mortals — and then ALWAYS, even with no skills
    // ("Skil:\n0 0\n"). Zero-valued skills are not written.
    if pf.level < LVL_IMMORT {
        out.extend_from_slice(b"Skil:\n");
        for &(num, value) in &pf.skills {
            if num != 0 && value != 0 {
                out.extend_from_slice(format!("{num} {value}\n").as_bytes());
            }
        }
        out.extend_from_slice(b"0 0\n");
    }

    // Affs: gated on the FIRST affect's spell being > 0;
    // rows are capped at MAX_AFFECT and skip spell == 0; the bitvector
    // words print as signed ints.
    if pf.affects.first().is_some_and(|a| a.spell > 0) {
        out.extend_from_slice(b"Affs:\n");
        for aff in pf.affects.iter().take(MAX_AFFECT) {
            if aff.spell != 0 {
                out.extend_from_slice(
                    format!(
                        "{} {} {} {} {} {} {} {}\n",
                        aff.spell,
                        aff.duration,
                        aff.modifier,
                        aff.location,
                        aff.bitvector[0] as i32,
                        aff.bitvector[1] as i32,
                        aff.bitvector[2] as i32,
                        aff.bitvector[3] as i32,
                    )
                    .as_bytes(),
                );
            }
        }
        out.extend_from_slice(b"0 0 0 0 0 0 0 0\n");
    }

    // write_aliases_ascii: nothing at all when empty.
    // The alias line gets a space prepended (to survive get_line's
    // '*'-comment rule); the replacement is written raw — it keeps the
    // leading space it carries in memory.
    if !pf.aliases.is_empty() {
        put_str(&mut out, b"Alis", pf.aliases.len().to_string().as_bytes());
        for alias in &pf.aliases {
            out.push(b' ');
            out.extend_from_slice(&alias.alias);
            out.push(b'\n');
            out.extend_from_slice(&alias.replacement);
            out.push(b'\n');
            out.extend_from_slice(alias.type_.to_string().as_bytes());
            out.push(b'\n');
        }
    }

    // save_char_vars_ascii: vars whose name begins
    // with '-' are session-local and skipped; block only when any remain.
    let saved_vars = || pf.vars.iter().filter(|v| v.name.first() != Some(&b'-'));
    let count = saved_vars().count();
    if count != 0 {
        put_str(&mut out, b"Vars", count.to_string().as_bytes());
        for var in saved_vars() {
            out.extend_from_slice(&var.name);
            out.push(b' ');
            out.extend_from_slice(var.context.to_string().as_bytes());
            out.push(b' ');
            out.extend_from_slice(&var.value);
            out.push(b'\n');
        }
    }

    out
}

/* ---------------------------------------------------------------- */
/* files on disk                                                    */
/* ---------------------------------------------------------------- */

/// Write `bytes` to `path` via `<path>.tmp` + fsync + atomic rename
/// (§14 atomicity; std::fs::rename replaces on Win10+).
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.to_path_buf().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Write a player's pfile under `lib` (creating the bucket directory if
/// needed, rather than relying on shipped placeholder files), atomically.
pub fn write_pfile(lib: &Path, name: &[u8], bytes: &[u8]) -> io::Result<()> {
    let rel = get_filename(FileKind::Plr, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty player name"))?;
    let path = lib.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&path, bytes)
}

/* ---------------------------------------------------------------- */
/* player index                                                     */
/* ---------------------------------------------------------------- */

/// One `lib/plrfiles/index` row (player_index_element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: Vec<u8>,
    pub id: i64,
    pub level: i32,
    pub flags: i32,
    pub last: i64,
}

/// build_player_index: read `lib/plrfiles/index` with
/// get_line semantics, one `<id> <name> <level> <flags> <last>` row per
/// player until the `~` terminator line; flags via asciiflag_conv;
/// top_idnum = max id, never below 0.
/// None when the file is missing ("No player index file!").
pub fn load_index(lib: &Path) -> Option<(Vec<IndexEntry>, i64)> {
    let data = std::fs::read(lib.join("plrfiles").join("index")).ok()?;
    let mut r = Reader::new(&data);
    let mut entries = Vec::new();
    let mut top_idnum = 0i64;
    while let Some(line) = r.get_line() {
        if line.first() == Some(&b'~') {
            break;
        }
        // Five fields, in order; stop at the first that does not parse and
        // leave the rest at their defaults.
        let mut entry = IndexEntry { name: Vec::new(), id: 0, level: 0, flags: 0, last: 0 };
        let mut pos = 0usize;
        'fields: {
            let Some(id) = scan_long(&line, &mut pos) else { break 'fields };
            entry.id = id;
            let Some(name) = scan_str(&line, &mut pos) else { break 'fields };
            entry.name = name;
            let Some(level) = scan_int(&line, &mut pos) else { break 'fields };
            entry.level = level;
            let Some(bits) = scan_str(&line, &mut pos) else { break 'fields };
            entry.flags = asciiflag_conv(&bits) as i32;
            let Some(last) = scan_long(&line, &mut pos) else { break 'fields };
            entry.last = last;
        }
        top_idnum = top_idnum.max(entry.id);
        entries.push(entry);
    }
    Some((entries, top_idnum))
}

/// save_player_index: `<id> <name> <level> <flags>
/// <last>\n` per entry (empty names skipped), flags via sprintascii ("0"
/// when empty), then the `~\n` terminator. Names are stored lowercase
/// (create_entry's invariant); written atomically.
pub fn save_index(lib: &Path, entries: &[IndexEntry]) -> io::Result<()> {
    let mut out: Vec<u8> = Vec::new();
    for entry in entries {
        if entry.name.is_empty() {
            continue;
        }
        out.extend_from_slice(entry.id.to_string().as_bytes());
        out.push(b' ');
        out.extend(entry.name.iter().map(|&b| lower(b)));
        out.push(b' ');
        out.extend_from_slice(entry.level.to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(&sprintascii(entry.flags as u32));
        out.push(b' ');
        out.extend_from_slice(entry.last.to_string().as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"~\n");

    let dir = lib.join("plrfiles");
    std::fs::create_dir_all(&dir)?;
    write_atomic(&dir.join("index"), &out)
}

/* ---------------------------------------------------------------- */
/* tests                                                            */
/* ---------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_lib(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("rustmud-players-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lines(ls: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for l in ls {
            out.extend_from_slice(l);
            out.push(b'\n');
        }
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn get_filename_buckets() {
        assert_eq!(
            get_filename(FileKind::Plr, b"Bob").unwrap(),
            Path::new("plrfiles/A-E/bob.plr")
        );
        assert_eq!(
            get_filename(FileKind::Plr, b"Zeus").unwrap(),
            Path::new("plrfiles/U-Z/zeus.plr")
        );
        assert_eq!(
            get_filename(FileKind::Plr, b"fizban").unwrap(),
            Path::new("plrfiles/F-J/fizban.plr")
        );
        assert_eq!(
            get_filename(FileKind::Plr, b"Kvothe").unwrap(),
            Path::new("plrfiles/K-O/kvothe.plr")
        );
        assert_eq!(
            get_filename(FileKind::Plr, b"Ptah").unwrap(),
            Path::new("plrfiles/P-T/ptah.plr")
        );
        assert_eq!(
            get_filename(FileKind::Objs, b"Bob").unwrap(),
            Path::new("plrobjs/A-E/bob.objs")
        );
        assert_eq!(
            get_filename(FileKind::Vars, b"Bob").unwrap(),
            Path::new("plrvars/A-E/bob.mem")
        );
        assert_eq!(
            get_filename(FileKind::Text, b"Bob").unwrap(),
            Path::new("plrtext/A-E/bob.text")
        );
        // Non-letter first char buckets to ZZZ.
        assert_eq!(
            get_filename(FileKind::Plr, b"123abc").unwrap(),
            Path::new("plrfiles/ZZZ/123abc.plr")
        );
        assert!(get_filename(FileKind::Plr, b"").is_none());
    }

    #[test]
    fn defaults_match_pfdefaults() {
        let pf = PlayerFile::default();
        assert_eq!(pf.olc_zone, 65535);
        assert_eq!(pf.page_length, 22);
        assert_eq!(pf.screen_width, 80);
        assert_eq!(pf.current_quest, 65535);
        assert_eq!(pf.load_room, 0); // quirk §13.6: 0, not NOWHERE
        assert_eq!(pf.level, 0);
        assert_eq!(pf.hunger, 0);
        assert!(pf.name.is_none());
        assert!(pf.passwd.is_empty());
        assert!(pf.skills.is_empty());
    }

    #[test]
    fn tag_argument_semantics() {
        // 4-char tag, then ALL consecutive ':'/' ' skipped.
        assert_eq!(tag_argument(b"Name: Bob"), (b"Name".to_vec(), b"Bob".to_vec()));
        assert_eq!(tag_argument(b"Ac  : 100"), (b"Ac  ".to_vec(), b"100".to_vec()));
        assert_eq!(tag_argument(b"Levl:::   7"), (b"Levl".to_vec(), b"7".to_vec()));
        assert_eq!(tag_argument(b"Qest:"), (b"Qest".to_vec(), b"".to_vec()));
        // A value can never begin with a space or colon.
        assert_eq!(tag_argument(b"Titl:  : x"), (b"Titl".to_vec(), b"x".to_vec()));
        // Short lines yield short tags that match nothing.
        assert_eq!(tag_argument(b"Id"), (b"Id".to_vec(), b"".to_vec()));
    }

    #[test]
    fn hmvs_parsing() {
        assert_eq!(load_hmvs(b"45/60"), (45, 60));
        assert_eq!(load_hmvs(b"100"), (100, 0)); // missing /max -> 0
        assert_eq!(load_hmvs(b"-5/-8"), (-5, -8));
        assert_eq!(load_hmvs(b"45 /60"), (45, 0)); // literal '/' must follow directly
        assert_eq!(load_hmvs(b"45/ 60"), (45, 60)); // %d skips its own leading ws
        assert_eq!(load_hmvs(b"junk"), (0, 0));
    }

    fn full_pf() -> PlayerFile {
        PlayerFile {
            name: Some(b"Bob".to_vec()),
            passwd: b"BoXhYrmlqTW7g".to_vec(),
            title: Some(b"the Fool".to_vec()),
            // '@@' survives parse_at; mid-line '~' is literal; every line
            // (incl. the last) must end \r\n for a stable round-trip.
            description: Some(b"A short fellow with @@ color codes.\r\nSecond ~ line.\r\n".to_vec()),
            poofin: Some(b"appears in a puff of smoke".to_vec()),
            poofout: Some(b"vanishes abruptly".to_vec()),
            sex: 1,
            class: 3,
            level: 20,
            idnum: 42,
            birth: 1755856039,
            played: 3600,
            last_logon: 1755870000,
            last_motd: 1755870001,
            last_news: 1755870002,
            host: Some(b"127.0.0.1".to_vec()),
            height: 170,
            weight: 150,
            alignment: -350,
            plr_flags: [0b101, 1, 0, 1 << 31],
            aff_flags: [1 << 3, 0, 1 << 26, 0],
            prf_flags: [0x0f, 0, 0, 3],
            saving_throws: [1, 2, -3, 4, 5],
            wimpy: 10,
            freeze_level: 31,
            invis_level: 33,
            load_room: 65535, // NOWHERE is written because != 0 (§13.6)
            bad_pws: 2,
            practices: 5,
            hunger: 24,
            thirst: -1,
            drunk: 3,
            hit: 45,
            max_hit: 60,
            mana: 100,
            max_mana: 100,
            mov: 82,
            max_move: 96,
            str_: 18,
            str_add: 50,
            intel: 11,
            wis: 12,
            dex: 16,
            con: 13,
            cha: 10,
            ac: 100,
            gold: 1500,
            bank: 100000,
            exp: 12000,
            hitroll: 2,
            damroll: 3,
            olc_zone: 30,
            page_length: 40,
            screen_width: 120,
            questpoints: 7,
            quest_counter: 4,
            current_quest: 1102,
            completed_quests: vec![3000, 3001],
            triggers: vec![100, 200],
            skills: vec![(131, 75), (141, 40)],
            affects: vec![
                PfAffect { spell: 27, duration: 5, modifier: -2, location: 13, bitvector: [0; 4] },
                PfAffect {
                    spell: 2,
                    duration: 10,
                    modifier: 1,
                    location: 1,
                    bitvector: [1 << 31, 2, 3, 0x8000_0001],
                },
            ],
            aliases: vec![
                PfAlias { alias: b"gb".to_vec(), replacement: b" get all bag".to_vec(), type_: 0 },
                PfAlias {
                    alias: b"kill".to_vec(),
                    replacement: b" cast 'magic missile'".to_vec(),
                    type_: 1,
                },
            ],
            vars: vec![
                PfVar { name: b"questflag".to_vec(), context: 0, value: b"done".to_vec() },
                PfVar { name: b"multi".to_vec(), context: 0, value: b"two words here".to_vec() },
            ],
        }
    }

    #[test]
    fn full_round_trip_is_byte_identical() {
        let lib = temp_lib("roundtrip");
        let pf = full_pf();
        let bytes1 = save_char(&pf);

        // Spot-check some exact renderings.
        assert!(contains(&bytes1, b"Name: Bob\n"));
        assert!(contains(&bytes1, b"Act : ac a 0 F\n"));
        assert!(contains(&bytes1, b"Aff : d 0 A 0\n"));
        assert!(contains(&bytes1, b"Pref: abcd 0 0 ab\n"));
        assert!(contains(&bytes1, b"Room: 65535\n"));
        assert!(contains(&bytes1, b"Hit : 45/60\n"));
        assert!(contains(&bytes1, b"Str : 18/50\n"));
        assert!(contains(&bytes1, b"Thir: -1\n"));
        assert!(contains(&bytes1, b"Qest:\n3000\n3001\n65535\n"));
        assert!(contains(&bytes1, b"Trig: 100\nTrig: 200\n"));
        assert!(contains(&bytes1, b"Skil:\n131 75\n141 40\n0 0\n"));
        assert!(contains(&bytes1, b"2 10 1 1 -2147483648 2 3 -2147483647\n"));
        assert!(contains(&bytes1, b"Alis: 2\n gb\n get all bag\n0\n"));
        assert!(contains(&bytes1, b"Vars: 2\nquestflag 0 done\nmulti 0 two words here\n"));
        assert!(contains(&bytes1, b"Desc:\nA short fellow with @@ color codes.\nSecond ~ line.\n~\n"));

        write_pfile(&lib, b"Bob", &bytes1).unwrap();
        assert!(lib.join("plrfiles/A-E/bob.plr").is_file());

        let (loaded, errors) = load_char(&lib, b"Bob").unwrap();
        assert!(errors.is_empty(), "unexpected SYSERRs: {errors:?}");
        assert_eq!(loaded.name.as_deref(), Some(b"Bob".as_slice()));
        assert_eq!(loaded.description.as_deref(), pf.description.as_deref());
        assert_eq!(loaded.plr_flags, pf.plr_flags);
        assert_eq!(loaded.affects, pf.affects);
        assert_eq!(loaded.aliases, pf.aliases);
        assert_eq!(loaded.vars, pf.vars);
        assert_eq!(loaded.skills, pf.skills);
        assert_eq!(loaded.completed_quests, pf.completed_quests);
        assert_eq!(loaded.triggers, pf.triggers);
        assert_eq!(loaded.load_room, 65535);
        assert_eq!(loaded.thirst, -1);

        let bytes2 = save_char(&loaded);
        assert_eq!(
            bytes1,
            bytes2,
            "save -> load -> save must be byte-identical\nfirst:\n{}\nsecond:\n{}",
            String::from_utf8_lossy(&bytes1),
            String::from_utf8_lossy(&bytes2)
        );
    }

    #[test]
    fn parses_handwritten_doc_example() {
        let lib = temp_lib("handwritten");
        let text = lines(&[
            b"* comment lines and blanks are skipped by get_line",
            b"",
            b"Name: Bob",
            b"Pass: BoXhYrmlqTW7g",
            b"Titl: the Fool",
            b"Desc:",
            b"A short @@fellow~ish soul with @Rred toes.",
            b"~",
            b"Sex : 1",
            b"Clas: 3",
            b"Levl: 5",
            b"Id  : 42",
            b"Brth: 1755856039",
            b"Plyd: 3600",
            b"Last: 1755870000",
            b"Host: 127.0.0.1",
            b"Hite: 170",
            b"Wate: 150",
            b"Act : c 0 0 0",
            b"Aff : 64",
            b"Pref: acdl 0 0 0",
            b"Thr1: 2",
            b"Room: 3001",
            b"Hung: 24",
            b"Thir: 24",
            b"Hit : 45/60",
            b"Mana: 100",
            b"Move: 82/96",
            b"Str : 14/0",
            b"Int : 11",
            b"Wis : 12",
            b"Dex : 16",
            b"Con : 13",
            b"Cha : 10",
            b"Ac  : 100",
            b"Gold: 1500",
            b"Exp : 12000",
            b"Junk: 99",
            b"Qest:",
            b"3000",
            b"65535",
            b"Skil:",
            b"131 75",
            b"141 40",
            b"0 0",
            b"Affs:",
            b"27 5 -2 13 0 0 0 0",
            b"1 12 0 0 3",
            b"0 0 0 0 0 0 0 0",
            b"Alis: 1",
            b" gb",
            b"get all bag",
            b"0",
            b"Vars: 1",
            b"QuestFlag 0 done deal",
        ]);
        write_pfile(&lib, b"Bob", &text).unwrap();

        let (pf, errors) = load_char(&lib, b"Bob").unwrap();
        assert!(errors.is_empty(), "unexpected SYSERRs: {errors:?}");
        assert_eq!(pf.name.as_deref(), Some(b"Bob".as_slice()));
        assert_eq!(pf.passwd, b"BoXhYrmlqTW7g");
        assert_eq!(pf.title.as_deref(), Some(b"the Fool".as_slice()));
        // '@@' preserved, mid-line '~' literal, bare '@' -> '\t' (parse_at).
        assert_eq!(
            pf.description.as_deref(),
            Some(b"A short @@fellow~ish soul with \tRred toes.\r\n".as_slice())
        );
        assert_eq!((pf.sex, pf.class, pf.level), (1, 3, 5));
        assert_eq!((pf.idnum, pf.birth, pf.played, pf.last_logon), (42, 1755856039, 3600, 1755870000));
        assert_eq!(pf.host.as_deref(), Some(b"127.0.0.1".as_slice()));
        assert_eq!((pf.height, pf.weight), (170, 150));
        assert_eq!(pf.plr_flags, [1 << 2, 0, 0, 0]);
        // Legacy single-token numeric form fills word 0 via atol.
        assert_eq!(pf.aff_flags, [64, 0, 0, 0]);
        assert_eq!(pf.prf_flags, [(1 << 0) | (1 << 2) | (1 << 3) | (1 << 11), 0, 0, 0]);
        assert_eq!(pf.saving_throws, [2, 0, 0, 0, 0]);
        assert_eq!(pf.load_room, 3001);
        assert_eq!((pf.hunger, pf.thirst, pf.drunk), (24, 24, 0));
        assert_eq!((pf.hit, pf.max_hit), (45, 60));
        assert_eq!((pf.mana, pf.max_mana), (100, 0)); // missing /max -> 0
        assert_eq!((pf.mov, pf.max_move), (82, 96));
        assert_eq!((pf.str_, pf.str_add), (14, 0));
        assert_eq!((pf.intel, pf.wis, pf.dex, pf.con, pf.cha), (11, 12, 16, 13, 10));
        assert_eq!((pf.ac, pf.gold, pf.exp), (100, 1500, 12000));
        // Absent tags keep defaults; unknown "Junk:" silently ignored.
        assert_eq!(pf.olc_zone, 65535);
        assert_eq!(pf.page_length, 22);
        assert_eq!(pf.screen_width, 80);
        assert_eq!(pf.current_quest, 65535);
        assert_eq!(pf.completed_quests, vec![3000]);
        assert_eq!(pf.skills, vec![(131, 75), (141, 40)]);
        // Second affect row is the legacy 5-token form: bit 3 set.
        assert_eq!(pf.affects.len(), 2);
        assert_eq!(pf.affects[0], PfAffect { spell: 27, duration: 5, modifier: -2, location: 13, bitvector: [0; 4] });
        assert_eq!(pf.affects[1], PfAffect { spell: 1, duration: 12, modifier: 0, location: 0, bitvector: [1 << 3, 0, 0, 0] });
        // Old-format replacement (no leading space in the file) gains one.
        assert_eq!(
            pf.aliases,
            vec![PfAlias { alias: b"gb".to_vec(), replacement: b" get all bag".to_vec(), type_: 0 }]
        );
        // Var name is LOWERCASED by any_one_arg; value is the untrimmed
        // rest of the line after the two words.
        assert_eq!(
            pf.vars,
            vec![PfVar { name: b"questflag".to_vec(), context: 0, value: b"done deal".to_vec() }]
        );
    }

    #[test]
    fn affs_bad_token_count_records_syserr_but_still_adds() {
        let lib = temp_lib("badaffs");
        let text = lines(&[
            b"Name: Al",
            b"Affs:",
            b"7 1 2 3 4 5", // 6 tokens: neither 5 nor 8
            b"0 0 0 0 0 0 0 0",
        ]);
        write_pfile(&lib, b"Al", &text).unwrap();
        let (pf, errors) = load_char(&lib, b"Al").unwrap();
        assert_eq!(
            errors,
            vec!["SYSERR: Invalid affects in pfile (Al), expecting 5 or 8 values".to_owned()]
        );
        // The affect is still added, with a zero bitvector.
        assert_eq!(
            pf.affects,
            vec![PfAffect { spell: 7, duration: 1, modifier: 2, location: 3, bitvector: [0; 4] }]
        );
    }

    #[test]
    fn name_is_truncated_to_max_name_length() {
        // The Name tag is truncated at MAX_NAME_LENGTH, so a pfile whose
        // Name line was lengthened still loads. The index name is
        // untouched -- that is what makes such a pfile load at all.
        let lib = temp_lib("longname");
        let mut long_name = b"Name: ".to_vec();
        long_name.extend(std::iter::repeat(b'a').take(250));
        let text = lines(&[&long_name[..], b"Levl: 5"]);
        write_pfile(&lib, b"Al", &text).unwrap();
        let (pf, _) = load_char(&lib, b"Al").unwrap();
        assert_eq!(pf.name.as_deref(), Some(&b"aaaaaaaaaaaaaaaaaaaa"[..]));
        assert_eq!(pf.name.unwrap().len(), MAX_NAME_LENGTH);
    }

    #[test]
    fn a_name_that_fits_is_untouched() {
        // MAX_NAME_LENGTH is the longest creation issues, so the truncation
        // must not touch it: exactly 20 characters survives whole.
        let lib = temp_lib("fitsname");
        let exact = b"Name: abcdefghijklmnopqrst";
        let text = lines(&[&exact[..], b"Levl: 5"]);
        write_pfile(&lib, b"Al", &text).unwrap();
        let (pf, _) = load_char(&lib, b"Al").unwrap();
        assert_eq!(pf.name.as_deref(), Some(&b"abcdefghijklmnopqrst"[..]));
        assert_eq!(pf.name.unwrap().len(), MAX_NAME_LENGTH);
    }

    #[test]
    fn pref_stale_f1_fallback() {
        // A short Pref line falls back to the stale token left by the most
        // recent Act/Aff line, not to the Pref line itself.
        let lib = temp_lib("staleprf");
        let text = lines(&[b"Name: Al", b"Aff : abc", b"Pref:"]);
        write_pfile(&lib, b"Al", &text).unwrap();
        let (pf, _) = load_char(&lib, b"Al").unwrap();
        assert_eq!(pf.aff_flags[0], 0b111);
        assert_eq!(pf.prf_flags[0], 0b111); // from f1 == "abc"
    }

    #[test]
    fn immortal_omits_conditions_and_skills() {
        let mut pf = full_pf();
        pf.level = 34;
        pf.hunger = -1;
        pf.thirst = -1;
        pf.drunk = -1;
        let bytes = save_char(&pf);
        assert!(!contains(&bytes, b"Hung:"));
        assert!(!contains(&bytes, b"Thir:"));
        assert!(!contains(&bytes, b"Drnk:"));
        assert!(!contains(&bytes, b"Skil:"));

        // A mortal with no skills still gets the empty Skil block.
        let mut pf = PlayerFile::default();
        pf.level = 30;
        let bytes = save_char(&pf);
        assert!(contains(&bytes, b"Skil:\n0 0\n"));
        // And a zero-condition mortal writes no condition lines.
        assert!(!contains(&bytes, b"Hung:"));
    }

    #[test]
    fn minimal_default_save_shape() {
        // A default (level-0, everything default) PlayerFile writes only
        // the always-present rows: Pass, Id/Brth/Plyd/Last, the three
        // flag lines, and the mortal Skil block.
        let bytes = save_char(&PlayerFile::default());
        let expected = lines(&[
            b"Pass: ",
            b"Id  : 0",
            b"Brth: 0",
            b"Plyd: 0",
            b"Last: 0",
            b"Act : 0 0 0 0",
            b"Aff : 0 0 0 0",
            b"Pref: 0 0 0 0",
            b"Skil:",
            b"0 0",
        ]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn load_missing_returns_none() {
        let lib = temp_lib("missing");
        assert!(load_char(&lib, b"nobody").is_none());
        assert!(load_char(&lib, b"").is_none());
    }

    #[test]
    fn write_pfile_replaces_atomically() {
        let lib = temp_lib("atomic");
        write_pfile(&lib, b"Bob", b"first\n").unwrap();
        write_pfile(&lib, b"Bob", b"second\n").unwrap();
        let path = lib.join("plrfiles/A-E/bob.plr");
        assert_eq!(std::fs::read(&path).unwrap(), b"second\n");
        assert!(!lib.join("plrfiles/A-E/bob.plr.tmp").exists());
    }

    #[test]
    fn index_round_trip() {
        let lib = temp_lib("index");
        let entries = vec![
            IndexEntry { name: b"bob".to_vec(), id: 42, level: 5, flags: 0, last: 1755870000 },
            IndexEntry { name: b"Zeus".to_vec(), id: 100, level: 34, flags: 0b1010, last: 1755870001 },
            IndexEntry { name: Vec::new(), id: 999, level: 1, flags: 0, last: 0 }, // skipped
        ];
        save_index(&lib, &entries).unwrap();
        let raw = std::fs::read(lib.join("plrfiles/index")).unwrap();
        // Names lowercased; flags 0 -> "0", bits 1+3 -> "bd"; empty name skipped.
        assert_eq!(
            raw,
            lines(&[b"42 bob 5 0 1755870000", b"100 zeus 34 bd 1755870001", b"~"])
        );

        let (loaded, top_idnum) = load_index(&lib).unwrap();
        assert_eq!(top_idnum, 100);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], IndexEntry { name: b"bob".to_vec(), id: 42, level: 5, flags: 0, last: 1755870000 });
        assert_eq!(loaded[1], IndexEntry { name: b"zeus".to_vec(), id: 100, level: 34, flags: 0b1010, last: 1755870001 });

        // Round trip: saving the loaded entries reproduces the bytes.
        save_index(&lib, &loaded).unwrap();
        assert_eq!(std::fs::read(lib.join("plrfiles/index")).unwrap(), raw);

        // Missing index -> None ("No player index file!").
        let empty = temp_lib("noindex");
        assert!(load_index(&empty).is_none());
    }
}
