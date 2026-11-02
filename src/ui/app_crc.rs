//! Reconnaissance du dictionnaire du mod (B-12): the identity of the plugin
//! checked at opening against the header of its mod's dictionary — another
//! version asks what to do, a renamed plugin finds its dictionary back.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui;

use super::{Doc, XimodApp};
use crate::dict_stack::DictStack;
use crate::plugin_id::Ident;

/// What the opening asks.
pub(crate) enum CrcAsk {
    /// The dictionary was written for another version of the plugin.
    OtherVersion {
        dict: PathBuf,
        dict_id: Ident,
        changed: usize,
    },
    /// No dictionary under the plugin's name, but one written for the same
    /// file under another name.
    Renamed { old: PathBuf, old_id: Ident },
}

/// Who waits for the answer: a document being opened (it joins the tabs
/// after), or an open one whose target was just chosen (its dictionaries
/// apply after).
pub(crate) enum Who {
    Opening { doc: Box<Doc>, head: String },
    Open(usize),
}

pub(crate) struct CrcPrompt {
    who: Who,
    ask: CrcAsk,
}

/// The outcome of the check.
enum Check {
    /// Nothing to say (no dictionary, no identity, option off…).
    Nothing,
    /// Same size and CRC-32: « dictionnaire du mod reconnu ».
    Recognized,
    Ask(CrcAsk),
}

/// The state of the session (R-B12-02/03).
#[derive(Default)]
pub(crate) struct CrcState {
    pub prompt: Option<CrcPrompt>,
    /// (plugin, CRC of the dictionary) already asked about.
    seen: HashSet<(PathBuf, u32)>,
    /// Mod dictionaries set aside for the session (« Ouvrir sans »).
    pub off: HashSet<PathBuf>,
    /// The document whose set-aside dictionary is about to be rewritten.
    pub rewrite_ask: Option<usize>,
    /// Set-aside dictionaries the user chose not to rewrite.
    skip: HashSet<PathBuf>,
}

impl CrcState {
    pub fn is_open(&self) -> bool {
        self.prompt.is_some() || self.rewrite_ask.is_some()
    }
}

impl XimodApp {
    /// The identity of an opened plugin or text file (R-B12-01).
    pub(crate) fn ident_of(doc: &Doc) -> Option<Ident> {
        if doc.tm.meta.source_format != "plugin" && !doc.is_text() {
            return None;
        }
        crate::slow_io("plugin crc", || crate::plugin_id::of_file(&doc.path)).ok()
    }

    /// The check of a document's identity against its mod's dictionary
    /// (R-B12-02/04/05).
    fn crc_check(
        &mut self,
        path: &std::path::Path,
        settings: &crate::doc_settings::DocSettings,
        ident: Option<&Ident>,
        entries: &[crate::tm::TmEntry],
    ) -> Check {
        let Some(ident) = ident.cloned() else {
            return Check::Nothing;
        };
        if !self.config.check_crc || settings.target.trim().is_empty() {
            return Check::Nothing;
        }
        let dict = DictStack::mod_dict_path(path, settings, &self.config.mod_base_dir());
        if self.crc.off.contains(&dict) {
            return Check::Nothing;
        }
        if dict.is_file() {
            let Some(dict_id) = crate::basedb::peek_ident(&dict) else {
                // Written before B-12: completed at the next write.
                return Check::Nothing;
            };
            if dict_id.same_content(&ident) {
                return Check::Recognized;
            }
            if !self.crc.seen.insert((path.to_path_buf(), dict_id.crc)) {
                return Check::Nothing;
            }
            tracing::info!(
                "mod dictionary {} written for {} {:08X}, plugin is {} {:08X}",
                dict.display(),
                dict_id.size,
                dict_id.crc,
                ident.size,
                ident.crc
            );
            let changed = self.changed_since_dict(path, settings, entries);
            return Check::Ask(CrcAsk::OtherVersion {
                dict,
                dict_id,
                changed,
            });
        }
        // A renamed plugin: a dictionary of the same pair written for the
        // same file under another name.
        match Self::dict_written_for(&dict, &ident) {
            Some((old, old_id)) if self.crc.seen.insert((path.to_path_buf(), old_id.crc)) => {
                tracing::info!("renamed plugin: dictionary {} matches", old.display());
                Check::Ask(CrcAsk::Renamed { old, old_id })
            }
            _ => Check::Nothing,
        }
    }

