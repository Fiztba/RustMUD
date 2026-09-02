//! The command interpreter (interpreter.c): the master command table in C
//! order (order IS the spec — abbreviation resolution walks it), the social
//! merge, dispatch with all gates, and the parsing utilities.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::game::Game;

pub type BStr = Vec<u8>;
pub type CmdFn = fn(&mut Game, CharId, &[u8], usize, i32);

#[derive(Clone, Copy)]
pub enum Handler {
    Fn(CmdFn),
    /// do_action — socials; pass 1 of the matcher skips these.
    Action,
    /// RESERVED slot 0.
    None,
}

#[derive(Clone)]
pub struct CommandEntry {
    pub command: BStr,
    pub sort_as: BStr,
    pub minimum_position: u8,
    pub handler: Handler,
    pub minimum_level: u8,
    pub subcmd: i32,
    /// Index into g.socials for merged socials.
    pub social: Option<usize>,
}

pub const RESERVE_CMDS: usize = 7;

// Direction subcmds equal the direction constants.
pub const SCMD_NORTH: i32 = NORTH as i32;
pub const SCMD_EAST: i32 = EAST as i32;
pub const SCMD_SOUTH: i32 = SOUTH as i32;
pub const SCMD_WEST: i32 = WEST as i32;
pub const SCMD_UP: i32 = UP as i32;
pub const SCMD_DOWN: i32 = DOWN as i32;
pub const SCMD_NW: i32 = NORTHWEST as i32;
pub const SCMD_NE: i32 = NORTHEAST as i32;
pub const SCMD_SE: i32 = SOUTHEAST as i32;
pub const SCMD_SW: i32 = SOUTHWEST as i32;

// Command subcmds.
pub const SCMD_HOLLER: i32 = 0;
pub const SCMD_SHOUT: i32 = 1;
pub const SCMD_GOSSIP: i32 = 2;
pub const SCMD_AUCTION: i32 = 3;
pub const SCMD_GRATZ: i32 = 4;
pub const SCMD_GEMOTE: i32 = 5;
pub const SCMD_QSAY: i32 = 0;
pub const SCMD_QECHO: i32 = 1;
pub const SCMD_WHISPER: i32 = 0;
pub const SCMD_ASK: i32 = 1;
pub const SCMD_COMMANDS: i32 = 0;
pub const SCMD_SOCIALS: i32 = 1;
pub const SCMD_INFO: i32 = 0;
pub const SCMD_HANDBOOK: i32 = 1;
pub const SCMD_CREDITS: i32 = 2;
pub const SCMD_NEWS: i32 = 3;
pub const SCMD_WIZLIST: i32 = 4;
pub const SCMD_POLICIES: i32 = 5;
pub const SCMD_VERSION: i32 = 6;
pub const SCMD_IMMLIST: i32 = 7;
pub const SCMD_MOTD: i32 = 8;
pub const SCMD_IMOTD: i32 = 9;
pub const SCMD_CLEAR: i32 = 10;
pub const SCMD_WHOAMI: i32 = 11;
pub const SCMD_LOOK: i32 = 0;
pub const SCMD_READ: i32 = 1;
pub const SCMD_DROP: i32 = 0;
pub const SCMD_JUNK: i32 = 1;
pub const SCMD_DONATE: i32 = 2;
pub const SCMD_EAT: i32 = 0;
pub const SCMD_TASTE: i32 = 1;
pub const SCMD_DRINK: i32 = 2;
pub const SCMD_SIP: i32 = 3;
pub const SCMD_POUR: i32 = 0;
pub const SCMD_FILL: i32 = 1;
pub const SCMD_OPEN: i32 = 0;
pub const SCMD_CLOSE: i32 = 1;
pub const SCMD_UNLOCK: i32 = 2;
pub const SCMD_LOCK: i32 = 3;
pub const SCMD_PICK: i32 = 4;
pub const SCMD_HIT: i32 = 0;
pub const SCMD_NOSUMMON: i32 = 0;
pub const SCMD_NOHASSLE: i32 = 1;
pub const SCMD_BRIEF: i32 = 2;
pub const SCMD_COMPACT: i32 = 3;
pub const SCMD_NOTELL: i32 = 4;
pub const SCMD_NOAUCTION: i32 = 5;
pub const SCMD_NOSHOUT: i32 = 6;
pub const SCMD_NOGOSSIP: i32 = 7;
pub const SCMD_NOGRATZ: i32 = 8;
pub const SCMD_NOWIZ: i32 = 9;
pub const SCMD_QUEST: i32 = 10;
pub const SCMD_SHOWVNUMS: i32 = 11;
pub const SCMD_NOREPEAT: i32 = 12;
pub const SCMD_HOLYLIGHT: i32 = 13;
pub const SCMD_SLOWNS: i32 = 14;
pub const SCMD_AUTOEXIT: i32 = 15;
pub const SCMD_TRACK: i32 = 16;
pub const SCMD_CLS: i32 = 17;
pub const SCMD_BUILDWALK: i32 = 18;
pub const SCMD_AFK: i32 = 19;
pub const SCMD_AUTOLOOT: i32 = 20;
pub const SCMD_AUTOGOLD: i32 = 21;
pub const SCMD_AUTOSPLIT: i32 = 22;
pub const SCMD_AUTOSAC: i32 = 23;
pub const SCMD_AUTOASSIST: i32 = 24;
pub const SCMD_AUTOMAP: i32 = 25;
pub const SCMD_AUTOKEY: i32 = 26;
pub const SCMD_AUTODOOR: i32 = 27;
pub const SCMD_ZONERESETS: i32 = 28;
pub const SCMD_SYSLOG: i32 = 29;
pub const SCMD_WIMPY: i32 = 30;
pub const SCMD_PAGELENGTH: i32 = 31;
pub const SCMD_SCREENWIDTH: i32 = 32;
pub const SCMD_COLOR: i32 = 33;
pub const SCMD_QUI: i32 = 0;
pub const SCMD_QUIT: i32 = 1;
pub const SCMD_USE: i32 = 0;
pub const SCMD_QUAFF: i32 = 1;
pub const SCMD_RECITE: i32 = 2;
pub const SCMD_DATE: i32 = 0;
pub const SCMD_UPTIME: i32 = 1;
pub const SCMD_ECHO: i32 = 0;
pub const SCMD_EMOTE: i32 = 1;
pub const SCMD_SHUTDOW: i32 = 0;
pub const SCMD_SHUTDOWN: i32 = 1;
pub const SCMD_REROLL: i32 = 0;
pub const SCMD_PARDON: i32 = 1;
pub const SCMD_NOTITLE: i32 = 2;
pub const SCMD_MUTE: i32 = 3;
pub const SCMD_FREEZE: i32 = 4;
pub const SCMD_THAW: i32 = 5;
pub const SCMD_UNAFFECT: i32 = 6;
pub const SCMD_BUG: i32 = 0;
pub const SCMD_IDEA: i32 = 1;
pub const SCMD_TYPO: i32 = 2;
pub const SCMD_OASIS_RLIST: i32 = 0;
pub const SCMD_OASIS_MLIST: i32 = 1;
pub const SCMD_OASIS_OLIST: i32 = 2;
pub const SCMD_OASIS_SLIST: i32 = 3;
pub const SCMD_OASIS_ZLIST: i32 = 4;
pub const SCMD_OASIS_TLIST: i32 = 5;
pub const SCMD_OASIS_QLIST: i32 = 6;

