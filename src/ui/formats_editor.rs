//! Outils ▸ Données ▸ Formats texte… (C-05, R-C05-01): the text formats —
//! the shipped ones read-only, to duplicate; the user's own edited here
//! (name, name patterns, encoding, line endings, line expression,
//! escaping, options) — with « Essai sur un fichier » and a save that
//! refuses an invalid expression or one without the `key` / `value` groups.
//! A structured format (D-08, figure B.5) has a type and a table of its
//! fields — resolution and value — instead of the line expression.

use eframe::egui;

use crate::i18n::I18n;
use crate::textfmt::structured::{self, BYS, FIELD_NAMES, Fields, KINDS, Resolve};
use crate::textfmt::{CompileError, Formats, TextFormat};

pub struct FormatsEditor {
    formats: Formats,
    selected: usize,
    /// The format being edited (a copy of the selected one).
    draft: TextFormat,
    /// The patterns, one per line, as typed.
    patterns: String,
    error: Option<String>,
    message: Option<String>,
    dirty: bool,
    /// The file of the last trial (the columns are checked against it).
    trial: Option<std::path::PathBuf>,
    /// Counts the saves, for the import dialog to read the formats again.
    pub saves: u64,
}

impl FormatsEditor {
    pub fn new() -> Self {
        let formats = Formats::load();
        let mut ed = FormatsEditor {
            formats,
            selected: 0,
            draft: TextFormat::new_user("", ""),
            patterns: String::new(),
            error: None,
            message: None,
            dirty: false,
            trial: None,
            saves: 0,
        };
        ed.select(0);
        ed
    }

    /// The editor on a new format not yet saved (Nouveau format… of the
    /// import dialog).
    pub fn with_draft(f: TextFormat) -> Self {
        let mut ed = Self::new();
        match ed.formats.formats.iter().position(|x| x.id == f.id) {
            Some(i) => ed.select(i),
            None => {
                ed.formats.formats.push(f);
                ed.dirty = true;
                ed.select(ed.formats.formats.len() - 1);
            }
        }
        ed
    }

