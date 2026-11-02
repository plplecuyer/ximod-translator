//! The history of the translations in the application (A-18): the
//! operations of the documents become history entries, written next to the
//! mod's dictionary when it is saved; the Historique window (Outils), the
//! Historique tab of the edit window and the returns to an earlier version.

use std::collections::HashMap;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use super::XimodApp;
use crate::history::{Entry, Key, key_of};
use crate::undo::{EntryState, OpKind, op_from_before};

/// Which lines the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HistScope {
    Line,
    #[default]
    Selection,
    All,
}

/// The Historique window while it is open.
#[derive(Default)]
pub(crate) struct HistoryWindow {
    pub open: bool,
    pub needs_place: bool,
    pub scope: HistScope,
    /// Only the entries of this operation (code), when set.
    pub op: Option<String>,
    /// The entry chosen in the table (its index in the document's history).
    pub selected: Option<usize>,
    /// The line « Afficher l'historique de la ligne » asked for.
    pub line: Option<usize>,
    pub message: Option<String>,
    /// The keys of the document's lines, built once per document.
    keys: Option<(u64, HashMap<Key, usize>)>,
}

/// « Revenir à cette version » waiting for its confirmation (the source
/// changed since, R-A18-08).
pub(crate) struct HistoryConfirm {
    pub serial: u64,
    pub entry: Entry,
}

impl XimodApp {
    /// Turn the operations of every document into history entries; called
    /// every frame (R-A18-02).
    pub(crate) fn drain_history(&mut self) {
        // « Tous les mods » (B-25): its operations first carried out in
        // their files, whose logs follow.
        self.all_mods_tick();
        let (enabled, max) = (self.config.history_enabled, self.config.history_max);
        let logs: Vec<Vec<crate::undo::LogEvent>> =
            self.docs.iter_mut().map(|d| d.undo.take_log()).collect();
        // The translations written reach the other open files (B-25).
        self.propagate_across(&logs);
        for (doc, log) in self.docs.iter_mut().zip(&logs) {
            if enabled && !log.is_empty() && doc.all.is_none() {
                doc.history.record(log, &doc.tm.entries, max);
            }
        }
    }

    /// The `.ximhist` of a document: next to its mod's dictionary, when it
    /// has one (a target language).
    pub(crate) fn history_file(&self, idx: usize) -> Option<std::path::PathBuf> {
        let doc = self.docs.get(idx)?;
        if doc.settings.target.trim().is_empty() {
            return None;
        }
        let dict = crate::dict_stack::DictStack::mod_dict_path(
            &doc.path,
            &doc.settings,
            &self.config.mod_base_dir(),
        );
        Some(crate::history::file_for(&dict))
    }

    /// Read the history file of a document once, when it is looked at.
    pub(crate) fn ensure_history(&mut self, idx: usize) {
        self.drain_history();
        if let Some(path) = self.history_file(idx)
            && let Some(doc) = self.docs.get_mut(idx)
        {
            doc.history.load(&path);
        }
    }

    /// Write the entries of the session with the mod's dictionary (D-46).
    pub(crate) fn save_history(&mut self, idx: usize, mod_dict: &std::path::Path) {
        self.drain_history();
        let max = self.config.history_max;
        let path = crate::history::file_for(mod_dict);
        let Some(doc) = self.docs.get_mut(idx) else {
            return;
        };
        if doc.history.unsaved == 0 {
            return;
        }
        if let Err(e) = crate::slow_io("history write", || doc.history.save(&path, max)) {
            tracing::warn!("history not written to {}: {e}", path.display());
        }
    }

    /// The name of an entry's operation (« Saisie », « Annulation : … »).
    pub(crate) fn history_op_label(&self, e: &Entry) -> String {
        let name = match OpKind::from_code(&e.op) {
            Some(k) => self
                .i18n
                .t_arg(k.i18n_key(), "name", e.arg.as_deref().unwrap_or("")),
            None => e.op.clone(),
        };
        if e.u {
            self.i18n.t_arg("op-undo-entry", "op", &name)
        } else {
            name
        }
    }

