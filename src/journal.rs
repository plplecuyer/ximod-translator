//! The journal of the session (E-01, decision D-108): the operations of the
//! session in the interface language, kept in memory, shown by Aide ›
//! Journal… and saved on demand. Each entry is also emitted through
//! `tracing`, so `logs/ximod.log` keeps it between sessions with no new
//! mechanism (R-E01-03).
//!
//! Shared by the interface and `ximod-cli`; the module knows no language:
//! the caller renders the text of an entry.

use std::collections::VecDeque;
use std::time::SystemTime;

/// At most this many entries in memory; beyond, the oldest are dropped
/// (R-E01-01).
pub const CAPACITY: usize = 10_000;

/// At most this many lines kept per entry; beyond, the count alone
/// (R-E01-04).
pub const MAX_ROWS: usize = 5_000;

/// The severity of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    /// Stable name (JSON of the command line).
    pub fn id(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warning",
            Level::Error => "error",
        }
    }

    /// Key of its label (tab-separated text, the window's list).
    pub fn i18n_key(self) -> &'static str {
        match self {
            Level::Info => "journal-lv-info",
            Level::Warn => "journal-lv-warn",
            Level::Error => "journal-lv-err",
        }
    }
}

/// The nature of an entry, for the JSON and later filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Start,
    Open,
    Close,
    Dicts,
    Apply,
    Edit,
    Transform,
    Assist,
    Online,
    Check,
    Save,
    Export,
    Package,
    Scan,
    Tool,
    Config,
    /// A message of the status line or an alert, with no closer nature.
    Message,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Start => "start",
            Kind::Open => "open",
            Kind::Close => "close",
            Kind::Dicts => "dicts",
            Kind::Apply => "apply",
            Kind::Edit => "edit",
            Kind::Transform => "transform",
            Kind::Assist => "assist",
            Kind::Online => "online",
            Kind::Check => "check",
            Kind::Save => "save",
            Kind::Export => "export",
            Kind::Package => "package",
            Kind::Scan => "scan",
            Kind::Tool => "tool",
            Kind::Config => "config",
            Kind::Message => "message",
        }
    }
}

/// An entry of the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Rising number in the session (stable when the oldest entries are
    /// dropped: the window's selection holds on to it).
    pub id: u64,
    /// Unix seconds.
    pub at: i64,
    pub level: Level,
    pub kind: Kind,
    /// The name of the document concerned, when there is one.
    pub doc: Option<String>,
    /// The message, already rendered in the interface language.
    pub text: String,
    /// The lines concerned (indexes in the document), at most [`MAX_ROWS`].
    pub rows: Vec<usize>,
    /// How many lines are concerned, even beyond [`MAX_ROWS`].
    pub rows_total: usize,
}

/// The journal of the session: one for the application, shared by the
/// tabs.
#[derive(Debug, Clone)]
pub struct Journal {
    entries: VecDeque<Entry>,
    capacity: usize,
    next_id: u64,
    /// Entries dropped for room since the start of the session.
    trimmed: usize,
    /// Warnings and errors since the window was last opened.
    unread_warn: usize,
    unread_err: usize,
    /// Entries already written by the command line (--verbose, --log).
    flushed: u64,
    /// Also emit each entry through `tracing` (off in tests and the CLI).
    trace: bool,
}

impl Default for Journal {
    fn default() -> Self {
        Journal::new()
    }
}

