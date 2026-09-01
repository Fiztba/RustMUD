//! Telnet + KaVir protocol layer: descriptor buffering, negotiation,
//! `\t`-color translation, the pager, and the line editor.
//!
//! This crate owns everything between the socket and the command queue; it
//! knows nothing about the game world beyond a `CharId` opaque handle and a
//! caller-supplied color gate.

pub mod descriptor;
pub mod editor;
pub mod protocol;
pub mod telnet;
