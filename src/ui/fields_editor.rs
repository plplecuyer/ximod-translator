//! Outils ▸ Données ▸ Champs traduisibles par jeu… (A-06).
//!
//! Edits the user copy of a game's field file (`<data>/fields/<id>.json`).
//! The table shows the *effective* list — the game's own entries plus the
//! inherited ones, marked — so Skyrim SE does not look empty just because its
//! file only says `inherits: Skyrim`. Editing is done on the selected row in
//! the panel below the table (simpler and sturdier than cell editing in egui).
//! On save, only the entries that differ from the parents are written, so the
//! user file stays a small overlay and keeps following shipped updates.
//! Validation runs after every change (R-A06-05); an invalid file cannot be
//! saved. Changes apply at the next load of a plugin (R-A06-06).

use crate::ui::theme::ComboField as _;
use std::path::PathBuf;

use eframe::egui;

use crate::games::{
    ChainLink, ContextRule, FieldFile, FieldMode, FieldRule, FieldSource, FieldTable, Game,
    registry,
};
use crate::i18n::I18n;

/// Which key an entry uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Sig,
    Edid,
    Value,
}

/// One effective row.
#[derive(Debug, Clone)]
struct Row {
    rule: FieldRule,
    /// The parent game this row comes from unchanged (`None`: the game's own).
    inherited_from: Option<Game>,
}

/// Text buffers of the selected row's editable values.
#[derive(Debug, Clone, Default)]
struct RowEdit {
    key: String,
    max_chars: String,
    max_bytes: String,
    rule_field: String,
    rule_value: String,
}

pub struct FieldsEditor {
    pub game: Game,
    header: FieldFile,
    /// Parents' merged rules, by key (to tell an override from an inherited row).
    parent_rules: Vec<FieldRule>,
    rows: Vec<Row>,
    source: FieldSource,
    user_path: Option<PathBuf>,
    dirty: bool,
    issues: Vec<crate::games::fields::ValidationIssue>,
    pub filter: String,
    selected: Option<usize>,
    edit: RowEdit,
    /// Status line under the table (i18n key + argument), and whether it is an error.
    message: Option<(String, bool)>,
    /// Frames left during which the table scrolls to the selection (the first
    /// frame after a selection may precede the table's layout).
    scroll_to_selected: u8,
    /// The localizable fields of the game (C-09): the read-only « Liste »
    /// column.
    lstrings: Option<crate::lstrings::Table>,
}

impl FieldsEditor {
    pub fn new(game: Game) -> Self {
        let mut e = FieldsEditor {
            game,
            header: FieldFile {
                format: crate::games::fields::FIELDS_FORMAT,
                game: game.id().into(),
                inherits: None,
                tags: vec![],
                fields: vec![],
            },
            parent_rules: Vec::new(),
            rows: Vec::new(),
            source: FieldSource::Builtin,
            user_path: None,
            dirty: false,
            issues: Vec::new(),
            filter: String::new(),
            selected: None,
            edit: RowEdit::default(),
            message: None,
            scroll_to_selected: 0,
            lstrings: None,
        };
        e.load(game);
        e
    }

