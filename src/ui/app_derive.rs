//! Dérivation d'objets (B-21): the Dérivations tab of the window Glossaire,
//! motifs et dérivations (the rules of the game, the rule chosen, the
//! essay on the active document), the derivations of the dialog Appliquer
//! les assistants, and the derived lines that follow their parent.

use crate::ui::theme::ComboField as _;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use eframe::egui;

use super::XimodApp;
use super::app_patterns::PRESETS;
use crate::apply::Overwrite;
use crate::assist::derive::{self, DeriveFile, Invalid, PerLang, Rule, Sigs};
use crate::assist::patterns::{self, Resolver};
use crate::assist::{self, DeriveCtx, Gender};
use crate::tm::TmEntry;
use crate::undo::{EntryState, OpKind, op_from_before};

/// The genders a rule may declare, `None` first.
const GENDERS: [Option<Gender>; 5] = [
    None,
    Some(Gender::M),
    Some(Gender::F),
    Some(Gender::Mp),
    Some(Gender::Fp),
];

/// The extensions of the plugins, whose tomes may teach a spell.
fn is_plugin(p: &std::path::Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "esp" | "esm" | "esl"))
}

impl XimodApp {
    /// The words of an invalid derivation file.
    pub(crate) fn derive_invalid(&self, e: &Invalid) -> String {
        match e {
            Invalid::Json(m) => self.i18n.t_arg("glossary-err-json", "error", m),
            Invalid::Format(n) => self.i18n.t_arg("glossary-err-format", "n", &n.to_string()),
            Invalid::NoId(n) => self.i18n.t_arg("derive-err-noid", "n", &n.to_string()),
            Invalid::DuplicateId(id) => self.i18n.t_arg("derive-err-dup", "id", id),
            Invalid::Sig { id, sig } => self
                .i18n
                .t_args("derive-err-sig", &[("id", id), ("sig", sig)]),
            Invalid::EmptySrc(id) => self.i18n.t_arg("derive-err-empty", "id", id),
            Invalid::VarMissing { id, var, lang } => self.i18n.t_args(
                "derive-err-var",
                &[
                    ("id", id),
                    ("var", var),
                    ("lang", &self.derive_lang_name(lang)),
                ],
            ),
        }
    }

    /// The name of a target language; every language for « ».
    fn derive_lang_name(&self, lang: &str) -> String {
        if lang.is_empty() {
            self.i18n.t("derive-all-langs")
        } else {
            self.i18n.display_name(lang)
        }
    }

