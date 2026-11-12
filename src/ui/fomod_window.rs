//! « Traduire un FOMOD » window (M-02, rebuilt on the shared `ximod-fomod`
//! crate at F-03): the same window as XIMOD Architect's, working on the
//! same `.ximod-translation` sidecar, so a FOMOD translated in one opens in
//! the other.
//!
//! Translates the user-facing strings of an existing installer (mod name and
//! description, step / group / option names, option descriptions) into
//! another language. The work is stored in a sidecar JSON file next to the
//! FOMOD (`fomod/translations/<mod>.<lang>.ximod-translation`) and exported
//! through the lossless XML patcher (`xml::patch`), so the original
//! `ModuleConfig.xml` keeps its comments, encoding, unknown elements and
//! formatting: only the targeted strings change.
//!
//! The UI never goes through `load_ximod → save_ximod`, which is lossy for
//! third-party FOMODs. The model is only used for the breadcrumb context and
//! for the translated preview.

use super::app::{XimodApp, read_geometry};
use crate::ui::theme::ComboField as _;
use crate::ui::toasts::ToastLevel;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::path::PathBuf;
use ximod_fomod::package::{self as cli_translate, ApplyMode, ApplyOptions, SourceFiles};
use ximod_fomod::translate::{
    ExportMode, Glossary, GlossaryTerm, TField, TIssue, TStatus, TUnit, TranslationDoc,
    TranslationMemory, UpdateReport, glossary_issues, validate_doc,
};

const WINDOW_ID: &str = "ximod-fomod";

/// Row filter of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowFilter {
    #[default]
    All,
    Untranslated,
    Review,
    Issues,
    Locked,
}

/// Field-kind filter of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Names,
    Descriptions,
    Meta,
}

/// Everything the window needs between frames.
#[derive(Default)]
pub struct FomodTranslationState {
    pub open: bool,
    /// Root folder of the FOMOD being translated.
    root: Option<PathBuf>,
    /// The XML files as read from disk (encoding, raw bytes).
    sources: Option<SourceFiles>,
    /// The translation itself.
    doc: Option<TranslationDoc>,
    /// Where the sidecar is (or will be) saved.
    sidecar: Option<PathBuf>,
    /// Unsaved edits in `doc`.
    dirty: bool,
    /// Bumped on every edit; invalidates the row and issue caches.
    revision: u64,
    filter: RowFilter,
    kind: KindFilter,
    search: String,
    /// Indices into `doc.units` of the rows currently shown, with the inputs
    /// they were computed from.
    visible: Vec<usize>,
    visible_key: Option<(u64, RowFilter, KindFilter, String, bool)>,
    /// "Unique texts": one row per distinct source text; editing that row
    /// updates every string with the same text at once.
    pub uniques: bool,
    /// In unique mode: representative unit index → number of identical units.
    pub dup_count: HashMap<usize, usize>,
    /// Issues per unit key, computed for `issues_revision`.
    issues: HashMap<String, Vec<TIssue>>,
    issues_revision: Option<u64>,
    /// Selected row, as an index into `doc.units`.
    cursor: Option<usize>,
    /// Row to scroll into view on the next frame (index into `visible`).
    scroll_to: Option<usize>,
    /// Languages chosen before a FOMOD is loaded (and kept in the doc after).
    source_lang: String,
    target_lang: String,
    translator: String,
    export_mode: ExportMode,
    show_whitespace: bool,
    /// Report of the last "update from folder".
    last_report: Option<UpdateReport>,
    /// Message shown in the window (level, text).
    message: Option<(ToastLevel, String)>,
    /// Ask to discard unsaved edits before loading something else.
    pending_load: Option<PendingLoad>,
    /// The selected row to send to the online translator (Ctrl+G).
    online_request: Option<String>,
    /// A folder to load at the next frame (drop, command line).
    queued_folder: Option<PathBuf>,
    /// The « Coller des traductions… » box and its text.
    paste_open: bool,
    paste_buf: String,
    /// Translation memory and glossary of the current language pair.
    memory: TranslationMemory,
    glossary: Glossary,
    show_glossary: bool,
    /// Package export settings (lot T3).
    package_full: bool,
    package_7z: bool,
    name_template: String,
    /// A package export to start from the main window (needs the context).
    package_request: Option<PackageRequest>,
    /// A mod archive to extract (on the main window's worker thread) before
    /// loading its `fomod/` folder.
    archive_request: Option<PathBuf>,
    /// The loaded root came from an archive of which only `fomod/` was
    /// extracted: a "full mod" package cannot be built from it.
    partial: bool,
    /// The OS close button was pressed while edits were unsaved.
    close_requested: bool,
    /// Close after the unsaved-edits prompt was answered.
    close_now: bool,
}

#[derive(Clone)]
enum PendingLoad {
    Folder(PathBuf),
    Archive(PathBuf),
}

/// A translation package to build on the worker thread.
pub struct PackageRequest {
    pub root: PathBuf,
    pub doc: TranslationDoc,
    pub opts: cli_translate::PackageOptions,
}

impl FomodTranslationState {
    /// Changes whenever the texts shown by the window may have changed
    /// (used by the font loader to re-scan the scripts in use).
    pub fn font_revision(&self) -> u64 {
        if self.open {
            self.revision.wrapping_add(1)
        } else {
            0
        }
    }

