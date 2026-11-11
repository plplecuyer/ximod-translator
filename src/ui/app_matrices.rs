//! Matrices de remplacement (B-17): the editor window (Outils › Données ›
//! Matrices de remplacement…), the matrix list of Rechercher et remplacer
//! and the application of a matrix to the lines of a scope — one undoable
//! operation, status kept, provenance « remplacement (<matrice>) ».

use crate::ui::theme::ComboField as _;
use eframe::egui;

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::matrices::{Direction, Dirs, Invalid, Matrix, MatrixFile, MatrixRule, Origin, plan};
use crate::replace::Mode;
use crate::undo::{EntryState, OpKind, op_from_before};

/// How many translations of the active document the idempotence check
/// tries (R-B17-04).
const SAMPLE: usize = 3000;
/// Width of the list of matrices; the rules take the rest.
const MATRICES_LIST_W: f32 = 230.0;

/// The editor's state.
#[derive(Default)]
pub(crate) struct MatricesWindow {
    pub open: bool,
    pub needs_place: bool,
    /// The language edited (`fra`, `all`…).
    pub lang: String,
    pub languages: Vec<String>,
    pub file: MatrixFile,
    pub origin: Option<Origin>,
    pub selected: usize,
    pub dirty: bool,
    /// Bumped at each edit: the preview follows.
    pub edits: u64,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub message: Option<String>,
    /// Lines touched in the active document: direct, inverse (`None`: the
    /// matrix has no inverse).
    pub preview: Option<(usize, Option<usize>)>,
    pub preview_key: Option<(u64, u64, usize, u64, String)>,
    /// Closing with unsaved changes asks first.
    pub confirm_close: bool,
}

