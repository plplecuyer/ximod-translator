//! Analyser un dossier (D-07, R-D07-01 / 02 / 03 / 05, D-90): every file
//! the application reads under a folder, and for each what its dictionary
//! stack would make of it — lines, translated by the stack, left, locked,
//! the state of the mod's dictionary — the stack applied « à blanc »:
//! nothing is opened, nothing is written. The bases are indexed once for
//! the whole analysis.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use crate::basedb::FileDbCache;
use crate::config::AppConfig;
use crate::dict_stack::DictStack;
use crate::headless::{self, Kind};
use crate::textfmt::Formats;

/// The kinds of files taken (figure B.6, 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub subdirs: bool,
    pub plugins: bool,
    pub texts: bool,
    pub scripts: bool,
    pub dsd: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            subdirs: true,
            plugins: true,
            texts: true,
            scripts: false,
            dsd: true,
        }
    }
}

/// The mod's dictionary of a file (B-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictState {
    /// Written for this very file.
    Known,
    /// Written for another version of it.
    Other,
    Absent,
    /// The file is itself a dictionary or a translation.
    Is,
}

impl DictState {
    pub fn code(self) -> &'static str {
        match self {
            DictState::Known => "known",
            DictState::Other => "other",
            DictState::Absent => "none",
            DictState::Is => "is",
        }
    }
}

/// Where a file stands (R-D07-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Done,
    Almost,
    Todo,
    Update,
    Source,
    /// A dictionary of the application: listed, not analysed.
    Dictionary,
    Error,
}

impl State {
    pub fn code(self) -> &'static str {
        match self {
            State::Done => "done",
            State::Almost => "almost",
            State::Todo => "todo",
            State::Update => "update",
            State::Source => "source",
            State::Dictionary => "dictionary",
            State::Error => "error",
        }
    }
}

/// One file of the analysis.
#[derive(Debug, Clone)]
pub struct FileReport {
    pub path: PathBuf,
    /// The path under the folder analysed.
    pub rel: String,
    pub kind: Kind,
    pub game: String,
    pub lines: usize,
    pub filled: usize,
    pub left: usize,
    pub locked: usize,
    /// Lines per dictionary of the stack.
    pub per_level: Vec<(String, usize)>,
    pub dict: DictState,
    pub state: State,
    pub error: Option<String>,
}

