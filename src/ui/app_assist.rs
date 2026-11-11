//! Assistants hors ligne (B-15): the window Glossaire, motifs et
//! dérivations (Outils › Données), the dialog Appliquer les assistants
//! (Dictionnaires), the small dialog Ajouter au glossaire (Ctrl+Alt+G in
//! the edit window) and the glossary of the active document, shared by F9
//! and the Vocabulaire tab.

use crate::ui::theme::ComboField as _;
use std::sync::Arc;

use eframe::egui;

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::assist::glossary::{self, Dirs, Forms, GlossaryFile, Invalid, Origin};
use crate::assist::{self, Assistant, Class, Gender, Pos, Refusal, Term};
use crate::replace::Scope;
use crate::undo::{EntryState, OpKind, op_from_before};

/// Width of the list of groups; the terms take the rest.
const GROUPS_W: f32 = 230.0;

/// The window's state.
#[derive(Default)]
pub(crate) struct AssistWindow {
    pub open: bool,
    pub needs_place: bool,
    /// The pair edited (`ENG-FRA`) and the game of its second glossary.
    pub pair: String,
    pub game: String,
    /// The game's glossary is edited instead of the pair's.
    pub game_file: bool,
    pub file: Option<GlossaryFile>,
    pub origin: Option<Origin>,
    pub group: usize,
    pub term: Option<usize>,
    pub filter: String,
    pub dirty: bool,
    pub error: Option<String>,
    pub message: Option<String>,
    pub trial: String,
    pub confirm_close: bool,
    /// Glossaire or Motifs (B-16).
    pub tab: AssistTab,
    /// The patterns file edited (B-16), whether it is the user's copy, the
    /// pattern chosen, the essay.
    pub pfile: Option<assist::PatternsFile>,
    pub puser: bool,
    pub psel: Option<usize>,
    pub pdirty: bool,
    pub ptrial: String,
    pub perror: Option<String>,
    pub pmessage: Option<String>,
    /// The derivations of the game edited (B-21): the file, whether it is
    /// the user's copy, the rule chosen and the buffers of its lists, the
    /// essay on the active document (its key, its lines).
    pub dfile: Option<assist::derive::DeriveFile>,
    pub duser: bool,
    pub dsel: Option<usize>,
    pub dbuf: Option<(usize, String, String)>,
    pub ddirty: bool,
    pub dedit: u64,
    pub derror: Option<String>,
    pub dmessage: Option<String>,
    pub dtry: Option<(TryKey, Vec<String>)>,
}

/// What the essay of the Dérivations tab was computed for: the document's
/// serial and revision, the edits of the file, the glossaries' generation.
pub(crate) type TryKey = (u64, u64, u64, u64);

/// The tabs of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AssistTab {
    #[default]
    Glossary,
    Patterns,
    Derive,
}

/// What a preview was computed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanKey {
    serial: u64,
    revision: u64,
    scope: Scope,
    lines: usize,
    opts: assist::Options,
    generation: u64,
}

/// Appliquer les assistants (figure B.10).
pub(crate) struct AssistDialog {
    pub open: bool,
    pub scope: Scope,
    pub opts: assist::Options,
    pub plan: Option<assist::Plan>,
    pub key: Option<PlanKey>,
}

impl Default for AssistDialog {
    fn default() -> Self {
        AssistDialog {
            open: false,
            scope: Scope::Document,
            opts: assist::Options::default(),
            plan: None,
            key: None,
        }
    }
}

/// Ajouter au glossaire (Ctrl+Alt+G).
pub(crate) struct AddTerm {
    pub groups: Vec<String>,
    /// The group chosen; `None`: a new one, named in `new_group`.
    pub group: Option<usize>,
    pub new_group: String,
    pub term: Term,
    pub error: Option<String>,
    pub needs_focus: bool,
}

/// The four forms, in their order on screen.
const FORMS: [Gender; 4] = [Gender::M, Gender::F, Gender::Mp, Gender::Fp];

impl XimodApp {
    // ------------------------------------------------------------------
    // The glossary of the active document
    // ------------------------------------------------------------------

    /// The pair (`ENG-FRA`) and the game (`SkyrimSE`) of the active
    /// document; `None` without a document or its two languages.
    pub(crate) fn assist_pair(&self) -> Option<(String, String)> {
        let d = self.active_doc()?;
        if d.settings.source.trim().is_empty() || d.settings.target.trim().is_empty() {
            return None;
        }
        Some((d.settings.pair_suffix(), d.settings.game().id().to_string()))
    }

    /// The glossary and composition of the active document, read once per
    /// pair, game and change of the files.
    pub(crate) fn assistant(&mut self) -> Option<Arc<Assistant>> {
        let (pair, game) = self.assist_pair()?;
        let key = (pair.clone(), game.clone(), self.assist_gen);
        if let Some((k, a)) = &self.assist_cache
            && *k == key
        {
            return Some(a.clone());
        }
        let a = Arc::new(Assistant::load(&pair, &game));
        self.assist_cache = Some((key, a.clone()));
        Some(a)
    }

    /// The words of an invalid glossary.
    pub(crate) fn glossary_invalid(&self, e: &Invalid) -> String {
        match e {
            Invalid::Json(m) => self.i18n.t_arg("glossary-err-json", "error", m),
            Invalid::Format(n) => self.i18n.t_arg("glossary-err-format", "n", &n.to_string()),
            Invalid::GroupName(n) => self.i18n.t_arg("glossary-err-group", "n", &n.to_string()),
            Invalid::DuplicateGroup(n) => self.i18n.t_arg("glossary-err-group-dup", "name", n),
            Invalid::EmptySrc(_) => self.i18n.t("glossary-err-empty"),
            Invalid::Duplicate { src, group } => self
                .i18n
                .t_args("glossary-err-dup", &[("src", src), ("group", group)]),
            Invalid::Forms { src, group } => self
                .i18n
                .t_args("glossary-err-forms", &[("src", src), ("group", group)]),
        }
    }

    /// « English → Français » of a pair.
    fn pair_words(&self, pair: &str) -> (String, String) {
        let (s, d) = pair.split_once('-').unwrap_or((pair, ""));
        (
            self.i18n.display_name(&s.to_ascii_lowercase()),
            self.i18n.display_name(&d.to_ascii_lowercase()),
        )
    }

    /// The path followed by a translation, for the essays and the window.
    pub(crate) fn assist_path(&self, t: &assist::Translation) -> String {
        if t.composed {
            let classes: Vec<String> = t
                .classes
                .iter()
                .map(|c| self.i18n.t(c.i18n_key()))
                .collect();
            let gender = t
                .gender
                .map(|g| self.i18n.t(g.i18n_key()))
                .unwrap_or_else(|| self.i18n.t("glossary-gender-none"));
            self.i18n.t_args(
                "glossary-path-compose",
                &[("classes", &classes.join(", ")), ("gender", &gender)],
            )
        } else if t.not_composed == Some(assist::NotComposed::NoNoun) {
            self.i18n.t("patterns-no-noun")
        } else {
            self.i18n.t("glossary-path-glossary")
        }
    }