    /// At opening: `Some` to go on now, with the status head completed;
    /// `None` when a question waits.
    pub(crate) fn crc_on_open(&mut self, doc: Doc, head: String) -> Option<(Doc, String)> {
        let check = self.crc_check(
            &doc.path,
            &doc.settings,
            doc.ident.as_ref(),
            &doc.tm.entries,
        );
        match check {
            Check::Nothing => Some((doc, head)),
            Check::Recognized => {
                let head = self.i18n.dash(&head, &self.i18n.t("crc-recognized"));
                Some((doc, head))
            }
            Check::Ask(ask) => {
                self.crc.prompt = Some(CrcPrompt {
                    who: Who::Opening {
                        doc: Box::new(doc),
                        head,
                    },
                    ask,
                });
                None
            }
        }
    }

    /// When the target of an open document is chosen (its mod's dictionary
    /// is known only then): `Err` when a question waits, the dictionaries
    /// applying after the answer; otherwise the note for the status bar.
    pub(crate) fn crc_on_target(&mut self, idx: usize) -> Result<Option<String>, ()> {
        let Some(doc) = self.docs.get(idx) else {
            return Ok(None);
        };
        let (path, settings, ident, entries) = (
            doc.path.clone(),
            doc.settings.clone(),
            doc.ident.clone(),
            doc.tm.entries.clone(),
        );
        match self.crc_check(&path, &settings, ident.as_ref(), &entries) {
            Check::Nothing => Ok(None),
            Check::Recognized => Ok(Some(self.i18n.t("crc-recognized"))),
            Check::Ask(ask) => {
                self.crc.prompt = Some(CrcPrompt {
                    who: Who::Open(idx),
                    ask,
                });
                Err(())
            }
        }
    }

