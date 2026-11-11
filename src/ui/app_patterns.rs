//! Motifs à variables (B-16): the Motifs tab of the window Glossaire,
//! motifs et dérivations — the patterns of the pair in their order, the
//! pattern chosen, the composition and the rules of the target language,
//! the essay; Enregistrer validates the file (R-B16-07).

use crate::ui::theme::ComboField as _;
use std::sync::Arc;

use eframe::egui;

use super::XimodApp;
use crate::assist::patterns::{self, Invalid, Pattern, Resolver, Side};
use crate::assist::{self, Refusal};

/// The resolver presets a variable may get in the editor: all (the
/// default), or one of them.
pub(crate) const PRESETS: [Option<Resolver>; 5] = [
    None,
    Some(Resolver::Glossary),
    Some(Resolver::Adjective),
    Some(Resolver::Bases),
    Some(Resolver::Patterns),
];

impl XimodApp {
    /// The words of an invalid patterns file.
    pub(crate) fn patterns_invalid(&self, e: &Invalid) -> String {
        match e {
            Invalid::Json(m) => self.i18n.t_arg("glossary-err-json", "error", m),
            Invalid::Format(n) => self.i18n.t_arg("glossary-err-format", "n", &n.to_string()),
            Invalid::Empty(n) => self.i18n.t_arg("patterns-err-empty", "n", &n.to_string()),
            Invalid::VarMissing { n, var, side } => self.i18n.t_args(
                "patterns-err-var-missing",
                &[
                    ("n", &n.to_string()),
                    ("var", var),
                    (
                        "side",
                        &self.i18n.t(match side {
                            Side::Src => "patterns-side-src",
                            Side::Dst => "patterns-side-dst",
                        }),
                    ),
                ],
            ),
            Invalid::MaxLen(n) => self.i18n.t_arg("patterns-err-maxlen", "n", &n.to_string()),
            Invalid::Cycle { a, b } => self.i18n.t_args(
                "patterns-err-cycle",
                &[("a", &a.to_string()), ("b", &b.to_string())],
            ),
            Invalid::BadRef { n, target } => self.i18n.t_args(
                "patterns-err-ref",
                &[("n", &n.to_string()), ("target", &target.to_string())],
            ),
        }
    }

    /// Read the patterns file of the edited pair again.
    fn patterns_load(&mut self) {
        let dirs = assist::glossary::Dirs::installed();
        let w = &mut self.assist_win;
        let (f, user) = patterns::load_file(&dirs.shipped, dirs.user.as_ref(), &w.pair)
            .unwrap_or_else(|| {
                let (s, d) = w.pair.split_once('-').unwrap_or(("", ""));
                (
                    assist::PatternsFile {
                        source: s.to_ascii_lowercase(),
                        target: d.to_ascii_lowercase(),
                        ..Default::default()
                    },
                    false,
                )
            });
        w.pfile = Some(f);
        w.puser = user;
        w.psel = None;
        w.pdirty = false;
        w.perror = None;
        w.pmessage = None;
    }

    fn patterns_save(&mut self) -> bool {
        let Some(f) = self.assist_win.pfile.clone() else {
            return false;
        };
        match patterns::save(&patterns::user_dir(), &self.assist_win.pair, &f) {
            Ok(p) => {
                self.assist_win.pdirty = false;
                self.assist_win.puser = true;
                self.assist_win.perror = None;
                self.assist_win.pmessage = Some(self.i18n.t_arg(
                    "glossary-saved",
                    "path",
                    &p.display().to_string(),
                ));
                self.assist_changed();
                true
            }
            Err(e) => {
                self.assist_win.perror = Some(self.patterns_invalid(&e));
                false
            }
        }
    }