    /// (Re)load the effective list of `game` from the registry's folders.
    pub fn load(&mut self, game: Game) {
        self.game = game;
        self.lstrings = crate::lstrings::Table::load(game, &crate::config::paths::get().data_dir);
        let reg = registry();
        let (chain, _issues): (Vec<ChainLink>, _) = match reg.fields_dirs() {
            Some((shipped, user)) => {
                self.user_path = user.map(|u| u.join(format!("{}.json", game.id())));
                FieldTable::load_chain(shipped, user, game)
            }
            None => {
                self.user_path = None;
                let own = FieldFile::builtin(game).map(|f| ChainLink {
                    game,
                    file: f,
                    source: FieldSource::Builtin,
                });
                (own.into_iter().collect(), Vec::new())
            }
        };
        drop(reg);
        let (own, parents) = match chain.split_first() {
            Some((own, parents)) => (Some(own.clone()), parents.to_vec()),
            None => (None, Vec::new()),
        };
        self.parent_rules = FieldTable::merge_chain(&parents);
        let (header, source) = match own {
            Some(l) => (l.file.clone(), l.source.clone()),
            None => (
                FieldFile {
                    format: crate::games::fields::FIELDS_FORMAT,
                    game: game.id().into(),
                    inherits: None,
                    tags: vec![],
                    fields: vec![],
                },
                FieldSource::Builtin,
            ),
        };
        self.source = source;
        // Effective rows: parents first, then the game's own (overrides replace).
        let mut rows: Vec<Row> = Vec::new();
        let parent_of = |key: &str| {
            parents
                .iter()
                .find(|l| l.file.fields.iter().any(|r| r.key() == key))
                .map(|l| l.game)
        };
        for r in &self.parent_rules {
            rows.push(Row {
                rule: r.clone(),
                inherited_from: parent_of(&r.key()),
            });
        }
        for r in &header.fields {
            let k = r.key();
            if let Some(row) = rows.iter_mut().find(|row| row.rule.key() == k) {
                row.rule = r.clone();
                row.inherited_from = None;
            } else {
                rows.push(Row {
                    rule: r.clone(),
                    inherited_from: None,
                });
            }
        }
        self.header = FieldFile {
            fields: vec![],
            ..header
        };
        self.rows = rows;
        self.dirty = false;
        self.selected = None;
        self.edit = RowEdit::default();
        self.message = None;
        self.validate();
    }

    fn validate(&mut self) {
        let f = FieldFile {
            fields: self.rows.iter().map(|r| r.rule.clone()).collect(),
            ..self.header.clone()
        };
        self.issues = f.validate();
    }

    fn can_save(&self) -> bool {
        self.dirty && self.issues.is_empty() && self.user_path.is_some()
    }

    /// The game's own entries: rows that are not inherited unchanged.
    fn own_rules(&self) -> Vec<FieldRule> {
        self.rows
            .iter()
            .filter(|r| r.inherited_from.is_none())
            .filter(|r| !self.parent_rules.contains(&r.rule))
            .map(|r| r.rule.clone())
            .collect()
    }

    fn save(&mut self) {
        let Some(path) = self.user_path.clone() else {
            return;
        };
        let f = FieldFile {
            fields: self.own_rules(),
            ..self.header.clone()
        };
        match crate::config::paths::write_atomic(&path, (f.to_json() + "\n").as_bytes()) {
            Ok(()) => {
                let issues = crate::games::spec::reload_fields(self.game);
                if let Some(i) = issues.first() {
                    self.message = Some((
                        crate::l10n!("err-path-error", path = i.path.display(), error = i.reason),
                        true,
                    ));
                } else {
                    self.message = Some(("fields-applies-next-load".into(), false));
                }
                let game = self.game;
                let sel = self.selected;
                let filter = self.filter.clone();
                self.load(game);
                self.filter = filter;
                self.selected = sel.filter(|i| *i < self.rows.len());
                self.sync_edit();
                if self.message.is_none() {
                    self.message = Some(("fields-applies-next-load".into(), false));
                }
            }
            Err(e) => {
                self.message = Some((
                    crate::l10n!("err-path-error", path = path.display(), error = e),
                    true,
                ))
            }
        }
    }

    fn restore(&mut self) {
        if let Some(path) = &self.user_path
            && path.is_file()
            && let Err(e) = std::fs::remove_file(path)
        {
            self.message = Some((
                crate::l10n!("err-path-error", path = path.display(), error = e),
                true,
            ));
            return;
        }
        crate::games::spec::reload_fields(self.game);
        let game = self.game;
        self.load(game);
        self.message = Some(("fields-restored".into(), false));
    }

