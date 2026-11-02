//! Localiser un mod (C-09): the STRINGS output of a plugin whose texts are
//! inline — its label, what the export dialog says it will write, and the
//! writing of the localized plugin and its STRINGS.

use std::path::{Path, PathBuf};

use super::{Doc, XimodApp};
use crate::config::OutputMode;
use crate::games::Game;
use crate::lstrings::{self, Table};
use crate::tm::TranslationMemory;
use crate::tm::strings::StringsKind;

/// What the export dialog shows, computed once per document version and
/// export policy.
#[derive(Default)]
pub(crate) struct LocalizeCache {
    key: Option<String>,
    plan: Option<Plan>,
}

#[derive(Clone)]
struct Plan {
    ids: u32,
    per_list: [usize; 3],
    unknown: Vec<(String, usize)>,
    table: Table,
}

/// The lists of one language: (file kind, (identifier, text)).
type Lists = Vec<(StringsKind, Vec<(u32, String)>)>;

/// What the export dialog shows for a plugin to localize.
pub(crate) struct Lines {
    /// (file, what it holds).
    pub files: Vec<(String, String)>,
    pub table: String,
    pub unknown: Option<String>,
    pub source_label: String,
}

const KINDS: [StringsKind; 3] = [
    StringsKind::Strings,
    StringsKind::DlStrings,
    StringsKind::IlStrings,
];

impl XimodApp {
    /// The table of the localizable fields of a document that can be
    /// localized: a plugin whose texts are inline, of a game with STRINGS
    /// (R-C09-05). `None` otherwise.
    pub(crate) fn localize_table(&self, doc: &Doc) -> Option<Table> {
        if doc.tm.meta.source_format != "plugin" || doc.all.is_some() {
            return None;
        }
        if doc.strings.is_some() || doc.tm.entries.iter().any(|e| e.idstexte >= 0) {
            return None;
        }
        let game = doc.settings.game();
        if game == Game::Unknown || !crate::games::registry().spec(game).localized_strings {
            return None;
        }
        Table::load(game, &crate::config::paths::get().data_dir)
    }

    /// The label of an output mode for the active document: STRINGS reads
    /// « STRINGS (localiser le plugin) » on a plugin to localize.
    pub(crate) fn output_label(&self, m: OutputMode) -> String {
        if m == OutputMode::Strings
            && self
                .active_doc()
                .is_some_and(|d| self.localize_table(d).is_some())
        {
            return self.i18n.t("output-strings-localize");
        }
        self.i18n.t(super::output_mode_key(m))
    }

    /// The message of an output the game cannot receive.
    pub(crate) fn output_unfit_message(&self, m: OutputMode, game: Game) -> String {
        self.i18n.t_args(
            "status-output-not-for-game",
            &[
                ("output", &self.output_label(m)),
                ("game", game.display_name()),
            ],
        )
    }

    /// The output follows the game: a mode the game cannot receive (a
    /// file of Oblivion opened with the STRINGS of the last session, a game
    /// changed) empties the Sortie list, which the export then asks for.
    pub(crate) fn output_fit_game(&mut self) {
        let (game, output) = match self.active_doc() {
            Some(d) => (d.settings.game(), d.settings.output),
            None => (Game::from_id(self.idle.game.trim()), self.idle.output),
        };
        let Some(m) = output.filter(|m| !m.fits(game)) else {
            return;
        };
        self.status = self.output_unfit_message(m, game);
        match self.docs.get_mut(self.active) {
            Some(d) => d.settings.output = None,
            None => self.idle.output = None,
        }
    }

    /// The plan of the export dialog (figure B.2), recomputed when the
    /// document or the policy changes.
    fn localize_plan(&mut self, policy: &crate::export::ExportPolicy) -> Option<Plan> {
        let doc = self.active_doc()?;
        let table = self.localize_table(doc)?;
        let key = format!("{}:{}:{:?}", doc.serial, doc.undo.revision, policy.include);
        if self.localize_cache.key.as_deref() == Some(key.as_str()) {
            return self.localize_cache.plan.clone();
        }
        let game = doc.settings.game();
        let filtered = policy.filtered(&doc.tm);
        let plan = std::fs::read(&doc.path).ok().and_then(|src| {
            lstrings::localize(&src, &filtered, &table, game)
                .ok()
                .map(|out| Plan {
                    ids: out.report.ids,
                    per_list: out.report.per_list,
                    unknown: out.report.unknown,
                    table: table.clone(),
                })
        });
        self.localize_cache = LocalizeCache {
            key: Some(key),
            plan: plan.clone(),
        };
        plan
    }

