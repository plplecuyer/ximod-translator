//! Aide › Journal… (E-01): the window of the session's journal, the button
//! of the status bar, the « Journal : … » chip of the filter bar, and the
//! helpers that record an entry.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::journal::{Entry, Kind, Level, View};

/// The window while it is open, and what it keeps of the session.
#[derive(Default)]
pub(crate) struct JournalWindow {
    pub open: bool,
    pub needs_place: bool,
    /// Filters (R-E01-05).
    pub view: View,
    /// Selected entries (numbers).
    pub selected: BTreeSet<u64>,
    /// Last entry clicked (Shift+click).
    anchor: Option<u64>,
    /// The document of an entry with lines, by number (Voir les lignes).
    pub paths: HashMap<u64, PathBuf>,
    /// The confirmation of « Effacer » is open.
    confirm_clear: bool,
}

/// The « Journal : … » chip: its label, the lines, the document.
pub(crate) type JournalChip = (String, crate::filter::AnomalySet, PathBuf);

impl XimodApp {
    // ------------------------------------------------------------ entries

    /// The name of document `idx` for the journal.
    pub(crate) fn journal_doc_name(&self, idx: usize) -> Option<String> {
        self.docs.get(idx).map(|d| d.file_name())
    }

    /// An entry about the active document, if any.
    pub(crate) fn journal_add(&mut self, level: Level, kind: Kind, text: &str) {
        let doc = self.journal_doc_name(self.active);
        self.journal_for(level, kind, doc.as_deref(), text);
    }

    /// An entry about a named document (or none).
    pub(crate) fn journal_for(&mut self, level: Level, kind: Kind, doc: Option<&str>, text: &str) {
        let text = self.i18n.localize(text);
        if text.trim().is_empty() {
            return;
        }
        self.journal.push(level, kind, doc, &text);
    }

