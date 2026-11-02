//! Bethesda plugin (.esp/.esm/.esl) parsing and translatable-string extraction.

pub mod context;
pub mod localized;
pub mod names;
pub mod reader;
pub mod record;
pub mod sctx;
pub mod tes3;
pub mod tree;
pub mod vmad;
pub mod write;

pub use localized::{StringsBundle, build_strings_files};
pub use reader::{extract_strings, parse, read_file, read_file_as, read_file_with};
pub use record::{Plugin, Record, SubRecord};
pub use tree::{PluginTree, parse_tree};
pub use write::{
    DelocalizeReport, OverrideOptions, OverrideReport, plan_override, write_delocalized_esp,
    write_full_esp, write_override_esp, write_override_esp_with,
};