impl XimodApp {
    /// The target language of the active document (ISO 639-3), empty
    /// when none is chosen.
    pub(crate) fn matrices_target(&self) -> String {
        self.active_doc()
            .map(|d| d.settings.target.trim().to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// The words of an invalid file.
    pub(crate) fn matrices_invalid(&self, e: &Invalid) -> String {
        match e {
            Invalid::Json(m) => self.i18n.t_arg("matrices-err-json", "error", m),
            Invalid::Format(f) => self.i18n.t_arg("matrices-err-format", "n", &f.to_string()),
            Invalid::NoName(k) => self.i18n.t_arg("matrices-err-no-name", "n", &k.to_string()),
            Invalid::DuplicateName(n) => self.i18n.t_arg("matrices-err-name-dup", "name", n),
            Invalid::EmptyFind { matrix, rule } => self.i18n.t_args(
                "matrices-err-empty",
                &[("name", matrix), ("n", &rule.to_string())],
            ),
            Invalid::Regex {
                matrix,
                rule,
                error,
            } => self.i18n.t_args(
                "matrices-err-regex",
                &[("name", matrix), ("n", &rule.to_string()), ("error", error)],
            ),
        }
    }

    /// Apply compiled rules to the lines: one operation, status kept,
    /// provenance « remplacement (<matrice>) » (R-B17-02). Returns the
    /// lines changed.
    pub(crate) fn matrix_apply_lines(
        &mut self,
        lines: &[usize],
        matchers: &[crate::replace::Matcher],
        name: &str,
    ) -> usize {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return 0;
        };
        let changes = plan(&doc.tm.entries, lines, matchers);
        if changes.is_empty() {
            return 0;
        }
        let prov = crate::provenance::replace_matrix(name);
        let mut before = Vec::with_capacity(changes.len());
        for (i, text) in changes {
            let e = &mut doc.tm.entries[i];
            before.push((i, EntryState::capture(e)));
            e.trans = text;
            e.prov = prov.clone();
        }
        let mut op = op_from_before(OpKind::Matrix, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            op.label_arg = Some(name.to_string());
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        n
    }

    // ------------------------------------------------------------------
    // Rechercher et remplacer: the matrix list (R-B17-02)
    // ------------------------------------------------------------------

    /// Read again the matrices offered when the target language or the
    /// files changed.
    pub(crate) fn replace_matrices_refresh(&mut self) {
        let lang = self.matrices_target();
        let key = (lang.clone(), self.matrices_gen);
        if self.replace.matrices_key.as_ref() == Some(&key) {
            return;
        }
        let first = self.replace.matrices_key.is_none()
            || self.replace.matrices_key.as_ref().map(|k| &k.0) != Some(&lang);
        self.replace.matrices_key = Some(key);
        self.replace.matrices = Dirs::installed().offered(&lang);
        self.replace.matrix_preview_key = None;
        // The last matrix of this language when the language changed.
        if first {
            self.replace.matrix = self.config.last_matrix_for(&lang).and_then(|n| {
                self.replace
                    .matrices
                    .iter()
                    .find(|(_, m)| m.name == n)
                    .map(|(l, m)| (l.clone(), m.name.clone()))
            });
        } else if self.replace.chosen_matrix().is_none() {
            self.replace.matrix = None;
        }
        if self
            .replace
            .chosen_matrix()
            .is_some_and(|m| m.blocking_inverse().is_some())
        {
            self.replace.direction = Direction::Direct;
        }
    }

    /// The preview of the chosen matrix on the scope, recomputed when the
    /// document, the scope, the matrix or the direction moved.
    fn replace_matrix_preview(&mut self, scope: &[usize]) {
        let Some((_, name)) = self.replace.matrix.clone() else {
            self.replace.matrix_preview = None;
            return;
        };
        let revision = self.active_doc().map(|d| d.undo.revision).unwrap_or(0);
        let key = (
            name,
            self.replace.direction == Direction::Inverse,
            revision,
            scope.len(),
            scope.iter().sum::<usize>(),
            self.matrices_gen,
        );
        if self.replace.matrix_preview_key.as_ref() == Some(&key) {
            return;
        }
        self.replace.matrix_preview_key = Some(key);
        let Some(m) = self.replace.chosen_matrix().cloned() else {
            self.replace.matrix_preview = None;
            return;
        };
        let mut p = crate::ui::replace_window::MatrixPreview {
            rules: m.rules.len(),
            ..Default::default()
        };
        match m.compile(self.replace.direction) {
            Ok(c) => {
                if let Some(doc) = self.active_doc() {
                    let changes = plan(&doc.tm.entries, scope, &c);
                    p.lines = changes.len();
                    p.first = changes.first().map(|(i, t)| {
                        (
                            crate::ui::edit_window::one_line(&doc.tm.entries[*i].trans, 60),
                            crate::ui::edit_window::one_line(t, 60),
                        )
                    });
                }
            }
            Err(e) => p.error = Some(e),
        }
        self.replace.matrix_preview = Some(p);
    }

    /// « Appliquer la matrice »: the chosen matrix on the dialog's scope.
    fn replace_matrix_apply(&mut self, scope: &[usize]) {
        let Some(m) = self.replace.chosen_matrix().cloned() else {
            return;
        };
        let dir = self.replace.direction;
        let Ok(c) = m.compile(dir) else {
            return;
        };
        let n = self.matrix_apply_lines(scope, &c, &m.name);
        let dir_word = self.i18n.t(match dir {
            Direction::Direct => "replace-direct",
            Direction::Inverse => "replace-inverse",
        });
        let msg = self.i18n.t_args(
            "replace-matrix-done",
            &[
                ("name", &m.name),
                ("direction", &dir_word.to_lowercase()),
                ("n", &n.to_string()),
            ],
        );
        self.replace.message = Some(msg.clone());
        self.notify_ok(msg);
    }

    /// The matrix row at the top of Rechercher et remplacer (figure B.2).
    pub(crate) fn replace_matrix_ui(&mut self, ui: &mut egui::Ui, scope: &[usize]) {
        self.replace_matrix_preview(scope);
        let t = |k: &str| self.i18n.t(k);
        let none_label = t("replace-matrix-none");
        let all_suffix = t("matrices-all-short");
        let label_of = |lang: &str, m: &Matrix| {
            if lang == crate::matrices::ALL {
                format!("{} ({all_suffix})", m.name)
            } else {
                m.name.clone()
            }
        };
        let current = self
            .replace
            .matrix
            .as_ref()
            .and_then(|(l, n)| {
                self.replace
                    .matrices
                    .iter()
                    .find(|(ll, m)| ll == l && &m.name == n)
                    .map(|(l, m)| label_of(l, m))
            })
            .unwrap_or_else(|| none_label.clone());
        let mut chosen: Option<Option<(String, String)>> = None;
        let mut apply = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.strong(t("replace-matrix"));
                egui::ComboBox::from_id_salt("ximod-replace-matrix")
                    .selected_text(current)
                    .width(260.0)
                    .show_field(ui, |ui| {
                        if ui
                            .selectable_label(self.replace.matrix.is_none(), &none_label)
                            .clicked()
                        {
                            chosen = Some(None);
                        }
                        for (l, m) in &self.replace.matrices {
                            let sel =
                                self.replace.matrix.as_ref() == Some(&(l.clone(), m.name.clone()));
                            if ui.selectable_label(sel, label_of(l, m)).clicked() {
                                chosen = Some(Some((l.clone(), m.name.clone())));
                            }
                        }
                    });
                ui.add_space(12.0);
                ui.strong(t("replace-direction"));
                ui.radio_value(
                    &mut self.replace.direction,
                    Direction::Direct,
                    t("replace-direct"),
                );
                let blocking = self
                    .replace
                    .chosen_matrix()
                    .and_then(|m| m.blocking_inverse().map(|i| m.rules[i].rule.find.clone()));
                let r = ui.add_enabled(
                    self.replace.matrix.is_some() && blocking.is_none(),
                    egui::RadioButton::new(
                        self.replace.direction == Direction::Inverse,
                        t("replace-inverse"),
                    ),
                );
                if r.clicked() {
                    self.replace.direction = Direction::Inverse;
                }
                if let Some(find) = &blocking {
                    r.on_disabled_hover_text(self.i18n.t_arg(
                        "replace-matrix-no-inverse",
                        "find",
                        find,
                    ));
                }
            });
            if self.replace.matrix.is_none() {
                ui.label(egui::RichText::new(t("replace-matrix-hint")).weak().small());
                return;
            }
            match &self.replace.matrix_preview {
                Some(p) if p.error.is_some() => {
                    let (n, e) = p.error.clone().unwrap_or_default();
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        self.i18n.t_args(
                            "matrices-rule-error",
                            &[("n", &n.to_string()), ("error", &e)],
                        ),
                    );
                }
                Some(p) if p.lines == 0 => {
                    ui.label(egui::RichText::new(t("replace-none")).weak());
                }
                Some(p) => {
                    let (b, a) = p.first.clone().unwrap_or_default();
                    ui.add(
                        egui::Label::new(self.i18n.t_args(
                            "replace-matrix-preview",
                            &[
                                ("rules", &p.rules.to_string()),
                                ("n", &p.lines.to_string()),
                                ("before", &b),
                                ("after", &a),
                            ],
                        ))
                        .wrap(),
                    );
                }
                None => {}
            }
            ui.horizontal(|ui| {
                let can = self.replace.target.replaceable()
                    && self
                        .replace
                        .matrix_preview
                        .as_ref()
                        .is_some_and(|p| p.error.is_none() && p.lines > 0);
                if ui
                    .add_enabled(
                        can,
                        egui::Button::new(egui::RichText::new(t("replace-matrix-apply")).strong()),
                    )
                    .clicked()
                {
                    apply = true;
                }
                ui.label(
                    egui::RichText::new(t("replace-matrix-keeps"))
                        .weak()
                        .small(),
                );
            });
        });
        if let Some(c) = chosen {
            let lang = self.matrices_target();
            let name = c.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
            self.config.set_last_matrix(&lang, &name);
            let _ = self.config.save();
            self.replace.matrix = c;
            self.replace.matrix_preview_key = None;
            if self
                .replace
                .chosen_matrix()
                .is_some_and(|m| m.blocking_inverse().is_some())
            {
                self.replace.direction = Direction::Direct;
            }
        }
        if apply {
            self.replace_matrix_apply(scope);
        }
    }

    /// « Ajouter à une matrice… »: the menu of the matrices of the target
    /// language, and a new one.
    pub(crate) fn replace_add_to_matrix_menu(&mut self, ui: &mut egui::Ui) {
        let lang = {
            let l = self.matrices_target();
            if l.is_empty() {
                crate::matrices::ALL.to_string()
            } else {
                l
            }
        };
        let enabled = !self.replace.find.is_empty() && self.replace.error.is_none();
        let mut target: Option<String> = None;
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(self.i18n.t("replace-add-to-matrix"), |ui| {
                let dirs = Dirs::installed();
                let (file, _) = dirs.load(&lang);
                for m in &file.matrices {
                    if ui.button(&m.name).clicked() {
                        target = Some(m.name.clone());
                        ui.close_menu();
                    }
                }
                if !file.matrices.is_empty() {
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.replace.new_matrix)
                            .hint_text(self.i18n.t("replace-new-matrix-hint"))
                            .desired_width(180.0),
                    );
                    if ui
                        .add_enabled(
                            !self.replace.new_matrix.trim().is_empty(),
                            egui::Button::new(self.i18n.t("replace-new-matrix")),
                        )
                        .clicked()
                    {
                        target = Some(self.replace.new_matrix.trim().to_string());
                        ui.close_menu();
                    }
                });
            });
        });
        let Some(name) = target else {
            return;
        };
        let dirs = Dirs::installed();
        let (mut file, _) = dirs.load(&lang);
        let rule = MatrixRule {
            rule: self.replace.rule(),
            inverse: None,
        };
        let n = match file.matrices.iter_mut().find(|m| m.name == name) {
            Some(m) => {
                m.rules.push(rule);
                m.rules.len()
            }
            None => {
                file.matrices.push(Matrix {
                    name: name.clone(),
                    rules: vec![rule],
                });
                1
            }
        };
        match dirs.save(&lang, &file) {
            Ok(_) => {
                self.matrices_gen += 1;
                self.replace.new_matrix.clear();
                let msg = self.i18n.t_args(
                    "replace-rule-added",
                    &[("name", &name), ("n", &n.to_string())],
                );
                self.replace.message = Some(msg.clone());
                self.notify_ok(msg);
            }
            Err(e) => {
                let msg = self.matrices_invalid(&e);
                self.notify_err(msg);
            }
        }
    }

    // ------------------------------------------------------------------
    // The editor (Outils › Données › Matrices de remplacement…)
    // ------------------------------------------------------------------

    pub(crate) fn open_matrices_window(&mut self) {
        if self.matrices_win.open {
            self.matrices_win.open = false;
            return;
        }
        let lang = {
            let l = self.matrices_target();
            if l.is_empty() {
                crate::matrices::ALL.to_string()
            } else {
                l
            }
        };
        self.matrices_load(&lang);
        self.matrices_win.open = true;
        self.matrices_win.needs_place = true;
    }

    fn matrices_load(&mut self, lang: &str) {
        let dirs = Dirs::installed();
        let (file, origin) = dirs.load(lang);
        let w = &mut self.matrices_win;
        w.languages = dirs.languages();
        if !w.languages.iter().any(|l| l == lang) {
            w.languages.push(lang.to_string());
        }
        w.lang = lang.to_string();
        w.file = file;
        w.origin = Some(origin);
        w.selected = 0;
        w.dirty = false;
        w.edits += 1;
        w.error = None;
        w.warning = None;
        w.preview_key = None;
    }

    fn matrices_touch(&mut self) {
        self.matrices_win.dirty = true;
        self.matrices_win.edits += 1;
        self.matrices_win.error = None;
        self.matrices_win.message = None;
    }

    /// « Enregistrer »: validate, warn when a matrix is not idempotent on
    /// the active document, save the user's copy.
    fn matrices_save(&mut self) -> bool {
        let w = &self.matrices_win;
        if let Err(e) = w.file.validate() {
            self.matrices_win.error = Some(self.matrices_invalid(&e));
            return false;
        }
        // Idempotence (R-B17-04): a warning, never a refusal.
        let mut warning = None;
        if let Some(doc) = self.active_doc() {
            let texts: Vec<&str> = doc
                .tm
                .entries
                .iter()
                .filter(|e| !e.trans.is_empty())
                .take(SAMPLE)
                .map(|e| e.trans.as_str())
                .collect();
            for m in &w.file.matrices {
                if let Ok(c) = m.compile(Direction::Direct) {
                    let n = crate::matrices::not_idempotent(&c, &texts);
                    if n > 0 {
                        warning = Some(self.i18n.t_args(
                            "matrices-not-idempotent",
                            &[("name", &m.name), ("n", &n.to_string())],
                        ));
                        break;
                    }
                }
            }
        }
        let lang = w.lang.clone();
        match Dirs::installed().save(&lang, &w.file) {
            Ok(p) => {
                self.matrices_gen += 1;
                let w = &mut self.matrices_win;
                w.dirty = false;
                w.origin = Some(Origin::User);
                w.warning = warning;
                w.message = Some(self.i18n.t_arg(
                    "matrices-saved",
                    "path",
                    &p.display().to_string(),
                ));
                true
            }
            Err(e) => {
                self.matrices_win.error = Some(self.matrices_invalid(&e));
                false
            }
        }
    }

    fn matrices_restore(&mut self) {
        let lang = self.matrices_win.lang.clone();
        match Dirs::installed().restore(&lang) {
            Ok(()) => {
                self.matrices_gen += 1;
                self.matrices_load(&lang);
                self.matrices_win.message = Some(self.i18n.t("matrices-restored"));
            }
            Err(e) => self.matrices_win.error = Some(e.to_string()),
        }
    }

    /// « Importer un fichier de règles A-11… »: a matrix of the file's name.
    fn matrices_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-json"), &["json"])
            .pick_file()
        else {
            return;
        };
        match crate::replace::RulesFile::load(&path) {
            Ok(f) => {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("regles")
                    .to_string();
                let mut name = stem.clone();
                let mut k = 2;
                while self.matrices_win.file.find(&name).is_some() {
                    name = format!("{stem} {k}");
                    k += 1;
                }
                self.matrices_win
                    .file
                    .matrices
                    .push(Matrix::from_rules(&name, &f.rules));
                self.matrices_win.selected = self.matrices_win.file.matrices.len() - 1;
                self.matrices_touch();
                self.matrices_win.message = Some(self.i18n.t_args(
                    "matrices-imported",
                    &[("name", &name), ("n", &f.rules.len().to_string())],
                ));
            }
            Err(e) => {
                self.matrices_win.error = Some(self.i18n.t_arg("replace-rules-error", "error", &e))
            }
        }
    }

    /// « Tester sur la sélection (direct) »: the matrix being edited, on
    /// the selection of the grid, without saving it.
    fn matrices_test_selection(&mut self) {
        let Some(m) = self
            .matrices_win
            .file
            .matrices
            .get(self.matrices_win.selected)
            .cloned()
        else {
            return;
        };
        match m.compile(Direction::Direct) {
            Ok(c) => {
                let lines = self.selection.targets();
                let n = self.matrix_apply_lines(&lines, &c, &m.name);
                self.matrices_win.message = Some(self.i18n.t_args(
                    "replace-matrix-done",
                    &[
                        ("name", &m.name),
                        ("direction", &self.i18n.t("replace-direct").to_lowercase()),
                        ("n", &n.to_string()),
                    ],
                ));
            }
            Err((n, e)) => {
                self.matrices_win.error = Some(self.i18n.t_args(
                    "matrices-rule-error",
                    &[("n", &n.to_string()), ("error", &e)],
                ))
            }
        }
    }

    /// The lines the selected matrix would change in the active document.
    fn matrices_preview(&mut self) {
        let (serial, revision) = self
            .active_doc()
            .map(|d| (d.serial, d.undo.revision))
            .unwrap_or((0, 0));
        let w = &self.matrices_win;
        let key = (serial, revision, w.selected, w.edits, w.lang.clone());
        if w.preview_key.as_ref() == Some(&key) {
            return;
        }
        let preview = match (self.active_doc(), w.file.matrices.get(w.selected)) {
            (Some(doc), Some(m)) => {
                let all: Vec<usize> = (0..doc.tm.entries.len()).collect();
                let count = |dir| {
                    m.compile(dir)
                        .ok()
                        .map(|c| plan(&doc.tm.entries, &all, &c).len())
                };
                count(Direction::Direct).map(|d| {
                    let inv = if m.blocking_inverse().is_some() {
                        None
                    } else {
                        count(Direction::Inverse)
                    };
                    (d, inv)
                })
            }
            _ => None,
        };
        self.matrices_win.preview = preview;
        self.matrices_win.preview_key = Some(key);
    }

    pub(crate) fn matrices_window(&mut self, ctx: &egui::Context) {
        if !self.matrices_win.open {
            return;
        }
        self.matrices_preview();
        let id = egui::ViewportId::from_hash_of("ximod-matrices");
        let [sx, sy, sw, sh] = self.config.matrices_win;
        let just_opened = self.matrices_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(760.0), sh.max(420.0)],
            )
        });
        let lang_name = self.matrices_lang_label(&self.matrices_win.lang.clone());
        let title = self.i18n.t_arg("matrices-title", "lang", &lang_name);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([760.0, 420.0]);
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
            self.matrices_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::bottom("matrices_bottom").show(vctx, |ui| {
                if self.matrices_bottom(ui) {
                    close = true;
                }
            });
            egui::CentralPanel::default().show(vctx, |ui| {
                self.matrices_body(ui);
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.matrices_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            if self.matrices_win.dirty {
                self.matrices_win.confirm_close = true;
            } else {
                self.matrices_win.open = false;
                let _ = self.config.save();
            }
        }
    }

    fn matrices_lang_label(&self, lang: &str) -> String {
        if lang == crate::matrices::ALL {
            self.i18n.t("matrices-all-langs")
        } else {
            self.i18n.paren(&self.i18n.display_name(lang), lang)
        }
    }

    /// The language, the file, the matrices and their rules (figure B.1).
    fn matrices_body(&mut self, ui: &mut egui::Ui) {
        // 1 — language and file.
        let mut switch: Option<String> = None;
        ui.horizontal(|ui| {
            ui.strong(self.i18n.t("matrices-lang"));
            let cur = self.matrices_lang_label(&self.matrices_win.lang);
            egui::ComboBox::from_id_salt("ximod-matrices-lang")
                .selected_text(cur)
                .width(220.0)
                .show_field(ui, |ui| {
                    for l in self.matrices_win.languages.clone() {
                        let label = self.matrices_lang_label(&l);
                        if ui
                            .selectable_label(l == self.matrices_win.lang, label)
                            .clicked()
                        {
                            switch = Some(l);
                        }
                    }
                });
            let dirs = Dirs::installed();
            let lang = self.matrices_win.lang.clone();
            let (path, origin) = match self.matrices_win.origin {
                Some(Origin::User) => (
                    dirs.user_path(&lang).unwrap_or_default(),
                    self.i18n.t("matrices-origin-user"),
                ),
                Some(Origin::Shipped) => (
                    dirs.shipped_path(&lang),
                    self.i18n.t("matrices-origin-shipped"),
                ),
                _ => (
                    dirs.user_path(&lang).unwrap_or_default(),
                    self.i18n.t("matrices-origin-none"),
                ),
            };
            // The file name, the full path on hover: the path of the
            // configuration folder is long enough to push the origin out.
            let short = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.label(
                egui::RichText::new(
                    self.i18n
                        .t_args("matrices-file", &[("path", &short), ("origin", &origin)]),
                )
                .weak()
                .small(),
            )
            .on_hover_text(path.display().to_string());
        });
        if let Some(l) = switch
            && l != self.matrices_win.lang
        {
            if self.matrices_win.dirty {
                self.matrices_win.error = Some(self.i18n.t("matrices-unsaved-switch"));
            } else {
                self.matrices_load(&l);
            }
        }
        ui.separator();
        let mut touched = false;
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(MATRICES_LIST_W, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(MATRICES_LIST_W);
                    // 2 — the matrices.
                    ui.strong(self.i18n.t("matrices-list"));
                    egui::ScrollArea::vertical()
                        .id_salt("ximod-matrices-list")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for (k, m) in self.matrices_win.file.matrices.iter().enumerate() {
                                let label = if m.name.is_empty() {
                                    "—".to_string()
                                } else {
                                    m.name.clone()
                                };
                                if ui
                                    .selectable_label(k == self.matrices_win.selected, label)
                                    .clicked()
                                {
                                    self.matrices_win.selected = k;
                                }
                            }
                        });
                    ui.add_space(4.0);
                    let sel = self.matrices_win.selected;
                    if let Some(m) = self.matrices_win.file.matrices.get_mut(sel) {
                        ui.label(self.i18n.t("matrices-name"));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut m.name)
                                    .desired_width(MATRICES_LIST_W - 10.0),
                            )
                            .changed()
                        {
                            touched = true;
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button(self.i18n.t("matrices-add")).clicked() {
                            let mut name = self.i18n.t("matrices-new-name");
                            let mut k = 2;
                            while self.matrices_win.file.find(&name).is_some() {
                                name = format!("{} {k}", self.i18n.t("matrices-new-name"));
                                k += 1;
                            }
                            self.matrices_win.file.matrices.push(Matrix {
                                name,
                                rules: Vec::new(),
                            });
                            self.matrices_win.selected = self.matrices_win.file.matrices.len() - 1;
                            touched = true;
                        }
                        if ui
                            .add_enabled(
                                self.matrices_win.file.matrices.get(sel).is_some(),
                                egui::Button::new(self.i18n.t("matrices-duplicate")),
                            )
                            .clicked()
                            && let Some(m) = self.matrices_win.file.matrices.get(sel).cloned()
                        {
                            let mut copy = m;
                            copy.name = self.i18n.t_arg("matrices-copy-of", "name", &copy.name);
                            self.matrices_win.file.matrices.insert(sel + 1, copy);
                            self.matrices_win.selected = sel + 1;
                            touched = true;
                        }
                        if ui
                            .add_enabled(
                                self.matrices_win.file.matrices.get(sel).is_some(),
                                egui::Button::new(self.i18n.t("matrices-remove")),
                            )
                            .clicked()
                        {
                            self.matrices_win.file.matrices.remove(sel);
                            self.matrices_win.selected = sel.saturating_sub(1);
                            touched = true;
                        }
                    });
                },
            );
            ui.separator();
            let sel = self.matrices_win.selected;
            ui.vertical(|ui| {
                // 3 — the rules of the matrix.
                let Some(m) = self.matrices_win.file.matrices.get_mut(sel) else {
                    ui.label(egui::RichText::new(self.i18n.t("matrices-none")).weak());
                    return;
                };
                ui.strong(self.i18n.t_arg("matrices-rules-of", "name", &m.name));
                let mut action: Option<(usize, i32)> = None;
                egui::ScrollArea::both()
                    .id_salt("ximod-matrices-rules")
                    .max_height(ui.available_height() - 40.0)
                    .show(ui, |ui| {
                        egui::Grid::new("ximod-matrices-grid")
                            .striped(true)
                            .num_columns(9)
                            .show(ui, |ui| {
                                ui.label("");
                                ui.strong(self.i18n.t("replace-find"));
                                ui.strong(self.i18n.t("replace-with"));
                                ui.strong(self.i18n.t("replace-mode"));
                                ui.strong(self.i18n.t("matrices-col-case"));
                                ui.strong(self.i18n.t("matrices-col-whole"));
                                ui.strong(self.i18n.t("matrices-col-inverse"));
                                ui.strong(self.i18n.t("matrices-inverse-label"));
                                ui.label("");
                                ui.end_row();
                                let n = m.rules.len();
                                for (i, r) in m.rules.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(
                                                i > 0,
                                                egui::Button::new(crate::icons::CARET_UP).small(),
                                            )
                                            .clicked()
                                        {
                                            action = Some((i, -1));
                                        }
                                        if ui
                                            .add_enabled(
                                                i + 1 < n,
                                                egui::Button::new(crate::icons::CARET_DOWN).small(),
                                            )
                                            .clicked()
                                        {
                                            action = Some((i, 1));
                                        }
                                    });
                                    let mono = egui::TextStyle::Monospace;
                                    touched |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut r.rule.find)
                                                .font(mono.clone())
                                                .desired_width(150.0),
                                        )
                                        .changed();
                                    touched |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut r.rule.replace)
                                                .font(mono.clone())
                                                .desired_width(130.0),
                                        )
                                        .changed();
                                    egui::ComboBox::from_id_salt(("ximod-matrices-mode", i))
                                        .selected_text(self.i18n.t(r.rule.mode.i18n_key()))
                                        .width(80.0)
                                        .show_field(ui, |ui| {
                                            for md in Mode::ALL {
                                                if ui
                                                    .selectable_value(
                                                        &mut r.rule.mode,
                                                        md,
                                                        self.i18n.t(md.i18n_key()),
                                                    )
                                                    .changed()
                                                {
                                                    touched = true;
                                                }
                                            }
                                        });
                                    touched |=
                                        ui.checkbox(&mut r.rule.case_sensitive, "").changed();
                                    touched |= ui.checkbox(&mut r.rule.whole, "").changed();
                                    let inv_word = if r.has_explicit_inverse() {
                                        self.i18n.t("matrices-inverse-explicit")
                                    } else if r.inverse_rule().is_some() {
                                        self.i18n.t("matrices-inverse-derived")
                                    } else {
                                        self.i18n.t("matrices-inverse-none")
                                    };
                                    ui.label(inv_word);
                                    let inv = r.inverse.get_or_insert_with(Default::default);
                                    ui.horizontal(|ui| {
                                        touched |= ui
                                            .add(
                                                egui::TextEdit::singleline(&mut inv.find)
                                                    .font(mono.clone())
                                                    .hint_text(self.i18n.t("replace-find"))
                                                    .desired_width(120.0),
                                            )
                                            .changed();
                                        ui.label(crate::icons::ARROW_RIGHT);
                                        touched |= ui
                                            .add(
                                                egui::TextEdit::singleline(&mut inv.replace)
                                                    .font(mono)
                                                    .hint_text(self.i18n.t("replace-with"))
                                                    .desired_width(100.0),
                                            )
                                            .changed();
                                    });
                                    if r.inverse
                                        .as_ref()
                                        .is_some_and(|i| i.find.is_empty() && i.replace.is_empty())
                                    {
                                        r.inverse = None;
                                    }
                                    if ui
                                        .button(crate::icons::X)
                                        .on_hover_text(self.i18n.t("matrices-remove-rule"))
                                        .clicked()
                                    {
                                        action = Some((i, 0));
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                if let Some((i, d)) = action {
                    match d {
                        0 => {
                            m.rules.remove(i);
                        }
                        -1 => m.rules.swap(i, i - 1),
                        _ => m.rules.swap(i, i + 1),
                    }
                    touched = true;
                }
                if ui.button(self.i18n.t("matrices-add-rule")).clicked() {
                    m.rules.push(MatrixRule::new("", "", Mode::Normal));
                    touched = true;
                }
            });
        });
        if touched {
            self.matrices_touch();
        }
    }

    /// Preview, messages and buttons; returns `true` to close.
    fn matrices_bottom(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            match self.matrices_win.preview {
                Some((d, Some(i))) => {
                    ui.label(self.i18n.t_args(
                        "matrices-preview",
                        &[("direct", &d.to_string()), ("inverse", &i.to_string())],
                    ));
                }
                Some((d, None)) => {
                    ui.label(
                        self.i18n
                            .t_arg("matrices-preview-direct", "direct", &d.to_string()),
                    );
                }
                None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(self.i18n.t("matrices-no-restart"))
                        .weak()
                        .small(),
                );
            });
        });
        if let Some(e) = &self.matrices_win.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(w) = &self.matrices_win.warning {
            ui.colored_label(ui.visuals().warn_fg_color, w);
        }
        if let Some(m) = &self.matrices_win.message {
            ui.label(egui::RichText::new(m).small());
        }
        if self.matrices_win.confirm_close {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, self.i18n.t("matrices-unsaved"));
                if ui.button(self.i18n.t("btn-save")).clicked() && self.matrices_save() {
                    self.matrices_win.confirm_close = false;
                    self.matrices_win.open = false;
                }
                if ui.button(self.i18n.t("matrices-discard")).clicked() {
                    self.matrices_win.confirm_close = false;
                    self.matrices_win.dirty = false;
                    self.matrices_win.open = false;
                }
                if ui.button(self.i18n.t("btn-cancel")).clicked() {
                    self.matrices_win.confirm_close = false;
                }
            });
        }
        let mut action = 0u8;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.matrices_win.dirty,
                    egui::Button::new(egui::RichText::new(self.i18n.t("btn-save")).strong()),
                )
                .clicked()
            {
                action = 1;
            }
            if ui
                .add_enabled(
                    self.matrices_win.origin == Some(Origin::User),
                    egui::Button::new(self.i18n.t("matrices-restore")),
                )
                .clicked()
            {
                action = 2;
            }
            let can_test = !self.selection.targets().is_empty() && self.active_doc().is_some();
            if ui
                .add_enabled(
                    can_test,
                    egui::Button::new(self.i18n.t("matrices-test-selection")),
                )
                .clicked()
            {
                action = 3;
            }
            if ui.button(self.i18n.t("matrices-import-rules")).clicked() {
                action = 4;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.t("btn-close")).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(4.0);
        match action {
            1 => {
                self.matrices_save();
            }
            2 => self.matrices_restore(),
            3 => self.matrices_test_selection(),
            4 => self.matrices_import(),
            _ => {}
        }
        close
    }
}