    /// The result of an essay: « … » → « … » (path), or the words missed.
    pub(crate) fn assist_trial(
        &self,
        a: &Assistant,
        text: &str,
        bases: &dyn assist::Bases,
    ) -> String {
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        match a.run(text, true, true, bases) {
            Ok(o) => self.i18n.t_args(
                "glossary-try-result",
                &[
                    ("src", text),
                    ("dst", &o.text),
                    ("path", &self.assist_outcome_path(&o, a)),
                ],
            ),
            Err(Refusal::Partial(w)) => self.i18n.t_args(
                "glossary-try-partial",
                &[
                    ("src", text),
                    ("n", &w.len().to_string()),
                    ("words", &w.join(", ")),
                ],
            ),
            Err(Refusal::NoTerm) => self.i18n.t("glossary-try-none"),
        }
    }

    // ------------------------------------------------------------------
    // The window
    // ------------------------------------------------------------------

    /// Outils › Données › Glossaire, motifs et dérivations…: the pair of
    /// the active document, or the one edited last.
    pub(crate) fn open_assist_window(&mut self) {
        if self.assist_win.open {
            self.assist_win.needs_place = true;
            return;
        }
        if let Some((pair, game)) = self.assist_pair() {
            if pair != self.assist_win.pair || game != self.assist_win.game {
                self.assist_win.file = None;
            }
            self.assist_win.pair = pair;
            self.assist_win.game = game;
        }
        if self.assist_win.pair.is_empty() {
            self.assist_win.pair = "ENG-FRA".into();
            self.assist_win.game = crate::games::Game::SkyrimSE.id().to_string();
        }
        self.assist_win.open = true;
        self.assist_win.needs_place = true;
        if self.assist_win.file.is_none() {
            self.assist_load();
        }
    }

    fn assist_file_name(&self) -> String {
        let w = &self.assist_win;
        glossary::file_name(&w.pair, w.game_file.then_some(w.game.as_str()))
    }

    /// Read the edited file again (the user's copy or the shipped one).
    fn assist_load(&mut self) {
        let name = self.assist_file_name();
        let (mut f, origin, err) = Dirs::installed().load(&name);
        if origin == Origin::None {
            let (s, d) = self.assist_win.pair.split_once('-').unwrap_or(("", ""));
            f.source = s.to_ascii_lowercase();
            f.target = d.to_ascii_lowercase();
        }
        let error = err.map(|e| self.glossary_invalid(&e));
        let w = &mut self.assist_win;
        w.file = Some(f);
        w.origin = Some(origin);
        w.group = 0;
        w.term = None;
        w.dirty = false;
        w.confirm_close = false;
        w.message = None;
        w.error = error;
    }