    /// Visit every source and target text of the translation being edited
    /// (nothing when the window is closed).
    pub fn for_each_text(&self, mut f: impl FnMut(&str)) {
        if !self.open {
            return;
        }
        if let Some(doc) = &self.doc {
            for u in &doc.units {
                f(&u.source);
                f(&u.target);
            }
        }
        for e in &self.glossary.terms {
            f(&e.source);
            f(&e.target);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }

    /// A folder or a mod archive handed to the window (drop, `--fomod`):
    /// loaded on the next frame, through the unsaved-edits question when
    /// one is pending.
    pub(crate) fn open_path(&mut self, path: PathBuf) {
        let is_archive = path.is_file();
        if self.dirty {
            self.pending_load = Some(if is_archive {
                PendingLoad::Archive(path)
            } else {
                PendingLoad::Folder(path)
            });
        } else if is_archive {
            self.archive_request = Some(path);
        } else {
            self.queued_folder = Some(path);
        }
    }

    /// The visible sources as a numbered list (« 1. text »), one entry per
    /// row, line breaks inside a text replaced by « ⏎ » so an answer keeps
    /// one entry per number.
    pub fn numbered_sources(&self) -> String {
        let Some(doc) = &self.doc else {
            return String::new();
        };
        self.visible
            .iter()
            .enumerate()
            .filter_map(|(n, &i)| {
                doc.units
                    .get(i)
                    .map(|u| format!("{}. {}", n + 1, u.source.replace('\n', " ⏎ ")))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Apply a numbered list of translations to the visible rows (the
    /// number is the row's position in the list copied); a number outside
    /// the list is ignored; « ⏎ » becomes a line break again. Returns how
    /// many rows were filled.
    pub fn apply_numbered(&mut self, text: &str) -> usize {
        let entries = parse_numbered(text);
        let Some(doc) = self.doc.as_mut() else {
            return 0;
        };
        let mut n = 0;
        let mut keys = Vec::new();
        for (num, t) in entries {
            let Some(&idx) = num.checked_sub(1).and_then(|p| self.visible.get(p)) else {
                continue;
            };
            let Some(u) = doc.units.get_mut(idx) else {
                continue;
            };
            if u.locked || t.trim().is_empty() {
                continue;
            }
            u.target = t.replace(" ⏎ ", "\n").replace('⏎', "\n");
            u.status = TStatus::Translated;
            keys.push(u.key.clone());
            n += 1;
        }
        if n > 0 {
            if self.uniques {
                for k in keys {
                    doc.sync_identical(&k);
                }
            }
            self.bump();
        }
        n
    }

    /// The row of the last online request (D-01: the answer lands there).
    pub fn cursor_unit(&self) -> Option<usize> {
        self.cursor
    }

    /// The source of a row.
    pub fn unit_source(&self, unit: usize) -> Option<String> {
        self.doc.as_ref()?.units.get(unit).map(|u| u.source.clone())
    }

    /// Put an online answer into a row's target (D-01, R-D01-06), at
    /// « traduit », to be read again.
    pub fn set_unit_target(&mut self, unit: usize, text: &str) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let Some(u) = doc.units.get_mut(unit) else {
            return;
        };
        if u.locked {
            return;
        }
        u.target = text.to_string();
        u.status = TStatus::Translated;
        let key = u.key.clone();
        if self.uniques {
            doc.sync_identical(&key);
        }
        self.bump();
    }

    /// The languages of the window (ISO 639-3).
    pub fn langs(&self) -> (String, String) {
        (self.source_lang.clone(), self.target_lang.clone())
    }

    /// Send the selected row's source to the online translator (Ctrl+G).
    fn request_online(&mut self) {
        if let (Some(idx), Some(doc)) = (self.cursor, &self.doc)
            && let Some(u) = doc.units.get(idx)
        {
            self.online_request = Some(u.source.clone());
        }
    }

    fn set_message(&mut self, level: ToastLevel, text: impl Into<String>) {
        self.message = Some((level, text.into()));
    }

    /// Recompute the visible rows when the filter, search or data changed.
    fn refresh_visible(&mut self) {
        if self.doc.is_none() {
            self.visible.clear();
            self.visible_key = None;
            return;
        }
        let key = (
            self.revision,
            self.filter,
            self.kind,
            self.search.clone(),
            self.uniques,
        );
        if self.visible_key.as_ref() == Some(&key) {
            return;
        }
        self.refresh_issues();
        let Some(doc) = &self.doc else { return };
        let needle = self.search.trim().to_lowercase();
        // Unique mode: only the first unit of each identical-text group is
        // listed (the representative), with the group size for its badge.
        let groups = if self.uniques {
            doc.identical_groups()
        } else {
            HashMap::new()
        };
        let mut rows = Vec::with_capacity(doc.units.len());
        for (i, u) in doc.units.iter().enumerate() {
            if self.uniques && !groups.contains_key(&i) {
                continue;
            }
            let kind_ok = match self.kind {
                KindFilter::All => true,
                KindFilter::Names => matches!(
                    u.field,
                    TField::StepName | TField::GroupName | TField::PluginName
                ),
                KindFilter::Descriptions => u.field.is_multiline(),
                KindFilter::Meta => matches!(
                    u.field,
                    TField::InfoName
                        | TField::InfoAuthor
                        | TField::InfoWebsite
                        | TField::ModuleName
                ),
            };
            if !kind_ok {
                continue;
            }
            let filter_ok = match self.filter {
                RowFilter::All => true,
                RowFilter::Untranslated => !u.locked && u.target.trim().is_empty(),
                RowFilter::Review => matches!(u.status, TStatus::Auto | TStatus::Fuzzy),
                RowFilter::Issues => self.issues.get(&u.key).is_some_and(|v| !v.is_empty()),
                RowFilter::Locked => u.locked,
            };
            if !filter_ok {
                continue;
            }
            if !needle.is_empty()
                && !u.source.to_lowercase().contains(&needle)
                && !u.target.to_lowercase().contains(&needle)
                && !u.context.to_lowercase().contains(&needle)
            {
                continue;
            }
            rows.push(i);
        }
        self.visible = rows;
        self.dup_count = groups;
        self.visible_key = Some(key);
    }

    /// After an edit of unit `key` in unique mode, keep every identical text
    /// in step with it.
    fn sync_identical(&mut self, key: &str, i18n: &crate::i18n::I18n) {
        if !self.uniques {
            return;
        }
        let Some(doc) = self.doc.as_mut() else { return };
        let n = doc.sync_identical(key);
        if n > 0 {
            self.set_message(ToastLevel::Info, i18n.t_num("ftr-uniques-synced", n as i64));
        }
    }

    fn refresh_issues(&mut self) {
        if self.issues_revision == Some(self.revision) {
            return;
        }
        let mut map: HashMap<String, Vec<TIssue>> = HashMap::new();
        if let Some(doc) = &self.doc {
            for (key, issue) in validate_doc(doc) {
                map.entry(key).or_default().push(issue);
            }
            for (key, issue) in glossary_issues(doc, &self.glossary) {
                map.entry(key).or_default().push(issue);
            }
        }
        self.issues = map;
        self.issues_revision = Some(self.revision);
    }

    /// Load the FOMOD under `root`: read the XML, pick up an existing sidecar
    /// for the target language (merging it with the current strings), or
    /// start a fresh translation.
    fn load_root(&mut self, root: PathBuf, i18n: &crate::i18n::I18n) {
        self.load_root_with(root, false, i18n);
    }

    /// [`load_root`](Self::load_root) for a FOMOD extracted from an archive
    /// with only its `fomod/` folder: the "full mod" package is disabled.
    pub(crate) fn load_root_partial(&mut self, root: PathBuf, i18n: &crate::i18n::I18n) {
        self.load_root_with(root, true, i18n);
    }

    fn load_root_with(&mut self, root: PathBuf, partial: bool, i18n: &crate::i18n::I18n) {
        let sources = match cli_translate::read_sources(&root) {
            Ok(s) => s,
            Err(e) => {
                self.set_message(
                    ToastLevel::Error,
                    i18n.t_arg("ftr-load-error", "error", &e.to_string()),
                );
                return;
            }
        };
        let fresh =
            match cli_translate::extract_translation(&root, &self.source_lang, &self.target_lang) {
                Ok(d) => d,
                Err(e) => {
                    self.set_message(
                        ToastLevel::Error,
                        i18n.t_arg("ftr-load-error", "error", &e.to_string()),
                    );
                    return;
                }
            };
        let sidecar_path = TranslationDoc::sidecar_path(&root, &fresh.mod_name, &self.target_lang);
        // An existing sidecar for this language wins; it is merged with the
        // strings as they are today.
        let existing = TranslationDoc::find_sidecars(&root)
            .into_iter()
            .filter_map(|p| TranslationDoc::load(&p).ok().map(|d| (p, d)))
            .find(|(_, d)| d.target_lang == self.target_lang);
        self.last_report = None;
        let (doc, path) = match existing {
            Some((path, old)) => match cli_translate::update_translation(&root, &old) {
                Ok((merged, report)) => {
                    let mut args = fluent::FluentArgs::new();
                    args.set("new", report.new as i64);
                    args.set("changed", report.changed as i64);
                    args.set("removed", report.removed as i64);
                    self.set_message(
                        ToastLevel::Info,
                        i18n.t_with_args("ftr-sidecar-found", Some(&args)),
                    );
                    self.last_report = Some(report);
                    (merged, path)
                }
                Err(e) => {
                    self.set_message(
                        ToastLevel::Error,
                        i18n.t_arg("ftr-load-error", "error", &e.to_string()),
                    );
                    return;
                }
            },
            None => {
                self.set_message(
                    ToastLevel::Info,
                    i18n.t_num("ftr-extracted", fresh.units.len() as i64),
                );
                (fresh, sidecar_path)
            }
        };
        self.translator = if doc.translator.is_empty() {
            self.translator.clone()
        } else {
            doc.translator.clone()
        };
        self.root = Some(root);
        self.sources = Some(sources);
        self.doc = Some(doc);
        self.sidecar = Some(path);
        self.partial = partial;
        if partial {
            self.package_full = false;
        }
        self.dirty = false;
        self.cursor = None;
        self.visible_key = None;
        self.issues_revision = None;
        self.revision = self.revision.wrapping_add(1);
        self.export_mode = self.doc.as_ref().map(|d| d.export.mode).unwrap_or_default();
        self.name_template = self
            .doc
            .as_ref()
            .map(|d| d.export.name_template.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| cli_translate::DEFAULT_NAME_TEMPLATE.to_string());
        self.memory = TranslationMemory::load(&self.source_lang, &self.target_lang);
        self.glossary = Glossary::load(&self.source_lang, &self.target_lang);
    }

    /// Fill untranslated rows from the translation memory (exact matches).
    fn apply_memory(&mut self, i18n: &crate::i18n::I18n) {
        let Some(doc) = self.doc.as_mut() else { return };
        let n = self.memory.apply(doc);
        if n > 0 {
            self.bump();
            self.filter = RowFilter::Review;
        }
        self.set_message(ToastLevel::Info, i18n.t_num("ftr-memory-applied", n as i64));
    }

    /// Copy the selected row's translation to every identical source.
    fn propagate_current(&mut self, i18n: &crate::i18n::I18n) {
        let Some(idx) = self.cursor else { return };
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(key) = doc.units.get(idx).map(|u| u.key.clone()) else {
            return;
        };
        let n = doc.propagate(&key);
        if n > 0 {
            self.bump();
        }
        self.set_message(ToastLevel::Info, i18n.t_num("ftr-propagated", n as i64));
    }

    /// Export the table as CSV (for spreadsheet editing or external translators).
    fn export_csv(&mut self, i18n: &crate::i18n::I18n) {
        let Some(doc) = self.doc.as_ref() else { return };
        let default = format!(
            "{}.{}.csv",
            doc.mod_name.replace(['/', '\\'], "_"),
            doc.target_lang
        );
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default)
            .add_filter("CSV", &["csv"])
            .save_file()
        else {
            return;
        };
        let mut bytes = Vec::from(&b"\xEF\xBB\xBF"[..]); // BOM: Excel then reads UTF-8 correctly
        bytes.extend_from_slice(doc.to_csv().as_bytes());
        match std::fs::write(&path, bytes) {
            Ok(()) => self.set_message(
                ToastLevel::Success,
                i18n.t_arg("ftr-saved", "path", &path.display().to_string()),
            ),
            Err(e) => self.set_message(
                ToastLevel::Error,
                i18n.t_arg("ftr-csv-error", "error", &e.to_string()),
            ),
        }
    }

    /// Import translations from a CSV (matched by key).
    fn import_csv(&mut self, i18n: &crate::i18n::I18n) {
        if self.doc.is_none() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read(&path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                self.set_message(
                    ToastLevel::Error,
                    i18n.t_arg("ftr-csv-error", "error", &e.to_string()),
                );
                return;
            }
        };
        let Some(doc) = self.doc.as_mut() else { return };
        match doc.import_csv(&text) {
            Ok(n) => {
                if n > 0 {
                    self.bump();
                }
                self.set_message(
                    ToastLevel::Success,
                    i18n.t_num("ftr-csv-imported", n as i64),
                );
            }
            Err(e) => self.set_message(
                ToastLevel::Error,
                i18n.t_arg("ftr-csv-error", "error", &e.to_string()),
            ),
        }
    }

    /// Write the sidecar.
    fn save(&mut self, i18n: &crate::i18n::I18n) -> bool {
        let (Some(doc), Some(path)) = (self.doc.as_mut(), self.sidecar.as_ref()) else {
            return false;
        };
        doc.translator = self.translator.clone();
        doc.export.mode = self.export_mode;
        doc.export.name_template = self.name_template.clone();
        match doc.save(path) {
            Ok(()) => {
                self.dirty = false;
                // The memory learns every confirmed translation, so the next
                // mod with the same strings starts pre-filled.
                let learned = self.memory.learn(doc);
                if learned > 0 && self.memory.save().is_err() {
                    tracing::warn!("translation memory could not be saved");
                }
                self.set_message(
                    ToastLevel::Success,
                    i18n.t_arg("ftr-saved", "path", &path.display().to_string()),
                );
                true
            }
            Err(e) => {
                self.set_message(
                    ToastLevel::Error,
                    i18n.t_arg("ftr-save-error", "error", &e.to_string()),
                );
                false
            }
        }
    }

    /// Write the translated XML files (sibling folder or in place).
    fn export(&mut self, i18n: &crate::i18n::I18n) {
        self.refresh_issues();
        let (Some(root), Some(doc)) = (self.root.clone(), self.doc.as_ref()) else {
            return;
        };
        let blocking = self
            .issues
            .values()
            .flatten()
            .filter(|i| i.is_blocking())
            .count();
        if blocking > 0 {
            self.set_message(
                ToastLevel::Error,
                i18n.t_num("ftr-export-blocked", blocking as i64),
            );
            self.filter = RowFilter::Issues;
            return;
        }
        if self.export_mode == ExportMode::Package {
            // A Nexus-ready archive: built on the worker thread by the main
            // window (it owns the progress window), see `take_package_request`.
            let format = if self.package_7z {
                ximod_fomod::archive::ArchiveFormat::SevenZip
            } else {
                ximod_fomod::archive::ArchiveFormat::Zip
            };
            let kind = if self.package_full {
                cli_translate::PackageKind::Full
            } else {
                cli_translate::PackageKind::PatchOnly
            };
            let lang_display = i18n.display_name(&doc.target_lang);
            let ext = if self.package_7z { "7z" } else { "zip" };
            let file_name =
                cli_translate::package_name(doc, &self.name_template, &lang_display, ext);
            let Some(out) = rfd::FileDialog::new()
                .set_directory(root.parent().unwrap_or(&root))
                .set_file_name(file_name)
                .add_filter(ext.to_uppercase(), &[ext])
                .save_file()
            else {
                return;
            };
            let mut args = fluent::FluentArgs::new();
            args.set("name", doc.mod_name.clone());
            args.set("langname", lang_display);
            let readme = i18n.t_with_args(
                if self.package_full {
                    "ftr-readme-full"
                } else {
                    "ftr-readme-patch"
                },
                Some(&args),
            );
            self.package_request = Some(PackageRequest {
                root,
                doc: doc.clone(),
                opts: cli_translate::PackageOptions {
                    kind,
                    format,
                    out,
                    force_explicit_order: doc.export.force_explicit_order,
                    readme: Some(readme),
                },
            });
            return;
        }
        let mode = match self.export_mode {
            ExportMode::InPlace => ApplyMode::InPlace,
            ExportMode::Sibling | ExportMode::Package => ApplyMode::Sibling,
        };
        let opts = ApplyOptions {
            mode,
            force_explicit_order: doc.export.force_explicit_order,
            force: false,
        };
        match cli_translate::apply_translation(&root, doc, &opts) {
            Ok(outcome) => {
                let mut args = fluent::FluentArgs::new();
                args.set("count", outcome.applied as i64);
                let dir = outcome
                    .written
                    .first()
                    .and_then(|p| p.parent())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                args.set("path", dir);
                let msg = i18n.t_with_args("ftr-export-success", Some(&args));
                self.set_message(ToastLevel::Success, msg);
                if outcome.stale > 0 {
                    self.set_message(
                        ToastLevel::Warning,
                        i18n.t_num("ftr-export-stale", outcome.stale as i64),
                    );
                }
            }
            Err(e) => {
                self.set_message(
                    ToastLevel::Error,
                    i18n.t_arg("ftr-export-error", "error", &e.to_string()),
                );
            }
        }
    }

    /// Re-read the FOMOD from disk and merge the current translation into it.
    fn update_from_folder(&mut self, i18n: &crate::i18n::I18n) {
        let (Some(root), Some(doc)) = (self.root.clone(), self.doc.as_ref()) else {
            return;
        };
        match cli_translate::update_translation(&root, doc) {
            Ok((merged, report)) => {
                let mut args = fluent::FluentArgs::new();
                args.set("new", report.new as i64);
                args.set("changed", report.changed as i64);
                args.set("moved", report.moved as i64);
                args.set("removed", report.removed as i64);
                args.set("unchanged", report.unchanged as i64);
                self.set_message(
                    ToastLevel::Info,
                    i18n.t_with_args("ftr-update-report", Some(&args)),
                );
                self.last_report = Some(report);
                self.doc = Some(merged);
                self.sources = cli_translate::read_sources(&root).ok();
                self.bump();
            }
            Err(e) => {
                self.set_message(
                    ToastLevel::Error,
                    i18n.t_arg("ftr-load-error", "error", &e.to_string()),
                );
            }
        }
    }

    /// Move the cursor to the next row (in table order) that still needs a
    /// translation, starting after the current one.
    fn next_untranslated(&mut self) {
        let Some(doc) = &self.doc else { return };
        if self.visible.is_empty() {
            return;
        }
        let start = self
            .cursor
            .and_then(|c| self.visible.iter().position(|&i| i == c))
            .map(|p| p + 1)
            .unwrap_or(0);
        let n = self.visible.len();
        for k in 0..n {
            let pos = (start + k) % n;
            let idx = self.visible[pos];
            let u = &doc.units[idx];
            if !u.locked && u.target.trim().is_empty() {
                self.cursor = Some(idx);
                self.scroll_to = Some(pos);
                return;
            }
        }
    }
}

/// Parse a numbered list (« 1. text », « 2) text »…): a line that does not
/// start with a number continues the previous entry.
pub fn parse_numbered(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &trimmed[digits.len()..];
        if !digits.is_empty()
            && (rest.starts_with('.') || rest.starts_with(')') || rest.starts_with(':'))
            && let Ok(n) = digits.parse::<usize>()
        {
            out.push((n, rest[1..].trim().to_string()));
            continue;
        }
        if let Some(last) = out.last_mut()
            && !line.trim().is_empty()
        {
            last.1.push('\n');
            last.1.push_str(line.trim());
        }
    }
    out
}

/// Glyph and tooltip key for a unit's status column.
fn status_glyph(u: &TUnit) -> (&'static str, &'static str) {
    if u.locked {
        return (crate::icons::LOCK, "ftr-status-locked");
    }
    match u.status {
        TStatus::Translated => (crate::icons::CHECK, "ftr-status-translated"),
        TStatus::Auto => (crate::icons::CIRCLE_HALF, "ftr-status-auto"),
        TStatus::Fuzzy => ("~", "ftr-status-fuzzy"),
        TStatus::Obsolete => (crate::icons::X, "ftr-status-obsolete"),
        TStatus::Untranslated => {
            if u.target.trim().is_empty() {
                (crate::icons::CIRCLE, "ftr-status-untranslated")
            } else {
                (crate::icons::CHECK, "ftr-status-translated")
            }
        }
    }
}

pub(crate) fn field_key(f: TField) -> &'static str {
    match f {
        TField::InfoName => "ftr-field-info-name",
        TField::InfoAuthor => "ftr-field-author",
        TField::InfoWebsite => "ftr-field-website",
        TField::InfoDescription => "ftr-field-description",
        TField::ModuleName => "ftr-field-module-name",
        TField::StepName => "ftr-field-step",
        TField::GroupName => "ftr-field-group",
        TField::PluginName => "ftr-field-plugin",
        TField::PluginDescription => "ftr-field-plugin-desc",
    }
}

