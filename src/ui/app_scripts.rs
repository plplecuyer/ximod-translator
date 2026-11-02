//! The scripts of a mod in the application (C-06): the window Outils ▸
//! Scripts ▸ Scripts du mod…, the « scripts » document and its export, the
//! Script tab of the edit window, the rules file.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use super::{Doc, XimodApp, read_geometry as read_geometry_of};
use crate::config::OutputMode;
use crate::games::Game;
use crate::scripts::{Origin, ScriptSet};

/// The window while it is open.
#[derive(Default)]
pub(crate) struct ScriptsWindow {
    pub open: bool,
    pub needs_place: bool,
    /// The scripts found, and the game and settings of their plugin.
    pub set: Option<Arc<ScriptSet>>,
    pub game: Option<Game>,
    pub message: Option<String>,
    /// The games before Papyrus (C-07): the scripts are in the plugin, the
    /// window lists them from the plugin's document.
    pub in_plugin: bool,
    /// The script picked in the table (Ouvrir dans un onglet goes to it).
    pub selected: Option<String>,
}

/// Whether a game's scripts are in its plugins (SCPT, SCTX): the games
/// before Papyrus.
pub(crate) fn scripts_in_plugin(game: Game) -> bool {
    matches!(
        game.id(),
        "Oblivion" | "OblivionRD" | "Fallout3" | "FalloutNV"
    )
}

/// One row of the window's table.
struct Row {
    name: String,
    origin: String,
    format: String,
    strings: usize,
    texts: usize,
    translated: usize,
    /// « oui » / « non ».
    source: String,
    state: String,
}

impl XimodApp {
    /// The plugin of the active document, or of its scripts, and its game.
    fn scripts_plugin(&self) -> Option<(PathBuf, Game, Option<Arc<ScriptSet>>)> {
        let doc = self.active_doc()?;
        if doc.all.is_some() {
            return None;
        }
        let game = doc.settings.game();
        if let Some(s) = &doc.scripts {
            return Some((
                s.plugin.clone().unwrap_or_else(|| doc.path.clone()),
                game,
                Some(s.clone()),
            ));
        }
        (doc.tm.meta.source_format == "plugin").then(|| (doc.path.clone(), game, None))
    }

    /// Whether Scripts du mod… applies to the active document.
    pub(crate) fn scripts_available(&self) -> bool {
        self.scripts_plugin().is_some()
    }

    /// Outils ▸ Scripts ▸ Scripts du mod…: find the scripts of the active
    /// plugin and show them.
    pub(crate) fn open_scripts_window(&mut self) {
        let Some((plugin, game, known)) = self.scripts_plugin() else {
            return;
        };
        let in_plugin = known.is_none() && scripts_in_plugin(game);
        let set = match known {
            Some(s) => s,
            // The scripts of the old games are in the plugin (C-07).
            None if in_plugin => Arc::new(ScriptSet {
                plugin: Some(plugin.clone()),
                dir: plugin.parent().map(PathBuf::from).unwrap_or_default(),
                ..Default::default()
            }),
            None => Arc::new(crate::slow_io("scripts discover", || {
                ScriptSet::discover(&plugin, game)
            })),
        };
        self.scripts_win.in_plugin = in_plugin;
        self.scripts_win.selected = None;
        self.scripts_win.set = Some(set);
        self.scripts_win.game = Some(game);
        self.scripts_win.message = None;
        self.scripts_win.needs_place = !self.scripts_win.open;
        self.scripts_win.open = true;
    }

    /// The rules of the scripts, the user's copy first (read at each use:
    /// a rule removed applies no more at the next opening).
    pub(crate) fn script_rules() -> crate::script_rules::Rules {
        crate::script_rules::Rules::installed()
    }

    /// The facts the rules need for a scripts document.
    pub(crate) fn script_facts(doc: &Doc) -> Option<Arc<crate::scripts::Facts>> {
        doc.scripts.as_ref().map(|s| Arc::new(s.facts()))
    }