const POS_DEAD_C: u8 = POS_DEAD;
const POS_RESTING_C: u8 = POS_RESTING;
const POS_SITTING_C: u8 = POS_SITTING;
const POS_FIGHTING_C: u8 = POS_FIGHTING;
const POS_STANDING_C: u8 = POS_STANDING;
const POS_SLEEPING_C: u8 = POS_SLEEPING;

/// The static command table, in table order. The
/// terminator row is implicit (we iterate by length).
pub fn base_command_table() -> Vec<CommandEntry> {
    use crate::act::informative::*;
    use crate::act::movement::*;
    use crate::act::comm::*;
    use crate::act::item::*;
    use crate::act::other::*;
    use crate::act::wizard::*;
    use crate::act::wizset::do_set;
    use crate::act::wizshow::do_show;
    use crate::act::wizstat::{do_stat, do_vstat};
    use crate::ban::{do_ban, do_unban};

    let mut t: Vec<CommandEntry> = Vec::with_capacity(280);
    let mut row = |command: &[u8], sort_as: &[u8], pos: u8, h: Handler, level: u8, subcmd: i32| {
        t.push(CommandEntry {
            command: command.to_vec(),
            sort_as: sort_as.to_vec(),
            minimum_position: pos,
            handler: h,
            minimum_level: level,
            subcmd,
            social: None,
        });
    };
    let f = Handler::Fn;

    row(b"RESERVED", b"", 0, Handler::None, 0, 0); // 0
    row(b"north", b"n", POS_STANDING_C, f(do_move), 0, SCMD_NORTH);
    row(b"east", b"e", POS_STANDING_C, f(do_move), 0, SCMD_EAST);
    row(b"south", b"s", POS_STANDING_C, f(do_move), 0, SCMD_SOUTH);
    row(b"west", b"w", POS_STANDING_C, f(do_move), 0, SCMD_WEST);
    row(b"up", b"u", POS_STANDING_C, f(do_move), 0, SCMD_UP);
    row(b"down", b"d", POS_STANDING_C, f(do_move), 0, SCMD_DOWN);
    row(b"northwest", b"northw", POS_STANDING_C, f(do_move), 0, SCMD_NW);
    row(b"nw", b"nw", POS_STANDING_C, f(do_move), 0, SCMD_NW);
    row(b"northeast", b"northe", POS_STANDING_C, f(do_move), 0, SCMD_NE);
    row(b"ne", b"ne", POS_STANDING_C, f(do_move), 0, SCMD_NE);
    row(b"southeast", b"southe", POS_STANDING_C, f(do_move), 0, SCMD_SE);
    row(b"se", b"se", POS_STANDING_C, f(do_move), 0, SCMD_SE);
    row(b"southwest", b"southw", POS_STANDING_C, f(do_move), 0, SCMD_SW);
    row(b"sw", b"sw", POS_STANDING_C, f(do_move), 0, SCMD_SW);
    row(b"at", b"at", POS_DEAD_C, f(do_at), LVL_IMMORT, 0);
    row(b"advance", b"adv", POS_DEAD_C, f(do_advance), LVL_GRGOD, 0);
    row(b"aedit", b"aed", POS_DEAD_C, f(crate::olc::aedit::do_oasis_aedit), LVL_GOD, 0);
    row(b"alias", b"ali", POS_DEAD_C, f(do_alias), 0, 0);
    row(b"afk", b"afk", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AFK);
    row(b"areas", b"are", POS_DEAD_C, f(crate::act::informative::do_areas), 0, 0);
    row(b"assist", b"as", POS_FIGHTING_C, f(crate::act::offensive::do_assist), 1, 0);
    row(b"ask", b"ask", POS_RESTING_C, f(do_spec_comm), 0, SCMD_ASK);
    row(b"astat", b"ast", POS_DEAD_C, f(crate::olc::aedit::do_astat), 0, 0);
    row(b"attach", b"attach", POS_DEAD_C, Handler::Fn(crate::dg::commands::do_attach), LVL_BUILDER, 0);
    row(b"auction", b"auc", POS_SLEEPING_C, f(do_gen_comm), 0, SCMD_AUCTION);
    row(b"autoexits", b"autoex", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOEXIT);
    row(b"autoassist", b"autoass", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOASSIST);
    row(b"autodoor", b"autodoor", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTODOOR);
    row(b"autogold", b"autogold", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOGOLD);
    row(b"autokey", b"autokey", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOKEY);
    row(b"autoloot", b"autoloot", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOLOOT);
    row(b"automap", b"automap", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOMAP);
    row(b"autosac", b"autosac", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOSAC);
    row(b"autosplit", b"autospl", POS_DEAD_C, f(do_gen_tog), 0, SCMD_AUTOSPLIT);
    row(b"backstab", b"ba", POS_STANDING_C, f(crate::act::offensive::do_backstab), 1, 0);
    row(b"ban", b"ban", POS_DEAD_C, f(do_ban), LVL_GRGOD, 0);
    row(b"bandage", b"band", POS_RESTING_C, f(crate::act::offensive::do_bandage), 1, 0);
    row(b"balance", b"bal", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"bash", b"bas", POS_FIGHTING_C, f(crate::act::offensive::do_bash), 1, 0);
    row(b"brief", b"br", POS_DEAD_C, f(do_gen_tog), 0, SCMD_BRIEF);
    row(b"buildwalk", b"buildwalk", POS_STANDING_C, f(do_gen_tog), LVL_BUILDER, SCMD_BUILDWALK);
    row(b"buy", b"bu", POS_STANDING_C, f(do_not_here), 0, 0);
    row(b"bug", b"bug", POS_DEAD_C, f(crate::ibt::do_ibt), 0, SCMD_BUG);
    row(b"cast", b"c", POS_SITTING_C, f(crate::spell_parser::do_cast), 1, 0);
    row(b"cedit", b"cedit", POS_DEAD_C, f(crate::olc::cedit::do_oasis_cedit), LVL_IMPL, 0);
    row(b"changelog", b"cha", POS_DEAD_C, f(do_changelog), LVL_IMPL, 0);
    row(b"check", b"ch", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"checkload", b"checkl", POS_DEAD_C, f(do_checkloadstatus), LVL_GOD, 0);
    row(b"close", b"cl", POS_SITTING_C, f(do_gen_door), 0, SCMD_CLOSE);
    row(b"clear", b"cle", POS_DEAD_C, f(do_gen_ps), 0, SCMD_CLEAR);
    row(b"cls", b"cls", POS_DEAD_C, f(do_gen_ps), 0, SCMD_CLEAR);
    row(b"consider", b"con", POS_RESTING_C, f(do_consider), 0, 0);
    row(b"commands", b"com", POS_DEAD_C, f(do_commands), 0, SCMD_COMMANDS);
    row(b"compact", b"comp", POS_DEAD_C, f(do_gen_tog), 0, SCMD_COMPACT);
    row(b"copyover", b"copyover", POS_DEAD_C, f(crate::copyover::do_copyover), LVL_GRGOD, 0);
    row(b"credits", b"cred", POS_DEAD_C, f(do_gen_ps), 0, SCMD_CREDITS);
    row(b"date", b"da", POS_DEAD_C, f(do_date), LVL_IMMORT, SCMD_DATE);
    row(b"dc", b"dc", POS_DEAD_C, f(do_dc), LVL_GOD, 0);
    row(b"deposit", b"depo", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"detach", b"detach", POS_DEAD_C, Handler::Fn(crate::dg::commands::do_detach), LVL_BUILDER, 0);
    row(b"diagnose", b"diag", POS_RESTING_C, f(do_diagnose), 0, 0);
    row(b"dig", b"dig", POS_DEAD_C, f(crate::olc::copy::do_dig), LVL_BUILDER, 0);
    row(b"display", b"disp", POS_DEAD_C, f(do_display), 0, 0);
    row(b"donate", b"don", POS_RESTING_C, f(do_drop), 0, SCMD_DONATE);
    row(b"drink", b"dri", POS_RESTING_C, f(do_drink), 0, SCMD_DRINK);
    row(b"drop", b"dro", POS_RESTING_C, f(do_drop), 0, SCMD_DROP);
    row(b"eat", b"ea", POS_RESTING_C, f(do_eat), 0, SCMD_EAT);
    row(b"echo", b"ec", POS_SLEEPING_C, f(do_echo), LVL_IMMORT, SCMD_ECHO);
    row(b"emote", b"em", POS_RESTING_C, f(do_echo), 0, SCMD_EMOTE);
    row(b":", b":", POS_RESTING_C, f(do_echo), 1, SCMD_EMOTE);
    row(b"enter", b"ent", POS_STANDING_C, f(do_enter), 0, 0);
    row(b"equipment", b"eq", POS_SLEEPING_C, f(do_equipment), 0, 0);
    row(b"exits", b"ex", POS_RESTING_C, f(do_exits), 0, 0);
    row(b"examine", b"exa", POS_SITTING_C, f(do_examine), 0, 0);
    row(b"export", b"export", POS_DEAD_C, f(crate::olc::export::do_export_zone), LVL_IMPL, 0);
    row(b"force", b"force", POS_SLEEPING_C, f(do_force), LVL_GOD, 0);
    row(b"fill", b"fil", POS_STANDING_C, f(do_pour), 0, SCMD_FILL);
    row(b"file", b"file", POS_SLEEPING_C, f(do_file), LVL_GOD, 0);
    row(b"flee", b"fl", POS_FIGHTING_C, f(crate::act::offensive::do_flee), 1, 0);
    row(b"follow", b"fol", POS_RESTING_C, f(do_follow), 0, 0);
    row(b"freeze", b"freeze", POS_DEAD_C, f(do_wizutil), LVL_GRGOD, SCMD_FREEZE);
    row(b"get", b"g", POS_RESTING_C, f(do_get), 0, 0);
    row(b"gecho", b"gecho", POS_DEAD_C, f(do_gecho), LVL_GOD, 0);
    row(b"gemote", b"gem", POS_SLEEPING_C, f(do_gen_comm), 0, SCMD_GEMOTE);
    row(b"give", b"giv", POS_RESTING_C, f(do_give), 0, 0);
    row(b"goto", b"go", POS_SLEEPING_C, f(do_goto), LVL_IMMORT, 0);
    row(b"gold", b"gol", POS_RESTING_C, f(do_gold), 0, 0);
    row(b"gossip", b"gos", POS_SLEEPING_C, f(do_gen_comm), 0, SCMD_GOSSIP);
    row(b"group", b"gr", POS_RESTING_C, f(crate::act::other::do_group), 1, 0);
    row(b"grab", b"grab", POS_RESTING_C, f(do_grab), 0, 0);
    row(b"grats", b"grat", POS_SLEEPING_C, f(do_gen_comm), 0, SCMD_GRATZ);
    row(b"gsay", b"gsay", POS_SLEEPING_C, f(do_gsay), 0, 0);
    row(b"gtell", b"gt", POS_SLEEPING_C, f(do_gsay), 0, 0);
    row(b"help", b"h", POS_DEAD_C, f(do_help), 0, 0);
    row(b"happyhour", b"ha", POS_DEAD_C, f(do_happyhour), 0, 0);
    row(b"hedit", b"hedit", POS_DEAD_C, f(crate::olc::hedit::do_oasis_hedit), LVL_GOD, 0);
    row(b"helpcheck", b"helpch", POS_DEAD_C, f(crate::olc::hedit::do_helpcheck), LVL_GOD, 0);
    row(b"hide", b"hi", POS_RESTING_C, f(crate::act::other::do_hide), 1, 0);
    row(b"hindex", b"hind", POS_DEAD_C, f(do_hindex), 0, 0);
    row(b"handbook", b"handb", POS_DEAD_C, f(do_gen_ps), LVL_IMMORT, SCMD_HANDBOOK);
    row(b"hcontrol", b"hcontrol", POS_DEAD_C, f(crate::house::do_hcontrol), LVL_GRGOD, 0);
    row(b"history", b"history", POS_DEAD_C, f(do_history), 0, 0);
    row(b"hit", b"hit", POS_FIGHTING_C, f(crate::act::offensive::do_hit), 0, SCMD_HIT);
    row(b"hold", b"hold", POS_RESTING_C, f(do_grab), 1, 0);
    row(b"holler", b"holler", POS_RESTING_C, f(do_gen_comm), 1, SCMD_HOLLER);
    row(b"holylight", b"holy", POS_DEAD_C, f(do_gen_tog), LVL_IMMORT, SCMD_HOLYLIGHT);
    row(b"house", b"house", POS_RESTING_C, f(crate::house::do_house), 0, 0);
    row(b"inventory", b"i", POS_DEAD_C, f(do_inventory), 0, 0);
    row(b"identify", b"id", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"idea", b"ide", POS_DEAD_C, f(crate::ibt::do_ibt), 0, SCMD_IDEA);
    row(b"imotd", b"imo", POS_DEAD_C, f(do_gen_ps), LVL_IMMORT, SCMD_IMOTD);
    row(b"immlist", b"imm", POS_DEAD_C, f(do_gen_ps), 0, SCMD_IMMLIST);
    row(b"info", b"info", POS_SLEEPING_C, f(do_gen_ps), 0, SCMD_INFO);
    row(b"invis", b"invi", POS_DEAD_C, f(do_invis), LVL_IMMORT, 0);
    row(b"junk", b"j", POS_RESTING_C, f(do_drop), 0, SCMD_JUNK);
    row(b"kill", b"k", POS_FIGHTING_C, f(crate::act::offensive::do_kill), 0, 0);
    row(b"kick", b"ki", POS_FIGHTING_C, f(crate::act::offensive::do_kick), 1, 0);
    row(b"look", b"l", POS_RESTING_C, f(do_look), 0, SCMD_LOOK);
    row(b"last", b"last", POS_DEAD_C, f(crate::llog::do_last), LVL_GOD, 0);
    row(b"leave", b"lea", POS_STANDING_C, f(do_leave), 0, 0);
    row(b"levels", b"lev", POS_DEAD_C, f(do_levels), 0, 0);
    row(b"list", b"lis", POS_STANDING_C, f(do_not_here), 0, 0);
    row(b"links", b"lin", POS_STANDING_C, f(do_links), LVL_GOD, 0);
    row(b"lock", b"loc", POS_SITTING_C, f(do_gen_door), 0, SCMD_LOCK);
    row(b"load", b"load", POS_DEAD_C, f(do_load), LVL_BUILDER, 0);
    row(b"motd", b"motd", POS_DEAD_C, f(do_gen_ps), 0, SCMD_MOTD);
    row(b"mail", b"mail", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"map", b"map", POS_STANDING_C, f(crate::asciimap::do_map), 1, 0);
    row(b"medit", b"med", POS_DEAD_C, f(crate::olc::medit::do_oasis_medit), LVL_BUILDER, 0);
    row(b"mlist", b"mlist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_MLIST);
    row(b"mcopy", b"mcopy", POS_DEAD_C, f(crate::olc::copy::do_oasis_copy), LVL_GOD, 21);
    row(b"msgedit", b"msgedit", POS_DEAD_C, f(crate::olc::msgedit::do_msgedit), LVL_GOD, 0);
    row(b"mute", b"mute", POS_DEAD_C, f(do_wizutil), LVL_GOD, SCMD_MUTE);
    row(b"news", b"news", POS_SLEEPING_C, f(do_gen_ps), 0, SCMD_NEWS);
    row(b"noauction", b"noauction", POS_DEAD_C, f(do_gen_tog), 0, SCMD_NOAUCTION);
    row(b"nogossip", b"nogossip", POS_DEAD_C, f(do_gen_tog), 0, SCMD_NOGOSSIP);
    row(b"nograts", b"nograts", POS_DEAD_C, f(do_gen_tog), 0, SCMD_NOGRATZ);
    row(b"nohassle", b"nohassle", POS_DEAD_C, f(do_gen_tog), LVL_IMMORT, SCMD_NOHASSLE);
    row(b"norepeat", b"norepeat", POS_DEAD_C, f(do_gen_tog), 0, SCMD_NOREPEAT);
    row(b"noshout", b"noshout", POS_SLEEPING_C, f(do_gen_tog), 1, SCMD_NOSHOUT);
    row(b"nosummon", b"nosummon", POS_DEAD_C, f(do_gen_tog), 1, SCMD_NOSUMMON);
    row(b"notell", b"notell", POS_DEAD_C, f(do_gen_tog), 1, SCMD_NOTELL);
    row(b"notitle", b"notitle", POS_DEAD_C, f(do_wizutil), LVL_GOD, SCMD_NOTITLE);
    row(b"nowiz", b"nowiz", POS_DEAD_C, f(do_gen_tog), LVL_IMMORT, SCMD_NOWIZ);
    row(b"open", b"o", POS_SITTING_C, f(do_gen_door), 0, SCMD_OPEN);
    row(b"order", b"ord", POS_RESTING_C, f(crate::act::offensive::do_order), 1, 0);
    row(b"offer", b"off", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"olc", b"olc", POS_DEAD_C, f(crate::olc::do_show_save_list), LVL_BUILDER, 0);
    row(b"olist", b"olist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_OLIST);
    row(b"oedit", b"oedit", POS_DEAD_C, f(crate::olc::oedit::do_oasis_oedit), LVL_BUILDER, 0);
    row(b"oset", b"oset", POS_DEAD_C, f(do_oset), LVL_BUILDER, 0);
    row(b"ocopy", b"ocopy", POS_DEAD_C, f(crate::olc::copy::do_oasis_copy), LVL_GOD, 18);
    row(b"put", b"p", POS_RESTING_C, f(do_put), 0, 0);
    row(b"peace", b"pe", POS_DEAD_C, f(do_peace), LVL_BUILDER, 0);
    row(b"pick", b"pi", POS_STANDING_C, f(do_gen_door), 1, SCMD_PICK);
    row(b"practice", b"pr", POS_RESTING_C, f(do_practice), 1, 0);
    row(b"page", b"pag", POS_DEAD_C, f(do_page), 1, 0);
    row(b"pardon", b"pardon", POS_DEAD_C, f(do_wizutil), LVL_GOD, SCMD_PARDON);
    row(b"plist", b"plist", POS_DEAD_C, f(do_plist), LVL_GOD, 0);
    row(b"policy", b"pol", POS_DEAD_C, f(do_gen_ps), 0, SCMD_POLICIES);
    row(b"pour", b"pour", POS_STANDING_C, f(do_pour), 0, SCMD_POUR);
    row(b"prompt", b"pro", POS_DEAD_C, f(do_display), 0, 0);
    row(b"prefedit", b"pre", POS_DEAD_C, f(crate::olc::prefedit::do_oasis_prefedit), 0, 0);
    row(b"purge", b"purge", POS_DEAD_C, f(do_purge), LVL_BUILDER, 0);
    row(b"qedit", b"qedit", POS_DEAD_C, f(crate::olc::qedit::do_oasis_qedit), LVL_BUILDER, 0);
    row(b"qlist", b"qlist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_QLIST);
    row(b"quaff", b"qua", POS_RESTING_C, f(crate::act::other::do_use), 0, SCMD_QUAFF);
    row(b"qecho", b"qec", POS_DEAD_C, f(do_qcomm), LVL_GOD, SCMD_QECHO);
    row(b"quest", b"que", POS_DEAD_C, f(crate::quest::do_quest), 0, 0);
    row(b"qui", b"qui", POS_DEAD_C, f(do_quit), 0, 0);
    row(b"quit", b"quit", POS_DEAD_C, f(do_quit), 0, SCMD_QUIT);
    row(b"qsay", b"qsay", POS_RESTING_C, f(do_qcomm), 0, SCMD_QSAY);
    row(b"reply", b"r", POS_SLEEPING_C, f(do_reply), 0, 0);
    row(b"rest", b"res", POS_RESTING_C, f(do_rest), 0, 0);
    row(b"read", b"rea", POS_RESTING_C, f(do_look), 0, SCMD_READ);
    row(b"reload", b"reload", POS_DEAD_C, f(do_reboot), LVL_IMPL, 0);
    row(b"recite", b"reci", POS_RESTING_C, f(crate::act::other::do_use), 0, SCMD_RECITE);
    row(b"receive", b"rece", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"recent", b"recent", POS_DEAD_C, f(crate::llog::do_recent), LVL_IMMORT, 0);
    row(b"remove", b"rem", POS_RESTING_C, f(do_remove), 0, 0);
    row(b"rent", b"rent", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"report", b"repo", POS_RESTING_C, f(crate::act::other::do_report), 0, 0);
    row(b"reroll", b"rero", POS_DEAD_C, f(do_wizutil), LVL_GRGOD, SCMD_REROLL);
    row(b"rescue", b"resc", POS_FIGHTING_C, f(crate::act::offensive::do_rescue), 1, 0);
    row(b"restore", b"resto", POS_DEAD_C, f(do_restore), LVL_GOD, 0);
    row(b"return", b"retu", POS_DEAD_C, f(do_return), 0, 0);
    row(b"redit", b"redit", POS_DEAD_C, f(crate::olc::redit::do_oasis_redit), LVL_BUILDER, 0);
    row(b"rlist", b"rlist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_RLIST);
    row(b"rcopy", b"rcopy", POS_DEAD_C, f(crate::olc::copy::do_oasis_copy), LVL_GOD, 19);
    row(b"roomflags", b"roomflags", POS_DEAD_C, f(do_gen_tog), LVL_IMMORT, SCMD_SHOWVNUMS);
    row(b"sacrifice", b"sac", POS_RESTING_C, f(do_sac), 0, 0);
    row(b"say", b"s", POS_RESTING_C, f(do_say), 0, 0);
    row(b"score", b"sc", POS_DEAD_C, f(do_score), 0, 0);
    row(b"scan", b"sca", POS_RESTING_C, f(crate::act::informative::do_scan), 0, 0);
    row(b"scopy", b"scopy", POS_DEAD_C, f(crate::olc::copy::do_oasis_copy), LVL_GOD, 22);
    row(b"sit", b"si", POS_RESTING_C, f(do_sit), 0, 0);
    row(b"'", b"'", POS_RESTING_C, f(do_say), 0, 0);
    row(b"save", b"sav", POS_SLEEPING_C, f(do_save), 0, 0);
    row(b"saveall", b"saveall", POS_DEAD_C, f(do_saveall), LVL_BUILDER, 0);
    row(b"sell", b"sell", POS_STANDING_C, f(do_not_here), 0, 0);
    row(b"sedit", b"sedit", POS_DEAD_C, f(crate::olc::sedit::do_oasis_sedit), LVL_BUILDER, 0);
    row(b"send", b"send", POS_SLEEPING_C, f(do_send), LVL_GOD, 0);
    row(b"set", b"set", POS_DEAD_C, f(do_set), LVL_IMMORT, 0);
    row(b"shout", b"sho", POS_RESTING_C, f(do_gen_comm), 0, SCMD_SHOUT);
    row(b"show", b"show", POS_DEAD_C, f(do_show), LVL_IMMORT, 0);
    row(b"shutdow", b"shutdow", POS_DEAD_C, f(do_shutdown), LVL_IMPL, 0);
    row(b"shutdown", b"shutdown", POS_DEAD_C, f(do_shutdown), LVL_IMPL, SCMD_SHUTDOWN);
    row(b"sip", b"sip", POS_RESTING_C, f(do_drink), 0, SCMD_SIP);
    row(b"skillset", b"skillset", POS_SLEEPING_C, f(do_skillset), LVL_GRGOD, 0);
    row(b"sleep", b"sl", POS_SLEEPING_C, f(do_sleep), 0, 0);
    row(b"slist", b"slist", POS_SLEEPING_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_SLIST);
    row(b"sneak", b"sneak", POS_STANDING_C, f(crate::act::other::do_sneak), 1, 0);
    row(b"snoop", b"snoop", POS_DEAD_C, f(do_snoop), LVL_GOD, 0);
    row(b"socials", b"socials", POS_DEAD_C, f(do_commands), 0, SCMD_SOCIALS);
    row(b"split", b"split", POS_SITTING_C, f(crate::act::other::do_split), 1, 0);
    row(b"stand", b"st", POS_RESTING_C, f(do_stand), 0, 0);
    row(b"stat", b"stat", POS_DEAD_C, f(do_stat), LVL_IMMORT, 0);
    row(b"steal", b"ste", POS_STANDING_C, f(crate::act::other::do_steal), 1, 0);
    row(b"switch", b"switch", POS_DEAD_C, f(do_switch), LVL_GOD, 0);
    row(b"tell", b"t", POS_DEAD_C, f(do_tell), 0, 0);
    row(b"take", b"ta", POS_RESTING_C, f(do_get), 0, 0);
    row(b"taste", b"tas", POS_RESTING_C, f(do_eat), 0, SCMD_TASTE);
    row(b"teleport", b"tele", POS_DEAD_C, f(do_teleport), LVL_BUILDER, 0);
    row(b"tedit", b"tedit", POS_DEAD_C, f(crate::olc::tedit::do_tedit), LVL_GOD, 0);
    row(b"thaw", b"thaw", POS_DEAD_C, f(do_wizutil), LVL_GRGOD, SCMD_THAW);
    row(b"title", b"title", POS_DEAD_C, f(do_title), 0, 0);
    row(b"time", b"time", POS_DEAD_C, f(do_time), 0, 0);
    row(b"toggle", b"toggle", POS_DEAD_C, f(do_toggle), 0, 0);
    row(b"track", b"track", POS_STANDING_C, f(crate::graph::do_track), 0, 0);
    row(b"transfer", b"transfer", POS_SLEEPING_C, f(do_trans), LVL_GOD, 0);
    row(b"trigedit", b"trigedit", POS_DEAD_C, f(crate::olc::trigedit::do_oasis_trigedit), LVL_BUILDER, 0);
    row(b"typo", b"typo", POS_DEAD_C, f(crate::ibt::do_ibt), 0, SCMD_TYPO);
    row(b"tlist", b"tlist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_TLIST);
    row(b"tcopy", b"tcopy", POS_DEAD_C, f(crate::olc::copy::do_oasis_copy), LVL_GOD, 26);
    row(b"tstat", b"tstat", POS_DEAD_C, Handler::Fn(crate::dg::commands::do_tstat), LVL_BUILDER, 0);
    row(b"unlock", b"unlock", POS_SITTING_C, f(do_gen_door), 0, SCMD_UNLOCK);
    row(b"unban", b"unban", POS_DEAD_C, f(do_unban), LVL_GRGOD, 0);
    row(b"unaffect", b"unaffect", POS_DEAD_C, f(do_wizutil), LVL_GOD, SCMD_UNAFFECT);
    row(b"unfollow", b"unf", POS_RESTING_C, f(do_unfollow), 0, 0);
    row(b"uptime", b"uptime", POS_DEAD_C, f(do_date), LVL_GOD, SCMD_UPTIME);
    row(b"use", b"use", POS_SITTING_C, f(crate::act::other::do_use), 1, SCMD_USE);
    row(b"users", b"users", POS_DEAD_C, f(crate::act::informative::do_users), LVL_GOD, 0);
    row(b"value", b"val", POS_STANDING_C, f(do_not_here), 0, 0);
    row(b"version", b"ver", POS_DEAD_C, f(do_gen_ps), 0, SCMD_VERSION);
    row(b"visible", b"vis", POS_RESTING_C, f(do_visible), 1, 0);
    row(b"vnum", b"vnum", POS_DEAD_C, f(do_vnum), LVL_IMMORT, 0);
    row(b"vstat", b"vstat", POS_DEAD_C, f(do_vstat), LVL_IMMORT, 0);
    row(b"vdelete", b"vdelete", POS_DEAD_C, Handler::Fn(crate::dg::commands::do_vdelete), LVL_BUILDER, 0);
    row(b"wake", b"wake", POS_SLEEPING_C, f(do_wake), 0, 0);
    row(b"wear", b"wea", POS_RESTING_C, f(do_wear), 0, 0);
    row(b"weather", b"weather", POS_RESTING_C, f(do_weather), 0, 0);
    row(b"who", b"wh", POS_DEAD_C, f(do_who), 0, 0);
    row(b"whois", b"whoi", POS_DEAD_C, f(do_whois), 0, 0);
    row(b"whoami", b"whoami", POS_DEAD_C, f(do_gen_ps), 0, SCMD_WHOAMI);
    row(b"where", b"where", POS_RESTING_C, f(do_where), 1, 0);
    row(b"whirlwind", b"whirl", POS_FIGHTING_C, f(crate::act::offensive::do_whirlwind), 0, 0);
    row(b"whisper", b"whisper", POS_RESTING_C, f(do_spec_comm), 0, SCMD_WHISPER);
    row(b"wield", b"wie", POS_RESTING_C, f(do_wield), 0, 0);
    row(b"withdraw", b"withdraw", POS_STANDING_C, f(do_not_here), 1, 0);
    row(b"wiznet", b"wiz", POS_DEAD_C, f(do_wiznet), LVL_IMMORT, 0);
    row(b";", b";", POS_DEAD_C, f(do_wiznet), LVL_IMMORT, 0);
    row(b"wizhelp", b"wizhelp", POS_DEAD_C, f(do_commands), LVL_IMMORT, 2);
    row(b"wizlist", b"wizlist", POS_DEAD_C, f(do_gen_ps), 0, SCMD_WIZLIST);
    row(b"wizupdate", b"wizupde", POS_DEAD_C, f(do_wizupdate), LVL_GRGOD, 0);
    row(b"wizlock", b"wizlock", POS_DEAD_C, f(do_wizlock), LVL_IMPL, 0);
    row(b"write", b"write", POS_STANDING_C, f(crate::act::write::do_write), 1, 0);
    row(b"zoneresets", b"zoner", POS_DEAD_C, f(do_gen_tog), LVL_IMPL, SCMD_ZONERESETS);
    row(b"zreset", b"zreset", POS_DEAD_C, f(do_zreset), LVL_BUILDER, 0);
    row(b"zedit", b"zedit", POS_DEAD_C, f(crate::olc::zedit::do_oasis_zedit), LVL_BUILDER, 0);
    row(b"zlist", b"zlist", POS_DEAD_C, f(crate::olc::list::do_oasis_list), LVL_BUILDER, SCMD_OASIS_ZLIST);
    row(b"zlock", b"zlock", POS_DEAD_C, f(do_zlock), LVL_GOD, 0);
    row(b"zunlock", b"zunlock", POS_DEAD_C, f(do_zunlock), LVL_GOD, 0);
    row(b"zcheck", b"zcheck", POS_DEAD_C, f(crate::act::wizard::do_zcheck), LVL_BUILDER, 0);
    row(b"zpurge", b"zpurge", POS_DEAD_C, f(do_zpurge), LVL_BUILDER, 0);
    row(b"zdelete", b"zdelete", POS_DEAD_C, f(crate::act::wizard::do_zdelete), LVL_IMPL, 0);
    t
}

/// create_command_list: merge socials into the table.
pub fn create_command_list(g: &mut Game) {
    crate::social::sort_socials(&mut g.socials);
    let base = base_command_table();
    let mut complete: Vec<CommandEntry> = Vec::with_capacity(base.len() + g.socials.len() + 2);
    let mut i = 0usize;
    let mut j = 0usize;
    while i < base.len() || j < g.socials.len() {
        let take_cmd = if i >= base.len() {
            false
        } else if i < RESERVE_CMDS || j >= g.socials.len() {
            true
        } else {
            // Ties go to the command, not the social.
            crate::text::cmp_ci(&base[i].sort_as, &g.socials[j].sort_as)
                != std::cmp::Ordering::Greater
        };
        if take_cmd {
            complete.push(base[i].clone());
            i += 1;
        } else {
            let s = &mut g.socials[j];
            s.act_nr = complete.len();
            complete.push(CommandEntry {
                command: s.command.clone(),
                sort_as: s.sort_as.clone(),
                minimum_position: s.min_char_position.clamp(0, 255) as u8,
                handler: Handler::Action,
                minimum_level: s.min_level_char.clamp(0, 255) as u8,
                subcmd: 0,
                social: Some(j),
            });
            j += 1;
        }
    }
    g.log(format!("Command info rebuilt, {} total commands.", complete.len()));
    g.commands = complete;

    // Every index into the table just moved, and anything holding one has to
    // take it again. A stale index still names a command, just the wrong one,
    // so nothing reports an error -- see assign_shop_command_indices. Every
    // other command number is resolved at the point of use, so these are the
    // only ones that need retaking.
    crate::shop::assign_shop_command_indices(g);
}

/// find_command: exact match over the runtime table.
pub fn find_command(g: &Game, name: &[u8]) -> Option<usize> {
    g.commands.iter().position(|c| c.command == name)
}

/// CMD_IS: does this dispatch carry that command name?
/// Spec procs compare against the *table* name, never the player's abbrev.
pub fn cmd_is(g: &Game, cmd: usize, name: &[u8]) -> bool {
    g.commands.get(cmd).is_some_and(|c| c.command == name)
}

// parsing utilities ----

/// Whitespace except '\t' (color escape).
pub fn skip_spaces(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() && b[i] != b'\t' {
        i += 1;
    }
    &b[i..]
}

pub const FILL_WORDS: [&[u8]; 7] = [b"in", b"from", b"with", b"the", b"on", b"at", b"to"];
pub const RESERVED_WORDS: [&[u8]; 8] =
    [b"a", b"an", b"self", b"me", b"all", b"room", b"someone", b"something"];

pub fn fill_word(w: &[u8]) -> bool {
    FILL_WORDS.iter().any(|f| f.eq_ignore_ascii_case(w))
}

pub fn reserved_word(w: &[u8]) -> bool {
    RESERVED_WORDS.iter().any(|f| f.eq_ignore_ascii_case(w))
}

/// any_one_arg: one word, lowercased; returns (word, rest-at-delimiter).
pub fn any_one_arg(b: &[u8]) -> (BStr, &[u8]) {
    let b = skip_spaces(b);
    let mut i = 0;
    let mut word = Vec::new();
    while i < b.len() && !b[i].is_ascii_whitespace() {
        word.push(b[i].to_ascii_lowercase());
        i += 1;
    }
    (word, &b[i..])
}

/// one_argument: like any_one_arg but skips fill words.
pub fn one_argument(mut b: &[u8]) -> (BStr, &[u8]) {
    loop {
        let (word, rest) = any_one_arg(b);
        if !fill_word(&word) {
            return (word, rest);
        }
        b = rest;
    }
}

pub fn two_arguments(b: &[u8]) -> (BStr, BStr, &'_ [u8]) {
    let (a1, rest) = one_argument(b);
    let (a2, rest2) = one_argument(rest);
    (a1, a2, rest2)
}

/// half_chop: first word (lowercased) + remainder with leading spaces gone.
pub fn half_chop(b: &[u8]) -> (BStr, BStr) {
    let (a1, rest) = any_one_arg(b);
    (a1, skip_spaces(rest).to_vec())
}

/// one_word: like any_one_arg but keeps quoted strings
/// together.
pub fn one_word(b: &[u8]) -> (BStr, &[u8]) {
    let b = skip_spaces(b);
    let mut out = Vec::new();
    if b.first() == Some(&b'"') {
        let mut i = 1;
        while i < b.len() && b[i] != b'"' {
            out.push(b[i].to_ascii_lowercase());
            i += 1;
        }
        if i < b.len() {
            i += 1;
        }
        (out, &b[i..])
    } else {
        any_one_arg(b)
    }
}

/// is_abbrev: non-empty case-insensitive prefix.
pub fn is_abbrev_ci(arg1: &[u8], arg2: &[u8]) -> bool {
    crate::handler::is_abbrev(arg1, arg2)
}

pub fn is_number(b: &[u8]) -> bool {
    let b = if b.first() == Some(&b'-') { &b[1..] } else { b };
    !b.is_empty() && b.iter().all(|c| c.is_ascii_digit())
}

pub fn delete_doubledollar(b: &mut BStr) {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        out.push(b[i]);
        if b[i] == b'$' && b.get(i + 1) == Some(&b'$') {
            i += 2;
        } else {
            i += 1;
        }
    }
    *b = out;
}

pub fn levenshtein_distance(s1: &[u8], s2: &[u8]) -> i32 {
    let (n, m) = (s1.len(), s2.len());
    let mut d = vec![vec![0i32; m + 1]; n + 1];
    for (i, item) in d.iter_mut().enumerate().take(n + 1) {
        item[0] = i as i32;
    }
    for j in 0..=m {
        d[0][j] = j as i32;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = if s1[i - 1] == s2[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
        }
    }
    d[n][m]
}

pub fn command_interpreter(g: &mut Game, chid: CharId, argument: &[u8]) {
    // Any input, even blank, breaks hide.
    g.ch_mut(chid).affected_by.remove(flags::AFF_HIDE);

    let argument = skip_spaces(argument);
    if argument.is_empty() {
        return;
    }

    // Non-alphabetic first char: single-char command, no space needed.
    let (arg, line): (BStr, &[u8]) = if !argument[0].is_ascii_alphabetic() {
        (vec![argument[0]], &argument[1..])
    } else {
        any_one_arg(argument)
    };

    // Command triggers: world -> mob -> obj, each able to consume the
    // command.
    {
        let mut cont = crate::dg::triggers::command_wtrigger(g, chid, &arg, line);
        if !cont {
            cont = crate::dg::triggers::command_mtrigger(g, chid, &arg, line);
        }
        if !cont {
            cont = crate::dg::triggers::command_otrigger(g, chid, &arg, line);
        }
        if cont {
            return;
        }
    }
    if g.try_ch(chid).is_none() {
        return;
    }

    // Allow IMPLs to switch into mobs to test the commands.
    {
        let ch = g.ch(chid);
        if ch.is_npc() {
            if let Some(di) = ch.desc {
                let orig_lvl = g
                    .descriptors
                    .get(di)
                    .and_then(|d| d.original)
                    .and_then(|o| g.try_ch(o))
                    .map(|o| o.level);
                if orig_lvl.is_some_and(|l| l >= LVL_IMPL)
                    && crate::dg::mobcmd::script_command_interpreter(g, chid, argument)
                {
                    return;
                }
            }
        }
    }

    let level = g.ch(chid).level;
    let is_npc = g.ch(chid).is_npc();

    // Pass 1: non-social prefix match with inline level filter.
    let mut found: Option<usize> = None;
    for (idx, entry) in g.commands.iter().enumerate() {
        if matches!(entry.handler, Handler::Action) {
            continue;
        }
        if entry.command.starts_with(&arg[..]) && level >= entry.minimum_level {
            found = Some(idx);
            break;
        }
    }
    // Pass 2: socials.
    if found.is_none() {
        for (idx, entry) in g.commands.iter().enumerate() {
            if !matches!(entry.handler, Handler::Action) {
                continue;
            }
            if entry.command.starts_with(&arg[..]) && level >= entry.minimum_level {
                found = Some(idx);
                break;
            }
        }
    }

    // A zdelete report stands for exactly one command. Every command other
    // than zdelete cancels it here; zdelete cannot be cancelled here, because
    // that would cancel the confirmation it exists to allow, so do_zdelete
    // ends its own report on every path but a fresh one. Between the two, a
    // confirmation can only ever land on the zone the operator was looking at.
    let is_zdelete = found.is_some_and(|i| g.commands[i].command == b"zdelete");
    if !is_zdelete {
        let di = g.ch(chid).desc;
        let armed = di.and_then(|d| g.descriptors.get(d).and_then(|x| x.zdelete_armed));
        if let Some(z) = armed {
            if let Some(d) = di {
                if let Some(x) = g.descriptors.get_mut(d) {
                    x.zdelete_armed = None;
                }
            }
            let m = format!("The pending deletion of zone {} is cancelled.\r\n", z);
            crate::comm::send_to_char(g, chid, m.as_bytes());
        }
    }

    let Some(cmd) = found else {
        let huh = g.config.huh.clone();
        crate::comm::send_to_char(g, chid, &huh);
        // Levenshtein suggestions over the static table (socials excluded).
        let base = base_command_table();
        let mut first = true;
        let mut out: BStr = Vec::new();
        for entry in base.iter() {
            if matches!(entry.handler, Handler::Action | Handler::None) {
                continue;
            }
            if entry.command.first() != arg.first() {
                continue;
            }
            if entry.minimum_level > level {
                continue;
            }
            if levenshtein_distance(&arg, &entry.command) <= 2 {
                if first {
                    out.extend_from_slice(b"\r\nDid you mean:\r\n");
                    first = false;
                }
                // The same clickable form the help suggestions use; a <send>
                // with no href sends its text, which for a command is what a
                // click should send.
                out.extend_from_slice(b"  \t<send>");
                out.extend_from_slice(&entry.command);
                out.extend_from_slice(b"\t</send>\r\n");
            }
        }
        if !out.is_empty() {
            crate::comm::send_to_char(g, chid, &out);
        }
        return;
    };

    let entry = g.commands[cmd].clone();

    if !is_npc && g.ch(chid).plr(flags::PLR_FROZEN) && level < LVL_IMPL {
        crate::comm::send_to_char(g, chid, b"You try, but the mind-numbing cold prevents you...\r\n");
        return;
    }
    if matches!(entry.handler, Handler::None) {
        crate::comm::send_to_char(g, chid, b"Sorry, that command hasn't been implemented yet.\r\n");
        return;
    }
    if is_npc && entry.minimum_level >= LVL_IMMORT {
        crate::comm::send_to_char(g, chid, b"You can't use immortal commands while switched.\r\n");
        return;
    }
    if g.ch(chid).position < entry.minimum_position {
        let msg: &[u8] = match g.ch(chid).position {
            POS_DEAD => b"Lie still; you are DEAD!!! :-(\r\n",
            POS_INCAP | POS_MORTALLYW => b"You are in a pretty bad shape, unable to do anything!\r\n",
            POS_STUNNED => b"All you can do right now is think about the stars!\r\n",
            POS_SLEEPING => b"In your dreams, or what?\r\n",
            POS_RESTING => b"Nah... You feel too relaxed to do that..\r\n",
            POS_SITTING => b"Maybe you should get on your feet first?\r\n",
            POS_FIGHTING => b"No way!  You're fighting for your life!\r\n",
            _ => b"",
        };
        crate::comm::send_to_char(g, chid, msg);
        return;
    }

    // special chain: spec procs may consume the
    // command before the handler runs.
    if !g.no_specials && crate::spec::special(g, chid, cmd, line) {
        return;
    }
    match entry.handler {
        Handler::Fn(func) => func(g, chid, line, cmd, entry.subcmd),
        Handler::Action => crate::act::social::do_action(g, chid, line, cmd, entry.subcmd),
        Handler::None => {}
    }
}

// aliases ----

/// perform_alias: returns true when complex expansions were queued (caller
/// immediately dequeues the first).
pub fn perform_alias(g: &mut Game, di: usize, orig: &mut BStr) -> bool {
    let Some(chid) = g.descriptors.get(di).and_then(|d| d.character) else {
        return false;
    };
    if g.ch(chid).is_npc() {
        return false;
    }
    let (first, rest): (BStr, &[u8]) = any_one_arg(orig);
    if first.is_empty() {
        return false;
    }
    let alias = {
        let ps = g.ch(chid).ps();
        ps.aliases.iter().find(|a| a.alias == first).cloned()
    };
    let Some(alias) = alias else { return false };

    if alias.type_ == crate::ch::ALIAS_SIMPLE {
        *orig = alias.replacement.clone();
        // The replacement is copied over orig; the leading space survives.
        false
    } else {
        let expanded = perform_complex_alias(&alias.replacement, rest);
        match expanded {
            Some(queue) => {
                let d = g.descriptors.get_mut(di).unwrap();
                // Splice onto the FRONT of the input queue.
                for line in queue.into_iter().rev() {
                    d.input.push_front((line, true));
                }
                true
            }
            None => {
                crate::comm::send_to_char(g, chid, b"Alias expansion too long.\r\n");
                // Returns 1 with nothing queued: the caller dequeues
                // nothing.
                true
            }
        }
    }
}

fn perform_complex_alias(replacement: &[u8], args: &[u8]) -> Option<Vec<BStr>> {
    // Tokenize first 9 words of args.
    let mut tokens: Vec<BStr> = Vec::new();
    let mut rest = args;
    for _ in 0..9 {
        let (word, r) = any_one_arg_preserve_case(rest);
        if word.is_empty() {
            break;
        }
        tokens.push(word);
        rest = r;
    }

    let mut queue: Vec<BStr> = Vec::new();
    let mut current: BStr = Vec::new();
    let mut total = 0usize;
    let mut i = 0usize;
    while i < replacement.len() {
        let c = replacement[i];
        match c {
            b';' => {
                queue.push(std::mem::take(&mut current));
                i += 1;
            }
            b'$' => {
                i += 1;
                match replacement.get(i) {
                    Some(&d) => {
                        let num = d as i32 - b'1' as i32;
                        if num >= 0 && (num as usize) < tokens.len() {
                            let tok = &tokens[num as usize];
                            current.extend_from_slice(tok);
                            total += tok.len();
                        } else if d == b'*' {
                            let all = skip_spaces(args);
                            current.extend_from_slice(all);
                            total += all.len();
                        } else if d == b'$' {
                            // Redoubled for act safety.
                            current.extend_from_slice(b"$$");
                            total += 2;
                        } else {
                            // Out-of-range digit or any other char: the char
                            // itself is copied, '$' dropped.
                            current.push(d);
                            total += 1;
                        }
                        i += 1;
                    }
                    None => {}
                }
            }
            _ => {
                current.push(c);
                total += 1;
                i += 1;
            }
        }
        if total >= MAX_RAW_INPUT_LENGTH {
            return None;
        }
    }
    queue.push(current);
    Some(queue)
}

/// any_one_arg without lowercasing (alias tokens keep case; the
/// Tokenized on spaces, with case preserved.)
fn any_one_arg_preserve_case(b: &[u8]) -> (BStr, &[u8]) {
    let b = skip_spaces(b);
    let mut i = 0;
    let mut word = Vec::new();
    while i < b.len() && !b[i].is_ascii_whitespace() {
        word.push(b[i]);
        i += 1;
    }
    (word, &b[i..])
}
