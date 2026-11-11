//! One document of the command line (D-06): opened as the interface opens
//! it — the lines read, the mod's dictionary recognized, the stack applied
//! with the rules around it —, then the operations of the library on it,
//! each recorded as the interface records it (one operation, its history
//! entries), and what it produced remembered for the manifest.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::basedb::FileDbCache;
use crate::config::AppConfig;
use crate::dict_stack::{DictStack, Level as StackLevel};
use crate::dicts::Role;
use crate::export::ExportPolicy;
use crate::headless::{self, Applied, Kind, Opened, TreeExtra};
use crate::i18n::I18n;
use crate::manifest::{self, Manifest};
use crate::plugin_id::Ident;
use crate::status::{self, Status};
use crate::textfmt::Formats;
use crate::undo::{OpKind, UndoStack, record};

/// What the session shares between its documents.
pub struct Ctx {
    pub cfg: AppConfig,
    pub formats: Formats,
    pub cache: FileDbCache,
    pub i18n: I18n,
    /// The journal of the session (E-01): written by --verbose and --log.
    pub journal: crate::journal::Journal,
}

/// The game and languages asked for (`--game`, `--from`, `--to`), the
/// configuration's when not given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pair {
    pub game: String,
    pub from: String,
    pub to: String,
}

/// Where the stack of a document comes from.
pub enum Source {
    /// The tree of the document, as the interface resolves it, plus what
    /// the command line adds.
    Tree(TreeExtra),
    /// One dictionary only: the mod's (export), or the one given.
    Only(Option<PathBuf>),
}

/// The outputs of `export` (S-01, S-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Strings,
    Hybrid,
    Override,
    Full,
    Dsd,
}

impl Output {
    pub const ALL: [Output; 5] = [
        Output::Strings,
        Output::Hybrid,
        Output::Override,
        Output::Full,
        Output::Dsd,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Output::Strings => "strings",
            Output::Hybrid => "hybrid",
            Output::Override => "override",
            Output::Full => "full",
            Output::Dsd => "dsd",
        }
    }

    pub fn from_id(s: &str) -> Option<Output> {
        Output::ALL.into_iter().find(|o| o.id() == s.trim())
    }
}

/// The checks that are errors: what breaks the game or the text (the
/// aliases, the numbers, the characters the page cannot hold); the others
/// are warnings.
pub fn is_error(c: crate::quality::Check) -> bool {
    use crate::quality::Check;
    matches!(c, Check::Aliases | Check::Numbers | Check::Encoding)
}

/// The document and what was done to it.
pub struct Work {
    pub o: Opened,
    pub undo: UndoStack,
    /// The mod's dictionary of the document (B-03), written or not.
    pub dict: PathBuf,
    pub dict_state: crate::scan::DictState,
    pub ident: Option<Ident>,
    /// The forced pages the mod's dictionary remembers (D-09).
    pub enc_in: Option<String>,
    pub enc_out: Option<String>,
    pub stack: Option<DictStack>,
    /// What the opening did: the stack and the rules.
    pub applied: Applied,
    pub checks: Option<manifest::Checks>,
    pub outputs: Vec<PathBuf>,
    pub statuses: Vec<String>,
    /// Lines of the mod's dictionary written by this session.
    pub dict_written: Option<usize>,
}

/// The dictionary given to `--dict` alone, or the mod's.
fn only_stack(path: &Path, cache: &mut FileDbCache) -> Option<DictStack> {
    let db = cache.get(path)?;
    Some(DictStack {
        levels: vec![StackLevel {
            role: Role::Mod,
            path: path.to_path_buf(),
            db,
        }],
        normalize: false,
    })
}

