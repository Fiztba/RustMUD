//! Pure data layer for RustMUD: constants, flag sets, numeric tables, RNG.
//!
//! No I/O, no game state — this crate is consumed by every other layer and
//! is where the generated numeric tables live.

pub mod crypt;
pub mod flags;
pub mod ids;
pub mod rng;
pub mod spells;
pub mod tables;
pub mod types;