    fn sync_edit(&mut self) {
        self.edit = match self.selected.and_then(|i| self.rows.get(i)) {
            Some(r) => RowEdit {
                key: r
                    .rule
                    .sig
                    .clone()
                    .or(r.rule.edid.clone())
                    .or(r.rule.value.clone())
                    .unwrap_or_default(),
                max_chars: r.rule.max_chars.map(|v| v.to_string()).unwrap_or_default(),
                max_bytes: r.rule.max_bytes.map(|v| v.to_string()).unwrap_or_default(),
                rule_field: r
                    .rule
                    .rule
                    .as_ref()
                    .map(|c| c.parts().0.to_string())
                    .unwrap_or_default(),
                rule_value: r
                    .rule
                    .rule
                    .as_ref()
                    .map(|c| c.parts().1.to_string())
                    .unwrap_or_default(),
            },
            None => RowEdit::default(),
        };
    }

    fn key_kind(r: &FieldRule) -> KeyKind {
        if r.sig.is_some() {
            KeyKind::Sig
        } else if r.edid.is_some() {
            KeyKind::Edid
        } else {
            KeyKind::Value
        }
    }

    fn set_key(r: &mut FieldRule, kind: KeyKind, text: &str) {
        r.sig = None;
        r.edid = None;
        r.value = None;
        let t = text.trim().to_string();
        match kind {
            KeyKind::Sig => r.sig = Some(t.to_uppercase()),
            KeyKind::Edid => r.edid = Some(t),
            KeyKind::Value => r.value = Some(t),
        }
    }

    /// Mark the selected row as the game's own and the file as modified.
    fn touch(&mut self) {
        if let Some(i) = self.selected
            && let Some(r) = self.rows.get_mut(i)
        {
            r.inherited_from = None;
        }
        self.dirty = true;
        self.message = None;
        self.validate();
    }

    fn add_row(&mut self) {
        self.rows.push(Row {
            rule: FieldRule {
                sig: Some(String::new()),
                ..Default::default()
            },
            inherited_from: None,
        });
        self.selected = Some(self.rows.len() - 1);
        self.filter.clear();
        self.sync_edit();
        self.scroll_to_selected = 3;
        self.dirty = true;
        self.message = None;
        self.validate();
    }

    /// Remove the selected row; an inherited row cannot be removed from the
    /// parent file, so it becomes an `ignore` override instead.
    fn remove_row(&mut self) {
        let Some(i) = self.selected else { return };
        if i >= self.rows.len() {
            return;
        }
        if self
            .parent_rules
            .iter()
            .any(|p| p.key() == self.rows[i].rule.key())
        {
            self.rows[i].rule.mode = FieldMode::Ignore;
            self.rows[i].inherited_from = None;
        } else {
            self.rows.remove(i);
            self.selected = if self.rows.is_empty() {
                None
            } else {
                Some(i.min(self.rows.len() - 1))
            };
        }
        self.sync_edit();
        self.dirty = true;
        self.message = None;
        self.validate();
    }

    fn mode_label(i18n: &I18n, m: FieldMode) -> String {
        i18n.t(match m {
            FieldMode::Translate => "fields-mode-translate",
            FieldMode::Locked => "fields-mode-locked",
            FieldMode::Unused => "fields-mode-unused",
            FieldMode::Ignore => "fields-mode-ignore",
        })
    }

    /// Readable rendering of a context rule.
    fn rule_text(i18n: &I18n, c: &ContextRule) -> String {
        let (field, value) = c.parts();
        let key = match c {
            ContextRule::EdidPrefix { .. } => "fields-rule-edid-prefix",
            ContextRule::EdidGlob { .. } => "fields-rule-edid-glob",
            ContextRule::FieldEquals { .. } => "fields-rule-field-eq",
            ContextRule::FieldNotEquals { .. } => "fields-rule-field-ne",
            ContextRule::ValuePrefix { .. } => "fields-rule-value-prefix",
        };
        i18n.t_args(key, &[("field", field), ("value", value)])
    }