impl Work {
    /// Open a file as the interface opens it.
    pub fn open(ctx: &mut Ctx, path: &Path, pair: &Pair, source: Source) -> Result<Work, String> {
        let started = std::time::Instant::now();
        if !path.is_file() {
            return Err(crate::l10n!("cli-err-file", path = path.display()));
        }
        let kind = headless::kind_of(path, &ctx.formats)
            .ok_or_else(|| crate::l10n!("cli-err-kind", path = path.display()))?;
        let mut o = headless::read(
            path,
            kind,
            &pair.game,
            &pair.from,
            &pair.to,
            &ctx.cfg,
            &ctx.formats,
        )
        .map_err(|e| e.to_string())?;
        headless::remember(&ctx.cfg, &mut o);
        let dict = DictStack::mod_dict_path(path, &o.settings, &ctx.cfg.mod_base_dir());
        let ident = matches!(kind, Kind::Plugin | Kind::Text)
            .then(|| crate::plugin_id::of_file(path).ok())
            .flatten();
        let dict_state = if kind == Kind::Dictionary || kind == Kind::Dsd {
            crate::scan::DictState::Is
        } else if !o.settings.target.trim().is_empty() && dict.is_file() {
            match (crate::basedb::peek_ident(&dict), &ident) {
                (Some(d), Some(f)) if !d.same_content(f) => crate::scan::DictState::Other,
                _ => crate::scan::DictState::Known,
            }
        } else {
            crate::scan::DictState::Absent
        };
        let extra = crate::basedb::peek_extra(&dict);
        let stack = if matches!(kind, Kind::Dsd | Kind::Dictionary) {
            None
        } else {
            match source {
                Source::Tree(extra) => {
                    headless::stack_with(&ctx.cfg, path, &o.settings, &mut ctx.cache, &extra)
                }
                Source::Only(given) => {
                    let p = given.unwrap_or_else(|| dict.clone());
                    if !p.is_file() {
                        return Err(crate::l10n!("cli-err-file", path = p.display()));
                    }
                    only_stack(&p, &mut ctx.cache)
                }
            }
        };
        let plan = headless::rules_plan(&ctx.cfg, &o);
        let applied = headless::apply(&mut o, stack.as_ref(), &plan);
        // The journal (R-E01-02, R-E01-07): the opening, then the stack.
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let lines = o.tm.entries.iter().filter(|e| !e.unused).count();
        let text = crate::journal::opened_text(
            &ctx.i18n,
            &o.settings,
            lines,
            None,
            started.elapsed().as_secs_f32(),
        );
        let (info, open_k, dicts_k) = (
            crate::journal::Level::Info,
            crate::journal::Kind::Open,
            crate::journal::Kind::Dicts,
        );
        ctx.journal.push(info, open_k, Some(&name), &text);
        if stack.is_some() {
            for t in crate::journal::stack_texts(&ctx.i18n, &applied.report, &o.tm.entries) {
                ctx.journal.push(info, dicts_k, Some(&name), &t);
            }
        }
        Ok(Work {
            o,
            undo: UndoStack::default(),
            dict,
            dict_state,
            ident,
            enc_in: extra.enc_in,
            enc_out: extra.enc_out,
            stack,
            applied,
            checks: None,
            outputs: Vec::new(),
            statuses: Vec::new(),
            dict_written: None,
        })
    }

    pub fn needs_target(&self) -> Result<(), String> {
        if self.o.settings.target.trim().is_empty() {
            Err(crate::l10n!("dict-stack-no-target"))
        } else {
            Ok(())
        }
    }

    /// The levels of the stack and the lines each filled at the opening.
    pub fn levels(&self) -> Vec<manifest::Level> {
        let Some(s) = &self.stack else {
            return Vec::new();
        };
        s.levels
            .iter()
            .enumerate()
            .map(|(i, l)| manifest::Level {
                role: l.role.key().to_string(),
                file: l.name(),
                filled: self
                    .applied
                    .report
                    .per_level
                    .get(i)
                    .map(|x| x.1)
                    .unwrap_or(0),
            })
            .collect()
    }

    /// Lines shown in game, still to translate, locked (B-18, Ignoré).
    pub fn counts(&self) -> (usize, usize, usize) {
        headless::line_counts(&self.o.tm.entries)
    }

