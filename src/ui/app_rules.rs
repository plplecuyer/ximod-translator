//! Règles d'exclusion (B-18): the rules of a document (its file, the rules
//! learned on the mod, the rules off), the window Outils › Règles
//! d'exclusion… with its three tabs — Règles actives, Éditeur, Suggestions
//! — and the generalization proposed after a line was set Ignoré by hand.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;

use super::{Doc, XimodApp, read_geometry as read_geometry_of};
use crate::exclusions::{
    self, Choice, Compiled, Counts, Dirs, Invalid, Kind, Origin, Plan, Proposal, Rule, RulesFile,
    Scope, Source, Suggestion,
};
use crate::undo::OpKind;

/// The tabs of the window.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RulesTab {
    #[default]
    Active,
    Editor,
    Suggest,
}

/// One row of the Règles actives tab.
#[derive(Debug, Clone)]
struct ActiveRow {
    id: String,
    origin: RowOrigin,
    scope: String,
    game: String,
    lines: usize,
    example: String,
    /// Can be turned off for the document.
    checkable: bool,
    on: bool,
    comment: String,
    learned: Option<exclusions::Learned>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowOrigin {
    Shipped,
    Fields,
    Game,
    Deduced,
    LearnedLocal,
    LearnedGlobal,
    User,
}

impl RowOrigin {
    fn i18n_key(self) -> &'static str {
        match self {
            RowOrigin::Shipped => "rules-origin-shipped",
            RowOrigin::Fields => "rules-origin-fields",
            RowOrigin::Game => "rules-origin-game",
            RowOrigin::Deduced => "rules-origin-deduced",
            RowOrigin::LearnedLocal => "rules-origin-learned-local",
            RowOrigin::LearnedGlobal => "rules-origin-learned-global",
            RowOrigin::User => "rules-origin-user",
        }
    }
}

/// What the counts of a document depend on.
type CountsKey = (u64, usize, bool, u64, usize);

/// The window's state.
#[derive(Default)]
pub(crate) struct RulesWindow {
    pub open: bool,
    pub needs_place: bool,
    pub tab: RulesTab,
    /// The row selected in Règles actives.
    selected: Option<String>,
    /// The counts of the active document, with their key.
    counts: Option<(CountsKey, Counts)>,
    /// Bumped when the file changes.
    pub revision: u64,
    // ---- Éditeur
    file: Option<RulesFile>,
    source: Option<Source>,
    sel: usize,
    dirty: bool,
    functions: String,
    error: Option<String>,
    message: Option<String>,
    /// The trial of the selected rule: its key, lines, first text.
    trial: Option<(String, usize, String)>,
    confirm_close: bool,
    // ---- Suggestions
    suggestions: Option<Vec<Suggestion>>,
}

/// The generalization dialog (figure B.6).
pub(crate) struct GeneralizeDlg {
    pub doc: u64,
    pub proposal: Proposal,
    pub choice: Choice,
    pub stop: bool,
}

/// The rules file of the installation (the user's copy, else the shipped
/// file), read at each use: an edit applies without restart.
pub(crate) fn rules_file() -> RulesFile {
    Dirs::installed().load().0
}

/// The date of the day, `YYYY-MM-DD`.
fn today() -> String {
    crate::backup::today()
}

impl XimodApp {
    /// The rules learned on the mod and those turned off, as its files
    /// remember them: the header of the mod's dictionary when there is
    /// one, else the pivot's meta (R-B18-03, R-B18-05).
    pub(crate) fn rules_remembered(&self, doc: &mut Doc) {
        if doc.settings.target.trim().is_empty() {
            return;
        }
        let out = crate::dict_stack::DictStack::mod_dict_path(
            &doc.path,
            &doc.settings,
            &self.config.mod_base_dir(),
        );
        if out.is_file() {
            let extra = crate::basedb::peek_extra(&out);
            if let Some(r) = extra.rules {
                doc.tm.meta.rules = r;
            }
            if let Some(o) = extra.rules_off {
                doc.tm.meta.rules_off = o;
            }
        }
    }

    /// The deductions of a document (R-B18-02), built once.
    fn rules_deduced(doc: &Doc) -> Arc<exclusions::Deduced> {
        doc.deduced
            .get_or_init(|| {
                Arc::new(exclusions::Deduced::of(
                    &doc.tm.entries,
                    Some(&doc.path),
                    &doc.interface_files,
                ))
            })
            .clone()
    }

    fn rules_game(doc: &Doc) -> String {
        if doc.settings.game.trim().is_empty() {
            doc.tm.meta.game.clone()
        } else {
            doc.settings.game.clone()
        }
    }

    /// What a document needs to lay its locks.
    pub(crate) fn rules_plan(&self, doc: &Doc) -> Plan {
        let file = rules_file();
        let game = Self::rules_game(doc);
        Plan {
            compiled: Compiled::new(&file, &doc.tm.meta.rules, &game),
            deduced: Self::rules_deduced(doc),
            off: doc.tm.meta.rules_off.iter().cloned().collect(),
            game,
            path: doc.path.to_string_lossy().replace('\\', "/"),
            facts: Self::script_facts(doc),
        }
    }

    /// The words of an invalid file or rule.
    fn rules_invalid(&self, e: &Invalid) -> String {
        match e {
            Invalid::Json(m) => self.i18n.t_arg("rules-err-json", "error", m),
            Invalid::Format(f) => self.i18n.t_arg("rules-err-format", "n", &f.to_string()),
            Invalid::NoId(k) => self.i18n.t_arg("rules-err-no-id", "n", &k.to_string()),
            Invalid::IdDup(id) => self.i18n.t_arg("rules-err-id-dup", "id", id),
            Invalid::Empty(id) => self.i18n.t_arg("rules-err-empty", "id", id),
            Invalid::Regex { id, error } => self
                .i18n
                .t_args("rules-err-regex", &[("id", id), ("error", error)]),
            Invalid::Sig { id, text } => self
                .i18n
                .t_args("rules-err-sig", &[("id", id), ("text", text)]),
            Invalid::Scope(id) => self.i18n.t_arg("rules-err-scope", "id", id),
        }
    }