    fn yes_no(i18n: &I18n, b: bool) -> String {
        i18n.t(if b { "fields-yes" } else { "fields-no" })
    }

    /// Draw the window content. Returns `true` when the user asked to close.
    pub fn body(&mut self, ui: &mut egui::Ui, i18n: &I18n) -> bool {
        let mut close = false;
        let dash = "—";

        // ---- Top bar: game, file, filter ----
        ui.horizontal(|ui| {
            ui.label(i18n.t("fields-game"));
            let mut new_game: Option<Game> = None;
            egui::ComboBox::from_id_salt("fields_game")
                .selected_text(self.game.display_name())
                .show_field(ui, |ui| {
                    for g in Game::ALL {
                        if ui
                            .selectable_label(*g == self.game, g.display_name())
                            .clicked()
                            && *g != self.game
                        {
                            new_game = Some(*g);
                        }
                    }
                });
            if let Some(g) = new_game {
                self.load(g);
            }
            ui.add_space(12.0);
            ui.label(i18n.t("fields-file"));
            let (path, tag) = match &self.source {
                FieldSource::User(p) => (p.display().to_string(), i18n.t("fields-src-user")),
                FieldSource::Shipped(p) => (p.display().to_string(), i18n.t("fields-src-shipped")),
                FieldSource::Builtin => (String::from("—"), i18n.t("fields-src-builtin")),
            };
            ui.add(
                egui::Label::new(egui::RichText::new(i18n.paren(&path, &tag)).weak()).truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(i18n.t("btn-clear")).clicked() {
                    self.filter.clear();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(i18n.t("fields-filter-hint"))
                        .desired_width(260.0),
                );
            });
        });
        if let Some(parent) = &self.header.inherits {
            ui.label(
                egui::RichText::new(i18n.t_arg(
                    "fields-inherits",
                    "game",
                    Game::from_id(parent).display_name(),
                ))
                .weak()
                .small(),
            );
        }
        ui.add_space(4.0);