    /// Appliquer les dictionnaires (B-06) with a mode and a rule of
    /// overwriting, on the whole document: one operation.
    pub fn apply_mode(&mut self, ctx: &Ctx, opts: &crate::apply::Options) -> crate::apply::Plan {
        let Some(stack) = &self.stack else {
            return crate::apply::Plan::default();
        };
        let fuzzy = opts.mode == crate::apply::Mode::Fuzzy;
        let levels: Vec<crate::apply::Level> = stack
            .levels
            .iter()
            .map(|l| crate::apply::Level {
                name: l.name(),
                role: l.role,
                db: l.db.clone(),
                fuzzy: fuzzy
                    .then(|| {
                        let rows = l.db.rows_for_fuzzy();
                        let total = rows.len();
                        crate::fuzzy::FuzzyIndex::build(rows, total, |_, _| true).map(Arc::new)
                    })
                    .flatten(),
            })
            .collect();
        let lines: Vec<usize> = (0..self.o.tm.entries.len()).collect();
        let plan = crate::apply::plan(&self.o.tm.entries, &lines, &levels, opts, |_, _| true);
        if !plan.changes.is_empty() {
            let label = ctx
                .i18n
                .t_arg("op-apply-dicts", "n", &plan.changes.len().to_string());
            record(
                &mut self.undo,
                &mut self.o.tm.entries,
                OpKind::ApplyBase,
                Some(label),
                |es| crate::apply::commit(es, &plan),
            );
        }
        plan
    }

    /// Appliquer les assistants (B-15, B-16, B-21), the whole document.
    pub fn assist(&mut self, ctx: &Ctx, opts: &crate::assist::Options) -> crate::assist::Plan {
        let s = &self.o.settings;
        let a = crate::assist::Assistant::load(&s.pair_suffix(), s.game().id());
        let bases = crate::assist::StackBases(
            self.stack
                .as_ref()
                .map(|st| st.levels.iter().map(|l| l.db.clone()).collect())
                .unwrap_or_default(),
        );
        let teaches = if self.o.kind == Kind::Plugin {
            crate::assist::derive::teaches(&self.o.path)
        } else {
            Default::default()
        };
        let lang = s.target.trim().to_ascii_lowercase();
        let ctx_d = a.derive.as_deref().map(|f| crate::assist::DeriveCtx {
            file: f,
            lang: &lang,
            open: Vec::new(),
            parent_bases: &bases,
            teaches: &teaches,
        });
        let lines: Vec<usize> = (0..self.o.tm.entries.len()).collect();
        let plan =
            crate::assist::plan_all(&self.o.tm.entries, &lines, &a, opts, &bases, ctx_d.as_ref());
        if !plan.changes.is_empty() {
            let label = ctx
                .i18n
                .t_arg("assist-op-lines", "n", &plan.changes.len().to_string());
            record(
                &mut self.undo,
                &mut self.o.tm.entries,
                OpKind::Assist,
                Some(label),
                |es| crate::assist::commit(es, &plan),
            );
        }
        plan
    }

    /// The page the document is written in (D-09).
    pub fn out_page(&self) -> crate::codepage::Page {
        let s = &self.o.settings;
        let g = s.game();
        let loose = self.o.tm.meta.source_format != "plugin" && self.o.kind != Kind::Text;
        if g == crate::games::Game::Unknown || g.uses_utf8() || loose {
            return crate::codepage::Page::Utf8;
        }
        self.enc_out
            .as_deref()
            .and_then(crate::codepage::Page::from_id)
            .unwrap_or_else(|| crate::codepage::Pages::builtin().for_language(&s.target))
    }

    /// Vérifier la traduction (D-05, F12): the checks of the
    /// configuration on every line; (errors, warnings).
    pub fn check(&mut self, ctx: &Ctx) -> crate::quality::Report {
        let checks = crate::quality::Check::parse_set(&ctx.cfg.quality_checks);
        let lines: Vec<usize> = (0..self.o.tm.entries.len()).collect();
        let report = crate::quality::run_with_page(
            &self.o.tm.entries,
            &lines,
            &checks,
            self.o.settings.game(),
            self.out_page(),
            |_, _| true,
        );
        let errors = report.findings.iter().filter(|f| is_error(f.check)).count();
        self.checks = Some(manifest::Checks {
            errors,
            warnings: report.findings.len() - errors,
        });
        report
    }

