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
pub mod matrices;
pub mod mod_dict;
pub mod normalize;
pub mod online;
pub mod packaging;
pub mod pex;
pub mod plugin;
pub mod plugin_id;
pub mod propagate;
pub mod propagate_files;
pub mod provenance;
pub mod quality;
pub mod replace;
pub mod review;
pub mod savepart;
pub mod scan;
pub mod script_rules;
pub mod scripts;
pub mod search;
pub mod spell;
pub mod splash;
pub mod status;
pub mod tags;
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

/// Startup work run on a background thread while the splash is shown: detect the
/// Something the startup worker wants the user to know once the window is up
/// (it runs behind the splash, before the UI exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupNotice {
    /// A user field file could not be read; the shipped table is used (R-A06-07).
    FieldsFallback { game: String, path: String },
}

static STARTUP_NOTICES: std::sync::Mutex<Vec<StartupNotice>> = std::sync::Mutex::new(Vec::new());

fn push_startup_notice(n: StartupNotice) {
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

/// installed Bethesda games (updating `Config.ini`) then (re)generate the game
/// dictionaries from their `Data/Strings` for the configured source→target pair.
/// Progress is reported into `state` (detection ≈ first 30 %, generation the
/// rest); `state.finish()` is always called at the end so the splash dismisses.
pub fn run_startup_generation(state: &splash::SplashState) {
    // Isolate the work so a failure is logged, never a silent crash, and always
    // dismiss the splash. On Windows (GUI subsystem) there is no console, so the
    // outcome is written to `logs/ximod_startup.log` in the configuration
    // folder (E-05) for support.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| startup_work(state)));
    {
        let dir = config::paths::get().logs_dir.clone();
        let path = dir.join("ximod_startup.log");
        match outcome {
            // Write the log ONLY on an anomaly worth investigating; on a clean run
            // remove any stale log so it never lingers.
            Ok((_text, false)) => {
                let _ = std::fs::remove_file(&path);
            }
            Ok((text, true)) => {
                let _ = std::fs::write(&path, text);
            }
            Err(_) => {
                let _ = std::fs::write(&path, "PANIC during startup generation.");
            }
        }
    }
    state.set_fraction(1.0);
    state.finish();
}

/// The startup detection + generation. Returns `(log, anomaly)`: the diagnostic
/// text, and whether something worth investigating happened (no game detected, an
/// invalid registered path, a Config.ini save error, or a dictionary write
/// failure). Expected outcomes — old games without STRINGS, up-to-date rebuilds —
/// are not anomalies.
fn startup_work(state: &splash::SplashState) -> (String, bool) {
    use std::path::Path;
    let mut log: Vec<String> = Vec::new();

    let paths = config::paths::get();
    log.push(format!(
        "ximod startup generation — Config.ini: {} (mode {}{})",
        paths.config_path().display(),
        paths.mode.as_str(),
        if paths.read_only { ", READ-ONLY" } else { "" }
    ));

    let mut cfg = config::AppConfig::load().unwrap_or_default();
    log.push(format!(
        "languages: {} → {}",
        cfg.source_language, cfg.target_language
    ));

    // Game sheets (assets/data/games/<id>.json). A missing or invalid sheet
    // falls back to the built-in one; that is worth a line in the log but not
    // an anomaly by itself (the application keeps its previous behaviour).
    let games_dir = config::assets_data_dir().join("games");
    let fields_dir = config::assets_data_dir().join("fields");
    let user_fields_dir = paths.data_dir.join("fields");
    let (issues, field_issues) = games::spec::init(&games_dir, &fields_dir, Some(&user_fields_dir));
    log.push(format!(
        "game sheets: {} ({} issue(s))",
        games_dir.display(),
        issues.len()
    ));
    for i in &issues {
        log.push(format!(
            "  sheet {}: {} — built-in sheet used",
            i.path.display(),
            i.reason
        ));
    }
    // Translatable-field tables (A-06): a user file that cannot be read is
    // reported once here and in the UI; the shipped one is used (R-A06-07).
    log.push(format!(
        "field tables: {} (user: {}, {} issue(s))",
        fields_dir.display(),
        user_fields_dir.display(),
        field_issues.len()
    ));
    for i in &field_issues {
        log.push(format!(
            "  fields {}: {} — shipped table used",
            i.path.display(),
            i.reason
        ));
    }
    for i in &field_issues {
        push_startup_notice(StartupNotice::FieldsFallback {
            game: i.game.display_name().to_string(),
            path: i.path.display().to_string(),
        });
    }

    // Remember the paths before, to report corrections.
    let before: std::collections::BTreeMap<String, String> = cfg
        .games
        .iter()
        .map(|(k, e)| (k.clone(), e.path.clone()))
        .collect();

    // Phase 1 — detect installed games (bounded).
    let found = games::detect::detect_all_reporting(|done, total, _| {
        state.set_fraction(0.30 * done as f32 / total.max(1) as f32);
    });
    let mut anomaly = found.is_empty();
    log.push(format!("detected {} game folder(s):", found.len()));
    for d in &found {
        log.push(format!(
            "  [{}] {} → {}",
            d.source.as_str(),
            d.game.id(),
            d.root.display()
        ));
    }
    if found.is_empty() {
        log.push("— Steam detection diagnostics —".into());
        log.extend(games::detect::diagnostics());
    }

    // Merge detections over non-locked entries.
    for d in &found {
        cfg.set_game(
            d.game.id(),
            &d.root.to_string_lossy(),
            d.source.as_str(),
            false,
        );
    }

    // Validate: drop non-locked entries whose game is no longer at its path.
    let mut pruned: Vec<String> = Vec::new();
    cfg.games.retain(|id, e| {
        if e.locked {
            return true;
        }
        let ok = games::detect::is_game_at(games::Game::from_id(id), Path::new(&e.path));
        if !ok {
            pruned.push(format!("  pruned {id} (game not found at {})", e.path));
        }
        ok
    });

    log.push("registered games after detect + validate:".into());
    for (id, e) in &cfg.games {
        let tag = match before.get(id) {
            Some(old) if old != &e.path => format!("CORRECTED (was {old})"),
            Some(_) => "unchanged".into(),
            None => "new".into(),
        };
        let valid = games::detect::is_game_at(games::Game::from_id(id), Path::new(&e.path));
        if !valid {
            anomaly = true;
        }
        log.push(format!(
            "  {id} = {} [{}] — {tag}{}",
            e.path,
            e.source,
            if valid { "" } else { " (INVALID)" }
        ));
    }
    log.extend(pruned);
    if let Err(e) = cfg.save() {
        anomaly = true;
        log.push(format!("Config.ini save error: {e}"));
    } else {
        log.push("Config.ini saved.".into());
    }

    // Phase 2 — (re)build the game dictionaries (incremental): ALL English→other
    // pairs per localized game, including DLC STRINGS listed in GamesDLC.json.
    let jeux = cfg.game_base_dir();
    log.push(format!("dictionaries folder: {}", jeux.display()));
    let dlc = games::dlc::load_or_create(&config::assets_data_dir());
    let report = games::build::regenerate(&cfg, &jeux, &dlc, |done, total, _| {
        state.set_fraction(0.30 + 0.70 * done as f32 / total.max(1) as f32);
    });
    log.push(format!(
        "build: {} game(s) considered, {} built, {} up-to-date",
        report.games, report.built, report.skipped
    ));
    if report.details.iter().any(|d| d.contains("write failed")) {
        anomaly = true;
    }
    log.extend(report.details);

    (log.join("\n"), anomaly)
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