        // ---- Table ----
        let needle = self.filter.trim().to_lowercase();
        let visible: Vec<usize> =
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    needle.is_empty()
                        || r.rule.key().to_lowercase().contains(&needle)
                        || r.rule.comment.to_lowercase().contains(&needle)
                        || r.rule.rule.as_ref().is_some_and(|c| {
                            Self::rule_text(i18n, c).to_lowercase().contains(&needle)
                        })
                })
                .map(|(i, _)| i)
                .collect();
        let bad_lines: std::collections::HashSet<usize> =
            self.issues.iter().map(|i| i.line - 1).collect();

        let bottom_h = if self.selected.is_some() {
            190.0
        } else {
            130.0
        };
        let table_h = (ui.available_height() - bottom_h).max(120.0);
        let text_h = egui::TextStyle::Body.resolve(ui.style()).size + 8.0;
        let mut clicked: Option<usize> = None;
        let error_color = egui::Color32::from_rgb(220, 80, 60);
        let scroll_pos = if self.scroll_to_selected > 0 {
            self.scroll_to_selected -= 1;
            ui.ctx().request_repaint();
            self.selected
                .and_then(|s| visible.iter().position(|v| *v == s))
        } else {
            None
        };
        use egui_extras::{Column, TableBuilder};
        ui.push_id("fields_table", |ui| {
            let mut table = TableBuilder::new(ui)
                .striped(true)
                .min_scrolled_height(table_h)
                .max_scroll_height(table_h)
                .animate_scrolling(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(120.0).at_least(80.0))
                .column(Column::initial(90.0).at_least(60.0))
                .column(Column::initial(70.0).at_least(50.0))
                .column(Column::initial(70.0).at_least(50.0))
                .column(Column::initial(90.0).at_least(60.0))
                .column(Column::initial(80.0).at_least(60.0))
                .column(Column::initial(200.0).at_least(80.0))
                .column(Column::remainder().at_least(120.0))
                .sense(egui::Sense::click());
            if let Some(pos) = scroll_pos {
                table = table.scroll_to_row(pos, Some(egui::Align::Center));
            }
            table
                .header(text_h, |mut h| {
                    for key in [
                        "fields-col-sig",
                        "fields-col-mode",
                        "fields-col-nonempty",
                        "fields-col-max",
                        "fields-col-newline",
                        "fields-col-list",
                        "fields-col-rule",
                        "fields-col-comment",
                    ] {
                        h.col(|ui| {
                            let r = ui.strong(i18n.t(key));
                            if key == "fields-col-list" {
                                r.on_hover_text(i18n.t("fields-col-list-tip"));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(text_h, visible.len(), |mut row| {
                        let idx = visible[row.index()];
                        let r = &self.rows[idx];
                        row.set_selected(self.selected == Some(idx));
                        let bad = bad_lines.contains(&idx);
                        let inherited = r.inherited_from.is_some();
                        let paint = |t: String| {
                            let mut rt = egui::RichText::new(t);
                            if bad {
                                rt = rt.color(error_color);
                            } else if inherited {
                                rt = rt.weak();
                            }
                            rt
                        };
                        let key = match Self::key_kind(&r.rule) {
                            KeyKind::Sig => r.rule.sig.clone().unwrap_or_default(),
                            KeyKind::Edid => {
                                format!("EDID {}", r.rule.edid.clone().unwrap_or_default())
                            }
                            KeyKind::Value => format!(
                                "{} {}",
                                i18n.t("fields-key-value"),
                                r.rule.value.clone().unwrap_or_default()
                            ),
                        };
                        row.col(|ui| {
                            let resp = ui.label(paint(key));
                            if let Some(g) = r.inherited_from {
                                resp.on_hover_text(i18n.t_arg(
                                    "fields-inherited-from",
                                    "game",
                                    g.display_name(),
                                ));
                            }
                        });
                        row.col(|ui| {
                            ui.label(paint(Self::mode_label(i18n, r.rule.mode)));
                        });
                        let ignored = r.rule.mode == FieldMode::Ignore || r.rule.is_pattern();
                        row.col(|ui| {
                            ui.label(paint(if ignored {
                                dash.into()
                            } else {
                                Self::yes_no(i18n, r.rule.non_empty)
                            }));
                        });
                        row.col(|ui| {
                            ui.label(paint(
                                r.rule
                                    .max_chars
                                    .filter(|_| !ignored)
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| dash.into()),
                            ));
                        });
                        row.col(|ui| {
                            ui.label(paint(match (ignored, r.rule.newline) {
                                (true, _) | (false, None) => dash.into(),
                                (false, Some(b)) => Self::yes_no(i18n, b),
                            }));
                        });
                        // The list of a localizable field (C-09), read-only.
                        row.col(|ui| {
                            let list = r.rule.sig.as_deref().and_then(|sig| {
                                let (rec, fld) = sig.split_once(':')?;
                                self.lstrings
                                    .as_ref()?
                                    .fields
                                    .get(&format!("{rec}:{fld}"))
                                    .map(|f| f.list.extension())
                            });
                            ui.label(paint(list.unwrap_or(dash).to_string()));
                        });
                        row.col(|ui| {
                            ui.label(paint(
                                r.rule
                                    .rule
                                    .as_ref()
                                    .map(|c| Self::rule_text(i18n, c))
                                    .unwrap_or_default(),
                            ));
                        });
                        row.col(|ui| {
                            // Truncated: egui shows the full comment on hover by itself.
                            ui.add(egui::Label::new(paint(r.rule.comment.clone())).truncate());
                        });
                        if row.response().clicked() {
                            clicked = Some(idx);
                        }
                    });
                });
        });
        if let Some(i) = clicked {
            self.selected = Some(i);
            self.sync_edit();
        }

        // ---- Edit panel for the selected row ----
        ui.add_space(4.0);
        ui.separator();
        if let Some(i) = self.selected.filter(|i| *i < self.rows.len()) {
            let mut changed = false;
            let mut kind = Self::key_kind(&self.rows[i].rule);
            let mut mode = self.rows[i].rule.mode;
            let mut non_empty = self.rows[i].rule.non_empty;
            let mut newline = self.rows[i].rule.newline;
            let mut rule_kind: Option<String> = self.rows[i]
                .rule
                .rule
                .as_ref()
                .map(|c| c.kind().to_string());
            let mut comment = self.rows[i].rule.comment.clone();
            ui.horizontal(|ui| {
                ui.label(i18n.t("fields-edit-key"));
                for (k, label) in [
                    (KeyKind::Sig, "fields-key-sig"),
                    (KeyKind::Edid, "fields-key-edid"),
                    (KeyKind::Value, "fields-key-value"),
                ] {
                    if ui.radio(kind == k, i18n.t(label)).clicked() && kind != k {
                        kind = k;
                        changed = true;
                    }
                }
                let hint = match kind {
                    KeyKind::Sig => "WEAP:FULL",
                    _ => "LOC_FN_*",
                };
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.edit.key)
                            .hint_text(hint)
                            .desired_width(140.0),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.add_space(8.0);
                ui.label(i18n.t("fields-col-mode"));
                egui::ComboBox::from_id_salt("fields_mode")
                    .selected_text(Self::mode_label(i18n, mode))
                    .show_field(ui, |ui| {
                        for m in FieldMode::ALL {
                            if ui
                                .selectable_label(mode == m, Self::mode_label(i18n, m))
                                .clicked()
                                && mode != m
                            {
                                mode = m;
                                changed = true;
                            }
                        }
                    });
            });
            let is_sig = kind == KeyKind::Sig;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(is_sig, |ui| {
                    if ui
                        .checkbox(&mut non_empty, i18n.t("fields-col-nonempty"))
                        .changed()
                    {
                        changed = true;
                    }
                    ui.add_space(8.0);
                    ui.label(i18n.t("fields-col-max"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.edit.max_chars)
                                .desired_width(50.0),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(i18n.t("fields-max-bytes"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.edit.max_bytes)
                                .desired_width(50.0),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.add_space(8.0);
                    // Three states: unknown (—), yes, no.
                    ui.label(i18n.t("fields-col-newline"));
                    let shown = match newline {
                        None => "—".to_string(),
                        Some(b) => Self::yes_no(i18n, b),
                    };
                    egui::ComboBox::from_id_salt("fields-newline")
                        .selected_text(shown)
                        .width(70.0)
                        .show_field(ui, |ui| {
                            for (v, label) in [
                                (None, "—".to_string()),
                                (Some(true), Self::yes_no(i18n, true)),
                                (Some(false), Self::yes_no(i18n, false)),
                            ] {
                                if ui.selectable_label(newline == v, label).clicked()
                                    && newline != v
                                {
                                    newline = v;
                                    changed = true;
                                }
                            }
                        });
                });
            });
            ui.horizontal(|ui| {
                ui.add_enabled_ui(is_sig, |ui| {
                    ui.label(i18n.t("fields-col-rule"));
                    let none = i18n.t("fields-rule-none");
                    egui::ComboBox::from_id_salt("fields_rule_kind")
                        .selected_text(rule_kind.clone().unwrap_or_else(|| none.clone()))
                        .show_field(ui, |ui| {
                            if ui.selectable_label(rule_kind.is_none(), &none).clicked()
                                && rule_kind.is_some()
                            {
                                rule_kind = None;
                                changed = true;
                            }
                            for k in ContextRule::KINDS {
                                if ui
                                    .selectable_label(rule_kind.as_deref() == Some(k), k)
                                    .clicked()
                                    && rule_kind.as_deref() != Some(k)
                                {
                                    rule_kind = Some(k.to_string());
                                    changed = true;
                                }
                            }
                        });
                    let needs_field = matches!(
                        rule_kind.as_deref(),
                        Some("fieldEquals") | Some("fieldNotEquals")
                    );
                    ui.add_enabled_ui(rule_kind.is_some(), |ui| {
                        ui.add_enabled_ui(needs_field, |ui| {
                            ui.label(i18n.t("fields-rule-field"));
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.edit.rule_field)
                                        .hint_text("DNAM")
                                        .desired_width(60.0),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.label(i18n.t("fields-rule-value"));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.edit.rule_value)
                                    .desired_width(140.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                });
            });
            ui.horizontal(|ui| {
                ui.label(i18n.t("fields-col-comment"));
                if ui
                    .add(egui::TextEdit::singleline(&mut comment).desired_width(f32::INFINITY))
                    .changed()
                {
                    changed = true;
                }
            });
            if changed {
                let r = &mut self.rows[i].rule;
                Self::set_key(r, kind, &self.edit.key);
                r.mode = mode;
                if kind == KeyKind::Sig {
                    r.non_empty = non_empty;
                    r.newline = newline;
                    r.max_chars = self.edit.max_chars.trim().parse().ok();
                    r.max_bytes = self.edit.max_bytes.trim().parse().ok();
                    r.rule = rule_kind.as_deref().and_then(|k| {
                        ContextRule::from_parts(
                            k,
                            self.edit.rule_field.trim(),
                            self.edit.rule_value.trim(),
                        )
                    });
                } else {
                    r.non_empty = false;
                    r.newline = None;
                    r.max_chars = None;
                    r.max_bytes = None;
                    r.rule = None;
                }
                r.comment = comment;
                self.touch();
            }
        } else {
            ui.label(egui::RichText::new(i18n.t("fields-select-hint")).weak());
            ui.add_space(text_h * 2.0);
        }

        // ---- Buttons and status ----
        ui.add_space(4.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(i18n.t("fields-add")).clicked() {
                self.add_row();
            }
            if ui
                .add_enabled(
                    self.selected.is_some(),
                    egui::Button::new(i18n.t("fields-remove")),
                )
                .clicked()
            {
                self.remove_row();
            }
            let user_exists = self.user_path.as_ref().is_some_and(|p| p.is_file());
            if ui
                .add_enabled(user_exists, egui::Button::new(i18n.t("fields-restore")))
                .on_hover_text(i18n.t("fields-restore-hint"))
                .clicked()
            {
                self.restore();
            }
            ui.add_space(8.0);
            if let Some(first) = self.issues.first() {
                let reason = i18n.t_arg(first.reason.i18n_key(), "text", first.reason.arg());
                let msg = i18n.t_args(
                    "fields-invalid",
                    &[("line", &first.line.to_string()), ("reason", &reason)],
                );
                ui.colored_label(error_color, msg);
            } else if let Some((m, err)) = &self.message {
                let text = if m.starts_with("fields-") {
                    i18n.t(m)
                } else {
                    m.clone()
                };
                if *err {
                    ui.colored_label(error_color, text);
                } else {
                    ui.colored_label(egui::Color32::from_rgb(60, 160, 80), text);
                }
            } else {
                let n = self.rows.iter().filter(|r| r.rule.sig.is_some()).count();
                ui.colored_label(
                    egui::Color32::from_rgb(60, 160, 80),
                    i18n.t_arg("fields-valid", "count", &n.to_string()),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.can_save(), egui::Button::new(i18n.t("btn-save")))
                    .clicked()
                {
                    self.save();
                }
                if ui.button(i18n.t("btn-close")).clicked() {
                    close = true;
                }
            });
        });
        ui.label(egui::RichText::new(i18n.t("fields-footer")).weak().small());
        close
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Select a row by index (keyboard / preview).
    pub fn select(&mut self, idx: usize) {
        if idx < self.rows.len() {
            self.selected = Some(idx);
            self.sync_edit();
            self.scroll_to_selected = 3;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fallout 76 (no fixture uses it) with a temporary user folder; the
    /// shipped folder is the crate's.
    fn setup() -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("ximod-fields-editor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data");
        crate::games::spec::init(&data.join("games"), &data.join("fields"), Some(&tmp));
        tmp
    }

    #[test]
    fn inherited_rows_edit_remove_and_overlay_save() {
        let tmp = setup();
        let mut ed = FieldsEditor::new(Game::Fallout76);
        assert_eq!(ed.header.inherits.as_deref(), Some("Fallout4"));
        assert!(
            ed.rows
                .iter()
                .all(|r| r.inherited_from == Some(Game::Fallout4))
        );
        assert!(ed.own_rules().is_empty());
        assert!(!ed.can_save());

        // Editing an inherited row makes it the game's own.
        let i = ed
            .rows
            .iter()
            .position(|r| r.rule.sig.as_deref() == Some("WEAP:FULL"))
            .unwrap();
        ed.select(i);
        ed.rows[i].rule.max_chars = Some(64);
        ed.touch();
        assert_eq!(ed.own_rules().len(), 1);
        assert!(ed.can_save());

        // Removing an inherited row turns it into an `ignore` override.
        let j = ed
            .rows
            .iter()
            .position(|r| r.rule.sig.as_deref() == Some("BOOK:FULL"))
            .unwrap();
        ed.select(j);
        ed.remove_row();
        assert_eq!(ed.rows[j].rule.mode, FieldMode::Ignore);
        assert_eq!(ed.own_rules().len(), 2);

        // A new row, invalid until it gets a key → cannot save; then valid.
        ed.add_row();
        assert!(!ed.issues.is_empty() && !ed.can_save());
        let k = ed.rows.len() - 1;
        FieldsEditor::set_key(&mut ed.rows[k].rule, KeyKind::Sig, "xxxx:desc");
        ed.touch();
        assert!(ed.issues.is_empty(), "{:?}", ed.issues);
        // Duplicate key → invalid.
        FieldsEditor::set_key(&mut ed.rows[k].rule, KeyKind::Sig, "WEAP:FULL");
        ed.touch();
        assert!(ed.issues.iter().any(|i| matches!(
            i.reason,
            crate::games::fields::ValidationReason::Duplicate(_)
        )));
        FieldsEditor::set_key(&mut ed.rows[k].rule, KeyKind::Sig, "XXXX:DESC");
        ed.touch();

        // Save writes only the overlay (3 entries) and reloads the registry.
        ed.save();
        let written = std::fs::read_to_string(tmp.join("Fallout76.json")).unwrap();
        let f = FieldFile::from_json(&written, Some("Fallout76")).unwrap();
        assert_eq!(f.inherits.as_deref(), Some("Fallout4"));
        assert_eq!(f.fields.len(), 3);
        assert!(!ed.dirty);
        assert!(matches!(ed.source, FieldSource::User(_)));
        let reg = registry();
        assert_eq!(
            reg.fields(Game::Fallout76)
                .limits("WEAP", "FULL")
                .unwrap()
                .max_chars,
            Some(64)
        );
        assert!(!reg.fields(Game::Fallout76).is_translatable("BOOK", "FULL"));
        drop(reg);

        // Restore removes the user file and brings the shipped table back.
        ed.restore();
        assert!(!tmp.join("Fallout76.json").exists());
        assert!(
            registry()
                .fields(Game::Fallout76)
                .is_translatable("BOOK", "FULL")
        );
        assert!(ed.own_rules().is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