    /// An entry about lines of document `idx` (R-E01-04).
    pub(crate) fn journal_rows(
        &mut self,
        level: Level,
        kind: Kind,
        idx: usize,
        text: &str,
        rows: Vec<usize>,
    ) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let (name, path) = (doc.file_name(), doc.path.clone());
        let text = self.i18n.localize(text);
        let id = self
            .journal
            .push_rows(level, kind, Some(&name), &text, rows);
        if self.journal.get(id).is_some_and(|e| !e.rows.is_empty()) {
            self.journal_win.paths.insert(id, path);
        }
    }

    /// The dictionary stack applied to document `idx`: one level per entry,
    /// with its lines, then the total (R-E01-02).
    pub(crate) fn journal_stack(&mut self, idx: usize, report: &crate::dict_stack::StackReport) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let name = doc.file_name();
        for t in crate::journal::stack_texts(&self.i18n, report, &doc.tm.entries) {
            self.journal.push(Level::Info, Kind::Dicts, Some(&name), &t);
        }
    }

    /// The opening of document `idx` (R-E01-02).
    pub(crate) fn journal_opened(&mut self, idx: usize, secs: f32) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let lines = doc.tm.entries.iter().filter(|e| !e.unused).count();
        let enc = doc
            .enc
            .used
            .as_ref()
            .map(|c| self.i18n.t(c.page.name_key()));
        let text =
            crate::journal::opened_text(&self.i18n, &doc.settings, lines, enc.as_deref(), secs);
        let name = doc.file_name();
        self.journal
            .push(Level::Info, Kind::Open, Some(&name), &text);
    }

    // -------------------------------------------------------------- window

    /// Aide › Journal… / Ctrl+Shift+J / the status bar button.
    pub(crate) fn open_journal_window(&mut self) {
        let w = &mut self.journal_win;
        w.needs_place = !w.open;
        w.open = true;
        self.journal.mark_read();
    }

    /// The label of a level, in the interface language.
    fn journal_level_label(&self, l: Level) -> String {
        self.i18n.t(l.i18n_key())
    }

    /// The tab-separated text of the given entries.
    fn journal_tsv<'a>(&self, entries: impl IntoIterator<Item = &'a Entry>) -> String {
        crate::journal::tsv(entries, |l| self.journal_level_label(l))
    }

    pub(crate) fn journal_window(&mut self, ctx: &egui::Context) {
        if !self.journal_win.open {
            return;
        }
        // While it is open, nothing is « unread ».
        self.journal.mark_read();
        let id = egui::ViewportId::from_hash_of("ximod-journal");
        let geo = self.config.journal_win;
        let just_opened = self.journal_win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(560.0), geo[3].max(280.0)]))
        } else {
            None
        };
        let builder = {
            let b = egui::ViewportBuilder::default()
                .with_title(self.i18n.t("journal-title"))
                .with_min_inner_size([560.0, 280.0]);
            match place {
                Some((pos, size)) => b.with_position(pos).with_inner_size(size).with_active(true),
                None => b,
            }
        };
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
            self.journal_win.needs_place = false;
            if !self.journal_win.confirm_clear
                && vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            self.journal_body(vctx);
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.journal_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.journal_win.open = false;
            self.journal_win.confirm_clear = false;
            let _ = self.config.save();
        }
    }

    fn journal_body(&mut self, vctx: &egui::Context) {
        let t = |k: &str| self.i18n.t(k);
        let (lbl_level, lbl_all, lbl_warn, lbl_err) = (
            t("journal-level"),
            t("journal-level-all"),
            t("journal-level-warn"),
            t("journal-level-err"),
        );
        let (lbl_doc_all, lbl_filter) = (t("journal-doc-all"), t("journal-filter"));
        let (lbl_copy_sel, lbl_copy_all, lbl_save, lbl_clear) = (
            t("journal-copy-sel"),
            t("journal-copy-all"),
            t("journal-save"),
            t("journal-clear"),
        );
        let lbl_rows = t("journal-rows");
        let (lbl_time, lbl_doc, lbl_msg) = (
            t("journal-col-time"),
            t("journal-col-doc"),
            t("journal-col-msg"),
        );
        let docs = self.journal.docs();
        let shown: Vec<u64> = self
            .journal_win
            .view
            .filter(&self.journal)
            .map(|e| e.id)
            .collect();
        let (_, warn, err) = self.journal.counts();
        let count = self.i18n.t_args(
            "journal-count",
            &[
                ("n", &shown.len().to_string()),
                ("warn", &warn.to_string()),
                ("err", &err.to_string()),
            ],
        );
        let trimmed = (self.journal.trimmed() > 0).then(|| {
            self.i18n
                .t_arg("journal-trimmed", "n", &self.journal.trimmed().to_string())
        });
        let open_paths: Vec<PathBuf> = self.docs.iter().map(|d| d.path.clone()).collect();
        let mut show_rows: Option<u64> = None;
        let mut copy: Option<String> = None;
        let mut save = false;
        // The buttons at the bottom.
        egui::TopBottomPanel::bottom("ximod-journal-buttons").show(vctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let any_sel = !self.journal_win.selected.is_empty();
                if ui
                    .add_enabled(any_sel, egui::Button::new(&lbl_copy_sel))
                    .clicked()
                {
                    copy = Some(
                        self.journal_tsv(
                            shown
                                .iter()
                                .filter(|id| self.journal_win.selected.contains(id))
                                .filter_map(|id| self.journal.get(*id)),
                        ),
                    );
                }
                if ui
                    .add_enabled(!shown.is_empty(), egui::Button::new(&lbl_copy_all))
                    .clicked()
                {
                    copy =
                        Some(self.journal_tsv(shown.iter().filter_map(|id| self.journal.get(*id))));
                }
                if ui
                    .add_enabled(!shown.is_empty(), egui::Button::new(&lbl_save))
                    .clicked()
                {
                    save = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.journal.is_empty(), egui::Button::new(&lbl_clear))
                        .clicked()
                    {
                        self.journal_win.confirm_clear = true;
                    }
                });
            });
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(vctx, |ui| {
            // The filters (1).
            ui.horizontal_wrapped(|ui| {
                ui.label(&lbl_level);
                let cur = match self.journal_win.view.min_level {
                    None | Some(Level::Info) => &lbl_all,
                    Some(Level::Warn) => &lbl_warn,
                    Some(Level::Error) => &lbl_err,
                };
                egui::ComboBox::from_id_salt("journal-level")
                    .selected_text(cur.as_str())
                    .show_ui(ui, |ui| {
                        let v = &mut self.journal_win.view.min_level;
                        ui.selectable_value(v, None, &lbl_all);
                        ui.selectable_value(v, Some(Level::Warn), &lbl_warn);
                        ui.selectable_value(v, Some(Level::Error), &lbl_err);
                    });
                let cur_doc = self
                    .journal_win
                    .view
                    .doc
                    .clone()
                    .unwrap_or_else(|| lbl_doc_all.clone());
                egui::ComboBox::from_id_salt("journal-doc")
                    .selected_text(cur_doc)
                    .show_ui(ui, |ui| {
                        let v = &mut self.journal_win.view.doc;
                        ui.selectable_value(v, None, &lbl_doc_all);
                        for d in &docs {
                            ui.selectable_value(v, Some(d.clone()), d);
                        }
                    });
                ui.label(&lbl_filter);
                ui.add(
                    egui::TextEdit::singleline(&mut self.journal_win.view.text)
                        .desired_width(180.0),
                );
                ui.label(egui::RichText::new(&count).weak());
            });
            if let Some(t) = &trimmed {
                ui.label(egui::RichText::new(t).weak().italics());
            }
            ui.separator();
            // The list, virtualized, following the last entry.
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            let time_w = 64.0;
            let level_w = 22.0;
            let doc_w = 150.0;
            ui.horizontal(|ui| {
                ui.add_sized(
                    [time_w, row_h],
                    egui::Label::new(egui::RichText::new(&lbl_time).strong()),
                );
                ui.add_space(level_w);
                ui.add_sized(
                    [doc_w, row_h],
                    egui::Label::new(egui::RichText::new(&lbl_doc).strong()),
                );
                ui.label(egui::RichText::new(&lbl_msg).strong());
            });
            let warn_c = ui.visuals().warn_fg_color;
            let err_c = ui.visuals().error_fg_color;
            let sel_fill = ui.visuals().selection.bg_fill;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show_rows(ui, row_h, shown.len(), |ui, range| {
                    for i in range {
                        let Some(e) = self.journal.get(shown[i]) else {
                            continue;
                        };
                        let selected = self.journal_win.selected.contains(&e.id);
                        let (icon, color) = match e.level {
                            Level::Info => (crate::icons::INFO, ui.visuals().weak_text_color()),
                            Level::Warn => (crate::icons::WARNING, warn_c),
                            Level::Error => (crate::icons::X_CIRCLE, err_c),
                        };
                        // The row is allocated first, so the button drawn on it
                        // afterwards keeps its own click.
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_h),
                            egui::Sense::click(),
                        );
                        if selected {
                            ui.painter()
                                .rect_filled(rect, 2.0, sel_fill.gamma_multiply(0.45));
                        } else if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, 2.0, ui.visuals().faint_bg_color);
                        }
                        let mut row = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        row.add_sized(
                            [time_w, row_h],
                            egui::Label::new(crate::journal::time_text(e.at)).selectable(false),
                        );
                        row.add_sized(
                            [level_w, row_h],
                            egui::Label::new(egui::RichText::new(icon).color(color))
                                .selectable(false),
                        )
                        .on_hover_text(self.journal_level_label(e.level));
                        row.add_sized(
                            [doc_w, row_h],
                            egui::Label::new(e.doc.as_deref().unwrap_or(""))
                                .truncate()
                                .selectable(false),
                        );
                        let has_rows = !e.rows.is_empty()
                            && self
                                .journal_win
                                .paths
                                .get(&e.id)
                                .is_some_and(|p| open_paths.contains(p));
                        if has_rows && row.small_button(&lbl_rows).clicked() {
                            show_rows = Some(e.id);
                        }
                        let mut text = egui::RichText::new(&e.text);
                        if e.level != Level::Info {
                            text = text.color(color);
                        }
                        row.add(egui::Label::new(text).truncate().selectable(false));
                        let resp = resp.on_hover_text(&e.text);
                        if resp.clicked() {
                            let m = ui.input(|i| i.modifiers);
                            let id = e.id;
                            self.journal_select(id, m, &shown);
                        }
                    }
                });
        });
        // Ctrl+C: the selected entries.
        if !self.journal_win.selected.is_empty()
            && vctx.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::Copy)))
        {
            copy = Some(
                self.journal_tsv(
                    shown
                        .iter()
                        .filter(|id| self.journal_win.selected.contains(id))
                        .filter_map(|id| self.journal.get(*id)),
                ),
            );
        }
        if let Some(text) = copy {
            vctx.copy_text(text);
        }
        if save {
            let text = self.journal_tsv(shown.iter().filter_map(|id| self.journal.get(*id)));
            self.journal_save(&text);
        }
        if let Some(id) = show_rows {
            self.journal_show_rows(id);
        }
        if self.journal_win.confirm_clear {
            self.journal_confirm_clear(vctx);
        }
    }

    /// Click, Ctrl+click, Shift+click in the list.
    fn journal_select(&mut self, id: u64, m: egui::Modifiers, shown: &[u64]) {
        select(&mut self.journal_win, id, m, shown);
    }

    /// Enregistrer…: the tab-separated text, UTF-8, in `logs/` by default.
    fn journal_save(&mut self, text: &str) {
        let dir = crate::config::paths::get().logs_dir.clone();
        let name = crate::journal::file_name(
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        );
        let Some(path) = rfd::FileDialog::new()
            .set_directory(&dir)
            .set_file_name(&name)
            .add_filter(self.i18n.t("journal-file-filter"), &["txt"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => {
                let msg = self
                    .i18n
                    .t_arg("journal-saved", "path", &path.display().to_string());
                self.notify_ok(msg);
            }
            Err(e) => self.notify_err(
                self.i18n
                    .colon(&self.i18n.t("journal-save-error"), &e.to_string()),
            ),
        }
    }

    /// « Effacer le journal de la session ? » (confirmation, R-E01-05).
    fn journal_confirm_clear(&mut self, vctx: &egui::Context) {
        let title = self.i18n.t("journal-clear");
        let ask = self.i18n.t("journal-clear-ask");
        let (yes, no) = (self.i18n.t("journal-clear"), self.i18n.t("btn-cancel"));
        let mut choice = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(vctx, |ui| {
                ui.label(&ask);
                ui.add_space(10.0);
                let (p, c) = super::confirm_buttons(ui, &yes, &no);
                if p {
                    choice = Some(true);
                } else if c {
                    choice = Some(false);
                }
            });
        match choice {
            Some(true) => {
                self.journal.clear();
                self.journal_win.selected.clear();
                self.journal_win.anchor = None;
                self.journal_win.paths.clear();
                self.journal_win.confirm_clear = false;
            }
            Some(false) => self.journal_win.confirm_clear = false,
            None => {}
        }
    }

    /// Voir les lignes: the document's tab, the « Journal : … » chip
    /// (R-E01-04).
    pub(crate) fn journal_show_rows(&mut self, id: u64) {
        let Some(e) = self.journal.get(id) else {
            return;
        };
        let Some(path) = self.journal_win.paths.get(&id).cloned() else {
            return;
        };
        let Some(idx) = self.docs.iter().position(|d| d.path == path) else {
            return;
        };
        let short: String = e.text.chars().take(24).collect();
        let label = if short.chars().count() < e.text.chars().count() {
            self.i18n.t_arg("text-cut", "text", &short)
        } else {
            short
        };
        let set: std::collections::HashSet<usize> = e.rows.iter().copied().collect();
        self.active = idx;
        self.filter_journal = Some((label, crate::filter::AnomalySet(Arc::new(set)), path));
    }

    /// The lines of the chip, when it is about the active document.
    pub(crate) fn journal_filter_lines(&self) -> Option<crate::filter::AnomalySet> {
        let (_, set, path) = self.filter_journal.as_ref()?;
        (self.docs.get(self.active)?.path == *path).then(|| set.clone())
    }

    // ---------------------------------------------------------- status bar

    /// The « Journal » button of the status bar (R-E01-06): true when
    /// clicked.
    pub(crate) fn journal_status_button(&self, ui: &mut egui::Ui, compact: bool) -> bool {
        if !self.journal.has_activity() {
            return false;
        }
        let (w, e) = self.journal.unread();
        let label = self.i18n.t("statusbar-journal");
        let (text, color) = if e > 0 {
            (
                self.i18n.t_args(
                    "statusbar-journal-n",
                    &[("n", &e.to_string()), ("icon", crate::icons::X_CIRCLE)],
                ),
                Some(ui.visuals().error_fg_color),
            )
        } else if w > 0 {
            (
                self.i18n.t_args(
                    "statusbar-journal-n",
                    &[("n", &w.to_string()), ("icon", crate::icons::WARNING)],
                ),
                Some(ui.visuals().warn_fg_color),
            )
        } else {
            (label.clone(), None)
        };
        let text = if compact {
            match (w + e, color) {
                (0, _) => crate::icons::LIST_BULLETS.to_string(),
                (n, _) => format!("{} {n}", crate::icons::LIST_BULLETS),
            }
        } else {
            text
        };
        let mut rt = egui::RichText::new(text);
        if let Some(c) = color {
            rt = rt.color(c);
        }
        let key = super::shortcut_text(ui.ctx(), &super::MENU_SHORTCUTS.journal, &self.i18n);
        ui.add(egui::Button::new(rt).small())
            .on_hover_text(format!("{label} ({key})"))
            .clicked()
    }
}

