//! World-file formats (parse + write), world model, boot, zone resets.
//!
//! Stage 1 scope: parse the seven world formats and the index files, hold
//! them in the World model, and write them back in genolc-canonical form,
//! byte-matching the reference server's own saves.

pub mod boot;
pub mod lex;
pub mod model;
pub mod parse;
pub mod players;
pub mod import_binary;
pub mod rebuild;
pub mod write;