impl Journal {
    pub fn new() -> Journal {
        Journal::with_capacity(CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Journal {
        Journal {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
            next_id: 1,
            trimmed: 0,
            unread_warn: 0,
            unread_err: 0,
            flushed: 0,
            trace: true,
        }
    }

    /// No more emission through `tracing` (tests, command line).
    pub fn without_trace(mut self) -> Journal {
        self.trace = false;
        self
    }

    /// Add an entry; its number.
    pub fn push(&mut self, level: Level, kind: Kind, doc: Option<&str>, text: &str) -> u64 {
        self.push_rows(level, kind, doc, text, Vec::new())
    }

    /// Add an entry about lines (R-E01-04).
    pub fn push_rows(
        &mut self,
        level: Level,
        kind: Kind,
        doc: Option<&str>,
        text: &str,
        mut rows: Vec<usize>,
    ) -> u64 {
        let rows_total = rows.len();
        rows.sort_unstable();
        rows.dedup();
        rows.truncate(MAX_ROWS);
        let id = self.next_id;
        self.next_id += 1;
        let text = one_line(text);
        if self.trace {
            let doc_s = doc.unwrap_or("");
            match level {
                Level::Info => {
                    tracing::info!(target: "ximod_translator::journal", kind = kind.id(), doc = doc_s, "{text}")
                }
                Level::Warn => {
                    tracing::warn!(target: "ximod_translator::journal", kind = kind.id(), doc = doc_s, "{text}")
                }
                Level::Error => {
                    tracing::error!(target: "ximod_translator::journal", kind = kind.id(), doc = doc_s, "{text}")
                }
            }
        }
        match level {
            Level::Info => {}
            Level::Warn => self.unread_warn += 1,
            Level::Error => self.unread_err += 1,
        }
        self.entries.push_back(Entry {
            id,
            at: now_secs(),
            level,
            kind,
            doc: doc.filter(|d| !d.is_empty()).map(str::to_string),
            text,
            rows,
            rows_total,
        });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
            self.trimmed += 1;
        }
        id
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        // The numbers rise and are contiguous in the queue.
        let first = self.entries.front()?.id;
        let i = id.checked_sub(first)? as usize;
        self.entries.get(i).filter(|e| e.id == id)
    }

    /// Entries dropped for room since the start of the session.
    pub fn trimmed(&self) -> usize {
        self.trimmed
    }

    /// Unread `(warnings, errors)`.
    pub fn unread(&self) -> (usize, usize) {
        (self.unread_warn, self.unread_err)
    }

    /// The window was opened: nothing is pending any more (R-E01-06).
    pub fn mark_read(&mut self) {
        self.unread_warn = 0;
        self.unread_err = 0;
    }

    /// Has the session recorded more than its start? (the status bar
    /// button shows only then, R-E01-06).
    pub fn has_activity(&self) -> bool {
        self.trimmed > 0 || self.entries.iter().any(|e| e.kind != Kind::Start)
    }

    /// Entries per level: `(info, warnings, errors)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for e in &self.entries {
            match e.level {
                Level::Info => c.0 += 1,
                Level::Warn => c.1 += 1,
                Level::Error => c.2 += 1,
            }
        }
        c
    }

    /// Empty the list of the session (R-E01-05); the numbers go on.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.trimmed = 0;
        self.mark_read();
    }

    /// The entries the command line has not written yet, from then on
    /// held as written.
    pub fn take_unflushed(&mut self) -> Vec<Entry> {
        let from = self.flushed;
        let out: Vec<Entry> = self
            .entries
            .iter()
            .filter(|e| e.id > from)
            .cloned()
            .collect();
        if let Some(last) = self.entries.back() {
            self.flushed = self.flushed.max(last.id);
        }
        out
    }

    /// The document names present, in order of arrival (filter).
    pub fn docs(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for e in &self.entries {
            if let Some(d) = &e.doc
                && !v.contains(d)
            {
                v.push(d.clone());
            }
        }
        v
    }
}

/// What the window shows (R-E01-05).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    /// Lowest level shown: `None` or `Info` = all.
    pub min_level: Option<Level>,
    /// One document only.
    pub doc: Option<String>,
    /// Text looked for, ignoring case and accents, in the message and the
    /// document.
    pub text: String,
}

impl View {
    pub fn shows(&self, e: &Entry) -> bool {
        if let Some(min) = self.min_level
            && e.level < min
        {
            return false;
        }
        if let Some(d) = &self.doc
            && e.doc.as_ref() != Some(d)
        {
            return false;
        }
        let t = fold(self.text.trim());
        if t.is_empty() {
            return true;
        }
        fold(&e.text).contains(&t) || e.doc.as_deref().is_some_and(|d| fold(d).contains(&t))
    }

    /// The entries shown, in order.
    pub fn filter<'a>(&'a self, j: &'a Journal) -> impl Iterator<Item = &'a Entry> + 'a {
        j.entries().filter(move |e| self.shows(e))
    }
}

/// Lower case without accents, for the text filter.
pub fn fold(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The local time `HH:MM:SS` of an entry.
pub fn time_text(at: i64) -> String {
    let (_, _, _, h, m, s) = crate::backup::local_datetime(at);
    format!("{h:02}:{m:02}:{s:02}")
}

/// One line of the tab-separated text « time ⇥ level ⇥ document ⇥ message »
/// (R-E01-05, R-E01-07); `level` is the label of the level in the
/// language wanted.
pub fn tsv_line(e: &Entry, level: &str) -> String {
    format!(
        "{}\t{}\t{}\t{}",
        time_text(e.at),
        level,
        e.doc.as_deref().unwrap_or(""),
        e.text
    )
}

/// The tab-separated text of several entries, one per line, line end
/// included.
pub fn tsv<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    level: impl Fn(Level) -> String,
) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&tsv_line(e, &level(e.level)));
        out.push('\n');
    }
    out
}

