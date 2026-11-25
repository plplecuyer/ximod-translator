//! The preparation of the launch (E-13, decision D-116): the detection of
//! the installed games and the (re)building of their game bases run on a
//! background thread while the main window is already open.
//!
//! The thread never writes `Config.ini`: what the detection found goes to
//! the interface as an [`Event::Detected`], which merges it into its own
//! configuration and saves it — one writer, so a setting changed in the
//! Paramètres meanwhile is never lost (R-E13-03). Each game whose bases are
//! ready is announced ([`Event::GameReady`]): a document opened during the
//! preparation receives its game base then (R-E13-04). The bases are written
//! through a temporary file and renamed, the source stamp of a game last:
//! a launch closed in the middle leaves no half-written base, and the next
//! launch builds again what was not finished (R-E13-06).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::{self, AppConfig};
use crate::games;

/// Where the preparation is.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Looking for the installed games.
    Detecting,
    /// Building the bases of game `n` (from 1) of `total` (`game`: its id).
    Building {
        game: String,
        n: usize,
        total: usize,
    },
}

/// What the detection found, to merge into the configuration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detected {
    /// `(id, root, source)` of each game found.
    pub found: Vec<(String, String, String)>,
    /// `(id, path)` of the entries whose game is no longer at their path.
    pub pruned: Vec<(String, String)>,
}

/// What the interface learns, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Detected(Detected),
    /// The bases of a game are ready (built, up to date, or none to build);
    /// `built`: written by this preparation.
    GameReady {
        id: String,
        secs: f32,
        built: bool,
    },
    Finished {
        games: usize,
        built: usize,
        up_to_date: usize,
        secs: f32,
    },
}

#[derive(Default)]
struct Shared {
    phase: Option<Phase>,
    events: Vec<Event>,
    ready: Vec<String>,
    finished: bool,
}

/// The handle of a running preparation.
#[derive(Clone)]
pub struct Preparation {
    shared: Arc<Mutex<Shared>>,
}

impl Preparation {
    /// Start the preparation of `cfg` (a copy of the configuration of the
    /// interface) on its own thread.
    pub fn start(cfg: AppConfig) -> Preparation {
        let shared = Arc::new(Mutex::new(Shared {
            phase: Some(Phase::Detecting),
            ..Default::default()
        }));
        let s = shared.clone();
        std::thread::spawn(move || run(cfg, &s));
        Preparation { shared }
    }

    /// The preparation of the launch, unless `[Games] AutoDetectGames=0`
    /// (R-E13-01): neither detection nor regeneration then.
    pub fn launch(cfg: &AppConfig) -> Option<Preparation> {
        cfg.auto_detect_games
            .then(|| Preparation::start(cfg.clone()))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    /// Where it is; `None` once finished.
    pub fn phase(&self) -> Option<Phase> {
        self.with(|s| if s.finished { None } else { s.phase.clone() })
    }

    /// The events since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        self.with(|s| std::mem::take(&mut s.events))
    }

    /// Whether the bases of `game` may still change: the preparation runs
    /// and has not announced them.
    pub fn waits_for(&self, game: &str) -> bool {
        self.with(|s| !s.finished && !s.ready.iter().any(|g| g == game))
    }

    pub fn is_finished(&self) -> bool {
        self.with(|s| s.finished)
    }
}

/// Merge what the detection found into `cfg` (the interface's): a game
/// found takes its path unless its entry is locked; an entry the detection
/// pruned goes, unless it is locked or its path changed meanwhile (the user
/// set it in the Paramètres). Returns whether `cfg` changed.
pub fn merge(cfg: &mut AppConfig, d: &Detected) -> bool {
    let mut changed = false;
    for (id, root, source) in &d.found {
        let same = cfg
            .games
            .get(id)
            .is_some_and(|e| &e.path == root && &e.source == source);
        if !same {
            changed |= cfg.set_game(id, root, source, false);
        }
    }
    for (id, path) in &d.pruned {
        if cfg
            .games
            .get(id)
            .is_some_and(|e| !e.locked && &e.path == path)
        {
            cfg.games.remove(id);
            changed = true;
        }
    }
    changed
}