fn issue_text(i18n: &crate::i18n::I18n, issue: &TIssue) -> String {
    match issue {
        TIssue::EmptyTarget => i18n.t("ftr-issue-empty"),
        TIssue::WhitespaceOnly => i18n.t("ftr-issue-whitespace"),
        TIssue::EdgeWhitespaceMismatch => i18n.t("ftr-issue-edge-whitespace"),
        TIssue::TokenMismatch { missing, extra } => {
            let mut args = fluent::FluentArgs::new();
            args.set("missing", missing.join(", "));
            args.set("extra", extra.join(", "));
            i18n.t_with_args("ftr-issue-token", Some(&args))
        }
        TIssue::NewlineInName => i18n.t("ftr-issue-newline-name"),
        TIssue::ControlChars => i18n.t("ftr-issue-control"),
        TIssue::LengthRatio { ratio } => {
            i18n.t_arg("ftr-issue-length", "ratio", &format!("{ratio:.1}"))
        }
        TIssue::IdenticalToSource => i18n.t("ftr-issue-identical"),
        TIssue::InconsistentDuplicate { other_key } => {
            i18n.t_arg("ftr-issue-duplicate", "key", other_key)
        }
        TIssue::CdataTerminator => i18n.t("ftr-issue-cdata"),
        TIssue::GlossaryViolation { term } => i18n.t_arg("ftr-issue-glossary", "term", term),
    }
}