    /// The pairs offered: those of the shipped and user's files, and the
    /// active document's.
    fn assist_pairs(&self) -> Vec<String> {
        let dirs = Dirs::installed();
        let mut out: Vec<String> = Vec::new();
        for dir in std::iter::once(Some(dirs.shipped.clone())).chain(std::iter::once(dirs.user)) {
            let Some(dir) = dir else { continue };
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(stem) = n
                    .strip_prefix("glossary_")
                    .and_then(|s| s.strip_suffix(".json"))
                {
                    let pair = stem.rsplit('_').next().unwrap_or(stem).to_string();
                    if pair.len() == 7 && !out.contains(&pair) {
                        out.push(pair);
                    }
                }
            }
        }
        for p in [
            self.assist_pair().map(|p| p.0),
            Some(self.assist_win.pair.clone()),
        ]
        .into_iter()
        .flatten()
        {
            if !p.is_empty() && !out.contains(&p) {
                out.push(p);
            }
        }
        out.sort();
        out
    }

    /// Save the edited file; the glossary is taken into account at once.
    fn assist_save(&mut self) -> bool {
        let name = self.assist_file_name();
        let Some(f) = self.assist_win.file.clone() else {
            return false;
        };
        match Dirs::installed().save(&name, &f) {
            Ok(p) => {
                self.assist_win.dirty = false;
                self.assist_win.origin = Some(Origin::User);
                self.assist_win.error = None;
                self.assist_win.message = Some(self.i18n.t_arg(
                    "glossary-saved",
                    "path",
                    &p.display().to_string(),
                ));
                self.assist_changed();
                true
            }
            Err(e) => {
                self.assist_win.error = Some(self.glossary_invalid(&e));
                false
            }
        }
    }

    /// A glossary file changed: F9, the Vocabulaire tab and the dialog
    /// read it again.
    pub(crate) fn assist_changed(&mut self) {
        self.assist_gen += 1;
        self.assist_cache = None;
        self.assist_dlg.key = None;
        self.vocab.key = None;
    }

    pub(crate) fn assist_window(&mut self, ctx: &egui::Context) {
        if !self.assist_win.open {
            return;
        }
        if self.assist_win.file.is_none() {
            self.assist_load();
        }
        let id = egui::ViewportId::from_hash_of("ximod-assist");
        let [sx, sy, sw, sh] = self.config.assist_win;
        let just_opened = self.assist_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(900.0), sh.max(520.0)],
            )
        });
        let (src, dst) = self.pair_words(&self.assist_win.pair);
        let title = self
            .i18n
            .t_args("assist-title", &[("src", &src), ("dst", &dst)]);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([900.0, 520.0]);
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
            self.assist_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::top("assist_tabs").show(vctx, |ui| {
                ui.horizontal(|ui| {
                    let game = crate::games::Game::from_id(&self.assist_win.game);
                    let game = match game {
                        crate::games::Game::Unknown => self.assist_win.game.clone(),
                        g => g.display_name().to_string(),
                    };
                    for (tab, label) in [
                        (AssistTab::Glossary, self.i18n.t("assist-tab-glossary")),
                        (AssistTab::Patterns, self.i18n.t("assist-tab-patterns")),
                        (
                            AssistTab::Derive,
                            self.i18n.t_arg("assist-tab-derive", "game", &game),
                        ),
                    ] {
                        if ui
                            .selectable_label(self.assist_win.tab == tab, label)
                            .clicked()
                        {
                            self.assist_win.tab = tab;
                        }
                    }
                });
            });
            egui::TopBottomPanel::bottom("assist_bottom").show(vctx, |ui| {
                let closed = match self.assist_win.tab {
                    AssistTab::Glossary => self.assist_bottom(ui),
                    AssistTab::Patterns => self.patterns_bottom(ui),
                    AssistTab::Derive => self.derive_bottom(ui),
                };
                if closed {
                    close = true;
                }
            });
            egui::CentralPanel::default().show(vctx, |ui| match self.assist_win.tab {
                AssistTab::Glossary => self.assist_glossary_tab(ui),
                AssistTab::Patterns => self.patterns_tab(ui),
                AssistTab::Derive => self.derive_tab(ui),
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.assist_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            let w = &mut self.assist_win;
            if w.dirty || w.pdirty || w.ddirty {
                w.confirm_close = true;
                w.tab = if w.dirty {
                    AssistTab::Glossary
                } else if w.pdirty {
                    AssistTab::Patterns
                } else {
                    AssistTab::Derive
                };
            } else {
                self.assist_win.open = false;
                let _ = self.config.save();
            }
        }
    }

    fn assist_glossary_tab(&mut self, ui: &mut egui::Ui) {
        // The file: pair and game, path, counts.
        let pairs = self.assist_pairs();
        let mut reload = false;
        let dirs = Dirs::installed();
        let name = self.assist_file_name();
        let origin = self.assist_win.origin.unwrap_or(Origin::None);
        let path = match origin {
            Origin::User => dirs.user_path(&name).unwrap_or_default(),
            _ => dirs.shipped_path(&name),
        };
        let origin_w = self.i18n.t(match origin {
            Origin::User => "glossary-origin-user",
            Origin::Shipped => "glossary-origin-shipped",
            Origin::None => "glossary-origin-none",
        });
        let (groups_n, terms_n) = self
            .assist_win
            .file
            .as_ref()
            .map(|f| (f.groups.len(), f.term_count()))
            .unwrap_or((0, 0));
        let l_pair = self.i18n.t("glossary-pair");
        let l_pair_file = self.i18n.t("glossary-which-pair");
        let l_game_file = self
            .i18n
            .t_arg("glossary-which-game", "game", &self.assist_win.game);
        let dirty = self.assist_win.dirty;
        ui.horizontal(|ui| {
            ui.label(&l_pair);
            let cur = self.assist_win.pair.clone();
            egui::ComboBox::from_id_salt("ximod-assist-pair")
                .selected_text(&cur)
                .show_field(ui, |ui| {
                    for p in &pairs {
                        if ui.selectable_label(*p == cur, p).clicked() && *p != cur && !dirty {
                            self.assist_win.pair = p.clone();
                            reload = true;
                        }
                    }
                });
            ui.separator();
            let gf = self.assist_win.game_file;
            if ui
                .add_enabled(!dirty, egui::SelectableLabel::new(!gf, &l_pair_file))
                .clicked()
                && gf
            {
                self.assist_win.game_file = false;
                reload = true;
            }
            if ui
                .add_enabled(!dirty, egui::SelectableLabel::new(gf, &l_game_file))
                .clicked()
                && !gf
            {
                self.assist_win.game_file = true;
                reload = true;
            }
        });
        if reload {
            self.assist_load();
            return;
        }
        ui.label(
            egui::RichText::new(self.i18n.t_args(
                "glossary-file",
                &[
                    ("path", &format!("{}/{name} ({origin_w})", glossary::DIR)),
                    ("groups", &groups_n.to_string()),
                    ("terms", &terms_n.to_string()),
                ],
            ))
            .weak()
            .small(),
        )
        .on_hover_text(path.display().to_string());
        ui.add_space(4.0);
        let height = ui.available_height();
        let mut touched = false;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(GROUPS_W, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(GROUPS_W);
                    touched |= self.assist_groups(ui, height);
                },
            );
            ui.separator();
            ui.vertical(|ui| {
                touched |= self.assist_terms(ui);
            });
        });
        if touched {
            self.assist_win.dirty = true;
            self.assist_win.message = None;
        }
    }

    /// The list of groups, its name field and buttons.
    fn assist_groups(&mut self, ui: &mut egui::Ui, height: f32) -> bool {
        let mut touched = false;
        let l_add = self.i18n.t("glossary-group-add");
        let l_remove = self.i18n.t("glossary-group-remove");
        let l_name = self.i18n.t("glossary-group-name");
        let l_new = self.i18n.t("glossary-group-new");
        ui.strong(self.i18n.t("glossary-groups"));
        let w = &mut self.assist_win;
        let Some(f) = w.file.as_mut() else {
            return false;
        };
        egui::ScrollArea::vertical()
            .id_salt("ximod-assist-groups")
            .max_height((height - 110.0).max(80.0))
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    for (k, g) in f.groups.iter().enumerate() {
                        let label = format!("{} ({})", g.name, g.terms.len());
                        if ui.selectable_label(k == w.group, label).clicked() {
                            w.group = k;
                            w.term = None;
                        }
                    }
                });
            });
        if let Some(g) = f.groups.get_mut(w.group) {
            ui.horizontal(|ui| {
                ui.label(&l_name);
                if ui
                    .add(egui::TextEdit::singleline(&mut g.name).desired_width(GROUPS_W - 60.0))
                    .changed()
                {
                    touched = true;
                }
            });
        }
        ui.horizontal(|ui| {
            if ui.button(&l_add).clicked() {
                let mut name = l_new.clone();
                let mut n = 2;
                while f.groups.iter().any(|g| g.name.eq_ignore_ascii_case(&name)) {
                    name = format!("{l_new} {n}");
                    n += 1;
                }
                f.groups.push(glossary::Group {
                    name,
                    terms: Vec::new(),
                });
                w.group = f.groups.len() - 1;
                w.term = None;
                touched = true;
            }
            if ui
                .add_enabled(w.group < f.groups.len(), egui::Button::new(&l_remove))
                .clicked()
            {
                f.groups.remove(w.group);
                w.group = w.group.saturating_sub(1);
                w.term = None;
                touched = true;
            }
        });
        touched
    }

    /// The table of the terms of the group, the current term and the essay.
    fn assist_terms(&mut self, ui: &mut egui::Ui) -> bool {
        let mut touched = false;
        let cols = [
            self.i18n.t("glossary-col-src"),
            self.i18n.t("glossary-col-dst"),
            self.i18n.t("glossary-col-class"),
            self.i18n.t("glossary-col-whole"),
            self.i18n.t("glossary-col-pos"),
            self.i18n.t("glossary-col-case"),
            self.i18n.t("glossary-col-gender"),
        ];
        let class_l: Vec<(Class, String)> = Class::ALL
            .iter()
            .map(|c| (*c, self.i18n.t(c.i18n_key())))
            .collect();
        let pos_l: Vec<(Pos, String)> = Pos::ALL
            .iter()
            .map(|p| (*p, self.i18n.t(p.i18n_key())))
            .collect();
        let gender_l: Vec<(Option<Gender>, String)> =
            std::iter::once((None, self.i18n.t("glossary-gender-none")))
                .chain(
                    Gender::ALL
                        .iter()
                        .map(|g| (Some(*g), self.i18n.t(g.i18n_key()))),
                )
                .collect();
        let (yes, no) = (self.i18n.t("glossary-yes"), self.i18n.t("glossary-no"));
        let l_filter = self.i18n.t("glossary-filter-hint");
        let l_add = self.i18n.t("glossary-term-add");
        let l_remove = self.i18n.t("glossary-term-remove");
        let l_forms = self.i18n.t("glossary-forms");
        let l_try = self.i18n.t("glossary-try");
        let words = |v: &[(Class, String)], c: Class| {
            v.iter()
                .find(|(k, _)| *k == c)
                .map(|(_, s)| s.clone())
                .unwrap_or_default()
        };
        ui.add(
            egui::TextEdit::singleline(&mut self.assist_win.filter)
                .hint_text(&l_filter)
                .desired_width(360.0),
        );
        let filter = self.assist_win.filter.trim().to_lowercase();
        let avail = ui.available_height();
        let w = &mut self.assist_win;
        let Some(f) = w.file.as_mut() else {
            return false;
        };
        let Some(g) = f.groups.get_mut(w.group) else {
            return false;
        };
        egui::ScrollArea::both()
            .id_salt(("ximod-assist-terms", w.group))
            .max_height((avail - 170.0).max(120.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(("ximod-assist-grid", w.group))
                    .striped(true)
                    .num_columns(7)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for c in &cols {
                            ui.strong(c);
                        }
                        ui.end_row();
                        for (k, t) in g.terms.iter().enumerate() {
                            let shown = t.shown();
                            if !filter.is_empty()
                                && !t.src.to_lowercase().contains(&filter)
                                && !shown.to_lowercase().contains(&filter)
                            {
                                continue;
                            }
                            if ui.selectable_label(w.term == Some(k), &t.src).clicked() {
                                w.term = Some(k);
                            }
                            ui.label(&shown);
                            ui.label(words(&class_l, t.class));
                            ui.label(if t.whole { &yes } else { &no });
                            ui.label(
                                pos_l
                                    .iter()
                                    .find(|(p, _)| *p == t.pos)
                                    .map(|(_, s)| s.as_str())
                                    .unwrap_or(""),
                            );
                            ui.label(if t.case { &yes } else { &no });
                            ui.label(
                                gender_l
                                    .iter()
                                    .find(|(x, _)| *x == t.gender)
                                    .map(|(_, s)| s.as_str())
                                    .unwrap_or(""),
                            );
                            ui.end_row();
                        }
                    });
            });
        ui.horizontal(|ui| {
            if ui.button(&l_add).clicked() {
                g.terms.push(Term::default());
                w.term = Some(g.terms.len() - 1);
                touched = true;
            }
            if ui
                .add_enabled(w.term.is_some(), egui::Button::new(&l_remove))
                .clicked()
                && let Some(k) = w.term
                && k < g.terms.len()
            {
                g.terms.remove(k);
                w.term = None;
                touched = true;
            }
        });
        // The current term (4).
        if let Some(t) = w.term.and_then(|k| g.terms.get_mut(k)) {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&cols[0]).small());
                        touched |= ui
                            .add(egui::TextEdit::singleline(&mut t.src).desired_width(200.0))
                            .changed();
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&cols[1]).small());
                        touched |= ui
                            .add(egui::TextEdit::singleline(&mut t.dst).desired_width(200.0))
                            .changed();
                    });
                    touched |= ui.checkbox(&mut t.whole, &cols[3]).changed();
                    egui::ComboBox::from_id_salt("ximod-assist-class")
                        .selected_text(words(&class_l, t.class))
                        .show_field(ui, |ui| {
                            for (c, l) in &class_l {
                                touched |= ui.selectable_value(&mut t.class, *c, l).changed();
                            }
                        });
                    touched |= ui.checkbox(&mut t.case, &cols[5]).changed();
                    egui::ComboBox::from_id_salt("ximod-assist-gender")
                        .selected_text(
                            gender_l
                                .iter()
                                .find(|(x, _)| *x == t.gender)
                                .map(|(_, s)| s.clone())
                                .unwrap_or_default(),
                        )
                        .show_field(ui, |ui| {
                            for (x, l) in &gender_l {
                                touched |= ui.selectable_value(&mut t.gender, *x, l).changed();
                            }
                        });
                    egui::ComboBox::from_id_salt("ximod-assist-pos")
                        .selected_text(
                            pos_l
                                .iter()
                                .find(|(p, _)| *p == t.pos)
                                .map(|(_, s)| s.clone())
                                .unwrap_or_default(),
                        )
                        .show_field(ui, |ui| {
                            for (p, l) in &pos_l {
                                touched |= ui.selectable_value(&mut t.pos, *p, l).changed();
                            }
                        });
                });
                // Adjectives and colours: their four forms.
                if matches!(t.class, Class::Adj | Class::People | Class::Color)
                    || !t.forms.is_empty()
                {
                    ui.horizontal(|ui| {
                        ui.label(&l_forms);
                        for gd in FORMS {
                            let slot = t.forms.slot_mut(gd);
                            let mut s = slot.clone().unwrap_or_default();
                            if ui
                                .add(egui::TextEdit::singleline(&mut s).desired_width(110.0))
                                .changed()
                            {
                                *slot = (!s.trim().is_empty()).then_some(s);
                                touched = true;
                            }
                        }
                    });
                }
            });
        }
        // The essay (5): the file as edited, before saving.
        ui.horizontal(|ui| {
            ui.label(&l_try);
            ui.add(egui::TextEdit::singleline(&mut w.trial).desired_width(320.0));
        });
        let trial = w.trial.clone();
        if !trial.trim().is_empty() {
            let a = self.assist_edited();
            let bases = self.assist_bases();
            let shown = self.assist_trial(&a, &trial, &bases);
            ui.label(egui::RichText::new(shown).small());
        }
        // A problem of the group, shown at once.
        if let Some(e) = self
            .assist_win
            .file
            .as_ref()
            .and_then(|f| f.groups.get(self.assist_win.group))
            .and_then(glossary::group_problem)
        {
            let msg = self.glossary_invalid(&e);
            ui.colored_label(ui.visuals().error_fg_color, msg);
        }
        touched
    }

    /// The assistant of the file being edited (the other glossary of the
    /// pair as saved), for the essay.
    pub(crate) fn assist_edited(&self) -> Assistant {
        let w = &self.assist_win;
        let dirs = Dirs::installed();
        let edited = w.file.clone().unwrap_or_default();
        let other_name = glossary::file_name(&w.pair, (!w.game_file).then_some(w.game.as_str()));
        let (other, _, _) = dirs.load(&other_name);
        let files: Vec<(&GlossaryFile, bool)> = if w.game_file {
            vec![(&edited, true), (&other, false)]
        } else {
            vec![(&other, true), (&edited, false)]
        };
        let pfile = w.pfile.clone().or_else(|| {
            assist::patterns::load_file(&dirs.shipped, dirs.user.as_ref(), &w.pair).map(|f| f.0)
        });
        let comp = pfile.as_ref().and_then(|f| f.composition.clone());
        let mut a = Assistant::new(assist::Glossary::new(&files), comp);
        a.patterns = pfile.map(|f| Arc::new(assist::Patterns::new(f)));
        a.pair = w.pair.clone();
        a
    }

    /// The dictionaries of the active document's stack, for the variables
    /// of the patterns (R-B16-05).
    pub(crate) fn assist_bases(&mut self) -> assist::StackBases {
        let Some(doc) = self.docs.get(self.active) else {
            return assist::StackBases(Vec::new());
        };
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        match self.resolve_stack(&path, &settings) {
            Some(stack) => assist::StackBases(stack.levels.into_iter().map(|l| l.db).collect()),
            None => assist::StackBases(Vec::new()),
        }
    }

    /// The path of a translation by the assistants: the composition, the
    /// glossary, or a pattern and how each variable was resolved.
    pub(crate) fn assist_outcome_path(&self, o: &assist::Outcome, a: &Assistant) -> String {
        match o.rule {
            assist::Rule::Pattern(n) => {
                let src = a
                    .patterns
                    .as_ref()
                    .and_then(|p| p.file.patterns.get(n - 2))
                    .map(|p| p.src.clone())
                    .unwrap_or_default();
                let vars: Vec<String> = o
                    .steps
                    .iter()
                    .map(|s| {
                        let how = match &s.how {
                            assist::How::Glossary => self.i18n.t("patterns-resolver-glossary"),
                            assist::How::Adjective => self.i18n.t("patterns-resolver-adjective"),
                            assist::How::Bases => self.i18n.t("patterns-resolver-bases"),
                            assist::How::Pattern(k) => {
                                self.i18n.t_arg("prov-rule-pattern", "n", &k.to_string())
                            }
                            assist::How::Compose => self.i18n.t("prov-rule-compose"),
                            assist::How::Copy => self.i18n.t("patterns-resolver-copy"),
                            assist::How::Parent => self.i18n.t("derive-how-parent"),
                        };
                        self.i18n.t_args(
                            "patterns-path-var",
                            &[
                                ("var", &s.var),
                                ("capture", &s.capture),
                                ("text", &s.text),
                                ("how", &how),
                            ],
                        )
                    })
                    .collect();
                self.i18n.t_args(
                    "patterns-path",
                    &[
                        ("n", &n.to_string()),
                        ("src", &src),
                        ("vars", &vars.join(" ; ")),
                    ],
                )
            }
            _ => o
                .translation
                .as_ref()
                .map(|t| self.assist_path(t))
                .unwrap_or_default(),
        }
    }

    /// Enregistrer, Importer, Restaurer; the messages.
    fn assist_bottom(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        if let Some(e) = &self.assist_win.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(m) = &self.assist_win.message {
            ui.label(egui::RichText::new(m).small());
        }
        if self.assist_win.confirm_close {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, self.i18n.t("rules-unsaved"));
                if ui.button(self.i18n.t("btn-save")).clicked() && self.assist_save() {
                    self.assist_win.confirm_close = false;
                    self.assist_win.open = false;
                }
                if ui.button(self.i18n.t("matrices-discard")).clicked() {
                    self.assist_win.confirm_close = false;
                    self.assist_win.open = false;
                    self.assist_win.file = None;
                }
                if ui.button(self.i18n.t("btn-cancel")).clicked() {
                    self.assist_win.confirm_close = false;
                }
            });
        }
        let mut action = 0u8;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.assist_win.dirty,
                    egui::Button::new(egui::RichText::new(self.i18n.t("btn-save")).strong()),
                )
                .clicked()
            {
                action = 1;
            }
            if ui.button(self.i18n.t("glossary-import")).clicked() {
                action = 2;
            }
            if ui
                .add_enabled(
                    self.assist_win.origin == Some(Origin::User),
                    egui::Button::new(self.i18n.t("glossary-restore")),
                )
                .clicked()
            {
                action = 3;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.t("btn-close")).clicked() {
                    close = true;
                }
                ui.label(
                    egui::RichText::new(self.i18n.t("glossary-shortcut-hint"))
                        .weak()
                        .small(),
                );
            });
        });
        match action {
            1 => {
                self.assist_save();
            }
            2 => self.assist_import(),
            3 => {
                let name = self.assist_file_name();
                let _ = Dirs::installed().restore(&name);
                self.assist_load();
                self.assist_changed();
                self.assist_win.message = Some(self.i18n.t("glossary-restored"));
            }
            _ => {}
        }
        close
    }

    /// Importer (CSV/TSV): two columns in a group named after the file
    /// (new or existing); originals already in it are refused.
    fn assist_import(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-csv"), &["csv", "tsv", "txt"])
            .pick_file()
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&p) else {
            return;
        };
        let rows = glossary::read_table(&text);
        let group = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(f) = self.assist_win.file.as_mut() else {
            return;
        };
        let (mut n, mut refused) = (0usize, Vec::new());
        for (s, d) in rows {
            match f.add(&group, Term::new(&s, &d, Class::Free)) {
                Ok(()) => n += 1,
                Err(_) => refused.push(s),
            }
        }
        if let Some(k) = f
            .groups
            .iter()
            .position(|g| g.name.eq_ignore_ascii_case(group.trim()))
        {
            self.assist_win.group = k;
        }
        self.assist_win.term = None;
        self.assist_win.dirty |= n > 0;
        let mut msg = self.i18n.t_args(
            "glossary-import-done",
            &[("n", &n.to_string()), ("group", &group)],
        );
        if !refused.is_empty() {
            let shown: Vec<&str> = refused.iter().take(8).map(String::as_str).collect();
            msg = self.i18n.dash(
                &msg,
                &self.i18n.t_args(
                    "glossary-import-refused",
                    &[
                        ("n", &refused.len().to_string()),
                        ("list", &shown.join(", ")),
                    ],
                ),
            );
        }
        self.assist_win.message = Some(msg);
    }

    // ------------------------------------------------------------------
    // Ajouter au glossaire (Ctrl+Alt+G)
    // ------------------------------------------------------------------

    /// Open the small dialog with the source and translation selected in
    /// the edit window (the whole texts without a selection).
    pub(crate) fn open_add_term(&mut self, vctx: &egui::Context, line: usize) {
        let Some((pair, _)) = self.assist_pair() else {
            self.status = self.i18n.t("glossary-no-pair");
            return;
        };
        let Some(e) = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
        else {
            return;
        };
        // The selection kept when the context menu opened, else the
        // current one.
        let ctx_sel = self
            .edit_ctx_sel
            .take()
            .filter(|(l, _, _)| *l == line)
            .map(|(_, a, b)| (a, b));
        let src_id = egui::Id::new(("ximod-edit-src-te", line));
        let pick = |id: egui::Id, text: &str| -> String {
            let chars: Vec<char> = text.chars().collect();
            let kept = if id == src_id { ctx_sel } else { None };
            kept.or_else(|| {
                egui::TextEdit::load_state(vctx, id)
                    .and_then(|st| st.cursor.char_range())
                    .map(|r| (r.primary.index, r.secondary.index))
            })
            .map(|(a, b)| (a.min(b).min(chars.len()), a.max(b).min(chars.len())))
            .filter(|(a, b)| a < b)
            .map(|(a, b)| chars[a..b].iter().collect::<String>())
            .unwrap_or_else(|| text.to_string())
            .trim()
            .to_string()
        };
        let src = pick(src_id, &e.source);
        let dst = pick(egui::Id::new(("ximod-edit-trans", line)), &e.trans);
        let (f, _, _) = Dirs::installed().load(&glossary::file_name(&pair, None));
        let groups: Vec<String> = f.groups.iter().map(|g| g.name.clone()).collect();
        let last = self
            .add_term_last
            .as_ref()
            .and_then(|n| groups.iter().position(|g| g == n));
        self.add_term = Some(AddTerm {
            group: last.or((!groups.is_empty()).then_some(0)),
            groups,
            new_group: String::new(),
            term: Term {
                src,
                dst,
                ..Default::default()
            },
            error: None,
            needs_focus: true,
        });
    }

    /// The dialog, in the edit window's viewport. Returns the message for
    /// the edit window once a term is added.
    pub(crate) fn add_term_dialog(&mut self, vctx: &egui::Context) -> Option<String> {
        let mut dlg = self.add_term.take()?;
        let title = self.i18n.t("glossary-add-title");
        let l_group = self.i18n.t("glossary-add-group");
        let l_new = self.i18n.t("glossary-add-new-group");
        let l_src = self.i18n.t("glossary-col-src");
        let l_dst = self.i18n.t("glossary-col-dst");
        let l_whole = self.i18n.t("glossary-col-whole");
        let l_case = self.i18n.t("glossary-col-case");
        let l_class = self.i18n.t("glossary-col-class");
        let l_gender = self.i18n.t("glossary-col-gender");
        let class_l: Vec<(Class, String)> = Class::ALL
            .iter()
            .map(|c| (*c, self.i18n.t(c.i18n_key())))
            .collect();
        let gender_l: Vec<(Option<Gender>, String)> =
            std::iter::once((None, self.i18n.t("glossary-gender-none")))
                .chain(
                    Gender::ALL
                        .iter()
                        .map(|g| (Some(*g), self.i18n.t(g.i18n_key()))),
                )
                .collect();
        let l_forms = self.i18n.t("glossary-forms");
        let (l_ok, l_cancel) = (self.i18n.t("btn-ok"), self.i18n.t("btn-cancel"));
        let mut ok = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(vctx, |ui| {
                egui::Grid::new("ximod-add-term")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(&l_group);
                        ui.horizontal(|ui| {
                            let cur = match dlg.group {
                                Some(k) => dlg.groups.get(k).cloned().unwrap_or_default(),
                                None => l_new.clone(),
                            };
                            egui::ComboBox::from_id_salt("ximod-add-term-group")
                                .selected_text(cur)
                                .show_field(ui, |ui| {
                                    for (k, g) in dlg.groups.iter().enumerate() {
                                        ui.selectable_value(&mut dlg.group, Some(k), g);
                                    }
                                    ui.selectable_value(&mut dlg.group, None, &l_new);
                                });
                            if dlg.group.is_none() {
                                ui.add(
                                    egui::TextEdit::singleline(&mut dlg.new_group)
                                        .desired_width(160.0),
                                );
                            }
                        });
                        ui.end_row();
                        ui.label(&l_src);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut dlg.term.src).desired_width(300.0),
                        );
                        if dlg.needs_focus {
                            r.request_focus();
                            dlg.needs_focus = false;
                        }
                        ui.end_row();
                        ui.label(&l_dst);
                        ui.add(egui::TextEdit::singleline(&mut dlg.term.dst).desired_width(300.0));
                        ui.end_row();
                        ui.label(&l_class);
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("ximod-add-term-class")
                                .selected_text(
                                    class_l
                                        .iter()
                                        .find(|(c, _)| *c == dlg.term.class)
                                        .map(|(_, s)| s.clone())
                                        .unwrap_or_default(),
                                )
                                .show_field(ui, |ui| {
                                    for (c, l) in &class_l {
                                        ui.selectable_value(&mut dlg.term.class, *c, l);
                                    }
                                });
                            ui.label(&l_gender);
                            egui::ComboBox::from_id_salt("ximod-add-term-gender")
                                .selected_text(
                                    gender_l
                                        .iter()
                                        .find(|(x, _)| *x == dlg.term.gender)
                                        .map(|(_, s)| s.clone())
                                        .unwrap_or_default(),
                                )
                                .show_field(ui, |ui| {
                                    for (x, l) in &gender_l {
                                        ui.selectable_value(&mut dlg.term.gender, *x, l);
                                    }
                                });
                            ui.checkbox(&mut dlg.term.whole, &l_whole);
                            ui.checkbox(&mut dlg.term.case, &l_case);
                        });
                        ui.end_row();
                        if matches!(dlg.term.class, Class::Adj | Class::People | Class::Color) {
                            ui.label(&l_forms);
                            ui.horizontal(|ui| {
                                for gd in FORMS {
                                    let slot = dlg.term.forms.slot_mut(gd);
                                    let mut s = slot.clone().unwrap_or_default();
                                    if ui
                                        .add(egui::TextEdit::singleline(&mut s).desired_width(80.0))
                                        .changed()
                                    {
                                        *slot = (!s.trim().is_empty()).then_some(s);
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&l_cancel).clicked()
                            || ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            cancel = true;
                        }
                        if ui.button(egui::RichText::new(&l_ok).strong()).clicked()
                            || ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            })
                        {
                            ok = true;
                        }
                    });
                });
            });
        if cancel {
            return None;
        }
        if !ok {
            self.add_term = Some(dlg);
            return None;
        }
        // Into the user's copy of the pair's glossary, saved at once.
        let (pair, _) = self.assist_pair()?;
        let name = glossary::file_name(&pair, None);
        let dirs = Dirs::installed();
        let (mut f, origin, _) = dirs.load(&name);
        if origin == Origin::None {
            let (s, d) = pair.split_once('-').unwrap_or(("", ""));
            f.source = s.to_ascii_lowercase();
            f.target = d.to_ascii_lowercase();
        }
        let group = match dlg.group {
            Some(k) => dlg.groups.get(k).cloned().unwrap_or_default(),
            None => dlg.new_group.trim().to_string(),
        };
        if group.is_empty() {
            dlg.error = Some(self.i18n.t_arg("glossary-err-group", "n", "—"));
            self.add_term = Some(dlg);
            return None;
        }
        let mut term = dlg.term.clone();
        term.src = term.src.trim().to_string();
        term.dst = term.dst.trim().to_string();
        if term.forms.is_empty() {
            term.forms = Forms::default();
        }
        let src = term.src.clone();
        let result = f
            .add(&group, term.clone())
            .and_then(|_| dirs.save(&name, &f).map(|_| ()));
        match result {
            Ok(()) => {
                // The window shows the term too (added to its copy when it
                // edits the same file).
                let w = &mut self.assist_win;
                if w.pair == pair && !w.game_file {
                    if w.dirty {
                        if let Some(wf) = w.file.as_mut() {
                            let _ = wf.add(&group, term);
                        }
                    } else {
                        w.file = None;
                    }
                }
                self.assist_changed();
                // The group stays chosen for the next term.
                self.add_term = None;
                self.add_term_last = Some(group.clone());
                Some(
                    self.i18n
                        .t_args("glossary-added", &[("src", &src), ("group", &group)]),
                )
            }
            Err(e) => {
                dlg.error = Some(self.glossary_invalid(&e));
                self.add_term = Some(dlg);
                None
            }
        }
    }

    // ------------------------------------------------------------------
    // Appliquer les assistants (figure B.10)
    // ------------------------------------------------------------------

    /// Dictionnaires › Appliquer les assistants…
    pub(crate) fn open_assist_dialog(&mut self, scope: Scope) {
        if self.docs.is_empty() {
            return;
        }
        if self.all_active() {
            self.status = self.i18n.t("all-mods-file-only");
            return;
        }
        if self.assist_pair().is_none() {
            self.notify_warn(self.i18n.t("dict-stack-no-target"));
            return;
        }
        // Nothing to apply for the pair and the game (B-21).
        if let Some(a) = self.assistant()
            && a.glossary.is_empty()
            && a.patterns.as_ref().is_none_or(|p| p.is_empty())
            && self.derive_active_n(&a) == 0
        {
            let (src, dst) = self.pair_words(&a.pair);
            let game = self
                .active_doc()
                .map(|d| d.settings.game_label())
                .unwrap_or_default();
            self.notify_warn(self.i18n.t_args(
                "assist-none-available",
                &[("src", &src), ("dst", &dst), ("game", &game)],
            ));
            return;
        }
        self.assist_dlg.open = true;
        self.assist_dlg.scope = scope;
        self.assist_dlg.plan = None;
        self.assist_dlg.key = None;
    }

    fn assist_scope_lines(&self) -> Vec<usize> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        match self.assist_dlg.scope {
            Scope::Document => (0..doc.tm.entries.len()).collect(),
            Scope::View => self.visible.clone(),
            Scope::Selection => self.selection.targets(),
        }
    }

    /// The preview, computed again when something changed.
    fn assist_refresh_plan(&mut self) {
        let lines = self.assist_scope_lines();
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let key = PlanKey {
            serial: doc.serial,
            revision: doc.undo.revision,
            scope: self.assist_dlg.scope,
            lines: lines.len(),
            opts: self.assist_dlg.opts.clone(),
            generation: self.assist_gen,
        };
        if self.assist_dlg.key.as_ref() == Some(&key) && self.assist_dlg.plan.is_some() {
            return;
        }
        let opts = self.assist_dlg.opts.clone();
        let Some(plan) = self.assist_plan_with(&lines, &opts, None, false) else {
            return;
        };
        self.assist_dlg.plan = Some(plan);
        self.assist_dlg.key = Some(key);
    }

    /// Appliquer: one undoable operation.
    fn assist_commit(&mut self) {
        let Some(plan) = self.assist_dlg.plan.clone() else {
            return;
        };
        if plan.changes.is_empty() {
            return;
        }
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = plan
            .changes
            .iter()
            .filter_map(|c| {
                doc.tm
                    .entries
                    .get(c.idx)
                    .map(|e| (c.idx, EntryState::capture(e)))
            })
            .collect();
        let touched = assist::commit(&mut doc.tm.entries, &plan);
        let mut op = op_from_before(OpKind::Assist, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        op.label_arg = Some(self.i18n.t_arg("assist-op-lines", "n", &n.to_string()));
        doc.undo.push(op);
        self.pinned_rows.extend(touched.iter().copied());
        let msg = self.i18n.t_args(
            "assist-done",
            &[
                ("n", &plan.filled().to_string()),
                ("composed", &plan.composed.to_string()),
                ("glossary", &plan.glossary.to_string()),
            ],
        );
        let msg = if plan.patterned() > 0 {
            self.i18n.dash(
                &msg,
                &self
                    .i18n
                    .t_arg("assist-done-patterns", "n", &plan.patterned().to_string()),
            )
        } else {
            msg
        };
        let msg = if plan.derived_n() > 0 {
            self.i18n.dash(
                &msg,
                &self
                    .i18n
                    .t_arg("assist-done-derive", "n", &plan.derived_n().to_string()),
            )
        } else {
            msg
        };
        self.notify_ok(msg);
        self.refresh_anomaly_lines(&touched);
        self.assist_dlg.plan = None;
        self.assist_dlg.key = None;
        self.assist_dlg.open = false;
    }

    pub(crate) fn assist_dialog_window(&mut self, ctx: &egui::Context) {
        use crate::apply::Overwrite;
        if !self.assist_dlg.open {
            return;
        }
        if self.docs.is_empty() {
            self.assist_dlg.open = false;
            return;
        }
        self.assist_refresh_plan();
        let Some(a) = self.assistant() else {
            self.assist_dlg.open = false;
            return;
        };
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        let file_name = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let title = self.i18n.t_arg("assist-apply-title", "file", &file_name);
        let (src, dst) = self.pair_words(&a.pair);
        let l_glossary = self.i18n.t_args(
            "assist-apply-glossary",
            &[("src", &src), ("dst", &dst), ("n", &fmt(a.glossary.len()))],
        );
        let l_compose = match &a.composition {
            Some(c) => {
                let order: Vec<String> =
                    c.order.iter().map(|k| self.i18n.t(k.i18n_key())).collect();
                let order = if order.is_empty() {
                    self.i18n.t("assist-order-source")
                } else {
                    order.join(", ")
                };
                let n = a.patterns.as_ref().map(|p| p.len()).unwrap_or(0);
                self.i18n.t_args(
                    "assist-apply-patterns",
                    &[("n", &n.to_string()), ("order", &order)],
                )
            }
            None => self
                .i18n
                .t_arg("assist-apply-compose-none", "pair", &a.pair),
        };
        let (n_doc, n_view, n_sel) = (
            self.docs
                .get(self.active)
                .map(|d| d.tm.entries.len())
                .unwrap_or(0),
            self.visible.len(),
            self.selection.targets().len(),
        );
        let l_scope = [
            self.i18n.t_arg("apply-scope-doc", "n", &fmt(n_doc)),
            self.i18n.t_arg("apply-scope-view", "n", &fmt(n_view)),
            self.i18n.t_arg("apply-scope-selection", "n", &fmt(n_sel)),
        ];
        let labels_over: Vec<(Overwrite, String)> = Overwrite::ALL
            .iter()
            .map(|o| (*o, self.i18n.t(o.i18n_key())))
            .collect();
        let preview: Vec<(String, bool)> = match &self.assist_dlg.plan {
            Some(p) => vec![
                (
                    self.i18n
                        .t_arg("assist-preview-glossary", "n", &fmt(p.glossary)),
                    false,
                ),
                (
                    self.i18n
                        .t_arg("assist-preview-compose", "n", &fmt(p.composed)),
                    false,
                ),
                (
                    {
                        let detail: Vec<String> = p
                            .per_pattern
                            .iter()
                            .map(|(n, c)| {
                                let src = a
                                    .patterns
                                    .as_ref()
                                    .and_then(|x| x.file.patterns.get(n - 2))
                                    .map(|x| x.src.clone())
                                    .unwrap_or_default();
                                self.i18n.t_args(
                                    "assist-preview-pattern-one",
                                    &[("n", &n.to_string()), ("src", &src), ("count", &fmt(*c))],
                                )
                            })
                            .collect();
                        let detail = if detail.is_empty() {
                            "—".to_string()
                        } else {
                            detail.join(", ")
                        };
                        self.i18n.t_args(
                            "assist-preview-patterns",
                            &[("n", &fmt(p.patterned())), ("detail", &detail)],
                        )
                    },
                    false,
                ),
                (
                    self.i18n.t_args(
                        "assist-preview-derive",
                        &[
                            ("n", &fmt(p.derived_n())),
                            ("detail", &self.derive_detail(&p.derived)),
                            ("missing", &fmt(p.untranslated)),
                            ("notfound", &fmt(p.not_found)),
                            ("ambiguous", &fmt(p.ambiguous)),
                        ],
                    ),
                    false,
                ),
                (
                    self.i18n
                        .t_arg("assist-preview-partial", "n", &fmt(p.partial)),
                    false,
                ),
                (
                    self.i18n.t_args(
                        "assist-preview-total",
                        &[
                            ("n", &fmt(p.filled())),
                            ("over", &fmt(p.overwritten)),
                            ("locked", &fmt(p.locked)),
                        ],
                    ),
                    true,
                ),
            ],
            None => vec![(self.i18n.t("apply-preview-computing"), false)],
        };
        let can_apply = self
            .assist_dlg
            .plan
            .as_ref()
            .is_some_and(|p| !p.changes.is_empty());
        let l_assist = self.i18n.t("assist-apply-assistants");
        let l_scope_h = self.i18n.t("replace-scope");
        let l_over_h = self.i18n.t("apply-overwrite");
        let l_preview = self.i18n.t("assist-apply-preview");
        let l_undo = self.i18n.t("assist-preview-undo");
        let l_never = self.i18n.t("assist-never-at-open");
        let l_apply = self.i18n.t("apply-button");
        let l_cancel = self.i18n.t("btn-cancel");
        let has_patterns = a.composition.is_some() || a.patterns.is_some();
        let n_derive = self.derive_active_n(&a);
        let game_label = self
            .active_doc()
            .map(|d| d.settings.game_label())
            .unwrap_or_default();
        let l_derive = self.i18n.t_args(
            "assist-apply-derive",
            &[("game", &game_label), ("n", &n_derive.to_string())],
        );
        let mut do_apply = false;
        let mut cancel = false;
        let dlg = &mut self.assist_dlg;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(640.0);
                // 1 — the assistants.
                egui::Grid::new("ximod-assist-dlg")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.strong(&l_assist);
                        ui.vertical(|ui| {
                            ui.checkbox(&mut dlg.opts.glossary, &l_glossary);
                            ui.add_enabled(
                                has_patterns,
                                egui::Checkbox::new(&mut dlg.opts.patterns, &l_compose),
                            );
                            ui.add_enabled(
                                n_derive > 0,
                                egui::Checkbox::new(&mut dlg.opts.derive, &l_derive),
                            );
                        });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                // 2 — scope and overwriting.
                ui.columns(2, |cols| {
                    cols[0].group(|ui| {
                        ui.strong(&l_scope_h);
                        for (s, l) in [
                            (Scope::Document, &l_scope[0]),
                            (Scope::View, &l_scope[1]),
                            (Scope::Selection, &l_scope[2]),
                        ] {
                            ui.radio_value(&mut dlg.scope, s, l);
                        }
                    });
                    cols[1].group(|ui| {
                        ui.strong(&l_over_h);
                        for (o, l) in &labels_over {
                            ui.radio_value(&mut dlg.opts.overwrite, *o, l);
                        }
                    });
                });
                ui.add_space(6.0);
                // 3 — the preview.
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.strong(&l_preview);
                    for (line, strong) in &preview {
                        if *strong {
                            ui.label(egui::RichText::new(line).strong());
                        } else {
                            ui.label(line);
                        }
                    }
                    ui.label(egui::RichText::new(&l_undo).weak());
                });
                ui.label(egui::RichText::new(&l_never).weak().small());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&l_cancel).clicked()
                            || ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            cancel = true;
                        }
                        if ui
                            .add_enabled(
                                can_apply,
                                egui::Button::new(egui::RichText::new(&l_apply).strong()),
                            )
                            .clicked()
                        {
                            do_apply = true;
                        }
                    });
                });
            });
        if cancel {
            self.assist_dlg.open = false;
        }
        if do_apply {
            self.assist_commit();
        }
    }
}
