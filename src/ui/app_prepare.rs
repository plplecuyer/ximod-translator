//! The preparation of the launch on the interface side (E-13, D-116): its
//! events merged into the configuration, its progress in the status bar,
//! and the game bases applied to the documents opened before they were
//! ready.

use eframe::egui;

use super::XimodApp;
use crate::journal::{Kind, Level};
use crate::prepare::{Event, Phase, Preparation};

impl XimodApp {
    /// The preparation started by `main` (`None`: `AutoDetectGames=0`,
    /// said in the journal, R-E13-01).
    pub fn with_preparation(mut self, prep: Option<Preparation>) -> Self {
        if prep.is_none() {
            let text = self.i18n.t("journal-prep-off");
            self.journal_for(Level::Info, Kind::Start, None, &text);
        }
        self.prep = prep;
        self
    }

    /// Whether the bases of `game` may still change.
    pub(crate) fn prep_waits_for(&self, game: &str) -> bool {
        !game.trim().is_empty() && self.prep.as_ref().is_some_and(|p| p.waits_for(game))
    }

    /// After a stack applied to document `idx`: its game base comes later
    /// if the preparation has not announced it (R-E13-04).
    pub(crate) fn prep_note_stack(&mut self, idx: usize) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let game = doc.settings.game.clone();
        if doc.settings.target.trim().is_empty() || !self.prep_waits_for(&game) {
            return;
        }
        let serial = doc.serial;
        if !self.base_pending.contains(&serial) {
            self.base_pending.push(serial);
            let text = self.i18n.t_arg(
                "journal-prep-wait",
                "game",
                crate::games::Game::from_id(&game).display_name(),
            );
            let doc = self.journal_doc_name(idx);
            self.journal_for(Level::Info, Kind::Dicts, doc.as_deref(), &text);
        }
    }

    /// Each frame: the events of the preparation.
    pub(crate) fn prep_poll(&mut self, ctx: &egui::Context) {
        let Some(prep) = self.prep.clone() else {
            return;
        };
        for e in prep.take_events() {
            match e {
                Event::Detected(d) => self.prep_merge(d),
                Event::GameReady { id, secs, built } => self.prep_ready(&id, secs, built),
                Event::Finished {
                    games,
                    built,
                    up_to_date,
                    secs,
                } => {
                    let text = self.i18n.t_args(
                        "journal-prep-done",
                        &[
                            ("games", &games.to_string()),
                            ("built", &built.to_string()),
                            ("uptodate", &up_to_date.to_string()),
                            ("secs", &crate::journal::secs_text(&self.i18n, secs)),
                        ],
                    );
                    self.journal_for(Level::Info, Kind::Dicts, None, &text);
                }
            }
        }
        if prep.is_finished() {
            self.prep = None;
            // A document whose game was not in the preparation: its stack
            // as it is now.
            for serial in std::mem::take(&mut self.base_pending) {
                if let Some(idx) = self.docs.iter().position(|d| d.serial == serial) {
                    self.restack_doc(idx);
                }
            }
            // The bases are new: the dictionaries window lists them again.
            self.file_dbs.invalidate(None);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    /// What the detection found, merged into the configuration, which the
    /// interface alone writes (R-E13-03).
    fn prep_merge(&mut self, d: crate::prepare::Detected) {
        let changed = crate::prepare::merge(&mut self.config, &d);
        if changed && let Err(e) = self.config.save() {
            tracing::warn!("Config.ini save error after detection: {e}");
        }
        if d.found.is_empty() {
            return;
        }
        let names: Vec<&str> = d
            .found
            .iter()
            .map(|(id, _, _)| crate::games::Game::from_id(id).display_name())
            .collect();
        let text = self.i18n.t_args(
            "journal-prep-detected",
            &[
                ("n", &names.len().to_string()),
                ("games", &names.join(", ")),
            ],
        );
        self.journal_for(Level::Info, Kind::Dicts, None, &text);
    }

    /// The bases of game `id` are ready: the documents that waited for
    /// them receive them, in one undoable operation each.
    fn prep_ready(&mut self, id: &str, secs: f32, built: bool) {
        let name = crate::games::Game::from_id(id).display_name();
        // One entry per base written (the up-to-date ones and the games
        // without a base are counted by the final entry).
        if built {
            let text = self.i18n.t_args(
                "journal-prep-ready",
                &[
                    ("game", name),
                    ("secs", &crate::journal::secs_text(&self.i18n, secs)),
                ],
            );
            self.journal_for(Level::Info, Kind::Dicts, None, &text);
        }
        let waiting: Vec<u64> = self
            .base_pending
            .iter()
            .copied()
            .filter(|s| {
                self.docs
                    .iter()
                    .any(|d| d.serial == *s && d.settings.game == id)
            })
            .collect();
        if waiting.is_empty() {
            return;
        }
        self.base_pending.retain(|s| !waiting.contains(s));
        for serial in waiting {
            if let Some(idx) = self.docs.iter().position(|d| d.serial == serial) {
                self.restack_doc(idx);
                let text = self.i18n.t_arg("journal-prep-applied", "game", name);
                let doc = self.journal_doc_name(idx);
                self.journal_for(Level::Info, Kind::Dicts, doc.as_deref(), &text);
            }
        }
    }

    /// « Préparation des bases de jeu : Skyrim SE (2 / 5)… » in the status
    /// bar while the preparation runs (R-E13-02).
    pub(crate) fn prep_status(&self, ui: &mut egui::Ui) {
        let Some(phase) = self.prep.as_ref().and_then(Preparation::phase) else {
            return;
        };
        let text = match phase {
            Phase::Detecting => self.i18n.t("prep-detecting"),
            Phase::Building { game, n, total } => self.i18n.t_args(
                "prep-building",
                &[
                    ("game", &crate::doc_settings::game_short(&self.i18n, &game)),
                    ("n", &n.to_string()),
                    ("total", &total.to_string()),
                ],
            ),
        };
        let tip = self.i18n.t("prep-tip");
        ui.label(egui::RichText::new(text).weak())
            .on_hover_text(&tip);
        ui.add(egui::Spinner::new());
    }
}