/// Make tabs and line breaks visible in the read-only source pane.
/// Make every blank visible, the way editors do: `·` for a space, `⍽` for a
/// non-breaking space, `→` for a tab, `␍` for a carriage return and `¶` at
/// each line end (the line break itself is kept so the layout is unchanged).
fn mark_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    for c in s.chars() {
        match c {
            ' ' => out.push('·'),
            '\u{a0}' => out.push('⍽'),
            '\t' => {
                out.push(crate::icons::ARROW_RIGHT_CHAR);
                out.push_str("   ");
            }
            '\r' => out.push(crate::icons::RETURN_CHAR),
            '\n' => out.push_str("¶\n"),
            other => out.push(other),
        }
    }
    out
}

/// What a background job of the window reports when done.
pub enum JobDone {
    /// A mod archive's `fomod/` folder was extracted: load it.
    Extracted(PathBuf),
    /// A translation package was written (path, files).
    Packaged(PathBuf, usize),
}

/// A job running on its own thread (archive extraction, package build): the
/// header shows its label with a spinner until the result arrives.
pub struct FomodJob {
    pub label: String,
    pub rx: std::sync::mpsc::Receiver<Result<JobDone, String>>,
}

impl XimodApp {
    /// Open the window (Outils ▸ Traduire un FOMOD…), or bring it back.
    pub(crate) fn open_fomod_translation(&mut self) {
        let st = &mut self.fomod_trans;
        if st.source_lang.is_empty() {
            st.source_lang = "eng".to_string();
        }
        if st.target_lang.is_empty() {
            // The target of the active document, else the interface language.
            let doc_target = self
                .docs
                .get(self.active)
                .map(|d| d.settings.target.clone())
                .unwrap_or_default();
            st.target_lang = if !doc_target.trim().is_empty() {
                doc_target
            } else if self.config.locale == "eng" {
                "fra".to_string()
            } else {
                self.config.locale.clone()
            };
        }
        if st.translator.is_empty() {
            st.translator = self.config.translator.clone();
        }
        if !st.open {
            self.fomod_needs_center = true;
        }
        st.open = true;
    }

    /// Extract the `fomod/` folder of a mod archive on a thread; the window
    /// loads it when the job reports (`poll_fomod_job`).
    fn translate_open_archive(&mut self, archive: PathBuf) {
        use ximod_fomod::archive_open::{self, Prepared, WorkKind};
        if self.fomod_job.is_some() {
            return;
        }
        let dest = match archive_open::prepare(&archive, WorkKind::FomodOnly) {
            Ok(Prepared::Extract { dest }) => dest,
            // Never reused for the translation window (see `prepare`).
            Ok(Prepared::Reused { root }) => {
                let mut st = std::mem::take(&mut self.fomod_trans);
                st.load_root_partial(root, &self.i18n);
                self.fomod_trans = st;
                return;
            }
            Err(e) => {
                let msg = self.translate_archive_error(&e);
                self.fomod_trans.set_message(ToastLevel::Error, msg);
                return;
            }
        };
        let label = self.i18n.t_arg(
            "ftr-job-extract",
            "file",
            &archive
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let src = archive.clone();
        std::thread::spawn(move || {
            let mut progress = |_done: usize, _total: usize, _p: &std::path::Path| true;
            let r = archive_open::extract_archive(&src, &dest, true, &mut progress)
                .map(|_| JobDone::Extracted(src.clone()))
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        self.fomod_job = Some(FomodJob { label, rx });
    }

    /// Build a translation package on a thread.
    fn translate_package(&mut self, req: PackageRequest) {
        if self.fomod_job.is_some() {
            return;
        }
        let out = req.opts.out.clone();
        let label = self.i18n.t_arg(
            "ftr-job-package",
            "file",
            &out.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut progress = |_done: usize, _total: usize, _p: &std::path::Path| true;
            let r =
                cli_translate::package_translation(&req.root, &req.doc, &req.opts, &mut progress)
                    .map(|o| JobDone::Packaged(out.clone(), o.files))
                    .map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        self.fomod_job = Some(FomodJob { label, rx });
    }

    /// Collect a finished job: load the extracted `fomod/`, or report the
    /// package.
    fn poll_fomod_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.fomod_job else { return };
        match job.rx.try_recv() {
            Ok(result) => {
                self.fomod_job = None;
                match result {
                    Ok(JobDone::Extracted(archive)) => self.translate_archive_extracted(&archive),
                    Ok(JobDone::Packaged(out, files)) => {
                        let msg = self.i18n.t_args(
                            "ftr-package-done",
                            &[
                                ("path", &out.display().to_string()),
                                ("num", &files.to_string()),
                            ],
                        );
                        self.notify_ok(msg);
                    }
                    Err(e) => {
                        let msg = self.i18n.t_arg("ftr-export-error", "error", &e);
                        self.notify_err(msg);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.fomod_job = None;
            }
        }
    }

    /// Completion of an extraction: load the `fomod/`.
    fn translate_archive_extracted(&mut self, archive: &std::path::Path) {
        use ximod_fomod::archive_open::{self, WorkKind};
        match archive_open::extracted_root(archive, WorkKind::FomodOnly) {
            Some(root) => {
                let mut st = std::mem::take(&mut self.fomod_trans);
                st.load_root_partial(root, &self.i18n);
                self.fomod_trans = st;
            }
            None => {
                let e = archive_open::ArchiveError::NoFomod {
                    path: archive.to_path_buf(),
                };
                let msg = self.translate_archive_error(&e);
                self.fomod_trans.set_message(ToastLevel::Error, msg);
            }
        }
    }

    fn translate_archive_error(&self, e: &ximod_fomod::archive_open::ArchiveError) -> String {
        use ximod_fomod::archive_open::ArchiveError as E;
        match e {
            E::UnsupportedArchive { ext } => self.i18n.t_arg("msg-archive-unsupported", "ext", ext),
            E::NoFomod { path } => {
                self.i18n
                    .t_arg("msg-archive-no-fomod", "path", &path.display().to_string())
            }
            other => self
                .i18n
                .t_arg("msg-archive-error", "error", &other.to_string()),
        }
    }

    /// Draw the window. Called every frame; a no-op while closed.
    pub(crate) fn render_fomod_translation(&mut self, ctx: &egui::Context) {
        if !self.fomod_trans.open {
            return;
        }
        self.poll_fomod_job(ctx);
        // Take the state out so the window body can borrow `self.i18n` and
        // `self.config` disjointly.
        let mut st = std::mem::take(&mut self.fomod_trans);
        st.open = true;
        let title = format!("{} — {}", crate::APP_NAME, self.i18n.t("ftr-title"));
        let geo = self.config.fomod_win;
        let just_opened = self.fomod_needs_center;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(700.0), geo[3].max(480.0)]))
        } else {
            None
        };
        let mut vb = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([700.0, 480.0]);
        if let Some((pos, size)) = place {
            vb = vb
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let i18n = &self.i18n;
        let cfg = &mut self.config;
        let locales: Vec<String> = i18n.language_codes();
        let job_label = self.fomod_job.as_ref().map(|j| j.label.clone());
        let mut do_close = false;
        let mut toast: Option<(ToastLevel, String)> = None;
        let mut package: Option<PackageRequest> = None;
        let mut archive: Option<PathBuf> = None;
        let mut online: Option<String> = None;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(WINDOW_ID),
            vb,
            |ctx, _class| {
                if let Some((pos, size)) = place {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        pos[0], pos[1],
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        size[0], size[1],
                    )));
                }
                // ---- keyboard ----
                let (save_key, next_key, online_key) = ctx.input_mut(|i| {
                    (
                        i.consume_shortcut(&egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::S,
                        )),
                        i.consume_shortcut(&egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::Enter,
                        )),
                        i.consume_shortcut(&egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::G,
                        )),
                    )
                });
                if save_key && st.doc.is_some() {
                    st.save(i18n);
                }
                if next_key {
                    st.refresh_visible();
                    st.next_untranslated();
                }
                if online_key {
                    st.request_online();
                }

                if let Some(dir) = st.queued_folder.take() {
                    let root = ximod_fomod::archive_open::find_fomod_root(&dir, 2).unwrap_or(dir);
                    st.load_root(root, i18n);
                }
                st.refresh_visible();
                render_header(ctx, &mut st, i18n, &locales, job_label.as_deref());
                render_detail(ctx, &mut st, i18n);
                render_table(ctx, &mut st, i18n);

                if let Some((level, text)) = st.message.take() {
                    toast = Some((level, text));
                }
                if let Some(req) = st.package_request.take() {
                    package = Some(req);
                }
                if let Some(req) = st.archive_request.take() {
                    archive = Some(req);
                }
                if let Some(text) = st.online_request.take() {
                    online = Some(text);
                }

                if !just_opened && let Some((x, y, w, h)) = read_geometry(ctx) {
                    cfg.fomod_win = [x, y, w, h];
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    do_close = true;
                }
            },
        );
        self.fomod_needs_center = false;
        if self.config.translator != st.translator {
            self.config.translator = st.translator.clone();
        }

        if let Some((level, text)) = toast {
            self.notify(level, text);
        }
        if let Some(text) = online {
            // The service when one is usable (R-D01-06), the browser else.
            let (src, dst) = st.langs();
            let served = match st.cursor_unit() {
                Some(u) => self.online_fomod(u, text.clone(), &src, &dst),
                None => false,
            };
            if !served {
                // The site chosen in Paramètres › En ligne (R-T03).
                let dst = ximod_fomod::lang::iso639_3_to_1(&st.target_lang);
                let src = ximod_fomod::lang::iso639_3_to_1(&st.source_lang);
                self.browser_open(
                    &text,
                    src,
                    dst,
                    crate::games::Game::Unknown,
                    crate::tags::TagSet::any(),
                );
            }
        }
        if let Some(req) = package {
            self.translate_package(req);
        }
        if let Some(path) = archive {
            self.fomod_trans = st;
            self.translate_open_archive(path);
            return;
        }
        if st.close_now {
            st.close_now = false;
            st.open = false;
            let _ = self.config.save();
        } else if do_close {
            if st.dirty {
                // Keep the window open and tell the user instead of losing
                // the edits silently.
                st.set_message(ToastLevel::Warning, self.i18n.t("ftr-unsaved-close"));
                st.close_requested = true;
            } else {
                st.open = false;
                let _ = self.config.save();
            }
        }
        self.fomod_trans = st;
    }
}

