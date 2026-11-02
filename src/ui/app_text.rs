//! Text documents in the application (C-05): opening an interface file
//! (MCM, INI, JSON…) in a text format — asked for when the name patterns
//! do not decide —, the target file pre-filling the translations, the
//! writing of the target file by Enregistrer sous… and Exporter la
//! traduction…, and the interface files of a plugin.

use std::path::PathBuf;
use std::sync::Arc;

use super::{Doc, XimodApp};
use crate::doc_settings::DocSettings;
use crate::games::Game;
use crate::status::Status;
use crate::textfmt::{Formats, TextDoc, TextFormat};
use crate::tm::TranslationMemory;

impl XimodApp {
    /// Open a text file: in the format its name designates, or with the
    /// import dialog (R-C05-02, R-D08-06).
    pub(crate) fn open_text_file(&mut self, path: PathBuf, settings: Option<DocSettings>) -> bool {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                self.notify_err(self.i18n.t_arg("err-open", "error", &e.to_string()));
                return false;
            }
        };
        let formats = Formats::load();
        let cands = formats.candidates(&path, &data);
        if cands.is_empty() {
            self.notify_err(self.i18n.t("text-no-format"));
            return false;
        }
        if let Some(idx) = formats.pick_for(&path, &cands) {
            let fmt = formats.formats[idx].clone();
            return self.open_text_with(path, &fmt, settings);
        }
        self.open_import_dialog(path, settings, true)
    }

    /// Fichier › Importer avec un format… (D-08).
    pub(crate) fn import_with_format(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                self.i18n.t("open-filter-text"),
                &["txt", "ini", "json", "xml", "csv", "tsv", "cfg"],
            )
            .add_filter(self.i18n.t("dialog-all-files"), &["*"])
            .pick_file();
        if let Some(p) = picked {
            self.open_import_dialog(p, None, false);
        }
    }

    /// Open a text file in a given format (R-C05-02, R-C05-03).
    pub(crate) fn open_text_with(
        &mut self,
        path: PathBuf,
        fmt: &TextFormat,
        settings: Option<DocSettings>,
    ) -> bool {
        // A game in a page of code reads its text files in the page of
        // their language (D-09), the format's own setting first.
        let codec = settings
            .as_ref()
            .filter(|s| super::app_encoding::page_game(s))
            .map(|s| crate::tm::encoding::Codec {
                page: self.pages().for_language(&s.source),
                strict: false,
            });
        let text = match crate::tm::encoding::with_input(codec, || TextDoc::read(&path, fmt)) {
            Ok(t) => t,
            Err(e) => {
                self.notify_err(self.i18n.t_arg("err-open", "error", &e.to_string()));
                return false;
            }
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let mut settings = settings
            .unwrap_or_else(|| DocSettings::for_new_doc(Game::Unknown, &self.idle, &self.config));
        settings.output = Some(crate::config::OutputMode::Text);
        // The languages of the sibling files, when the name carries one;
        // the toolbar's otherwise.
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let named_lang = if fmt.lang_in_name == "suffix" {
            crate::textfmt::lang_of_stem(&stem)
                .and_then(|(_, tok)| crate::textfmt::iso3_of_token(&tok))
        } else {
            None
        };
        let mut source_langs: Vec<String> = Vec::new();
        if named_lang.is_some() {
            source_langs = crate::textfmt::siblings(&path)
                .into_iter()
                .map(|(l, _)| l)
                .collect();
            source_langs.sort();
            source_langs.dedup();
        }
        let tm = text.to_tm(fmt, &name, &settings.game);
        match &named_lang {
            // This file's own language is the source.
            Some(l) => {
                if !source_langs.contains(l) {
                    source_langs.insert(0, l.clone());
                }
                settings.source = l.clone();
            }
            // Without a language in the name: the toolbar's, else the
            // languages detected in the text, as for a base.
            None => {
                let own = settings.source.trim().to_string();
                if own.is_empty() {
                    source_langs = Self::source_languages_of(&path, &tm);
                    if let Some(first) = source_langs.first() {
                        settings.source = first.clone();
                    }
                } else {
                    source_langs.push(own);
                }
            }
        }
        let mut doc = Doc::new(path.clone(), tm, None, settings, self.config.undo_depth);
        doc.source_langs = source_langs;
        doc.text = Some(Arc::new(text));
        doc.text_fmt = Some(fmt.clone());
        // The target file, when it exists, gives its values at Validé
        // (provenance « fichier cible »).
        let target_note = self.text_fill_from_target(&mut doc, fmt);
        doc.mark_saved();
        let head = self.text_status_head(&doc, fmt, &target_note);
        self.finish_open(doc, head)
    }

    /// Fill the empty lines of a text document from its target file when
    /// it exists (R-C05-03): its values at Validé, provenance « fichier
    /// cible »; the keys it has and the source has not are counted and
    /// ignored. Returns the note for the status bar.
    pub(crate) fn text_fill_from_target(&self, doc: &mut Doc, fmt: &TextFormat) -> String {
        let target = self.text_target_of(doc, fmt);
        match &target {
            Some(tp) if tp.is_file() && *tp != doc.path => match crate::tm::encoding::with_input(
                super::app_encoding::page_game(&doc.settings).then(|| crate::tm::encoding::Codec {
                    page: self.pages().for_language(&doc.settings.target),
                    strict: false,
                }),
                || TextDoc::read(tp, fmt),
            ) {
                Ok(tdoc) => {
                    let mut values = tdoc.values_by_key();
                    let mut taken = 0usize;
                    let file = tp
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // An operation like the dictionaries' (undoable, the
                    // file stays « non modifié » after the open).
                    crate::undo::record(
                        &mut doc.undo,
                        &mut doc.tm.entries,
                        crate::undo::OpKind::ApplyBase,
                        Some(file.clone()),
                        |entries| {
                            for e in entries.iter_mut() {
                                let Some(list) = values.get_mut(&e.edid) else {
                                    continue;
                                };
                                let k = (e.index.max(1) - 1) as usize;
                                if let Some(v) = list.get(k) {
                                    if !v.is_empty() && e.trans.is_empty() {
                                        e.trans = v.clone();
                                        e.prov = crate::provenance::TARGET.to_string();
                                        crate::status::set(e, Status::Validated);
                                        taken += 1;
                                    }
                                    list[k] = String::new();
                                }
                            }
                        },
                    );
                    let foreign = values
                        .values()
                        .map(|l| l.iter().filter(|v| !v.is_empty()).count())
                        .sum::<usize>();
                    let mut note = self.i18n.t_args(
                        "text-target-found",
                        &[("file", &file), ("n", &taken.to_string())],
                    );
                    if foreign > 0 {
                        note.push_str(&format!(
                            " · {}",
                            self.i18n
                                .t_arg("text-target-foreign", "n", &foreign.to_string())
                        ));
                    }
                    note
                }
                Err(_) => self.i18n.t_arg(
                    "text-target-none",
                    "file",
                    &tp.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                ),
            },
            Some(tp) => self.i18n.t_arg(
                "text-target-none",
                "file",
                &tp.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
            None => self.i18n.t("text-target-unnamed"),
        }
    }

    /// The status line of a text document (R-C05-02).
    pub(crate) fn text_status_head(
        &self,
        doc: &Doc,
        fmt: &TextFormat,
        target_note: &str,
    ) -> String {
        let keys = doc.text.as_ref().map(|t| t.keys()).unwrap_or(0);
        let done = doc.tm.entries.iter().filter(|e| e.is_translated()).count();
        self.i18n.t_args(
            "text-status",
            &[
                ("format", &fmt.name),
                ("keys", &keys.to_string()),
                ("done", &done.to_string()),
                ("target", target_note),
            ],
        )
    }

    /// The target language changed on a text document (R-C05-03): the new
    /// target file pre-fills what is still empty, before the dictionaries.
    pub(crate) fn text_target_changed(&mut self, idx: usize) -> Option<String> {
        let fmt = self.docs.get(idx)?.text_format()?;
        let mut doc = self.docs.remove(idx);
        let note = self.text_fill_from_target(&mut doc, &fmt);
        let head = self.text_status_head(&doc, &fmt, &note);
        self.docs.insert(idx, doc);
        Some(head)
    }

    /// The target file of a text document (R-C05-04): `<base>_<langue
    /// cible>.<ext>` when the name carries a language, else the file
    /// itself (written in place).
    pub(crate) fn text_target_of(&self, doc: &Doc, fmt: &TextFormat) -> Option<PathBuf> {
        if doc.settings.target.trim().is_empty() {
            return None;
        }
        Some(super::import_dialog::target_file(
            &doc.path,
            fmt,
            doc.settings.game(),
            &doc.settings.target,
        ))
    }

    /// Write a text document's target file (R-C05-04) from the given
    /// lines; returns the number of lines written.
    pub(crate) fn write_text_doc(
        &mut self,
        idx: usize,
        tm: &TranslationMemory,
        out: &std::path::Path,
    ) -> Result<usize, String> {
        let doc = self
            .docs
            .get(idx)
            .ok_or_else(|| crate::l10n!("err-no-document"))?;
        let text = doc
            .text
            .clone()
            .ok_or_else(|| crate::l10n!("err-not-text-doc"))?;
        let fmt = doc
            .text_format()
            .ok_or_else(|| crate::l10n!("err-no-text-format"))?;
        // A structured file changed since it was read: its positions no
        // longer hold (R-D08-04).
        if text.structured.is_some() && text.changed_since(&doc.path) {
            return Err(crate::l10n!("export-changed-outside"));
        }
        let copy = self.config.export_untranslated_copy;
        let bytes = text.write(&fmt, &tm.entries, copy);
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        crate::config::paths::write_atomic(out, &bytes).map_err(|e| e.to_string())?;
        let n = tm
            .entries
            .iter()
            .filter(|e| !e.trans.is_empty() || copy)
            .count();
        Ok(n)
    }

    /// The Source list on a text document (R-C05-03): open the sibling
    /// file of the chosen language in place of the document. `false` when
    /// nothing changed (no such file, or unsaved work to keep).
    pub(crate) fn switch_text_source(&mut self, idx: usize, source: &str) -> bool {
        let Some(doc) = self.docs.get(idx) else {
            return false;
        };
        let Some(fmt) = doc.text_format() else {
            return false;
        };
        let Some((_, sibling)) = crate::textfmt::siblings(&doc.path)
            .into_iter()
            .find(|(l, _)| l == source)
        else {
            return false;
        };
        if sibling == doc.path {
            return false;
        }
        if doc.is_dirty() {
            self.notify_warn(self.i18n.t("text-source-unsaved"));
            return false;
        }
        let mut settings = doc.settings.clone();
        settings.source = source.to_string();
        self.docs.remove(idx);
        if self.active >= self.docs.len() {
            self.active = self.docs.len().saturating_sub(1);
        }
        self.open_text_with(sibling, &fmt, Some(settings))
    }

    /// Fichier ▸ Ouvrir les fichiers d'interface du mod (R-C05-05).
    pub(crate) fn open_interface_files(&mut self) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        // The new tab takes the plugin's game and languages.
        let mut settings = doc.settings.clone();
        settings.output = Some(crate::config::OutputMode::Text);
        let files: Vec<PathBuf> = doc.interface_files.clone();
        for p in files {
            self.load_path_with(p, Some(settings.clone()));
        }
    }
}
