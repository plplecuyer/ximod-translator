//! Transformer ▸ (A-13, D-112): the commands on the translation of lines,
//! the prefix / suffix dialog, the edit window's in-field transformation,
//! and the title case lists of Paramètres › Avancé.

use crate::ui::keys::TakeCmd as _;
use eframe::egui;

use super::{XimodApp, confirm_buttons};
use crate::journal::{Kind, Level};
use crate::text::transform::{self, Command, TitleLists};
use crate::text::zh::{self, ZhConv};
use crate::undo::{EntryState, OpKind, op_from_before};

/// The prefix / suffix dialog (R-A13-07).
pub(crate) struct AffixDialog {
    pub cmd: Command,
    pub lines: Vec<usize>,
    pub prefix: String,
    pub suffix: String,
    pub no_repeat: bool,
}

/// The Chinese simplified ↔ traditional dialog (E-08, R-Z): the
/// conversion and the scope; the selection when it was opened.
pub(crate) struct ZhDialog {
    pub selection: Vec<usize>,
    pub conv: ZhConv,
    pub scope: crate::replace::Scope,
}

/// What Transformer remembers in the session: the last prefix and suffix
/// typed, the exception lists once read.
#[derive(Default)]
pub(crate) struct TransformState {
    pub dialog: Option<AffixDialog>,
    pub zh: Option<ZhDialog>,
    /// The last conversion chosen.
    pub zh_last: Option<ZhConv>,
    pub last_prefix: String,
    pub last_suffix: String,
    pub lists: Option<TitleLists>,
}

impl XimodApp {
    /// The exception lists, read once (and again after the settings).
    fn title_rules(&mut self) -> transform::TitleRules {
        let target = self.active_settings().target.clone();
        self.transform
            .lists
            .get_or_insert_with(TitleLists::load)
            .rules(&target)
    }

    /// A command of the menus (R-A13-01): the prefix / suffix ones open
    /// their dialog, the others apply at once.
    pub(crate) fn open_transform(&mut self, cmd: Command, lines: Vec<usize>) {
        if cmd == Command::Chinese {
            if zh::is_chinese(&self.active_settings().target) {
                let scope = if lines.len() > 1 {
                    crate::replace::Scope::Selection
                } else {
                    crate::replace::Scope::Document
                };
                self.transform.zh = Some(ZhDialog {
                    selection: lines,
                    conv: self.transform.zh_last.unwrap_or(ZhConv::S2t),
                    scope,
                });
            }
            return;
        }
        if cmd.needs_dialog() {
            self.transform.dialog = Some(AffixDialog {
                cmd,
                lines,
                prefix: self.transform.last_prefix.clone(),
                suffix: self.transform.last_suffix.clone(),
                no_repeat: true,
            });
        } else {
            self.transform_lines(cmd, &lines, None);
        }
    }

    /// Transform the translation of `lines` (R-A13-01, R-A13-08): one
    /// undoable operation, the status as after typing, the provenance of a
    /// typed text; never a line never shown, Ignoré, held by its file's
    /// VMAD switch or without translation.
    pub(crate) fn transform_lines(
        &mut self,
        cmd: Command,
        lines: &[usize],
        affix: Option<(&str, &str, bool)>,
    ) {
        let rules = self.title_rules();
        let name = self.i18n.t(cmd.i18n_key());
        let f = |t: &str| match (cmd, affix) {
            (Command::AffixAdd, Some((p, s, no_repeat))) => {
                transform::affix_add(t, p, s, no_repeat)
            }
            (Command::AffixRemove, Some((p, s, _))) => transform::affix_remove(t, p, s),
            _ => transform::apply(cmd, t, &rules),
        };
        self.transform_lines_with(&name, lines, f);
    }

