//! The spell checker window (CDC §8.3), on the Traduction column or, since
//! D-11, on the Source column: a dictionary per column (the target language,
//! the source language), chosen per language and kept in `[Spell]
//! Dict_<langue>`; the source is never rewritten; the last scan of each
//! column marks the grid (A-16), and a line validated is checked again.

use eframe::egui;

use super::{EntryState, OpKind, SpellAction, SpellIssue, XimodApp, op_from_before, replace_word};

/// The column the correcteur checks (R-D11-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SpellCol {
    #[default]
    Trans,
    Source,
}

impl XimodApp {
    /// Open the spell-check window: load the dictionary of the column if
    /// needed, then scan the active document.
    pub(crate) fn open_spell(&mut self) {
        self.show_spell = true;
        self.spell_needs_center = true;
        self.ensure_spell_loaded();
        self.run_spell_scan();
    }

    /// The language of a column for the active document.
    fn spell_lang_of(&self, col: SpellCol) -> String {
        let s = self.active_settings();
        match col {
            SpellCol::Trans => s.target,
            SpellCol::Source => s.source,
        }
    }

    /// `langue|dictionnaire choisi`: what a checker was loaded for.
    fn spell_key(&self, lang: &str) -> String {
        let choice = self
            .config
            .spell_dicts
            .get(lang)
            .cloned()
            .unwrap_or_default();
        format!("{lang}|{choice}")
    }

    /// Load the checker of the window's column when its language or chosen
    /// dictionary changed (or it was never loaded). A missing dictionary
    /// leaves it `None`.
    pub(crate) fn ensure_spell_loaded(&mut self) {
        self.ensure_spell_col(self.spell_col);
    }

    fn ensure_spell_col(&mut self, col: SpellCol) {
        let lang = self.spell_lang_of(col);
        let key = self.spell_key(&lang);
        let choice = self.config.spell_dicts.get(&lang).cloned();
        let (slot, loaded) = match col {
            SpellCol::Trans => (&mut self.spell, &mut self.spell_lang),
            SpellCol::Source => (&mut self.spell_src, &mut self.spell_src_lang),
        };
        if *loaded != key {
            *loaded = key;
            *slot = crate::spell::SpellChecker::load(&lang, choice.as_deref());
        }
    }

    fn checker(&self, col: SpellCol) -> Option<&crate::spell::SpellChecker> {
        match col {
            SpellCol::Trans => self.spell.as_ref(),
            SpellCol::Source => self.spell_src.as_ref(),
        }
    }

    fn checker_mut(&mut self, col: SpellCol) -> Option<&mut crate::spell::SpellChecker> {
        match col {
            SpellCol::Trans => self.spell.as_mut(),
            SpellCol::Source => self.spell_src.as_mut(),
        }
    }

    /// Force a fresh reload attempt (e.g. after the user dropped new
    /// dictionary files), then rescan.
    fn reload_spell(&mut self) {
        match self.spell_col {
            SpellCol::Trans => self.spell_lang.clear(),
            SpellCol::Source => self.spell_src_lang.clear(),
        }
        self.ensure_spell_loaded();
        self.run_spell_scan();
    }

    /// The text of a column of a line.
    fn spell_text(e: &crate::tm::TmEntry, col: SpellCol) -> &str {
        match col {
            SpellCol::Trans => &e.trans,
            SpellCol::Source => &e.source,
        }
    }

