//! Telnet protocol bytes and option values.

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const TELOPT_ECHO: u8 = 1;
pub const TELOPT_TTYPE: u8 = 24;
pub const TELOPT_NAWS: u8 = 31;
pub const TELOPT_CHARSET: u8 = 42;
pub const TELOPT_MSDP: u8 = 69;
pub const TELOPT_MSSP: u8 = 70;
pub const TELOPT_MCCP2: u8 = 86;
pub const TELOPT_MSP: u8 = 90;
pub const TELOPT_MXP: u8 = 91;
pub const TELOPT_ATCP: u8 = 200;

pub const TELQUAL_IS: u8 = 0;
pub const TELQUAL_SEND: u8 = 1;

// CHARSET subnegotiation.
pub const CHARSET_REQUEST: u8 = 1;
pub const CHARSET_ACCEPTED: u8 = 2;

// MSDP subnegotiation markers.
pub const MSDP_VAR: u8 = 1;
pub const MSDP_VAL: u8 = 2;
pub const MSDP_TABLE_OPEN: u8 = 3;
pub const MSDP_TABLE_CLOSE: u8 = 4;
pub const MSDP_ARRAY_OPEN: u8 = 5;
pub const MSDP_ARRAY_CLOSE: u8 = 6;

// MSSP subnegotiation markers.
pub const MSSP_VAR: u8 = 1;
pub const MSSP_VAL: u8 = 2;

/// echo_off: IAC WILL TELOPT_ECHO.
pub const ECHO_OFF: &[u8] = &[IAC, WILL, TELOPT_ECHO];
/// echo_on: IAC WONT TELOPT_ECHO.
pub const ECHO_ON: &[u8] = &[IAC, WONT, TELOPT_ECHO];
