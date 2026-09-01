//! KaVir's protocol snippet v6 — telnet negotiation, TTYPE fingerprinting,
//! MSDP/MSSP/MXP/CHARSET, and the `\t` output translator.
//!
//! MCCP is not compiled in.
//!
//! The subnegotiation buffer is per-descriptor, so a subnegotiation split
//! across TCP reads survives the split.

use crate::telnet::*;

pub const SNIPPET_VERSION: i64 = 6;
pub const MUD_NAME: &[u8] = b"tbaMUD";
pub const MAX_OUTPUT_BUFFER: usize = mud_data::types::LARGE_BUFSIZE; // 24448
pub const MAX_VARIABLE_LENGTH: usize = 4096;
pub const MAX_MSDP_SIZE: usize = 100;

const S_CLEAN: &[u8] = b"\x1B[0;00m";

/// 256-color support level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Unknown,
    No,
    Sometimes,
    Yes,
}

/// MSDP variable identity. The declaration order is the LIST reply order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum Var {
    CHARACTER_NAME,
    SERVER_ID,
    SERVER_TIME,
    SNIPPET_VERSION_V,
    AFFECTS,
    ALIGNMENT,
    EXPERIENCE,
    EXPERIENCE_MAX,
    EXPERIENCE_TNL,
    HEALTH,
    HEALTH_MAX,
    LEVEL,
    RACE,
    CLASS,
    MANA,
    MANA_MAX,
    WIMPY,
    PRACTICE,
    MONEY,
    MOVEMENT,
    MOVEMENT_MAX,
    HITROLL,
    DAMROLL,
    AC,
    STR,
    INT,
    WIS,
    DEX,
    CON,
    STR_PERM,
    INT_PERM,
    WIS_PERM,
    DEX_PERM,
    CON_PERM,
    OPPONENT_HEALTH,
    OPPONENT_HEALTH_MAX,
    OPPONENT_LEVEL,
    OPPONENT_NAME,
    AREA_NAME,
    ROOM_EXITS,
    ROOM_NAME,
    ROOM_VNUM,
    WORLD_TIME,
    CLIENT_ID,
    CLIENT_VERSION,
    PLUGIN_ID,
    ANSI_COLORS,
    XTERM_256_COLORS,
    UTF_8,
    SOUND,
    MXP,
    BUTTON_1,
    BUTTON_2,
    BUTTON_3,
    BUTTON_4,
    BUTTON_5,
    GAUGE_1,
    GAUGE_2,
    GAUGE_3,
    GAUGE_4,
    GAUGE_5,
}

pub const NUM_VARS: usize = Var::GAUGE_5 as usize + 1;

pub struct VarDef {
    pub name: &'static [u8],
    pub is_string: bool,
    pub configurable: bool,
    pub write_once: bool,
    pub gui: bool,
    pub min: i64,
    pub max: i64,
    pub default_num: i64,
    pub default_str: Option<&'static [u8]>,
}

const fn num_ro() -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    (false, false, false, false, -1, -1, 0, None)
}
const fn str_ro() -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    (true, false, false, false, -1, -1, 0, None)
}
const fn boolean(x: i64) -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    (false, true, false, false, 0, 1, x, None)
}
const fn str_len(a: i64, b: i64) -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    (true, true, false, false, a, b, 0, None)
}
const fn str_once() -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    // Write-once string defaults; the length args are unused.
    (true, true, true, false, -1, -1, 0, None)
}
const fn str_gui(s: &'static [u8]) -> (bool, bool, bool, bool, i64, i64, i64, Option<&'static [u8]>) {
    (true, false, false, true, -1, -1, 0, Some(s))
}

macro_rules! vd {
    ($name:literal, $t:expr) => {{
        let (is_string, configurable, write_once, gui, min, max, default_num, default_str) = $t;
        VarDef { name: $name, is_string, configurable, write_once, gui, min, max, default_num, default_str }
    }};
}

/// VariableNameTable, index = Var as usize.
pub static VAR_TABLE: [VarDef; NUM_VARS] = [
    vd!(b"CHARACTER_NAME", str_ro()),
    vd!(b"SERVER_ID", str_ro()),
    vd!(b"SERVER_TIME", num_ro()),
    vd!(b"SNIPPET_VERSION", (false, false, false, false, -1, -1, SNIPPET_VERSION, None)),
    vd!(b"AFFECTS", str_ro()),
    vd!(b"ALIGNMENT", num_ro()),
    vd!(b"EXPERIENCE", num_ro()),
    vd!(b"EXPERIENCE_MAX", num_ro()),
    vd!(b"EXPERIENCE_TNL", num_ro()),
    vd!(b"HEALTH", num_ro()),
    vd!(b"HEALTH_MAX", num_ro()),
    vd!(b"LEVEL", num_ro()),
    vd!(b"RACE", str_ro()),
    vd!(b"CLASS", str_ro()),
    vd!(b"MANA", num_ro()),
    vd!(b"MANA_MAX", num_ro()),
    vd!(b"WIMPY", num_ro()),
    vd!(b"PRACTICE", num_ro()),
    vd!(b"MONEY", num_ro()),
    vd!(b"MOVEMENT", num_ro()),
    vd!(b"MOVEMENT_MAX", num_ro()),
    vd!(b"HITROLL", num_ro()),
    vd!(b"DAMROLL", num_ro()),
    vd!(b"AC", num_ro()),
    vd!(b"STR", num_ro()),
    vd!(b"INT", num_ro()),
    vd!(b"WIS", num_ro()),
    vd!(b"DEX", num_ro()),
    vd!(b"CON", num_ro()),
    vd!(b"STR_PERM", num_ro()),
    vd!(b"INT_PERM", num_ro()),
    vd!(b"WIS_PERM", num_ro()),
    vd!(b"DEX_PERM", num_ro()),
    vd!(b"CON_PERM", num_ro()),
    vd!(b"OPPONENT_HEALTH", num_ro()),
    vd!(b"OPPONENT_HEALTH_MAX", num_ro()),
    vd!(b"OPPONENT_LEVEL", num_ro()),
    vd!(b"OPPONENT_NAME", str_ro()),
    vd!(b"AREA_NAME", str_ro()),
    vd!(b"ROOM_EXITS", str_ro()),
    vd!(b"ROOM_NAME", str_ro()),
    vd!(b"ROOM_VNUM", num_ro()),
    vd!(b"WORLD_TIME", num_ro()),
    vd!(b"CLIENT_ID", str_once()),
    vd!(b"CLIENT_VERSION", str_once()),
    vd!(b"PLUGIN_ID", str_len(1, 40)),
    vd!(b"ANSI_COLORS", boolean(1)),
    vd!(b"XTERM_256_COLORS", boolean(0)),
    vd!(b"UTF_8", boolean(0)),
    vd!(b"SOUND", boolean(0)),
    vd!(b"MXP", boolean(0)),
    vd!(b"BUTTON_1", str_gui(b"\x05\x02Help\x02help\x06")),
    vd!(b"BUTTON_2", str_gui(b"\x05\x02Look\x02look\x06")),
    // The Score button sends "help".
    vd!(b"BUTTON_3", str_gui(b"\x05\x02Score\x02help\x06")),
    vd!(b"BUTTON_4", str_gui(b"\x05\x02Equipment\x02equipment\x06")),
    vd!(b"BUTTON_5", str_gui(b"\x05\x02Inventory\x02inventory\x06")),
    vd!(b"GAUGE_1", str_gui(b"\x05\x02Health\x02red\x02HEALTH\x02HEALTH_MAX\x06")),
    vd!(b"GAUGE_2", str_gui(b"\x05\x02Mana\x02blue\x02MANA\x02MANA_MAX\x06")),
    vd!(b"GAUGE_3", str_gui(b"\x05\x02Movement\x02green\x02MOVEMENT\x02MOVEMENT_MAX\x06")),
    vd!(b"GAUGE_4", str_gui(b"\x05\x02Exp TNL\x02yellow\x02EXPERIENCE\x02EXPERIENCE_MAX\x06")),
    vd!(b"GAUGE_5", str_gui(b"\x05\x02Opponent\x02darkred\x02OPPONENT_HEALTH\x02OPPONENT_HEALTH_MAX\x06")),
];

#[derive(Debug, Clone, Default)]
pub struct MsdpVal {
    pub report: bool,
    pub dirty: bool,
    pub value_int: i64,
    pub value_string: Option<Vec<u8>>,
}