/// Top panel: source folder, languages, translator, actions, progress.
fn render_header(
    ctx: &egui::Context,
    st: &mut FomodTranslationState,
    i18n: &crate::i18n::I18n,
    locales: &[String],
    job: Option<&str>,
) {
    egui::TopBottomPanel::top("ftr_header").show(ctx, |ui| {
        ui.add_space(4.0);
        // Row 1: source + languages.
        ui.horizontal(|ui| {
            // (no `if … && let`: let chains need Rust 1.88, rust-version is 1.85)
            if ui.button(i18n.t("ftr-open-folder")).clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                if st.dirty {
                    st.pending_load = Some(PendingLoad::Folder(path));
                } else {
                    st.load_root(path, i18n);
                }
            }
            if ui.button(i18n.t("ftr-open-archive")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(i18n.t("filter-archive"), &["zip", "7z"])
                    .pick_file()
            {
                if st.dirty {
                    st.pending_load = Some(PendingLoad::Archive(path));
                } else {
                    st.archive_request = Some(path);
                }
            }
            ui.separator();
            match &st.root {
                Some(r) => {
                    ui.label(RichText::new(r.display().to_string()).monospace().small());
                }
                None => {
                    ui.label(RichText::new(i18n.t("ftr-no-fomod")).weak());
                }
            }
            if let Some(src) = &st.sources {
                let enc = src
                    .config
                    .as_ref()
                    .or(src.info.as_ref())
                    .map(|f| f.encoding.label())
                    .unwrap_or("");
                if !enc.is_empty() {
                    ui.label(RichText::new(enc).small().weak())
                        .on_hover_text(i18n.t("ftr-encoding"));
                }
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(i18n.t("ftr-source-lang"));
            let loaded = st.doc.is_some();
            lang_combo(ui, "ftr_src", &mut st.source_lang, locales, i18n, !loaded);
            ui.label(crate::icons::ARROW_RIGHT);
            ui.label(i18n.t("ftr-target-lang"));
            lang_combo(ui, "ftr_tgt", &mut st.target_lang, locales, i18n, !loaded);
            if loaded {
                ui.label(RichText::new(i18n.t("ftr-lang-locked")).small().weak());
            }
            ui.separator();
            ui.label(i18n.t("ftr-translator"));
            if ui
                .add(egui::TextEdit::singleline(&mut st.translator).desired_width(160.0))
                .changed()
            {
                st.dirty = st.doc.is_some();
            }
        });
        ui.add_space(2.0);
        // Row 2: actions + progress.
        ui.horizontal(|ui| {
            let loaded = st.doc.is_some();
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("ftr-save")))
                .on_hover_text(crate::ui::app::key_text(
                    ui.ctx(),
                    i18n,
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ))
                .clicked()
            {
                st.save(i18n);
            }
            ui.add_enabled_ui(loaded, |ui| {
                egui::ComboBox::from_id_salt("ftr_export_mode")
                    .selected_text(match st.export_mode {
                        ExportMode::Sibling => i18n.t("ftr-export-sibling"),
                        ExportMode::InPlace => i18n.t("ftr-export-inplace"),
                        ExportMode::Package => i18n.t("ftr-export-package"),
                    })
                    .show_field(ui, |ui| {
                        ui.selectable_value(
                            &mut st.export_mode,
                            ExportMode::Sibling,
                            i18n.t("ftr-export-sibling"),
                        )
                        .on_hover_text(i18n.t("ftr-export-sibling-hint"));
                        ui.selectable_value(
                            &mut st.export_mode,
                            ExportMode::InPlace,
                            i18n.t("ftr-export-inplace"),
                        )
                        .on_hover_text(i18n.t("ftr-export-inplace-hint"));
                        ui.selectable_value(
                            &mut st.export_mode,
                            ExportMode::Package,
                            i18n.t("ftr-export-package"),
                        )
                        .on_hover_text(i18n.t("ftr-export-package-hint"));
                    });
                if st.export_mode == ExportMode::Package {
                    ui.checkbox(&mut st.package_7z, "7z");
                    // Only the fomod/ folder was extracted: no full package.
                    let full_hint = if st.partial {
                        i18n.t("ftr-package-full-partial")
                    } else {
                        i18n.t("ftr-package-full-hint")
                    };
                    ui.add_enabled(
                        !st.partial,
                        egui::Checkbox::new(&mut st.package_full, i18n.t("ftr-package-full")),
                    )
                    .on_hover_text(full_hint);
                    ui.label(i18n.t("ftr-package-name-template"));
                    ui.add(egui::TextEdit::singleline(&mut st.name_template).desired_width(140.0))
                        .on_hover_text("{name} {version} {LANG} {lang} {lang3} {langname}");
                }
            });
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("ftr-export")))
                .clicked()
            {
                st.export(i18n);
            }
            if let Some(doc) = st.doc.as_mut() {
                ui.checkbox(
                    &mut doc.export.force_explicit_order,
                    i18n.t("ftr-force-explicit-order"),
                )
                .on_hover_text(i18n.t("ftr-warn-order"));
            }
            ui.separator();
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("ftr-update")))
                .on_hover_text(i18n.t("ftr-update-hint"))
                .clicked()
            {
                st.update_from_folder(i18n);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(label) = job {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new(label).small());
                    ui.separator();
                }
                if let Some(doc) = &st.doc {
                    let s = doc.stats();
                    let frac = if s.total == 0 {
                        0.0
                    } else {
                        s.translated as f32 / s.total as f32
                    };
                    let mut args = fluent::FluentArgs::new();
                    args.set("done", s.translated as i64);
                    args.set("total", s.total as i64);
                    let text = i18n.t_with_args("ftr-progress", Some(&args));
                    if st.dirty {
                        ui.label(
                            RichText::new(crate::icons::DOT).color(ui.visuals().warn_fg_color),
                        )
                        .on_hover_text(i18n.t("statusbar-modified"));
                    }
                    // The bar only takes the room left by the buttons on its
                    // left, so it never overlaps them in a narrow window; it
                    // shrinks down to the bare counter, then disappears.
                    let avail = ui.available_width() - 8.0;
                    if avail >= 150.0 {
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .desired_width(avail.min(220.0))
                                .text(text),
                        );
                    } else if avail >= 60.0 {
                        ui.label(RichText::new(format!("{} / {}", s.translated, s.total)).small())
                            .on_hover_text(text);
                    }
                }
            });
        });
        ui.add_space(2.0);
        // Row 3: filters + search.
        ui.horizontal(|ui| {
            for (f, key) in [
                (RowFilter::All, "ftr-filter-all"),
                (RowFilter::Untranslated, "ftr-filter-untranslated"),
                (RowFilter::Review, "ftr-filter-review"),
                (RowFilter::Issues, "ftr-filter-issues"),
                (RowFilter::Locked, "ftr-filter-locked"),
            ] {
                ui.selectable_value(&mut st.filter, f, i18n.t(key));
            }
            ui.separator();
            ui.toggle_value(&mut st.uniques, i18n.t("ftr-uniques"))
                .on_hover_text(i18n.t("ftr-uniques-hint"));
            ui.separator();
            egui::ComboBox::from_id_salt("ftr_kind")
                .selected_text(match st.kind {
                    KindFilter::All => i18n.t("ftr-type-all"),
                    KindFilter::Names => i18n.t("ftr-type-names"),
                    KindFilter::Descriptions => i18n.t("ftr-type-descriptions"),
                    KindFilter::Meta => i18n.t("ftr-type-meta"),
                })
                .show_field(ui, |ui| {
                    ui.selectable_value(&mut st.kind, KindFilter::All, i18n.t("ftr-type-all"));
                    ui.selectable_value(&mut st.kind, KindFilter::Names, i18n.t("ftr-type-names"));
                    ui.selectable_value(
                        &mut st.kind,
                        KindFilter::Descriptions,
                        i18n.t("ftr-type-descriptions"),
                    );
                    ui.selectable_value(&mut st.kind, KindFilter::Meta, i18n.t("ftr-type-meta"));
                });
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut st.search)
                    .hint_text(i18n.t("ftr-search-hint"))
                    .desired_width(220.0),
            );
            if !st.search.is_empty() && ui.small_button(crate::icons::X).clicked() {
                st.search.clear();
            }
            ui.separator();
            if ui
                .add_enabled(
                    st.doc.is_some(),
                    egui::Button::new(i18n.t("ftr-next-untranslated")),
                )
                .on_hover_text(crate::ui::app::key_text(
                    ui.ctx(),
                    i18n,
                    egui::Modifiers::COMMAND,
                    egui::Key::Enter,
                ))
                .clicked()
            {
                st.refresh_visible();
                st.next_untranslated();
            }
            ui.checkbox(&mut st.show_whitespace, i18n.t("ftr-show-whitespace"));
        });
        ui.add_space(2.0);
        // Row 4: productivity tools (memory, glossary, CSV).
        ui.horizontal(|ui| {
            let loaded = st.doc.is_some();
            let tm_count = st.memory.entries.len();
            if ui
                .add_enabled(
                    loaded && tm_count > 0,
                    egui::Button::new(i18n.t("ftr-apply-memory")),
                )
                .on_hover_text(i18n.t_num("ftr-memory-size", tm_count as i64))
                .clicked()
            {
                st.apply_memory(i18n);
            }
            if ui
                .add_enabled(
                    loaded && st.cursor.is_some(),
                    egui::Button::new(i18n.t("ftr-propagate")),
                )
                .on_hover_text(i18n.t("ftr-propagate-hint"))
                .clicked()
            {
                st.propagate_current(i18n);
            }
            ui.separator();
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("ftr-csv-export")))
                .clicked()
            {
                st.export_csv(i18n);
            }
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("ftr-csv-import")))
                .clicked()
            {
                st.import_csv(i18n);
            }
            ui.separator();
            // The numbered-list round trip with an AI of the user's choice
            // (kept from the first M-02): copy the visible sources, paste
            // the answer.
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("fomod-copy-list")))
                .on_hover_text(i18n.t("fomod-copy-list-tip"))
                .clicked()
            {
                let text = st.numbered_sources();
                ui.ctx().copy_text(text);
                st.set_message(
                    ToastLevel::Info,
                    i18n.t_num("fomod-copied", st.visible.len() as i64),
                );
            }
            if ui
                .add_enabled(loaded, egui::Button::new(i18n.t("fomod-paste-list")))
                .on_hover_text(i18n.t("fomod-paste-list-tip"))
                .clicked()
            {
                st.paste_open = !st.paste_open;
            }
            ui.separator();
            ui.toggle_value(
                &mut st.show_glossary,
                format!("{} ({})", i18n.t("ftr-glossary"), st.glossary.terms.len()),
            );
            if let Some(r) = &st.last_report {
                ui.separator();
                let mut args = fluent::FluentArgs::new();
                args.set("new", r.new as i64);
                args.set("changed", r.changed as i64);
                args.set("moved", r.moved as i64);
                args.set("removed", r.removed as i64);
                args.set("unchanged", r.unchanged as i64);
                ui.label(
                    RichText::new(i18n.t_with_args("ftr-update-report", Some(&args)))
                        .small()
                        .weak(),
                );
            }
        });
        if st.show_glossary {
            render_glossary(ui, st, i18n);
        }
        if st.paste_open {
            ui.label(RichText::new(i18n.t("fomod-paste-hint")).small().weak());
            ui.add(
                egui::TextEdit::multiline(&mut st.paste_buf)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                if ui.button(i18n.t("fomod-paste-apply")).clicked() {
                    let n = st.apply_numbered(&st.paste_buf.clone());
                    st.set_message(
                        ToastLevel::Success,
                        i18n.t_arg("fomod-pasted", "n", &n.to_string()),
                    );
                    if n > 0 {
                        st.paste_open = false;
                        st.paste_buf.clear();
                    }
                }
                if ui.button(i18n.t("btn-clear")).clicked() {
                    st.paste_buf.clear();
                }
                if ui.button(i18n.t("btn-close")).clicked() {
                    st.paste_open = false;
                }
            });
        }
        ui.add_space(4.0);

        // Pending load confirmation (unsaved edits).
        if let Some(pending) = st.pending_load.clone() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(i18n.t("ftr-discard-question")).color(ui.visuals().warn_fg_color),
                );
                if ui.button(i18n.t("ftr-discard-yes")).clicked() {
                    st.pending_load = None;
                    st.dirty = false;
                    match pending {
                        PendingLoad::Folder(p) => st.load_root(p, i18n),
                        PendingLoad::Archive(p) => st.archive_request = Some(p),
                    }
                }
                if ui.button(i18n.t("btn-cancel")).clicked() {
                    st.pending_load = None;
                }
            });
        }
        if st.close_requested {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(i18n.t("ftr-unsaved-close")).color(ui.visuals().warn_fg_color),
                );
                if ui.button(i18n.t("ftr-save")).clicked() && st.save(i18n) {
                    st.close_requested = false;
                    st.close_now = true;
                }
                if ui.button(i18n.t("ftr-discard-yes")).clicked() {
                    st.dirty = false;
                    st.close_requested = false;
                    st.close_now = true;
                }
                if ui.button(i18n.t("btn-cancel")).clicked() {
                    st.close_requested = false;
                }
            });
        }
    });
}