    /// The memory as the writers see it (S-04), in the document's page.
    pub fn export(
        &mut self,
        output: Output,
        policy: ExportPolicy,
        dir: &Path,
        esl: bool,
    ) -> Result<Vec<(PathBuf, usize)>, String> {
        let page = self.out_page();
        crate::tm::encoding::with_output(Some(page), || self.export_in(output, policy, dir, esl))
    }

    fn export_in(
        &mut self,
        output: Output,
        policy: ExportPolicy,
        dir: &Path,
        esl: bool,
    ) -> Result<Vec<(PathBuf, usize)>, String> {
        let doc = &self.o;
        let game = crate::games::Game::from_id(&doc.tm.meta.game);
        let is_plugin = doc.tm.meta.source_format == "plugin";
        let filtered = policy.filtered(&doc.tm);
        let tm: &crate::tm::TranslationMemory = &filtered;
        let counts = policy.counts(&doc.tm);
        let utf8 = if is_plugin { game.uses_utf8() } else { true };
        let stem = doc
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let plugin_name = doc
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        std::fs::create_dir_all(dir)
            .map_err(|e| crate::l10n!("err-write-path", path = dir.display(), error = e))?;
        let write = |path: &Path, bytes: &[u8]| -> Result<(), String> {
            if crate::backup::same_file(path, &doc.path) {
                return Err(crate::l10n!("cli-export-same-file", path = path.display()));
            }
            crate::config::paths::write_atomic(path, bytes)
                .map_err(|e| crate::l10n!("err-write-path", path = path.display(), error = e))
        };
        let mut out = Vec::new();
        match output {
            Output::Full | Output::Override => {
                if !is_plugin {
                    return Err(crate::l10n!("status-export-needs-plugin"));
                }
                let src = std::fs::read(&doc.path).map_err(|e| {
                    crate::l10n!("err-write-path", path = doc.path.display(), error = e)
                })?;
                let bytes = if output == Output::Full {
                    crate::plugin::write_delocalized_esp(&src, tm, doc.strings.as_ref())
                        .map(|(b, _)| b)
                        .map_err(|e| e.to_string())?
                } else {
                    let opts = crate::plugin::OverrideOptions {
                        bundle: doc.strings.as_ref(),
                        esl_if_eligible: esl,
                    };
                    crate::plugin::write_override_esp_with(&src, tm, &plugin_name, &opts)
                        .map(|(b, _)| b)
                        .map_err(|e| e.to_string())?
                };
                let name = if output == Output::Override {
                    format!("{stem}_patch.esp")
                } else {
                    plugin_name.clone()
                };
                let path = dir.join(name);
                write(&path, &bytes)?;
                out.push((path, counts.total));
            }
            Output::Strings | Output::Hybrid => {
                if output == Output::Hybrid && !is_plugin {
                    return Err(crate::l10n!("status-hybrid-needs-plugin"));
                }
                let files = crate::plugin::build_strings_files(tm, utf8);
                if files.is_empty() {
                    return Err(crate::l10n!("status-export-no-strings"));
                }
                let lang = crate::lang::strings_suffix_for(game, &doc.settings.target);
                for (kind, bytes) in &files {
                    let path = dir.join(format!("{stem}_{lang}.{}", kind.extension()));
                    write(&path, bytes)?;
                    let n = crate::tm::load(&path).map(|t| t.entries.len()).unwrap_or(0);
                    out.push((path, n));
                }
            }
            Output::Dsd => {
                if !game.supports_dsd() {
                    return Err(crate::l10n!("warn-dsd-skyrim-only"));
                }
                let (bytes, n) = crate::tm::dsd::to_json_bytes(tm);
                if n == 0 {
                    return Err(crate::l10n!("status-export-no-strings"));
                }
                let path = dir.join(format!("{stem}.json"));
                write(&path, &bytes)?;
                out.push((path, n));
            }
        }
        self.statuses = ExportPolicy::optional_statuses()
            .filter(|s| policy.allows(*s))
            .chain(std::iter::once(Status::Validated))
            .map(|s| s.key().to_string())
            .collect();
        self.statuses.sort();
        for (p, _) in &out {
            if !self.outputs.contains(p) {
                self.outputs.push(p.clone());
            }
        }
        Ok(out)
    }

