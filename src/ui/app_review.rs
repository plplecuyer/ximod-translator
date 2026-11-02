//! The Relecture column in the application (A-17): the commands of the row
//! menu (Traduction = Relecture, Relecture = Traduction, Effacer la
//! relecture) as undoable operations, and the dialog Fichier ▸ Charger dans
//! la relecture….

use std::path::PathBuf;

use eframe::egui;

use super::XimodApp;
use crate::review::{self, LoadOptions, LoadPlan, Match};
use crate::tm::TranslationMemory;
use crate::undo::{EntryState, OpKind, op_from_before};

/// The commands of the Relecture sub-menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewCmd {
    Accept,
    FromTrans,
    Clear,
}

/// « Charger dans la relecture » while it is open.
#[derive(Default)]
pub(crate) struct ReviewDialog {
    /// The document it was opened for.
    pub doc_serial: u64,
    pub path: String,
    /// The copy read for `path`, or the reading error.
    copy: Option<(PathBuf, Result<TranslationMemory, String>)>,
    pub opts: LoadOptions,
    /// The preview of the current file and options.
    plan: Option<(LoadOptions, LoadPlan)>,
}

impl XimodApp {
    /// Run a command of the Relecture sub-menu on `lines` (R-A17-03/04):
    /// one undoable operation, its count in the status bar.
    pub(crate) fn review_command(&mut self, cmd: ReviewCmd, lines: &[usize]) {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = lines
            .iter()
            .filter_map(|&i| doc.tm.entries.get(i).map(|e| (i, EntryState::capture(e))))
            .collect();
        let (kind, changed) = match cmd {
            ReviewCmd::Accept => (
                OpKind::ReviewAccept,
                review::accept(&mut doc.tm.entries, lines),
            ),
            ReviewCmd::FromTrans => (
                OpKind::Review,
                review::from_trans(&mut doc.tm.entries, lines),
            ),
            ReviewCmd::Clear => (OpKind::Review, review::clear(&mut doc.tm.entries, lines)),
        };
        let op = op_from_before(kind, &before, &mut doc.tm.entries);
        if !op.changes.is_empty() {
            doc.undo.push(op);
            self.pinned_rows.extend(changed.iter().copied());
        }
        if cmd == ReviewCmd::Accept {
            self.refresh_anomaly_lines(&changed);
        }
        let n = changed.len().to_string();
        self.status = match cmd {
            ReviewCmd::Accept => self.i18n.t_arg("review-accepted", "n", &n),
            ReviewCmd::FromTrans => self.i18n.t_arg("review-copied", "n", &n),
            ReviewCmd::Clear => self.i18n.t_arg("review-cleared", "n", &n),
        };
    }