/// The list's selection: click, Ctrl+click (toggle), Shift+click (range from
/// the last entry clicked).
fn select(w: &mut JournalWindow, id: u64, m: egui::Modifiers, shown: &[u64]) {
    if m.shift
        && let Some(a) = w.anchor
        && let (Some(i), Some(j)) = (
            shown.iter().position(|x| *x == a),
            shown.iter().position(|x| *x == id),
        )
    {
        let (lo, hi) = (i.min(j), i.max(j));
        if !m.command {
            w.selected.clear();
        }
        w.selected.extend(shown[lo..=hi].iter().copied());
        return;
    }
    if m.command {
        if !w.selected.remove(&id) {
            w.selected.insert(id);
        }
    } else {
        w.selected.clear();
        w.selected.insert(id);
    }
    w.anchor = Some(id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_click_ctrl_and_shift() {
        let mut app_w = JournalWindow::default();
        let shown = [1u64, 2, 3, 4, 5];
        app_w.selected.insert(9);
        let none = egui::Modifiers::NONE;
        let ctrl = egui::Modifiers::COMMAND;
        let shift = egui::Modifiers::SHIFT;
        select(&mut app_w, 2, none, &shown);
        assert_eq!(app_w.selected.iter().copied().collect::<Vec<_>>(), [2]);
        select(&mut app_w, 4, ctrl, &shown);
        assert_eq!(app_w.selected.iter().copied().collect::<Vec<_>>(), [2, 4]);
        select(&mut app_w, 4, ctrl, &shown);
        assert_eq!(app_w.selected.iter().copied().collect::<Vec<_>>(), [2]);
        // Shift+click: the range from the last entry clicked (4).
        select(&mut app_w, 5, shift, &shown);
        assert_eq!(app_w.selected.iter().copied().collect::<Vec<_>>(), [4, 5]);
        select(&mut app_w, 2, none, &shown);
        select(&mut app_w, 5, shift, &shown);
        assert_eq!(
            app_w.selected.iter().copied().collect::<Vec<_>>(),
            [2, 3, 4, 5]
        );
        // Ctrl+Shift+click adds the range to the selection.
        select(&mut app_w, 1, ctrl, &shown);
        select(&mut app_w, 1, ctrl.plus(shift), &shown);
        assert_eq!(
            app_w.selected.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
    }
}