/// Per-descriptor protocol state.
pub struct ProtocolState {
    pub write_oob: i32,
    pub iac_mode: bool,
    pub negotiated: bool,
    pub block_mxp: bool,
    pub ttype: bool,
    pub naws: bool,
    pub charset: bool,
    pub msdp: bool,
    pub atcp: bool,
    pub msp: bool,
    pub mxp: bool,
    pub mccp: bool,
    pub b256_support: Support,
    pub screen_width: i32,
    pub screen_height: i32,
    pub mxp_version: Vec<u8>,
    pub last_ttype: Option<Vec<u8>>,
    pub vars: Vec<MsdpVal>,
    /// Per-descriptor subneg accumulator (see module doc for the F2 fix).
    iac_buf: Vec<u8>,
    /// Bytes the protocol layer wants sent (negotiation responses etc.).
    /// The descriptor layer drains this through its normal Write path.
    pub out: Vec<u8>,
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolState {
    /// ProtocolCreate.
    pub fn new() -> Self {
        let vars = VAR_TABLE
            .iter()
            .map(|def| MsdpVal {
                report: false,
                dirty: false,
                value_int: if def.is_string { 0 } else { def.default_num },
                value_string: if def.is_string {
                    Some(match def.default_str {
                        Some(d) => d.to_vec(),
                        None if def.configurable => b"Unknown".to_vec(),
                        None => Vec::new(),
                    })
                } else {
                    None
                },
            })
            .collect();
        Self {
            write_oob: 0,
            iac_mode: false,
            negotiated: false,
            block_mxp: false,
            ttype: false,
            naws: false,
            charset: false,
            msdp: false,
            atcp: false,
            msp: false,
            mxp: false,
            mccp: false,
            b256_support: Support::Unknown,
            screen_width: 0,
            screen_height: 0,
            mxp_version: b"Unknown".to_vec(),
            last_ttype: None,
            vars,
            iac_buf: Vec::new(),
            out: Vec::new(),
        }
    }

    pub fn var_int(&self, v: Var) -> i64 {
        self.vars[v as usize].value_int
    }

    pub fn var_str(&self, v: Var) -> &[u8] {
        self.vars[v as usize].value_string.as_deref().unwrap_or(b"")
    }

    /// MSDPSetNumber: dirty only on change.
    pub fn set_number(&mut self, v: Var, value: i64) {
        let slot = &mut self.vars[v as usize];
        if slot.value_int != value {
            slot.value_int = value;
            slot.dirty = true;
        }
    }

    /// MSDPSetString.
    pub fn set_string(&mut self, v: Var, value: &[u8]) {
        let slot = &mut self.vars[v as usize];
        if slot.value_string.as_deref() != Some(value) {
            slot.value_string = Some(value.to_vec());
            slot.dirty = true;
        }
    }

    /// MSDPSetTable: the caller supplies the VAR/VAL
    /// pairs and the stored value is those wrapped in TABLE_OPEN/TABLE_CLOSE.
    /// An empty value is wrapped too: the variable's type must not follow its
    /// contents, or a client that indexes ROOM_EXITS as a table has the type
    /// pulled out from under it the moment a room has no visible exits.
    pub fn set_table(&mut self, v: Var, value: &[u8]) {
        let mut table = Vec::with_capacity(value.len() + 2);
        table.push(MSDP_TABLE_OPEN);
        table.extend_from_slice(value);
        table.push(MSDP_TABLE_CLOSE);
        let slot = &mut self.vars[v as usize];
        if slot.value_string.as_deref() != Some(table.as_slice()) {
            slot.value_string = Some(table);
            slot.dirty = true;
        }
    }

    /// MSDPSetArray: `set_table` with ARRAY_OPEN and
    /// ARRAY_CLOSE in place of the table pair, empty value included.
    /// AFFECTS is empty for any
    /// character with no spells running, which is most characters most of the
    /// time, so this is the empty case that actually reaches a client.
    pub fn set_array(&mut self, v: Var, value: &[u8]) {
        let mut arr = Vec::with_capacity(value.len() + 2);
        arr.push(MSDP_ARRAY_OPEN);
        arr.extend_from_slice(value);
        arr.push(MSDP_ARRAY_CLOSE);
        let slot = &mut self.vars[v as usize];
        if slot.value_string.as_deref() != Some(arr.as_slice()) {
            slot.value_string = Some(arr);
            slot.dirty = true;
        }
    }

    fn write(&mut self, data: &[u8], output_empty: bool) {
        // Write: OOB marker when buffer empty or already OOB.
        if self.write_oob > 0 || output_empty {
            self.write_oob = 2;
        }
        self.out.extend_from_slice(data);
    }
}

/// One log line the layer wants recorded (ReportBug → mudlog CMP).
pub type BugLog = Vec<String>;

/// Result of feeding one read's bytes through ProtocolInput.
pub struct InputResult {
    /// In-band bytes to append to the descriptor's inbuf.
    pub in_band: Vec<u8>,
    pub bugs: BugLog,
    /// True if the connection must drop (buffer overflow).
    pub fatal: bool,
}

/// ProtocolNegotiate: just IAC DO TTYPE.
pub fn negotiate(p: &mut ProtocolState, output_empty: bool) {
    p.write(&[IAC, DO, TELOPT_TTYPE], output_empty);
}

/// ProtocolInput. MXP-tag ESC[...z parsing from the
/// client is handled; MCCP never negotiated.
pub fn protocol_input(p: &mut ProtocolState, data: &[u8], output_empty: bool) -> InputResult {
    let mut r = InputResult { in_band: Vec::new(), bugs: Vec::new(), fatal: false };
    let mut i = 0usize;
    while i < data.len() {
        let b = data[i];
        if b == IAC && i + 1 < data.len() && data[i + 1] == IAC {
            if p.iac_mode {
                p.iac_buf.push(IAC);
            } else {
                r.in_band.push(IAC);
            }
            i += 2;
            continue;
        }
        if p.iac_mode {
            // Inside subnegotiation: collect until IAC SE.
            if b == IAC && i + 1 < data.len() && data[i + 1] == SE {
                let buf = std::mem::take(&mut p.iac_buf);
                if buf.len() >= 2 {
                    perform_subnegotiation(p, buf[0], &buf[1..], &mut r.bugs, output_empty);
                } else if buf.len() == 1 {
                    perform_subnegotiation(p, buf[0], &[], &mut r.bugs, output_empty);
                }
                p.iac_mode = false;
                i += 2;
                continue;
            }
            if b == IAC {
                // Lone IAC inside subneg with an unknown follow byte:
                // accumulate the byte.
                p.iac_buf.push(b);
                i += 1;
                continue;
            }
            if p.iac_buf.len() >= mud_data::types::MAX_RAW_INPUT_LENGTH {
                r.bugs.push("ProtocolInput: Too much incoming data to store in the buffer.\n".into());
                r.fatal = true;
                return r;
            }
            p.iac_buf.push(b);
            i += 1;
            continue;
        }
        if b == 0x1B && i + 2 < data.len() && data[i + 1] == b'[' && data[i + 2].is_ascii_digit() {
            // Client-side MXP tag: ESC [ <digit> z <tag> >.
            if i + 3 < data.len() && data[i + 3] == b'z' {
                let mut j = i + 4;
                let mut tag = Vec::new();
                let mut hit_end = false;
                while j < data.len() && tag.len() < 1000 {
                    if data[j] == b'>' {
                        hit_end = true;
                        break;
                    }
                    tag.push(data[j]);
                    j += 1;
                }
                if hit_end {
                    parse_client_mxp_tag(p, &tag, output_empty);
                    i = j + 1;
                    continue;
                }
                // No terminator in this read: drop the ESC and continue —
                // a tag fragmented across reads cannot be recovered.
                i += 1;
                continue;
            }
            r.in_band.push(b);
            i += 1;
            continue;
        }
        if b == IAC {
            match data.get(i + 1).copied() {
                Some(SB) => {
                    p.iac_mode = true;
                    p.iac_buf.clear();
                    i += 2;
                }
                Some(cmd @ (DO | DONT | WILL | WONT)) => {
                    if let Some(opt) = data.get(i + 2).copied() {
                        perform_handshake(p, cmd, opt, &mut r.bugs, output_empty);
                        i += 3;
                    } else {
                        i += 2; // truncated at read boundary; C also mis-steps here
                    }
                }
                Some(_) => i += 2,
                None => i += 1,
            }
            continue;
        }
        r.in_band.push(b);
        i += 1;
    }
    r
}

/// The `[INFO]` banner the protocol layer puts in front of its own notices.
///
/// The banner is written in `\t` colour codes, so it has to go through
/// [`protocol_output`] before it reaches the buffer. Written straight to the
/// buffer instead, those codes reach the client as text and show up on screen
/// as `[F210][ oINFO`.
///
/// A message too large to render is dropped rather than sent raw.
fn info_message(p: &mut ProtocolState, text: &[u8], output_empty: bool) {
    let mut line = b"\t[F210][\toINFO\t[F210]]\tn ".to_vec();
    line.extend_from_slice(text);
    let mut bugs: BugLog = Vec::new();
    if let Some(rendered) = protocol_output(p, &line, true, &mut bugs) {
        p.write(&rendered, output_empty);
    }
}

fn parse_client_mxp_tag(p: &mut ProtocolState, tag: &[u8], output_empty: bool) {
    // GetMxpTag: VALUE = letters/digits/dots ≤ 60,
    // optional quotes.
    fn get_tag(name: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
        let pos = tag.windows(name.len()).position(|w| w.eq_ignore_ascii_case(name))?;
        let mut j = pos + name.len();
        let mut out = Vec::new();
        if tag.get(j) == Some(&b'"') || tag.get(j) == Some(&b'\'') {
            j += 1;
        }
        while j < tag.len() && out.len() < 60 {
            let c = tag[j];
            if c.is_ascii_alphanumeric() || c == b'.' {
                out.push(c);
                j += 1;
            } else {
                break;
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    if let Some(client) = get_tag(b"CLIENT=", tag) {
        // Overwrite CLIENT_ID (harder to fake than TTYPE).
        p.vars[Var::CLIENT_ID as usize].value_string = Some(client);
    }
    if let Some(version) = get_tag(b"VERSION=", tag) {
        info_message(p, b"Receiving MXP Version From Client.\r\n", output_empty);
        p.vars[Var::CLIENT_VERSION as usize].value_string = Some(version.clone());
        let client = p.var_str(Var::CLIENT_ID).to_vec();
        let upgraded = (client.eq_ignore_ascii_case(b"MUSHCLIENT") && version.as_slice() >= b"4.02" as &[u8])
            || (client.eq_ignore_ascii_case(b"CMUD") && version.as_slice() >= b"3.04" as &[u8])
            || client.eq_ignore_ascii_case(b"ATLANTIS");
        if upgraded {
            p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
            p.b256_support = Support::Yes;
        }
    }
    if let Some(mxp) = get_tag(b"MXP=", tag) {
        p.mxp_version = mxp;
    }
}

/// Negotiate after the TTYPE answer.
fn negotiate_full(p: &mut ProtocolState, request_ttype: bool, output_empty: bool) {
    if request_ttype {
        p.write(&[IAC, SB, TELOPT_TTYPE, TELQUAL_SEND, IAC, SE], output_empty);
    }
    p.write(&[IAC, DO, TELOPT_NAWS], output_empty);
    p.write(&[IAC, DO, TELOPT_CHARSET], output_empty);
    p.write(&[IAC, WILL, TELOPT_MSDP], output_empty);
    p.write(&[IAC, WILL, TELOPT_MSSP], output_empty);
    p.write(&[IAC, DO, TELOPT_ATCP], output_empty);
    p.write(&[IAC, WILL, TELOPT_MSP], output_empty);
    p.write(&[IAC, DO, TELOPT_MXP], output_empty);
}

fn perform_handshake(
    p: &mut ProtocolState,
    cmd: u8,
    opt: u8,
    bugs: &mut BugLog,
    output_empty: bool,
) {
    match (cmd, opt) {
        (WILL, TELOPT_TTYPE) => {
            if !p.negotiated {
                p.negotiated = true;
                p.ttype = true;
                negotiate_full(p, true, output_empty);
            } else if !p.ttype {
                // Client changed its mind.
                p.ttype = true;
                p.write(&[IAC, SB, TELOPT_TTYPE, TELQUAL_SEND, IAC, SE], output_empty);
            }
        }
        (WONT, TELOPT_TTYPE) => {
            if !p.negotiated {
                p.negotiated = true;
                negotiate_full(p, false, output_empty);
            }
            p.ttype = false;
        }
        (WILL, TELOPT_NAWS) => p.naws = true,
        (WONT, TELOPT_NAWS) => p.naws = false,
        (WILL, TELOPT_CHARSET) => {
            if !p.charset {
                p.charset = true;
                // IAC SB CHARSET REQUEST " UTF-8" IAC SE.
                p.write(&[IAC, SB, TELOPT_CHARSET, CHARSET_REQUEST], output_empty);
                p.write(b" UTF-8", output_empty);
                p.write(&[IAC, SE], output_empty);
            }
        }
        (WONT, TELOPT_CHARSET) => p.charset = false,
        (DO, TELOPT_MSDP) => {
            if !p.msdp {
                p.msdp = true;
                msdp_send_pair(p, b"SERVER_ID", MUD_NAME, output_empty);
            }
        }
        (DONT, TELOPT_MSDP) => p.msdp = false,
        (DO, TELOPT_MSSP) => send_mssp(p, output_empty),
        (DO, TELOPT_MCCP2) => {
            // Stock never offers MCCP -- USING_MCCP is commented out in
            // protocol.h -- but this arm is OUTSIDE that ifdef, so a client
            // that asks for compression unprompted still gets here, still
            // sets the flag, and still reaches CompressStart, whose whole
            // body is a bug report saying it does nothing.
            p.mccp = true;
            bugs.push(
                "CompressStart() in protocol.c is being called, but it doesn't do anything!\n"
                    .into(),
            );
        }
        (DONT, TELOPT_MCCP2) => {
            p.mccp = false;
            bugs.push(
                "CompressEnd() in protocol.c is being called, but it doesn't do anything!\n"
                    .into(),
            );
        }
        (DO, TELOPT_MSP) => p.msp = true,
        (DONT, TELOPT_MSP) => p.msp = false,
        (WILL | DO, TELOPT_MXP) => {
            if !p.mxp {
                p.mxp = true;
                p.write(&[IAC, SB, TELOPT_MXP, IAC, SE], output_empty);
                p.write(b"\x1B[7z", output_empty);
                p.vars[Var::MXP as usize].value_int = 1;
            }
        }
        (WONT, TELOPT_MXP) => {
            if !p.mxp {
                // Retry the other polarity once.
                p.write(&[IAC, WILL, TELOPT_MXP], output_empty);
            } else {
                p.mxp = false;
                p.vars[Var::MXP as usize].value_int = 0;
            }
        }
        (DONT, TELOPT_MXP) => {
            p.mxp = false;
            p.vars[Var::MXP as usize].value_int = 0;
        }
        (WILL, TELOPT_ATCP) => {
            if !p.msdp && !p.atcp {
                p.atcp = true;
                msdp_send_pair(p, b"SERVER_ID", MUD_NAME, output_empty);
            }
        }
        (WONT, TELOPT_ATCP) => p.atcp = false,
        _ => {}
    }
}

fn perform_subnegotiation(
    p: &mut ProtocolState,
    option: u8,
    data: &[u8],
    bugs: &mut BugLog,
    output_empty: bool,
) {
    match option {
        TELOPT_TTYPE => perform_ttype(p, data, output_empty),
        TELOPT_NAWS => {
            if data.len() >= 4 {
                p.screen_width = ((data[0] as i32) << 8) | data[1] as i32;
                p.screen_height = ((data[2] as i32) << 8) | data[3] as i32;
            }
        }
        TELOPT_CHARSET => {
            if data.first() == Some(&CHARSET_ACCEPTED) {
                p.vars[Var::UTF_8 as usize].value_int = 1;
            }
        }
        TELOPT_MSDP => parse_msdp(p, data, bugs, output_empty),
        TELOPT_ATCP => parse_atcp(p, data, bugs, output_empty),
        _ => {}
    }
}

/// TTYPE cycling + client fingerprinting.
fn perform_ttype(p: &mut ProtocolState, data: &[u8], output_empty: bool) {
    if data.first() != Some(&(TELQUAL_IS)) {
        return;
    }
    let name: Vec<u8> = data[1..]
        .iter()
        .copied()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .take(64)
        .collect();
    // No empty-name guard: C builds an empty string, stores it as CLIENT_ID
    // and asks again, which is the cycle behaving normally.

    // First response initializes CLIENT_ID if still "Unknown". C sets its
    // stop-cycling flag inside that same branch, so an "ANSI" arriving as a
    // LATER response does not stop the cycle.
    let mut stop_cyclic = false;
    if p.var_str(Var::CLIENT_ID) == b"Unknown" {
        p.vars[Var::CLIENT_ID as usize].value_string = Some(name.clone());
        // Cyclic TTYPE locks up Windows telnet (protocol.c:1717-1733).
        if name == b"ANSI" {
            stop_cyclic = true;
        }
    }

    // RFC1091 cycle (protocol.c:1736-1774). This runs BEFORE the fingerprints
    // below and has to: those overwrite CLIENT_ID for Mudlet and DecafMUD,
    // and C compares against the value *this* response set, not the rewritten
    // one.
    //
    // The `last_ttype.is_none()` arm is the whole of it. C short-circuits on
    // `pLastTTYPE == NULL`, so the first response always asks again and the
    // CLIENT_ID comparison only starts mattering from the second. Without
    // that arm the test was always satisfied on the first response — CLIENT_ID
    // had just been initialised from that very name — so the cycle never ran
    // for any client, and everything only a later response can carry (the
    // MTTS bitmask, a `-256color` terminal name) was unreachable.
    let repeat = p.last_ttype.as_deref() == Some(name.as_slice())
        || name.as_slice() == p.var_str(Var::CLIENT_ID);
    if p.last_ttype.is_none() || !repeat {
        // Stored only here, not on every response: the response that ends the
        // cycle leaves the one before it recorded.
        p.last_ttype = Some(name.clone());
        // 256 colours by terminal name. C takes everything from the FIRST
        // hyphen and requires that to equal "-256color", so a second hyphen
        // disqualifies the name: "rxvt-unicode-256color" is not a 256-colour
        // terminal to C, however much it reads like one.
        let from_hyphen = name.iter().position(|c| *c == b'-').map(|i| &name[i..]);
        if from_hyphen.is_some_and(|s| s.eq_ignore_ascii_case(b"-256color"))
            || name.eq_ignore_ascii_case(b"xterm")
        {
            p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
            p.b256_support = Support::Yes;
        }
        if !stop_cyclic {
            p.write(&[IAC, SB, TELOPT_TTYPE, TELQUAL_SEND, IAC, SE], output_empty);
        }
    }

    let upper: Vec<u8> = name.to_ascii_uppercase();

    // The fingerprint ladder (protocol.c:1771-1849). MTTS is the head of the
    // same if/else-if chain in C, not a separate test.
    //
    // Two helpers because C asks two different questions: PrefixString for
    // "is this that client", and only inside the branch, "is there a version
    // after the name".
    fn has_prefix(name: &[u8], prefix: &[u8]) -> bool {
        name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
    }
    /// The version half of "Mudlet 1.1", or None when only the name is given.
    ///
    /// C nulled the byte after the name and read from one past it, skipping
    /// exactly one character whatever it was -- right for the space Mudlet
    /// sends, but it ate the leading digit of "Mudlet1.1" and reported ".1",
    /// which then failed the >= "1.1" test and cost that client its
    /// 256-colour flag. Both sides now skip a run of separators, which is
    /// identical for every string a real client sends.
    fn version_of(name: &[u8], prefix: &[u8]) -> Option<Vec<u8>> {
        if !has_prefix(name, prefix) {
            return None;
        }
        let v: Vec<u8> = name[prefix.len()..]
            .iter()
            .copied()
            .skip_while(|c| !c.is_ascii_alphanumeric())
            .collect();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }

    if has_prefix(&name, b"MTTS ") {
        // C: CLIENT_VERSION->ValueInt = atoi(pClientName+5), then the bits
        // are read off that same value. atoi takes a leading integer and
        // ignores the rest, and answers 0 for no digits -- so a garbage
        // MTTS response still SETS the variable, to zero.
        let n = crate::editor::parse_int_prefix(&name[5..]) as i64;
        p.vars[Var::CLIENT_VERSION as usize].value_int = n;
        if n & 1 != 0 {
            p.vars[Var::ANSI_COLORS as usize].value_int = 1;
        }
        if n & 4 != 0 {
            p.vars[Var::UTF_8 as usize].value_int = 1;
        }
        if n & 8 != 0 {
            p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
            p.b256_support = Support::Yes;
        }
    } else if has_prefix(&name, b"MUDLET") {
        // Set before the version is looked for, as in C: a bare "Mudlet"
        // still reaches this.
        p.b256_support = Support::Sometimes;
        if let Some(ver) = version_of(&name, b"MUDLET") {
            // The client's own spelling, not the upper-cased copy.
            p.vars[Var::CLIENT_ID as usize].value_string = Some(name[..6].to_vec());
            p.vars[Var::CLIENT_VERSION as usize].value_string = Some(ver.clone());
            // strcmp(version, "1.1") >= 0: a string compare, so "10.0"
            // counts as newer and "0.9" does not.
            if ver.as_slice() >= b"1.1" as &[u8] {
                p.b256_support = Support::Yes;
                p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
            }
        }
    } else if upper == b"EMACS-RINZAI" {
        p.b256_support = Support::Yes;
        p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
    } else if has_prefix(&name, b"DECAFMUD") {
        // Likewise before the version: "DecafMUD" with no version is an
        // ordinary response and still gets the flag.
        p.b256_support = Support::Yes;
        p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
        if let Some(ver) = version_of(&name, b"DECAFMUD") {
            p.vars[Var::CLIENT_ID as usize].value_string = Some(name[..8].to_vec());
            p.vars[Var::CLIENT_VERSION as usize].value_string = Some(ver);
        }
    } else if upper == b"MUSHCLIENT"
        || upper == b"CMUD"
        || upper == b"ATLANTIS"
        || upper == b"KILDCLIENT"
        || upper == b"TINTIN++"
        || upper == b"TINYFUGUE"
    {
        p.b256_support = Support::Sometimes;
    } else if upper == b"ZMUD" {
        p.b256_support = Support::No;
    }
}

/// MSDPSendPair as raw subneg or ATCP fallback.
/// MXPSendTag (protocol.c:1364): wrap a tag in the MXP secure-line escapes
/// and send it on a line of its own.
///
/// Gated on the client having MXP on and the tag being under 1000 bytes; a
/// longer one is dropped in silence rather than truncated.
///
/// Its only callers are the two places a player enters the game, both of
/// which send `<VERSION>`.
pub fn mxp_send_tag(p: &mut ProtocolState, tag: &[u8], output_empty: bool) {
    if p.vars[Var::MXP as usize].value_int == 0 || tag.len() >= 1000 {
        return;
    }
    let mut buf = b"\x1B[1z".to_vec();
    buf.extend_from_slice(tag);
    buf.extend_from_slice(b"\x1B[7z\r\n");
    p.write(&buf, output_empty);
}

pub fn msdp_send_pair(p: &mut ProtocolState, name: &[u8], value: &[u8], output_empty: bool) {
    if name.len() + value.len() + 10 > MAX_VARIABLE_LENGTH {
        return;
    }
    if p.msdp {
        let mut buf = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        buf.extend_from_slice(name);
        buf.push(MSDP_VAL);
        buf.extend_from_slice(value);
        buf.extend_from_slice(&[IAC, SE]);
        p.write(&buf, output_empty);
    } else if p.atcp {
        let mut buf = vec![IAC, SB, TELOPT_ATCP];
        buf.extend_from_slice(b"MSDP.");
        buf.extend_from_slice(name);
        buf.push(b' ');
        buf.extend_from_slice(value);
        buf.extend_from_slice(&[IAC, SE]);
        p.write(&buf, output_empty);
    }
}

fn msdp_send_var(p: &mut ProtocolState, v: Var, output_empty: bool) {
    let def = &VAR_TABLE[v as usize];
    if def.is_string {
        let val = p.var_str(v).to_vec();
        msdp_send_pair(p, def.name, &val, output_empty);
    } else {
        let val = p.var_int(v).to_string();
        msdp_send_pair(p, def.name, val.as_bytes(), output_empty);
    }
    p.vars[v as usize].dirty = false;
}

/// MSDPUpdate: flush reported+dirty variables.
pub fn msdp_update_flush(p: &mut ProtocolState, output_empty: bool) {
    for i in 0..NUM_VARS {
        if p.vars[i].report && p.vars[i].dirty {
            let v = var_from_index(i);
            msdp_send_var(p, v, output_empty);
        }
    }
}

fn var_from_index(i: usize) -> Var {
    // Safe: NUM_VARS bound, repr(usize) contiguous.
    debug_assert!(i < NUM_VARS);
    // A match would be huge; index the table through a const list instead.
    ALL_VARS[i]
}

pub static ALL_VARS: [Var; NUM_VARS] = [
    Var::CHARACTER_NAME,
    Var::SERVER_ID,
    Var::SERVER_TIME,
    Var::SNIPPET_VERSION_V,
    Var::AFFECTS,
    Var::ALIGNMENT,
    Var::EXPERIENCE,
    Var::EXPERIENCE_MAX,
    Var::EXPERIENCE_TNL,
    Var::HEALTH,
    Var::HEALTH_MAX,
    Var::LEVEL,
    Var::RACE,
    Var::CLASS,
    Var::MANA,
    Var::MANA_MAX,
    Var::WIMPY,
    Var::PRACTICE,
    Var::MONEY,
    Var::MOVEMENT,
    Var::MOVEMENT_MAX,
    Var::HITROLL,
    Var::DAMROLL,
    Var::AC,
    Var::STR,
    Var::INT,
    Var::WIS,
    Var::DEX,
    Var::CON,
    Var::STR_PERM,
    Var::INT_PERM,
    Var::WIS_PERM,
    Var::DEX_PERM,
    Var::CON_PERM,
    Var::OPPONENT_HEALTH,
    Var::OPPONENT_HEALTH_MAX,
    Var::OPPONENT_LEVEL,
    Var::OPPONENT_NAME,
    Var::AREA_NAME,
    Var::ROOM_EXITS,
    Var::ROOM_NAME,
    Var::ROOM_VNUM,
    Var::WORLD_TIME,
    Var::CLIENT_ID,
    Var::CLIENT_VERSION,
    Var::PLUGIN_ID,
    Var::ANSI_COLORS,
    Var::XTERM_256_COLORS,
    Var::UTF_8,
    Var::SOUND,
    Var::MXP,
    Var::BUTTON_1,
    Var::BUTTON_2,
    Var::BUTTON_3,
    Var::BUTTON_4,
    Var::BUTTON_5,
    Var::GAUGE_1,
    Var::GAUGE_2,
    Var::GAUGE_3,
    Var::GAUGE_4,
    Var::GAUGE_5,
];

/// ParseMSDP (protocol.c:1899-1925): walk VAR/VAL pairs.
///
/// C keeps two fixed buffers and one write cursor, and executes a pair on
/// every marker and at end of data — so one MSDP_VAR followed by several
/// MSDP_VALs is a pair per VAL, which is how a real client registers a batch
/// of REPORTs. Concatenating those VALs into one value instead loses the
/// whole batch, because the joined name matches nothing.
///
/// A marker rewinds the cursor without terminating the buffer it points at,
/// so the previous text survives until the next byte overwrites it: an
/// MSDP_VAR with nothing after it reuses the previous variable name. That
/// quirk is reproduced by clearing a buffer on the first byte written to it
/// rather than at the marker.
fn parse_msdp(p: &mut ProtocolState, data: &[u8], bugs: &mut BugLog, output_empty: bool) {
    let mut var: Vec<u8> = Vec::new();
    let mut val: Vec<u8> = Vec::new();
    // None until the first marker: bytes before one are dropped.
    let mut target: Option<bool> = None; // true = var buffer, false = val
    let mut rewound = false;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        i += 1;
        match b {
            MSDP_VAR | MSDP_VAL => {
                target = Some(b == MSDP_VAR);
                rewound = true;
            }
            c => {
                if let Some(is_var) = target {
                    let buf = if is_var { &mut var } else { &mut val };
                    if rewound {
                        buf.clear();
                        rewound = false;
                    }
                    if buf.len() < MAX_MSDP_SIZE {
                        buf.push(c);
                    }
                }
                // C only falls through to ExecuteMSDPPair when this was the
                // last byte; otherwise it continues the walk.
                if i < data.len() {
                    continue;
                }
            }
        }
        execute_msdp_pair(p, &var, &val, bugs, output_empty);
        val.clear();
    }
}

/// ParseATCP: "MSDP.NAME value".
fn parse_atcp(p: &mut ProtocolState, data: &[u8], bugs: &mut BugLog, output_empty: bool) {
    if !data.starts_with(b"MSDP.") {
        return;
    }
    let rest = &data[5..];
    let space = rest.iter().position(|c| *c == b' ').unwrap_or(rest.len());
    let (var, val) = rest.split_at(space);
    let val = val.strip_prefix(b" ").unwrap_or(val);
    let var: Vec<u8> = var.iter().copied().take(MAX_MSDP_SIZE).collect();
    let val: Vec<u8> = val.iter().copied().take(MAX_MSDP_SIZE).collect();
    execute_msdp_pair(p, &var, &val, bugs, output_empty);
}

fn find_var(name: &[u8]) -> Option<Var> {
    VAR_TABLE.iter().position(|d| d.name == name).map(var_from_index)
}

fn send_msdp_list(p: &mut ProtocolState, name: &[u8], items: &[&[u8]], output_empty: bool) {
    // MSDPSendList: IAC SB MSDP VAR name VAL ARRAY_OPEN (VAL item)* ARRAY_CLOSE IAC SE.
    let mut buf = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
    buf.extend_from_slice(name);
    buf.push(MSDP_VAL);
    buf.push(MSDP_ARRAY_OPEN);
    for item in items {
        buf.push(MSDP_VAL);
        buf.extend_from_slice(item);
    }
    buf.push(MSDP_ARRAY_CLOSE);
    buf.extend_from_slice(&[IAC, SE]);
    if p.msdp {
        p.write(&buf, output_empty);
    } else if p.atcp {
        // ATCP list: space-separated.
        let mut abuf = vec![IAC, SB, TELOPT_ATCP];
        abuf.extend_from_slice(b"MSDP.");
        abuf.extend_from_slice(name);
        abuf.push(b' ');
        let joined = items.join(&b' ');
        abuf.extend_from_slice(&joined);
        abuf.extend_from_slice(&[IAC, SE]);
        p.write(&abuf, output_empty);
    }
}

fn execute_msdp_pair(
    p: &mut ProtocolState,
    var: &[u8],
    val: &[u8],
    _bugs: &mut BugLog,
    output_empty: bool,
) {
    if var.is_empty() || val.is_empty() {
        return;
    }
    match var {
        b"SEND" => {
            if let Some(v) = find_var(val) {
                msdp_send_var(p, v, output_empty);
            }
        }
        b"REPORT" => {
            if let Some(v) = find_var(val) {
                p.vars[v as usize].report = true;
                p.vars[v as usize].dirty = true;
            }
        }
        b"UNREPORT" => {
            if let Some(v) = find_var(val) {
                p.vars[v as usize].report = false;
                p.vars[v as usize].dirty = false;
            }
        }
        b"RESET" => {
            if val == b"REPORTABLE_VARIABLES" || val == b"REPORTED_VARIABLES" {
                for slot in &mut p.vars {
                    slot.report = false;
                    slot.dirty = false;
                }
            }
        }
        b"LIST" => match val {
            b"COMMANDS" => {
                send_msdp_list(p, b"COMMANDS", &[b"LIST", b"REPORT", b"RESET", b"SEND", b"UNREPORT"], output_empty)
            }
            b"LISTS" => send_msdp_list(
                p,
                b"LISTS",
                &[
                    b"COMMANDS",
                    b"LISTS",
                    b"CONFIGURABLE_VARIABLES",
                    b"REPORTABLE_VARIABLES",
                    b"REPORTED_VARIABLES",
                    b"SENDABLE_VARIABLES",
                    b"GUI_VARIABLES",
                ],
                output_empty,
            ),
            b"SENDABLE_VARIABLES" | b"REPORTABLE_VARIABLES" => {
                // Built with a leading separator, so the wire array begins
                // with one empty element.
                let mut items: Vec<&[u8]> = vec![b""];
                for (i, def) in VAR_TABLE.iter().enumerate() {
                    let _ = i;
                    if !def.gui {
                        items.push(def.name);
                    }
                }
                send_msdp_list(p, val, &items, output_empty);
            }
            b"REPORTED_VARIABLES" => {
                let names: Vec<&[u8]> =
                    VAR_TABLE.iter().enumerate().filter(|(i, _)| p.vars[*i].report).map(|(_, d)| d.name).collect();
                send_msdp_list(p, b"REPORTED_VARIABLES", &names, output_empty);
            }
            b"CONFIGURABLE_VARIABLES" => {
                let names: Vec<&[u8]> = VAR_TABLE.iter().filter(|d| d.configurable).map(|d| d.name).collect();
                send_msdp_list(p, b"CONFIGURABLE_VARIABLES", &names, output_empty);
            }
            b"GUI_VARIABLES" => {
                let names: Vec<&[u8]> = VAR_TABLE.iter().filter(|d| d.gui).map(|d| d.name).collect();
                send_msdp_list(p, b"GUI_VARIABLES", &names, output_empty);
            }
            _ => {}
        },
        _ => {
            // Configurable variable set.
            if let Some(v) = find_var(var) {
                let def = &VAR_TABLE[v as usize];
                if !def.configurable {
                    return;
                }
                if def.write_once && p.var_str(v) != b"Unknown" {
                    return;
                }
                if def.is_string {
                    let mut s: Vec<u8> = val
                        .iter()
                        .copied()
                        .filter(|c| c.is_ascii() && !c.is_ascii_control())
                        .collect();
                    if def.min >= 0 && (s.len() as i64) < def.min {
                        return;
                    }
                    if def.max >= 0 && (s.len() as i64) > def.max {
                        s.truncate(def.max as usize);
                    }
                    p.vars[v as usize].value_string = Some(s);
                } else if let Ok(txt) = std::str::from_utf8(val) {
                    if let Ok(n) = txt.trim().parse::<i64>() {
                        if (def.min < 0 || n >= def.min) && (def.max < 0 || n <= def.max) {
                            p.vars[v as usize].value_int = n;
                            if v == Var::XTERM_256_COLORS && n == 1 {
                                p.b256_support = Support::Yes;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// SendMSSP: NAME, PLAYERS, UPTIME, CRAWL DELAY.
fn send_mssp(p: &mut ProtocolState, output_empty: bool) {
    let players = MSSP_PLAYERS.with(|c| c.get());
    let uptime = MSSP_UPTIME.with(|c| c.get());
    let mut buf = vec![IAC, SB, TELOPT_MSSP];
    let pair = |buf: &mut Vec<u8>, name: &[u8], value: &[u8]| {
        buf.push(MSSP_VAR);
        buf.extend_from_slice(name);
        buf.push(MSSP_VAL);
        buf.extend_from_slice(value);
    };
    pair(&mut buf, b"NAME", MUD_NAME);
    pair(&mut buf, b"PLAYERS", players.to_string().as_bytes());
    pair(&mut buf, b"UPTIME", uptime.to_string().as_bytes());
    pair(&mut buf, b"CRAWL DELAY", b"-1");
    buf.extend_from_slice(&[IAC, SE]);
    p.write(&buf, output_empty);
}

thread_local! {
    static MSSP_PLAYERS: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    static MSSP_UPTIME: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// MSSPSetPlayers: also stamps UPTIME on first call —
/// the "uptime is first-sample time, not boot time" quirk (study 01 §7.9).
pub fn mssp_set_players(count: i64, now: i64) {
    MSSP_PLAYERS.with(|c| c.set(count));
    MSSP_UPTIME.with(|c| {
        if c.get() == 0 {
            c.set(now);
        }
    });
}

fn unicode_get(cp: u32) -> Vec<u8> {
    let mut out = Vec::new();
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
    out
}

fn is_valid_colour(buf: &[u8]) -> bool {
    buf.len() >= 4
        && (buf[0].eq_ignore_ascii_case(&b'f') || buf[0].eq_ignore_ascii_case(&b'b'))
        && buf[1..4].iter().all(|c| (b'0'..=b'5').contains(c))
}

/// GetRGBColour: ESC[<3|4>8;5;NNNm, three digits.
fn get_rgb_colour(background: bool, r: u8, g: u8, b: u8) -> Vec<u8> {
    let val = 16 + (r as u32) * 36 + (g as u32) * 6 + b as u32;
    let mut out = Vec::with_capacity(11);
    out.extend_from_slice(b"\x1B[");
    out.push(if background { b'4' } else { b'3' });
    out.extend_from_slice(b"8;5;");
    out.push(b'0' + (val / 100) as u8);
    out.push(b'0' + ((val % 100) / 10) as u8);
    out.push(b'0' + (val % 10) as u8);
    out.push(b'm');
    out
}

/// GetAnsiColour. All backgrounds carry the spurious
/// `1;` bold attribute — preserved.
fn get_ansi_colour(background: bool, r: u8, g: u8, b: u8) -> &'static [u8] {
    if r == g && r == b && r < 2 {
        if background { b"\x1B[1;40m" } else if r >= 1 { b"\x1B[1;30m" } else { b"\x1B[0;30m" }
    } else if r == g && r == b {
        if background { b"\x1B[1;47m" } else if r >= 4 { b"\x1B[1;37m" } else { b"\x1B[0;37m" }
    } else if r > g && r > b {
        if background { b"\x1B[1;41m" } else if r > 3 { b"\x1B[1;31m" } else { b"\x1B[0;31m" }
    } else if r == g && r > b {
        if background { b"\x1B[1;43m" } else if r > 3 { b"\x1B[1;33m" } else { b"\x1B[0;33m" }
    } else if r == b && r > g {
        if background { b"\x1B[1;45m" } else if r >= 3 { b"\x1B[1;35m" } else { b"\x1B[0;35m" }
    } else if g > b {
        if background { b"\x1B[1;42m" } else if g >= 3 { b"\x1B[1;32m" } else { b"\x1B[0;32m" }
    } else if g == b {
        if background { b"\x1B[1;46m" } else if g >= 3 { b"\x1B[1;36m" } else { b"\x1B[0;36m" }
    } else if background {
        b"\x1B[1;44m"
    } else if b >= 3 {
        b"\x1B[1;34m"
    } else {
        b"\x1B[0;34m"
    }
}

/// ColourRGB. `color_allowed` is the caller-computed
/// character gate: true when there is no character, or the character's color
/// level is Complete (clr(ch, C_CMP)).
fn colour_rgb(p: &ProtocolState, color_allowed: bool, rgb: &[u8]) -> Vec<u8> {
    if p.var_int(Var::ANSI_COLORS) != 0 && color_allowed {
        if is_valid_colour(rgb) {
            let background = rgb[0].eq_ignore_ascii_case(&b'b');
            let (r, g, b) = (rgb[1] - b'0', rgb[2] - b'0', rgb[3] - b'0');
            if p.var_int(Var::XTERM_256_COLORS) != 0 {
                get_rgb_colour(background, r, g, b)
            } else {
                get_ansi_colour(background, r, g, b).to_vec()
            }
        } else {
            S_CLEAN.to_vec()
        }
    } else {
        Vec::new()
    }
}

/// ProtocolOutput. Returns None when the translated
/// output exceeds MAX_OUTPUT_BUFFER. The entire message is dropped and
/// logged.
pub fn protocol_output(
    p: &mut ProtocolState,
    data: &[u8],
    color_allowed: bool,
    bugs: &mut BugLog,
) -> Option<Vec<u8>> {
    let mut result: Vec<u8> = Vec::with_capacity(data.len() + 16);
    let use_msp = p.msp || p.var_int(Var::SOUND) != 0;
    let mut use_mxp = false;
    let mut terminate = false;
    let mut j = 0usize;

    // A fixed buffer would cap writes at MAX_OUTPUT_BUFFER; the Vec grows
    // instead and
    // compare at the end (same observable outcome: whole-message drop).
    while j < data.len() && !terminate {
        let c = data[j];
        if c == b'\t' {
            j += 1;
            let code = data.get(j).copied().unwrap_or(0);
            let mut copy: Option<Vec<u8>> = None;
            match code {
                b'\t' => copy = Some(vec![b'\t']),
                b'_' => copy = Some(b"\x1B[4m".to_vec()),
                b'+' => copy = Some(b"\x1B[1m".to_vec()),
                b'-' => copy = Some(b"\x1B[5m".to_vec()),
                b'=' => copy = Some(b"\x1B[7m".to_vec()),
                b'*' => copy = Some(vec![b'@']),
                b'1' => copy = Some(colour_rgb(p, color_allowed, b"F022")),
                b'2' => copy = Some(colour_rgb(p, color_allowed, b"F055")),
                b'3' => copy = Some(colour_rgb(p, color_allowed, b"F555")),
                b'n' => copy = Some(S_CLEAN.to_vec()),
                b'd' => copy = Some(colour_rgb(p, color_allowed, b"F000")),
                b'D' => copy = Some(colour_rgb(p, color_allowed, b"F111")),
                b'a' => copy = Some(colour_rgb(p, color_allowed, b"F021")),
                b'A' => copy = Some(colour_rgb(p, color_allowed, b"F053")),
                b'r' => copy = Some(colour_rgb(p, color_allowed, b"F200")),
                b'R' => copy = Some(colour_rgb(p, color_allowed, b"F500")),
                b'g' => copy = Some(colour_rgb(p, color_allowed, b"F020")),
                b'G' => copy = Some(colour_rgb(p, color_allowed, b"F050")),
                b'y' => copy = Some(colour_rgb(p, color_allowed, b"F330")),
                b'Y' => copy = Some(colour_rgb(p, color_allowed, b"F550")),
                b'b' => copy = Some(colour_rgb(p, color_allowed, b"F012")),
                b'B' => copy = Some(colour_rgb(p, color_allowed, b"F025")),
                b'm' => copy = Some(colour_rgb(p, color_allowed, b"F202")),
                b'M' => copy = Some(colour_rgb(p, color_allowed, b"F505")),
                b'c' => copy = Some(colour_rgb(p, color_allowed, b"F022")),
                b'C' => copy = Some(colour_rgb(p, color_allowed, b"F055")),
                b'w' => copy = Some(colour_rgb(p, color_allowed, b"F333")),
                b'W' => copy = Some(colour_rgb(p, color_allowed, b"F555")),
                b'o' => copy = Some(colour_rgb(p, color_allowed, b"F520")),
                b'O' => copy = Some(colour_rgb(p, color_allowed, b"F530")),
                b'p' => copy = Some(colour_rgb(p, color_allowed, b"F301")),
                b'P' => copy = Some(colour_rgb(p, color_allowed, b"F501")),
                b'(' => {
                    if !p.block_mxp && p.var_int(Var::MXP) != 0 {
                        copy = Some(b"\x1B[1z<send>\x1B[7z".to_vec());
                    }
                }
                b')' => {
                    if !p.block_mxp && p.var_int(Var::MXP) != 0 {
                        copy = Some(b"\x1B[1z</send>\x1B[7z".to_vec());
                    }
                    p.block_mxp = false;
                }
                b'<' => {
                    if !p.block_mxp && p.var_int(Var::MXP) != 0 {
                        copy = Some(b"\x1B[1z<".to_vec());
                        use_mxp = true;
                    } else {
                        while j < data.len() && data[j] != b'>' {
                            j += 1;
                        }
                        if j >= data.len() {
                            // Ran off the end of the data.
                            terminate = true;
                        }
                    }
                    p.block_mxp = false;
                }
                b'[' => {
                    j += 1;
                    let kind = data.get(j).copied().unwrap_or(0);
                    if kind.eq_ignore_ascii_case(&b'u') {
                        // \t[U####/ascii]
                        let mut number: u32 = 0;
                        while j + 1 < data.len() && data[j + 1].is_ascii_digit() {
                            j += 1;
                            number = number.wrapping_mul(10).wrapping_add((data[j] - b'0') as u32);
                        }
                        j += 1; // move past last digit (or onto '/'/']'/end)
                        if data.get(j) == Some(&b'/') {
                            j += 1;
                        }
                        let mut buffer: Vec<u8> = Vec::new();
                        let mut done = false;
                        let mut valid = true;
                        while j < data.len() && !done {
                            if data[j] == b']' {
                                done = true;
                            } else if buffer.len() < 7 {
                                buffer.push(data[j]);
                                j += 1;
                            } else {
                                j += 1;
                                valid = false;
                            }
                        }
                        if !done {
                            bugs.push(format!(
                                "BUG: Unicode substitute '{}' wasn't terminated with ']'.\n",
                                String::from_utf8_lossy(&buffer)
                            ));
                        } else if !valid {
                            bugs.push(format!(
                                "BUG: Unicode substitute '{}' truncated.  Missing ']'?\n",
                                String::from_utf8_lossy(&buffer)
                            ));
                        } else if p.var_int(Var::UTF_8) != 0 {
                            copy = Some(unicode_get(number));
                        } else {
                            copy = Some(buffer.clone());
                        }
                        terminate = !done;
                    } else if kind.eq_ignore_ascii_case(&b'f') || kind.eq_ignore_ascii_case(&b'b') {
                        // \t[F###] / \t[B###]
                        let mut buffer: Vec<u8> = vec![data[j]];
                        j += 1;
                        let mut done = false;
                        let mut valid = true;
                        while j < data.len() && !done && valid {
                            if data[j] == b']' {
                                done = true;
                            } else if buffer.len() < 4 {
                                buffer.push(data[j]);
                                j += 1;
                            } else {
                                valid = false;
                            }
                        }
                        if !done || !valid {
                            bugs.push(format!(
                                "BUG: RGB {}ground colour '{}' wasn't terminated with ']'.\n",
                                if buffer[0].eq_ignore_ascii_case(&b'f') { "fore" } else { "back" },
                                String::from_utf8_lossy(&buffer[1..])
                            ));
                        } else if !is_valid_colour(&buffer) {
                            bugs.push(format!(
                                "BUG: RGB {}ground colour '{}' invalid (each digit must be in the range 0-5).\n",
                                if buffer[0].eq_ignore_ascii_case(&b'f') { "fore" } else { "back" },
                                String::from_utf8_lossy(&buffer[1..])
                            ));
                        } else {
                            copy = Some(colour_rgb(p, color_allowed, &buffer));
                        }
                    } else if kind.eq_ignore_ascii_case(&b'x') {
                        // \t[x<version>] MXP gate.
                        j += 1;
                        let mut buffer: Vec<u8> = Vec::new();
                        let mut done = false;
                        while j < data.len() && !done {
                            if data[j] == b']' {
                                done = true;
                            } else if buffer.len() < 7 {
                                buffer.push(data[j]);
                                j += 1;
                            } else {
                                j += 1;
                            }
                        }
                        if !done {
                            bugs.push(format!(
                                "BUG: Required MXP version '{}' wasn't terminated with ']'.\n",
                                String::from_utf8_lossy(&buffer)
                            ));
                        } else if p.mxp_version == b"Unknown" || p.mxp_version.as_slice() < buffer.as_slice() {
                            p.block_mxp = true;
                        } else {
                            p.block_mxp = false;
                        }
                        terminate = !done;
                    }
                    // Unknown '[x' kinds: '[' plus one char is consumed; the
                    // loop tail
                    // advances past it below.
                }
                b'!' => copy = Some(b"!!".to_vec()),
                0 => terminate = true,
                _ => {} // both chars silently dropped
            }
            if let Some(bytes) = copy {
                result.extend_from_slice(&bytes);
            }
            if j < data.len() {
                j += 1;
            }
        } else if use_mxp && c == b'>' {
            result.extend_from_slice(b">\x1B[7z");
            use_mxp = false;
            j += 1;
        } else if use_msp && j > 0 && data[j - 1] == b'!' && c == b'!' && data[j + 1..].starts_with(b"SOUND(") {
            result.push(b'?');
            j += 1;
        } else {
            result.push(c);
            j += 1;
        }
    }

    if result.len() >= MAX_OUTPUT_BUFFER {
        bugs.push("ProtocolOutput: Too much outgoing data to store in the buffer.\n".into());
        return None;
    }
    Some(result)
}

/// CopyoverGet — persisted protocol string.
pub fn copyover_get(p: &ProtocolState) -> Vec<u8> {
    let mut s = format!("{}/{}", p.screen_width, p.screen_height).into_bytes();
    if p.ttype {
        s.push(b'T');
    }
    if p.naws {
        s.push(b'N');
    }
    if p.msdp {
        s.push(b'M');
    }
    if p.atcp {
        s.push(b'A');
    }
    if p.msp {
        s.push(b'S');
    }
    if p.mxp {
        s.push(b'X');
    }
    if p.mccp {
        s.push(b'c');
    }
    if p.var_int(Var::XTERM_256_COLORS) != 0 {
        s.push(b'C');
    }
    if p.charset {
        s.push(b'H');
    }
    if p.var_int(Var::UTF_8) != 0 {
        s.push(b'U');
    }
    s.truncate(64);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_colors(x256: bool) -> ProtocolState {
        let mut p = ProtocolState::new();
        if x256 {
            p.vars[Var::XTERM_256_COLORS as usize].value_int = 1;
        }
        p
    }

    #[test]
    fn color_codes_render_ansi() {
        let mut p = state_with_colors(false);
        let mut bugs = Vec::new();
        let out = protocol_output(&mut p, b"\tRhi\tn", true, &mut bugs).unwrap();
        assert_eq!(out, b"\x1B[1;31mhi\x1B[0;00m");
    }

    #[test]
    fn color_codes_render_256() {
        let mut p = state_with_colors(true);
        let mut bugs = Vec::new();
        let out = protocol_output(&mut p, b"\t[F500]x", true, &mut bugs).unwrap();
        assert_eq!(out, b"\x1B[38;5;196mx");
    }

    #[test]
    fn reset_is_unconditional_but_colors_gate() {
        let mut p = state_with_colors(false);
        let mut bugs = Vec::new();
        // color_allowed=false (player color off): \tR strips, \tn stays.
        let out = protocol_output(&mut p, b"\tRred\tn", false, &mut bugs).unwrap();
        assert_eq!(out, b"red\x1B[0;00m");
    }

    #[test]
    fn mxp_links_strip_without_mxp() {
        let mut p = ProtocolState::new();
        let mut bugs = Vec::new();
        let out = protocol_output(&mut p, b"\t(Y\t)/\t(N\t)", true, &mut bugs).unwrap();
        assert_eq!(out, b"Y/N");
    }

    #[test]
    fn unknown_codes_drop_both_chars() {
        let mut p = ProtocolState::new();
        let mut bugs = Vec::new();
        let out = protocol_output(&mut p, b"a\tqb", true, &mut bugs).unwrap();
        assert_eq!(out, b"ab");
    }

    #[test]
    fn negotiation_starts_with_do_ttype_only() {
        let mut p = ProtocolState::new();
        negotiate(&mut p, true);
        assert_eq!(p.out, vec![IAC, DO, TELOPT_TTYPE]);
        p.out.clear();
        // Client answers WILL TTYPE -> full negotiate with TTYPE request first.
        let r = protocol_input(&mut p, &[IAC, WILL, TELOPT_TTYPE], false);
        assert!(r.in_band.is_empty());
        assert!(p.out.starts_with(&[IAC, SB, TELOPT_TTYPE, TELQUAL_SEND, IAC, SE]));
        assert!(p.ttype && p.negotiated);
    }

    #[test]
    fn split_subnegotiation_survives_reads() {
        // Fragments across reads must be joined.
        let mut p = ProtocolState::new();
        let _ = protocol_input(&mut p, &[IAC, SB, TELOPT_NAWS, 0, 80], false);
        let r = protocol_input(&mut p, &[0, 25, IAC, SE], false);
        assert!(!r.fatal);
        assert_eq!((p.screen_width, p.screen_height), (80, 25));
    }

    #[test]
    fn ttype_fingerprints_xterm() {
        let mut p = ProtocolState::new();
        let mut data = vec![IAC, SB, TELOPT_TTYPE, TELQUAL_IS];
        data.extend_from_slice(b"xterm-256color");
        data.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(&mut p, &data, false);
        assert_eq!(p.var_int(Var::XTERM_256_COLORS), 1);
        assert_eq!(p.var_str(Var::CLIENT_ID), b"xterm-256color");
    }

    // ---- RFC1091 TTYPE cycling ----

    fn ttype(p: &mut ProtocolState, name: &[u8]) {
        let mut data = vec![IAC, SB, TELOPT_TTYPE, TELQUAL_IS];
        data.extend_from_slice(name);
        data.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(p, &data, false);
    }

    const TTYPE_REQUEST: [u8; 6] = [IAC, SB, TELOPT_TTYPE, TELQUAL_SEND, IAC, SE];

    fn requests(p: &ProtocolState) -> usize {
        p.out.windows(TTYPE_REQUEST.len()).filter(|w| *w == TTYPE_REQUEST).count()
    }

    /// C short-circuits its cycle test on `pLastTTYPE == NULL`, so the first
    /// response always asks again — even though CLIENT_ID was just set from
    /// that same name. Missing that arm made the test always stop on the
    /// first response, so no client was ever cycled.
    #[test]
    fn ttype_first_response_always_asks_again() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"PROBE");
        assert_eq!(p.var_str(Var::CLIENT_ID), b"PROBE");
        assert_eq!(requests(&p), 1, "first TTYPE response did not re-request");
    }

    /// A repeat of the same name ends the cycle (RFC1091's end-of-list).
    #[test]
    fn ttype_repeated_response_stops_the_cycle() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"PROBE");
        p.out.clear();
        ttype(&mut p, b"PROBE");
        assert_eq!(requests(&p), 0, "repeat should have ended the cycle");
    }

    /// Back at the top of the list stops it too, and C stores the TTYPE only
    /// inside the cycle block — so the response that ends the cycle is not
    /// the one left recorded.
    #[test]
    fn ttype_wrapping_to_client_id_stops_and_leaves_the_previous_name() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"A");
        ttype(&mut p, b"B");
        assert_eq!(requests(&p), 2, "second distinct name should re-request");
        p.out.clear();
        ttype(&mut p, b"A"); // back to CLIENT_ID: end of list
        assert_eq!(requests(&p), 0, "wrap to CLIENT_ID should stop the cycle");
        assert_eq!(p.last_ttype.as_deref(), Some(b"B" as &[u8]), "last_ttype was overwritten");
    }

    /// Cyclic TTYPE locks up Windows telnet, so C suppresses the request for
    /// an "ANSI" *first* response — and only the first, because the flag
    /// lives in the branch that initialises CLIENT_ID.
    #[test]
    fn ttype_ansi_first_response_sends_no_request() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"ANSI");
        assert_eq!(requests(&p), 0, "ANSI must not be cycled");
        assert_eq!(p.last_ttype.as_deref(), Some(b"ANSI" as &[u8]));
    }

    #[test]
    fn ttype_ansi_as_a_later_response_does_not_stop_the_cycle() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"PROBE");
        p.out.clear();
        ttype(&mut p, b"ANSI");
        assert_eq!(requests(&p), 1, "a later ANSI should still be cycled past");
    }

    /// C takes everything from the FIRST hyphen and requires it to equal
    /// "-256color", so a name with a second hyphen does not qualify.
    #[test]
    fn ttype_256color_suffix_must_follow_the_first_hyphen() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"screen-256color");
        assert_eq!(p.var_int(Var::XTERM_256_COLORS), 1);