fn lang_combo(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut String,
    locales: &[String],
    i18n: &crate::i18n::I18n,
    enabled: bool,
) {
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(format!("{} ({})", i18n.display_name(value), value))
            .width(200.0)
            .show_field(ui, |ui| {
                for code in locales {
                    let label = format!("{} ({})", i18n.display_name(code), code);
                    ui.selectable_value(value, code.clone(), label);
                }
            });
    });
}

/// Inline glossary editor (terms whose translation is imposed, or that must
/// stay untranslated). Saved on every change.
fn render_glossary(ui: &mut egui::Ui, st: &mut FomodTranslationState, i18n: &crate::i18n::I18n) {
    let mut changed = false;
    let mut remove: Option<usize> = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::Grid::new("ftr_glossary")
            .num_columns(5)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new(i18n.t("ftr-glossary-source")).strong());
                ui.label(RichText::new(i18n.t("ftr-glossary-target")).strong());
                ui.label(RichText::new(i18n.t("ftr-glossary-case")).strong());
                ui.label(RichText::new(i18n.t("ftr-glossary-dnt")).strong());
                ui.label("");
                ui.end_row();
                for (i, term) in st.glossary.terms.iter_mut().enumerate() {
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut term.source).desired_width(160.0))
                        .lost_focus();
                    changed |= ui
                        .add_enabled(
                            !term.do_not_translate,
                            egui::TextEdit::singleline(&mut term.target).desired_width(160.0),
                        )
                        .lost_focus();
                    changed |= ui.checkbox(&mut term.case_sensitive, "").changed();
                    changed |= ui.checkbox(&mut term.do_not_translate, "").changed();
                    if ui.small_button(crate::icons::X).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if ui
            .button(format!("{} {}", "+", i18n.t("ftr-glossary-add")))
            .clicked()
        {
            st.glossary.terms.push(GlossaryTerm::default());
            changed = true;
        }
    });
    if let Some(i) = remove {
        st.glossary.terms.remove(i);
        changed = true;
    }
    if changed {
        st.glossary.source_lang = st.source_lang.clone();
        st.glossary.target_lang = st.target_lang.clone();
        if st.glossary.save().is_err() {
            tracing::warn!("glossary could not be saved");
        }
        // Glossary checks are part of the issue list.
        st.issues_revision = None;
        st.visible_key = None;
    }
}