    /// Lines with a translation the policy does not export, or none.
    pub fn untranslated(&self, policy: &ExportPolicy) -> usize {
        self.o
            .tm
            .entries
            .iter()
            .filter(|e| {
                !e.unused
                    && !e.source.trim().is_empty()
                    && status::derive(e) != Status::Ignored
                    && !policy.exportable(e)
            })
            .count()
    }

    /// Write the mod's dictionary (B-03) and the history of the session
    /// with it (A-18, D-46).
    pub fn write_dict(&mut self, ctx: &Ctx) -> Result<usize, String> {
        self.needs_target()?;
        let rows = crate::mod_dict::rows(&self.o.tm.entries);
        if rows.is_empty() && !self.dict.is_file() {
            return Err(crate::l10n!("dict-mod-nothing"));
        }
        let n = crate::mod_dict::write(&crate::mod_dict::Write {
            entries: &self.o.tm.entries,
            rules: self.o.tm.meta.rules.clone(),
            rules_off: self.o.tm.meta.rules_off.clone(),
            enc_in: self.enc_in.clone(),
            enc_out: self.enc_out.clone(),
            lang: self.o.settings.target.trim(),
            out: &self.dict,
            ident: self.ident.as_ref(),
            backup_copies: ctx.cfg.backup_copies,
            old_months: ctx.cfg.old_data_months,
        })?;
        let log = self.undo.take_log();
        if ctx.cfg.history_enabled && !log.is_empty() {
            let path = crate::history::file_for(&self.dict);
            let mut h = crate::history::History::default();
            h.load(&path);
            h.record(&log, &self.o.tm.entries, ctx.cfg.history_max);
            if let Err(e) = h.save(&path, ctx.cfg.history_max) {
                tracing::warn!("history not written to {}: {e}", path.display());
            }
        }
        self.dict_written = Some(n);
        Ok(n)
    }

    /// The manifest of the work (R-D06-04): paths relative to `base`.
    pub fn manifest(&self, base: Option<&Path>) -> Manifest {
        let s = &self.o.settings;
        let mut m = Manifest::new(s.game.trim(), s.source.trim(), s.target.trim());
        if let Ok(i) = manifest::input(&self.o.path, base) {
            m.inputs.push(i);
        }
        if self.dict.is_file() {
            let rows = crate::mod_dict::rows(&self.o.tm.entries);
            let mut d = manifest::Dictionary {
                path: manifest::rel(&self.dict, base),
                lines: rows.len(),
                ..Default::default()
            };
            for e in self.o.tm.entries.iter().filter(|e| !e.unused) {
                match status::derive(e) {
                    Status::Validated | Status::Custom => d.validated += 1,
                    Status::Ignored => d.ignored += 1,
                    Status::ToTranslate => {}
                    _ => d.pending += 1,
                }
            }
            m.dictionary = Some(d);
        }
        m.stack = self.levels();
        m.statuses = self.statuses.clone();
        for p in &self.outputs {
            if let Ok(o) = manifest::output(p, base) {
                m.outputs.push(o);
            }
        }
        m.checks = self.checks;
        m
    }
}

/// The key of a service: `XIMOD_KEY_<SERVICE>` first (continuous
/// integration), then the keyring or `secrets.json` as for the interface
/// (R-D06-05). Never echoed.
pub fn service_key(service: &crate::online::Service) -> Option<String> {
    if let Ok(k) = std::env::var(key_var(service))
        && !k.trim().is_empty()
    {
        return Some(k.trim().to_string());
    }
    crate::online::secrets::get(service).0
}