/// The files of a folder the analysis takes, in the order of their paths
/// (archives and STRINGS are not walked).
pub fn list(root: &Path, opts: Options, formats: &Formats) -> Vec<(PathBuf, Kind)> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if opts.subdirs {
                    dirs.push(p);
                }
                continue;
            }
            let Some(kind) = headless::kind_of(&p, formats) else {
                continue;
            };
            let wanted = match kind {
                Kind::Plugin => opts.plugins,
                Kind::Text => opts.texts,
                Kind::Script => opts.scripts,
                Kind::Dsd => opts.dsd,
                Kind::Dictionary => true,
            };
            if wanted {
                out.push((p, kind));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The analysis of one folder: the configuration, the pair, the indexes
/// of the dictionaries read so far.
pub struct Scanner {
    pub cfg: AppConfig,
    pub formats: Formats,
    pub game: String,
    pub source: String,
    pub target: String,
    pub root: PathBuf,
    cache: FileDbCache,
}

impl Scanner {
    pub fn new(cfg: AppConfig, root: &Path, game: &str, source: &str, target: &str) -> Scanner {
        Scanner {
            cfg,
            formats: Formats::load(),
            game: game.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            root: root.to_path_buf(),
            cache: FileDbCache::default(),
        }
    }

    /// One file, the stack applied in memory (R-D07-02).
    pub fn analyze(&mut self, path: &Path, kind: Kind) -> FileReport {
        let rel = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut r = FileReport {
            path: path.to_path_buf(),
            rel,
            kind,
            game: self.game.clone(),
            lines: 0,
            filled: 0,
            left: 0,
            locked: 0,
            per_level: Vec::new(),
            dict: DictState::Absent,
            state: State::Todo,
            error: None,
        };
        if kind == Kind::Dictionary {
            r.dict = DictState::Is;
            r.state = State::Dictionary;
            r.game = String::new();
            return r;
        }
        let mut o = match headless::read(
            path,
            kind,
            &self.game,
            &self.source,
            &self.target,
            &self.cfg,
            &self.formats,
        ) {
            Ok(o) => o,
            Err(e) => {
                r.error = Some(e.to_string());
                r.state = State::Error;
                return r;
            }
        };
        r.game = o.settings.game.clone();
        // A translation or a dictionary: counted, not translated.
        if matches!(kind, Kind::Dsd | Kind::Dictionary) {
            for e in &o.tm.entries {
                if e.source.trim().is_empty() {
                    continue;
                }
                r.lines += 1;
                if e.trans.trim().is_empty() {
                    r.left += 1;
                } else {
                    r.filled += 1;
                }
            }
            r.dict = DictState::Is;
            r.state = State::Source;
            return r;
        }
        headless::remember(&self.cfg, &mut o);
        let stack = headless::stack(&self.cfg, path, &o.settings, &mut self.cache);
        let plan = headless::rules_plan(&self.cfg, &o);
        let a = headless::apply(&mut o, stack.as_ref(), &plan);
        r.lines = a.lines;
        r.filled = a.filled;
        r.left = a.left;
        r.locked = a.locked;
        r.per_level = a.report.per_level.into_iter().filter(|x| x.1 > 0).collect();
        // The mod's dictionary (B-12).
        let mod_path = DictStack::mod_dict_path(path, &o.settings, &self.cfg.mod_base_dir());
        let mut changed = 0usize;
        if !self.target.trim().is_empty() && mod_path.is_file() {
            r.dict = match (
                crate::basedb::peek_ident(&mod_path),
                crate::plugin_id::of_file(path).ok(),
            ) {
                (Some(d), Some(f)) if !d.same_content(&f) => DictState::Other,
                _ => DictState::Known,
            };
            if r.dict == DictState::Other
                && let Some(db) = self.cache.get(&mod_path)
            {
                changed =
                    o.tm.entries
                        .iter()
                        .filter(|e| !e.unused && db.lookup_by_id(e).is_some())
                        .count();
            }
        }
        // A plugin in the target language is a translation.
        let in_target = kind == Kind::Plugin
            && r.lines >= 10
            && !self.target.trim().is_empty()
            && crate::langdetect::detect_texts(
                o.tm.entries
                    .iter()
                    .filter(|e| !e.unused)
                    .map(|e| e.source.as_str()),
            )
            .first()
            .is_some_and(|c| c.iso3 == self.target.trim());
        r.state = if in_target {
            State::Source
        } else if r.dict == DictState::Other && changed > 0 {
            State::Update
        } else if r.left == 0 {
            State::Done
        } else if r.left * 20 < r.lines {
            State::Almost
        } else {
            State::Todo
        };
        r
    }
}

/// What the analysis reports while it runs.
#[derive(Debug, Clone)]
pub enum Msg {
    File(Box<FileReport>),
    /// The end: seconds, stopped before the end.
    Done {
        secs: f32,
        stopped: bool,
    },
}

/// Analyse `files` in a thread; the flag stops it after the current file.
pub fn start(
    mut scanner: Scanner,
    files: Vec<(PathBuf, Kind)>,
    stop: Arc<AtomicBool>,
) -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let mut stopped = false;
        for (p, k) in files {
            if stop.load(Ordering::Relaxed) {
                stopped = true;
                break;
            }
            let r = scanner.analyze(&p, k);
            if tx.send(Msg::File(Box::new(r))).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::Done {
            secs: t0.elapsed().as_secs_f32(),
            stopped,
        });
    });
    rx
}

/// The totals of the analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub files: usize,
    pub lines: usize,
    pub filled: usize,
    pub left: usize,
    pub locked: usize,
}

pub fn totals(reports: &[FileReport]) -> Totals {
    let mut t = Totals::default();
    for r in reports {
        t.files += 1;
        if matches!(r.kind, Kind::Dsd | Kind::Dictionary) {
            continue;
        }
        t.lines += r.lines;
        t.filled += r.filled;
        t.left += r.left;
        t.locked += r.locked;
    }
    t
}

/// A CSV cell: `;`-separated, quoted when needed.
pub fn csv_cell(s: &str) -> String {
    if s.contains(';') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The report as CSV (R-D07-05): UTF-8 with BOM, `;`, a header, one line
/// per file; dictionary and state as codes.
pub fn to_csv(reports: &[FileReport]) -> String {
    let mut s = String::from("\u{feff}file;game;lines;translated;left;locked;dictionary;state\r\n");
    for r in reports {
        let cells = [
            csv_cell(&r.rel),
            csv_cell(&r.game),
            r.lines.to_string(),
            r.filled.to_string(),
            r.left.to_string(),
            r.locked.to_string(),
            r.dict.code().to_string(),
            r.state.code().to_string(),
        ];
        s.push_str(&cells.join(";"));
        s.push_str("\r\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_are_quoted() {
        assert_eq!(csv_cell("a;b"), "\"a;b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }
}