        let mut p = ProtocolState::new();
        ttype(&mut p, b"rxvt-unicode-256color");
        assert_eq!(p.var_int(Var::XTERM_256_COLORS), 0, "second hyphen must disqualify the name");
    }

    /// What the cycle is for: MTTS only ever arrives on a later response, so
    /// it was unreachable while the first response ended the cycle.
    #[test]
    fn ttype_cycle_reaches_the_mtts_bitmask() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"TINTIN++");
        assert_eq!(requests(&p), 1, "cycle must ask for the next TTYPE");
        ttype(&mut p, b"MTTS 13"); // ANSI(1) | 256(8) | UTF-8(4)
        assert_eq!(p.var_int(Var::ANSI_COLORS), 1);
        assert_eq!(p.var_int(Var::UTF_8), 1);
        assert_eq!(p.var_int(Var::XTERM_256_COLORS), 1);
        assert_eq!(p.b256_support, Support::Yes);
    }

    /// The cycle block and the fingerprints both run, in this order: the
    /// cycle asks again, and only then does the Mudlet fingerprint rewrite
    /// CLIENT_ID out from under it. (No name in the current fingerprint set
    /// makes that order observable on its own — it is pinned here because
    /// running them the other way round is what a future fingerprint would
    /// silently break.)
    #[test]
    fn ttype_cycle_and_fingerprint_both_run_on_one_response() {
        let mut p = ProtocolState::new();
        ttype(&mut p, b"Mudlet 1.1");
        assert_eq!(requests(&p), 1, "cycle did not ask again");
        assert_eq!(p.last_ttype.as_deref(), Some(b"Mudlet 1.1" as &[u8]));
        // Asserted on the two the fingerprint gets right; it also rewrites
        // CLIENT_ID, where the case does not match C — see the write-up.
        assert_eq!(p.var_str(Var::CLIENT_VERSION), b"1.1", "fingerprint did not run");
        assert_eq!(p.b256_support, Support::Yes);
    }

    // ---- B75: the room variables ----

    #[test]
    fn b75_set_table_wraps_in_table_markers() {
        let mut p = ProtocolState::new();
        let mut pairs = vec![MSDP_VAR];
        pairs.extend_from_slice(b"n");
        pairs.push(MSDP_VAL);
        pairs.extend_from_slice(b"3001");
        p.set_table(Var::ROOM_EXITS, &pairs);

        let mut want = vec![MSDP_TABLE_OPEN];
        want.extend_from_slice(&pairs);
        want.push(MSDP_TABLE_CLOSE);
        assert_eq!(p.var_str(Var::ROOM_EXITS), want.as_slice());
    }

    // ---- the character sheet: AFFECTS is the one array the MUD sends ----

    #[test]
    fn set_array_wraps_in_array_markers() {
        let mut p = ProtocolState::new();
        let mut vals = vec![MSDP_VAL];
        vals.extend_from_slice(b"sanctuary");
        vals.push(MSDP_VAL);
        vals.extend_from_slice(b"armor");
        p.set_array(Var::AFFECTS, &vals);

        let mut want = vec![MSDP_ARRAY_OPEN];
        want.extend_from_slice(&vals);
        want.push(MSDP_ARRAY_CLOSE);
        assert_eq!(p.var_str(Var::AFFECTS), want.as_slice());
    }

    #[test]
    fn empty_array_is_still_an_array() {
        // An unaffected character reports an empty array, not a bare empty
        // string. AFFECTS is an array whatever its contents, so a client that
        // walks it keeps something to walk.
        let mut p = ProtocolState::new();
        p.set_array(Var::AFFECTS, b"");
        assert_eq!(
            p.var_str(Var::AFFECTS),
            [MSDP_ARRAY_OPEN, MSDP_ARRAY_CLOSE].as_slice()
        );
    }

    #[test]
    fn b75_empty_table_is_still_a_table() {
        // A room with no visible exits reports an empty table rather than a
        // bare empty string, so a mapper has a table to index either way.
        let mut p = ProtocolState::new();
        p.set_table(Var::ROOM_EXITS, b"");
        assert_eq!(
            p.var_str(Var::ROOM_EXITS),
            [MSDP_TABLE_OPEN, MSDP_TABLE_CLOSE].as_slice()
        );
    }

    #[test]
    fn b75_table_is_dirty_only_on_change() {
        let mut p = ProtocolState::new();
        p.set_table(Var::ROOM_EXITS, b"x");
        p.vars[Var::ROOM_EXITS as usize].dirty = false;
        p.set_table(Var::ROOM_EXITS, b"x");
        assert!(!p.vars[Var::ROOM_EXITS as usize].dirty, "unchanged table redirtied");
        p.set_table(Var::ROOM_EXITS, b"y");
        assert!(p.vars[Var::ROOM_EXITS as usize].dirty, "changed table stayed clean");
    }

    #[test]
    fn msdp_report_over_the_wire_registers_and_delivers() {
        // Drive REPORT the way a real client does -- through protocol_input as
        // a subnegotiation -- rather than by setting the flag on the struct.
        let mut p = ProtocolState::new();
        p.msdp = true;
        let mut sb = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        sb.extend_from_slice(b"REPORT");
        sb.push(MSDP_VAL);
        sb.extend_from_slice(b"AFFECTS");
        sb.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(&mut p, &sb, true);
        assert!(p.vars[Var::AFFECTS as usize].report, "REPORT did not register");

        p.out.clear();
        p.set_array(Var::AFFECTS, b"");
        msdp_update_flush(&mut p, true);
        let mut want = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        want.extend_from_slice(b"AFFECTS");
        want.push(MSDP_VAL);
        want.push(MSDP_ARRAY_OPEN);
        want.push(MSDP_ARRAY_CLOSE);
        want.extend_from_slice(&[IAC, SE]);
        assert_eq!(p.out, want, "empty AFFECTS must reach the wire as an empty array");
    }

    #[test]
    fn b75_room_variables_stay_off_the_wire_until_reported() {
        // The whole reason B75 cannot move a *script* transcript: setting a
        // variable writes nothing until the client has REPORTed it, and
        // a client that has not answered negotiation never sees one.
        let mut p = ProtocolState::new();
        p.msdp = true;
        p.set_number(Var::ROOM_VNUM, 3001);
        p.set_string(Var::ROOM_NAME, b"The Temple Of Midgaard");
        msdp_update_flush(&mut p, true);
        assert!(p.out.is_empty(), "unreported room variables reached the wire");

        p.vars[Var::ROOM_VNUM as usize].report = true;
        p.vars[Var::ROOM_VNUM as usize].dirty = true;
        msdp_update_flush(&mut p, true);
        assert!(!p.out.is_empty(), "a reported room variable was not sent");
    }

    /// One MSDP_VAR followed by several MSDP_VALs is a pair per VAL in C, and
    /// it is how a client registers a batch of REPORTs in one subnegotiation.
    /// Joining the values instead matched no variable and lost the batch --
    /// the live failure this reproduces.
    #[test]
    fn msdp_one_var_with_many_vals_is_a_pair_per_val() {
        let mut p = ProtocolState::new();
        p.msdp = true;
        let mut sb = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        sb.extend_from_slice(b"REPORT");
        for name in [b"AFFECTS" as &[u8], b"LEVEL", b"ROOM_NAME"] {
            sb.push(MSDP_VAL);
            sb.extend_from_slice(name);
        }
        sb.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(&mut p, &sb, true);
        for v in [Var::AFFECTS, Var::LEVEL, Var::ROOM_NAME] {
            assert!(p.vars[v as usize].report, "{:?} was not registered by the batch", v);
        }

        // SEND answers each value too, in the order they arrived.
        p.out.clear();
        p.set_number(Var::LEVEL, 7);
        p.set_string(Var::SERVER_ID, MUD_NAME);
        let mut sb = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        sb.extend_from_slice(b"SEND");
        sb.push(MSDP_VAL);
        sb.extend_from_slice(b"LEVEL");
        sb.push(MSDP_VAL);
        sb.extend_from_slice(b"SERVER_ID");
        sb.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(&mut p, &sb, true);
        let mut want = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        want.extend_from_slice(b"LEVEL");
        want.push(MSDP_VAL);
        want.extend_from_slice(b"7");
        want.extend_from_slice(&[IAC, SE, IAC, SB, TELOPT_MSDP, MSDP_VAR]);
        want.extend_from_slice(b"SERVER_ID");
        want.push(MSDP_VAL);
        want.extend_from_slice(b"tbaMUD");
        want.extend_from_slice(&[IAC, SE]);
        assert_eq!(p.out, want, "batched SEND did not answer every value");
    }

    /// C rewinds its write cursor on a marker without terminating the buffer,
    /// so an MSDP_VAR with no name after it keeps the previous name.
    #[test]
    fn msdp_empty_var_reuses_the_previous_name() {
        let mut p = ProtocolState::new();
        p.msdp = true;
        let mut sb = vec![IAC, SB, TELOPT_MSDP, MSDP_VAR];
        sb.extend_from_slice(b"REPORT");
        sb.push(MSDP_VAL);
        sb.extend_from_slice(b"AFFECTS");
        sb.push(MSDP_VAR); // no name follows: C still reads "REPORT"
        sb.push(MSDP_VAL);
        sb.extend_from_slice(b"LEVEL");
        sb.extend_from_slice(&[IAC, SE]);
        let _ = protocol_input(&mut p, &sb, true);
        assert!(p.vars[Var::AFFECTS as usize].report, "first pair was lost");
        assert!(p.vars[Var::LEVEL as usize].report, "empty VAR did not reuse REPORT");
    }
}
