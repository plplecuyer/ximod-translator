//! Fichier ▸ Enregistrer une partie… (B-10): the modal dialog — scope,
//! statuses, records and field, the file and its preview — and the write.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use eframe::egui;

use super::XimodApp;
use crate::savepart::{self, Breakdown, Choices, Refusal, Scope};

/// The dialog while it is open.
pub(crate) struct SavePartDialog {
    pub choices: Choices,
    pub path: String,
    /// The lines of each scope, taken at the opening.
    selection: Vec<usize>,
    view: Vec<usize>,
    all: Vec<usize>,
    /// The record signatures of the document and their lines.
    records: Vec<(String, usize)>,
    /// The preview of the current choices.
    preview: Option<(Choices, Vec<usize>, Breakdown)>,
    pub error: Option<String>,
}

/// The choices of the dialog per file, for the session (R-B10-03).
pub(crate) type SavePartMemory = HashMap<PathBuf, Choices>;

impl SavePartDialog {
    fn scope_lines(&self) -> &[usize] {
        match self.choices.scope {
            Scope::Selection => &self.selection,
            Scope::View => &self.view,
            Scope::All => &self.all,
        }
    }
}

impl XimodApp {
    /// Fichier ▸ Enregistrer une partie….
    pub(crate) fn open_savepart_dialog(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.status = self.i18n.t("status-no-doc");
            return;
        };
        let all: Vec<usize> = (0..doc.tm.entries.len()).collect();
        let mut selection = self.selection.targets();
        selection.sort_unstable();
        let view = self.visible.clone();
        let records = savepart::records_of(&doc.tm.entries);
        let mut choices = self
            .savepart_memory
            .get(&doc.path)
            .cloned()
            .unwrap_or_default();
        // A record the document no longer has is dropped.
        if let Some(r) = choices.records.as_mut() {
            r.retain(|k| records.iter().any(|(s, _)| s == k));
            if r.is_empty() {
                choices.records = None;
            }
        }
        if choices.scope == Scope::Selection && selection.len() < 2 {
            choices.scope = Scope::All;
        }
        let dir = self
            .last_save_dir
            .clone()
            .or_else(|| doc.path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let path = savepart::default_path(&doc.path, &dir, &self.i18n.t("savepart-suffix"));
        self.savepart_dlg = Some(SavePartDialog {
            choices,
            path: path.display().to_string(),
            selection,
            view,
            all,
            records,
            preview: None,
            error: None,
        });
    }