    // ------------------------------------------------------ the actions

    /// Apply the rules again to document `idx` (lift, fill, lay): one
    /// undoable operation of `kind`.
    pub(crate) fn rules_reapply(&mut self, idx: usize, kind: OpKind, label: Option<String>) {
        self.close_edit_session();
        if self.docs.get(idx).is_none_or(|d| d.all.is_some()) {
            return;
        }
        self.restack_doc_as(idx, kind, label);
        if let Some(doc) = self.docs.get_mut(idx) {
            doc.undo.touch();
        }
        self.rules_win.counts = None;
        let n = self.docs[idx]
            .tm
            .entries
            .iter()
            .filter(|e| exclusions::lock_rule(e).is_some())
            .count();
        self.status = self.i18n.t_arg(
            "rules-reapplied",
            "n",
            &crate::ui::export_dialog::fmt_count(&self.i18n, n),
        );
    }

    /// Turn a rule on or off for the active document (R-B18-05).
    fn rules_toggle(&mut self, id: &str, on: bool) {
        let idx = self.active;
        let Some(doc) = self.docs.get_mut(idx) else {
            return;
        };
        doc.tm.meta.rules_off.retain(|r| r != id);
        if !on {
            doc.tm.meta.rules_off.push(id.to_string());
        }
        let kind = if on { OpKind::RuleOn } else { OpKind::RuleOff };
        self.rules_reapply(idx, kind, Some(id.to_string()));
    }

    /// Move a learned rule between the mod (local) and the user's copy
    /// (global).
    fn rules_toggle_place(&mut self, id: &str) {
        let idx = self.active;
        let Some(doc) = self.docs.get_mut(idx) else {
            return;
        };
        let dirs = Dirs::installed();
        let (mut file, _, _) = dirs.load();
        if let Some(k) = doc.tm.meta.rules.iter().position(|r| r.id == id) {
            let r = doc.tm.meta.rules.remove(k);
            file.rules.retain(|x| x.id != r.id);
            file.rules.push(r);
        } else if let Some(k) = file.rules.iter().position(|r| r.id == id) {
            let r = file.rules.remove(k);
            doc.tm.meta.rules.push(r);
        } else {
            return;
        }
        if let Err(e) = dirs.save(&file) {
            let msg = self.rules_invalid(&e);
            self.notify_err(msg);
            return;
        }
        self.rules_win.revision += 1;
        self.rules_win.file = None;
        doc.undo.touch();
        self.rules_reapply(idx, OpKind::RulesApply, Some(id.to_string()));
    }

