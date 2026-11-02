//! Vendored, self-contained Bethesda plugin parser.
//!
//! Copied (parse path only) from XIMOD Translator so this explorer builds with
//! **no dependency on XIMOD**. The public types (`Plugin`, `Record`, `SubRecord`,
//! `Game`) keep the same shape as XIMOD's, so when the explorer is later folded
//! into XIMOD Translator as a tool this module is simply dropped in favour of
//! `ximod_translator::plugin`.
//!
//! The full parser API is kept verbatim even where milestone 1 does not use it
//! yet (per-game translatable-field tables, record flags, …); those feed the Cell
//! View and category filtering of later milestones, so dead-code is allowed here.
//! The re-exports below are the module's public API surface (used by later
//! milestones and by the eventual XIMOD integration), so unused-import is allowed.
#![allow(dead_code, unused_imports)]

pub mod archive;
pub mod category;
pub mod cellview;
pub mod games;
pub mod index;
pub mod keywords;
pub mod lang;
pub mod reader;
pub mod record;
pub mod strings;
pub mod stringtable;
pub mod tes3;
pub mod tree;

pub use cellview::CellView;
pub use games::Game;
pub use index::PluginIndex;
pub use reader::parse;
pub use record::{Plugin, Record, SubRecord};
pub use stringtable::{StringTable, StringsLoad};
