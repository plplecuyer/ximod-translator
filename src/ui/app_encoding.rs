//! Pages de code (D-09): the page a document is read and written in — UTF-8
//! for the 64-bit games, otherwise the page of its languages, detected
//! when the source language is unknown, forced by Fichier ▸ Encodage du
//! fichier… and kept in the mod's dictionary — and the dialog itself.

use std::path::Path;

use eframe::egui;

use super::{Doc, XimodApp};
use crate::codepage::{self, Choice, Page, Pages, Reason};
use crate::doc_settings::DocSettings;
use crate::tm::TranslationMemory;
use crate::tm::encoding::Codec;

/// What a document knows of its encoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EncState {
    /// How the file was read (R-D09-02); `None` before the first reading.
    pub used: Option<Choice>,
    /// The pages the user forced (R-D09-04).
    pub forced_in: Option<Page>,
    pub forced_out: Option<Page>,
}

impl EncState {
    /// The codec a new reading of the file must use: the forced page
    /// (strict), else the page found the first time (after the UTF-8 test).
    pub fn codec(&self) -> Option<Codec> {
        if let Some(p) = self.forced_in {
            return Some(Codec {
                page: p,
                strict: true,
            });
        }
        match &self.used {
            Some(c) if c.page != Page::Utf8 => Some(Codec {
                page: c.page,
                strict: false,
            }),
            _ => None,
        }
    }

    /// The page the file was read in, for the preview and the chip.
    pub fn read_page(&self) -> Option<Page> {
        self.forced_in.or(self.used.as_ref().map(|c| c.page))
    }
}

/// The dialog Fichier ▸ Encodage du fichier… (figure B.1).
pub(crate) struct EncodingDialog {
    /// `None` = automatic.
    read: Option<Page>,
    write: Option<Page>,
    /// Asked to re-read the file with the page of `read`.
    reload: bool,
}

/// Whether a game stores its text in a page of code (the older engines).
pub(crate) fn page_game(settings: &DocSettings) -> bool {
    let g = settings.game();
    g != crate::games::Game::Unknown && !g.uses_utf8()
}