    /// The dialog, each frame while it is open.
    pub(crate) fn savepart_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut dlg) = self.savepart_dlg.take() else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        // The preview, recomputed when a choice changed.
        if dlg
            .preview
            .as_ref()
            .is_none_or(|(c, _, _)| *c != dlg.choices)
        {
            let lines = savepart::lines(&doc.tm.entries, dlg.scope_lines(), &dlg.choices);
            let b = savepart::breakdown(&doc.tm.entries, &lines);
            dlg.preview = Some((dlg.choices.clone(), lines, b));
        }
        let fields = savepart::fields_of(&doc.tm.entries, dlg.choices.records.as_ref());
        let title = self.i18n.t_arg("savepart-title", "doc", &doc.file_name());
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        let colors = self.config.status_colors;
        let n = dlg.preview.as_ref().map(|(_, l, _)| l.len()).unwrap_or(0);
        let mut save = false;
        let mut cancel = false;
        let mut browse = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.set_max_width(640.0);
                let label_w = 90.0;
                let head = |ui: &mut egui::Ui, text: String| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(label_w, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(label_w);
                            ui.label(egui::RichText::new(text).strong());
                        },
                    );
                };
                // ---- 1. The scope.
                ui.horizontal_wrapped(|ui| {
                    head(ui, self.i18n.t("savepart-scope"));
                    for (scope, key, count) in [
                        (
                            Scope::Selection,
                            "savepart-scope-selection",
                            dlg.selection.len(),
                        ),
                        (Scope::View, "savepart-scope-view", dlg.view.len()),
                        (Scope::All, "savepart-scope-all", dlg.all.len()),
                    ] {
                        let enabled = scope != Scope::Selection || count > 1;
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.radio_value(
                                &mut dlg.choices.scope,
                                scope,
                                self.i18n.t_arg(key, "n", &fmt(count)),
                            );
                        });
                    }
                });
                // ---- 2. The statuses.
                ui.horizontal(|ui| {
                    head(ui, self.i18n.t("savepart-statuses"));
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for s in crate::status::Status::ALL {
                        let (r, g, b) = colors[s.index()];
                        let on = dlg.choices.statuses[s.index()];
                        let (fill, fg) = if on {
                            (
                                egui::Color32::from_rgb(r, g, b),
                                egui::Color32::from_rgb(35, 35, 35),
                            )
                        } else {
                            (
                                ui.visuals().widgets.inactive.weak_bg_fill,
                                ui.visuals().weak_text_color(),
                            )
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(s.glyph()).color(fg).size(13.0),
                                )
                                .fill(fill)
                                .selected(on)
                                .min_size(egui::vec2(22.0, 20.0)),
                            )
                            .on_hover_text(self.i18n.t(s.legend_key()))
                            .clicked()
                        {
                            dlg.choices.statuses[s.index()] = !on;
                        }
                    }
                });
                // ---- 3. The records and the field.
                ui.horizontal_wrapped(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    head(ui, self.i18n.t("savepart-records"));
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (rec, count) in &dlg.records {
                        let on = dlg
                            .choices
                            .records
                            .as_ref()
                            .is_some_and(|r| r.contains(rec));
                        if ui
                            .selectable_label(on, egui::RichText::new(rec).monospace())
                            .on_hover_text(fmt(*count))
                            .clicked()
                        {
                            let set = dlg.choices.records.get_or_insert_with(BTreeSet::new);
                            if on {
                                set.remove(rec);
                            } else {
                                set.insert(rec.clone());
                            }
                            if set.is_empty() {
                                dlg.choices.records = None;
                            }
                        }
                    }
                    if ui
                        .selectable_label(
                            dlg.choices.records.is_none(),
                            self.i18n.t("savepart-records-all"),
                        )
                        .clicked()
                    {
                        dlg.choices.records = None;
                    }
                });
                if dlg
                    .choices
                    .field
                    .as_ref()
                    .is_some_and(|f| !fields.contains(f))
                {
                    dlg.choices.field = None;
                }
                ui.horizontal(|ui| {
                    head(ui, self.i18n.t("savepart-field"));
                    let all_l = self.i18n.t("savepart-field-all");
                    egui::ComboBox::from_id_salt("ximod-savepart-field")
                        .width(120.0)
                        .selected_text(dlg.choices.field.clone().unwrap_or(all_l.clone()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut dlg.choices.field, None, all_l);
                            for f in &fields {
                                ui.selectable_value(
                                    &mut dlg.choices.field,
                                    Some(f.clone()),
                                    egui::RichText::new(f).monospace(),
                                );
                            }
                        });
                });
                ui.separator();
                // ---- 4. The file, the preview and the action.
                ui.horizontal(|ui| {
                    head(ui, self.i18n.t("savepart-file"));
                    let w = (ui.available_width() - 110.0).max(200.0);
                    if ui
                        .add(egui::TextEdit::singleline(&mut dlg.path).desired_width(w))
                        .changed()
                    {
                        dlg.error = None;
                    }
                    if ui.button(self.i18n.t("savepart-browse")).clicked() {
                        browse = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(label_w + ui.spacing().item_spacing.x);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(self.i18n.t("savepart-format-hint"))
                                .weak()
                                .small(),
                        );
                        ui.checkbox(
                            &mut dlg.choices.keep_source,
                            self.i18n.t("savepart-keep-source"),
                        )
                        .on_hover_text(self.i18n.t("savepart-keep-source-tip"));
                    });
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &dlg.preview {
                        Some((_, lines, b)) if !lines.is_empty() => {
                            let recs: Vec<String> = b
                                .records
                                .iter()
                                .map(|(r, n)| format!("{r} {}", fmt(*n)))
                                .collect();
                            let sts: Vec<String> = b
                                .statuses
                                .iter()
                                .map(|(s, n)| {
                                    format!("{} {}", self.i18n.t(s.legend_key()), fmt(*n))
                                })
                                .collect();
                            let detail = self.i18n.t_args(
                                "savepart-preview-detail",
                                &[("records", &recs.join(", ")), ("statuses", &sts.join(", "))],
                            );
                            ui.label(self.i18n.dash(
                                &self.i18n.t_arg("savepart-preview", "n", &fmt(lines.len())),
                                &detail,
                            ));
                        }
                        _ => {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                self.i18n.t("savepart-none"),
                            );
                        }
                    }
                    ui.label(
                        egui::RichText::new(self.i18n.t("savepart-note"))
                            .weak()
                            .small(),
                    );
                });
                ui.add_space(4.0);
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                // One row high: a bare layout would take the rest of the
                // window's height.
                let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 4.0);
                ui.allocate_ui_with_layout(
                    row,
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button(self.i18n.t("btn-cancel")).clicked() || esc {
                            cancel = true;
                        }
                        let ok = n > 0 && !dlg.path.trim().is_empty();
                        if (ui
                            .add_enabled(
                                ok,
                                egui::Button::new(
                                    egui::RichText::new(self.i18n.t_arg(
                                        "savepart-save",
                                        "n",
                                        &fmt(n),
                                    ))
                                    .strong(),
                                ),
                            )
                            .clicked()
                            || enter)
                            && ok
                        {
                            save = true;
                        }
                    },
                );
            });
        if browse {
            let cur = PathBuf::from(dlg.path.trim());
            let mut fd = rfd::FileDialog::new().set_parent(frame);
            if let Some(dir) = cur.parent().filter(|d| d.is_dir()) {
                fd = fd.set_directory(dir);
            }
            if let Some(name) = cur.file_name() {
                fd = fd.set_file_name(name.to_string_lossy().to_string());
            }
            let picked = fd
                .add_filter(self.i18n.t("dialog-pivot-json"), &["json"])
                .add_filter(self.i18n.t("dialog-sst"), &["sst"])
                .add_filter(self.i18n.t("dialog-eet"), &["eet"])
                .add_filter(self.i18n.t("dialog-dsd-json"), &["dsd"])
                .add_filter(self.i18n.t("dialog-csv"), &["csv"])
                .add_filter(self.i18n.t("dialog-xml"), &["xml"])
                .save_file();
            if let Some(p) = picked {
                dlg.path = p.display().to_string();
                dlg.error = None;
            }
        }
        if cancel {
            self.remember_savepart(&dlg);
            return;
        }
        if save {
            match self.savepart_write(&dlg) {
                Ok(()) => {
                    self.remember_savepart(&dlg);
                    return;
                }
                Err(e) => dlg.error = Some(e),
            }
        }
        self.savepart_dlg = Some(dlg);
    }

    /// The choices kept for the file (R-B10-03).
    fn remember_savepart(&mut self, dlg: &SavePartDialog) {
        if let Some(doc) = self.docs.get(self.active) {
            self.savepart_memory
                .insert(doc.path.clone(), dlg.choices.clone());
        }
    }

    /// Write the part (R-B10-02/03): the document is left as it is.
    fn savepart_write(&mut self, dlg: &SavePartDialog) -> Result<(), String> {
        let path = PathBuf::from(dlg.path.trim());
        match savepart::check_path(&path) {
            Err(Refusal::Strings) => return Err(self.i18n.t("savepart-strings")),
            Err(Refusal::Unknown(ext)) => {
                return Err(self.i18n.t_arg("savepart-ext", "ext", &ext));
            }
            Ok(()) => {}
        }
        let Some(doc) = self.docs.get(self.active) else {
            return Ok(());
        };
        let Some((_, lines, _)) = &dlg.preview else {
            return Ok(());
        };
        let json = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let history: &[crate::history::Entry] = if json { &doc.history.entries } else { &[] };
        let part = savepart::memory(&doc.tm, lines, dlg.choices.keep_source, history);
        let n = part.entries.len();
        crate::slow_io("save part", || crate::tm::save(&part, &path))
            .map_err(|e| self.i18n.t_arg("err-save", "error", &e.to_string()))?;
        tracing::info!("save part: {n} lines to {}", path.display());
        if let Some(dir) = path.parent() {
            self.last_save_dir = Some(dir.to_path_buf());
        }
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        self.notify_ok(self.i18n.t_args(
            "savepart-done",
            &[
                ("n", &crate::ui::export_dialog::fmt_count(&self.i18n, n)),
                ("file", &file),
            ],
        ));
        Ok(())
    }
}