/// The JSON object of an entry (`ximod-cli --json --verbose`).
pub fn json(e: &Entry) -> serde_json::Value {
    serde_json::json!({
        "at": e.at,
        "level": e.level.id(),
        "kind": e.kind.id(),
        "doc": e.doc,
        "text": e.text,
    })
}

/// The name offered when saving: `ximod-journal-<AAAAMMJJ-HHMM>.txt`.
pub fn file_name(at: i64) -> String {
    let (y, mo, d, h, mi, _) = crate::backup::local_datetime(at);
    format!("ximod-journal-{y:04}{mo:02}{d:02}-{h:02}{mi:02}.txt")
}

/// The entries of an applied dictionary stack (R-E01-02): one level per
/// entry with its lines, then the total; shared by the interface and the
/// command line.
pub fn stack_texts(
    i18n: &crate::i18n::I18n,
    report: &crate::dict_stack::StackReport,
    entries: &[crate::tm::TmEntry],
) -> Vec<String> {
    let fmt = |n: usize| crate::ui::export_dialog::fmt_count(i18n, n);
    let (_, left, locked) = crate::headless::line_counts(entries);
    let mut out: Vec<String> = report
        .per_level
        .iter()
        .map(|(level, n)| {
            let file = std::path::Path::new(level)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| level.clone());
            i18n.t_args("journal-dict-level", &[("name", &file), ("n", &fmt(*n))])
        })
        .collect();
    out.push(i18n.t_args(
        "journal-dict-total",
        &[
            ("n", &fmt(report.total())),
            ("left", &fmt(left)),
            ("locked", &fmt(locked)),
            ("norm", &fmt(report.normalized)),
            ("amb", &fmt(report.ambiguous)),
        ],
    ));
    out
}

/// The text of an opening (R-E01-02): game, languages, lines, encoding,
/// duration; « — » for what is not known.
pub fn opened_text(
    i18n: &crate::i18n::I18n,
    settings: &crate::doc_settings::DocSettings,
    lines: usize,
    enc: Option<&str>,
    secs: f32,
) -> String {
    let dash = || "—".to_string();
    let lang = |code: &str| {
        if code.trim().is_empty() {
            dash()
        } else {
            i18n.display_name(code)
        }
    };
    let game = if settings.game.trim().is_empty() {
        dash()
    } else {
        settings.game_label()
    };
    let secs = format!("{secs:.1}");
    let secs = if matches!(
        i18n.current_locale(),
        "eng" | "gle" | "mlt" | "jpn" | "kor" | "zho" | "roh"
    ) {
        secs
    } else {
        secs.replace('.', ",")
    };
    i18n.t_args(
        "journal-opened",
        &[
            ("game", &game),
            ("src", &lang(&settings.source)),
            ("dst", &lang(&settings.target)),
            ("n", &crate::ui::export_dialog::fmt_count(i18n, lines)),
            ("enc", &enc.map(str::to_string).unwrap_or_else(dash)),
            ("secs", &secs),
        ],
    )
}

