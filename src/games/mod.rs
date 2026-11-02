//! Bethesda game definitions relevant to translation.
//!
//! This is *not* the Nexus-category catalog used by XIMOD Architect; here we
//! only need what the plugin parser and the output stage require: which games
//! we support, how their strings are stored, and which record/subrecord pairs
//! carry translatable text.

pub mod build;
pub mod detect;
pub mod dlc;

// The definitions themselves live in the shared `ximod-games` crate (S-05):
// identifiers, per-game specs and translatable-field tables.
pub use ximod_games::*;
