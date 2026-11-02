//! Fichier › Importer avec un format… (D-08, figure B.4, R-D08-05 / 06):
//! a file read with a chosen text format before it is opened — the format
//! (the one chosen last for the file's kind first; Modifier… and Nouveau
//! format… open the Formats texte editor), its description of the fields,
//! the account of the reading (values read, lines per field, values left
//! out, identifiers, target file) and the first fifty lines as the grid
//! will show them. Ouvrir… comes here for a file no precise name pattern
//! takes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui;

use super::XimodApp;
use crate::doc_settings::DocSettings;
use crate::games::Game;
use crate::textfmt::{Formats, TextDoc, TextFormat};

/// The lines shown (R-D08-05).
const PREVIEW: usize = 50;

/// The reading of the file in the chosen format.
#[derive(Debug, Clone, Default)]
pub(crate) struct Preview {
    pub encoding: String,
    pub read: usize,
    pub lines: usize,
    /// « text 270, help 193… ».
    pub detail: String,
    pub excluded: usize,
    /// The names of the values left out.
    pub excluded_names: String,
    /// Distinct identifiers, lines sharing one.
    pub ids: Option<(usize, usize)>,
    pub target: String,
    /// GRUP, Champ, identifier, source, translation.
    pub rows: Vec<[String; 5]>,
}

pub(crate) struct ImportDialog {
    pub path: PathBuf,
    pub settings: Option<DocSettings>,
    pub data: Vec<u8>,
    pub formats: Formats,
    /// The formats on disk or shipped (a new one is not yet).
    pub saved: HashSet<String>,
    /// The format chosen (its id).
    pub chosen: String,
    /// The formats in the order of the list.
    pub order: Vec<usize>,
    /// The reading of `chosen`, with the generation it was made for.
    pub preview: Option<(String, u64, Result<Preview, String>)>,
    pub generation: u64,
    /// The editor's saves seen, to read the formats again.
    pub editor_saves: Option<u64>,
    /// Opened because Ouvrir… recognised nothing (R-D08-06).
    pub unknown: bool,
}