/// Bottom panel: side-by-side source / target editor for the selected row.
fn render_detail(ctx: &egui::Context, st: &mut FomodTranslationState, i18n: &crate::i18n::I18n) {
    egui::TopBottomPanel::bottom("ftr_detail")
        .resizable(true)
        .default_height(200.0)
        .min_height(120.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            let Some(idx) = st.cursor else {
                ui.label(RichText::new(i18n.t("ftr-select-row")).weak());
                return;
            };
            let Some(doc) = st.doc.as_mut() else { return };
            let Some(unit) = doc.units.get_mut(idx) else {
                st.cursor = None;
                return;
            };
            let issues = st.issues.get(&unit.key).cloned().unwrap_or_default();
            let show_ws = st.show_whitespace;
            let mut changed = false;
            let mut online = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new(i18n.t(field_key(unit.field))).strong());
                if !unit.context.is_empty() {
                    ui.label(RichText::new(&unit.context).weak());
                }
                ui.label(RichText::new(&unit.key).monospace().small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .checkbox(&mut unit.locked, i18n.t("ftr-lock"))
                        .on_hover_text(i18n.t("ftr-lock-hint"))
                        .changed()
                    {
                        changed = true;
                    }
                    if ui.button(i18n.t("ftr-copy-source")).clicked() {
                        unit.target = unit.source.clone();
                        unit.status = TStatus::Translated;
                        changed = true;
                    }
                    if ui
                        .button(i18n.t("ftr-online"))
                        .on_hover_text(i18n.t("ftr-online-hint"))
                        .clicked()
                    {
                        online = true;
                    }
                    if ui.button(i18n.t("ftr-clear-target")).clicked() {
                        unit.target.clear();
                        unit.status = TStatus::Untranslated;
                        changed = true;
                    }
                });
            });
            let avail = ui.available_height() - 30.0;
            ui.columns(2, |cols| {
                cols[0].label(RichText::new(i18n.t("ftr-col-source")).small().weak());
                let src = if show_ws {
                    mark_whitespace(&unit.source)
                } else {
                    unit.source.clone()
                };
                egui::ScrollArea::vertical()
                    .id_salt("ftr_src_scroll")
                    .max_height(avail)
                    .show(&mut cols[0], |ui| {
                        let mut s = src.as_str();
                        ui.add(
                            egui::TextEdit::multiline(&mut s)
                                .desired_width(f32::INFINITY)
                                .desired_rows(4)
                                .interactive(false),
                        );
                    });
                cols[1].label(RichText::new(i18n.t("ftr-col-target")).small().weak());
                let editable = !unit.locked;
                egui::ScrollArea::vertical()
                    .id_salt("ftr_tgt_scroll")
                    .max_height(avail)
                    .show(&mut cols[1], |ui| {
                        let edit = egui::TextEdit::multiline(&mut unit.target)
                            .desired_width(f32::INFINITY)
                            .desired_rows(4)
                            .interactive(editable);
                        if ui.add(edit).changed() {
                            unit.status = if unit.target.trim().is_empty() {
                                TStatus::Untranslated
                            } else {
                                TStatus::Translated
                            };
                            changed = true;
                        }
                    });
            });
            // Translation-memory suggestion for this source.
            let suggestion = st.memory.suggest(&unit.source).map(|e| e.target.clone());
            if let Some(sug) = suggestion.filter(|s| s != &unit.target && !unit.locked) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(i18n.t("ftr-memory-suggestion"))
                            .small()
                            .weak(),
                    );
                    ui.add(egui::Label::new(RichText::new(&sug).small()).truncate());
                    if ui.small_button(i18n.t("ftr-use-suggestion")).clicked() {
                        unit.target = sug.clone();
                        unit.status = TStatus::Auto;
                        changed = true;
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} / {}",
                        unit.source.chars().count(),
                        unit.target.chars().count()
                    ))
                    .small()
                    .weak(),
                );
                for issue in &issues {
                    let color = if issue.is_blocking() {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().warn_fg_color
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            crate::icons::WARNING,
                            issue_text(i18n, issue)
                        ))
                        .small()
                        .color(color),
                    );
                }
                ui.label(RichText::new(i18n.t("ftr-note")).small().weak());
                if ui
                    .add(egui::TextEdit::singleline(&mut unit.note).desired_width(200.0))
                    .changed()
                {
                    changed = true;
                }
            });
            if changed {
                let key = st
                    .doc
                    .as_ref()
                    .and_then(|d| d.units.get(idx))
                    .map(|u| u.key.clone());
                if let Some(key) = key {
                    st.sync_identical(&key, i18n);
                }
                st.bump();
            }
            if online {
                st.request_online();
            }
        });
}

/// Central panel: the virtualised table of strings.
fn render_table(ctx: &egui::Context, st: &mut FomodTranslationState, i18n: &crate::i18n::I18n) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if st.doc.is_none() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(i18n.t("ftr-empty-hint")).weak());
            });
            return;
        }
        st.refresh_visible();
        let visible = st.visible.clone();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;

        let col_num = i18n.t("ftr-col-num");
        let col_status = i18n.t("ftr-col-status");
        let col_context = i18n.t("ftr-col-context");
        let col_source = i18n.t("ftr-col-source");
        let col_target = i18n.t("ftr-col-target");
        let col_issues = i18n.t("ftr-col-issues");
        let mut new_cursor: Option<usize> = None;
        let mut changed = false;
        let page = ((ui.available_height() - row_h * 2.0) / row_h.max(1.0))
            .floor()
            .max(1.0) as usize;

        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(44.0))
            .column(Column::exact(28.0))
            .column(Column::initial(220.0).clip(true).at_least(80.0))
            .column(Column::remainder().clip(true).at_least(120.0))
            .column(Column::remainder().clip(true).at_least(120.0))
            .column(Column::exact(32.0))
            .min_scrolled_height(0.0);
        // Keyboard: ↑ ↓ Page Up/Down Home End move the selected row (not
        // while a translation field is being typed in).
        let cur_pos = st.cursor.and_then(|c| visible.iter().position(|&i| i == c));
        if let Some(p) = crate::ui::grid::table_nav_keys(
            ctx,
            cur_pos,
            visible.len(),
            page,
            ctx.wants_keyboard_input(),
        ) {
            st.cursor = Some(visible[p]);
            st.scroll_to = Some(p);
        }
        if let Some(pos) = st.scroll_to.take() {
            table = table.scroll_to_row(pos, Some(egui::Align::Center));
        }
        let show_ws = st.show_whitespace;
        let cursor = st.cursor;
        let issues = &st.issues;
        let dup_count = &st.dup_count;
        let mut changed_key: Option<String> = None;
        let doc = st.doc.as_mut().expect("checked above");
        let empty = i18n.t("ftr-empty-filter");
        table
            .header(row_h, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new(&col_num).strong());
                });
                h.col(|ui| {
                    ui.label(RichText::new(&col_status).strong());
                });
                h.col(|ui| {
                    ui.label(RichText::new(&col_context).strong());
                });
                h.col(|ui| {
                    ui.label(RichText::new(&col_source).strong());
                });
                h.col(|ui| {
                    ui.label(RichText::new(&col_target).strong());
                });
                h.col(|ui| {
                    ui.label(RichText::new(&col_issues).strong());
                });
            })
            .body(|body| {
                if visible.is_empty() {
                    body.rows(row_h, 1, |mut row| {
                        row.col(|_| {});
                        row.col(|_| {});
                        row.col(|ui| {
                            ui.label(RichText::new(&empty).weak());
                        });
                        row.col(|_| {});
                        row.col(|_| {});
                        row.col(|_| {});
                    });
                    return;
                }
                body.rows(row_h, visible.len(), |mut row| {
                    let pos = row.index();
                    let idx = visible[pos];
                    let unit = &mut doc.units[idx];
                    row.set_selected(cursor == Some(idx));
                    // Read-only cells: plain, non-selectable text, so the pointer
                    // stays an arrow and a click anywhere on the row selects it
                    // (selectable labels would show a text cursor and swallow
                    // the click for text selection). Only the translation
                    // field is an input.
                    let plain =
                        |ui: &mut egui::Ui| ui.style_mut().interaction.selectable_labels = false;
                    row.col(|ui| {
                        plain(ui);
                        ui.label(RichText::new((idx + 1).to_string()).weak());
                        if let Some(&n) = dup_count.get(&idx).filter(|&&n| n > 1) {
                            ui.label(
                                RichText::new(i18n.t_num("strings-dup-badge", n as i64))
                                    .small()
                                    .weak(),
                            )
                            .on_hover_text(i18n.t_num("ftr-uniques-group", n as i64));
                        }
                    });
                    row.col(|ui| {
                        plain(ui);
                        let (glyph, key) = status_glyph(unit);
                        ui.label(glyph).on_hover_text(i18n.t(key));
                    });
                    row.col(|ui| {
                        plain(ui);
                        let ctx_text = if unit.context.is_empty() {
                            i18n.t(field_key(unit.field))
                        } else {
                            unit.context.clone()
                        };
                        ui.add(egui::Label::new(RichText::new(ctx_text).weak()).truncate());
                    });
                    row.col(|ui| {
                        plain(ui);
                        let one_line: String = unit.source.lines().next().unwrap_or("").to_string();
                        let one_line = if show_ws {
                            mark_whitespace(&one_line)
                        } else {
                            one_line
                        };
                        let more = unit.source.lines().count() > 1;
                        ui.add(
                            egui::Label::new(if more {
                                i18n.t_arg("text-cut", "text", &one_line)
                            } else {
                                one_line
                            })
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        if unit.field.is_multiline() || unit.locked {
                            plain(ui);
                            let one_line: String =
                                unit.target.lines().next().unwrap_or("").to_string();
                            let more = unit.target.lines().count() > 1;
                            let text = if more {
                                i18n.t_arg("text-cut", "text", &one_line)
                            } else {
                                one_line
                            };
                            if text.is_empty() {
                                ui.label(
                                    RichText::new(unit.source.lines().next().unwrap_or(""))
                                        .weak()
                                        .italics(),
                                );
                            } else {
                                ui.add(egui::Label::new(text).truncate());
                            }
                        } else {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut unit.target)
                                    .desired_width(f32::INFINITY)
                                    .hint_text(unit.source.as_str()),
                            );
                            if resp.gained_focus() {
                                new_cursor = Some(idx);
                            }
                            if resp.changed() {
                                unit.status = if unit.target.trim().is_empty() {
                                    TStatus::Untranslated
                                } else {
                                    TStatus::Translated
                                };
                                changed = true;
                                changed_key = Some(unit.key.clone());
                            }
                        }
                    });
                    row.col(|ui| {
                        if let Some(list) = issues.get(&unit.key).filter(|l| !l.is_empty()) {
                            let blocking = list.iter().any(|i| i.is_blocking());
                            let color = if blocking {
                                ui.visuals().error_fg_color
                            } else {
                                ui.visuals().warn_fg_color
                            };
                            let tip: Vec<String> =
                                list.iter().map(|i| issue_text(i18n, i)).collect();
                            ui.label(RichText::new(crate::icons::WARNING).color(color))
                                .on_hover_text(tip.join("\n"));
                        }
                    });
                    if row.response().clicked() {
                        new_cursor = Some(idx);
                    }
                });
            });
        if let Some(c) = new_cursor {
            st.cursor = Some(c);
        }
        if changed {
            if let Some(key) = changed_key {
                st.sync_identical(&key, i18n);
            }
            st.bump();
        }
    });
}