    /// Date and time of an entry, in the order of the language.
    pub(crate) fn history_time(&self, e: &Entry) -> String {
        match crate::history::iso_parts(&e.t) {
            Some((y, mo, d, h, mi, s)) => self.i18n.t_args(
                "date-time",
                &[
                    ("y", &y),
                    ("mo", &mo),
                    ("d", &d),
                    ("h", &h),
                    ("mi", &mi),
                    ("s", &s),
                ],
            ),
            None => e.t.clone(),
        }
    }

    /// The line of the active document an entry is about.
    fn history_line_of(&mut self, key: &Key) -> Option<usize> {
        self.history_keys();
        self.history_win
            .keys
            .as_ref()
            .and_then(|(_, m)| m.get(key).copied())
    }

    /// Build the keys of the active document's lines, once per document.
    fn history_keys(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let fresh = !matches!(&self.history_win.keys, Some((s, _)) if *s == doc.serial);
        if fresh {
            let map = doc
                .tm
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| (key_of(e), i))
                .collect();
            self.history_win.keys = Some((doc.serial, map));
        }
    }

    /// Set lines to given versions (translation and status), provenance
    /// « historique », as one undoable operation (R-A18-06/07).
    fn history_apply(&mut self, versions: Vec<(usize, String, String)>) -> usize {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return 0;
        };
        let before: Vec<(usize, EntryState)> = versions
            .iter()
            .filter_map(|(i, _, _)| doc.tm.entries.get(*i).map(|e| (*i, EntryState::capture(e))))
            .collect();
        for (i, trans, status) in &versions {
            let Some(e) = doc.tm.entries.get_mut(*i) else {
                continue;
            };
            e.trans = trans.clone();
            e.set_translated_flags(!e.trans.is_empty());
            let st = crate::status::Status::from_key(status)
                .filter(|s| s.applicable_to(e))
                .unwrap_or(if e.trans.is_empty() {
                    crate::status::Status::ToTranslate
                } else {
                    crate::status::Status::Pending
                });
            crate::status::set(e, st);
            e.prov = if e.trans.is_empty() {
                String::new()
            } else {
                crate::provenance::HISTORY.to_string()
            };
        }
        let op = op_from_before(OpKind::History, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        let touched = op.touched();
        doc.undo.push(op);
        self.pinned_rows.extend(touched.iter().copied());
        self.refresh_anomaly_lines(&touched);
        n
    }

    /// « Revenir à la traduction précédente » on `lines` (Ctrl+Alt+Z): each
    /// takes the translation and status it had before its last change in
    /// the history; lines without one are counted.
    pub(crate) fn history_revert_previous(&mut self, lines: &[usize]) {
        if lines.is_empty() {
            self.status = self.i18n.t("status-keys-no-selection");
            return;
        }
        let active = self.active;
        self.ensure_history(active);
        let Some(doc) = self.docs.get(active) else {
            return;
        };
        let mut versions = Vec::new();
        let mut skipped = 0usize;
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            match doc.history.previous_of(&key_of(e), &e.trans) {
                Some(h) => versions.push((i, h.b.clone(), h.sb.clone())),
                None => skipped += 1,
            }
        }
        let n = self.history_apply(versions);
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        self.status = if skipped > 0 {
            self.i18n.t_args(
                "history-restored-skipped",
                &[("n", &fmt(n)), ("skipped", &fmt(skipped))],
            )
        } else {
            self.i18n.t_arg("history-restored", "n", &fmt(n))
        };
    }

    /// « Revenir à cette version » of an entry, after the confirmation when
    /// the source of the line changed since.
    pub(crate) fn history_restore(&mut self, entry: &Entry, confirmed: bool) {
        let Some(line) = self.history_line_of(&entry.k) else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let source_changed = !entry.s.is_empty() && doc.tm.entries[line].source != entry.s;
        if source_changed && !confirmed {
            self.history_confirm = Some(HistoryConfirm {
                serial: doc.serial,
                entry: entry.clone(),
            });
            return;
        }
        let n = self.history_apply(vec![(line, entry.a.clone(), entry.sa.clone())]);
        self.status = self.i18n.t_arg(
            "history-restored",
            "n",
            &crate::ui::export_dialog::fmt_count(&self.i18n, n),
        );
    }

    /// The confirmation of a return to a version whose source changed.
    pub(crate) fn history_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(c) = self.history_confirm.as_ref() else {
            return;
        };
        if self.docs.get(self.active).map(|d| d.serial) != Some(c.serial) {
            self.history_confirm = None;
            return;
        }
        let (yes, no) = (self.i18n.t("history-restore"), self.i18n.t("btn-cancel"));
        let text = self.i18n.t("history-source-changed");
        let mut choice = None;
        egui::Window::new(self.i18n.t("history-restore"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(text);
                ui.add_space(8.0);
                let (p, c) = super::confirm_buttons(ui, &yes, &no);
                if p {
                    choice = Some(true);
                } else if c {
                    choice = Some(false);
                }
            });
        match choice {
            Some(true) => {
                if let Some(c) = self.history_confirm.take() {
                    self.history_restore(&c.entry, true);
                }
            }
            Some(false) => self.history_confirm = None,
            None => {}
        }
    }

    /// Outils ▸ Historique… (or « Afficher l'historique de la ligne »).
    pub(crate) fn open_history_window(&mut self, line: Option<usize>) {
        if self.docs.is_empty() {
            self.status = self.i18n.t("status-no-doc");
            return;
        }
        let active = self.active;
        self.ensure_history(active);
        let w = &mut self.history_win;
        if !w.open {
            w.needs_place = true;
        }
        w.open = true;
        w.line = line;
        if line.is_some() {
            w.scope = HistScope::Line;
        }
        w.selected = None;
        w.message = None;
    }

    /// The entries of the window: their indices in the history, most recent
    /// first, after the scope and the operation filter.
    fn history_rows(&mut self) -> Vec<usize> {
        let active = self.active;
        let lines: Option<Vec<usize>> = match self.history_win.scope {
            HistScope::All => None,
            HistScope::Line => Some(
                self.history_win
                    .line
                    .or(self.selection.cursor)
                    .into_iter()
                    .collect(),
            ),
            HistScope::Selection => Some(self.selection.targets()),
        };
        let Some(doc) = self.docs.get(active) else {
            return Vec::new();
        };
        let keys: Option<std::collections::HashSet<Key>> = lines.map(|ls| {
            ls.iter()
                .filter_map(|&i| doc.tm.entries.get(i).map(key_of))
                .collect()
        });
        let op = self.history_win.op.clone();
        doc.history
            .entries
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, e)| keys.as_ref().is_none_or(|k| k.contains(&e.k)))
            .filter(|(_, e)| op.as_ref().is_none_or(|o| &e.op == o))
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn history_window(&mut self, ctx: &egui::Context) {
        if !self.history_win.open {
            return;
        }
        if self.docs.is_empty() {
            self.history_win.open = false;
            return;
        }
        self.drain_history();
        let id = egui::ViewportId::from_hash_of("ximod-history");
        let geo = self.config.hist_win;
        let just_opened = self.history_win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(640.0), geo[3].max(360.0)]))
        } else {
            None
        };
        let file = self.docs[self.active].file_name();
        let title = self.i18n.t_arg("history-title", "doc", &file);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([640.0, 360.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.history_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.history_body(ui) {
                    close = true;
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.hist_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.history_win.open = false;
            let _ = self.config.save();
        }
    }

    /// Body of the window; returns `true` to close it.
    fn history_body(&mut self, ui: &mut egui::Ui) -> bool {
        let rows = self.history_rows();
        self.history_keys();
        let t = |k: &str| self.i18n.t(k);
        let n_sel = self.selection.targets().len();
        let (lines_l, line_l, all_l) = (
            t("history-lines"),
            t("history-scope-line"),
            t("history-scope-all"),
        );
        let sel_l = self
            .i18n
            .t_arg("history-scope-selection", "n", &n_sel.to_string());
        let (op_l, op_all) = (t("history-op"), t("history-op-all"));
        let cols = [
            t("history-col-time"),
            t("history-col-line"),
            t("history-col-before"),
            t("history-col-after"),
            t("history-col-op"),
        ];
        let empty_l = t("history-empty-value");
        let (restore_l, prev_l, export_l, close_l) = (
            t("history-restore"),
            t("history-restore-previous-sel"),
            t("history-export"),
            t("btn-close"),
        );
        let differs_l = t("history-source-differs");
        // The operations present, for the filter.
        let active = self.active;
        let mut ops: Vec<String> = Vec::new();
        for e in &self.docs[active].history.entries {
            if !ops.contains(&e.op) {
                ops.push(e.op.clone());
            }
        }
        let op_names: Vec<(String, String)> = ops
            .iter()
            .map(|o| {
                let name = OpKind::from_code(o)
                    .map(|k| self.i18n.t_arg(k.i18n_key(), "name", ""))
                    .unwrap_or_else(|| o.clone());
                (o.clone(), name.trim().to_string())
            })
            .collect();

        // The texts of the visible rows.
        struct Row {
            idx: usize,
            time: String,
            line: String,
            before: String,
            after: String,
            op: String,
            differs: bool,
        }
        let doc = &self.docs[active];
        let keys = self.history_win.keys.as_ref().map(|(_, m)| m);
        let shown: Vec<Row> = rows
            .iter()
            .map(|&i| {
                let e = &doc.history.entries[i];
                let differs = keys
                    .and_then(|m| m.get(&e.k))
                    .and_then(|&l| doc.tm.entries.get(l))
                    .is_some_and(|l| !e.s.is_empty() && l.source != e.s);
                Row {
                    idx: i,
                    time: self.history_time(e),
                    line: format!("{}:{} {}", e.k.0, e.k.1, e.k.3),
                    before: if e.b.is_empty() {
                        empty_l.clone()
                    } else {
                        e.b.clone()
                    },
                    after: if e.a.is_empty() {
                        empty_l.clone()
                    } else {
                        e.a.clone()
                    },
                    op: self.history_op_label(e),
                    differs,
                }
            })
            .collect();
        let total = doc.history.entries.len();
        let file_note = match self.history_file(active) {
            Some(p) => self.i18n.t_args(
                "history-file",
                &[
                    (
                        "file",
                        &p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    ),
                    ("shown", &shown.len().to_string()),
                    ("total", &total.to_string()),
                ],
            ),
            None => self.i18n.t_args(
                "history-file-none",
                &[
                    ("shown", &shown.len().to_string()),
                    ("total", &total.to_string()),
                ],
            ),
        };
        let selected = self.history_win.selected.filter(|s| rows.contains(s));
        let detail = selected.map(|i| {
            let e = &self.docs[active].history.entries[i];
            let st = |k: &str| {
                crate::status::Status::from_key(k)
                    .map(|s| self.i18n.t(s.legend_key()))
                    .unwrap_or_default()
            };
            let prov = {
                let tmp = crate::tm::TmEntry {
                    trans: e.a.clone(),
                    prov: e.p.clone(),
                    ..Default::default()
                };
                crate::provenance::full_label(&self.i18n, &tmp)
                    .unwrap_or_else(|| self.i18n.t("prov-none"))
            };
            let mut d = self.i18n.t_args(
                "history-detail",
                &[
                    ("before", &st(&e.sb)),
                    ("after", &st(&e.sa)),
                    ("provenance", &prov),
                ],
            );
            if let (Some(rb), Some(ra)) = (&e.rb, &e.ra) {
                d = self.i18n.dot(
                    &d,
                    &self.i18n.t_args(
                        "history-detail-review",
                        &[
                            ("before", if rb.is_empty() { &empty_l } else { rb }),
                            ("after", if ra.is_empty() { &empty_l } else { ra }),
                        ],
                    ),
                );
            }
            d
        });
        let disabled_note = (!self.config.history_enabled).then(|| t("history-disabled"));

        let mut close = false;
        let mut clicked: Option<usize> = None;
        let mut do_restore = false;
        let mut do_prev = false;
        let mut do_export = false;
        ui.horizontal(|ui| {
            ui.strong(&lines_l);
            ui.radio_value(&mut self.history_win.scope, HistScope::Line, &line_l);
            ui.radio_value(&mut self.history_win.scope, HistScope::Selection, &sel_l);
            ui.radio_value(&mut self.history_win.scope, HistScope::All, &all_l);
            ui.add_space(16.0);
            ui.label(&op_l);
            let cur = self
                .history_win
                .op
                .as_ref()
                .and_then(|o| op_names.iter().find(|(c, _)| c == o))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| op_all.clone());
            egui::ComboBox::from_id_salt("ximod-history-op")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.history_win.op, None, &op_all);
                    for (code, name) in &op_names {
                        ui.selectable_value(&mut self.history_win.op, Some(code.clone()), name);
                    }
                });
        });
        ui.add_space(4.0);
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let avail = ui.available_height() - 90.0;
        TableBuilder::new(ui)
            .striped(true)
            .sense(egui::Sense::click())
            .max_scroll_height(avail.max(80.0))
            .column(Column::initial(140.0).at_least(90.0))
            .column(Column::initial(170.0).at_least(90.0))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::initial(170.0).at_least(90.0))
            .header(row_h, |mut h| {
                for c in &cols {
                    h.col(|ui| {
                        ui.strong(c);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, shown.len(), |mut row| {
                    let r = &shown[row.index()];
                    row.set_selected(selected == Some(r.idx));
                    row.col(|ui| {
                        ui.add(egui::Label::new(&r.time).selectable(false));
                    });
                    row.col(|ui| {
                        let mut text = egui::RichText::new(&r.line);
                        if r.differs {
                            text = text.color(ui.visuals().warn_fg_color);
                        }
                        let resp = ui.add(egui::Label::new(text).truncate().selectable(false));
                        if r.differs {
                            resp.on_hover_text(&differs_l);
                        }
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&r.before).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&r.after).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&r.op).truncate().selectable(false));
                    });
                    if row.response().clicked() {
                        clicked = Some(r.idx);
                    }
                });
            });
        ui.add_space(6.0);
        if let Some(d) = &detail {
            ui.label(egui::RichText::new(d).weak());
        }
        ui.label(egui::RichText::new(&file_note).weak().small());
        if let Some(n) = &disabled_note {
            ui.label(egui::RichText::new(n).weak().small());
        }
        if let Some(m) = &self.history_win.message {
            ui.label(egui::RichText::new(m).small());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    selected.is_some(),
                    egui::Button::new(egui::RichText::new(&restore_l).strong()),
                )
                .clicked()
            {
                do_restore = true;
            }
            if ui
                .add_enabled(n_sel > 0, egui::Button::new(&prev_l))
                .clicked()
            {
                do_prev = true;
            }
            if ui
                .add_enabled(!shown.is_empty(), egui::Button::new(&export_l))
                .clicked()
            {
                do_export = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(&close_l).clicked() {
                    close = true;
                }
            });
        });

        if let Some(i) = clicked {
            self.history_win.selected = Some(i);
            // The line in the grid.
            let key = self.docs[active].history.entries[i].k.clone();
            if let Some(line) = self.history_line_of(&key) {
                if !self.selection.rows.contains(&line)
                    || self.history_win.scope != HistScope::Selection
                {
                    self.selection.cursor = Some(line);
                    if self.history_win.scope != HistScope::Selection {
                        self.selection.set_single(line);
                    }
                }
                self.grid_scroll_to = Some(line);
            }
        }
        if do_restore && let Some(i) = selected {
            let entry = self.docs[active].history.entries[i].clone();
            self.history_restore(&entry, false);
            self.history_win.message = Some(self.status.clone());
        }
        if do_prev {
            let targets = self.selection.targets();
            self.history_revert_previous(&targets);
            self.history_win.message = Some(self.status.clone());
        }
        if do_export {
            let entries: Vec<Entry> = rows
                .iter()
                .map(|&i| self.docs[active].history.entries[i].clone())
                .collect();
            self.history_export(&entries, &cols);
        }
        close
    }

    /// Exporter… : the entries shown, as CSV (R-A18-05).
    fn history_export(&mut self, entries: &[Entry], cols: &[String; 5]) {
        let name = self.docs[self.active]
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(path) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-csv"), &["csv"])
            .set_file_name(format!("{name}_history.csv"))
            .save_file()
        else {
            return;
        };
        let titles: Vec<String> = vec![
            cols[0].clone(),
            cols[1].clone(),
            cols[2].clone(),
            cols[3].clone(),
            self.i18n.t("history-col-status-before"),
            self.i18n.t("history-col-status-after"),
            self.i18n.t("col-provenance"),
            cols[4].clone(),
        ];
        let refs: Vec<&Entry> = entries.iter().collect();
        let csv = crate::history::to_csv(&refs, &titles, |e| self.history_op_label(e));
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.history_win.message = Some(match std::fs::write(&path, csv) {
            Ok(()) => self.i18n.t_args(
                "history-exported",
                &[("n", &entries.len().to_string()), ("file", &file)],
            ),
            Err(e) => self.i18n.t_arg("err-save", "error", &e.to_string()),
        });
    }

    /// The Historique tab of the edit window: the entries of the line, most
    /// recent first, each with « Revenir à cette version ».
    pub(crate) fn history_tab(&mut self, ui: &mut egui::Ui, line: usize) {
        let active = self.active;
        if self.docs[active].history.file.is_none() {
            self.ensure_history(active);
        }
        let Some(e) = self.docs[active].tm.entries.get(line) else {
            return;
        };
        let key = key_of(e);
        let source = e.source.clone();
        let entries: Vec<Entry> = self.docs[active].history.of_key(&key).cloned().collect();
        if entries.is_empty() {
            ui.label(egui::RichText::new(self.i18n.t("history-none")).weak());
            return;
        }
        let restore_l = self.i18n.t("history-restore");
        let empty_l = self.i18n.t("history-empty-value");
        let differs_l = self.i18n.t("history-source-differs");
        let mut pick: Option<Entry> = None;
        egui::ScrollArea::vertical()
            .id_salt(("ximod-edit-history", line))
            .show(ui, |ui| {
                for h in &entries {
                    ui.horizontal(|ui| {
                        let differs = !h.s.is_empty() && h.s != source;
                        let mut time = egui::RichText::new(self.history_time(h)).small();
                        if differs {
                            time = time.color(ui.visuals().warn_fg_color);
                        }
                        let r = ui.label(time);
                        if differs {
                            r.on_hover_text(&differs_l);
                        }
                        ui.label(egui::RichText::new(self.history_op_label(h)).small().weak());
                        if ui.small_button(&restore_l).clicked() {
                            pick = Some(h.clone());
                        }
                        let after = if h.a.is_empty() { &empty_l } else { &h.a };
                        ui.add(egui::Label::new(after).truncate())
                            .on_hover_text(after);
                    });
                }
            });
        if let Some(h) = pick {
            self.history_restore(&h, false);
        }
    }
}

fn read_geometry_of(ctx: &egui::Context) -> Option<(f32, f32, f32, f32)> {
    super::read_geometry(ctx)
}