    /// A dictionary next to `dict` (same pair suffix) whose header names
    /// the content `ident`.
    fn dict_written_for(dict: &std::path::Path, ident: &Ident) -> Option<(PathBuf, Ident)> {
        let dir = dict.parent()?;
        let name = dict.file_name()?.to_str()?;
        // « <plugin>_<SRC-DST>.ximdb »: the suffix after the last « _ ».
        let suffix = &name[name.rfind('_')?..];
        let mut hits: Vec<(PathBuf, Ident)> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(suffix))
            })
            .filter_map(|p| crate::basedb::peek_ident(&p).map(|i| (p, i)))
            .filter(|(_, i)| i.same_content(ident))
            .collect();
        // The most recent write first.
        hits.sort_by_key(|(_, i)| std::cmp::Reverse(i.written));
        hits.into_iter().next()
    }

    /// The document of a prompt (its name, identity, path and settings).
    fn crc_doc<'a>(&'a self, who: &'a Who) -> Option<&'a Doc> {
        match who {
            Who::Opening { doc, .. } => Some(doc),
            Who::Open(i) => self.docs.get(*i),
        }
    }

    /// Go on after the answer, with a note for the status bar.
    fn crc_go_on(&mut self, who: Who, note: Option<String>) -> bool {
        match who {
            Who::Opening { doc, head } => {
                let head = match note {
                    Some(n) => self.i18n.dash(&head, &n),
                    None => head,
                };
                self.finish_open_now(*doc, head)
            }
            Who::Open(idx) => {
                if idx >= self.docs.len() {
                    return false;
                }
                self.active = idx;
                self.restack_doc(idx);
                if let Some(n) = note {
                    self.status = self.i18n.dash(&self.status, &n);
                }
                true
            }
        }
    }

    /// The question, each frame while it waits.
    pub(crate) fn crc_window(&mut self, ctx: &egui::Context) {
        self.crc_rewrite_window(ctx);
        let Some(prompt) = self.crc.prompt.as_ref() else {
            return;
        };
        let Some(doc) = self.crc_doc(&prompt.who) else {
            self.crc.prompt = None;
            return;
        };
        let title = self.i18n.t_arg("crc-title", "doc", &doc.file_name());
        let fmt = |n: u64| crate::ui::export_dialog::fmt_count(&self.i18n, n as usize);
        let ident = doc.ident.clone().unwrap_or_default();
        #[derive(PartialEq)]
        enum Pick {
            Continue,
            Update,
            Without,
            RenameYes,
            RenameNo,
        }
        let mut pick: Option<Pick> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                match &prompt.ask {
                    CrcAsk::OtherVersion {
                        dict,
                        dict_id,
                        changed,
                    } => {
                        let name = dict
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.label(self.i18n.t_arg("crc-other-version", "dict", &name));
                        let date = if dict_id.written > 0 {
                            crate::backup::display_datetime(&self.i18n, dict_id.written)
                        } else {
                            "—".to_string()
                        };
                        ui.label(
                            egui::RichText::new(self.i18n.t_args(
                                "crc-details",
                                &[
                                    ("size", &fmt(ident.size)),
                                    ("crc", &ident.crc_hex()),
                                    ("dsize", &fmt(dict_id.size)),
                                    ("dcrc", &dict_id.crc_hex()),
                                    ("date", &date),
                                ],
                            ))
                            .weak()
                            .small(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            self.i18n
                                .t_arg("crc-changed-lines", "n", &fmt(*changed as u64)),
                        );
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(egui::RichText::new(self.i18n.t("crc-continue")).strong())
                                .clicked()
                                || esc
                            {
                                pick = Some(Pick::Continue);
                            }
                            if ui.button(self.i18n.t("crc-update")).clicked() {
                                pick = Some(Pick::Update);
                            }
                            if ui.button(self.i18n.t("crc-without")).clicked() {
                                pick = Some(Pick::Without);
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(self.i18n.t("crc-once")).weak().small());
                    }
                    CrcAsk::Renamed { old, old_id } => {
                        let dict = old
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        ui.label(self.i18n.t_args(
                            "crc-renamed-ask",
                            &[("old", &old_id.plugin), ("dict", &dict)],
                        ));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(egui::RichText::new(self.i18n.t("btn-yes")).strong())
                                .clicked()
                            {
                                pick = Some(Pick::RenameYes);
                            }
                            if ui.button(self.i18n.t("btn-no")).clicked() || esc {
                                pick = Some(Pick::RenameNo);
                            }
                        });
                    }
                }
            });
        let Some(pick) = pick else {
            return;
        };
        let Some(CrcPrompt { who, ask }) = self.crc.prompt.take() else {
            return;
        };
        match (pick, ask) {
            (Pick::Without, CrcAsk::OtherVersion { dict, .. }) => {
                self.crc.off.insert(dict);
                let note = self.i18n.t("crc-without-done");
                self.crc_go_on(who, Some(note));
            }
            (Pick::Update, _) => {
                if self.crc_go_on(who, None) {
                    self.open_update_window();
                }
            }
            (Pick::RenameYes, CrcAsk::Renamed { old, old_id }) => {
                let new = self.crc_doc(&who).map(|d| {
                    DictStack::mod_dict_path(&d.path, &d.settings, &self.config.mod_base_dir())
                });
                let res = match &new {
                    Some(new) => crate::backup::rename_with_companions(&old, new),
                    None => Ok(()),
                };
                match (res, new) {
                    (Ok(()), Some(new)) => {
                        self.invalidate_dict(&old);
                        self.invalidate_dict(&new);
                        let new_name = new
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let note = self.i18n.t_arg("crc-renamed-found", "old", &old_id.plugin);
                        if self.crc_go_on(who, Some(note)) {
                            self.notify_ok(self.i18n.t_arg("crc-renamed-done", "new", &new_name));
                        }
                    }
                    (Err(e), _) => {
                        self.crc_go_on(who, None);
                        self.notify_err(self.i18n.t_arg(
                            "crc-renamed-error",
                            "error",
                            &e.to_string(),
                        ));
                    }
                    (Ok(()), None) => {
                        self.crc_go_on(who, None);
                    }
                }
            }
            (_, _) => {
                self.crc_go_on(who, None);
            }
        }
    }

    /// Before writing a mod dictionary set aside at opening (R-B12-03):
    /// `true` when the write may go on.
    pub(crate) fn crc_may_write(&mut self, idx: usize, dict: &std::path::Path) -> bool {
        if !self.crc.off.contains(dict) {
            return true;
        }
        if !self.crc.skip.contains(dict) && self.crc.rewrite_ask.is_none() {
            self.crc.rewrite_ask = Some(idx);
        }
        false
    }

    /// « Réécrire le dictionnaire écarté ? ».
    fn crc_rewrite_window(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.crc.rewrite_ask else {
            return;
        };
        let Some(doc) = self.docs.get(idx) else {
            self.crc.rewrite_ask = None;
            return;
        };
        let dict = DictStack::mod_dict_path(&doc.path, &doc.settings, &self.config.mod_base_dir());
        let name = dict
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut answer: Option<bool> = None;
        egui::Window::new(self.i18n.t_arg("crc-title", "doc", &doc.file_name()))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.set_max_width(480.0);
                ui.label(self.i18n.t_arg("crc-rewrite-ask", "dict", &name));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(self.i18n.t("btn-yes")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(self.i18n.t("btn-no")).clicked()
                        || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                self.crc.rewrite_ask = None;
                self.crc.off.remove(&dict);
                self.write_mod_dict(idx, true);
            }
            Some(false) => {
                self.crc.rewrite_ask = None;
                self.crc.skip.insert(dict);
            }
            None => {}
        }
    }
}