    /// « A : tout ; B : glossaire ».
    fn patterns_vars_words(&self, p: &Pattern) -> String {
        patterns::variables(&p.src)
            .into_iter()
            .map(|(v, _)| {
                let how = match p.vars.get(&v) {
                    None => self.i18n.t("patterns-resolver-all"),
                    Some(list) => list
                        .iter()
                        .map(|r| match r {
                            Resolver::Pattern(n) => {
                                self.i18n.t_arg("prov-rule-pattern", "n", &n.to_string())
                            }
                            r => self.i18n.t(r.i18n_key()),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                format!("{v} : {how}")
            })
            .collect::<Vec<_>>()
            .join(" ; ")
    }

    pub(crate) fn patterns_tab(&mut self, ui: &mut egui::Ui) {
        if self.assist_win.pfile.is_none() {
            self.patterns_load();
        }
        let name = patterns::file_name(&self.assist_win.pair);
        let origin = self.i18n.t(if self.assist_win.puser {
            "glossary-origin-user"
        } else {
            "glossary-origin-shipped"
        });
        let f = self.assist_win.pfile.clone().unwrap_or_default();
        ui.label(
            egui::RichText::new(self.i18n.t_args(
                "patterns-file",
                &[
                    (
                        "path",
                        &format!("{}/{name} ({origin})", assist::glossary::DIR),
                    ),
                    ("n", &f.patterns.len().to_string()),
                ],
            ))
            .weak()
            .small(),
        );
        // The composition, motif 1, and the rules of the language.
        if let Some(c) = &f.composition {
            let order: Vec<String> = c.order.iter().map(|k| self.i18n.t(k.i18n_key())).collect();
            let order = if order.is_empty() {
                self.i18n.t("assist-order-source")
            } else {
                order.join(", ")
            };
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.i18n.t("patterns-composition"));
                ui.label(self.i18n.t_args(
                    "patterns-composition-order",
                    &[("order", &order), ("joiner", &c.joiner)],
                ));
            });
        }
        if !f.elision.is_empty() || !f.contractions.is_empty() {
            let el: Vec<String> = f
                .elision
                .iter()
                .map(|(a, b)| format!("{a} {} {b}", crate::icons::ARROW_RIGHT))
                .collect();
            let co: Vec<String> = f
                .contractions
                .iter()
                .map(|(a, b)| format!("{a} {} {b}", crate::icons::ARROW_RIGHT))
                .collect();
            ui.label(
                egui::RichText::new(self.i18n.t_args(
                    "patterns-rules",
                    &[
                        ("elision", &el.join(", ")),
                        ("contractions", &co.join(", ")),
                    ],
                ))
                .small(),
            );
        }
        ui.add_space(4.0);
        let cols = [
            self.i18n.t("patterns-col-order"),
            self.i18n.t("patterns-col-src"),
            self.i18n.t("patterns-col-dst"),
            self.i18n.t("patterns-col-maxlen"),
            self.i18n.t("patterns-col-recursive"),
            self.i18n.t("patterns-col-vars"),
        ];
        let (yes, no) = (self.i18n.t("glossary-yes"), self.i18n.t("glossary-no"));
        let vars_words: Vec<String> = f
            .patterns
            .iter()
            .map(|p| self.patterns_vars_words(p))
            .collect();
        let avail = ui.available_height();
        let mut touched = false;
        let mut moved: Option<i32> = None;
        let mut add = false;
        let mut remove = false;
        egui::ScrollArea::both()
            .id_salt("ximod-patterns-list")
            .max_height((avail - 230.0).max(110.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("ximod-patterns-grid")
                    .striped(true)
                    .num_columns(6)
                    .min_col_width(50.0)
                    .show(ui, |ui| {
                        for c in &cols {
                            ui.strong(c);
                        }
                        ui.end_row();
                        for (k, p) in f.patterns.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.assist_win.psel == Some(k),
                                    (k + 2).to_string(),
                                )
                                .clicked()
                            {
                                self.assist_win.psel = Some(k);
                            }
                            ui.label(egui::RichText::new(&p.src).monospace());
                            ui.label(egui::RichText::new(&p.dst).monospace());
                            ui.label(p.max_len.to_string());
                            ui.label(if p.recursive { &yes } else { &no });
                            ui.label(&vars_words[k]);
                            ui.end_row();
                        }
                    });
            });
        ui.horizontal(|ui| {
            if ui.small_button(crate::icons::CARET_UP).clicked() {
                moved = Some(-1);
            }
            if ui.small_button(crate::icons::CARET_DOWN).clicked() {
                moved = Some(1);
            }
            if ui.button(self.i18n.t("patterns-add")).clicked() {
                add = true;
            }
            if ui
                .add_enabled(
                    self.assist_win.psel.is_some(),
                    egui::Button::new(self.i18n.t("patterns-remove")),
                )
                .clicked()
            {
                remove = true;
            }
        });
        // The pattern chosen.
        let l_src = self.i18n.t("patterns-src-hint");
        let l_dst = self.i18n.t("patterns-dst-hint");
        let l_max = cols[3].clone();
        let l_rec = cols[4].clone();
        let l_head = self.i18n.t("patterns-head");
        let l_last = self.i18n.t("patterns-head-last");
        let preset_l: Vec<(Option<Resolver>, String)> = PRESETS
            .iter()
            .map(|p| {
                (
                    *p,
                    match p {
                        None => self.i18n.t("patterns-resolver-all"),
                        Some(r) => self.i18n.t(r.i18n_key()),
                    },
                )
            })
            .collect();
        let w = &mut self.assist_win;
        let Some(file) = w.pfile.as_mut() else {
            return;
        };
        if let Some(d) = moved
            && let Some(k) = w.psel
        {
            let j = k as i64 + d as i64;
            if j >= 0 && (j as usize) < file.patterns.len() {
                file.patterns.swap(k, j as usize);
                w.psel = Some(j as usize);
                touched = true;
            }
        }
        if add {
            file.patterns.push(Pattern::default());
            w.psel = Some(file.patterns.len() - 1);
            touched = true;
        }
        if remove && let Some(k) = w.psel.filter(|k| *k < file.patterns.len()) {
            file.patterns.remove(k);
            w.psel = None;
            touched = true;
        }
        if let Some(p) = w.psel.and_then(|k| file.patterns.get_mut(k)) {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    touched |= ui
                        .add(
                            egui::TextEdit::singleline(&mut p.src)
                                .hint_text(&l_src)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(260.0),
                        )
                        .changed();
                    ui.label(crate::icons::ARROW_RIGHT);
                    touched |= ui
                        .add(
                            egui::TextEdit::singleline(&mut p.dst)
                                .hint_text(&l_dst)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(260.0),
                        )
                        .changed();
                    ui.label(&l_max);
                    touched |= ui
                        .add(egui::DragValue::new(&mut p.max_len).range(1..=500))
                        .changed();
                    touched |= ui.checkbox(&mut p.recursive, &l_rec).changed();
                });
                let vars = patterns::variables(&p.src);
                ui.horizontal_wrapped(|ui| {
                    ui.label(&l_head);
                    let cur = p.head.clone().unwrap_or_else(|| l_last.clone());
                    egui::ComboBox::from_id_salt("ximod-patterns-head")
                        .selected_text(cur)
                        .show_field(ui, |ui| {
                            if ui.selectable_label(p.head.is_none(), &l_last).clicked() {
                                p.head = None;
                                touched = true;
                            }
                            for (v, _) in &vars {
                                if ui
                                    .selectable_label(p.head.as_deref() == Some(v), v)
                                    .clicked()
                                {
                                    p.head = Some(v.clone());
                                    touched = true;
                                }
                            }
                        });
                    for (v, _) in &vars {
                        ui.separator();
                        ui.label(format!("{v} :"));
                        let cur: Option<Resolver> = match p.vars.get(v).map(Vec::as_slice) {
                            Some([r]) => Some(*r),
                            _ => None,
                        };
                        let shown = preset_l
                            .iter()
                            .find(|(r, _)| *r == cur)
                            .map(|(_, s)| s.clone())
                            .unwrap_or_else(|| {
                                p.vars
                                    .get(v)
                                    .map(|l| {
                                        l.iter().map(|r| r.code()).collect::<Vec<_>>().join(", ")
                                    })
                                    .unwrap_or_default()
                            });
                        egui::ComboBox::from_id_salt(("ximod-patterns-var", v.clone()))
                            .selected_text(shown)
                            .show_field(ui, |ui| {
                                for (r, l) in &preset_l {
                                    if ui.selectable_label(cur == *r, l).clicked() {
                                        match r {
                                            None => {
                                                p.vars.remove(v);
                                            }
                                            Some(r) => {
                                                p.vars.insert(v.clone(), vec![*r]);
                                            }
                                        }
                                        touched = true;
                                    }
                                }
                            });
                    }
                    // Resolvers of variables no longer in the source.
                    let names: Vec<String> = vars.iter().map(|v| v.0.clone()).collect();
                    let before = p.vars.len();
                    p.vars.retain(|k, _| names.contains(k));
                    touched |= p.vars.len() != before;
                });
            });
        }
        if touched {
            w.pdirty = true;
            w.pmessage = None;
        }
        // The essay: the file as edited.
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("glossary-try"));
            ui.add(egui::TextEdit::singleline(&mut self.assist_win.ptrial).desired_width(360.0));
        });
        let trial = self.assist_win.ptrial.trim().to_string();
        if !trial.is_empty() {
            let a = self.assist_edited();
            let bases = self.assist_bases();
            let shown = match a.run(&trial, true, true, &bases) {
                Ok(o) => self.i18n.t_args(
                    "patterns-try-result",
                    &[
                        ("dst", &o.text),
                        ("path", &self.assist_outcome_path(&o, &a)),
                    ],
                ),
                Err(e) => {
                    let reason = match e {
                        Refusal::Partial(w) => self.i18n.t_args(
                            "glossary-try-partial",
                            &[
                                ("src", &trial),
                                ("n", &w.len().to_string()),
                                ("words", &w.join(", ")),
                            ],
                        ),
                        Refusal::NoTerm => self.i18n.t("glossary-try-none"),
                    };
                    self.i18n.t_arg("patterns-try-fail", "reason", &reason)
                }
            };
            ui.label(egui::RichText::new(shown).small());
        }
        // A problem of the file, shown at once.
        if let Some(Err(e)) = self.assist_win.pfile.as_ref().map(|f| f.validate()) {
            let msg = self.patterns_invalid(&e);
            ui.colored_label(ui.visuals().error_fg_color, msg);
        }
    }

    /// Enregistrer, Restaurer; the messages.
    pub(crate) fn patterns_bottom(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        if let Some(e) = &self.assist_win.perror {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(m) = &self.assist_win.pmessage {
            ui.label(egui::RichText::new(m).small());
        }
        if self.assist_win.confirm_close {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, self.i18n.t("rules-unsaved"));
                if ui.button(self.i18n.t("btn-save")).clicked() && self.patterns_save() {
                    self.assist_win.confirm_close = false;
                    if !self.assist_win.dirty {
                        self.assist_win.open = false;
                    }
                }
                if ui.button(self.i18n.t("matrices-discard")).clicked() {
                    self.assist_win.confirm_close = false;
                    self.assist_win.pfile = None;
                    self.assist_win.pdirty = false;
                    if !self.assist_win.dirty {
                        self.assist_win.open = false;
                    }
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
                    self.assist_win.pdirty,
                    egui::Button::new(egui::RichText::new(self.i18n.t("btn-save")).strong()),
                )
                .clicked()
            {
                action = 1;
            }
            if ui
                .add_enabled(
                    self.assist_win.puser,
                    egui::Button::new(self.i18n.t("glossary-restore")),
                )
                .clicked()
            {
                action = 2;
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
                self.patterns_save();
            }
            2 => {
                let p = patterns::user_dir().join(patterns::file_name(&self.assist_win.pair));
                let _ = std::fs::remove_file(p);
                self.patterns_load();
                self.assist_changed();
                self.assist_win.pmessage = Some(self.i18n.t("glossary-restored"));
            }
            _ => {}
        }
        close
    }

    /// The pattern that would translate the line, for the Vocabulaire tab
    /// (R-B16-06): (number, source of the pattern, translation).
    pub(crate) fn vocab_pattern(&mut self, line: usize) -> Option<(usize, String, String)> {
        let source = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
            .map(|e| e.source.clone())?;
        let a: Arc<assist::Assistant> = self.assistant()?;
        let bases = self.assist_bases();
        let o = a.run(&source, false, true, &bases).ok()?;
        match o.rule {
            assist::Rule::Pattern(n) => {
                let src = a.patterns.as_ref()?.file.patterns.get(n - 2)?.src.clone();
                Some((n, src, o.text))
            }
            _ => None,
        }
    }
}