    /// Change the translation of `lines` by `f` (`None`: left), as
    /// [`Self::transform_lines`] does; `name` names the change in the
    /// journal and the history.
    pub(crate) fn transform_lines_with(
        &mut self,
        name: &str,
        lines: &[usize],
        f: impl Fn(&str) -> Option<String>,
    ) {
        self.close_edit_session();
        let name = name.to_string();
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        let mut before = Vec::new();
        let mut skipped = 0usize;
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            let st = crate::status::derive(e);
            if e.unused
                || st == crate::status::Status::Ignored
                || crate::vmad_lock::is_file_locked(e)
                || e.trans.is_empty()
            {
                skipped += 1;
                continue;
            }
            let Some(new) = f(&e.trans) else {
                skipped += 1;
                continue;
            };
            if new == e.trans {
                continue;
            }
            let state = EntryState::capture(e);
            let old = e.trans.clone();
            let e = &mut doc.tm.entries[i];
            e.trans = new;
            crate::status::after_typing(e, &old, st);
            before.push((i, state));
        }
        let mut op = op_from_before(OpKind::Transform, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            op.label_arg = Some(name.clone());
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        let msg = if n == 0 {
            self.i18n.t_arg("transform-none", "name", &name)
        } else if skipped > 0 {
            self.i18n.t_args(
                "transform-done-skipped",
                &[
                    ("name", &name),
                    ("n", &n.to_string()),
                    ("skipped", &skipped.to_string()),
                ],
            )
        } else {
            self.i18n
                .t_args("transform-done", &[("name", &name), ("n", &n.to_string())])
        };
        self.journal_add(Level::Info, Kind::Transform, &msg);
        let changed: Vec<usize> = before.iter().map(|(i, _)| *i).collect();
        self.refresh_anomaly_lines(&changed);
        self.status = msg;
    }