    /// The lines of the export dialog for a plugin to localize: the files,
    /// the table, the unknown fields.
    pub(crate) fn localize_lines(&mut self, policy: &crate::export::ExportPolicy) -> Option<Lines> {
        let plan = self.localize_plan(policy)?;
        let doc = self.active_doc()?;
        let game = doc.settings.game();
        let stem = doc
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mod")
            .to_string();
        let dst = crate::lang::strings_suffix_for(game, &doc.settings.target);
        let src = crate::lang::strings_suffix_for(game, &doc.settings.source);
        let mut files = vec![(
            doc.file_name(),
            self.i18n
                .t_arg("localize-plugin-line", "n", &plan.ids.to_string()),
        )];
        let what = [
            "localize-strings-line",
            "localize-dlstrings-line",
            "localize-ilstrings-line",
        ];
        for (k, kind) in KINDS.iter().enumerate() {
            if plan.per_list[k] == 0 {
                continue;
            }
            files.push((
                format!("Strings/{stem}_{dst}.{}", kind.extension()),
                self.i18n.t_arg(what[k], "n", &plan.per_list[k].to_string()),
            ));
        }
        if self.config.localize_source_strings && src != dst {
            files.push((
                format!("Strings/{stem}_{src}.*"),
                self.i18n.t("localize-source-line"),
            ));
        }
        let table = if plan.table.derived_from.is_empty() {
            self.i18n
                .t_arg("localize-table-builtin", "game", game.display_name())
        } else {
            self.i18n.t_args(
                "localize-table",
                &[
                    ("game", game.display_name()),
                    ("file", &plan.table.derived_from),
                    ("date", &plan.table.date),
                    ("n", &plan.table.len().to_string()),
                ],
            )
        };
        let unknown_n: usize = plan.unknown.iter().map(|(_, n)| n).sum();
        let unknown = (unknown_n > 0).then(|| {
            let list: Vec<String> = plan.unknown.iter().map(|(s, _)| s.clone()).collect();
            self.i18n.t_args(
                "localize-unknown",
                &[("n", &unknown_n.to_string()), ("list", &list.join(", "))],
            )
        });
        let source_label = self.i18n.t_arg(
            "localize-source-too",
            "lang",
            &self.i18n.display_name(&doc.settings.source),
        );
        Some(Lines {
            files,
            table,
            unknown,
            source_label,
        })
    }

    /// Write the localized plugin and its STRINGS (R-C09-03, R-C09-04): the
    /// plugin where the user says (its own place by default, the original
    /// kept in backups/), the STRINGS in Strings/ next to it.
    pub(crate) fn export_localized(
        &mut self,
        parent: &eframe::Frame,
        tm: &TranslationMemory,
        exported_line: &str,
    ) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        let Some(table) = self.localize_table(doc) else {
            return;
        };
        let game = doc.settings.game();
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        let plugin_name = doc.file_name();
        let src = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.status = self
                    .i18n
                    .colon(&self.i18n.t("status-export-read-error"), &e.to_string());
                return;
            }
        };
        let out = match lstrings::localize(&src, tm, &table, game) {
            Ok(o) => o,
            Err(e) => {
                self.notify_err(
                    self.i18n
                        .colon(&self.i18n.t("status-export-error"), &e.to_string()),
                );
                return;
            }
        };
        let plugin_exts = crate::games::registry().all_plugin_extensions();
        let mut fd = rfd::FileDialog::new().set_parent(parent);
        if let Some(dir) = path.parent() {
            fd = fd.set_directory(dir);
        }
        let Some(dest) = fd
            .set_file_name(&plugin_name)
            .add_filter(
                self.i18n.t("dialog-plugin"),
                &plugin_exts.iter().map(String::as_str).collect::<Vec<_>>(),
            )
            .save_file()
        else {
            return;
        };
        let mut kept_all: Vec<PathBuf> = Vec::new();
        match self.export_write(&dest, &out.plugin) {
            Ok(kept) => kept_all.extend(kept),
            Err(e) => {
                self.notify_err(e);
                return;
            }
        }
        let utf8 = game.uses_utf8();
        let stem = dest
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mod")
            .to_string();
        let dir = dest
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join("Strings");
        let _ = std::fs::create_dir_all(&dir);
        let dst = crate::lang::strings_suffix_for(game, &settings.target);
        let src_l = crate::lang::strings_suffix_for(game, &settings.source);
        let mut sets: Vec<(&str, &Lists)> = vec![(dst.as_str(), &out.target)];
        if self.config.localize_source_strings && src_l != dst {
            sets.push((src_l.as_str(), &out.source));
        }
        let mut written = 0usize;
        for (lang, lists) in sets {
            // The source-language STRINGS are in the page of their language
            // for the older games (D-09).
            let page = if lang == src_l && lang != dst && !utf8 {
                Some(self.pages().for_language(&settings.source))
            } else {
                None
            };
            for (kind, list) in lists {
                let bytes = crate::tm::encoding::with_output(page, || {
                    crate::tm::strings::write(list, *kind, utf8)
                });
                let file = dir.join(format!("{stem}_{lang}.{}", kind.extension()));
                match self.export_write(&file, &bytes) {
                    Ok(kept) => {
                        kept_all.extend(kept);
                        written += 1;
                    }
                    Err(e) => {
                        self.notify_err(e);
                        return;
                    }
                }
            }
        }
        let mut msg = self.i18n.dash(
            &self.i18n.t_args(
                "localize-done",
                &[
                    ("n", &out.report.ids.to_string()),
                    ("files", &written.to_string()),
                ],
            ),
            exported_line,
        );
        if !out.report.unknown.is_empty() {
            let list: Vec<String> = out
                .report
                .unknown
                .iter()
                .map(|(s, n)| format!("{s} ({n})"))
                .collect();
            tracing::warn!(
                "localize: string fields unknown to the table: {}",
                list.join(", ")
            );
        }
        if let Some(note) = self.kept_list_note(&kept_all) {
            msg = self.i18n.dash(&msg, &note);
        }
        self.notify_ok(msg);
        let idx = self.active;
        self.write_mod_dict(idx, false);
        self.personal_autofill(idx);
    }
}