    /// The target language of the window's pair (`fra`).
    fn derive_win_lang(&self) -> String {
        self.assist_win
            .pair
            .split_once('-')
            .map(|(_, d)| d.to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// Read the derivation file of the edited game again.
    fn derive_load(&mut self) {
        let dirs = assist::glossary::Dirs::installed();
        let w = &mut self.assist_win;
        let (f, user) = derive::load_file(&dirs.shipped, dirs.user.as_ref(), &w.game)
            .unwrap_or_else(|| {
                (
                    DeriveFile {
                        game: w.game.clone(),
                        ..Default::default()
                    },
                    false,
                )
            });
        w.dfile = Some(f);
        w.duser = user;
        w.dsel = None;
        w.dbuf = None;
        w.ddirty = false;
        w.dedit += 1;
        w.derror = None;
        w.dmessage = None;
    }

    fn derive_save(&mut self) -> bool {
        let Some(f) = self.assist_win.dfile.clone() else {
            return false;
        };
        match derive::save(&derive::user_dir(), &self.assist_win.game, &f) {
            Ok(p) => {
                let w = &mut self.assist_win;
                w.ddirty = false;
                w.duser = true;
                w.derror = None;
                w.dmessage = Some(self.i18n.t_arg(
                    "glossary-saved",
                    "path",
                    &p.display().to_string(),
                ));
                self.assist_changed();
                true
            }
            Err(e) => {
                self.assist_win.derror = Some(self.derive_invalid(&e));
                false
            }
        }
    }

    /// The spells the tomes of a document teach, read once per document.
    fn derive_teaches_of(&mut self, idx: usize) -> Arc<HashMap<u32, u32>> {
        let Some(doc) = self.docs.get(idx) else {
            return Arc::default();
        };
        if let Some(t) = self.derive_teaches.get(&doc.serial) {
            return t.clone();
        }
        let t = if is_plugin(&doc.path) {
            Arc::new(derive::teaches(&doc.path))
        } else {
            Arc::default()
        };
        self.derive_teaches.insert(doc.serial, t.clone());
        t
    }

    /// What the assistants would fill among `lines` of the active document
    /// (R-B21-05): the glossary and the patterns, then the derivations of
    /// `file` (the game's by default), whose parents are the document, the
    /// other open files of the same pair and the bases of the stack.
    pub(crate) fn assist_plan_with(
        &mut self,
        lines: &[usize],
        opts: &assist::Options,
        file: Option<Arc<DeriveFile>>,
        blank_targets: bool,
    ) -> Option<assist::Plan> {
        let a = self.assistant()?;
        let bases = self.assist_bases();
        let teaches = self.derive_teaches_of(self.active);
        let doc = self.docs.get(self.active)?;
        let lang = doc.settings.target.trim().to_ascii_lowercase();
        let pair = doc.settings.pair_suffix();
        let open: Vec<&[TmEntry]> = self
            .docs
            .iter()
            .enumerate()
            .filter(|(i, d)| {
                *i != self.active && d.all.is_none() && d.settings.pair_suffix() == pair
            })
            .map(|(_, d)| d.tm.entries.as_slice())
            .collect();
        let file = file.or_else(|| a.derive.clone());
        // The essay: the derived objects as if they had no translation yet.
        let blanked: Option<Vec<TmEntry>> = match (&file, blank_targets) {
            (Some(f), true) => {
                let targets: HashSet<&str> = f.rules.iter().map(|r| r.target.as_str()).collect();
                Some(
                    doc.tm
                        .entries
                        .iter()
                        .map(|e| {
                            let mut e = e.clone();
                            if targets.contains(format!("{}:{}", e.rec_sig, e.field_sig).as_str())
                                && crate::status::derive(&e) != crate::status::Status::Ignored
                            {
                                e.trans.clear();
                                e.prov.clear();
                            }
                            e
                        })
                        .collect(),
                )
            }
            _ => None,
        };
        let entries = blanked.as_deref().unwrap_or(&doc.tm.entries);
        let ctx = file.as_deref().map(|f| DeriveCtx {
            file: f,
            lang: &lang,
            open,
            parent_bases: &bases,
            teaches: &teaches,
        });
        Some(assist::plan_all(
            entries,
            lines,
            &a,
            opts,
            &bases,
            ctx.as_ref(),
        ))
    }

    /// The active rules of the active document's game for its target.
    pub(crate) fn derive_active_n(&self, a: &assist::Assistant) -> usize {
        let lang = self
            .active_doc()
            .map(|d| d.settings.target.trim().to_ascii_lowercase())
            .unwrap_or_default();
        a.derive
            .as_ref()
            .map(|f| f.active(&lang).count())
            .unwrap_or(0)
    }

    /// « scroll : 136, tome : 190 ».
    pub(crate) fn derive_detail(&self, per: &BTreeMap<String, usize>) -> String {
        if per.is_empty() {
            return "—".to_string();
        }
        per.iter()
            .map(|(id, n)| {
                self.i18n.t_args(
                    "derive-detail-one",
                    &[
                        ("id", id),
                        ("n", &crate::ui::export_dialog::fmt_count(&self.i18n, *n)),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    // ------------------------------------------------------------------
    // Following the parent (R-B21-04)
    // ------------------------------------------------------------------

    /// After each operation on the active document, the derived lines still
    /// En attente whose parent it changed are derived again, in the same
    /// operation (one Ctrl+Z undoes both); the status bar says so.
    pub(crate) fn derive_follow(&mut self) {
        let active = self.active;
        let Some(doc) = self.docs.get(active) else {
            return;
        };
        if doc.all.is_some() {
            return;
        }
        let pushes = doc.undo.pushes;
        let seen = self.derive_seen.entry(doc.serial).or_insert(pushes);
        if *seen == pushes {
            return;
        }
        *seen = pushes;
        let Some(op) = doc.undo.last_op() else {
            return;
        };
        let changed: Vec<usize> = op.changes.iter().map(|c| c.idx).collect();
        let entries = &doc.tm.entries;
        let edids: HashSet<&str> = changed
            .iter()
            .filter_map(|&i| entries.get(i))
            .filter(|e| !e.edid.is_empty())
            .map(|e| e.edid.as_str())
            .collect();
        let followed: HashSet<&str> = entries
            .iter()
            .filter_map(|e| derive::derived_from(&e.prov).map(|(_, p)| p))
            .filter(|p| edids.contains(p))
            .collect();
        if followed.is_empty() {
            return;
        }
        let parent_name = changed
            .iter()
            .filter_map(|&i| entries.get(i))
            .find(|e| followed.contains(e.edid.as_str()))
            .map(|e| e.source.clone())
            .unwrap_or_default();
        let Some(a) = self.assistant() else {
            return;
        };
        let Some(file) = a.derive.clone() else {
            return;
        };
        let bases = self.assist_bases();
        let teaches = self.derive_teaches_of(active);
        let Some(doc) = self.docs.get(active) else {
            return;
        };
        let lang = doc.settings.target.trim().to_ascii_lowercase();
        let pair = doc.settings.pair_suffix();
        let sigs = derive::parent_sigs(&file, &lang);
        let mut parents = derive::Parents::default();
        parents.add(&doc.tm.entries, derive::Origin::Document, &sigs, &|_, e| {
            e.trans.clone()
        });
        for (i, d) in self.docs.iter().enumerate() {
            if i != active && d.all.is_none() && d.settings.pair_suffix() == pair {
                parents.add(&d.tm.entries, derive::Origin::OpenFile, &sigs, &|_, e| {
                    e.trans.clone()
                });
            }
        }
        let d = derive::Deriver {
            file: &file,
            lang: &lang,
            parents: &parents,
            parent_bases: &bases,
            assistant: &a,
            bases: &bases,
            teaches: &teaches,
        };
        let again = derive::refollow(&d, &doc.tm.entries, &changed);
        if again.is_empty() {
            return;
        }
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = again
            .iter()
            .filter_map(|(i, _, _)| doc.tm.entries.get(*i).map(|e| (*i, EntryState::capture(e))))
            .collect();
        for (i, text, prov) in &again {
            if let Some(e) = doc.tm.entries.get_mut(*i) {
                e.trans = text.clone();
                e.prov = prov.clone();
                e.set_translated_flags(true);
                crate::status::set(e, crate::status::Status::Pending);
            }
        }
        let op = op_from_before(OpKind::Assist, &before, &mut doc.tm.entries);
        let touched: Vec<usize> = op.changes.iter().map(|c| c.idx).collect();
        doc.undo.extend_last(op.changes);
        self.status = self.i18n.t_args(
            "derive-followed",
            &[
                (
                    "n",
                    &crate::ui::export_dialog::fmt_count(&self.i18n, touched.len()),
                ),
                ("parent", &parent_name),
            ],
        );
        self.refresh_anomaly_lines(&touched);
    }

    // ------------------------------------------------------------------
    // The tab (figure B.9)
    // ------------------------------------------------------------------

    /// The essay on the active document, with the file as edited: the lines
    /// derivable, per rule, the parents without a translation, a preview.
    fn derive_essay(&mut self) -> Vec<String> {
        let Some(doc) = self.docs.get(self.active) else {
            return vec![self.i18n.t("derive-try-no-doc")];
        };
        if doc.all.is_some() || self.assist_pair().is_none() {
            return vec![self.i18n.t("derive-try-no-doc")];
        }
        let key = (
            doc.serial,
            doc.undo.revision,
            self.assist_win.dedit,
            self.assist_gen,
        );
        if let Some((k, lines)) = &self.assist_win.dtry
            && *k == key
        {
            return lines.clone();
        }
        let file = Arc::new(self.assist_win.dfile.clone().unwrap_or_default());
        let all: Vec<usize> = (0..doc.tm.entries.len()).collect();
        let opts = assist::Options {
            glossary: false,
            patterns: false,
            derive: true,
            overwrite: Overwrite::None,
        };
        let Some(plan) = self.assist_plan_with(&all, &opts, Some(file), true) else {
            return Vec::new();
        };
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        let mut out = vec![self.i18n.t_args(
            "derive-try",
            &[
                ("n", &fmt(plan.filled())),
                ("detail", &self.derive_detail(&plan.derived)),
                ("missing", &fmt(plan.untranslated)),
                ("notfound", &fmt(plan.not_found)),
            ],
        )];
        if let Some(doc) = self.docs.get(self.active) {
            for c in plan.changes.iter().take(4) {
                if let Some(e) = doc.tm.entries.get(c.idx) {
                    out.push(
                        self.i18n
                            .t_args("derive-try-line", &[("src", &e.source), ("dst", &c.trans)]),
                    );
                }
            }
        }
        self.assist_win.dtry = Some((key, out.clone()));
        out
    }

    pub(crate) fn derive_tab(&mut self, ui: &mut egui::Ui) {
        if self.assist_win.dfile.is_none()
            || self
                .assist_win
                .dfile
                .as_ref()
                .is_some_and(|f| !f.game.is_empty() && f.game != self.assist_win.game)
        {
            self.derive_load();
        }
        let lang = self.derive_win_lang();
        let lang_name = self.derive_lang_name(&lang);
        let name = derive::file_name(&self.assist_win.game);
        let origin = self.i18n.t(if self.assist_win.duser {
            "glossary-origin-user"
        } else {
            "glossary-origin-shipped"
        });
        let f = self.assist_win.dfile.clone().unwrap_or_default();
        // 1 — the file.
        ui.label(
            egui::RichText::new(self.i18n.t_args(
                "derive-file",
                &[
                    (
                        "path",
                        &format!("{}/{name} ({origin})", assist::glossary::DIR),
                    ),
                    ("n", &f.rules.len().to_string()),
                ],
            ))
            .weak()
            .small(),
        );
        ui.label(
            egui::RichText::new(self.i18n.t_arg("derive-target-lang", "lang", &lang_name)).small(),
        );
        ui.add_space(4.0);
        // 2 — the rules.
        let cols = [
            self.i18n.t("derive-col-id"),
            self.i18n.t("derive-col-target"),
            self.i18n.t("derive-col-src"),
            self.i18n.t("derive-col-parent"),
            self.i18n.t("derive-col-dst"),
            self.i18n.t("derive-col-active"),
        ];
        let inactive_l = self.i18n.t("derive-inactive");
        let teaches_l = self.i18n.t("derive-ref-teaches");
        let avail = ui.available_height();
        let mut touched = false;
        let mut moved: Option<i32> = None;
        let mut add = false;
        let mut remove = false;
        let mut toggled: Option<usize> = None;
        egui::ScrollArea::both()
            .id_salt("ximod-derive-list")
            .max_height((avail - 330.0).max(110.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("ximod-derive-grid")
                    .striped(true)
                    .num_columns(6)
                    .min_col_width(50.0)
                    .show(ui, |ui| {
                        for c in &cols {
                            ui.strong(c);
                        }
                        ui.end_row();
                        for (k, r) in f.rules.iter().enumerate() {
                            if ui
                                .selectable_label(self.assist_win.dsel == Some(k), &r.id)
                                .clicked()
                            {
                                self.assist_win.dsel = Some(k);
                            }
                            ui.label(egui::RichText::new(&r.target).monospace());
                            ui.label(egui::RichText::new(&r.src).monospace());
                            let mut parent = r.parents().join(", ");
                            if r.reference.is_some() {
                                parent = format!("{parent} + {teaches_l}");
                            }
                            ui.label(egui::RichText::new(parent).monospace());
                            match r.dst_for(&lang) {
                                Some(d) => ui.label(egui::RichText::new(d).monospace()),
                                None => ui.label(egui::RichText::new(&inactive_l).weak().italics()),
                            };
                            let mut on = r.enabled;
                            if ui.checkbox(&mut on, "").changed() {
                                toggled = Some(k);
                            }
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
            if ui.button(self.i18n.t("derive-add")).clicked() {
                add = true;
            }
            if ui
                .add_enabled(
                    self.assist_win.dsel.is_some(),
                    egui::Button::new(self.i18n.t("derive-remove")),
                )
                .clicked()
            {
                remove = true;
            }
        });
        // The rules inactive for the target language (T-B21-07).
        let inactive: Vec<String> = f
            .rules
            .iter()
            .filter(|r| r.enabled && r.dst_for(&lang).is_none())
            .map(|r| {
                self.i18n.t_args(
                    "derive-no-target-lang",
                    &[("id", &r.id), ("lang", &lang_name)],
                )
            })
            .collect();
        for m in &inactive {
            ui.label(egui::RichText::new(m).weak().small());
        }
        // The rule chosen.
        let l_id = cols[0].clone();
        let l_target = cols[1].clone();
        let l_parent = cols[3].clone();
        let l_dst = self.i18n.t_arg("derive-dst-lang", "lang", &lang_name);
        let l_strip = self.i18n.t("derive-col-strip");
        let l_strip_hint = self.i18n.t("derive-strip-hint");
        let l_gender = self.i18n.t("derive-gender");
        let l_none = self.i18n.t("glossary-gender-none");
        let l_src_hint = self.i18n.t("patterns-src-hint");
        let l_comment = self.i18n.t("derive-comment");
        let gender_l: Vec<(Option<Gender>, String)> = GENDERS
            .iter()
            .map(|g| {
                (
                    *g,
                    g.map(|g| self.i18n.t(g.i18n_key()))
                        .unwrap_or_else(|| l_none.clone()),
                )
            })
            .collect();
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
        let Some(file) = w.dfile.as_mut() else {
            return;
        };
        if let Some(k) = toggled
            && let Some(r) = file.rules.get_mut(k)
        {
            r.enabled = !r.enabled;
            touched = true;
        }
        if let Some(d) = moved
            && let Some(k) = w.dsel
        {
            let j = k as i64 + d as i64;
            if j >= 0 && (j as usize) < file.rules.len() {
                file.rules.swap(k, j as usize);
                w.dsel = Some(j as usize);
                w.dbuf = None;
                touched = true;
            }
        }
        if add {
            file.rules.push(Rule {
                id: format!("rule{}", file.rules.len() + 1),
                target: String::new(),
                src: "%A%".into(),
                parent: Sigs::One(String::new()),
                reference: None,
                dst: PerLang::Langs(BTreeMap::from([(lang.clone(), "%A%".to_string())])),
                strip: None,
                vars: BTreeMap::new(),
                gender: None,
                enabled: true,
                comment: String::new(),
            });
            w.dsel = Some(file.rules.len() - 1);
            w.dbuf = None;
            touched = true;
        }
        if remove && let Some(k) = w.dsel.filter(|k| *k < file.rules.len()) {
            file.rules.remove(k);
            w.dsel = None;
            w.dbuf = None;
            touched = true;
        }
        if let Some(k) = w.dsel
            && let Some(r) = file.rules.get_mut(k)
        {
            // The buffers of the lists: the parents, the prefixes to strip.
            if w.dbuf.as_ref().is_none_or(|b| b.0 != k) {
                w.dbuf = Some((k, r.parents().join(", "), r.strip_for(&lang).join("\n")));
            }
            let buf = w.dbuf.as_mut().expect("set above");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&l_id);
                    touched |= ui
                        .add(egui::TextEdit::singleline(&mut r.id).desired_width(90.0))
                        .changed();
                    ui.label(&l_target);
                    touched |= ui
                        .add(
                            egui::TextEdit::singleline(&mut r.target)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(90.0),
                        )
                        .changed();
                    ui.label(&l_parent);
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut buf.1)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(180.0),
                        )
                        .changed()
                    {
                        let list: Vec<String> = buf
                            .1
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        r.parent = match list.len() {
                            1 => Sigs::One(list[0].clone()),
                            _ => Sigs::Many(list),
                        };
                        touched = true;
                    }
                    let mut teaches = r.reference.as_deref() == Some("teaches");
                    if ui.checkbox(&mut teaches, &teaches_l).changed() {
                        r.reference = teaches.then(|| "teaches".to_string());
                        touched = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    touched |= ui
                        .add(
                            egui::TextEdit::singleline(&mut r.src)
                                .hint_text(&l_src_hint)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(240.0),
                        )
                        .changed();
                    ui.label(crate::icons::ARROW_RIGHT);
                    ui.label(&l_dst);
                    let mut dst = r.dst_for(&lang).unwrap_or_default().to_string();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut dst)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(260.0),
                        )
                        .changed()
                    {
                        match &mut r.dst {
                            PerLang::All(s) => *s = dst,
                            PerLang::Langs(m) => {
                                if dst.trim().is_empty() {
                                    m.remove(&lang);
                                } else {
                                    m.insert(lang.clone(), dst);
                                }
                            }
                        }
                        touched = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(&l_strip);
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut buf.2)
                                .hint_text(&l_strip_hint)
                                .desired_rows(2)
                                .desired_width(200.0),
                        )
                        .changed()
                    {
                        let list: Vec<String> = buf
                            .2
                            .lines()
                            .map(|l| l.trim_end_matches('\r').to_string())
                            .filter(|l| !l.trim().is_empty())
                            .collect();
                        let mut m = match r.strip.take() {
                            Some(PerLang::Langs(m)) => m,
                            Some(PerLang::All(v)) => BTreeMap::from([(lang.clone(), v)]),
                            None => BTreeMap::new(),
                        };
                        if list.is_empty() {
                            m.remove(&lang);
                        } else {
                            m.insert(lang.clone(), list);
                        }
                        r.strip = (!m.is_empty()).then_some(PerLang::Langs(m));
                        touched = true;
                    }
                    ui.separator();
                    ui.label(&l_gender);
                    let cur = r.gender_for(&lang);
                    let shown = gender_l
                        .iter()
                        .find(|(g, _)| *g == cur)
                        .map(|(_, s)| s.clone())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("ximod-derive-gender")
                        .selected_text(shown)
                        .show_field(ui, |ui| {
                            for (g, l) in &gender_l {
                                if ui.selectable_label(cur == *g, l).clicked() {
                                    let mut m = match r.gender.take() {
                                        Some(PerLang::Langs(m)) => m,
                                        Some(PerLang::All(v)) => {
                                            BTreeMap::from([(lang.clone(), v)])
                                        }
                                        None => BTreeMap::new(),
                                    };
                                    match g {
                                        Some(g) => {
                                            m.insert(lang.clone(), *g);
                                        }
                                        None => {
                                            m.remove(&lang);
                                        }
                                    }
                                    r.gender = (!m.is_empty()).then_some(PerLang::Langs(m));
                                    touched = true;
                                }
                            }
                        });
                    // The other variables (%A% is the parent).
                    let vars = patterns::variables(&r.src);
                    for (v, _) in vars.iter().filter(|(v, _)| v != "A") {
                        ui.separator();
                        ui.label(format!("{v} :"));
                        let cur: Option<Resolver> = match r.vars.get(v).map(Vec::as_slice) {
                            Some([x]) => Some(*x),
                            _ => None,
                        };
                        let shown = preset_l
                            .iter()
                            .find(|(x, _)| *x == cur)
                            .map(|(_, s)| s.clone())
                            .unwrap_or_else(|| {
                                r.vars
                                    .get(v)
                                    .map(|l| {
                                        l.iter().map(|x| x.code()).collect::<Vec<_>>().join(", ")
                                    })
                                    .unwrap_or_default()
                            });
                        egui::ComboBox::from_id_salt(("ximod-derive-var", v.clone()))
                            .selected_text(shown)
                            .show_field(ui, |ui| {
                                for (x, l) in &preset_l {
                                    if ui.selectable_label(cur == *x, l).clicked() {
                                        match x {
                                            None => {
                                                r.vars.remove(v);
                                            }
                                            Some(x) => {
                                                r.vars.insert(v.clone(), vec![*x]);
                                            }
                                        }
                                        touched = true;
                                    }
                                }
                            });
                    }
                    let names: Vec<String> = vars.iter().map(|v| v.0.clone()).collect();
                    let before = r.vars.len();
                    r.vars.retain(|k, _| names.contains(k));
                    touched |= r.vars.len() != before;
                });
                ui.horizontal(|ui| {
                    ui.label(&l_comment);
                    touched |= ui
                        .add(egui::TextEdit::singleline(&mut r.comment).desired_width(480.0))
                        .changed();
                });
            });
        }
        if touched {
            w.ddirty = true;
            w.dedit += 1;
            w.dmessage = None;
        }
        // 3 — the essay on the active document.
        ui.add_space(4.0);
        ui.strong(self.i18n.t("derive-try-title"));
        for (k, line) in self.derive_essay().iter().enumerate() {
            let t = egui::RichText::new(line).small();
            ui.label(if k == 0 { t } else { t.weak() });
        }
        // A problem of the file, shown at once.
        if let Some(Err(e)) = self.assist_win.dfile.as_ref().map(|f| f.validate()) {
            let msg = self.derive_invalid(&e);
            ui.colored_label(ui.visuals().error_fg_color, msg);
        }
    }

    /// Enregistrer, Restaurer; the messages.
    pub(crate) fn derive_bottom(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        if let Some(e) = &self.assist_win.derror {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(m) = &self.assist_win.dmessage {
            ui.label(egui::RichText::new(m).small());
        }
        if self.assist_win.confirm_close {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, self.i18n.t("rules-unsaved"));
                if ui.button(self.i18n.t("btn-save")).clicked() && self.derive_save() {
                    self.assist_win.confirm_close = false;
                    if !self.assist_win.dirty && !self.assist_win.pdirty {
                        self.assist_win.open = false;
                    }
                }
                if ui.button(self.i18n.t("matrices-discard")).clicked() {
                    self.assist_win.confirm_close = false;
                    self.assist_win.dfile = None;
                    self.assist_win.ddirty = false;
                    if !self.assist_win.dirty && !self.assist_win.pdirty {
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
                    self.assist_win.ddirty,
                    egui::Button::new(egui::RichText::new(self.i18n.t("btn-save")).strong()),
                )
                .clicked()
            {
                action = 1;
            }
            if ui
                .add_enabled(
                    self.assist_win.duser,
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
                self.derive_save();
            }
            2 => {
                let p = derive::user_dir().join(derive::file_name(&self.assist_win.game));
                let _ = std::fs::remove_file(p);
                self.derive_load();
                self.assist_changed();
                self.assist_win.dmessage = Some(self.i18n.t("glossary-restored"));
            }
            _ => {}
        }
        close
    }
}