    /// Transform the selection of the edit window's translation field, or
    /// the whole field without selection (R-A13-09): a change of the field
    /// in the typing session when there is one, else an operation of its own.
    pub(crate) fn edit_transform_field(&mut self, vctx: &egui::Context, line: usize, cmd: Command) {
        if cmd.needs_dialog() {
            self.open_transform(cmd, vec![line]);
            return;
        }
        let rules = self.title_rules();
        let trans_id = egui::Id::unique(("ximod-edit-trans", line));
        let in_session = self
            .edit_session
            .as_ref()
            .is_some_and(|s| s.doc == self.active && s.before.first().is_some_and(|b| b.0 == line));
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        let Some(e) = doc.tm.entries.get(line) else {
            return;
        };
        if e.unused || crate::vmad_lock::is_file_locked(e) || e.trans.is_empty() {
            return;
        }
        let chars: Vec<char> = e.trans.chars().collect();
        let (a, b) = egui::TextEdit::load_state(vctx, trans_id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b).min(chars.len()), a.max(b).min(chars.len()))
            })
            .filter(|(a, b)| a < b)
            .unwrap_or((0, chars.len()));
        let piece: String = chars[a..b].iter().collect();
        let Some(done) = transform::apply(cmd, &piece, &rules) else {
            return;
        };
        let new: String =
            chars[..a].iter().collect::<String>() + &done + &chars[b..].iter().collect::<String>();
        if new == e.trans {
            return;
        }
        if in_session {
            doc.tm.entries[line].trans = new;
            doc.undo.touch();
        } else {
            let state = EntryState::capture(e);
            let (old, st) = (e.trans.clone(), crate::status::derive(e));
            let e = &mut doc.tm.entries[line];
            e.trans = new;
            crate::status::after_typing(e, &old, st);
            let mut op = op_from_before(OpKind::Transform, &[(line, state)], &mut doc.tm.entries);
            if !op.changes.is_empty() {
                op.label_arg = Some(self.i18n.t(cmd.i18n_key()));
                doc.undo.push(op);
            }
        }
        // The selection stays on the transformed text.
        if let Some(mut st) = egui::TextEdit::load_state(vctx, trans_id) {
            let end = a + done.chars().count();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(a),
                egui::text::CCursor::new(end),
            )));
            st.store(vctx, trans_id);
        }
    }

    /// The prefix / suffix dialog (R-A13-07, figure B.4).
    pub(crate) fn affix_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.transform.dialog.take() else {
            return;
        };
        let add = d.cmd == Command::AffixAdd;
        let title = self.i18n.t(if add {
            "affix-title-add"
        } else {
            "affix-title-remove"
        });
        let (with, first) = match self.docs.get(self.active) {
            Some(doc) => {
                let with: Vec<&crate::tm::TmEntry> = d
                    .lines
                    .iter()
                    .filter_map(|&i| doc.tm.entries.get(i))
                    .filter(|e| !e.trans.is_empty())
                    .collect();
                (with.len(), with.first().map(|e| e.trans.clone()))
            }
            None => (0, None),
        };
        let preview = first.and_then(|t| {
            if add {
                transform::affix_add(&t, &d.prefix, &d.suffix, d.no_repeat)
            } else {
                transform::affix_remove(&t, &d.prefix, &d.suffix)
            }
        });
        let (lbl_prefix, lbl_suffix, lbl_no_repeat) = (
            self.i18n.t("affix-prefix"),
            self.i18n.t("affix-suffix"),
            self.i18n.t("affix-no-repeat"),
        );
        let targets = self.i18n.t_args(
            "affix-targets",
            &[
                ("n", &d.lines.len().to_string()),
                ("with", &with.to_string()),
            ],
        );
        let preview_l = preview.map(|p| self.i18n.t_arg("affix-preview", "text", &p));
        let (ok_l, cancel_l) = (
            self.i18n
                .t(if add { "affix-apply" } else { "affix-remove" }),
            self.i18n.t("btn-cancel"),
        );
        let mut choice = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                egui::Grid::new("ximod-affix")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(&lbl_prefix);
                        ui.add(egui::TextEdit::singleline(&mut d.prefix).desired_width(260.0));
                        ui.end_row();
                        ui.label(&lbl_suffix);
                        ui.add(egui::TextEdit::singleline(&mut d.suffix).desired_width(260.0));
                        ui.end_row();
                    });
                if add {
                    ui.checkbox(&mut d.no_repeat, &lbl_no_repeat);
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&targets).weak());
                if let Some(p) = &preview_l {
                    ui.add(egui::Label::new(egui::RichText::new(p).italics()).wrap());
                }
                ui.add_space(8.0);
                let (ok, cancel) = confirm_buttons(ui, &ok_l, &cancel_l);
                if ok {
                    choice = Some(true);
                } else if cancel {
                    choice = Some(false);
                }
            });
        match choice {
            Some(true) => {
                self.transform.last_prefix = d.prefix.clone();
                self.transform.last_suffix = d.suffix.clone();
                self.transform_lines(d.cmd, &d.lines, Some((&d.prefix, &d.suffix, d.no_repeat)));
            }
            Some(false) => {}
            None => self.transform.dialog = Some(d),
        }
    }

    /// The Chinese simplified ↔ traditional dialog (R-Z): the conversion,
    /// the scope, an example on the first line; one undoable operation.
    pub(crate) fn zh_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.transform.zh.take() else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        use crate::replace::Scope;
        let lines_of = |s: Scope| -> Vec<usize> {
            match s {
                Scope::Document => (0..doc.tm.entries.len()).collect(),
                Scope::View => self.visible.clone(),
                Scope::Selection => d.selection.clone(),
            }
        };
        let lines = lines_of(d.scope);
        let example = lines
            .iter()
            .filter_map(|&i| doc.tm.entries.get(i))
            .find(|e| !e.trans.is_empty() && !e.unused)
            .map(|e| (e.trans.clone(), zh::convert(d.conv, &e.trans)));
        let title = self.i18n.t("zh-title");
        let (ok_l, cancel_l) = (self.i18n.t("zh-apply"), self.i18n.t("btn-cancel"));
        let mut choice = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(480.0);
                ui.label(egui::RichText::new(self.i18n.t("zh-conversion")).strong());
                for c in ZhConv::ALL {
                    if c == ZhConv::T2s {
                        ui.add_space(2.0);
                    }
                    ui.radio_value(&mut d.conv, c, self.i18n.t(c.i18n_key()));
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(self.i18n.t("zh-scope")).strong());
                ui.horizontal(|ui| {
                    for s in [Scope::Selection, Scope::View, Scope::Document] {
                        let n = lines_of(s).len();
                        let label = format!("{} ({n})", self.i18n.t(s.i18n_key()));
                        ui.add_enabled_ui(n > 0, |ui| {
                            ui.radio_value(&mut d.scope, s, label);
                        });
                    }
                });
                if let Some((before, after)) = &example {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(self.i18n.t("zh-example")).weak());
                    ui.add(egui::Label::new(before.as_str()).wrap());
                    ui.add(egui::Label::new(egui::RichText::new(after.as_str()).strong()).wrap());
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(self.i18n.t("zh-kept")).weak().small());
                ui.add_space(8.0);
                let (ok, cancel) = confirm_buttons(ui, &ok_l, &cancel_l);
                if ok {
                    choice = Some(true);
                } else if cancel || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(false);
                }
            });
        match choice {
            Some(true) => {
                self.transform.zh_last = Some(d.conv);
                let name = self.i18n.t(d.conv.i18n_key());
                let conv = d.conv;
                self.transform_lines_with(&name, &lines, |t| Some(zh::convert(conv, t)));
            }
            Some(false) => {}
            None => self.transform.zh = Some(d),
        }
    }

    /// The Transformer ▸ entries of a menu; the command chosen.
    pub(crate) fn transform_menu(
        &self,
        ui: &mut egui::Ui,
        labels: &[(String, String)],
    ) -> Option<Command> {
        let mut pick = None;
        let zh_ok = zh::is_chinese(&self.active_settings().target);
        for (cmd, (label, k)) in Command::ALL.iter().zip(labels) {
            if cmd.starts_group() {
                ui.separator();
            }
            let mut b = egui::Button::new(label);
            if !k.is_empty() {
                b = b.shortcut_text(k);
            }
            let enabled = *cmd != Command::Chinese || zh_ok;
            let resp = ui.add_enabled(enabled, b);
            let resp = if enabled {
                resp
            } else {
                resp.on_disabled_hover_text(self.i18n.t("zh-needs-chinese"))
            };
            if resp.clicked() {
                pick = Some(*cmd);
                ui.close();
            }
        }
        pick
    }

    /// The labels and shortcuts of the Transformer ▸ entries.
    pub(crate) fn transform_labels(&self, ctx: &egui::Context) -> Vec<(String, String)> {
        Command::ALL
            .iter()
            .map(|c| {
                let k = match c {
                    Command::Upper => {
                        super::cmd_text(ctx, crate::shortcuts::Cmd::TransformUpper, &self.i18n)
                    }
                    Command::Lower => {
                        super::cmd_text(ctx, crate::shortcuts::Cmd::TransformLower, &self.i18n)
                    }
                    Command::Title => {
                        super::cmd_text(ctx, crate::shortcuts::Cmd::TransformTitle, &self.i18n)
                    }
                    _ => String::new(),
                };
                (self.i18n.t(c.i18n_key()), k)
            })
            .collect()
    }

    /// The Ctrl+Maj+U / L / T shortcuts consumed from `ctx`, if any.
    pub(crate) fn transform_shortcut(ctx: &egui::Context) -> Option<Command> {
        ctx.input_mut(|i| {
            if i.take_cmd(crate::shortcuts::Cmd::TransformUpper) {
                Some(Command::Upper)
            } else if i.take_cmd(crate::shortcuts::Cmd::TransformLower) {
                Some(Command::Lower)
            } else if i.take_cmd(crate::shortcuts::Cmd::TransformTitle) {
                Some(Command::Title)
            } else {
                None
            }
        })
    }

    /// Paramètres › Avancé › Casse de titre (D-112): the lists of the
    /// target language of the current document, as typed.
    pub(crate) fn title_case_texts(&mut self) -> (String, String, String) {
        let target = self.active_settings().target.clone();
        let rules = TitleLists::load().rules(&target);
        (target, rules.lower.join(", "), rules.keep.join(", "))
    }

    /// Write the lists typed in the settings into the user's copy.
    pub(crate) fn save_title_case(&mut self, lang: &str, lower: &str, keep: &str) {
        if lang.trim().is_empty() {
            return;
        }
        let rules = transform::TitleRules {
            lower: transform::parse_words(lower),
            keep: transform::parse_words(keep),
        };
        if TitleLists::load().rules(lang) == rules {
            return;
        }
        if let Err(e) = TitleLists::save_user(lang, rules) {
            self.notify_err(
                self.i18n
                    .colon(&self.i18n.t("settings-title-case"), &e.to_string()),
            );
        }
        self.transform.lists = None;
    }
}
