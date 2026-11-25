//! The settings that follow the game (E-04, D-120): the « Réglages de » list
//! and the « Propre à ce jeu » group of the Paramètres, the initial folder of
//! the export's save window, and the journal entry when a document changes
//! game.

use eframe::egui;

use super::XimodApp;
use crate::config::{GamePrefs, OutputMode, Pref, ResolvedPrefs};
use crate::doc_settings::DocSettings;
use crate::games::Game;
use crate::journal::{Kind, Level};

impl XimodApp {
    /// The save window of an export, opened in the folder of the exports of
    /// the document's game (R-E04-05): its own, else the last of all games.
    pub(crate) fn export_dialog(&self, parent: &eframe::Frame) -> rfd::FileDialog {
        let fd = rfd::FileDialog::new().set_parent(parent);
        let dir = self
            .config
            .prefs_for(&self.active_settings().game)
            .export_dir;
        if !dir.is_empty() && std::path::Path::new(&dir).is_dir() {
            fd.set_directory(dir)
        } else {
            fd
        }
    }

    /// « Réglages d'Oblivion appliqués : sortie Plugin complet » when the
    /// document `idx` changes game and takes settings of the new one
    /// (R-E04-04).
    pub(crate) fn journal_game_prefs(&mut self, idx: usize, s: &DocSettings, changed: &[Pref]) {
        if changed.is_empty() {
            return;
        }
        let parts: Vec<String> = changed
            .iter()
            .map(|p| match p {
                Pref::Source => self.i18n.t_arg(
                    "game-prefs-source-is",
                    "value",
                    &self.i18n.display_name(&s.source),
                ),
                Pref::Target => self.i18n.t_arg(
                    "game-prefs-target-is",
                    "value",
                    &self.i18n.display_name(&s.target),
                ),
                _ => self.i18n.t_arg(
                    "game-prefs-output-is",
                    "value",
                    &s.output
                        .map(|m| self.output_label(m))
                        .unwrap_or_else(|| "—".into()),
                ),
            })
            .collect();
        let text = self.i18n.t_args(
            "journal-game-prefs",
            &[
                ("game", &s.game_short(&self.i18n)),
                ("what", &parts.join(", ")),
            ],
        );
        let doc = self.journal_doc_name(idx);
        self.journal_for(Level::Info, Kind::Config, doc.as_deref(), &text);
    }

    /// The games of the « Réglages de » list: those known (detected,
    /// configured, with a base) and those with a section.
    fn prefs_games(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            self.selectable_games().into_iter().collect();
        set.extend(self.temp_game_prefs.keys().cloned());
        set.into_iter()
            .filter(|id| Game::from_id(id) != Game::Unknown)
            .collect()
    }

    /// The common values as the Paramètres show them (the « Conserver
    /// l'original » of the Avancé tab as edited).
    fn temp_common_prefs(&self) -> ResolvedPrefs {
        let mut c = self.config.common_prefs();
        c.keep_original = self.temp_keep_original;
        c
    }

