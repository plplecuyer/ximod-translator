//! XIMOD Translator library crate.
//!
//! Houses every subsystem (formats, plugin parsing, i18n, data, UI) so it can
//! be exercised by integration tests and, in the future, reused by other tools.
//! The `main.rs` binary is a thin launcher on top of this crate.

pub mod ambiguity;
pub mod apply;
pub mod archive;
pub mod assist;
pub mod backup;
pub mod basedb;
pub mod book;
pub mod cli;
pub mod codepage;
pub mod color;
pub mod comments;
pub mod common;
pub mod compare;
pub mod concord;
pub mod conditions;
pub mod config;
pub mod data;
pub mod dialogue;
pub mod dict_edit;
pub mod dict_stack;
pub mod dicts;
pub mod diff;
pub mod doc_settings;
pub mod engine;
pub mod exclusions;
pub mod export;
pub mod filter;
pub mod fonts;
pub mod fuzzy;
pub mod games;
pub mod headless;
pub mod history;
pub mod i18n;
pub mod icon;
pub mod icons;
pub mod instant;
pub mod journal;
pub mod l10n;
pub mod lang;
pub mod langdetect;
pub mod length;
pub mod locres;
pub mod lstrings;
pub mod manifest;
pub mod manual;
pub mod marks;
pub mod matrices;
pub mod mod_dict;
pub mod normalize;
pub mod numstyle;
pub mod online;
pub mod packaging;
pub mod pex;
pub mod plugin;
pub mod plugin_id;
pub mod prepare;
pub mod propagate;
pub mod propagate_files;
pub mod provenance;
pub mod quality;
pub mod release_check;
pub mod remember;
pub mod replace;
pub mod review;
pub mod savepart;
pub mod scan;
pub mod script_rules;
pub mod scripts;
pub mod search;
pub mod shortcuts;
pub mod sort;
pub mod spell;
pub mod splash;
pub mod status;
pub mod tags;
pub mod text;
pub mod textfmt;
pub mod tm;
pub mod ui;
pub mod undo;
pub mod update;
pub mod vmad_lock;
pub mod voices;

use std::path::PathBuf;

/// Threshold, in milliseconds, above which an operation that leaves the
/// process (a file, the clipboard, a dialog) is logged as slow; 0 logs
/// nothing. Set with the frame log (`[Diagnostics] FrameLog`).
pub static SLOW_IO_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Run `f`, logging « slow io <name>: N ms » when it exceeds the threshold
/// (a diagnostic: where the time goes on a machine that stalls).
pub fn slow_io<R>(name: &str, f: impl FnOnce() -> R) -> R {
    let limit = SLOW_IO_MS.load(std::sync::atomic::Ordering::Relaxed);
    if limit == 0 {
        return f();
    }
    let started = std::time::Instant::now();
    let r = f();
    let ms = started.elapsed().as_millis() as u64;
    if ms >= limit {
        tracing::warn!("slow io {name}: {ms} ms");
    }
    r
}

/// Application name.
pub const APP_NAME: &str = "XIMOD Translator";
/// Application version (from Cargo).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default window size.
pub const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;

/// Something the launch wants the user to know once the window is up (the
/// game sheets are read before the interface exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupNotice {
    /// A user field file could not be read; the shipped table is used (R-A06-07).
    FieldsFallback { game: String, path: String },
}

static STARTUP_NOTICES: std::sync::Mutex<Vec<StartupNotice>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn push_startup_notice(n: StartupNotice) {
    if let Ok(mut v) = STARTUP_NOTICES.lock() {
        v.push(n);
    }
}

/// Drain the notices queued by the startup worker (the UI shows them once).
pub fn take_startup_notices() -> Vec<StartupNotice> {
    STARTUP_NOTICES
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Primary-screen information (kept minimal; passed to the app for future use).
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub width: f32,
    pub height: f32,
    pub dpi: f32,
}

impl Default for ScreenInfo {
    fn default() -> Self {
        Self {
            width: 1920.0,
            height: 1080.0,
            dpi: 96.0,
        }
    }
}

/// Locate `assets/images/splash.png` relative to the executable (with a couple
/// of fallbacks for dev runs and the macOS bundle).
pub fn get_splash_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))?;

    let candidates = [
        exe_dir.join("assets").join("images").join("splash.png"),
        exe_dir.join("splash.png"),
        PathBuf::from("assets").join("images").join("splash.png"),
    ];
    if let Some(c) = candidates.into_iter().find(|c| c.exists()) {
        return Some(c);
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(resources) = exe_dir.parent().map(|p| p.join("Resources")) {
            let p = resources.join("assets").join("images").join("splash.png");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}