    /// The distinct misspelled words of a text, in order.
    fn spell_words(checker: &crate::spell::SpellChecker, text: &str) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        checker
            .check_text(text)
            .into_iter()
            .filter(|m| seen.insert(m.word.to_lowercase()))
            .map(|m| m.word)
            .collect()
    }

    /// Scan the active document's column, filling `spell_issues` with one
    /// entry per distinct misspelled word per line (with cached
    /// suggestions), and the marks of that column.
    pub(crate) fn run_spell_scan(&mut self) {
        self.spell_issues.clear();
        let col = self.spell_col;
        let Some(checker) = self.checker(col) else {
            self.spell_status = self.i18n.t("spell-no-dict");
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            self.spell_status = self.i18n.t("status-no-doc");
            return;
        };
        let mut issues = Vec::new();
        let mut map: std::collections::HashMap<usize, Vec<String>> = Default::default();
        for (i, e) in doc.tm.entries.iter().enumerate() {
            let text = Self::spell_text(e, col);
            // The source of a line never shown in game, or Ignoré (an
            // identifier locked by a rule, a line set aside), is not checked:
            // it is not text a player reads.
            if text.is_empty()
                || (col == SpellCol::Source
                    && (e.unused || crate::status::derive(e) == crate::status::Status::Ignored))
            {
                continue;
            }
            let words = Self::spell_words(checker, text);
            for w in &words {
                issues.push(SpellIssue {
                    entry: i,
                    word: w.clone(),
                });
            }
            if !words.is_empty() {
                map.insert(i, words);
            }
        }
        let count = issues.len();
        let code = checker.code().to_string();
        let marks = Some((doc.serial, std::sync::Arc::new(map)));
        match col {
            SpellCol::Trans => self.spell_marks = marks,
            SpellCol::Source => self.spell_marks_src = marks,
        }
        self.spell_issues = issues;
        self.spell_sugg.borrow_mut().clear();
        self.spell_status = match (col, count) {
            (_, 0) => self.i18n.t("spell-clean"),
            (SpellCol::Trans, n) => self.i18n.t_arg("spell-issues", "n", &n.to_string()),
            (SpellCol::Source, n) => {
                // The dictionary the translation is checked with, loaded or
                // not.
                let target = self.spell_lang_of(SpellCol::Trans);
                let all = crate::spell::available_dictionaries();
                let other = crate::spell::pick(
                    &target,
                    self.config.spell_dicts.get(&target).map(String::as_str),
                    &all,
                )
                .map(|d| d.code.clone())
                .unwrap_or_else(|| "—".into());
                self.i18n.t_args(
                    "spell-issues-source",
                    &[("n", &n.to_string()), ("dict", &code), ("other", &other)],
                )
            }
        };
    }

    /// Lines validated (R-D11-04): their translation is checked again when
    /// the Traduction column has a scan of this document, so its marks and
    /// the window's list follow at once.
    pub(crate) fn spell_recheck_lines(&mut self, lines: &[usize]) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let Some((serial, map)) = &self.spell_marks else {
            return;
        };
        if *serial != doc.serial {
            return;
        }
        // The checker must still be the document's target language.
        if !self
            .spell_lang
            .starts_with(&format!("{}|", doc.settings.target))
        {
            return;
        }
        let Some(checker) = self.spell.as_ref() else {
            return;
        };
        let mut map = (**map).clone();
        let mut fresh: Vec<SpellIssue> = Vec::new();
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            let words = Self::spell_words(checker, &e.trans);
            for w in &words {
                fresh.push(SpellIssue {
                    entry: i,
                    word: w.clone(),
                });
            }
            if words.is_empty() {
                map.remove(&i);
            } else {
                map.insert(i, words);
            }
        }
        self.spell_marks = Some((doc.serial, std::sync::Arc::new(map)));
        if self.spell_col == SpellCol::Trans {
            self.spell_issues.retain(|x| !lines.contains(&x.entry));
            self.spell_issues.extend(fresh);
            self.spell_issues.sort_by_key(|x| x.entry);
        }
    }

    /// Apply a spell action requested from a window row, then rescan. The
    /// source is never rewritten (R-D11-03).
    fn apply_spell_action(&mut self, act: SpellAction) {
        let col = self.spell_col;
        match act {
            SpellAction::Fix(idx, to) if col == SpellCol::Trans => {
                if let Some(issue) = self.spell_issues.get(idx) {
                    let (entry, from) = (issue.entry, issue.word.clone());
                    if let Some(doc) = self.docs.get_mut(self.active)
                        && entry < doc.tm.entries.len()
                    {
                        let before = vec![(entry, EntryState::capture(&doc.tm.entries[entry]))];
                        replace_word(&mut doc.tm.entries[entry].trans, &from, &to);
                        if doc.tm.entries[entry].trans != before[0].1.trans {
                            doc.tm.entries[entry].prov = crate::provenance::REPLACE.to_string();
                        }
                        doc.undo
                            .push(op_from_before(OpKind::Edit, &before, &mut doc.tm.entries));
                    }
                }
            }
            SpellAction::Fix(..) => {}
            SpellAction::Ignore(idx) => {
                if let Some(w) = self.spell_issues.get(idx).map(|i| i.word.clone())
                    && let Some(c) = self.checker_mut(col)
                {
                    c.ignore(&w);
                }
            }
            SpellAction::Add(idx) => {
                if let Some(w) = self.spell_issues.get(idx).map(|i| i.word.clone())
                    && let Some(c) = self.checker_mut(col)
                {
                    c.add_word(&w);
                }
            }
        }
        self.run_spell_scan();
    }

    /// The spell-check window (independent, resizable viewport; geometry
    /// saved).
    pub(crate) fn spell_window(&mut self, ctx: &egui::Context) {
        if !self.show_spell {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-spell");
        let (w, h) = (self.config.spell_w, self.config.spell_h);
        let mut builder = egui::ViewportBuilder::default().with_title(self.i18n.t("spell-title"));
        let just_opened = self.spell_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.spell_x, self.config.spell_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        ctx.show_viewport_immediate(id, builder, |vui, _class| {
            let vctx = &vui.ctx().clone();
            self.spell_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let mut out = SpellOut::default();
            egui::CentralPanel::default().show(vui, |ui| {
                self.spell_body(ui, &mut out);
            });
            if let Some((x, y, w, h)) = super::read_geometry(vctx) {
                self.config.spell_x = x;
                self.config.spell_y = y;
                self.config.spell_w = w;
                self.config.spell_h = h;
            }
            if let Some(col) = out.column {
                self.spell_col = col;
                self.ensure_spell_loaded();
                self.run_spell_scan();
            } else if let Some(choice) = out.dict {
                // The dictionary of the language (R-D11-02), for both columns.
                let lang = self.spell_lang_of(self.spell_col);
                match choice {
                    Some(code) => {
                        self.config.spell_dicts.insert(lang, code);
                    }
                    None => {
                        self.config.spell_dicts.remove(&lang);
                    }
                }
                let _ = self.config.save();
                self.ensure_spell_loaded();
                self.run_spell_scan();
            } else if let Some(act) = out.action {
                self.apply_spell_action(act);
            } else if out.recheck {
                self.ensure_spell_loaded();
                self.run_spell_scan();
            } else if out.reload {
                self.reload_spell();
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.show_spell = false;
            }
        });
    }

    /// Render the spell window's contents. Only reads `self`; what is asked
    /// is returned through `out` and applied by the caller.
    fn spell_body(&self, ui: &mut egui::Ui, out: &mut SpellOut) {
        let col = self.spell_col;
        // Colonne : Traduction / Source.
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("spell-column"));
            let mut c = col;
            ui.radio_value(&mut c, SpellCol::Trans, self.i18n.t("spell-col-trans"));
            ui.radio_value(&mut c, SpellCol::Source, self.i18n.t("spell-col-source"));
            if c != col {
                out.column = Some(c);
            }
        });
        // Dictionnaire : automatique (en_US) / en_AU / en_GB…
        let lang = self.spell_lang_of(col);
        let all = crate::spell::available_dictionaries();
        let auto = crate::spell::resolve_for_language(&lang, &all).map(|d| d.code.clone());
        let chosen = self.config.spell_dicts.get(&lang).cloned();
        let auto_label = self
            .i18n
            .t_arg("spell-dict-auto", "code", auto.as_deref().unwrap_or("—"));
        if !lang.trim().is_empty() {
            ui.horizontal(|ui| {
                ui.label(self.i18n.t("spell-dict"));
                let selected = chosen.clone().unwrap_or_else(|| auto_label.clone());
                let mut pick = chosen.clone();
                egui::ComboBox::from_id_salt("spell_dict")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, &auto_label);
                        for d in crate::spell::dictionaries_for_language(&lang, &all) {
                            ui.selectable_value(&mut pick, Some(d.code.clone()), &d.code);
                        }
                    });
                if pick != chosen {
                    out.dict = Some(pick);
                }
                let follows = match col {
                    SpellCol::Trans => "spell-follows-target",
                    SpellCol::Source => "spell-follows-source",
                };
                ui.label(egui::RichText::new(self.i18n.t(follows)).weak());
                if self.checker(col).is_some() && ui.button(self.i18n.t("spell-recheck")).clicked()
                {
                    out.recheck = true;
                }
            });
        }

        // No language chosen for the column: nothing to load.
        if lang.trim().is_empty() {
            ui.add_space(6.0);
            let key = match col {
                SpellCol::Trans => "spell-no-target",
                SpellCol::Source => "spell-no-source",
            };
            ui.colored_label(egui::Color32::from_rgb(200, 150, 60), self.i18n.t(key));
            return;
        }
        // No dictionary → which pair of files to drop, and where.
        if self.checker(col).is_none() {
            ui.add_space(6.0);
            ui.colored_label(
                egui::Color32::from_rgb(200, 150, 60),
                self.i18n
                    .t_arg("spell-none-lang", "lang", &self.i18n.display_name(&lang)),
            );
            let dir = crate::spell::dictionaries_dir()
                .map(|d| std::path::absolute(&d).unwrap_or(d).display().to_string())
                .unwrap_or_else(|| "assets/dictionaries".to_string());
            ui.label(self.i18n.t_args(
                "spell-none-pair",
                &[("code", &crate::spell::expected_code(&lang)), ("dir", &dir)],
            ));
            if ui.button(self.i18n.t("spell-reload")).clicked() {
                out.reload = true;
            }
            return;
        }

        ui.separator();
        ui.label(&self.spell_status);
        ui.add_space(4.0);
        if col == SpellCol::Source {
            ui.label(egui::RichText::new(self.i18n.t("spell-source-never")).weak());
        }
        if self.spell_issues.is_empty() {
            return;
        }

        let entries = self.docs.get(self.active).map(|d| &d.tm.entries);
        let ignore_lbl = self.i18n.t("spell-ignore");
        let add_lbl = self.i18n.t("spell-add");
        let sugg_lbl = self.i18n.t("spell-suggestions");
        let row_h = 26.0;
        let n = self.spell_issues.len();
        // At most a few new suggestion lists per frame: the window stays
        // responsive, the rest follows on the next frames.
        let mut budget = 3usize;
        let checker = self.checker(col);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, n, |ui, range| {
                for idx in range {
                    let issue = &self.spell_issues[idx];
                    let suggestions: Option<Vec<String>> = {
                        let mut cache = self.spell_sugg.borrow_mut();
                        if !cache.contains_key(&issue.word)
                            && budget > 0
                            && let Some(c) = checker
                        {
                            budget -= 1;
                            cache.insert(issue.word.clone(), c.suggest(&issue.word));
                        }
                        cache.get(&issue.word).cloned()
                    };
                    if suggestions.is_none() {
                        ui.ctx().request_repaint();
                    }
                    let suggestions = suggestions.unwrap_or_default();
                    ui.horizontal(|ui| {
                        // Locator: entry number + snippet (fixed width): the
                        // source in the Traduction column, the EDID in the
                        // Source column.
                        let loc = entries
                            .and_then(|e| e.get(issue.entry))
                            .map(|e| {
                                let s = match col {
                                    SpellCol::Trans => e.source.replace('\n', " "),
                                    SpellCol::Source => format!("{} · {}", e.edid, e.field_sig),
                                };
                                let s: String = if s.chars().count() > 26 {
                                    s.chars().take(26).collect::<String>() + "…"
                                } else {
                                    s
                                };
                                format!("#{}  {s}", issue.entry + 1)
                            })
                            .unwrap_or_else(|| format!("#{}", issue.entry + 1));
                        ui.allocate_ui_with_layout(
                            egui::vec2(190.0, row_h),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(loc).weak())
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                            },
                        );
                        ui.colored_label(egui::Color32::from_rgb(210, 90, 90), &issue.word);
                        ui.separator();
                        // Ignorer / + Dictionnaire first: they stay in
                        // view whatever the length of the suggestions.
                        if ui.button(&ignore_lbl).clicked() {
                            out.action = Some(SpellAction::Ignore(idx));
                        }
                        if ui.button(&add_lbl).clicked() {
                            out.action = Some(SpellAction::Add(idx));
                        }
                        match col {
                            // Suggestions, applied to the translation.
                            SpellCol::Trans => {
                                for s in &suggestions {
                                    if ui.small_button(s).clicked() {
                                        out.action = Some(SpellAction::Fix(idx, s.clone()));
                                    }
                                }
                            }
                            // Shown only: the source is never rewritten.
                            SpellCol::Source => {
                                if !suggestions.is_empty() {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{sugg_lbl} {}",
                                            suggestions.join(" · ")
                                        ))
                                        .weak(),
                                    );
                                }
                            }
                        }
                    });
                }
            });
    }
}

/// What the window asks for this frame.
#[derive(Default)]
struct SpellOut {
    column: Option<SpellCol>,
    /// `Some(None)`: back to the automatic dictionary.
    dict: Option<Option<String>>,
    action: Option<SpellAction>,
    recheck: bool,
    reload: bool,
}