    /// « Réglages de : Tous les jeux / <jeu> », at the top of the
    /// Paramètres (R-E04, figure B.1).
    pub(crate) fn settings_prefs_scope(&mut self, ui: &mut egui::Ui) {
        let games = self.prefs_games();
        let label_of = |id: &str| -> String {
            if id.is_empty() {
                self.i18n.t("prefs-all-games")
            } else {
                Game::from_id(id).display_name().to_string()
            }
        };
        let mut sel = self.temp_prefs_game.clone();
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("prefs-scope"));
            egui::ComboBox::from_id_salt("prefs_scope")
                .selected_text(label_of(&sel))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sel, String::new(), label_of(""));
                    for id in &games {
                        ui.selectable_value(&mut sel, id.clone(), label_of(id));
                    }
                });
            ui.label(egui::RichText::new(self.i18n.t("prefs-scope-tip")).weak());
        });
        if sel != self.temp_prefs_game {
            self.temp_prefs_game = sel;
            // The game's settings are on the Général tab.
            self.settings_tab = super::SettingsTab::General;
        }
        ui.add_space(4.0);
    }

    /// The « Propre à ce jeu » group of the Général tab, for the game chosen
    /// in « Réglages de »: a box per setting; ticked, the value follows the
    /// game; unticked, the field is greyed and recalls the common value.
    pub(crate) fn settings_game_prefs(&mut self, ui: &mut egui::Ui) {
        let game_id = self.temp_prefs_game.clone();
        if game_id.is_empty() {
            return;
        }
        let game = Game::from_id(&game_id);
        let common = self.temp_common_prefs();
        let mut prefs = self
            .temp_game_prefs
            .get(&game_id)
            .cloned()
            .unwrap_or_default();
        let shown = resolved(&prefs, &common);
        let fw = 280.0_f32;
        let mut browse: Option<Pref> = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new(self.i18n.t("prefs-own-title")).strong());
            ui.add_space(4.0);
            egui::Grid::new("game_prefs_grid")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for p in Pref::ALL {
                        let mut own = prefs.has(p);
                        let r = ui.checkbox(&mut own, self.i18n.t(pref_key(p)));
                        if r.changed() {
                            if own {
                                prefs.take(p, &shown);
                            } else {
                                prefs.clear(p);
                            }
                        }
                        let mut value = resolved(&prefs, &common);
                        ui.add_enabled_ui(own, |ui| {
                            ui.set_min_width(fw);
                            self.pref_field(ui, p, game, &mut value, fw, &mut browse);
                        });
                        if own {
                            prefs.take(p, &value);
                        }
                        // The common value recalled (and when it does not
                        // apply to this game).
                        let mut note = String::new();
                        if !own {
                            note = self.i18n.t_arg(
                                "prefs-common-is",
                                "value",
                                &self.pref_text(p, &common),
                            );
                        }
                        if p == Pref::LocalizeSourceStrings && !game.uses_localized_strings() {
                            let na =
                                self.i18n
                                    .t_arg("prefs-not-for-game", "game", game.display_name());
                            note = if note.is_empty() {
                                na
                            } else {
                                format!("{note} {na}")
                            };
                        }
                        ui.label(egui::RichText::new(note).weak());
                        ui.end_row();
                    }
                });
        });
        ui.label(
            egui::RichText::new(self.i18n.t_arg("prefs-own-help", "game", &game_id))
                .weak()
                .small(),
        );
        if let Some(p) = browse {
            let start = match p {
                Pref::OpenDir => prefs.open_dir.clone(),
                _ => prefs.export_dir.clone(),
            }
            .unwrap_or_default();
            let mut fd = rfd::FileDialog::new();
            if std::path::Path::new(&start).is_dir() {
                fd = fd.set_directory(&start);
            }
            if let Some(dir) = fd.pick_folder() {
                let d = Some(dir.display().to_string());
                match p {
                    Pref::OpenDir => prefs.open_dir = d,
                    _ => prefs.export_dir = d,
                }
            }
        }
        if prefs.is_empty() {
            self.temp_game_prefs.remove(&game_id);
        } else {
            self.temp_game_prefs.insert(game_id, prefs);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new(self.i18n.t("prefs-common-title")).strong());
        ui.add_space(4.0);
    }

    /// The field of a setting.
    fn pref_field(
        &self,
        ui: &mut egui::Ui,
        p: Pref,
        game: Game,
        v: &mut ResolvedPrefs,
        fw: f32,
        browse: &mut Option<Pref>,
    ) {
        match p {
            Pref::Source | Pref::Target => {
                let cur = if p == Pref::Source {
                    &mut v.source
                } else {
                    &mut v.target
                };
                let mut codes: Vec<String> = crate::lang::STRINGS_LANGUAGES
                    .iter()
                    .map(|(_, iso3, _)| iso3.to_string())
                    .collect();
                if p == Pref::Target {
                    for c in &self.target_langs {
                        if !codes.contains(c) {
                            codes.push(c.clone());
                        }
                    }
                }
                if !cur.is_empty() && !codes.contains(cur) {
                    codes.insert(0, cur.clone());
                }
                let selected = if cur.is_empty() {
                    "—".to_string()
                } else {
                    self.i18n.display_name(cur)
                };
                egui::ComboBox::from_id_salt(("game_pref_lang", pref_key(p)))
                    .width(fw)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for c in &codes {
                            let name = self.i18n.display_name(c);
                            ui.selectable_value(cur, c.clone(), name);
                        }
                    });
            }
            Pref::Output => {
                egui::ComboBox::from_id_salt("game_pref_output")
                    .width(fw)
                    .selected_text(self.output_label(v.output))
                    .show_ui(ui, |ui| {
                        for m in OutputMode::variants() {
                            if m.fits(game) {
                                ui.selectable_value(&mut v.output, *m, self.output_label(*m));
                            }
                        }
                    });
            }
            Pref::OpenDir | Pref::ExportDir => {
                let d = if p == Pref::OpenDir {
                    &mut v.open_dir
                } else {
                    &mut v.export_dir
                };
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(d).desired_width(fw - 110.0));
                    if ui.button(self.i18n.t("settings-browse")).clicked() {
                        *browse = Some(p);
                    }
                });
            }
            Pref::LocalizeSourceStrings => {
                ui.checkbox(&mut v.localize_source_strings, self.i18n.t("prefs-yes"));
            }
            Pref::KeepOriginal => {
                ui.checkbox(&mut v.keep_original, self.i18n.t("prefs-yes"));
            }
        }
    }

    /// A value as text (« Français », « Plugin complet », « oui »…).
    fn pref_text(&self, p: Pref, r: &ResolvedPrefs) -> String {
        let yes_no = |b: bool| self.i18n.t(if b { "prefs-yes" } else { "prefs-no" });
        match p {
            Pref::Source => self.lang_or_dash(&r.source),
            Pref::Target => self.lang_or_dash(&r.target),
            Pref::Output => self.output_label(r.output),
            Pref::OpenDir => dash(&r.open_dir),
            Pref::ExportDir => dash(&r.export_dir),
            Pref::LocalizeSourceStrings => yes_no(r.localize_source_strings),
            Pref::KeepOriginal => yes_no(r.keep_original),
        }
    }

    fn lang_or_dash(&self, code: &str) -> String {
        if code.trim().is_empty() {
            "—".into()
        } else {
            self.i18n.display_name(code)
        }
    }
}