    /// Open the « scripts » document of a set (one per plugin, R-C06-05):
    /// its lines, the languages of the plugin's document when it is open,
    /// the output Scripts.
    pub(crate) fn open_scripts_doc(&mut self, set: Arc<ScriptSet>, game: Game) -> bool {
        let (path, plugin_name) = match &set.plugin {
            Some(p) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
                (
                    p.with_file_name(format!("{stem}_scripts.pex")),
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            }
            None => match set.scripts.first().map(|s| &s.origin) {
                Some(Origin::Loose(p)) => (p.clone(), String::new()),
                _ => return false,
            },
        };
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = i;
            return true;
        }
        let mut settings = set
            .plugin
            .as_ref()
            .and_then(|p| self.docs.iter().find(|d| &d.path == p))
            .map(|d| d.settings.clone())
            .unwrap_or_else(|| {
                crate::doc_settings::DocSettings::for_new_doc(game, &self.idle, &self.config)
            });
        settings.output = Some(OutputMode::Scripts);
        let tm = set.to_tm(game, &plugin_name);
        let n = tm.entries.len();
        let mut source_langs = Self::source_languages_of(&path, &tm);
        if !settings.source.trim().is_empty() && !source_langs.contains(&settings.source) {
            source_langs.insert(0, settings.source.clone());
        }
        if settings.source.trim().is_empty()
            && let Some(first) = source_langs.first()
        {
            settings.source = first.clone();
        }
        let display = if plugin_name.is_empty() {
            None
        } else {
            Some(self.i18n.t_arg("doc-scripts-name", "plugin", &plugin_name))
        };
        let mut doc = Doc::new(path, tm, None, settings, self.config.undo_depth);
        doc.source_langs = source_langs;
        doc.scripts = Some(set);
        doc.display = display;
        let name = doc.file_name();
        let head = self
            .i18n
            .t_args("status-opened", &[("name", &name), ("n", &n.to_string())]);
        self.finish_open(doc, head)
    }

    /// Fichier ▸ Ouvrir on a `.pex`: the script alone; the path of a
    /// « scripts » document (recent files): its plugin's scripts.
    pub(crate) fn open_pex_path(
        &mut self,
        path: PathBuf,
        settings: Option<crate::doc_settings::DocSettings>,
    ) -> bool {
        let imposed = settings
            .as_ref()
            .map(|s| s.game())
            .filter(|g| *g != Game::Unknown);
        if !path.is_file() {
            // `<plugin>_scripts.pex`: the plugin next to it.
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(base) = stem.strip_suffix("_scripts") {
                let exts = crate::games::registry().all_plugin_extensions();
                if let Some(plugin) = exts
                    .iter()
                    .map(|e| path.with_file_name(format!("{base}.{e}")))
                    .find(|p| p.is_file())
                {
                    let game = imposed.unwrap_or_else(|| {
                        crate::plugin::read_file(&plugin)
                            .map(|t| Game::from_id(&t.meta.game))
                            .unwrap_or(Game::Unknown)
                    });
                    let set = Arc::new(ScriptSet::discover(&plugin, game));
                    return self.open_scripts_doc(set, game);
                }
            }
            self.notify_err(
                self.i18n
                    .t_arg("err-open", "error", &path.display().to_string()),
            );
            return false;
        }
        let set = match ScriptSet::single(&path) {
            Ok(s) => s,
            Err(e) => {
                self.notify_err(self.i18n.t_arg("err-open", "error", &e.to_string()));
                return false;
            }
        };
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let game = match &set.scripts[0].pex {
            Ok(p) => imposed.unwrap_or(match p.game_id {
                2 => Game::Fallout4,
                3 => Game::from_id("Fallout76"),
                4 => Game::Starfield,
                _ => Game::SkyrimSE,
            }),
            Err(e) => {
                self.notify_err(self.pex_error(&file, e));
                return false;
            }
        };
        self.open_scripts_doc(Arc::new(set), game)
    }

    /// The message of a script that cannot be read or written.
    pub(crate) fn pex_error(&self, file: &str, e: &crate::pex::PexError) -> String {
        use crate::pex::PexError as E;
        match e {
            E::Version(a, b) => self.i18n.t_args(
                "err-pex-version",
                &[("file", file), ("v", &format!("{a}.{b}"))],
            ),
            E::TooMany => self.i18n.t_arg("err-pex-too-many", "file", file),
            _ => self.i18n.t_arg("err-pex-truncated", "file", file),
        }
    }

    /// Export of a « scripts » document (R-C06-06): the scripts with
    /// exported lines, as loose files in `<folder>/Scripts/`, the original
    /// kept as for any export (E-06).
    pub(crate) fn export_scripts(
        &mut self,
        parent: &eframe::Frame,
        tm: &crate::tm::TranslationMemory,
        exported_line: &str,
    ) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        let Some(set) = doc.scripts.clone() else {
            self.status = self.i18n.t("status-export-needs-scripts");
            return;
        };
        let game = doc.settings.game();
        let page = super::app_encoding::page_game(&doc.settings).then_some(self.out_page(doc));
        // The sources too, on option (R-C07-04, D-67).
        let sources = if self.config.scripts_copy_to_source {
            crate::tm::encoding::with_output(page, || set.translated_sources(tm, game))
        } else {
            Vec::new()
        };
        let files = match crate::tm::encoding::with_output(page, || set.translated(tm, game)) {
            Ok(f) => f,
            Err((rel, e)) => {
                let msg = self.pex_error(&rel, &e);
                self.notify_err(msg);
                return;
            }
        };
        if files.is_empty() {
            self.status = self.i18n.t("status-export-no-strings");
            return;
        }
        let Some(dir) = rfd::FileDialog::new()
            .set_parent(parent)
            .set_directory(&set.dir)
            .pick_folder()
        else {
            return;
        };
        let mut kept_all: Vec<PathBuf> = Vec::new();
        let mut written = 0usize;
        for (rel, bytes) in files.iter().chain(sources.iter()) {
            let path = dir.join(rel);
            if let Some(p) = path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match self.export_write(&path, bytes) {
                Ok(kept) => {
                    kept_all.extend(kept);
                    written += 1;
                }
                Err(e) => {
                    self.notify_err(e);
                    return;
                }
            }
        }
        let scripts_dir = dir.join("Scripts");
        let mut msg = self.i18n.dash(
            &self.i18n.t_args(
                "scripts-written",
                &[
                    ("dir", &scripts_dir.display().to_string()),
                    ("n", &written.to_string()),
                ],
            ),
            exported_line,
        );
        msg = self.i18n.dash(&msg, &self.i18n.t("scripts-written-note"));
        if let Some(note) = self.kept_list_note(&kept_all) {
            msg = self.i18n.dash(&msg, &note);
        }
        self.notify_ok(msg);
        let idx = self.active;
        self.write_mod_dict(idx, false);
        self.personal_autofill(idx);
    }

    /// The rows of the window.
    fn scripts_rows(&self, set: &ScriptSet) -> Vec<Row> {
        let rules = Self::script_rules();
        let doc = set.plugin.as_ref().and_then(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let path = p.with_file_name(format!("{stem}_scripts.pex"));
            self.docs.iter().find(|d| d.path == path)
        });
        let yes = self.i18n.t("word-yes");
        let no = self.i18n.t("word-no");
        let loose = self.i18n.t("scripts-loose");
        set.scripts
            .iter()
            .map(|s| {
                let origin = s.origin.archive_name().unwrap_or_else(|| loose.clone());
                let (format, strings, texts, state) = match &s.pex {
                    Ok(p) => {
                        let texts = p
                            .literals()
                            .filter(|i| {
                                let text = String::from_utf8_lossy(&p.strings[*i]);
                                let fact = (
                                    p.ident[*i],
                                    p.uses[*i].iter().filter_map(|u| u.call.clone()).collect(),
                                );
                                !text.is_empty()
                                    && crate::scripts::lock_of(&text, Some(&fact), &rules).is_none()
                            })
                            .count();
                        (s.format().unwrap_or_default(), p.strings.len(), texts, None)
                    }
                    Err(crate::pex::PexError::Version(a, b)) => (
                        format!("{a}.{b}"),
                        0,
                        0,
                        Some(self.i18n.t("scripts-state-unknown")),
                    ),
                    Err(_) => (
                        String::new(),
                        0,
                        0,
                        Some(self.i18n.t("scripts-state-unread")),
                    ),
                };
                let translated = doc
                    .map(|d| {
                        d.tm.entries
                            .iter()
                            .filter(|e| e.edid == s.name && e.is_translated())
                            .count()
                    })
                    .unwrap_or(0);
                let state = state.unwrap_or_else(|| {
                    if texts == 0 && translated == 0 {
                        self.i18n.t("scripts-state-none")
                    } else if translated > 0 && translated >= texts {
                        self.i18n.t("scripts-state-translated")
                    } else if translated > 0 {
                        self.i18n.t("scripts-state-modified")
                    } else {
                        String::new()
                    }
                });
                Row {
                    name: s.name.clone(),
                    origin,
                    format,
                    strings,
                    texts,
                    translated,
                    source: if s.source.is_some() {
                        yes.clone()
                    } else {
                        no.clone()
                    },
                    state,
                }
            })
            .collect()
    }

    /// The window, each frame while it is open.
    pub(crate) fn scripts_window(&mut self, ctx: &egui::Context) {
        if !self.scripts_win.open {
            return;
        }
        let Some(set) = self.scripts_win.set.clone() else {
            self.scripts_win.open = false;
            return;
        };
        let id = egui::ViewportId::from_hash_of("ximod-scripts");
        let geo = self.config.scripts_win;
        let just_opened = self.scripts_win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(760.0), geo[3].max(380.0)]))
        } else {
            None
        };
        let doc_name = set
            .plugin
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = self.i18n.t_arg("scripts-title", "doc", &doc_name);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([760.0, 380.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let in_plugin = self.scripts_win.in_plugin;
        let rows = if in_plugin {
            self.sctx_rows(&set)
        } else {
            self.scripts_rows(&set)
        };
        let mut close = false;
        let mut action: Option<u8> = None;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.scripts_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::bottom("scripts_bottom").show(vctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let any = !rows.is_empty();
                    if ui
                        .add_enabled(any, egui::Button::new(self.i18n.t("scripts-open")))
                        .clicked()
                    {
                        action = Some(1);
                    }
                    // The scripts of the old games are written with the
                    // plugin (R-C07-05).
                    let files = any && !in_plugin;
                    if ui
                        .add_enabled(files, egui::Button::new(self.i18n.t("scripts-save")))
                        .clicked()
                    {
                        action = Some(2);
                    }
                    if ui
                        .add_enabled(files, egui::Button::new(self.i18n.t("scripts-open-dir")))
                        .clicked()
                    {
                        action = Some(3);
                    }
                });
                // The loose files and the archives: not for the scripts
                // compiled in the plugin.
                if !in_plugin {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.i18n.t("scripts-written-note"))
                                .weak()
                                .small(),
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                }
                if let Some(m) = &self.scripts_win.message {
                    ui.label(m);
                }
                ui.add_space(4.0);
            });
            egui::CentralPanel::default().show(vctx, |ui| {
                if let Some(pick) = self.scripts_body(ui, &set, &rows, in_plugin) {
                    self.scripts_win.selected = Some(pick);
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.scripts_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        match action {
            Some(1) if in_plugin => {
                let name = self
                    .scripts_win
                    .selected
                    .clone()
                    .or_else(|| rows.first().map(|r| r.name.clone()));
                if let (Some(plugin), Some(name)) = (set.plugin.clone(), name) {
                    self.sctx_go_to(&plugin, &name);
                }
            }
            Some(1) | Some(2) => {
                let game = self.scripts_win.game.unwrap_or(Game::Unknown);
                if self.open_scripts_doc(set.clone(), game) && action == Some(2) {
                    self.open_export_dialog();
                }
            }
            Some(3) => {
                let dir = set.dir.join("Scripts");
                crate::fonts::open_path(if dir.is_dir() { &dir } else { &set.dir });
            }
            _ => {}
        }
        if close {
            self.scripts_win.open = false;
            let _ = self.config.save();
        }
    }

    /// The places searched and the table; returns the script clicked.
    fn scripts_body(
        &self,
        ui: &mut egui::Ui,
        set: &ScriptSet,
        rows: &[Row],
        in_plugin: bool,
    ) -> Option<String> {
        if in_plugin {
            ui.label(
                self.i18n
                    .t_arg("scripts-in-plugin", "n", &rows.len().to_string()),
            );
            ui.add_space(4.0);
            return self.scripts_table(ui, rows);
        }
        let loose = set
            .scripts
            .iter()
            .filter(|s| matches!(s.origin, Origin::Loose(_)))
            .count();
        let archived = set.scripts.len() - loose;
        let sources = set.scripts.iter().filter(|s| s.source.is_some()).count();
        ui.label(self.i18n.t_args(
            "scripts-found",
            &[
                ("dir", &set.dir.display().to_string()),
                ("loose", &loose.to_string()),
                ("archived", &archived.to_string()),
                ("sources", &sources.to_string()),
            ],
        ));
        if !set.archives.is_empty() {
            let names: Vec<String> = set
                .archives
                .iter()
                .filter_map(|a| a.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            ui.label(egui::RichText::new(names.join(", ")).weak().small());
        }
        if set.restricted {
            ui.label(
                egui::RichText::new(self.i18n.t("scripts-restricted"))
                    .color(ui.visuals().warn_fg_color)
                    .small(),
            );
        }
        ui.add_space(4.0);
        if rows.is_empty() {
            ui.label(self.i18n.t("scripts-none"));
            return None;
        }
        self.scripts_table(ui, rows)
    }

    /// The table of the window; returns the script clicked.
    fn scripts_table(&self, ui: &mut egui::Ui, rows: &[Row]) -> Option<String> {
        let mut picked = None;
        let selected = self.scripts_win.selected.as_deref();
        let heads = [
            "scripts-col-script",
            "scripts-col-origin",
            "scripts-col-format",
            "scripts-col-strings",
            "scripts-col-texts",
            "scripts-col-translated",
            "scripts-col-source",
            "scripts-col-state",
        ];
        use egui_extras::{Column, TableBuilder};
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::initial(230.0).at_least(120.0).clip(true))
            .column(Column::initial(150.0).clip(true))
            .column(Column::initial(100.0))
            .column(Column::initial(60.0))
            .column(Column::initial(55.0))
            .column(Column::initial(65.0))
            .column(Column::initial(80.0))
            .column(Column::remainder().at_least(110.0))
            .header(20.0, |mut h| {
                for k in heads {
                    h.col(|ui| {
                        ui.strong(self.i18n.t(k));
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, rows.len(), |mut row| {
                    let r = &rows[row.index()];
                    row.col(|ui| {
                        if ui
                            .selectable_label(selected == Some(r.name.as_str()), &r.name)
                            .on_hover_text(&r.name)
                            .clicked()
                        {
                            picked = Some(r.name.clone());
                        }
                    });
                    row.col(|ui| {
                        ui.label(&r.origin);
                    });
                    row.col(|ui| {
                        ui.label(&r.format);
                    });
                    row.col(|ui| {
                        ui.label(r.strings.to_string());
                    });
                    row.col(|ui| {
                        ui.label(r.texts.to_string());
                    });
                    row.col(|ui| {
                        ui.label(r.translated.to_string());
                    });
                    row.col(|ui| {
                        ui.label(&r.source);
                    });
                    row.col(|ui| {
                        ui.label(&r.state);
                    });
                });
            });
        picked
    }

    /// The rows of an old game's plugin (C-07): its scripts with messages,
    /// from the plugin's document.
    fn sctx_rows(&self, set: &ScriptSet) -> Vec<Row> {
        let Some(doc) = set
            .plugin
            .as_ref()
            .and_then(|p| self.docs.iter().find(|d| &d.path == p))
        else {
            return Vec::new();
        };
        let plugin = doc.file_name();
        let mut rows: Vec<Row> = Vec::new();
        for e in doc.tm.entries.iter().filter(|e| e.field_sig == "SCTX") {
            let name = if e.edid.is_empty() {
                format!("{} {:08X}", e.rec_sig, e.formid)
            } else {
                e.edid.clone()
            };
            if rows.last().is_none_or(|r| r.name != name) {
                rows.push(Row {
                    name,
                    origin: plugin.clone(),
                    format: format!("{} · SCTX", e.rec_sig),
                    strings: 0,
                    texts: 0,
                    translated: 0,
                    source: self.i18n.t("word-yes"),
                    state: String::new(),
                });
            }
            let r = rows.last_mut().unwrap();
            r.strings += 1;
            if crate::status::derive(e) != crate::status::Status::Ignored {
                r.texts += 1;
            }
            if e.is_translated() {
                r.translated += 1;
            }
        }
        for r in &mut rows {
            r.state = if r.texts == 0 && r.translated == 0 {
                self.i18n.t("scripts-state-none")
            } else if r.translated > 0 && r.translated >= r.texts {
                self.i18n.t("scripts-state-translated")
            } else if r.translated > 0 {
                self.i18n.t("scripts-state-modified")
            } else {
                String::new()
            };
        }
        rows
    }

    /// Ouvrir dans un onglet for an old game (R-C07-05): the plugin's tab,
    /// the first line of the script selected in the grid.
    fn sctx_go_to(&mut self, plugin: &std::path::Path, name: &str) {
        let Some(i) = self.docs.iter().position(|d| d.path == plugin) else {
            return;
        };
        self.active = i;
        let line = self.docs[i].tm.entries.iter().position(|e| {
            e.field_sig == "SCTX"
                && (e.edid == name || format!("{} {:08X}", e.rec_sig, e.formid) == name)
        });
        if let Some(line) = line {
            self.selection.set_single(line);
            self.pinned_rows.insert(line);
            self.grid_scroll_to = Some(line);
        }
    }

    /// The Script tab of the edit window (figure B.5) for a line of a
    /// compiled script: where the string is used, and the Papyrus source
    /// around the literal when it is found, with the option of copying the
    /// translation into it (D-67); for a SCTX line, the script of the old
    /// games (figure B.6). Returns the option's new value when it changed.
    pub(crate) fn script_tab(&self, ui: &mut egui::Ui, line: usize) -> Option<bool> {
        let doc = self.active_doc()?;
        let e = doc.tm.entries.get(line)?;
        if e.field_sig == "SCTX" {
            self.sctx_tab(ui, doc, e);
            return None;
        }
        let set = doc.scripts.as_ref();
        let script = set.and_then(|s| s.script(&e.edid));
        let (Some(set), Some(script)) = (set, script) else {
            ui.label(egui::RichText::new(self.i18n.t("script-tab-none")).weak());
            return None;
        };
        let Ok(p) = &script.pex else {
            return None;
        };
        let i = e.index.max(0) as usize;
        // 1: identity.
        ui.label(egui::RichText::new(self.i18n.localize(&e.comment)).strong());
        let uses = p.uses.get(i).map(Vec::len).unwrap_or(0);
        let shared = p.ident.get(i).copied().unwrap_or(false);
        ui.label(self.i18n.t_arg("script-uses", "n", &uses.to_string()));
        ui.label(self.i18n.t_arg(
            "script-shared",
            "yesno",
            &self.i18n.t(if shared { "word-yes" } else { "word-no" }),
        ));
        ui.add_space(4.0);
        // 3: the source.
        let game = doc.settings.game();
        let mut copy = self.config.scripts_copy_to_source;
        let changed = |ui: &mut egui::Ui, copy: &mut bool, enabled: bool| -> bool {
            ui.add_space(4.0);
            ui.add_enabled(
                enabled,
                egui::Checkbox::new(copy, self.i18n.t("script-copy-to-source")),
            )
            .changed()
        };
        let Some((file, text)) = set.source_text(&script.name, game) else {
            ui.label(egui::RichText::new(self.i18n.t("script-source-none")).weak());
            return changed(ui, &mut copy, false).then_some(copy);
        };
        let lines: Vec<&str> = text.lines().collect();
        let quoted = format!("\"{}\"", crate::scripts::papyrus_escape(&e.source));
        let Some(at) = lines.iter().position(|l| l.contains(&quoted)) else {
            ui.label(
                egui::RichText::new(self.i18n.t_arg("script-source-nomatch", "file", &file)).weak(),
            );
            return changed(ui, &mut copy, true).then_some(copy);
        };
        ui.label(self.i18n.t_args(
            "script-source-at",
            &[("file", &file), ("line", &(at + 1).to_string())],
        ));
        source_lines(
            ui,
            &lines,
            at.saturating_sub(2),
            (at + 3).min(lines.len()),
            at,
            false,
        );
        changed(ui, &mut copy, true).then_some(copy)
    }

    /// The Script tab on a SCTX line (C-07, figure B.6): the script, its
    /// source read-only with the instruction in blue, the lines it gives
    /// and where the literal is in the compiled form.
    fn sctx_tab(&self, ui: &mut egui::Ui, doc: &Doc, e: &crate::tm::TmEntry) {
        ui.label(egui::RichText::new(format!("{} · {:08X}", e.edid, e.formid)).strong());
        ui.label(self.i18n.localize(&e.comment));
        let Some(map) = self.sctx_scripts(doc) else {
            return;
        };
        let Some(blocks) = map.get(&e.formid) else {
            return;
        };
        // The block and segment of the line: the rank runs over the
        // record's scripts.
        let mut rank = 0i32;
        let mut found = None;
        for (b, (src, comp)) in blocks.iter().enumerate() {
            let a = crate::plugin::sctx::analyze(src, comp.as_deref());
            let n = a.segments.len() as i32;
            if e.index > rank && e.index <= rank + n {
                found = Some((b, a, (e.index - rank - 1) as usize));
                break;
            }
            rank += n;
        }
        let Some((b, a, si)) = found else {
            return;
        };
        let (src, _) = &blocks[b];
        let decode = |bytes: &[u8]| {
            crate::tm::encoding::with_input(doc.enc.codec(), || {
                crate::tm::encoding::decode(bytes, false)
            })
        };
        let seg = &a.segments[si];
        let lit = &a.literals[seg.literal];
        // The other lines of the script and the compiled form first: the
        // source, long, comes last (T-C07-03).
        let list: Vec<String> = a
            .segments
            .iter()
            .map(|s| format!("\"{}\"", decode(&s.bytes)))
            .collect();
        ui.add(
            egui::Label::new(self.i18n.t_arg("sctx-lines", "list", &list.join(" · ")))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        match lit.compiled {
            Some((pos, _)) => {
                ui.label(self.i18n.t_args(
                    "sctx-compiled-at",
                    &[
                        ("pos", &pos.to_string()),
                        ("len", &(lit.end - lit.start).to_string()),
                    ],
                ));
            }
            None => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    self.i18n.t("sctx-compiled-none"),
                );
            }
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(self.i18n.t("sctx-source-title")).weak());
        let text = decode(src.strip_suffix(&[0]).unwrap_or(src));
        let lines: Vec<&str> = text.lines().collect();
        egui::ScrollArea::vertical()
            .id_salt("ximod-sctx-source")
            .max_height(180.0)
            .show(ui, |ui| {
                source_lines(ui, &lines, 0, lines.len(), lit.line, true);
            });
    }

    /// The scripts of the active plugin, read once: FormID → its script
    /// blocks `(SCTX, SCDA)`.
    fn sctx_scripts(&self, doc: &Doc) -> Option<std::sync::Arc<SctxMap>> {
        let mut cache = self.sctx_cache.borrow_mut();
        if let Some((p, m)) = cache.as_ref()
            && p == &doc.path
        {
            return Some(m.clone());
        }
        let bytes = std::fs::read(&doc.path).ok()?;
        let plugin = crate::plugin::parse(&bytes).ok()?;
        let mut map: SctxMap = std::collections::HashMap::new();
        for r in &plugin.records {
            let sigs: Vec<&str> = r.subrecords.iter().map(|s| s.sig.as_str()).collect();
            let blocks = crate::plugin::sctx::blocks(&sigs);
            if blocks.is_empty() {
                continue;
            }
            map.insert(
                r.formid,
                blocks
                    .into_iter()
                    .map(|(_, scda, sctx)| {
                        (
                            r.subrecords[sctx].data.clone(),
                            scda.map(|i| r.subrecords[i].data.clone()),
                        )
                    })
                    .collect(),
            );
        }
        let m = std::sync::Arc::new(map);
        *cache = Some((doc.path.clone(), m.clone()));
        Some(m)
    }
}

/// FormID → the script blocks of the record, `(SCTX, SCDA)`.
pub(crate) type SctxMap = std::collections::HashMap<u32, Vec<(Vec<u8>, Option<Vec<u8>>)>>;

/// Source lines `from..to`, monospace, numbered: line `at` in blue (and
/// scrolled to), the comments in grey.
fn source_lines(
    ui: &mut egui::Ui,
    lines: &[&str],
    from: usize,
    to: usize,
    at: usize,
    scroll: bool,
) {
    let blue = ui.visuals().hyperlink_color;
    let weak = ui.visuals().weak_text_color();
    let normal = ui.visuals().text_color();
    egui::Frame::default()
        .fill(ui.visuals().extreme_bg_color)
        .inner_margin(egui::Margin::same(4.0))
        .show(ui, |ui| {
            for (k, l) in lines[from..to].iter().enumerate() {
                let n = from + k + 1;
                let trimmed = l.trim_start();
                let color = if from + k == at {
                    blue
                } else if trimmed.starts_with(';') || trimmed.starts_with('{') {
                    weak
                } else {
                    normal
                };
                let resp = ui.label(
                    egui::RichText::new(format!("{n:>5}  {}", l.trim_end()))
                        .monospace()
                        .color(color),
                );
                if scroll && from + k == at {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
            }
        });
}