    /// The editor on a format (Modifier… of the import dialog).
    pub fn open_on(id: &str) -> Self {
        let mut ed = Self::new();
        if let Some(i) = ed.formats.formats.iter().position(|f| f.id == id) {
            ed.select(i);
        }
        ed
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn select(&mut self, idx: usize) {
        self.selected = idx.min(self.formats.formats.len().saturating_sub(1));
        if let Some(f) = self.formats.formats.get(self.selected) {
            self.draft = f.clone();
            self.patterns = f.patterns.join("\n");
        }
        self.error = None;
        self.message = None;
    }

    /// The draft back into the list (user formats only); `false` with the
    /// reason when the expression is refused.
    fn commit(&mut self, i18n: &I18n) -> bool {
        let Some(f) = self.formats.formats.get(self.selected) else {
            return true;
        };
        if f.builtin {
            return true;
        }
        self.draft.patterns = self
            .patterns
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if self.draft.name.trim().is_empty() {
            self.draft.name = self.draft.id.clone();
        }
        if !self.draft.is_line() {
            // R-D08-07: paths, expressions, the texts, the type and the
            // patterns, the columns of the last trial.
            if let Err(p) = structured::validate(&self.draft) {
                self.error = Some(i18n.localize(&p.message()));
                return false;
            }
            let ext = format!(".{}", self.draft.kind);
            if let Some(p) = self.draft.patterns.iter().find(|p| {
                let p = p.to_lowercase();
                p.contains('.')
                    && !p.ends_with(&ext)
                    && !p.ends_with(".*")
                    && !(self.draft.kind == "csv" && p.ends_with(".tsv"))
            }) {
                self.error = Some(i18n.t_arg("fmt-err-kind-patterns", "pattern", p));
                return false;
            }
            if let Some(path) = &self.trial
                && let Ok(data) = std::fs::read(path)
            {
                let (text, _) = crate::textfmt::decode(&data, &self.draft.encoding);
                if let Some(c) = structured::missing_columns(&self.draft, &text) {
                    self.error = Some(i18n.t_arg("fmt-err-column", "column", &c));
                    return false;
                }
            }
        } else {
            match self.draft.compile() {
                Ok(_) => {}
                Err(CompileError::Invalid(e)) => {
                    self.error = Some(i18n.t_arg("formats-regex-invalid", "err", &e));
                    return false;
                }
                Err(CompileError::Groups) | Err(CompileError::Structured) => {
                    self.error = Some(i18n.t("formats-regex-groups"));
                    return false;
                }
            }
        }
        self.error = None;
        let changed = self.formats.formats[self.selected] != self.draft;
        self.formats.formats[self.selected] = self.draft.clone();
        if changed {
            self.dirty = true;
        }
        true
    }

    fn save(&mut self, i18n: &I18n) {
        if !self.commit(i18n) {
            return;
        }
        match self.formats.save() {
            Ok(()) => {
                self.dirty = false;
                self.saves += 1;
                self.message = Some(i18n.t("formats-saved"));
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Draw the editor; returns true when it should close.
    pub fn body(&mut self, ui: &mut egui::Ui, i18n: &I18n) -> bool {
        let mut close = false;
        let is_builtin = self
            .formats
            .formats
            .get(self.selected)
            .is_some_and(|f| f.builtin);
        let full_h = ui.available_height();
        ui.horizontal(|ui| {
            // The list, the full height of the window.
            ui.allocate_ui_with_layout(
                egui::vec2(230.0, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(230.0);
                    ui.label(egui::RichText::new(i18n.t("formats-list")).strong());
                    ui.horizontal(|ui| {
                        if ui.button(i18n.t("formats-new")).clicked() && self.commit(i18n) {
                            let id = self.formats.free_id("format");
                            let name = i18n.t("formats-new-name");
                            self.formats.formats.push(TextFormat::new_user(&id, &name));
                            self.dirty = true;
                            self.select(self.formats.formats.len() - 1);
                        }
                        if ui.button(i18n.t("formats-duplicate")).clicked()
                            && self.commit(i18n)
                            && let Some(src) = self.formats.formats.get(self.selected).cloned()
                        {
                            let id = self.formats.free_id(&src.id);
                            let mut f = src;
                            f.id = id;
                            f.builtin = false;
                            f.name = i18n.t_arg("formats-copy-name", "name", &f.name);
                            self.formats.formats.push(f);
                            self.dirty = true;
                            self.select(self.formats.formats.len() - 1);
                        }
                        if ui
                            .add_enabled(!is_builtin, egui::Button::new(i18n.t("formats-delete")))
                            .clicked()
                        {
                            self.formats.formats.remove(self.selected);
                            self.dirty = true;
                            self.select(self.selected.saturating_sub(1));
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut pick: Option<usize> = None;
                            for (i, f) in self.formats.formats.iter().enumerate() {
                                let label = if f.builtin {
                                    format!("{} {}", crate::icons::LOCK, f.name)
                                } else {
                                    f.name.clone()
                                };
                                let text = if f.builtin {
                                    egui::RichText::new(label).weak()
                                } else {
                                    egui::RichText::new(label)
                                };
                                if ui.selectable_label(self.selected == i, text).clicked() {
                                    pick = Some(i);
                                }
                            }
                            if let Some(i) = pick
                                && i != self.selected
                                && self.commit(i18n)
                            {
                                self.select(i);
                            }
                        });
                },
            );
            ui.separator();
            // The form.
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                if is_builtin {
                    ui.label(egui::RichText::new(i18n.t("formats-builtin-hint")).weak());
                    ui.add_space(4.0);
                }
                ui.add_enabled_ui(!is_builtin, |ui| {
                    egui::Grid::new("ximod-format-form")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(i18n.t("formats-name"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.draft.name)
                                    .desired_width(ui.available_width() - 8.0),
                            );
                            ui.end_row();
                            ui.label(i18n.t("formats-id"));
                            ui.label(egui::RichText::new(&self.draft.id).monospace());
                            ui.end_row();
                            ui.label(i18n.t("fmt-kind"));
                            let before = self.draft.kind.clone();
                            egui::ComboBox::from_id_salt("ximod-format-kind")
                                .selected_text(
                                    i18n.t(&format!("fmt-kind-{}", kind_of(&self.draft))),
                                )
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for k in KINDS {
                                        ui.selectable_value(
                                            &mut self.draft.kind,
                                            k.to_string(),
                                            i18n.t(&format!("fmt-kind-{k}")),
                                        );
                                    }
                                });
                            if self.draft.kind != before {
                                if self.draft.is_line() {
                                    if self.draft.line.trim().is_empty() {
                                        self.draft.line = TextFormat::new_user("", "").line;
                                    }
                                } else {
                                    self.draft.fields = Some(Fields::blank(&self.draft.kind));
                                }
                            }
                            ui.end_row();
                            ui.label(i18n.t("formats-patterns"));
                            ui.add(
                                egui::TextEdit::multiline(&mut self.patterns)
                                    .desired_rows(3)
                                    .desired_width(ui.available_width() - 8.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();
                            ui.label(i18n.t("formats-encoding"));
                            combo(
                                ui,
                                "enc",
                                &mut self.draft.encoding,
                                &crate::textfmt::ENCODINGS,
                            );
                            ui.end_row();
                            ui.label(i18n.t("formats-newline"));
                            combo(ui, "nl", &mut self.draft.newline, &crate::textfmt::NEWLINES);
                            ui.end_row();
                            ui.label(i18n.t("formats-lang-in-name"));
                            combo(
                                ui,
                                "lang",
                                &mut self.draft.lang_in_name,
                                &crate::textfmt::LANG_IN_NAME,
                            );
                            ui.end_row();
                            if self.draft.is_line() {
                                ui.label(i18n.t("formats-line"));
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.draft.line)
                                        .desired_rows(2)
                                        .desired_width(ui.available_width() - 8.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                ui.end_row();
                                ui.label(i18n.t("formats-escape"));
                                combo(ui, "esc", &mut self.draft.escape, &crate::textfmt::ESCAPES);
                                ui.end_row();
                                ui.label("");
                                ui.checkbox(
                                    &mut self.draft.skip_empty,
                                    i18n.t("formats-skip-empty"),
                                );
                                ui.end_row();
                            } else if self.draft.kind == "csv" {
                                ui.label(i18n.t("fmt-separator"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.draft.separator)
                                            .hint_text(";")
                                            .desired_width(50.0)
                                            .font(egui::TextStyle::Monospace),
                                    );
                                    ui.checkbox(&mut self.draft.header, i18n.t("fmt-header"));
                                });
                                ui.end_row();
                            }
                        });
                    if !self.draft.is_line() {
                        ui.add_space(6.0);
                        fields_table(ui, i18n, &mut self.draft);
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(i18n.t(if self.draft.is_line() {
                        "formats-line-hint"
                    } else {
                        "fmt-resolutions-hint"
                    }))
                    .weak()
                    .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(i18n.t("formats-test")).clicked() {
                        self.draft.patterns = self
                            .patterns
                            .lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect();
                        let picked = rfd::FileDialog::new().pick_file();
                        if let Some(p) = &picked
                            && !self.draft.is_line()
                        {
                            self.trial = Some(p.clone());
                            match crate::textfmt::try_structured(&self.draft, p) {
                                Ok(s) => {
                                    let (text, field, id) = s
                                        .items
                                        .first()
                                        .map(|i| {
                                            (
                                                crate::ui::edit_window::one_line(&i.value, 60),
                                                i.field.clone(),
                                                i.id.clone(),
                                            )
                                        })
                                        .unwrap_or_default();
                                    self.message = Some(i18n.t_args(
                                        "fmt-try-result",
                                        &[
                                            ("n", &s.items.len().to_string()),
                                            ("excluded", &s.stats.excluded.to_string()),
                                            ("text", &text),
                                            ("field", &field),
                                            ("id", &id),
                                        ],
                                    ));
                                    self.error = None;
                                }
                                Err(e) => {
                                    self.error = Some(i18n.localize(&e));
                                    self.message = None;
                                }
                            }
                        } else if let Some(p) = picked {
                            match crate::textfmt::try_file(&self.draft, &p) {
                                Ok((n, total, first)) => {
                                    let mut msg = i18n.t_args(
                                        "text-format-matches",
                                        &[("n", &n.to_string()), ("total", &total.to_string())],
                                    );
                                    for (k, v) in first {
                                        msg.push('\n');
                                        msg.push_str(&i18n.t_args(
                                            "formats-sample-pair",
                                            &[
                                                ("key", &crate::ui::edit_window::one_line(&k, 40)),
                                                (
                                                    "value",
                                                    &crate::ui::edit_window::one_line(&v, 60),
                                                ),
                                            ],
                                        ));
                                    }
                                    self.message = Some(msg);
                                    self.error = None;
                                }
                                Err(e) => {
                                    self.error = Some(
                                        if e.to_string() == crate::l10n!("formats-regex-groups") {
                                            i18n.t("formats-regex-groups")
                                        } else {
                                            i18n.t_arg(
                                                "formats-regex-invalid",
                                                "err",
                                                &e.to_string(),
                                            )
                                        },
                                    );
                                }
                            }
                        }
                    }
                    if ui
                        .add_enabled(
                            !is_builtin || self.dirty,
                            egui::Button::new(i18n.t("btn-save")),
                        )
                        .clicked()
                    {
                        self.save(i18n);
                    }
                    if ui.button(i18n.t("btn-close")).clicked() {
                        close = true;
                    }
                });
                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                if let Some(m) = &self.message {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(egui::RichText::new(m).small()).wrap());
                }
            });
        });
        close
    }
}

