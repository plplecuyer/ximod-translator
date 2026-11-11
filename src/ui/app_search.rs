//! Outils ▸ Recherche avancée… (A-19): the window, its presets, its
//! preview, and the search applied to the grid as a filter chip.

use crate::ui::theme::ComboField as _;
use std::sync::Arc;

use eframe::egui;

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::filter::SearchFilter;
use crate::search::{self, Errors, Found, Groups, N_FIELDS, Preset, Spec};

/// The window while it is open.
#[derive(Default)]
pub(crate) struct SearchWindow {
    pub open: bool,
    pub needs_place: bool,
    pub spec: Spec,
    /// The presets, loaded at the first opening.
    pub presets: Option<Vec<Preset>>,
    /// The preset shown in the list (index in `presets`).
    pub preset: Option<usize>,
    /// « Enregistrer… »: the name being typed.
    pub naming: Option<String>,
    pub confirm_delete: bool,
    /// The last change of a criterion (the preview waits 250 ms).
    pub changed_at: Option<f64>,
    /// The spec compiled, or the errors of its fields.
    compiled: Option<(Spec, Result<Arc<search::Compiled>, Errors>)>,
    /// The preview: (document serial, revision, spec) and the result.
    preview: Option<((u64, u64, Spec), Found)>,
    pub message: Option<String>,
}

impl SearchWindow {
    /// The compiled spec, recompiled when the spec changed.
    fn compiled(&mut self) -> &Result<Arc<search::Compiled>, Errors> {
        if self.compiled.as_ref().is_none_or(|(s, _)| *s != self.spec) {
            let c = search::compile(&self.spec).map(Arc::new);
            self.compiled = Some((self.spec.clone(), c));
        }
        &self.compiled.as_ref().unwrap().1
    }
}

/// The search applied to the grid: the document it was applied to, the
/// filter, and its number of lines (the chip).
pub(crate) type AppliedSearch = (u64, SearchFilter, usize);

impl XimodApp {
    /// Outils ▸ Recherche avancée… / Ctrl+Alt+F: open, or bring forward.
    pub(crate) fn open_search_window(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        if self.search_win.presets.is_none() {
            let shipped = crate::data::find_data_file(search::PRESETS_FILE);
            self.search_win.presets = Some(search::load(shipped.as_deref(), &presets_path()));
        }
        self.search_win.needs_place = !self.search_win.open;
        self.search_win.open = true;
    }

    /// The chip's search, when it belongs to the active document.
    pub(crate) fn active_search(&self) -> Option<&AppliedSearch> {
        let serial = self.docs.get(self.active)?.serial;
        self.search_filter.as_ref().filter(|(s, _, _)| *s == serial)
    }