/// `XIMOD_KEY_OPENAI_LOCAL` for `openai:local`.
pub fn key_var(service: &crate::online::Service) -> String {
    let id: String = service
        .id()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("XIMOD_KEY_{id}")
}

/// What `translate` did.
#[derive(Debug, Default)]
pub struct Translated {
    pub lines: usize,
    pub items: usize,
    pub cached: usize,
    pub chars: usize,
    pub skipped: crate::online::mass::Skipped,
    pub summary: crate::online::mass::Summary,
    pub stopped: bool,
}

/// Why `translate` did not send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotSent {
    /// Over `--max-chars`: (chars, max).
    Over(usize, usize),
    /// Nothing to send.
    Nothing,
}

impl Work {
    /// Traduire en ligne (D-04), the whole document or the lines to
    /// translate; Validé and Personnalisé are never replaced (nobody to
    /// ask, R-D01-02). Refuses beyond `max_chars` without sending.
    pub fn translate(
        &mut self,
        ctx: &Ctx,
        service: &crate::online::Service,
        key: String,
        only_untranslated: bool,
        max_chars: usize,
    ) -> Result<Translated, NotSent> {
        use crate::online::mass;
        let es = &self.o.tm.entries;
        let (mut lines, mut skipped) = mass::select(es, 0..es.len(), only_untranslated);
        let before = lines.len();
        lines.retain(|&i| {
            !es.get(i).is_some_and(|e| {
                !e.trans.is_empty()
                    && matches!(status::derive(e), Status::Validated | Status::Custom)
            })
        });
        skipped.done += before - lines.len();
        let s = &self.o.settings;
        let iso1 = |code: &str| {
            ctx.i18n
                .iso3_to_iso1(code)
                .map(|s| s.to_lowercase())
                .unwrap_or_else(|| crate::online::iso1(code))
        };
        let (src, dst) = (iso1(s.source.trim()), iso1(s.target.trim()));
        let oc = &ctx.cfg.online;
        let opts = crate::online::protect::Options {
            tags: oc.protect_tags,
            numbers: oc.protect_numbers,
            newlines: oc.protect_newlines,
        };
        let prepared = mass::prepare(es, &lines, s.game(), opts);
        if prepared.items.is_empty() {
            return Err(NotSent::Nothing);
        }
        let cache_path = crate::online::cache::Cache::path_for(service, &src, &dst);
        let cache = crate::online::cache::Cache::open_at(cache_path.clone(), &dst);
        let chars: usize = prepared
            .items
            .iter()
            .filter(|i| cache.get(&i.protected).is_none())
            .map(|i| i.protected.chars().count())
            .sum();
        if chars > max_chars {
            return Err(NotSent::Over(chars, max_chars));
        }
        let mut t = Translated {
            lines: prepared.lines.len(),
            items: prepared.items.len(),
            cached: prepared.cached(&cache),
            chars,
            skipped,
            ..Default::default()
        };
        let mut cfg = oc.for_service(service);
        cfg.game = s.game_label();
        let input = crate::online::job::JobInput {
            service: service.clone(),
            cfg,
            key,
            src,
            dst,
            items: prepared.job_items(),
            cache_path: Some(cache_path),
        };
        let rx = crate::online::job::start(input, Arc::new(AtomicBool::new(false)));
        let c = mass::collect(rx, |_, _| {});
        t.stopped = c.stopped;
        let outcomes = mass::outcomes(&prepared.items, &c.answers);
        let prov = crate::provenance::online(&service.id());
        let label = service.short_label(&ctx.i18n);
        let targets = prepared.lines.clone();
        t.summary = record(
            &mut self.undo,
            &mut self.o.tm.entries,
            OpKind::Online,
            Some(label),
            |entries| mass::apply(entries, &targets, &outcomes, &prov),
        );
        Ok(t)
    }
}
