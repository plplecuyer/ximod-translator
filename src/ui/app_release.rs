//! Checking for a newer version (E-14), as XIMOD Architect does: once a
//! day at start when *Paramètres › Général › Vérifier les mises à jour au
//! démarrage* is checked, or on request (*Aide › Rechercher les mises à
//! jour…*). The request runs on its own thread; a newer version shows a
//! discreet banner under the menu bar with the download links.

use std::sync::mpsc;

use eframe::egui;
use std::time::Duration;

use super::XimodApp;
use crate::journal::{Kind, Level};
use crate::release_check::{self, Outcome, Release, Version};

/// The state of the check.
#[derive(Default)]
pub(crate) struct ReleaseState {
    /// The answer of the check under way.
    rx: Option<mpsc::Receiver<Outcome>>,
    /// The daily check was considered (once per session).
    started: bool,
    /// The check under way was asked from the menu: its result is shown.
    manual: bool,
    /// The newer release of the banner, until it is hidden.
    banner: Option<Release>,
    /// The window of a check asked from the menu: `None` while it runs.
    window: Option<Option<Outcome>>,
}

impl XimodApp {
    /// The daily check, at the first frame (R-E14-01).
    pub(crate) fn release_check_at_start(&mut self, ctx: &egui::Context) {
        if self.release.started {
            return;
        }
        self.release.started = true;
        let today = crate::backup::today();
        if !self.config.check_updates
            || !release_check::is_due(&self.config.last_update_check, &today)
        {
            return;
        }
        self.config.last_update_check = today;
        let _ = self.config.save();
        self.release_check_now(ctx, false);
    }

    /// Start a check; `manual`: asked from the menu, its result is shown
    /// in a window, up to date or failed included.
    pub(crate) fn release_check_now(&mut self, ctx: &egui::Context, manual: bool) {
        if manual {
            self.release.manual = true;
            self.release.window = Some(None);
        }
        if self.release.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let proxy = self.config.online.effective_proxy();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = release_check::check(proxy.as_deref(), Duration::from_secs(10));
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
        self.release.rx = Some(rx);
    }

    /// Collect the answer, if it came.
    pub(crate) fn release_poll(&mut self) {
        let Some(rx) = &self.release.rx else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(o) => o,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Outcome::Failed("no answer".into()),
        };
        self.release.rx = None;
        let current = Version::current().to_string();
        match &outcome {
            Outcome::Newer(r) => {
                let text = self.i18n.t_args(
                    "release-journal-newer",
                    &[("version", &r.version.to_string()), ("url", &r.url)],
                );
                self.journal_add(Level::Info, Kind::Message, &text);
                self.release.banner = Some(r.clone());
            }
            Outcome::UpToDate(v) => {
                let text = self.i18n.t_args(
                    "release-uptodate",
                    &[("current", &current), ("version", &v.to_string())],
                );
                self.journal_add(Level::Info, Kind::Message, &text);
            }
            Outcome::Failed(e) => {
                tracing::info!("update check: {e}");
                let text = self.i18n.t_arg("release-failed", "error", e);
                self.journal_add(Level::Info, Kind::Message, &text);
            }
        }
        if std::mem::take(&mut self.release.manual) {
            self.release.window = Some(Some(outcome));
        }
    }

    /// The download links of a release: GitHub, and Nexus when its page is
    /// set (`[Update] UpdateNexusUrl`).
    fn release_links(&self, ui: &mut egui::Ui, r: &Release) {
        if ui.link(self.i18n.t("release-link-github")).clicked() {
            crate::fonts::open_url(&r.url);
        }
        let nexus = self.config.update_nexus_url.trim();
        if nexus.starts_with("https://") && ui.link(self.i18n.t("release-link-nexus")).clicked() {
            crate::fonts::open_url(nexus);
        }
    }

    /// The banner under the menu bar (R-E14-02), until hidden.
    pub(crate) fn release_banner(&mut self, root: &mut egui::Ui) {
        let Some(r) = self.release.banner.clone() else {
            return;
        };
        let mut hide = false;
        egui::Panel::top("update_banner").show(root, |ui| {
            ui.horizontal_wrapped(|ui| {
                let pal = crate::ui::theme::Palette::of(ui.visuals());
                ui.label(
                    egui::RichText::new(self.i18n.t_args(
                        "release-banner",
                        &[
                            ("version", &r.version.to_string()),
                            ("current", &Version::current().to_string()),
                        ],
                    ))
                    .color(pal.success),
                );
                self.release_links(ui, &r);
                if ui
                    .small_button(crate::icons::X)
                    .on_hover_text(self.i18n.t("release-dismiss"))
                    .clicked()
                {
                    hide = true;
                }
            });
        });
        if hide {
            self.release.banner = None;
        }
    }

    /// The window of a check asked from the menu.
    pub(crate) fn release_window(&mut self, ctx: &egui::Context) {
        let Some(state) = self.release.window.clone() else {
            return;
        };
        let mut close = false;
        egui::Window::new(self.i18n.t("release-title"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                let current = Version::current().to_string();
                match &state {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.i18n.t("release-checking"));
                        });
                    }
                    Some(Outcome::Newer(r)) => {
                        ui.label(self.i18n.t_args(
                            "release-banner",
                            &[("version", &r.version.to_string()), ("current", &current)],
                        ));
                        ui.horizontal(|ui| self.release_links(ui, r));
                    }
                    Some(Outcome::UpToDate(v)) => {
                        ui.label(self.i18n.t_args(
                            "release-uptodate",
                            &[("current", &current), ("version", &v.to_string())],
                        ));
                    }
                    Some(Outcome::Failed(e)) => {
                        ui.add(
                            egui::Label::new(self.i18n.t_arg("release-failed", "error", e))
                                .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                    }
                }
                ui.add_space(8.0);
                let enter = state.is_some()
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                if ui.button(self.i18n.t("btn-close")).clicked() || enter || esc {
                    close = true;
                }
            });
        if close {
            self.release.window = None;
            // A check still under way keeps going; its result lands in the
            // journal and, when newer, in the banner.
            self.release.manual = false;
        }
    }
}