/// A message holds on one line of the journal: line breaks and tabs
/// become spaces.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c == '\n' || c == '\r' || c == '\t' {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = c == ' ';
        }
    }
    out.trim_end().to_string()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j() -> Journal {
        Journal::new().without_trace()
    }

    #[test]
    fn levels_count_unread_until_read() {
        let mut j = j();
        j.push(Level::Info, Kind::Start, None, "start");
        assert!(!j.has_activity());
        j.push(Level::Warn, Kind::Export, Some("a.esp"), "w");
        j.push(Level::Error, Kind::Export, Some("a.esp"), "e");
        j.push(Level::Error, Kind::Message, None, "e2");
        assert!(j.has_activity());
        assert_eq!(j.unread(), (1, 2));
        assert_eq!(j.counts(), (1, 1, 2));
        j.mark_read();
        assert_eq!(j.unread(), (0, 0));
        assert_eq!(j.counts(), (1, 1, 2));
    }

    #[test]
    fn capacity_retires_the_oldest() {
        let mut j = Journal::with_capacity(3).without_trace();
        for i in 0..5 {
            j.push(Level::Info, Kind::Message, None, &format!("m{i}"));
        }
        assert_eq!(j.len(), 3);
        assert_eq!(j.trimmed(), 2);
        let texts: Vec<&str> = j.entries().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["m2", "m3", "m4"]);
        // The numbers stay stable.
        assert_eq!(j.get(3).map(|e| e.text.as_str()), Some("m2"));
        assert!(j.get(1).is_none());
    }

    #[test]
    fn ten_thousand_and_one() {
        let mut j = j();
        for i in 0..=CAPACITY {
            j.push(Level::Info, Kind::Message, None, &i.to_string());
        }
        assert_eq!(j.len(), CAPACITY);
        assert_eq!(j.trimmed(), 1);
        assert_eq!(j.entries().next().map(|e| e.text.as_str()), Some("1"));
    }

    #[test]
    fn rows_are_sorted_deduplicated_and_bounded() {
        let mut j = j();
        let mut rows: Vec<usize> = (0..MAX_ROWS + 10).rev().collect();
        rows.push(3);
        let id = j.push_rows(Level::Warn, Kind::Check, Some("d"), "x", rows);
        let e = j.get(id).unwrap();
        assert_eq!(e.rows.len(), MAX_ROWS);
        assert_eq!(e.rows_total, MAX_ROWS + 11);
        assert_eq!(e.rows[0], 0);
        assert!(e.rows.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn view_filters_by_level_doc_and_folded_text() {
        let mut j = j();
        j.push(
            Level::Info,
            Kind::Open,
            Some("Épées.esp"),
            "Ouvert : Skyrim",
        );
        j.push(
            Level::Warn,
            Kind::Export,
            Some("b.esp"),
            "Lignes non encodables",
        );
        j.push(Level::Error, Kind::Export, None, "Écriture refusée");
        let all = View::default();
        assert_eq!(all.filter(&j).count(), 3);
        let warn = View {
            min_level: Some(Level::Warn),
            ..Default::default()
        };
        assert_eq!(warn.filter(&j).count(), 2);
        let err = View {
            min_level: Some(Level::Error),
            ..Default::default()
        };
        assert_eq!(err.filter(&j).count(), 1);
        let doc = View {
            doc: Some("b.esp".into()),
            ..Default::default()
        };
        assert_eq!(doc.filter(&j).count(), 1);
        // Case and accents ignored, in the message as in the document.
        let t = View {
            text: "ECRITURE".into(),
            ..Default::default()
        };
        assert_eq!(t.filter(&j).count(), 1);
        let t = View {
            text: "epees".into(),
            ..Default::default()
        };
        assert_eq!(t.filter(&j).count(), 1);
    }

    #[test]
    fn tsv_is_one_line_per_entry() {
        let mut j = j();
        j.push(
            Level::Warn,
            Kind::Export,
            Some("a.esp"),
            "deux\nlignes\tici",
        );
        let t = tsv(j.entries(), |l| l.id().to_string());
        let cols: Vec<&str> = t.trim_end().split('\t').collect();
        assert_eq!(cols.len(), 4);
        assert_eq!(cols[1], "warning");
        assert_eq!(cols[2], "a.esp");
        assert_eq!(cols[3], "deux lignes ici");
        assert!(t.ends_with('\n'));
        assert_eq!(t.lines().count(), 1);
    }

    #[test]
    fn unflushed_entries_come_once() {
        let mut j = j();
        j.push(Level::Info, Kind::Open, None, "a");
        assert_eq!(j.take_unflushed().len(), 1);
        assert!(j.take_unflushed().is_empty());
        j.push(Level::Info, Kind::Apply, None, "b");
        j.push(Level::Info, Kind::Export, None, "c");
        let v = j.take_unflushed();
        assert_eq!(
            v.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
    }

    #[test]
    fn clear_keeps_numbering() {
        let mut j = j();
        j.push(Level::Error, Kind::Message, None, "a");
        j.clear();
        assert!(j.is_empty());
        assert_eq!(j.unread(), (0, 0));
        let id = j.push(Level::Info, Kind::Message, None, "b");
        assert_eq!(id, 2);
        assert_eq!(j.get(2).map(|e| e.text.as_str()), Some("b"));
    }

    #[test]
    fn json_has_the_documented_keys() {
        let mut j = j();
        j.push(Level::Error, Kind::Export, Some("a.esp"), "x");
        let v = json(j.entries().next().unwrap());
        for k in ["at", "level", "kind", "doc", "text"] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(v["level"], "error");
        assert_eq!(v["kind"], "export");
    }

    #[test]
    fn file_name_shape() {
        let n = file_name(0);
        assert!(n.starts_with("ximod-journal-") && n.ends_with(".txt"));
        assert_eq!(n.len(), "ximod-journal-AAAAMMJJ-HHMM.txt".len());
    }
}