/// The key of a file's kind for `[Import] Last.<ext>`.
fn ext_key(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// The file a text document writes (R-C05-04, R-D08-04): the file itself
/// for a translation kept in it, `<base>_<langue>.<ext>` when the name
/// carries a language, `<nom>_<langue>.<ext>` for a structured copy, the
/// file itself otherwise.
pub(crate) fn target_file(path: &Path, fmt: &TextFormat, game: Game, target: &str) -> PathBuf {
    if fmt.in_place() || target.trim().is_empty() {
        return path.to_path_buf();
    }
    if fmt.lang_in_name == "suffix" {
        if let Some(p) = crate::textfmt::target_path(path, game, target) {
            return p;
        }
        if !fmt.is_line()
            && let (Some(stem), Some(ext)) = (
                path.file_stem().and_then(|s| s.to_str()),
                path.extension().and_then(|e| e.to_str()),
            )
        {
            let token = crate::textfmt::mcm_token(game, target);
            return path.with_file_name(format!("{stem}_{token}.{ext}"));
        }
    }
    path.to_path_buf()
}

impl XimodApp {
    /// Open the dialog on a file (`unknown`: Ouvrir… recognised nothing).
    pub(crate) fn open_import_dialog(
        &mut self,
        path: PathBuf,
        settings: Option<DocSettings>,
        unknown: bool,
    ) -> bool {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.notify_err(self.i18n.t_arg("err-open", "error", &e.to_string()));
                return false;
            }
        };
        let formats = Formats::load();
        if formats.formats.is_empty() {
            self.notify_err(self.i18n.t("text-no-format"));
            return false;
        }
        let mut dlg = ImportDialog {
            path,
            settings,
            data,
            saved: formats.formats.iter().map(|f| f.id.clone()).collect(),
            formats,
            chosen: String::new(),
            order: Vec::new(),
            preview: None,
            generation: 0,
            editor_saves: None,
            unknown,
        };
        self.import_order(&mut dlg);
        dlg.chosen = dlg
            .order
            .first()
            .map(|&i| dlg.formats.formats[i].id.clone())
            .unwrap_or_default();
        self.import_dlg = Some(dlg);
        true
    }

    /// The formats in the order of the list: the one chosen last for this
    /// kind of file, then by name, then by what they recognise.
    fn import_order(&self, dlg: &mut ImportDialog) {
        let cands = dlg.formats.candidates(&dlg.path, &dlg.data);
        let mut order: Vec<usize> = cands.iter().map(|c| c.index).collect();
        if let Some(last) = self.config.import_last.get(&ext_key(&dlg.path))
            && let Some(i) = dlg.formats.formats.iter().position(|f| f.id == *last)
            && let Some(p) = order.iter().position(|&x| x == i)
        {
            let x = order.remove(p);
            order.insert(0, x);
        }
        dlg.order = order;
    }

    /// Read the file in a format for the dialog.
    fn import_preview(&self, dlg: &ImportDialog, fmt: &TextFormat) -> Result<Preview, String> {
        let doc = TextDoc::parse(&dlg.data, fmt).map_err(|e| self.i18n.localize(&e.to_string()))?;
        let name = dlg
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let tm = doc.to_tm(fmt, &name, "");
        let settings = dlg
            .settings
            .clone()
            .unwrap_or_else(|| DocSettings::for_new_doc(Game::Unknown, &self.idle, &self.config));
        let target = target_file(&dlg.path, fmt, settings.game(), &settings.target);
        let mut p = Preview {
            encoding: doc.encoding.label().to_string(),
            lines: doc.keys(),
            target: target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            ..Default::default()
        };
        match &doc.structured {
            Some(s) => {
                p.read = s.stats.read;
                p.excluded = s.stats.excluded;
                p.detail = s
                    .stats
                    .by_field
                    .iter()
                    .map(|(f, n)| format!("{f} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut names: Vec<String> = s
                    .stats
                    .excluded_names
                    .iter()
                    .take(6)
                    .map(|(n, _)| n.clone())
                    .collect();
                if s.stats.excluded_names.len() > 6 {
                    names.push("…".into());
                }
                p.excluded_names = names.join(", ");
                p.ids = Some((s.stats.unique_ids, s.stats.dup_lines));
            }
            None => {
                p.read = doc.lines.len();
                p.excluded = doc.lines.len().saturating_sub(doc.keys());
                p.detail = crate::textfmt::FIELD.to_string();
            }
        }
        p.rows = tm
            .entries
            .iter()
            .take(PREVIEW)
            .map(|e| {
                [
                    e.rec_sig.clone(),
                    e.field_sig.clone(),
                    if e.index > 1 {
                        format!("{} #{}", e.edid, e.index)
                    } else {
                        e.edid.clone()
                    },
                    crate::ui::edit_window::one_line(&e.source, 48),
                    crate::ui::edit_window::one_line(&e.trans, 40),
                ]
            })
            .collect();
        Ok(p)
    }

    /// The description of the fields, as lines « label : value ».
    fn import_fields_lines(&self, fmt: &TextFormat) -> Vec<(String, String)> {
        let t = |k: &str| self.i18n.t(k);
        if fmt.is_line() {
            return vec![(t("formats-line"), fmt.line.clone())];
        }
        let Some(f) = &fmt.fields else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for name in crate::textfmt::structured::FIELD_NAMES {
            let Some(r) = f.get(name) else { continue };
            if r.by.is_empty() {
                continue;
            }
            let mut v = t(&format!("fmt-by-{}", r.by));
            if !r.value.is_empty() {
                v = format!("{v} · {}", r.value);
            }
            if r.otherwise == "auto" {
                v = format!("{v} · {}", t("fmt-else-auto"));
            }
            if !r.regex.is_empty() {
                v = format!("{v} · {}", r.regex);
            }
            out.push((t(&format!("fmt-field-{name}")), v));
        }
        out
    }

    /// The dialog, each frame while it is open.
    pub(crate) fn import_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.import_dlg.take() else {
            return;
        };
        // The editor saved: the formats again, the choice kept.
        if let Some(ed) = &self.formats_editor {
            if dlg.editor_saves.is_some_and(|s| s != ed.saves) {
                let drafts: Vec<TextFormat> = dlg
                    .formats
                    .formats
                    .iter()
                    .filter(|f| !dlg.saved.contains(&f.id))
                    .cloned()
                    .collect();
                dlg.formats = Formats::load();
                for d in drafts {
                    if dlg.formats.get(&d.id).is_none() {
                        dlg.formats.formats.push(d);
                    }
                }
                dlg.saved = Formats::load()
                    .formats
                    .iter()
                    .map(|f| f.id.clone())
                    .collect();
                self.import_order(&mut dlg);
                dlg.generation += 1;
            }
            dlg.editor_saves = Some(ed.saves);
        }
        let Some(fmt) = dlg.formats.get(&dlg.chosen).cloned() else {
            dlg.chosen = dlg
                .order
                .first()
                .map(|&i| dlg.formats.formats[i].id.clone())
                .unwrap_or_default();
            self.import_dlg = Some(dlg);
            return;
        };
        if dlg
            .preview
            .as_ref()
            .is_none_or(|(id, g, _)| *id != dlg.chosen || *g != dlg.generation)
        {
            let p = self.import_preview(&dlg, &fmt);
            dlg.preview = Some((dlg.chosen.clone(), dlg.generation, p));
        }
        let t = |k: &str| self.i18n.t(k);
        let file = dlg
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let title = self.i18n.t_arg("import-title", "file", &file);
        let kind_label = |f: &TextFormat| {
            let k = if f.is_line() { "line" } else { f.kind.as_str() };
            format!("{} — {}", f.name, self.i18n.t(&format!("fmt-kind-{k}")))
        };
        let rows: Vec<(String, String)> = dlg
            .order
            .iter()
            .map(|&i| {
                let f = &dlg.formats.formats[i];
                (f.id.clone(), kind_label(f))
            })
            .collect();
        let fields = self.import_fields_lines(&fmt);
        let preview = dlg.preview.as_ref().map(|p| p.2.clone());
        let enc = preview
            .as_ref()
            .and_then(|p| p.as_ref().ok())
            .map(|p| p.encoding.clone())
            .unwrap_or_default();
        let file_line = self.i18n.t_args(
            "import-file",
            &[
                ("path", &dlg.path.display().to_string()),
                ("encoding", &enc),
            ],
        );
        let (l_format, l_edit, l_new, l_fields, l_summary) = (
            t("import-format"),
            t("import-edit"),
            t("import-new"),
            t("import-fields"),
            t("import-summary"),
        );
        let (l_import, l_save, l_cancel, l_unknown) = (
            t("import-button"),
            t("import-save-format"),
            t("btn-cancel"),
            t("import-unknown-file"),
        );
        let heads = [
            t("col-grup"),
            t("col-field"),
            t("col-edid"),
            t("col-source"),
            t("col-trans"),
        ];
        let summary: Vec<(String, Option<egui::Color32>)> = match &preview {
            Some(Ok(p)) => {
                let mut v = vec![
                    (
                        self.i18n.t_arg("import-read", "n", &p.read.to_string()),
                        None,
                    ),
                    (
                        self.i18n.t_args(
                            "import-lines",
                            &[("n", &p.lines.to_string()), ("detail", &p.detail)],
                        ),
                        None,
                    ),
                ];
                let mut ex = self
                    .i18n
                    .t_arg("import-excluded", "n", &p.excluded.to_string());
                if !p.excluded_names.is_empty() {
                    ex = format!("{ex} · {}", p.excluded_names);
                }
                v.push((ex, None));
                if let Some((unique, dup)) = p.ids {
                    let pal = crate::ui::theme::Palette::of(&ctx.style().visuals);
                    if dup == 0 {
                        v.push((
                            self.i18n
                                .t_arg("import-ids-unique", "n", &unique.to_string()),
                            Some(pal.success),
                        ));
                    } else {
                        v.push((
                            self.i18n.t_arg("import-ids-dup", "n", &dup.to_string()),
                            Some(ctx.style().visuals.warn_fg_color),
                        ));
                    }
                }
                v.push((self.i18n.t_arg("import-target", "path", &p.target), None));
                if p.lines == 0 {
                    v.push((t("import-none"), Some(ctx.style().visuals.warn_fg_color)));
                }
                v
            }
            Some(Err(e)) => vec![(e.clone(), Some(ctx.style().visuals.error_fg_color))],
            None => Vec::new(),
        };
        let n_rows = match &preview {
            Some(Ok(p)) => p.rows.len(),
            _ => 0,
        };
        let l_preview = self.i18n.t_arg("import-preview", "n", &n_rows.to_string());
        let lines_ok = matches!(&preview, Some(Ok(p)) if p.lines > 0);
        let unsaved = !dlg.saved.contains(&dlg.chosen);
        let mut chosen = dlg.chosen.clone();
        let (mut do_import, mut do_save, mut do_cancel, mut do_edit, mut do_new) =
            (false, false, false, false, false);
        egui::Window::new(title)
            .id(egui::Id::new("ximod-import-dialog"))
            .collapsible(false)
            .resizable(true)
            .order(egui::Order::Foreground)
            .default_width(900.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(860.0);
                if dlg.unknown {
                    ui.label(egui::RichText::new(&l_unknown).weak());
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&l_format).strong());
                    let shown = rows
                        .iter()
                        .find(|(id, _)| *id == chosen)
                        .map(|r| r.1.clone())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("ximod-import-format")
                        .selected_text(shown)
                        .width(380.0)
                        .show_ui(ui, |ui| {
                            for (id, label) in &rows {
                                ui.selectable_value(&mut chosen, id.clone(), label);
                            }
                        });
                    if ui.button(&l_edit).clicked() {
                        do_edit = true;
                    }
                    if ui.button(&l_new).clicked() {
                        do_new = true;
                    }
                });
                ui.label(egui::RichText::new(&file_line).weak().small());
                ui.add_space(4.0);
                ui.columns(2, |cols| {
                    egui::Frame::group(cols[0].style()).show(&mut cols[0], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&l_fields).strong());
                        egui::Grid::new("ximod-import-fields")
                            .num_columns(2)
                            .spacing([10.0, 3.0])
                            .show(ui, |ui| {
                                for (k, v) in &fields {
                                    ui.label(k);
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(v).monospace()).wrap(),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
                    egui::Frame::group(cols[1].style()).show(&mut cols[1], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(&l_summary).strong());
                        for (s, c) in &summary {
                            let r = egui::RichText::new(s);
                            ui.add(
                                egui::Label::new(match c {
                                    Some(c) => r.color(*c),
                                    None => r,
                                })
                                .wrap(),
                            );
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&l_preview).strong());
                egui::ScrollArea::both()
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        egui::Grid::new("ximod-import-preview")
                            .num_columns(5)
                            .striped(true)
                            .spacing([12.0, 2.0])
                            .show(ui, |ui| {
                                for h in &heads {
                                    ui.label(egui::RichText::new(h).strong());
                                }
                                ui.end_row();
                                if let Some(Ok(p)) = &preview {
                                    for r in &p.rows {
                                        ui.label(&r[0]);
                                        ui.label(&r[1]);
                                        ui.label(egui::RichText::new(&r[2]).monospace());
                                        ui.label(&r[3]);
                                        ui.label(&r[4]);
                                        ui.end_row();
                                    }
                                }
                            });
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&l_cancel).clicked() {
                            do_cancel = true;
                        }
                        if ui
                            .add_enabled(unsaved, egui::Button::new(&l_save))
                            .clicked()
                        {
                            do_save = true;
                        }
                        if ui
                            .add_enabled(lines_ok, egui::Button::new(&l_import))
                            .clicked()
                        {
                            do_import = true;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.formats_editor.is_none() {
            do_cancel = true;
        }
        dlg.chosen = chosen;
        if do_new {
            // A format of the type the extension says (R-D08-05).
            let kind = TextFormat::kind_of_path(&dlg.path);
            let stem = dlg
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let id = dlg.formats.free_id(&stem);
            let name = self.i18n.t("formats-new-name");
            let mut f = if kind == "line" {
                TextFormat::new_user(&id, &name)
            } else {
                TextFormat::new_structured(&id, &name, kind)
            };
            if let Some(n) = dlg.path.file_name().and_then(|n| n.to_str()) {
                f.patterns = vec![n.to_string()];
            }
            dlg.formats.formats.push(f.clone());
            let i = dlg.formats.formats.len() - 1;
            dlg.order.insert(0, i);
            dlg.chosen = f.id.clone();
            dlg.generation += 1;
            let ed = crate::ui::formats_editor::FormatsEditor::with_draft(f);
            dlg.editor_saves = Some(ed.saves);
            self.formats_editor = Some(ed);
            self.formats_needs_center = true;
        }
        if do_edit {
            let ed = if dlg.saved.contains(&dlg.chosen) {
                crate::ui::formats_editor::FormatsEditor::open_on(&dlg.chosen)
            } else {
                match dlg.formats.get(&dlg.chosen) {
                    Some(f) => crate::ui::formats_editor::FormatsEditor::with_draft(f.clone()),
                    None => crate::ui::formats_editor::FormatsEditor::new(),
                }
            };
            dlg.editor_saves = Some(ed.saves);
            self.formats_editor = Some(ed);
            self.formats_needs_center = true;
        }
        if do_save {
            let mut all = Formats::load();
            if let Some(f) = dlg.formats.get(&dlg.chosen).cloned()
                && all.get(&f.id).is_none()
            {
                all.formats.push(f);
            }
            match all.save() {
                Ok(()) => {
                    dlg.saved.insert(dlg.chosen.clone());
                    self.notify_ok(self.i18n.t("formats-saved"));
                }
                Err(e) => self.notify_err(self.i18n.t_arg("err-save", "error", &e.to_string())),
            }
        }
        if do_cancel {
            return;
        }
        if do_import {
            let fmt = dlg.formats.get(&dlg.chosen).cloned();
            if let Some(fmt) = fmt {
                self.config
                    .import_last
                    .insert(ext_key(&dlg.path), fmt.id.clone());
                let _ = self.config.save();
                self.open_text_with(dlg.path, &fmt, dlg.settings);
            }
            return;
        }
        self.import_dlg = Some(dlg);
    }
}