    /// The lines a rule holds in the active document, in the grid.
    fn rules_see_lines(&mut self, id: &str) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        let g = crate::games::Game::from_id(&Self::rules_game(doc));
        let reg = crate::games::registry();
        let table = reg.fields(g);
        let loc = g.locks_loc_keys();
        let set: HashSet<usize> = doc
            .tm
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match exclusions::lock_rule(e) {
                Some(r) => r == id || (id == "functions" && r.starts_with("function:")),
                None => id.starts_with("fields:") && rule_of_fields(e, table, loc) == id,
            })
            .map(|(i, _)| i)
            .collect();
        self.filter_lock_rule = Some((id.to_string(), crate::filter::AnomalySet(Arc::new(set))));
        self.status = self.i18n.t_arg("rules-see-lines-done", "rule", id);
    }

    // ------------------------------------------------- generalization

    /// After a line was set Ignoré by hand: the proposal (R-B18-03).
    pub(crate) fn rules_after_ignore(&mut self, idx: usize) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        if doc.all.is_some() {
            return;
        }
        let file = rules_file();
        let Some(p) = exclusions::propose(&doc.tm.entries, idx, &file) else {
            return;
        };
        if !p.worth() {
            return;
        }
        let serial = doc.serial;
        if self.config.propose_generalization {
            self.generalize = Some(GeneralizeDlg {
                doc: serial,
                choice: default_choice(&p),
                proposal: p,
                stop: false,
            });
        } else {
            self.generalize_offer = Some((serial, p));
        }
    }

    /// The link of the status bar: « n autres lignes de même texte dans ce
    /// mod : Généraliser… », for the active document.
    pub(crate) fn rules_offer_label(&self) -> Option<String> {
        let (serial, p) = self.generalize_offer.as_ref()?;
        if self.active_doc().is_none_or(|d| d.serial != *serial) {
            return None;
        }
        let n = p.same_text.max(p.same_field);
        Some(self.i18n.t_arg("generalize-link", "n", &n.to_string()))
    }

    /// The link clicked: the dialog.
    pub(crate) fn rules_open_offer(&mut self) {
        if let Some((serial, p)) = self.generalize_offer.take() {
            self.generalize = Some(GeneralizeDlg {
                doc: serial,
                choice: default_choice(&p),
                proposal: p,
                stop: !self.config.propose_generalization,
            });
        }
    }

    /// The dialog of figure B.6.
    pub(crate) fn generalize_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.generalize.take() else {
            return;
        };
        let Some(idx) = self.docs.iter().position(|d| d.serial == dlg.doc) else {
            return;
        };
        let game = Self::rules_game(&self.docs[idx]);
        let p = dlg.proposal.clone();
        let mut done: Option<bool> = None;
        egui::Window::new(self.i18n.t("generalize-title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(self.i18n.t_args(
                    "generalize-intro",
                    &[("sig", &p.sig), ("text", &ellipsize(&p.text, 80))],
                ));
                ui.add_space(4.0);
                let n = |k: usize| k.to_string();
                if p.same_text >= 1 {
                    ui.radio_value(
                        &mut dlg.choice,
                        Choice::TextLocal,
                        self.i18n
                            .t_arg("generalize-text-local", "n", &n(p.same_text)),
                    );
                }
                // The prefix: the longest at first, a shorter one to choose.
                if p.prefixes.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.label(self.i18n.t("generalize-prefix-choose"));
                        let cur = dlg.proposal.prefix.clone().unwrap_or_default();
                        egui::ComboBox::from_id_salt("ximod-generalize-prefix")
                            .selected_text(format!("{}  ({})", cur.0, cur.1))
                            .show_ui(ui, |ui| {
                                for c in &p.prefixes {
                                    if ui
                                        .selectable_label(
                                            c.0 == cur.0,
                                            format!("{}  ({})", c.0, c.1),
                                        )
                                        .clicked()
                                    {
                                        dlg.proposal.prefix = Some(c.clone());
                                    }
                                }
                            });
                    });
                }
                if let Some((pre, k)) = &dlg.proposal.prefix.clone() {
                    ui.radio_value(
                        &mut dlg.choice,
                        Choice::PrefixLocal,
                        self.i18n
                            .t_args("generalize-prefix-local", &[("n", &n(*k)), ("prefix", pre)]),
                    );
                    ui.radio_value(
                        &mut dlg.choice,
                        Choice::PrefixGlobal,
                        self.i18n.t_arg("generalize-prefix-global", "prefix", pre),
                    );
                }
                let tip = self.i18n.t_arg("generalize-field-displayed", "sig", &p.sig);
                ui.add_enabled_ui(!p.displayed && p.same_field >= 1, |ui| {
                    ui.radio_value(
                        &mut dlg.choice,
                        Choice::FieldLocal,
                        self.i18n.t_args(
                            "generalize-field-local",
                            &[("n", &n(p.same_field)), ("sig", &p.sig)],
                        ),
                    )
                    .on_disabled_hover_text(&tip);
                    ui.radio_value(
                        &mut dlg.choice,
                        Choice::FieldGlobal,
                        self.i18n.t_args(
                            "generalize-field-global",
                            &[("sig", &p.sig), ("game", &game)],
                        ),
                    )
                    .on_disabled_hover_text(&tip);
                });
                ui.radio_value(
                    &mut dlg.choice,
                    Choice::None,
                    self.i18n.t("generalize-none"),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut dlg.stop, self.i18n.t("generalize-stop"));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new(self.i18n.t("btn-ok")).strong())
                        .clicked()
                    {
                        done = Some(true);
                    }
                    if ui.button(self.i18n.t("btn-cancel")).clicked() {
                        done = Some(false);
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = Some(false);
        }
        match done {
            None => self.generalize = Some(dlg),
            Some(ok) => {
                if dlg.stop == self.config.propose_generalization {
                    self.config.propose_generalization = !dlg.stop;
                    let _ = self.config.save();
                }
                if ok {
                    self.generalize_apply(idx, &dlg.proposal, &dlg.choice);
                } else {
                    // The link stays (D-75).
                    self.generalize_offer = Some((dlg.doc, dlg.proposal));
                }
            }
        }
    }

    /// Make the rule of a choice and lay it: local in the mod, global in
    /// the user's copy (D-74); one undoable operation.
    fn generalize_apply(&mut self, idx: usize, p: &Proposal, choice: &Choice) {
        if *choice == Choice::None {
            return;
        }
        let dirs = Dirs::installed();
        let (mut file, _, _) = dirs.load();
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let game = Self::rules_game(doc);
        let from = doc
            .tm
            .entries
            .get(p.line)
            .map(|e| {
                if e.edid.is_empty() {
                    format!("{:08X}", e.formid)
                } else {
                    e.edid.clone()
                }
            })
            .unwrap_or_default();
        let taken: Vec<&Rule> = file.rules.iter().chain(&doc.tm.meta.rules).collect();
        let Some(rule) = exclusions::learned_rule(p, choice, &from, &today(), &game, &taken) else {
            return;
        };
        let id = rule.id.clone();
        if exclusions::is_global(choice) {
            file.rules.push(rule);
            if let Err(e) = dirs.save(&file) {
                let msg = self.rules_invalid(&e);
                self.notify_err(msg);
                return;
            }
            self.rules_win.revision += 1;
            self.rules_win.file = None;
        } else {
            self.docs[idx].tm.meta.rules.push(rule);
        }
        let before = self.docs[idx]
            .tm
            .entries
            .iter()
            .filter(|e| exclusions::lock_rule(e) == Some(id.as_str()))
            .count();
        self.rules_reapply(idx, OpKind::Generalize, Some(id.clone()));
        let n = self.docs[idx]
            .tm
            .entries
            .iter()
            .filter(|e| exclusions::lock_rule(e) == Some(id.as_str()))
            .count()
            - before;
        self.generalize_offer = None;
        self.notify_ok(
            self.i18n
                .t_args("generalize-done", &[("id", &id), ("n", &n.to_string())]),
        );
        // A global rule reaches the other open documents.
        if exclusions::is_global(choice) {
            for k in 0..self.docs.len() {
                if k != idx && self.docs[k].all.is_none() {
                    self.rules_reapply(k, OpKind::RulesApply, Some(id.clone()));
                }
            }
        }
    }

    // ------------------------------------------------------ the window

    pub(crate) fn open_rules_window(&mut self, tab: RulesTab) {
        if self.rules_win.open && self.rules_win.tab == tab {
            self.rules_win.open = false;
            return;
        }
        self.rules_win.open = true;
        self.rules_win.needs_place = true;
        self.rules_win.tab = tab;
        self.rules_win.counts = None;
        if self.rules_win.file.is_none() {
            self.rules_editor_load();
        }
    }

    fn rules_editor_load(&mut self) {
        let (file, source, err) = Dirs::installed().load();
        self.rules_win.functions = file.functions.join("\n");
        self.rules_win.file = Some(file);
        self.rules_win.source = Some(source);
        self.rules_win.sel = 0;
        self.rules_win.dirty = false;
        self.rules_win.trial = None;
        self.rules_win.error = err.map(|e| self.rules_invalid(&e));
    }

    pub(crate) fn rules_window(&mut self, ctx: &egui::Context) {
        if !self.rules_win.open {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-rules");
        let [sx, sy, sw, sh] = self.config.rules_win;
        let just_opened = self.rules_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(820.0), sh.max(460.0)],
            )
        });
        let title = match self.active_doc() {
            Some(d) => self.i18n.t_args(
                "rules-title",
                &[
                    ("doc", &self.tab_name(self.active)),
                    ("game", &Self::rules_game(d)),
                ],
            ),
            None => self.i18n.t("menu-rules"),
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([820.0, 460.0]);
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
            self.rules_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::top("rules_tabs").show(vctx, |ui| {
                ui.horizontal(|ui| {
                    let n = self.rules_win.suggestions.as_ref().map(Vec::len);
                    for (tab, label) in [
                        (RulesTab::Active, self.i18n.t("rules-tab-active")),
                        (RulesTab::Editor, self.i18n.t("rules-tab-editor")),
                        (
                            RulesTab::Suggest,
                            match n {
                                Some(n) => {
                                    self.i18n.t_arg("rules-tab-suggest", "n", &n.to_string())
                                }
                                None => self.i18n.t("rules-tab-suggest-none"),
                            },
                        ),
                    ] {
                        if ui
                            .selectable_label(self.rules_win.tab == tab, label)
                            .clicked()
                        {
                            self.rules_win.tab = tab;
                        }
                    }
                });
            });
            egui::TopBottomPanel::bottom("rules_bottom").show(vctx, |ui| {
                if self.rules_bottom(ui) {
                    close = true;
                }
            });
            egui::CentralPanel::default().show(vctx, |ui| match self.rules_win.tab {
                RulesTab::Active => self.rules_active_tab(ui),
                RulesTab::Editor => self.rules_editor_tab(ui),
                RulesTab::Suggest => self.rules_suggest_tab(ui),
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.rules_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            if self.rules_win.dirty {
                self.rules_win.confirm_close = true;
                self.rules_win.tab = RulesTab::Editor;
            } else {
                self.rules_win.open = false;
                let _ = self.config.save();
            }
        }
    }

    /// The rows of Règles actives for the active document.
    fn rules_rows(&mut self) -> (Vec<ActiveRow>, Counts) {
        let Some(doc) = self.active_doc() else {
            return (Vec::new(), Counts::default());
        };
        let key = (
            doc.serial,
            doc.undo.len(),
            doc.undo.can_redo(),
            self.rules_win.revision,
            doc.tm.meta.rules_off.len() + 7 * doc.tm.meta.rules.len(),
        );
        let plan = self.rules_plan(doc);
        let counts = match &self.rules_win.counts {
            Some((k, c)) if *k == key => c.clone(),
            _ => {
                let c = exclusions::counts(&doc.tm.entries, &plan.compiled, &plan.ctx());
                self.rules_win.counts = Some((key, c.clone()));
                c
            }
        };
        let doc = self.active_doc().expect("checked");
        let game = Self::rules_game(doc);
        let file = rules_file();
        let off: HashSet<&String> = doc.tm.meta.rules_off.iter().collect();
        let scope_words = |s: &[Scope]| -> String {
            s.iter()
                .map(|x| self.i18n.t(x.i18n_key()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut rows = Vec::new();
        let push = |r: &Rule, origin: RowOrigin, rows: &mut Vec<ActiveRow>| {
            let (lines, example) = counts.per_rule.get(&r.id).cloned().unwrap_or_default();
            rows.push(ActiveRow {
                id: r.id.clone(),
                origin,
                scope: match r.kind {
                    Kind::Field => self.i18n.t("rules-scope-field"),
                    Kind::Edid => self.i18n.t("rules-scope-edid"),
                    Kind::Key => self.i18n.t("rules-scope-key"),
                    _ => scope_words(&r.scope),
                },
                game: r.game.clone(),
                lines,
                example,
                checkable: true,
                on: !off.contains(&r.id),
                comment: r.comment.clone(),
                learned: r.learned.clone(),
            });
        };
        for r in file
            .rules
            .iter()
            .filter(|r| r.enabled && r.fits(&game) && r.kind != Kind::Function)
        {
            let origin = match r.origin {
                Origin::Shipped => RowOrigin::Shipped,
                Origin::Game => RowOrigin::Game,
                Origin::Learned => RowOrigin::LearnedGlobal,
                Origin::User => RowOrigin::User,
            };
            push(r, origin, &mut rows);
        }
        for r in doc.tm.meta.rules.iter().filter(|r| r.fits(&game)) {
            push(r, RowOrigin::LearnedLocal, &mut rows);
        }
        // The functions of the scripts (C-06), one row.
        let (lines, example) = counts
            .per_rule
            .get("functions")
            .cloned()
            .unwrap_or_default();
        rows.push(ActiveRow {
            id: "functions".into(),
            origin: RowOrigin::Shipped,
            scope: self.i18n.t("rules-scope-script"),
            game: "*".into(),
            lines,
            example,
            checkable: true,
            on: !off.contains(&"functions".to_string()),
            comment: self.i18n.t("rules-functions-comment"),
            learned: None,
        });
        if let Some((lines, example)) = counts.per_rule.get("ident").cloned() {
            rows.push(ActiveRow {
                id: "ident".into(),
                origin: RowOrigin::Deduced,
                scope: self.i18n.t("rules-scope-script"),
                game: "*".into(),
                lines,
                example,
                checkable: true,
                on: !off.contains(&"ident".to_string()),
                comment: self.i18n.t("lock-ident"),
                learned: None,
            });
        }
        // The deductions (R-B18-02).
        for (id, key) in [
            (exclusions::DEDUCED_EDID, "rules-deduced-edid"),
            (exclusions::DEDUCED_IFKEY, "rules-deduced-ifkey"),
            (exclusions::DEDUCED_FILE, "rules-deduced-file"),
        ] {
            let (lines, example) = counts.per_rule.get(id).cloned().unwrap_or_default();
            rows.push(ActiveRow {
                id: id.into(),
                origin: RowOrigin::Deduced,
                scope: if id == exclusions::DEDUCED_EDID {
                    self.i18n.t("rules-scope-script-files")
                } else {
                    scope_words(&[Scope::Text, Scope::Script])
                },
                game: "*".into(),
                lines,
                example,
                checkable: true,
                on: !off.contains(&id.to_string()),
                comment: self.i18n.t(key),
                learned: None,
            });
        }
        // The fields file's locked fields and the engine's keys (listed,
        // never turned off here).
        let mut fields: std::collections::BTreeMap<String, (usize, String)> = Default::default();
        let g = crate::games::Game::from_id(&game);
        let reg = crate::games::registry();
        let table = reg.fields(g);
        for e in &doc.tm.entries {
            let id = rule_of_fields(e, table, g.locks_loc_keys());
            if !id.is_empty() {
                let slot = fields.entry(id).or_default();
                slot.0 += 1;
                if slot.1.is_empty() {
                    slot.1 = e.source.clone();
                }
            }
        }
        for (id, (lines, example)) in fields {
            let loc = id == format!("fields:{}", exclusions::LOC_KEYS);
            rows.push(ActiveRow {
                id: id.clone(),
                origin: if loc {
                    RowOrigin::Game
                } else {
                    RowOrigin::Fields
                },
                scope: self.i18n.t("rules-scope-field"),
                game: game.clone(),
                lines,
                example,
                checkable: false,
                on: true,
                comment: if loc {
                    self.i18n.t("rules-loc-keys")
                } else {
                    self.i18n.t("rules-fields-comment")
                },
                learned: None,
            });
        }
        (rows, counts)
    }

    fn rules_active_tab(&mut self, ui: &mut egui::Ui) {
        if self.active_doc().is_none_or(|d| d.all.is_some()) {
            ui.label(egui::RichText::new(self.i18n.t("rules-no-doc")).weak());
            return;
        }
        let (rows, counts) = self.rules_rows();
        let n_rules = rows.iter().filter(|r| r.lines > 0).count();
        ui.label(self.i18n.t_args(
            "rules-summary",
            &[
                ("lines", &counts.locked.to_string()),
                ("rules", &n_rules.to_string()),
                ("hand", &counts.hand.to_string()),
            ],
        ));
        ui.separator();
        let mut toggle: Option<(String, bool)> = None;
        let avail = ui.available_height() - 110.0;
        egui::ScrollArea::both()
            .id_salt("ximod-rules-active")
            .max_height(avail.max(120.0))
            .show(ui, |ui| {
                egui::Grid::new("ximod-rules-active-grid")
                    .striped(true)
                    .num_columns(7)
                    .show(ui, |ui| {
                        for k in [
                            "rules-col-active",
                            "rules-col-rule",
                            "rules-col-origin",
                            "rules-col-scope",
                            "rules-col-game",
                            "rules-col-lines",
                            "rules-col-example",
                        ] {
                            ui.strong(self.i18n.t(k));
                        }
                        ui.end_row();
                        for r in &rows {
                            let mut on = r.on;
                            let resp =
                                ui.add_enabled(r.checkable, egui::Checkbox::without_text(&mut on));
                            if resp.changed() {
                                toggle = Some((r.id.clone(), on));
                            }
                            let sel = self.rules_win.selected.as_deref() == Some(r.id.as_str());
                            if ui.selectable_label(sel, &r.id).clicked() {
                                self.rules_win.selected = Some(r.id.clone());
                            }
                            ui.label(self.i18n.t(r.origin.i18n_key()));
                            ui.label(&r.scope);
                            ui.label(&r.game);
                            ui.label(crate::ui::export_dialog::fmt_count(&self.i18n, r.lines));
                            ui.label(egui::RichText::new(ellipsize(&r.example, 48)).monospace());
                            ui.end_row();
                        }
                    });
            });
        if let Some((id, on)) = toggle {
            self.rules_toggle(&id, on);
            return;
        }
        ui.separator();
        // The selected rule (4) and the actions (5).
        let sel = self
            .rules_win
            .selected
            .clone()
            .and_then(|id| rows.iter().find(|r| r.id == id).cloned());
        match &sel {
            Some(r) => {
                let text = match &r.learned {
                    Some(l) => self.i18n.t_args(
                        "rules-selected-learned",
                        &[
                            ("rule", &r.id),
                            ("date", &l.date),
                            ("edid", &l.from),
                            ("scope", &self.i18n.t(r.origin.i18n_key())),
                        ],
                    ),
                    None => self.i18n.t_args(
                        "rules-selected",
                        &[("rule", &r.id), ("comment", &r.comment)],
                    ),
                };
                ui.label(text);
            }
            None => {
                ui.label(egui::RichText::new(self.i18n.t("rules-select-hint")).weak());
            }
        }
        let mut action = 0u8;
        ui.horizontal(|ui| {
            let has = sel.is_some();
            if ui
                .add_enabled(has, egui::Button::new(self.i18n.t("rules-see-lines")))
                .clicked()
            {
                action = 1;
            }
            if ui
                .add_enabled(
                    sel.as_ref()
                        .is_some_and(|r| r.checkable && r.on && r.lines > 0),
                    egui::Button::new(self.i18n.t("rules-unlock")),
                )
                .on_hover_text(self.i18n.t("rules-unlock-tip"))
                .clicked()
            {
                action = 2;
            }
            let learned = sel.as_ref().is_some_and(|r| {
                matches!(r.origin, RowOrigin::LearnedLocal | RowOrigin::LearnedGlobal)
            });
            let label = if sel
                .as_ref()
                .is_some_and(|r| r.origin == RowOrigin::LearnedGlobal)
            {
                self.i18n.t("rules-make-local")
            } else {
                self.i18n.t("rules-make-global")
            };
            if ui.add_enabled(learned, egui::Button::new(label)).clicked() {
                action = 3;
            }
            if ui.button(self.i18n.t("rules-reapply")).clicked() {
                action = 4;
            }
        });
        let id = sel.map(|r| r.id).unwrap_or_default();
        match action {
            1 => self.rules_see_lines(&id),
            2 => self.rules_toggle(&id, false),
            3 => self.rules_toggle_place(&id),
            4 => {
                let idx = self.active;
                self.rules_reapply(idx, OpKind::RulesApply, None);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------ Éditeur

    fn rules_editor_tab(&mut self, ui: &mut egui::Ui) {
        if self.rules_win.file.is_none() {
            self.rules_editor_load();
        }
        let dirs = Dirs::installed();
        let (path, origin) = match self.rules_win.source {
            Some(Source::User) => (
                dirs.user_path().unwrap_or_default(),
                self.i18n.t("rules-file-user"),
            ),
            _ => (dirs.shipped_path(), self.i18n.t("rules-file-shipped")),
        };
        ui.label(
            egui::RichText::new(self.i18n.t_args(
                "rules-file",
                &[("path", exclusions::FILE), ("origin", &origin)],
            ))
            .weak()
            .small(),
        )
        .on_hover_text(path.display().to_string());
        let mut touched = false;
        let height = ui.available_height();
        let mut moved: Option<(usize, i32)> = None;
        let mut add: Option<u8> = None;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(300.0, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(300.0);
                    ui.strong(self.i18n.t("rules-list"));
                    let file = self.rules_win.file.as_mut().expect("loaded");
                    egui::ScrollArea::vertical()
                        .id_salt("ximod-rules-list")
                        .max_height(height - 60.0)
                        .show(ui, |ui| {
                            for (k, r) in file.rules.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut r.enabled, "").changed() {
                                        touched = true;
                                    }
                                    let label =
                                        format!("{}  ·  {}", r.id, self.i18n.t(r.kind.i18n_key()));
                                    if ui
                                        .selectable_label(k == self.rules_win.sel, label)
                                        .clicked()
                                    {
                                        self.rules_win.sel = k;
                                        self.rules_win.trial = None;
                                    }
                                });
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.button(self.i18n.t("rules-add")).clicked() {
                            add = Some(0);
                        }
                        if ui.button(self.i18n.t("rules-duplicate")).clicked() {
                            add = Some(1);
                        }
                        if ui.button(self.i18n.t("rules-remove")).clicked() {
                            add = Some(2);
                        }
                        if ui.small_button(crate::icons::CARET_UP).clicked() {
                            moved = Some((self.rules_win.sel, -1));
                        }
                        if ui.small_button(crate::icons::CARET_DOWN).clicked() {
                            moved = Some((self.rules_win.sel, 1));
                        }
                    });
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                touched |= self.rules_editor_form(ui);
            });
        });
        let file = self.rules_win.file.as_mut().expect("loaded");
        if let Some((k, d)) = moved {
            let j = k as i64 + d as i64;
            if j >= 0 && (j as usize) < file.rules.len() {
                file.rules.swap(k, j as usize);
                self.rules_win.sel = j as usize;
                touched = true;
            }
        }
        match add {
            Some(0) => {
                let taken: Vec<&Rule> = file.rules.iter().collect();
                let id = exclusions::free_id("user", &taken).replacen("learned-", "", 1);
                file.rules
                    .push(Rule::new(&id, Kind::Regex, "^$", vec![Scope::Text]));
                self.rules_win.sel = file.rules.len() - 1;
                touched = true;
            }
            Some(1) => {
                if let Some(r) = file.rules.get(self.rules_win.sel).cloned() {
                    let mut c = r;
                    let mut id = format!("{}-2", c.id);
                    while file.find(&id).is_some() {
                        id.push('+');
                    }
                    c.id = id;
                    c.origin = Origin::User;
                    file.rules.insert(self.rules_win.sel + 1, c);
                    self.rules_win.sel += 1;
                    touched = true;
                }
            }
            Some(2) if self.rules_win.sel < file.rules.len() => {
                {
                    // A shipped rule is turned off, not removed, so that
                    // the restoration finds it (R-B18-06).
                    if file.rules[self.rules_win.sel].origin == Origin::Shipped {
                        file.rules[self.rules_win.sel].enabled = false;
                    } else {
                        file.rules.remove(self.rules_win.sel);
                        self.rules_win.sel = self.rules_win.sel.saturating_sub(1);
                    }
                    touched = true;
                }
            }
            _ => {}
        }
        if touched {
            self.rules_win.dirty = true;
            self.rules_win.trial = None;
            self.rules_win.message = None;
            self.rules_win.error = None;
        }
    }

    /// The form of the selected rule, its trial on the active document
    /// and the functions. Returns whether something changed.
    fn rules_editor_form(&mut self, ui: &mut egui::Ui) -> bool {
        let mut touched = false;
        let sel = self.rules_win.sel;
        let i18n = &self.i18n;
        let Some(file) = self.rules_win.file.as_mut() else {
            return false;
        };
        let Some(r) = file.rules.get_mut(sel) else {
            ui.label(egui::RichText::new(i18n.t("rules-none")).weak());
            return false;
        };
        egui::Grid::new("ximod-rules-form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(i18n.t("rules-field-id"));
                touched |= ui
                    .add(egui::TextEdit::singleline(&mut r.id).desired_width(260.0))
                    .changed();
                ui.end_row();
                ui.label(i18n.t("rules-field-kind"));
                egui::ComboBox::from_id_salt("ximod-rules-kind")
                    .selected_text(i18n.t(r.kind.i18n_key()))
                    .show_ui(ui, |ui| {
                        for k in Kind::ALL {
                            touched |= ui
                                .selectable_value(&mut r.kind, k, i18n.t(k.i18n_key()))
                                .changed();
                        }
                    });
                ui.end_row();
                ui.label(i18n.t("rules-field-pattern"));
                touched |= ui
                    .add(
                        egui::TextEdit::singleline(&mut r.pattern)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(420.0),
                    )
                    .on_hover_text(i18n.t(match r.kind {
                        Kind::Regex => "rules-pattern-regex-tip",
                        Kind::Field => "rules-pattern-field-tip",
                        Kind::Edid => "rules-pattern-edid-tip",
                        Kind::Function => "rules-pattern-function-tip",
                        Kind::Key => "rules-pattern-key-tip",
                    }))
                    .changed();
                ui.end_row();
                if r.kind == Kind::Regex {
                    ui.label(i18n.t("rules-field-also"));
                    let mut also = r.also.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut also)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(420.0),
                        )
                        .changed()
                    {
                        r.also = (!also.is_empty()).then_some(also);
                        touched = true;
                    }
                    ui.end_row();
                    ui.label(i18n.t("rules-field-scope"));
                    ui.horizontal(|ui| {
                        for s in [Scope::Text, Scope::Script] {
                            let mut on = r.scope.contains(&s);
                            if ui.checkbox(&mut on, i18n.t(s.i18n_key())).changed() {
                                r.scope.retain(|x| *x != s);
                                if on {
                                    r.scope.push(s);
                                }
                                touched = true;
                            }
                        }
                    });
                    ui.end_row();
                }
                if r.kind == Kind::Key {
                    ui.label(i18n.t("rules-field-files"));
                    let mut files = r.files.join(", ");
                    if ui
                        .add(egui::TextEdit::singleline(&mut files).desired_width(420.0))
                        .changed()
                    {
                        r.files = files
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        touched = true;
                    }
                    ui.end_row();
                }
                ui.label(i18n.t("rules-field-game"));
                touched |= ui
                    .add(egui::TextEdit::singleline(&mut r.game).desired_width(160.0))
                    .on_hover_text(i18n.t("rules-field-game-tip"))
                    .changed();
                ui.end_row();
                ui.label(i18n.t("rules-field-comment"));
                touched |= ui
                    .add(egui::TextEdit::singleline(&mut r.comment).desired_width(420.0))
                    .changed();
                ui.end_row();
                ui.label(i18n.t("rules-col-origin"));
                ui.label(
                    egui::RichText::new(i18n.t(match r.origin {
                        Origin::Shipped => "rules-origin-shipped",
                        Origin::Game => "rules-origin-game",
                        Origin::Learned => "rules-origin-learned-global",
                        Origin::User => "rules-origin-user",
                    }))
                    .weak(),
                );
                ui.end_row();
            });
        // The trial on the active document (4).
        let key = format!("{r:?}");
        let rule = r.clone();
        let fresh = self
            .rules_win
            .trial
            .as_ref()
            .is_some_and(|(k, _, _)| *k == key);
        if !fresh {
            let trial = self.rules_trial(&rule);
            self.rules_win.trial = trial.map(|(n, t)| (key.clone(), n, t));
        }
        ui.add_space(6.0);
        match &self.rules_win.trial {
            Some((_, n, first)) => {
                ui.label(self.i18n.t_args(
                    "rules-try",
                    &[("n", &n.to_string()), ("text", &ellipsize(first, 70))],
                ));
            }
            None => {
                if let Err(e) = exclusions::validate_rule(&rule) {
                    ui.colored_label(ui.visuals().error_fg_color, self.rules_invalid(&e));
                }
            }
        }
        ui.add_space(6.0);
        ui.strong(self.i18n.t("rules-functions"));
        let tip = self.i18n.t("rules-functions-tip");
        egui::ScrollArea::vertical()
            .id_salt("ximod-rules-functions")
            .max_height(130.0)
            .show(ui, |ui| {
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.rules_win.functions)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(420.0),
                    )
                    .on_hover_text(tip)
                    .changed()
                {
                    touched = true;
                }
            });
        touched
    }

    /// How many lines of the active document a rule alone would lock, and
    /// the first (the lines nobody decided, and those already locked).
    fn rules_trial(&self, r: &Rule) -> Option<(usize, String)> {
        exclusions::validate_rule(r).ok()?;
        let doc = self.active_doc()?;
        let mut alone = RulesFile::default();
        let mut r = r.clone();
        r.enabled = true;
        r.game = "*".into();
        alone.rules.push(r);
        alone.deduce.edid_scopes.clear();
        let c = Compiled::new(&alone, &[], "*");
        let d = exclusions::Deduced::default();
        let off = HashSet::new();
        let ctx = exclusions::Ctx {
            game: "*",
            path: &doc.path.to_string_lossy(),
            deduced: &d,
            off: &off,
            facts: None,
        };
        let mut n = 0;
        let mut first = String::new();
        for e in &doc.tm.entries {
            if !(exclusions::is_subject(e) || exclusions::lock_rule(e).is_some()) {
                continue;
            }
            if c.lock_of(e, &ctx).is_some() {
                n += 1;
                if first.is_empty() {
                    first = e.source.clone();
                }
            }
        }
        Some((n, first))
    }

    /// Save the user's copy, then apply the rules to the open documents.
    fn rules_save(&mut self) -> bool {
        let Some(mut file) = self.rules_win.file.clone() else {
            return false;
        };
        file.functions = self
            .rules_win
            .functions
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match Dirs::installed().save(&file) {
            Ok(p) => {
                self.rules_win.file = Some(file);
                self.rules_win.source = Some(Source::User);
                self.rules_win.dirty = false;
                self.rules_win.revision += 1;
                self.rules_win.error = None;
                self.rules_win.message = Some(self.i18n.t_arg(
                    "rules-saved",
                    "path",
                    &p.display().to_string(),
                ));
                for k in 0..self.docs.len() {
                    if self.docs[k].all.is_none() {
                        self.rules_reapply(k, OpKind::RulesApply, None);
                    }
                }
                true
            }
            Err(e) => {
                self.rules_win.error = Some(self.rules_invalid(&e));
                false
            }
        }
    }

    fn rules_bottom(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        if let Some(m) = &self.rules_migrated {
            ui.label(egui::RichText::new(m).small());
        }
        if self.rules_win.tab == RulesTab::Editor {
            if let Some(e) = &self.rules_win.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            if let Some(m) = &self.rules_win.message {
                ui.label(egui::RichText::new(m).small());
            }
            if self.rules_win.confirm_close {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().warn_fg_color, self.i18n.t("rules-unsaved"));
                    if ui.button(self.i18n.t("btn-save")).clicked() && self.rules_save() {
                        self.rules_win.confirm_close = false;
                        self.rules_win.open = false;
                    }
                    if ui.button(self.i18n.t("matrices-discard")).clicked() {
                        self.rules_win.confirm_close = false;
                        self.rules_win.open = false;
                        self.rules_win.file = None;
                    }
                    if ui.button(self.i18n.t("btn-cancel")).clicked() {
                        self.rules_win.confirm_close = false;
                    }
                });
            }
        }
        let mut action = 0u8;
        ui.horizontal(|ui| {
            if self.rules_win.tab == RulesTab::Editor {
                if ui
                    .add_enabled(
                        self.rules_win.dirty,
                        egui::Button::new(egui::RichText::new(self.i18n.t("btn-save")).strong()),
                    )
                    .clicked()
                {
                    action = 1;
                }
                if ui
                    .add_enabled(
                        self.rules_win.source == Some(Source::User),
                        egui::Button::new(self.i18n.t("rules-restore")),
                    )
                    .clicked()
                {
                    action = 2;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.t("btn-close")).clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new(self.i18n.t("matrices-no-restart"))
                        .weak()
                        .small(),
                );
            });
        });
        match action {
            1 => {
                self.rules_save();
            }
            2 => {
                let _ = Dirs::installed().restore();
                self.rules_editor_load();
                self.rules_win.revision += 1;
                self.rules_win.message = Some(self.i18n.t("rules-restored"));
                for k in 0..self.docs.len() {
                    if self.docs[k].all.is_none() {
                        self.rules_reapply(k, OpKind::RulesApply, None);
                    }
                }
            }
            _ => {}
        }
        close
    }

    // --------------------------------------------------- Suggestions

    fn rules_suggest_tab(&mut self, ui: &mut egui::Ui) {
        if self.active_doc().is_none_or(|d| d.all.is_some()) {
            ui.label(egui::RichText::new(self.i18n.t("rules-no-doc")).weak());
            return;
        }
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("rules-suggest-intro"));
            if ui.button(self.i18n.t("rules-suggest-run")).clicked() {
                self.rules_win.suggestions = Some(self.rules_suggestions());
            }
        });
        ui.separator();
        let Some(list) = self.rules_win.suggestions.clone() else {
            return;
        };
        if list.is_empty() {
            ui.label(egui::RichText::new(self.i18n.t("rules-suggest-none")).weak());
            return;
        }
        let mut take: Option<Rule> = None;
        egui::ScrollArea::vertical()
            .id_salt("ximod-rules-suggest")
            .show(ui, |ui| {
                for s in &list {
                    ui.horizontal(|ui| {
                        let (text, rule) = match s {
                            Suggestion::FieldBase(sig, n) => {
                                let mut r = Rule::new(
                                    &format!("field-{}", sig.replace(':', "-").to_lowercase()),
                                    Kind::Field,
                                    sig,
                                    vec![Scope::Field],
                                );
                                r.game = self
                                    .active_doc()
                                    .map(Self::rules_game)
                                    .unwrap_or_else(|| "*".into());
                                (
                                    self.i18n.t_args(
                                        "suggest-field-base",
                                        &[("sig", sig), ("n", &n.to_string())],
                                    ),
                                    r,
                                )
                            }
                            Suggestion::Group(r, n) => (
                                self.i18n.t_args(
                                    "suggest-group-hand",
                                    &[("n", &n.to_string()), ("what", &r.pattern)],
                                ),
                                r.clone(),
                            ),
                        };
                        if ui.button(self.i18n.t("rules-suggest-add")).clicked() {
                            take = Some(rule);
                        }
                        ui.label(text);
                    });
                }
            });
        if let Some(r) = take {
            if self.rules_win.file.is_none() {
                self.rules_editor_load();
            }
            if let Some(file) = self.rules_win.file.as_mut() {
                let mut r = r;
                while file.find(&r.id).is_some() {
                    r.id.push('+');
                }
                file.rules.push(r);
                self.rules_win.sel = file.rules.len() - 1;
                self.rules_win.dirty = true;
                self.rules_win.trial = None;
            }
            self.rules_win.tab = RulesTab::Editor;
        }
    }

    fn rules_suggestions(&mut self) -> Vec<Suggestion> {
        let Some(doc) = self.active_doc() else {
            return Vec::new();
        };
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        let stack = self.resolve_stack(&path, &settings);
        let doc = self.active_doc().expect("checked");
        let mut out = Vec::new();
        if let Some(stack) = stack {
            out.extend(exclusions::suggest_fields(
                &doc.tm.entries,
                stack
                    .levels
                    .iter()
                    .filter(|l| l.role == crate::dicts::Role::Game)
                    .map(|l| &*l.db),
            ));
        }
        let file = rules_file();
        let taken: Vec<&Rule> = file.rules.iter().chain(&doc.tm.meta.rules).collect();
        out.extend(exclusions::suggest_groups(&doc.tm.entries, &taken));
        out
    }
}