fn run(cfg: AppConfig, shared: &Arc<Mutex<Shared>>) {
    let push = |e: Event| {
        let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
        if let Event::GameReady { id, .. } = &e {
            g.ready.push(id.clone());
        }
        g.events.push(e);
    };
    let set_phase = |p: Phase| {
        let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
        g.phase = Some(p);
    };
    let started = Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        work(cfg, &push, &set_phase)
    }));
    let (log, anomaly, counts) = match outcome {
        Ok(r) => r,
        Err(_) => ("PANIC during the preparation.".to_string(), true, (0, 0, 0)),
    };
    // The diagnostic file, only on an anomaly worth investigating (E-05).
    let path = config::paths::get().logs_dir.join("ximod_startup.log");
    if anomaly {
        let _ = std::fs::write(&path, log);
    } else {
        let _ = std::fs::remove_file(&path);
    }
    push(Event::Finished {
        games: counts.0,
        built: counts.1,
        up_to_date: counts.2,
        secs: started.elapsed().as_secs_f32(),
    });
    let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
    g.finished = true;
}

type Counts = (usize, usize, usize);

/// Detection, then the bases. Returns the diagnostic text, whether
/// something is worth investigating (no game, an invalid path, a write
/// failure), and `(games, built, up to date)`.
fn work(
    mut cfg: AppConfig,
    push: &dyn Fn(Event),
    set_phase: &dyn Fn(Phase),
) -> (String, bool, Counts) {
    use std::path::Path;
    let mut log: Vec<String> = Vec::new();
    let paths = config::paths::get();
    log.push(format!(
        "ximod launch preparation — Config.ini: {} (mode {}{})",
        paths.config_path().display(),
        paths.mode.as_str(),
        if paths.read_only { ", READ-ONLY" } else { "" }
    ));

    // Phase 1 — the installed games.
    let found = games::detect::detect_all_reporting(|_, _, _| {});
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
    let mut detected = Detected::default();
    for d in &found {
        let (id, root) = (
            d.game.id().to_string(),
            d.root.to_string_lossy().to_string(),
        );
        cfg.set_game(&id, &root, d.source.as_str(), false);
        detected
            .found
            .push((id, root, d.source.as_str().to_string()));
    }
    cfg.games.retain(|id, e| {
        if e.locked {
            return true;
        }
        let ok = games::detect::is_game_at(games::Game::from_id(id), Path::new(&e.path));
        if !ok {
            detected.pruned.push((id.clone(), e.path.clone()));
        }
        ok
    });
    for (id, e) in &cfg.games {
        let valid = games::detect::is_game_at(games::Game::from_id(id), Path::new(&e.path));
        anomaly |= !valid;
        log.push(format!(
            "  {id} = {} [{}]{}",
            e.path,
            e.source,
            if valid { "" } else { " (INVALID)" }
        ));
    }
    push(Event::Detected(detected));

    // Phase 2 — the bases, game after game (incremental). The order is the
    // one `regenerate` follows: the registered games with a path.
    let ids: Vec<String> = cfg
        .games
        .iter()
        .filter(|(_, e)| !e.path.trim().is_empty())
        .map(|(id, _)| id.clone())
        .collect();
    let jeux = cfg.game_base_dir();
    log.push(format!("dictionaries folder: {}", jeux.display()));
    let dlc = games::dlc::load_or_create(&config::assets_data_dir());
    let mut game_started = Instant::now();
    let mut written_before = 0;
    let report = games::build::regenerate(&cfg, &jeux, &dlc, |done, total, name, written| {
        if done > 0
            && let Some(id) = ids.get(done - 1)
        {
            push(Event::GameReady {
                id: id.clone(),
                secs: game_started.elapsed().as_secs_f32(),
                built: written > written_before,
            });
            game_started = Instant::now();
        }
        written_before = written;
        if done < total {
            set_phase(Phase::Building {
                game: ids.get(done).cloned().unwrap_or_else(|| name.to_string()),
                n: done + 1,
                total,
            });
        }
    });
    log.push(format!(
        "build: {} game(s) considered, {} built, {} up-to-date",
        report.games, report.built, report.skipped
    ));
    anomaly |= report.details.iter().any(|d| d.contains("write failed"));
    log.extend(report.details);
    (
        log.join("\n"),
        anomaly,
        (report.games, report.built, report.skipped),
    )
}