    /// The window, each frame while it is open.
    pub(crate) fn search_window(&mut self, ctx: &egui::Context) {
        if !self.search_win.open {
            return;
        }
        if self.docs.is_empty() {
            self.search_win.open = false;
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-search");
        let geo = self.config.search_win;
        let just_opened = self.search_win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(700.0), geo[3].max(440.0)]))
        } else {
            None
        };
        let file = self.docs[self.active].file_name();
        let title = self.i18n.t_arg("search-title", "doc", &file);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([700.0, 440.0]);
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
            self.search_win.needs_place = false;
            // Échap closes (an applied chip stays, R-A19-04), unless a
            // name or a confirmation is being asked.
            let busy = self.search_win.naming.is_some() || self.search_win.confirm_delete;
            if !busy && vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.search_body(ui) {
                    close = true;
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.search_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.search_win.open = false;
            self.search_win.naming = None;
            self.search_win.confirm_delete = false;
            let _ = self.config.save();
        }
    }

    /// The label of a preset: its message for a shipped one.
    fn preset_label(&self, p: &Preset) -> String {
        if p.key.is_empty() {
            p.name.clone()
        } else {
            self.i18n.t(&p.key)
        }
    }

    /// Body of the window; returns `true` to close it.
    fn search_body(&mut self, ui: &mut egui::Ui) -> bool {
        let now = ui.input(|i| i.time);
        let mut close = false;
        let mut changed = false;
        let labels: [String; N_FIELDS] = [
            self.i18n.t("search-source"),
            self.i18n.t("search-trans"),
            self.i18n.t("search-id"),
            self.i18n.t("search-record"),
            self.i18n.t("search-field"),
            self.i18n.t("search-keyword"),
        ];
        let (regex_tip, case_l, case_tip) = (
            self.i18n.t("filter-regex-tip"),
            self.i18n.t("search-case"),
            self.i18n.t("search-case-tip"),
        );
        // ---- 1. Presets.
        let presets = self.search_win.presets.clone().unwrap_or_default();
        let names: Vec<String> = presets.iter().map(|p| self.preset_label(p)).collect();
        let cur = self.search_win.preset.filter(|&i| i < presets.len());
        let mut picked: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.i18n.t("search-preset")).strong());
            egui::ComboBox::from_id_salt("ximod-search-preset")
                .width(240.0)
                .selected_text(cur.map(|i| names[i].as_str()).unwrap_or(""))
                .show_field(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        if ui.selectable_label(cur == Some(i), n).clicked() {
                            picked = Some(i);
                        }
                    }
                });
            if ui.button(self.i18n.t("search-preset-save")).clicked() {
                let name = cur
                    .filter(|&i| !presets[i].shipped)
                    .map(|i| presets[i].name.clone())
                    .unwrap_or_default();
                self.search_win.naming = Some(name);
                self.search_win.confirm_delete = false;
            }
            let can_delete = cur.is_some_and(|i| !presets[i].shipped);
            let del = ui.add_enabled(
                can_delete,
                egui::Button::new(self.i18n.t("search-preset-delete")),
            );
            let del = if cur.is_some() && !can_delete {
                del.on_disabled_hover_text(self.i18n.t("search-preset-shipped"))
            } else {
                del
            };
            if del.clicked() {
                self.search_win.confirm_delete = true;
                self.search_win.naming = None;
            }
        });
        if let Some(i) = picked {
            self.search_win.preset = Some(i);
            self.search_win.spec = presets[i].spec.clone();
            changed = true;
        }
        // The name of a preset being saved, or the deletion asked.
        let mut save_name: Option<String> = None;
        if let Some(mut name) = self.search_win.naming.take() {
            let mut keep = true;
            ui.horizontal(|ui| {
                ui.label(self.i18n.t("search-preset-name"));
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
                r.request_focus();
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                let ok = !name.trim().is_empty();
                if (ui
                    .add_enabled(ok, egui::Button::new(self.i18n.t("btn-ok")))
                    .clicked()
                    || enter)
                    && ok
                {
                    save_name = Some(name.trim().to_string());
                    keep = false;
                }
                if ui.button(self.i18n.t("btn-cancel")).clicked() || esc {
                    keep = false;
                }
            });
            if keep {
                self.search_win.naming = Some(name);
            }
        }
        if let Some(name) = save_name {
            self.search_save_preset(name);
        }
        if self.search_win.confirm_delete
            && let Some(i) = cur
        {
            let ask = self
                .i18n
                .t_arg("search-preset-delete-ask", "name", &names[i]);
            let mut answer: Option<bool> = None;
            ui.horizontal(|ui| {
                ui.label(ask);
                if ui.button(self.i18n.t("btn-yes")).clicked() {
                    answer = Some(true);
                }
                if ui.button(self.i18n.t("btn-no")).clicked() {
                    answer = Some(false);
                }
            });
            match answer {
                Some(true) => {
                    self.search_delete_preset(i);
                    self.search_win.confirm_delete = false;
                }
                Some(false) => self.search_win.confirm_delete = false,
                None => {}
            }
        }
        ui.separator();
        // ---- 2. The six fields.
        let errors: Errors = match self.search_win.compiled() {
            Err(e) => e.clone(),
            Ok(_) => Default::default(),
        };
        // The fields take the width left by the label and the two boxes.
        let field_w = (ui.available_width() - 340.0).max(260.0);
        // One row per field: the label in a fixed width, then the field at
        // its full width (a grid cell would cap the field at the width of
        // its first frame), then the two boxes.
        let label_w = 170.0;
        for i in 0..N_FIELDS {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(label_w, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(label_w);
                        ui.label(&labels[i]);
                    },
                );
                let spec = &mut self.search_win.spec;
                let mut edit = egui::TextEdit::singleline(&mut spec.texts[i])
                    .desired_width(field_w)
                    .font(egui::TextStyle::Monospace);
                if errors[i].is_some() {
                    edit = edit.text_color(ui.visuals().error_fg_color);
                }
                let r = ui.add(edit);
                let r = match &errors[i] {
                    Some(e) => r.on_hover_text(self.i18n.t_arg("filter-regex-error", "error", e)),
                    None => r,
                };
                if r.changed() {
                    changed = true;
                }
                if ui
                    .checkbox(&mut spec.regex[i], ".*")
                    .on_hover_text(&regex_tip)
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .checkbox(&mut spec.case[i], &case_l)
                    .on_hover_text(&case_tip)
                    .changed()
                {
                    changed = true;
                }
            });
        }
        ui.separator();
        // ---- 3. Groups and statuses.
        let groups_ok = self.search_win.spec.groups_possible()
            && matches!(self.search_win.compiled(), Ok(c) if c.groups_ready());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.i18n.t("search-groups")).strong());
            ui.add_enabled_ui(groups_ok, |ui| {
                let g = &mut self.search_win.spec.groups;
                for (v, k) in [
                    (Groups::None, "search-groups-none"),
                    (Groups::Same, "search-groups-same"),
                    (Groups::Differ, "search-groups-differ"),
                ] {
                    if ui.radio_value(g, v, self.i18n.t(k)).changed() {
                        changed = true;
                    }
                }
            })
            .response
            .on_disabled_hover_text(self.i18n.t("search-groups-tip"));
        });
        let colors = self.config.status_colors;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.i18n.t("filter-status")).strong());
            ui.spacing_mut().item_spacing.x = 3.0;
            for s in crate::status::Status::ALL {
                let (r, g, b) = colors[s.index()];
                let on = self.search_win.spec.statuses[s.index()];
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
                        egui::Button::new(egui::RichText::new(s.glyph()).color(fg).size(13.0))
                            .fill(fill)
                            .selected(on)
                            .min_size(egui::vec2(22.0, 20.0)),
                    )
                    .on_hover_text(self.i18n.t(s.legend_key()))
                    .clicked()
                {
                    self.search_win.spec.statuses[s.index()] = !on;
                    changed = true;
                }
            }
            ui.add_space(10.0);
            if ui
                .checkbox(
                    &mut self.search_win.spec.unused,
                    self.i18n.t("filter-unused"),
                )
                .changed()
            {
                changed = true;
            }
        });
        if changed {
            self.search_win.changed_at = Some(now);
            self.search_win.message = None;
        }
        // ---- 4. The preview, 250 ms after the last change.
        let found = self.search_preview(ui.ctx(), now);
        let total = self
            .docs
            .get(self.active)
            .map(|d| d.tm.entries.len())
            .unwrap_or(0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
            match (&found, errors.iter().any(Option::is_some)) {
                (_, true) => {
                    ui.colored_label(ui.visuals().error_fg_color, self.i18n.t("search-invalid"));
                }
                (Some(f), false) => {
                    let mut line = self.i18n.t_args(
                        "search-preview",
                        &[("n", &fmt(f.lines.len())), ("total", &fmt(total))],
                    );
                    if !f.pairs.is_empty() {
                        let list: Vec<String> = f
                            .pairs
                            .iter()
                            .map(|(a, b)| {
                                self.i18n
                                    .t_args("search-pair", &[("source", a), ("trans", b)])
                            })
                            .collect();
                        line = self.i18n.dash(
                            &line,
                            &self
                                .i18n
                                .t_arg("search-preview-pairs", "list", &list.join(", ")),
                        );
                    }
                    ui.label(line);
                }
                (None, false) => {
                    ui.label(egui::RichText::new("…").weak());
                }
            }
            ui.label(
                egui::RichText::new(self.i18n.t("search-hint"))
                    .weak()
                    .small(),
            );
        });
        if let Some(m) = &self.search_win.message {
            ui.label(egui::RichText::new(m).weak());
        }
        // ---- 5. Actions.
        let ready = found.is_some() && !errors.iter().any(Option::is_some);
        let n = found.as_ref().map(|f| f.lines.len()).unwrap_or(0);
        let enter = self.search_win.naming.is_none()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let apply = ui.add_enabled(
                ready,
                egui::Button::new(egui::RichText::new(self.i18n.t("search-apply")).strong()),
            );
            if (apply.clicked() || enter) && ready {
                self.search_apply();
            }
            if ui
                .add_enabled(
                    ready && n > 0,
                    egui::Button::new(self.i18n.t("search-select")),
                )
                .clicked()
            {
                self.search_select();
            }
            if ui
                .add_enabled(
                    ready && n > 0,
                    egui::Button::new(self.i18n.t("search-copy")),
                )
                .clicked()
            {
                self.search_copy(ui.ctx());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.t("btn-close")).clicked() {
                    close = true;
                }
            });
        });
        close
    }

    /// The preview of the current spec on the active document, recomputed
    /// a quarter of a second after the last change (R-A19-06).
    fn search_preview(&mut self, ctx: &egui::Context, now: f64) -> Option<Found> {
        let (serial, revision) = self
            .docs
            .get(self.active)
            .map(|d| (d.serial, d.undo.revision))?;
        let key = (serial, revision, self.search_win.spec.clone());
        if self
            .search_win
            .preview
            .as_ref()
            .is_some_and(|(k, _)| *k == key)
        {
            return self.search_win.preview.as_ref().map(|(_, f)| f.clone());
        }
        if let Some(at) = self.search_win.changed_at
            && now - at < 0.25
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
            return self.search_win.preview.as_ref().map(|(_, f)| f.clone());
        }
        self.search_win.changed_at = None;
        let compiled = match self.search_win.compiled() {
            Ok(c) => c.clone(),
            Err(_) => return None,
        };
        if compiled.needs_comments() {
            self.ensure_comment_texts(ctx);
        }
        let comments = self.comment_texts.as_ref().map(|(_, c)| c.clone());
        let doc = &self.docs[self.active];
        let found = crate::slow_io("advanced search", || {
            search::find(&compiled, &doc.tm.entries, comments.as_ref())
        });
        self.search_win.preview = Some((key, found.clone()));
        Some(found)
    }

    /// Appliquer à la grille (R-A19-03): the chip of the filter bar.
    fn search_apply(&mut self) {
        let Ok(c) = self.search_win.compiled().clone() else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let n = self
            .search_win
            .preview
            .as_ref()
            .map(|(_, f)| f.lines.len())
            .unwrap_or(0);
        self.search_filter = Some((doc.serial, SearchFilter(c), n));
        self.pinned_rows.clear();
        let msg = self.i18n.t_arg(
            "search-applied",
            "n",
            &crate::ui::export_dialog::fmt_count(&self.i18n, n),
        );
        self.search_win.message = Some(msg.clone());
        self.status = msg;
    }

    /// Sélectionner: the lines found become the grid's selection.
    fn search_select(&mut self) {
        let Some((_, f)) = self.search_win.preview.clone() else {
            return;
        };
        self.selection.rows = f.lines.iter().copied().collect();
        self.selection.cursor = f.lines.first().copied();
        self.selection.anchor = self.selection.cursor;
        self.grid_scroll_to = self.selection.cursor;
        let msg = self.i18n.t_arg(
            "search-selected",
            "n",
            &crate::ui::export_dialog::fmt_count(&self.i18n, f.lines.len()),
        );
        self.search_win.message = Some(msg.clone());
        self.status = msg;
    }

    /// Copier la liste: « FormID ⇥ EDID ⇥ source ⇥ traduction ».
    fn search_copy(&mut self, ctx: &egui::Context) {
        let Some((_, f)) = self.search_win.preview.clone() else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        ctx.copy_text(search::copy_list(&doc.tm.entries, &f.lines));
        let msg = self.i18n.t_arg(
            "search-copied",
            "n",
            &crate::ui::export_dialog::fmt_count(&self.i18n, f.lines.len()),
        );
        self.search_win.message = Some(msg.clone());
        self.status = msg;
    }

    /// Enregistrer… (R-A19-05): a user preset, replacing one of the same
    /// name.
    fn search_save_preset(&mut self, name: String) {
        let spec = self.search_win.spec.clone();
        let presets = self.search_win.presets.get_or_insert_with(Vec::new);
        let p = Preset {
            name: name.clone(),
            key: String::new(),
            spec,
            shipped: false,
        };
        let idx = match presets.iter().position(|q| q.name == name) {
            Some(i) => {
                presets[i] = p;
                i
            }
            None => {
                presets.push(p);
                presets.len() - 1
            }
        };
        self.search_win.preset = Some(idx);
        let res = search::save_user(presets, &presets_path());
        self.search_win.message = Some(match res {
            Ok(()) => self.i18n.t_arg("search-preset-saved", "name", &name),
            Err(e) => self.i18n.t_arg("err-generic", "error", &e),
        });
    }

    /// Supprimer: a user preset, after confirmation.
    fn search_delete_preset(&mut self, i: usize) {
        let Some(presets) = self.search_win.presets.as_mut() else {
            return;
        };
        if presets.get(i).is_none_or(|p| p.shipped) {
            return;
        }
        let name = presets.remove(i).name;
        // A shipped preset the user's copy hid comes back.
        let shipped = crate::data::find_data_file(search::PRESETS_FILE);
        let res = search::save_user(presets, &presets_path());
        *presets = search::load(shipped.as_deref(), &presets_path());
        self.search_win.preset = None;
        self.search_win.message = Some(match res {
            Ok(()) => self.i18n.t_arg("search-preset-deleted", "name", &name),
            Err(e) => self.i18n.t_arg("err-generic", "error", &e),
        });
    }
}

/// The user's copy of the presets.
fn presets_path() -> std::path::PathBuf {
    crate::config::paths::get()
        .data_dir
        .join(search::PRESETS_FILE)
}
