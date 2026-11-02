//! `ximod-fomod` — what XIMOD Architect and XIMOD Translator share about
//! FOMOD installers (F-03):
//!
//! - [`model`]: the installer model (`Ximod`, steps, groups, options…);
//! - [`xml`]: reading and writing `info.xml` / `ModuleConfig.xml`, the
//!   validation tree and the **lossless patcher** (`xml::patch`) that changes
//!   only the targeted strings of a third-party FOMOD;
//! - [`translate`]: the translation document (`.ximod-translation` sidecar),
//!   its statuses, the translation memory, the glossary and the checks;
//! - [`package`]: reading a FOMOD's sources, extracting / updating / applying
//!   a translation (sibling, in place, package) and building the archive;
//! - [`export`] / [`archive`]: zip / 7z writers used by the packages;
//! - [`archive_open`]: extracting the `fomod/` folder of a mod archive.
//!
//! The crate knows nothing of any interface or configuration: the host
//! application names itself with [`set_tool`] (generator comments in the
//! XML written) and gives the folder of the translation memory with
//! [`set_data_dir`].

pub mod archive;
pub mod archive_open;
pub mod export;
pub mod lang;
pub mod model;
pub mod package;
pub mod translate;
pub mod xml;

use std::path::PathBuf;
use std::sync::RwLock;

/// Name, version and URL the host application signs its XML files with.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub version: String,
    pub url: String,
}

static TOOL: RwLock<Option<Tool>> = RwLock::new(None);
static DATA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Name the host application (once, at start-up).
pub fn set_tool(name: &str, version: &str, url: &str) {
    *TOOL.write().unwrap_or_else(|e| e.into_inner()) = Some(Tool {
        name: name.to_string(),
        version: version.to_string(),
        url: url.to_string(),
    });
}

/// The host application, or a neutral « XIMOD » signature.
pub fn tool() -> Tool {
    TOOL.read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| Tool {
            name: "XIMOD".into(),
            version: String::new(),
            url: String::new(),
        })
}

/// Where the translation memory and the glossaries live (the host's
/// configuration folder); `None` disables them.
pub fn set_data_dir(dir: Option<PathBuf>) {
    *DATA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Folders the packages never ship (Architect's backups and scenarios).
pub const BACKUPS_DIR: &str = "backups";
pub const SCENARIOS_DIR: &str = "scenarios";