impl Default for FormatsEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// The syntax of the extraction group (not a text: the regex itself).
const EXTRACT_HINT: &str = concat!("(?P<", "text", ">…)");

/// The type of a format, `line` for a format of the first version.
fn kind_of(f: &TextFormat) -> &str {
    if f.is_line() { "line" } else { f.kind.as_str() }
}

/// The table of the fields of a structured format (figure B.5, 2): per
/// field its resolution and its value.
fn fields_table(ui: &mut egui::Ui, i18n: &I18n, f: &mut TextFormat) {
    let kind = f.kind.clone();
    let fields = f.fields.get_or_insert_with(|| Fields::blank(&kind));
    ui.label(egui::RichText::new(i18n.t("import-fields")).strong());
    egui::Grid::new("ximod-format-fields")
        .num_columns(3)
        .striped(true)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for name in FIELD_NAMES {
                ui.label(i18n.t(&format!("fmt-field-{name}")));
                let choices = Fields::choices(&kind, name);
                let mut cur = fields.get(name).cloned();
                let shown = match &cur {
                    Some(r) if !r.by.is_empty() => i18n.t(&format!("fmt-by-{}", r.by)),
                    _ => "—".to_string(),
                };
                let mut by = cur.as_ref().map(|r| r.by.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt(format!("ximod-format-by-{name}"))
                    .selected_text(shown)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        if name != "text" {
                            ui.selectable_value(&mut by, String::new(), "—");
                        }
                        for b in choices {
                            debug_assert!(BYS.contains(b));
                            ui.selectable_value(
                                &mut by,
                                b.to_string(),
                                i18n.t(&format!("fmt-by-{b}")),
                            );
                        }
                    });
                if by != cur.as_ref().map(|r| r.by.clone()).unwrap_or_default() {
                    cur = if by.is_empty() {
                        None
                    } else {
                        Some(Resolve {
                            by: by.clone(),
                            ..cur.clone().unwrap_or_default()
                        })
                    };
                }
                ui.horizontal(|ui| {
                    if let Some(r) = cur.as_mut() {
                        let wants_value =
                            !matches!(r.by.as_str(), "auto" | "property-name" | "copy");
                        ui.add_enabled(
                            wants_value,
                            egui::TextEdit::singleline(&mut r.value)
                                .desired_width(250.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        if name == "id" && r.by == "property" {
                            let mut auto = r.otherwise == "auto";
                            if ui.checkbox(&mut auto, i18n.t("fmt-else-auto")).changed() {
                                r.otherwise = if auto { "auto".into() } else { String::new() };
                            }
                        }
                    }
                });
                ui.end_row();
                if name == "text" {
                    ui.label(egui::RichText::new(i18n.t("fmt-extract")).weak());
                    ui.label("");
                    if let Some(r) = cur.as_mut() {
                        ui.add(
                            egui::TextEdit::singleline(&mut r.regex)
                                .hint_text(EXTRACT_HINT)
                                .desired_width(250.0)
                                .font(egui::TextStyle::Monospace),
                        );
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
                fields.set(name, cur);
            }
        });
}

fn combo(ui: &mut egui::Ui, salt: &str, value: &mut String, choices: &[&str]) {
    egui::ComboBox::from_id_salt(format!("ximod-format-{salt}"))
        .selected_text(value.as_str())
        .width(150.0)
        .show_ui(ui, |ui| {
            for c in choices {
                ui.selectable_value(value, c.to_string(), *c);
            }
        });
}