fn dash(s: &str) -> String {
    if s.trim().is_empty() {
        "—".into()
    } else {
        s.to_string()
    }
}

/// The label key of a setting.
fn pref_key(p: Pref) -> &'static str {
    match p {
        Pref::Source => "prefs-source",
        Pref::Target => "prefs-target",
        Pref::Output => "prefs-output",
        Pref::OpenDir => "prefs-open-dir",
        Pref::ExportDir => "prefs-export-dir",
        Pref::LocalizeSourceStrings => "prefs-localize-source",
        Pref::KeepOriginal => "prefs-keep-original",
    }
}

/// The values of a game being edited: its own, else the common ones.
fn resolved(p: &GamePrefs, c: &ResolvedPrefs) -> ResolvedPrefs {
    ResolvedPrefs {
        source: p.source.clone().unwrap_or_else(|| c.source.clone()),
        target: p.target.clone().unwrap_or_else(|| c.target.clone()),
        output: p.output.unwrap_or(c.output),
        open_dir: p.open_dir.clone().unwrap_or_else(|| c.open_dir.clone()),
        export_dir: p.export_dir.clone().unwrap_or_else(|| c.export_dir.clone()),
        localize_source_strings: p
            .localize_source_strings
            .unwrap_or(c.localize_source_strings),
        keep_original: p.keep_original.unwrap_or(c.keep_original),
    }
}