impl XimodApp {
    pub(crate) fn pages(&self) -> &'static Pages {
        Pages::builtin()
    }

    /// The page a document is written in (R-D09-03): UTF-8 for the 64-bit
    /// games and the loose files, else the forced page or the one of the
    /// target language.
    pub(crate) fn out_page(&self, doc: &Doc) -> Page {
        if !page_game(&doc.settings) || doc.tm.meta.source_format != "plugin" && !doc.is_text() {
            return Page::Utf8;
        }
        doc.enc
            .forced_out
            .unwrap_or_else(|| self.pages().for_language(&doc.settings.target))
    }

    /// The page a document would be written in by default (automatic).
    pub(crate) fn auto_out_page(&self, settings: &DocSettings) -> Page {
        if !page_game(settings) {
            Page::Utf8
        } else {
            self.pages().for_language(&settings.target)
        }
    }

    /// Decide the page of a plugin just read (R-D09-02) and re-read it when
    /// another page than the fallback applies: the forced page of the mod's
    /// dictionary, else the page of the declared source language, else the
    /// detection, else Windows-1252. `declared` says whether `settings.source`
    /// was chosen for this file (a resume) rather than a session default.
    pub(crate) fn encoding_resolve(
        &mut self,
        path: &Path,
        tm: &mut TranslationMemory,
        settings: &DocSettings,
        declared: bool,
    ) -> EncState {
        let mut st = EncState::default();
        if !page_game(settings) {
            st.used = Some(Choice {
                page: Page::Utf8,
                reason: Reason::Utf8,
                language: None,
            });
            return st;
        }
        // The pages the mod's dictionary remembers (R-D09-04).
        let dict = crate::dict_stack::DictStack::mod_dict_path(
            path,
            settings,
            &self.config.mod_base_dir(),
        );
        let extra = if dict.is_file() {
            crate::basedb::peek_extra(&dict)
        } else {
            Default::default()
        };
        st.forced_in = extra.enc_in.as_deref().and_then(Page::from_id);
        st.forced_out = extra.enc_out.as_deref().and_then(Page::from_id);
        if tm.meta.enc_in.is_empty() {
            tm.meta.enc_in = st.forced_in.map(|p| p.id().to_string()).unwrap_or_default();
        } else if st.forced_in.is_none() {
            st.forced_in = Page::from_id(&tm.meta.enc_in);
        }
        if tm.meta.enc_out.is_empty() {
            tm.meta.enc_out = st
                .forced_out
                .map(|p| p.id().to_string())
                .unwrap_or_default();
        } else if st.forced_out.is_none() {
            st.forced_out = Page::from_id(&tm.meta.enc_out);
        }
        let game = settings.game();
        let reread = |page: Page, strict: bool| -> Option<TranslationMemory> {
            if tm.meta.source_format != "plugin" {
                return None;
            }
            crate::plugin::read_file_with(path, Some(game), Some(Codec { page, strict })).ok()
        };
        if let Some(p) = st.forced_in {
            if let Some(t) = reread(p, true) {
                let (ei, eo) = (tm.meta.enc_in.clone(), tm.meta.enc_out.clone());
                *tm = t;
                tm.meta.enc_in = ei;
                tm.meta.enc_out = eo;
            }
            st.used = Some(Choice {
                page: p,
                reason: Reason::Forced,
                language: None,
            });
            return st;
        }
        // The texts the Windows-1252 fallback decoded.
        let fallback: Vec<&str> = tm
            .entries
            .iter()
            .map(|e| e.source.as_str())
            .filter(|s| s.chars().any(|c| c as u32 >= 0x80))
            .filter(|s| codepage::from_win1252(s))
            .collect();
        let any_high = tm
            .entries
            .iter()
            .any(|e| e.source.chars().any(|c| c as u32 >= 0x80));
        if fallback.is_empty() {
            st.used = Some(if any_high {
                Choice {
                    page: Page::Utf8,
                    reason: Reason::Utf8,
                    language: None,
                }
            } else {
                Choice {
                    page: Page::Win1252,
                    reason: Reason::Default,
                    language: None,
                }
            });
            return st;
        }
        let (page, reason, language) = if declared && !settings.source.trim().is_empty() {
            (
                self.pages().for_language(&settings.source),
                Reason::SourceLanguage,
                None,
            )
        } else {
            let start = std::time::Instant::now();
            let got = codepage::detect(&fallback, self.pages());
            tracing::debug!(
                "encoding: detection on {} texts in {:?}: {:?}",
                fallback.len(),
                start.elapsed(),
                got
            );
            match got {
                Some((p, lang, _)) => (p, Reason::Detected, Some(lang)),
                None => (Page::Win1252, Reason::Default, None),
            }
        };
        if page != Page::Win1252
            && let Some(t) = reread(page, false)
        {
            let (ei, eo) = (tm.meta.enc_in.clone(), tm.meta.enc_out.clone());
            *tm = t;
            tm.meta.enc_in = ei;
            tm.meta.enc_out = eo;
        }
        st.used = Some(Choice {
            page,
            reason,
            language,
        });
        st
    }

    /// « Lu en Cyrillique (Windows-1251) (détecté) » for the status bar, for
    /// a page worth saying (not the plain default).
    pub(crate) fn encoding_note(&self, st: &EncState) -> Option<String> {
        let c = st.used.as_ref()?;
        if matches!(c.reason, Reason::Utf8 | Reason::Default) {
            return None;
        }
        Some(self.i18n.t_args(
            "status-read-encoding",
            &[
                ("page", &self.i18n.t(c.page.name_key())),
                ("reason", &self.i18n.t(c.reason.key())),
            ],
        ))
    }

    /// The short mark of the tab badge (« 1251 ») when a page other than
    /// the game's default applies, reading or writing.
    pub(crate) fn encoding_chip(&self, doc: &Doc) -> Option<String> {
        if !page_game(&doc.settings) {
            return None;
        }
        let read = doc.enc.read_page().filter(|p| *p != Page::Win1252);
        let write = Some(self.out_page(doc)).filter(|p| *p != Page::Win1252);
        match (read, write) {
            (None, None) => None,
            (Some(r), Some(w)) if r == w => Some(r.short().to_string()),
            (r, w) => Some(format!(
                "{}{}{}",
                r.unwrap_or(Page::Win1252).short(),
                crate::icons::ARROW_RIGHT_CHAR,
                w.unwrap_or(Page::Win1252).short()
            )),
        }
    }

    /// The lines of `tm` whose translation the page cannot hold (R-D09-05).
    pub(crate) fn unencodable_lines(tm: &TranslationMemory, page: Page) -> usize {
        if page == Page::Utf8 {
            return 0;
        }
        tm.entries
            .iter()
            .filter(|e| !e.unused && e.is_translated() && !page.can_encode(&e.trans))
            .count()
    }

    /// Their indexes, for « Voir les lignes » of the journal (E-01).
    pub(crate) fn unencodable_rows(tm: &TranslationMemory, page: Page) -> Vec<usize> {
        if page == Page::Utf8 {
            return Vec::new();
        }
        tm.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.unused && e.is_translated() && !page.can_encode(&e.trans))
            .map(|(i, _)| i)
            .collect()
    }

    /// Fichier ▸ Encodage du fichier… : open the dialog on the active
    /// document.
    pub(crate) fn open_encoding_dialog(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if doc.all.is_some() {
            self.status = self.i18n.t("all-mods-file-only");
            return;
        }
        if !page_game(&doc.settings) {
            self.status = self.i18n.t("encoding-utf8-game");
            return;
        }
        self.encoding_dlg = Some(EncodingDialog {
            read: doc.enc.forced_in,
            write: doc.enc.forced_out,
            reload: false,
        });
    }

    /// The first lines with characters above ASCII, as `page` reads their
    /// bytes (the preview, R-D09-04): the bytes come back from the page the
    /// document was read in (lossless for a single-byte page).
    fn encoding_preview(&self, doc: &Doc, page: Page) -> Vec<(String, String)> {
        let from = doc.enc.read_page().unwrap_or(Page::Win1252);
        doc.tm
            .entries
            .iter()
            .filter(|e| !e.unused && e.source.chars().any(|c| c as u32 >= 0x80))
            .take(4)
            .map(|e| {
                let bytes = codepage::recover(&e.source, from);
                (
                    format!("{}:{}", e.rec_sig, e.field_sig),
                    crate::ui::edit_window::one_line(&page.decode(&bytes), 80),
                )
            })
            .collect()
    }

    /// The modal dialog (figure B.1).
    pub(crate) fn encoding_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.encoding_dlg.take() else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let title = self.i18n.t_arg("encoding-title", "doc", &doc.file_name());
        let game_line = self
            .i18n
            .t_arg("encoding-game-page", "game", &doc.settings.game_label());
        let auto_in = {
            let (page, reason) = match &doc.enc.used {
                Some(c) if c.reason != Reason::Forced => (c.page, c.reason),
                _ => (
                    self.pages().for_language(&doc.settings.source),
                    Reason::SourceLanguage,
                ),
            };
            self.i18n.t_args(
                match reason {
                    Reason::Detected => "encoding-auto-detected",
                    Reason::Utf8 => "encoding-auto-utf8",
                    Reason::Default => "encoding-auto-default",
                    _ => "encoding-auto-source",
                },
                &[("page", &self.i18n.t(page.name_key()))],
            )
        };
        let auto_out = self.i18n.t_arg(
            "encoding-auto-target",
            "page",
            &self.i18n.t(self.auto_out_page(&doc.settings).name_key()),
        );
        let read_page = dlg.read.or(doc.enc.read_page()).unwrap_or(Page::Win1252);
        let preview = self.encoding_preview(doc, read_page);
        let write_page = dlg
            .write
            .unwrap_or_else(|| self.auto_out_page(&doc.settings));
        let bad = Self::unencodable_lines(&doc.tm, write_page);
        let dirty = doc.is_dirty();
        let labels = (
            self.i18n.t("encoding-read"),
            self.i18n.t("encoding-write"),
            self.i18n.t("encoding-reload"),
            self.i18n.t("encoding-preview"),
            self.i18n.t("encoding-preview-none"),
            self.i18n.t("encoding-remembered"),
            self.i18n.t("btn-ok"),
            self.i18n.t("btn-cancel"),
        );
        let bad_line = (bad > 0).then(|| {
            self.i18n.t_args(
                "encoding-bad-chars",
                &[
                    ("page", &self.i18n.t(write_page.name_key())),
                    ("n", &bad.to_string()),
                ],
            )
        });
        let names: Vec<(Option<Page>, String)> = std::iter::once((None, auto_in.clone()))
            .chain(
                Page::ALL
                    .iter()
                    .map(|p| (Some(*p), self.i18n.t(p.name_key()))),
            )
            .collect();
        let names_out: Vec<(Option<Page>, String)> = std::iter::once((None, auto_out.clone()))
            .chain(
                Page::ALL
                    .iter()
                    .map(|p| (Some(*p), self.i18n.t(p.name_key()))),
            )
            .collect();
        let mut action: Option<bool> = None;
        let error_c = ctx.style().visuals.error_fg_color;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&game_line).weak().small());
                ui.add_space(6.0);
                egui::Grid::new("ximod-encoding-grid")
                    .num_columns(3)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong(&labels.0);
                        let cur = names
                            .iter()
                            .find(|(p, _)| *p == dlg.read)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_default();
                        egui::ComboBox::from_id_salt("ximod-encoding-read")
                            .width(400.0)
                            .selected_text(cur)
                            .show_ui(ui, |ui| {
                                for (p, n) in &names {
                                    ui.selectable_value(&mut dlg.read, *p, n);
                                }
                            });
                        if ui
                            .add_enabled(dlg.read.is_some(), egui::Button::new(&labels.2))
                            .on_disabled_hover_text(self.i18n.t("encoding-reload-tip"))
                            .clicked()
                        {
                            dlg.reload = true;
                            action = Some(true);
                        }
                        ui.end_row();
                        ui.strong(&labels.1);
                        let cur = names_out
                            .iter()
                            .find(|(p, _)| *p == dlg.write)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_default();
                        egui::ComboBox::from_id_salt("ximod-encoding-write")
                            .width(400.0)
                            .selected_text(cur)
                            .show_ui(ui, |ui| {
                                for (p, n) in &names_out {
                                    ui.selectable_value(&mut dlg.write, *p, n);
                                }
                            });
                        ui.label("");
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&labels.3).weak().small());
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if preview.is_empty() {
                        ui.label(egui::RichText::new(&labels.4).weak());
                    }
                    for (rec, text) in &preview {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(rec).monospace().weak().small());
                            ui.add(egui::Label::new(text).truncate());
                        });
                    }
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&labels.5).weak().small());
                if let Some(b) = &bad_line {
                    ui.colored_label(error_c, b);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&labels.7).clicked() {
                            action = Some(false);
                        }
                        if ui.button(egui::RichText::new(&labels.6).strong()).clicked() {
                            action = Some(true);
                        }
                    });
                });
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                if enter {
                    action = Some(true);
                }
                if esc {
                    action = Some(false);
                }
            });
        match action {
            Some(true) => {
                let idx = self.active;
                let changed_in = self.docs[idx].enc.forced_in != dlg.read;
                self.docs[idx].enc.forced_in = dlg.read;
                self.docs[idx].enc.forced_out = dlg.write;
                self.docs[idx].tm.meta.enc_in =
                    dlg.read.map(|p| p.id().to_string()).unwrap_or_default();
                self.docs[idx].tm.meta.enc_out =
                    dlg.write.map(|p| p.id().to_string()).unwrap_or_default();
                if dlg.reload || (changed_in && dlg.read.is_some()) {
                    if dirty {
                        self.notify_warn(self.i18n.t("encoding-reload-dirty"));
                    } else {
                        self.encoding_reload(idx);
                    }
                }
                // The mod's dictionary keeps the choice (R-D09-04).
                self.write_mod_dict(idx, false);
            }
            Some(false) => {}
            None => self.encoding_dlg = Some(dlg),
        }
    }

    /// The target was chosen (R-D09-04): the mod's dictionary, known only
    /// now, may remember forced pages; a forced reading page re-reads the
    /// file when the document is still untouched.
    pub(crate) fn encoding_on_target(&mut self, idx: usize) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        if !page_game(&doc.settings) || doc.tm.meta.source_format != "plugin" {
            return;
        }
        let dict = crate::dict_stack::DictStack::mod_dict_path(
            &doc.path,
            &doc.settings,
            &self.config.mod_base_dir(),
        );
        if !dict.is_file() {
            return;
        }
        let extra = crate::basedb::peek_extra(&dict);
        let forced_in = extra.enc_in.as_deref().and_then(Page::from_id);
        let forced_out = extra.enc_out.as_deref().and_then(Page::from_id);
        let doc = &mut self.docs[idx];
        if let Some(p) = forced_out {
            doc.enc.forced_out = Some(p);
            doc.tm.meta.enc_out = p.id().to_string();
        }
        let Some(p) = forced_in else {
            return;
        };
        if doc.enc.forced_in == Some(p) {
            return;
        }
        doc.enc.forced_in = Some(p);
        doc.tm.meta.enc_in = p.id().to_string();
        if doc.is_dirty() || doc.enc.read_page() == Some(p) {
            doc.enc.used = Some(Choice {
                page: p,
                reason: Reason::Forced,
                language: None,
            });
            return;
        }
        self.encoding_reload(idx);
    }

    /// Relire: read the file again with the forced page (strict), the
    /// translations carried over by the layered matching of a reload.
    pub(crate) fn encoding_reload(&mut self, idx: usize) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let game = doc.settings.game.clone();
        let (page, reason) = match doc.enc.forced_in {
            Some(p) => (p, Reason::Forced),
            None => return,
        };
        self.reload_doc_as(idx, game);
        if let Some(d) = self.docs.get_mut(idx) {
            d.enc.used = Some(Choice {
                page,
                reason,
                language: None,
            });
        }
        let note = self.docs.get(idx).and_then(|d| self.encoding_note(&d.enc));
        if let Some(n) = note {
            self.status = self.i18n.dash(&self.status, &n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_of_a_state() {
        let mut st = EncState::default();
        assert_eq!(st.codec(), None);
        st.used = Some(Choice {
            page: Page::Win1251,
            reason: Reason::Detected,
            language: Some("rus".into()),
        });
        assert_eq!(
            st.codec(),
            Some(Codec {
                page: Page::Win1251,
                strict: false
            })
        );
        st.forced_in = Some(Page::Win1250);
        assert_eq!(
            st.codec(),
            Some(Codec {
                page: Page::Win1250,
                strict: true
            })
        );
        assert_eq!(st.read_page(), Some(Page::Win1250));
        let utf = EncState {
            used: Some(Choice {
                page: Page::Utf8,
                reason: Reason::Utf8,
                language: None,
            }),
            ..Default::default()
        };
        assert_eq!(utf.codec(), None);
    }
}