/// The part of the launch that stays before the window: the game sheets
/// and the tables of translatable fields, read in a few milliseconds and
/// needed by everything else.
pub fn init_sheets() {
    let paths = config::paths::get();
    let games_dir = config::assets_data_dir().join("games");
    let fields_dir = config::assets_data_dir().join("fields");
    let user_fields_dir = paths.data_dir.join("fields");
    let (issues, field_issues) = games::spec::init(&games_dir, &fields_dir, Some(&user_fields_dir));
    for i in &issues {
        tracing::warn!(
            "game sheet {}: {} — built-in sheet used",
            i.path.display(),
            i.reason
        );
    }
    for i in &field_issues {
        tracing::warn!(
            "fields {}: {} — shipped table used",
            i.path.display(),
            i.reason
        );
        crate::push_startup_notice(crate::StartupNotice::FieldsFallback {
            game: i.game.display_name().to_string(),
            path: i.path.display().to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_detection_merges_without_undoing_the_user() {
        let mut cfg = AppConfig::default();
        cfg.set_game("SkyrimSE", "D:/Old/Skyrim", "steam", false);
        cfg.set_game("Fallout4", "E:/Mine/Fallout 4", "manual", true);
        cfg.set_game("Oblivion", "F:/Gone/Oblivion", "gog", false);
        cfg.set_game("Starfield", "G:/User/Starfield", "manual", false);
        let d = Detected {
            found: vec![
                ("SkyrimSE".into(), "C:/Steam/Skyrim".into(), "steam".into()),
                (
                    "Fallout4".into(),
                    "C:/Steam/Fallout 4".into(),
                    "steam".into(),
                ),
            ],
            pruned: vec![
                ("Oblivion".into(), "F:/Gone/Oblivion".into()),
                // Pruned at its launch path, changed since by the user.
                ("Starfield".into(), "G:/Old/Starfield".into()),
            ],
        };
        assert!(merge(&mut cfg, &d));
        assert_eq!(cfg.games["SkyrimSE"].path, "C:/Steam/Skyrim");
        // Locked: kept.
        assert_eq!(cfg.games["Fallout4"].path, "E:/Mine/Fallout 4");
        assert!(!cfg.games.contains_key("Oblivion"));
        assert_eq!(cfg.games["Starfield"].path, "G:/User/Starfield");
        // T-E13-04: the other settings, changed in the Paramètres meanwhile,
        // are the interface's: the merge leaves them alone.
        let mut user = AppConfig {
            target_language: "deu".into(),
            ..AppConfig::default()
        };
        assert!(merge(&mut user, &d));
        assert_eq!(user.target_language, "deu");
        // The same detection again changes nothing.
        let again = Detected {
            found: d.found.clone(),
            pruned: Vec::new(),
        };
        assert!(!merge(&mut cfg, &again));
    }

    #[test]
    fn auto_detect_games_off_means_no_preparation() {
        // T-E13-01.
        let cfg = AppConfig {
            auto_detect_games: false,
            ..AppConfig::default()
        };
        assert!(Preparation::launch(&cfg).is_none());
    }

    #[test]
    fn a_game_is_waited_for_until_announced() {
        let p = Preparation {
            shared: Arc::new(Mutex::new(Shared {
                phase: Some(Phase::Detecting),
                ..Default::default()
            })),
        };
        assert!(p.waits_for("SkyrimSE"));
        p.with(|s| {
            s.ready.push("SkyrimSE".into());
            s.events.push(Event::GameReady {
                id: "SkyrimSE".into(),
                secs: 1.0,
                built: true,
            });
        });
        assert!(!p.waits_for("SkyrimSE"));
        assert!(p.waits_for("Fallout4"));
        assert_eq!(p.take_events().len(), 1);
        assert!(p.take_events().is_empty());
        p.with(|s| s.finished = true);
        assert!(!p.waits_for("Fallout4"));
        assert_eq!(p.phase(), None);
    }
}