#[cfg(test)]
mod tests {

    /// The whole line reaches Google Translate: every « & », « # », « / »,
    /// space or line break of the text is encoded, so the query keeps its
    /// target and its text (the Windows opening cut it at the first « & »).
    use super::*;

    fn sample_fomod(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ximod_ftr_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fomod")).unwrap();
        std::fs::write(
            dir.join("fomod/info.xml"),
            "<fomod><Name>Guns &amp; Roses</Name><Author>Axl</Author><Description>Big mod</Description></fomod>",
        )
        .unwrap();
        std::fs::write(
            dir.join("fomod/ModuleConfig.xml"),
            r#"<config><moduleName>Guns &amp; Roses</moduleName><installSteps order="Explicit"><installStep name="Textures"><optionalFileGroups order="Explicit"><group name="Resolution" type="SelectExactlyOne"><plugins order="Explicit"><plugin name="4K"><description>High resolution.</description><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins></group></optionalFileGroups></installStep></installSteps></config>"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn numbered_list_round_trip() {
        let root = sample_fomod("numbered");
        let i18n = crate::i18n::I18n::new();
        let mut st = FomodTranslationState {
            source_lang: "eng".into(),
            target_lang: "fra".into(),
            ..Default::default()
        };
        st.load_root(root.clone(), &i18n);
        st.refresh_visible();
        let list = st.numbered_sources();
        assert!(list.starts_with("1. "), "{list}");
        let n = st
            .apply_numbered("1. Guns et Roses\n2) Axl\n\n4: Haute résolution.\nsuite\n99. ignoré");
        assert!(n >= 2, "{n}");
        let doc = st.doc.as_ref().unwrap();
        assert_eq!(doc.units[st.visible[0]].target, "Guns et Roses");
        assert!(st.dirty);
        assert_eq!(
            parse_numbered("3. a\n   b\n4. c"),
            vec![(3, "a\nb".to_string()), (4, "c".to_string())]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A root extracted with only its `fomod/` disables the full package.
    #[test]
    fn partial_root_disables_full_package() {
        let root = sample_fomod("partial");
        let i18n = crate::i18n::I18n::new();
        let mut st = FomodTranslationState {
            source_lang: "eng".into(),
            target_lang: "fra".into(),
            package_full: true,
            ..Default::default()
        };
        st.load_root_partial(root.clone(), &i18n);
        assert!(st.doc.is_some());
        assert!(st.partial);
        assert!(!st.package_full);
        st.load_root(root.clone(), &i18n);
        assert!(!st.partial);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_edit_save_export_round_trip() {
        let root = sample_fomod("roundtrip");
        let i18n = crate::i18n::I18n::new();
        let mut st = FomodTranslationState {
            source_lang: "eng".into(),
            target_lang: "fra".into(),
            ..Default::default()
        };
        st.load_root(root.clone(), &i18n);
        let doc = st.doc.as_ref().expect("loaded");
        assert!(
            doc.units
                .iter()
                .any(|u| u.key == "config/step[0]/name" && u.source == "Textures")
        );
        // Author is locked by default, website absent.
        assert!(
            doc.units
                .iter()
                .any(|u| u.field == TField::InfoAuthor && u.locked)
        );

        // Filters: everything is untranslated at first.
        st.filter = RowFilter::Untranslated;
        st.refresh_visible();
        let untranslated = st.visible.len();
        assert!(untranslated >= 4, "{untranslated}");

        // Translate the step name and save the sidecar.
        let idx = st
            .doc
            .as_ref()
            .unwrap()
            .units
            .iter()
            .position(|u| u.key == "config/step[0]/name")
            .unwrap();
        st.doc.as_mut().unwrap().units[idx].target = "Textures (FR)".into();
        st.doc.as_mut().unwrap().units[idx].status = TStatus::Translated;
        st.bump();
        assert!(st.dirty);
        assert!(st.save(&i18n));
        assert!(!st.dirty);
        let sidecar = st.sidecar.clone().unwrap();
        assert!(sidecar.is_file(), "{}", sidecar.display());

        // Export to the sibling folder: the original is untouched.
        st.export_mode = ExportMode::Sibling;
        st.export(&i18n);
        let out = std::fs::read_to_string(root.join("fomod_fra/ModuleConfig.xml")).unwrap();
        assert!(out.contains("installStep name=\"Textures (FR)\""), "{out}");
        assert!(
            out.contains("Guns &amp; Roses"),
            "untranslated strings stay as they were: {out}"
        );
        let orig = std::fs::read_to_string(root.join("fomod/ModuleConfig.xml")).unwrap();
        assert!(orig.contains("installStep name=\"Textures\""));

        // Re-opening the folder picks the sidecar up again.
        let mut st2 = FomodTranslationState {
            source_lang: "eng".into(),
            target_lang: "fra".into(),
            ..Default::default()
        };
        st2.load_root(root.clone(), &i18n);
        let u = st2
            .doc
            .as_ref()
            .unwrap()
            .units
            .iter()
            .find(|u| u.key == "config/step[0]/name")
            .unwrap();
        assert_eq!(u.target, "Textures (FR)");

        // The next-untranslated helper skips translated and locked rows.
        st2.filter = RowFilter::All;
        st2.refresh_visible();
        st2.next_untranslated();
        let c = st2.cursor.expect("cursor");
        let u = &st2.doc.as_ref().unwrap().units[c];
        assert!(u.target.is_empty() && !u.locked);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_glyphs() {
        let mut u = TUnit {
            key: "k".into(),
            field: TField::StepName,
            source: "a".into(),
            target: String::new(),
            status: TStatus::Untranslated,
            locked: false,
            context: String::new(),
            note: String::new(),
        };
        assert_eq!(status_glyph(&u).0, crate::icons::CIRCLE);
        u.target = "b".into();
        assert_eq!(status_glyph(&u).0, crate::icons::CHECK);
        u.locked = true;
        assert_eq!(status_glyph(&u).0, crate::icons::LOCK);
    }
}