/// The row of the fields file that holds a line: `fields:REC:FLD`, or
/// `fields:loc-keys` for the keys an engine locks; empty otherwise.
fn rule_of_fields(
    e: &crate::tm::TmEntry,
    table: &ximod_games::fields::FieldTable,
    loc: bool,
) -> String {
    if e.unused
        || !e.prov.is_empty()
        || crate::status::derive(e) != crate::status::Status::Ignored
        || exclusions::line_kind(e) != exclusions::LineKind::Record
    {
        return String::new();
    }
    let none = |_: &str| None;
    let ctx = ximod_games::fields::FieldCtx {
        edid: &e.edid,
        value: &e.source,
        other: &none,
    };
    if table.mode(&e.rec_sig, &e.field_sig, &ctx) != ximod_games::fields::FieldMode::Locked {
        return String::new();
    }
    // The keys the engine locks (Oblivion Remastered, `locksLocKeys`).
    if loc && e.source.starts_with("LOC_") {
        return format!("fields:{}", exclusions::LOC_KEYS);
    }
    format!("fields:{}:{}", e.rec_sig, e.field_sig)
}

/// The choice the dialog opens on: the exact text when other lines share
/// it, else the prefix, else the field when it is not displayed.
fn default_choice(p: &Proposal) -> Choice {
    if p.same_text >= 1 {
        Choice::TextLocal
    } else if p.prefix.is_some() {
        Choice::PrefixLocal
    } else if !p.displayed && p.same_field >= 1 {
        Choice::FieldLocal
    } else {
        Choice::None
    }
}

/// A text cut to `n` characters.
fn ellipsize(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= n {
        s
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}