    /// Fichier ▸ Charger dans la relecture… (R-A17-05).
    pub(crate) fn open_review_dialog(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.status = self.i18n.t("status-no-doc");
            return;
        };
        let serial = doc.serial;
        let keep = self
            .review_dlg
            .as_ref()
            .filter(|d| d.doc_serial == serial)
            .map(|d| (d.path.clone(), d.opts));
        let (path, opts) = keep.unwrap_or_default();
        self.review_dlg = Some(ReviewDialog {
            doc_serial: serial,
            path,
            opts,
            ..Default::default()
        });
    }

    /// The dialog, each frame while it is open.
    pub(crate) fn review_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.review_dlg.take() else {
            return;
        };
        let Some(doc_idx) = self.docs.iter().position(|d| d.serial == dlg.doc_serial) else {
            return; // the document was closed
        };
        // Read the file when its path changed, then the preview.
        let path = PathBuf::from(dlg.path.trim());
        let path_ok = !dlg.path.trim().is_empty() && path.is_file();
        if path_ok && dlg.copy.as_ref().is_none_or(|(p, _)| p != &path) {
            // A text document's copy, in its format and paired by key
            // (D-08); any translation file otherwise.
            let doc = &self.docs[doc_idx];
            let paired = doc
                .text_format()
                .filter(|_| doc.is_text())
                .and_then(|fmt| crate::textfmt::pair_copy(&fmt, &doc.tm.entries, &path).ok());
            let copy = match paired {
                Some(tm) => Ok(tm),
                None => review::read_copy(&path),
            };
            dlg.copy = Some((path.clone(), copy));
            dlg.plan = None;
        }
        if !path_ok {
            dlg.copy = None;
            dlg.plan = None;
        }
        if let Some((_, Ok(copy))) = &dlg.copy
            && dlg.plan.as_ref().is_none_or(|(o, _)| *o != dlg.opts)
        {
            let p = review::plan(&self.docs[doc_idx].tm.entries, copy, dlg.opts);
            dlg.plan = Some((dlg.opts, p));
        }
        let doc_name = self.docs[doc_idx].file_name();
        let t = |k: &str| self.i18n.t(k);
        let title = self.i18n.t_arg("review-title", "doc", &doc_name);
        let (file_l, browse_l, match_l) =
            (t("review-file"), t("settings-browse"), t("review-match"));
        let (m_rec, m_text) = (t("review-match-record"), t("review-match-text"));
        let (only_l, over_l) = (t("review-only-diff"), t("review-overwrite"));
        let (preview_l, note_l) = (t("review-preview-title"), t("review-preview-note"));
        let (load_l, cancel_l) = (t("review-load"), t("btn-cancel"));
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        let preview: Option<Result<(String, String), String>> = match (&dlg.copy, &dlg.plan) {
            (Some((_, Err(e))), _) => Some(Err(self.i18n.localize(e))),
            (_, Some((_, p))) => Some(Ok((
                self.i18n.t_args(
                    "review-preview",
                    &[
                        ("total", &fmt(p.total)),
                        ("matched", &fmt(p.matched)),
                        ("diff", &fmt(p.diff)),
                    ],
                ),
                self.i18n.t_args(
                    "review-preview-rest",
                    &[
                        ("unmatched", &fmt(p.unmatched)),
                        ("existing", &fmt(p.existing)),
                    ],
                ),
            ))),
            _ => None,
        };
        let can_load = dlg
            .plan
            .as_ref()
            .is_some_and(|(_, p)| !p.changes.is_empty());
        let mut browse = false;
        let mut decided: Option<bool> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(640.0);
                egui::Grid::new("ximod-review-load")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(&file_l);
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut dlg.path).desired_width(480.0));
                            if ui.button(&browse_l).clicked() {
                                browse = true;
                            }
                        });
                        ui.end_row();
                        ui.label(&match_l);
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut dlg.opts.matching, Match::Record, &m_rec);
                            ui.radio_value(&mut dlg.opts.matching, Match::Text, &m_text);
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut dlg.opts.only_diff, &only_l);
                            ui.checkbox(&mut dlg.opts.overwrite, &over_l);
                        });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&preview_l).strong());
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &preview {
                        Some(Ok((a, b))) => {
                            ui.label(a);
                            ui.label(egui::RichText::new(b).weak());
                        }
                        Some(Err(e)) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        None => {
                            ui.label(egui::RichText::new("—").weak());
                        }
                    }
                    ui.label(egui::RichText::new(&note_l).weak().small());
                });
                ui.add_space(8.0);
                // Charger needs something to load; Annuler is always there.
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                ui.horizontal(|ui| {
                    let load = ui.add_enabled(
                        can_load,
                        egui::Button::new(egui::RichText::new(&load_l).strong()),
                    );
                    if (load.clicked() || enter) && can_load {
                        decided = Some(true);
                    }
                    if ui.button(&cancel_l).clicked() || esc {
                        decided = Some(false);
                    }
                });
            });
        if browse {
            let mut d = rfd::FileDialog::new()
                .add_filter(
                    self.i18n.t("review-filter"),
                    &[
                        "xml",
                        "json",
                        "sst",
                        "eet",
                        "dsd",
                        "ximdb",
                        "strings",
                        "dlstrings",
                        "ilstrings",
                    ],
                )
                .add_filter(self.i18n.t("dialog-all-files"), &["*"]);
            if let Some(dir) = path.parent().filter(|p| p.is_dir()) {
                d = d.set_directory(dir);
            } else if let Some(dir) = self.docs[doc_idx].path.parent() {
                d = d.set_directory(dir);
            }
            if let Some(p) = d.pick_file() {
                dlg.path = p.display().to_string();
            }
        }
        match decided {
            Some(true) => {
                if let Some((_, plan)) = dlg.plan.take() {
                    self.review_commit(doc_idx, &plan);
                }
            }
            Some(false) => {}
            None => self.review_dlg = Some(dlg),
        }
    }

    /// Write a loaded review, one undoable operation; the column shows.
    fn review_commit(&mut self, doc_idx: usize, plan: &LoadPlan) {
        self.close_edit_session();
        let Some(doc) = self.docs.get_mut(doc_idx) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = plan
            .changes
            .iter()
            .filter_map(|(i, _)| doc.tm.entries.get(*i).map(|e| (*i, EntryState::capture(e))))
            .collect();
        let changed = review::commit(&mut doc.tm.entries, plan);
        let op = op_from_before(OpKind::Review, &before, &mut doc.tm.entries);
        if !op.changes.is_empty() {
            doc.undo.push(op);
        }
        self.pinned_rows.extend(changed.iter().copied());
        if !self.config.show_review {
            self.config.show_review = true;
            let _ = self.config.save();
        }
        self.notify_ok(
            self.i18n
                .t_arg("review-loaded", "n", &changed.len().to_string()),
        );
    }

    /// Open a review edit session on line `idx` (the cell of the column or
    /// the field of the edit window): one « Relecture » operation when it
    /// closes.
    pub(crate) fn open_review_session(&mut self, idx: usize) {
        self.close_edit_session();
        let active = self.active;
        if let Some(e) = self.docs.get(active).and_then(|d| d.tm.entries.get(idx)) {
            self.edit_session = Some(super::EditSession {
                doc: active,
                before: vec![(idx, EntryState::capture(e))],
                review: true,
            });
        }
    }
}
