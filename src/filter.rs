//! Combinable grid filters (CDC §7): text, status, record type (GRUP) and
//! character count, all AND-combined, plus helpers for the counters. The
//! text filter has a target and an optional regular expression (A-09).

use std::collections::HashSet;

use crate::status;
use crate::tm::TmEntry;

/// Where the text filter looks (R-A09-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchIn {
    /// Source, translation, EDID and record signature (the historical rule).
    #[default]
    All,
    Source,
    Trans,
    /// EDID, or the FormID when the text is 6 to 8 hexadecimal digits.
    Id,
    /// « WEAP:FULL », « WEAP » or « :FULL ».
    Sig,
    /// The Commentaire column (C-03): the author's comment and the
    /// generated text.
    Comment,
}

impl SearchIn {
    pub const ALL: [SearchIn; 6] = [
        SearchIn::All,
        SearchIn::Source,
        SearchIn::Trans,
        SearchIn::Id,
        SearchIn::Sig,
        SearchIn::Comment,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            SearchIn::All => "filter-in-all",
            SearchIn::Source => "filter-in-source",
            SearchIn::Trans => "filter-in-trans",
            SearchIn::Id => "filter-in-id",
            SearchIn::Sig => "filter-in-sig",
            SearchIn::Comment => "filter-in-comment",
        }
    }
}

/// A FormID typed as 6 to 8 hexadecimal digits, with or without `0x`
/// (R-A09-02); `None` for anything else.
pub fn parse_formid(text: &str) -> Option<u32> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if !(6..=8).contains(&t.len()) || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok()
}

/// The text criterion, ready to run: a lowercase needle, a regular
/// expression, a FormID or a signature pair.
#[derive(Debug, Clone)]
pub enum TextMatch {
    None,
    Plain(String),
    Regex(regex::Regex),
    FormId(u32),
    /// Record and field parts, lowercase, each optional.
    Sig(String, String),
}

impl TextMatch {
    /// Compile the text of a criteria (R-A09-01/02): an invalid expression
    /// is an error the bar shows, the previous filter staying in force.
    pub fn compile(text: &str, target: SearchIn, use_regex: bool) -> Result<TextMatch, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(TextMatch::None);
        }
        if use_regex {
            return regex::RegexBuilder::new(t)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map(TextMatch::Regex)
                .map_err(|e| e.to_string());
        }
        match target {
            SearchIn::Id => Ok(parse_formid(t)
                .map(TextMatch::FormId)
                .unwrap_or_else(|| TextMatch::Plain(t.to_lowercase()))),
            SearchIn::Sig => {
                let (rec, fld) = t.split_once(':').unwrap_or((t, ""));
                Ok(TextMatch::Sig(
                    rec.trim().to_lowercase(),
                    fld.trim().to_lowercase(),
                ))
            }
            _ => Ok(TextMatch::Plain(t.to_lowercase())),
        }
    }

    fn hits(&self, s: &str) -> bool {
        match self {
            TextMatch::None => true,
            TextMatch::Plain(n) => s.to_lowercase().contains(n.as_str()),
            TextMatch::Regex(re) => re.is_match(s),
            TextMatch::FormId(_) | TextMatch::Sig(..) => false,
        }
    }

    /// Whether an entry passes the text criterion for a target.
    pub fn matches(&self, e: &TmEntry, target: SearchIn) -> bool {
        match self {
            TextMatch::None => true,
            TextMatch::FormId(v) => {
                e.formid == *v || (e.formid & 0x00FF_FFFF) == (*v & 0x00FF_FFFF)
            }
            TextMatch::Sig(rec, fld) => {
                (rec.is_empty() || e.rec_sig.eq_ignore_ascii_case(rec))
                    && (fld.is_empty() || e.field_sig.eq_ignore_ascii_case(fld))
            }
            _ => match target {
                SearchIn::All => {
                    self.hits(&e.edid)
                        || self.hits(&e.source)
                        || self.hits(&e.trans)
                        || self.hits(&e.rec_sig)
                }
                SearchIn::Source => self.hits(&e.source),
                SearchIn::Trans => self.hits(&e.trans),
                SearchIn::Id => self.hits(&e.edid) || self.hits(&format!("{:08X}", e.formid)),
                SearchIn::Sig => self.hits(&format!("{}:{}", e.rec_sig, e.field_sig)),
                // Without the generated texts, the author's comment alone.
                SearchIn::Comment => self.hits(&e.comment),
            },
        }
    }

    /// Whether a comment text (author + generated, C-03) passes.
    pub fn matches_comment(&self, text: &str) -> bool {
        self.hits_text(text)
    }

    /// Whether a plain text passes (a FormID or signature criterion never
    /// matches a text).
    pub fn hits_text(&self, text: &str) -> bool {
        match self {
            TextMatch::None => true,
            TextMatch::FormId(_) | TextMatch::Sig(..) => false,
            _ => self.hits(text),
        }
    }
}

/// The columns of the filter row under the grid headers (A-10), in the
/// grid order: one field per text column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterCol {
    Record,
    Field,
    Edid,
    Source,
    Trans,
    Review,
    Prov,
    Comment,
    /// The file of a line, in the « Tous les mods » tab (B-25).
    File,
}

/// The number of columns of the filter row.
pub const N_COLS: usize = 9;

impl FilterCol {
    pub const ALL: [FilterCol; N_COLS] = [
        FilterCol::Record,
        FilterCol::Field,
        FilterCol::Edid,
        FilterCol::Source,
        FilterCol::Trans,
        FilterCol::Review,
        FilterCol::Prov,
        FilterCol::Comment,
        FilterCol::File,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The position of the column's title among the grid headers (the
    /// six fixed titles, then Relecture, Provenance, Commentaire).
    pub fn header(self) -> usize {
        self.index() + 1
    }
}

/// The texts typed in the filter row (A-10), one per column; empty = no
/// filter on that column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnFilters {
    pub texts: [String; N_COLS],
}

impl ColumnFilters {
    pub fn get(&self, c: FilterCol) -> &str {
        &self.texts[c.index()]
    }

    pub fn get_mut(&mut self, c: FilterCol) -> &mut String {
        &mut self.texts[c.index()]
    }

    pub fn is_empty(&self) -> bool {
        self.texts.iter().all(|t| t.trim().is_empty())
    }

    /// The eraser (R-A10-05): every field emptied.
    pub fn clear(&mut self) {
        self.texts.iter_mut().for_each(String::clear);
    }

    /// The columns with a text, in the grid order.
    pub fn active(&self) -> impl Iterator<Item = (FilterCol, &str)> {
        FilterCol::ALL
            .iter()
            .map(|c| (*c, self.get(*c).trim()))
            .filter(|(_, t)| !t.is_empty())
    }

    /// Compile every field (R-A10-02): « contains » without the .* mode,
    /// a regular expression with it, the EDID field also taking a FormID.
    /// An invalid expression is reported in `errors` and that field alone
    /// is ignored.
    pub fn compile(&self, use_regex: bool) -> ColumnMatch {
        let mut out = ColumnMatch::default();
        for (c, t) in self.active() {
            let target = match c {
                FilterCol::Edid => SearchIn::Id,
                _ => SearchIn::All,
            };
            match TextMatch::compile(t, target, use_regex) {
                Ok(m) => out.matchers.push((c, m)),
                Err(e) => out.errors[c.index()] = Some(e),
            }
        }
        out
    }
}

/// The filter row, compiled: the matchers of the valid fields and the
/// errors of the others.
#[derive(Debug, Clone, Default)]
pub struct ColumnMatch {
    pub matchers: Vec<(FilterCol, TextMatch)>,
    pub errors: [Option<String>; N_COLS],
}

impl ColumnMatch {
    /// Whether line `i` passes every column filter (combined with AND,
    /// R-A10-03). The Provenance and Commentaire columns match their
    /// displayed texts when given, the raw code / the author's comment
    /// otherwise.
    pub fn matches(
        &self,
        e: &TmEntry,
        i: usize,
        prov: Option<&CommentTexts>,
        comments: Option<&CommentTexts>,
    ) -> bool {
        let prov_t = prov.and_then(|t| t.0.get(i)).map(String::as_str);
        let com_t = comments.and_then(|t| t.0.get(i)).map(String::as_str);
        self.matchers.iter().all(|(c, m)| match c {
            FilterCol::Record => m.hits_text(&e.rec_sig),
            FilterCol::Field => m.hits_text(&e.field_sig),
            FilterCol::Edid => m.matches(e, SearchIn::Id),
            FilterCol::Source => m.hits_text(&e.source),
            FilterCol::Trans => m.hits_text(&e.trans),
            FilterCol::Review => m.hits_text(&e.review),
            FilterCol::Prov => m.hits_text(prov_t.unwrap_or(&e.prov)),
            FilterCol::Comment => m.hits_text(com_t.unwrap_or(&e.comment)),
            // The synthesis puts each line's file name in `plugin`.
            FilterCol::File => m.hits_text(&e.plugin),
        })
    }
}

/// A set of filter criteria. Empty/all-enabled criteria pass everything.
#[derive(Debug, Clone, PartialEq)]
pub struct Criteria {
    /// The text typed in the filter field (raw; lowercased or compiled by
    /// [`Criteria::compile`]).
    pub text: String,
    /// Where it is looked for, and whether it is a regular expression (A-09).
    pub target: SearchIn,
    pub use_regex: bool,
    /// Which statuses are shown (indexed by [`status::Status::index`]).
    pub statuses: [bool; status::COUNT],
    /// « Provenance : » (B-09, R-B09-06).
    pub prov: crate::provenance::ProvFilter,
    /// Only the lines whose review differs from the translation (A-17).
    pub review_diff: bool,
    /// « = source » (`Some(true)`) / « ≠ source » (`Some(false)`) (A-21):
    /// the lines whose translation is the source, or differs from it; see
    /// [`same_as_source`].
    pub same_as_source: Option<bool>,
    /// Exact record signature (GRUP) to keep (empty = all).
    pub record: String,
    /// Exact field signature (Champ) to keep (empty = all).
    pub field: String,
    /// Minimum source length in characters (0 = no minimum).
    pub min_chars: usize,
    /// Maximum source length in characters (0 = no maximum).
    pub max_chars: usize,
    /// Collapse the view to one row per distinct source string (unique lines).
    pub unique_only: bool,
    /// Show the lines flagged « never shown in game » (hidden by default).
    pub show_unused: bool,
    /// « Filtrer sur cet enregistrement » (M-01): exact FormID.
    pub formid: Option<u32>,
    /// « Filtrer sur cette source » (M-01): exact source text.
    pub source_exact: Option<String>,
    /// « Anomalies » (D-05): only the lines flagged by the last check.
    pub anomalies_only: bool,
    pub anomalies: AnomalySet,
    /// « Ambiguës » (B-08): only the lines the dictionaries left marked.
    pub ambiguous_only: bool,
    /// « Filtrer sur ce PNJ » (C-02): only these lines, when set.
    pub only_lines: Option<AnomalySet>,
    /// The comment texts per line (C-03), for « Dans : Commentaire » and
    /// the Commentaire column filter; the author's comment alone is
    /// searched when absent.
    pub comments: Option<CommentTexts>,
    /// The filter row under the headers (A-10).
    pub columns: ColumnFilters,
    /// The short provenance labels per line, for the Provenance column
    /// filter; the raw code is searched when absent.
    pub prov_texts: Option<CommentTexts>,
    /// « Recherche avancée » applied to the grid (A-19).
    pub search: Option<SearchFilter>,
}

/// A compiled advanced search, shared with the view cache; equal when it
/// is the same allocation (like [`AnomalySet`]).
#[derive(Debug, Clone)]
pub struct SearchFilter(pub std::sync::Arc<crate::search::Compiled>);

impl PartialEq for SearchFilter {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The comments of the document, one per line, shared with the view cache
/// (equal when they are the same allocation, like [`AnomalySet`]).
#[derive(Debug, Clone, Default)]
pub struct CommentTexts(pub std::sync::Arc<Vec<String>>);

impl PartialEq for CommentTexts {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The lines in anomaly, shared with the view cache; two sets are equal
/// when they are the same allocation (the cache key never deep-compares).
#[derive(Debug, Clone, Default)]
pub struct AnomalySet(pub std::sync::Arc<std::collections::HashSet<usize>>);

impl PartialEq for AnomalySet {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl AnomalySet {
    pub fn contains(&self, i: usize) -> bool {
        self.0.contains(&i)
    }
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            text: String::new(),
            target: SearchIn::All,
            use_regex: false,
            statuses: [true; status::COUNT],
            prov: crate::provenance::ProvFilter::All,
            review_diff: false,
            same_as_source: None,
            record: String::new(),
            field: String::new(),
            min_chars: 0,
            max_chars: 0,
            unique_only: false,
            show_unused: false,
            formid: None,
            source_exact: None,
            anomalies_only: false,
            anomalies: AnomalySet::default(),
            ambiguous_only: false,
            only_lines: None,
            comments: None,
            columns: ColumnFilters::default(),
            prov_texts: None,
            search: None,
        }
    }
}

impl Criteria {
    /// Compile the text criterion once per application.
    pub fn compile(&self) -> Result<TextMatch, String> {
        TextMatch::compile(&self.text, self.target, self.use_regex)
    }

    /// Whether an entry passes **all** active criteria, the text one
    /// through its compiled form.
    pub fn matches(&self, e: &TmEntry) -> bool {
        let tm = self.compile().unwrap_or(TextMatch::None);
        self.matches_with(e, &tm)
    }

    pub fn matches_with(&self, e: &TmEntry, tm: &TextMatch) -> bool {
        if e.unused && !self.show_unused {
            return false;
        }
        if let Some(f) = self.formid
            && e.formid != f
        {
            return false;
        }
        if let Some(src) = &self.source_exact
            && &e.source != src
        {
            return false;
        }
        let text_done_by_apply = self.target == SearchIn::Comment && self.comments.is_some();
        if !text_done_by_apply && !tm.matches(e, self.target) {
            return false;
        }
        if !self.statuses[status::derive(e).index()] {
            return false;
        }
        if !self.prov.matches(e) {
            return false;
        }
        if self.review_diff && !crate::review::differs(e) {
            return false;
        }
        if let Some(want) = self.same_as_source
            && same_as_source(e) != Some(want)
        {
            return false;
        }
        if self.ambiguous_only && !crate::ambiguity::is_marked(e) {
            return false;
        }
        if !self.record.is_empty() && e.rec_sig != self.record {
            return false;
        }
        if !self.field.is_empty() && e.field_sig != self.field {
            return false;
        }
        let n = e.source.chars().count();
        if self.min_chars > 0 && n < self.min_chars {
            return false;
        }
        if self.max_chars > 0 && n > self.max_chars {
            return false;
        }
        true
    }

    /// Whether any filter actually restricts the view.
    pub fn is_active(&self) -> bool {
        !self.text.is_empty()
            || !self.record.is_empty()
            || !self.field.is_empty()
            || self.min_chars > 0
            || self.max_chars > 0
            || self.statuses.iter().any(|b| !b)
            || self.prov != crate::provenance::ProvFilter::All
            || self.review_diff
            || self.same_as_source.is_some()
            || self.formid.is_some()
            || self.source_exact.is_some()
            || self.anomalies_only
            || self.ambiguous_only
            || self.search.is_some()
            || self.only_lines.is_some()
            || !self.columns.is_empty()
    }

    /// Indices of the entries that pass, in order. With `unique_only`, only the
    /// **first** occurrence of each distinct source is kept (the unique-lines
    /// view — translate each distinct string once).
    pub fn apply(&self, entries: &[TmEntry]) -> Vec<usize> {
        let tm = self.compile().unwrap_or(TextMatch::None);
        let cols = self.columns.compile(self.use_regex);
        let mut out = Vec::with_capacity(entries.len() / 4);
        let mut seen: HashSet<&str> = HashSet::new();
        for (i, e) in entries.iter().enumerate() {
            if self.anomalies_only && !self.anomalies.contains(i) {
                continue;
            }
            if let Some(only) = &self.only_lines
                && !only.contains(i)
            {
                continue;
            }
            if !self.matches_with(e, &tm) {
                continue;
            }
            if self.target == SearchIn::Comment
                && let Some(c) = &self.comments
            {
                let text = c.0.get(i).map(String::as_str).unwrap_or("");
                if !tm.matches_comment(text) {
                    continue;
                }
            }
            if !cols.matchers.is_empty()
                && !cols.matches(e, i, self.prov_texts.as_ref(), self.comments.as_ref())
            {
                continue;
            }
            if let Some(s) = &self.search
                && !s.0.matches(e, i, self.comments.as_ref())
            {
                continue;
            }
            if self.unique_only && !seen.insert(e.source.as_str()) {
                continue;
            }
            out.push(i);
        }
        out
    }
}

/// Whether a line's translation is its source (A-21, R-A21-01): `None`
/// without translation, else whether both are equal once the spaces at
/// both ends are removed — case and accents count.
pub fn same_as_source(e: &TmEntry) -> Option<bool> {
    let t = e.trans.trim();
    (!t.is_empty()).then(|| t == e.source.trim())
}

/// The lines shown in game whose translation is their source (A-21): the
/// count of `ximod-cli check`.
pub fn count_same_as_source(entries: &[TmEntry]) -> usize {
    entries
        .iter()
        .filter(|e| !e.unused && same_as_source(e) == Some(true))
        .count()
}

/// Number of **unique** source strings among the entries.
pub fn unique_sources(entries: &[TmEntry]) -> usize {
    entries
        .iter()
        .map(|e| e.source.as_str())
        .collect::<HashSet<_>>()
        .len()
}

/// Line statistics that matter to a translator. A distinct *source* is counted
/// once regardless of how many times it appears, because translating it once
/// propagates to every occurrence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Total translatable entries (lines, with duplicates).
    pub total: usize,
    /// Distinct source strings.
    pub unique: usize,
    /// Distinct source strings that already have at least one effective
    /// translation somewhere.
    pub unique_translated: usize,
    /// Entries (with duplicates) that carry an effective translation.
    pub translated: usize,
    /// Lines per status (A-15), indexed by [`status::Status::index`]; the
    /// lines never shown in game count only when they are shown.
    pub per_status: [usize; status::COUNT],
}

impl Stats {
    /// Distinct source strings still to translate — the real remaining work.
    pub fn unique_remaining(&self) -> usize {
        self.unique - self.unique_translated
    }
}

/// Compute [`Stats`] in a single pass; `show_unused` says whether the
/// lines never shown in game join the per-status counts (A-15).
pub fn stats(entries: &[TmEntry], show_unused: bool) -> Stats {
    let mut by_source: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
    let mut translated = 0usize;
    let mut per_status = [0usize; status::COUNT];
    for e in entries {
        let t = e.is_translated();
        if t {
            translated += 1;
        }
        let done = by_source.entry(e.source.as_str()).or_insert(false);
        *done = *done || t;
        if show_unused || !e.unused {
            per_status[status::derive(e).index()] += 1;
        }
    }
    Stats {
        total: entries.len(),
        unique: by_source.len(),
        unique_translated: by_source.values().filter(|d| **d).count(),
        translated,
        per_status,
    }
}

/// A click on a status counter (A-15, R-A15-04): that status alone — a
/// second click on it releases the filter —; with `add` (Ctrl), the status
/// joins or leaves the statuses already filtered, and leaving none of them
/// means all of them.
pub fn click_status(statuses: &mut [bool; status::COUNT], st: status::Status, add: bool) {
    let i = st.index();
    let all = statuses.iter().all(|&b| b);
    if add {
        if all {
            *statuses = [false; status::COUNT];
        }
        statuses[i] = !statuses[i];
        if statuses.iter().all(|&b| !b) {
            *statuses = [true; status::COUNT];
        }
    } else if statuses.iter().enumerate().all(|(k, &b)| b == (k == i)) {
        *statuses = [true; status::COUNT];
    } else {
        *statuses = [false; status::COUNT];
        statuses[i] = true;
    }
}

/// The lines of `visible` per status (the « dont N affichées » of the
/// counters' tooltip, A-15).
pub fn shown_per_status(entries: &[TmEntry], visible: &[usize]) -> [usize; status::COUNT] {
    let mut out = [0usize; status::COUNT];
    for &i in visible {
        if let Some(e) = entries.get(i) {
            out[status::derive(e).index()] += 1;
        }
    }
    out
}

/// Propagate the translation of entry `idx` to every entry sharing its source
/// (translate a unique line once, fill all its duplicates). Returns how many
/// other entries were updated.
pub fn propagate_translation(entries: &mut [TmEntry], idx: usize) -> usize {
    let src = entries[idx].source.clone();
    let trans = entries[idx].trans.clone();
    let mut n = 0;
    for (i, e) in entries.iter_mut().enumerate() {
        if i != idx && e.source == src && e.trans != trans {
            e.trans = trans.clone();
            e.set_translated_flags(!trans.is_empty());
            n += 1;
        }
    }
    n
}

/// Distinct record signatures present, sorted — the GRUP filter's choices.
pub fn record_sigs(entries: &[TmEntry]) -> Vec<String> {
    let mut set: Vec<String> = entries
        .iter()
        .map(|e| e.rec_sig.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    set.sort();
    set
}

/// Distinct field signatures present, sorted — the Champ filter's choices.
pub fn field_sigs(entries: &[TmEntry]) -> Vec<String> {
    let mut set: Vec<String> = entries
        .iter()
        .map(|e| e.field_sig.clone())
        .filter(|s| !s.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    set.sort();
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::Status;

    fn entry(rec: &str, src: &str, tr: &str) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: rec.into(),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        };
        status::set(
            &mut e,
            if tr.is_empty() {
                Status::ToTranslate
            } else {
                Status::Validated
            },
        );
        e
    }

    #[test]
    fn same_as_source_has_three_states_and_combines() {
        // A-21: trimmed, exact (case and accents count); no translation is
        // in neither state.
        let entries = vec![
            entry("WEAP", "Iron Sword", "Iron Sword "),
            entry("WEAP", "Iron Sword", "iron sword"),
            entry("WEAP", "Iron Sword", ""),
            entry("MISC", "OK", "OK"),
            entry("WEAP", "Steel Sword", "Épée en acier"),
        ];
        assert_eq!(same_as_source(&entries[0]), Some(true));
        assert_eq!(same_as_source(&entries[1]), Some(false));
        assert_eq!(same_as_source(&entries[2]), None);
        let mut c = Criteria {
            same_as_source: Some(true),
            ..Criteria::default()
        };
        assert!(c.is_active());
        assert_eq!(c.apply(&entries), vec![0, 3]);
        c.same_as_source = Some(false);
        assert_eq!(c.apply(&entries), vec![1, 4]);
        // AND with the other criteria: GRUP and text.
        c.same_as_source = Some(true);
        c.record = "WEAP".into();
        c.text = "sword".into();
        assert_eq!(c.apply(&entries), vec![0]);
        c.same_as_source = None;
        assert_eq!(c.apply(&entries), vec![0, 1, 2, 4]);
        assert_eq!(count_same_as_source(&entries), 2);
        let mut hidden = entries[3].clone();
        hidden.unused = true;
        assert_eq!(count_same_as_source(&[hidden]), 0);
    }

    #[test]
    fn record_and_source_filters_from_the_row_menu() {
        let mut a = entry("WEAP", "Iron Sword", "Épée en fer");
        a.formid = 0x1A3F2;
        let mut b = entry("WEAP", "Iron Sword", "");
        b.formid = 0x2000;
        let mut c = entry("ARMO", "Leather", "Cuir");
        c.formid = 0x1A3F2;
        let entries = vec![a, b, c];
        let crit = Criteria {
            formid: Some(0x1A3F2),
            ..Default::default()
        };
        assert_eq!(crit.apply(&entries), vec![0, 2]);
        assert!(crit.is_active());
        let crit = Criteria {
            source_exact: Some("Iron Sword".into()),
            ..Default::default()
        };
        assert_eq!(crit.apply(&entries), vec![0, 1]);
        let crit = Criteria {
            source_exact: Some("Iron".into()),
            ..Default::default()
        };
        assert!(crit.apply(&entries).is_empty(), "exact, not a substring");
    }

    /// T-D05-05: « Anomalies » keeps the flagged lines only, and the set
    /// compares by identity (the view cache must not scan it).
    #[test]
    fn anomalies_only_keeps_the_flagged_lines() {
        let entries = vec![
            entry("WEAP", "a", "x"),
            entry("WEAP", "b", "y"),
            entry("WEAP", "c", "z"),
        ];
        let set = AnomalySet(std::sync::Arc::new([0usize, 2].into_iter().collect()));
        let crit = Criteria {
            anomalies_only: true,
            anomalies: set.clone(),
            ..Default::default()
        };
        assert!(crit.is_active());
        assert_eq!(crit.apply(&entries), vec![0, 2]);
        let same = Criteria {
            anomalies_only: true,
            anomalies: set,
            ..Default::default()
        };
        assert_eq!(crit, same);
        let other = Criteria {
            anomalies_only: true,
            anomalies: AnomalySet(std::sync::Arc::new([0usize, 2].into_iter().collect())),
            ..Default::default()
        };
        assert_ne!(crit, other, "another allocation is another criteria");
        let off = Criteria {
            anomalies_only: false,
            anomalies: crit.anomalies.clone(),
            ..Default::default()
        };
        assert!(!off.is_active());
        assert_eq!(off.apply(&entries), vec![0, 1, 2]);
    }

    #[test]
    fn targets_formid_signature_and_regex() {
        let mut a = entry("WEAP", "Iron Sword", "Épée en fer");
        a.formid = 0x0001A3F2;
        a.edid = "IronSword".into();
        a.field_sig = "FULL".into();
        let mut b = entry("NPC_", "Draugr", "Draugr");
        b.formid = 0x02000ABC;
        b.edid = "EncDraugr".into();
        b.field_sig = "FULL".into();
        let mut c = entry("DIAL", "A dragon!", "Un dragon !");
        c.formid = 0x00000777;
        c.field_sig = "FULL".into();
        let entries = vec![a, b, c];
        let crit = |text: &str, target: SearchIn, re: bool| Criteria {
            text: text.into(),
            target,
            use_regex: re,
            ..Default::default()
        };
        // T-A09-01: FormID with or without the master index, or 0x.
        assert_eq!(
            crit("0001A3F2", SearchIn::Id, false).apply(&entries),
            vec![0]
        );
        assert_eq!(crit("01A3F2", SearchIn::Id, false).apply(&entries), vec![0]);
        assert_eq!(
            crit("0x0501A3F2", SearchIn::Id, false).apply(&entries),
            vec![0]
        );
        assert_eq!(parse_formid("A3F2"), None, "too short: an EDID");
        assert_eq!(crit("draugr", SearchIn::Id, false).apply(&entries), vec![1]);
        // Signature: WEAP:FULL, WEAP, :FULL.
        assert_eq!(
            crit("weap:full", SearchIn::Sig, false).apply(&entries),
            vec![0]
        );
        assert_eq!(crit("NPC_", SearchIn::Sig, false).apply(&entries), vec![1]);
        assert_eq!(
            crit(":FULL", SearchIn::Sig, false).apply(&entries),
            vec![0, 1, 2]
        );
        // Source / translation only.
        assert_eq!(
            crit("dragon", SearchIn::Source, false).apply(&entries),
            vec![2]
        );
        assert_eq!(
            crit("épée", SearchIn::Trans, false).apply(&entries),
            vec![0]
        );
        assert_eq!(
            crit("épée", SearchIn::Source, false).apply(&entries),
            Vec::<usize>::new()
        );
        // T-A09-02: regex, case-insensitive by default, (?-i) to be strict.
        assert_eq!(
            crit("(dragon|draugr)", SearchIn::All, true).apply(&entries),
            vec![1, 2]
        );
        assert_eq!(
            crit("(?-i)Dragon", SearchIn::All, true).apply(&entries),
            Vec::<usize>::new()
        );
        assert_eq!(
            crit("^Iron", SearchIn::Source, true).apply(&entries),
            vec![0]
        );
        assert!(crit("(dragon|", SearchIn::All, true).compile().is_err());
        // The regex also applies to the id and signature targets.
        assert_eq!(crit("^Enc", SearchIn::Id, true).apply(&entries), vec![1]);
        assert_eq!(
            crit("^(WEAP|DIAL):", SearchIn::Sig, true).apply(&entries),
            vec![0, 2]
        );
    }

    /// T-C03-03: « Dans : Commentaire » searches the author's comment and
    /// the generated text, with the regex and the case of the other targets.
    #[test]
    fn comment_target_searches_author_and_generated() {
        let mut a = entry("INFO", "Hello there.", "");
        a.comment = "à relire".into();
        let b = entry("WEAP", "Iron Sword", "");
        let c = entry("INFO", "Go away.", "");
        let entries = vec![a, b, c];
        let texts = CommentTexts(std::sync::Arc::new(vec![
            "à relire · Ralof (h) · MQ101 · voix ✔".into(),
            "modèle : Weapons\\Iron\\IronWarAxe.nif".into(),
            "Gerdur (f) · MQ101 · voix ✘".into(),
        ]));
        let crit = |text: &str, re: bool, texts: Option<CommentTexts>| Criteria {
            text: text.into(),
            target: SearchIn::Comment,
            use_regex: re,
            comments: texts,
            ..Default::default()
        };
        assert_eq!(
            crit("voix ✘", false, Some(texts.clone())).apply(&entries),
            vec![2]
        );
        assert_eq!(
            crit("(f)", false, Some(texts.clone())).apply(&entries),
            vec![2]
        );
        assert_eq!(
            crit("mq101", false, Some(texts.clone())).apply(&entries),
            vec![0, 2]
        );
        assert_eq!(
            crit("relire", false, Some(texts.clone())).apply(&entries),
            vec![0]
        );
        assert_eq!(
            crit("^modèle", true, Some(texts.clone())).apply(&entries),
            vec![1]
        );
        // Without the generated texts: the author's comment alone.
        assert_eq!(crit("relire", false, None).apply(&entries), vec![0]);
        assert_eq!(
            crit("mq101", false, None).apply(&entries),
            Vec::<usize>::new()
        );
        assert!(crit("x", false, None).is_active());
    }

    /// T-A09-04: 50 000 lines filtered well under the budget (debug build).
    #[test]
    fn fifty_thousand_lines_filter_fast() {
        let entries: Vec<TmEntry> = (0..50_000)
            .map(|i| {
                let mut e = entry(
                    "WEAP",
                    &format!("Iron Sword {i} of the dragon"),
                    &format!("Épée en fer {i}"),
                );
                e.formid = i as u32;
                e.edid = format!("IronSword{i}");
                e
            })
            .collect();
        let crit = Criteria {
            text: "sword".into(),
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let n = crit.apply(&entries).len();
        let plain = t.elapsed();
        assert_eq!(n, 50_000);
        let crit = Criteria {
            text: "sword 4\\d{3} of".into(),
            use_regex: true,
            target: SearchIn::Source,
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let n = crit.apply(&entries).len();
        let re = t.elapsed();
        assert_eq!(n, 1_000);
        // Debug builds are several times slower than the release the user runs.
        assert!(
            plain.as_millis() < 1_000 && re.as_millis() < 1_000,
            "plain {plain:?}, regex {re:?}"
        );
        eprintln!("50 000 lignes : texte {plain:?}, regex {re:?}");
    }

    #[test]
    fn combinable_and_logic() {
        let entries = vec![
            entry("BOOK", "Hello world", "Bonjour le monde"),
            entry("WEAP", "Iron Sword", ""),
            entry("BOOK", "A", "B"),
        ];

        // Text alone.
        let mut c = Criteria {
            text: "iron".into(),
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![1]);

        // Record alone.
        c = Criteria {
            record: "BOOK".into(),
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![0, 2]);

        // Char count (source length ≥ 5).
        c = Criteria {
            min_chars: 5,
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![0, 1]);

        // Status: only "to translate" (untranslated).
        let mut st = [false; status::COUNT];
        st[Status::ToTranslate.index()] = true;
        c = Criteria {
            statuses: st,
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![1]);

        // Combined: BOOK AND length ≥ 5 → only the first.
        c = Criteria {
            record: "BOOK".into(),
            min_chars: 5,
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![0]);
    }

    #[test]
    fn counters() {
        let entries = vec![
            entry("BOOK", "Chest", "Coffre"),
            entry("CONT", "Chest", "Coffre"),
            entry("WEAP", "Sword", ""),
        ];
        assert_eq!(unique_sources(&entries), 2); // "Chest" twice
        assert_eq!(record_sigs(&entries), vec!["BOOK", "CONT", "WEAP"]);
    }

    #[test]
    fn field_filter_and_sigs() {
        let mut a = entry("BOOK", "Hello", "H");
        a.field_sig = "FULL".into();
        let mut b = entry("BOOK", "World", "W");
        b.field_sig = "DESC".into();
        let entries = vec![a, b];
        let c = Criteria {
            field: "FULL".into(),
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![0]);
        assert_eq!(field_sigs(&entries), vec!["DESC", "FULL"]);
        // Combined GRUP + Champ.
        let c = Criteria {
            record: "BOOK".into(),
            field: "DESC".into(),
            ..Default::default()
        };
        assert_eq!(c.apply(&entries), vec![1]);
    }

    #[test]
    fn default_passes_everything() {
        let e = entry("BOOK", "x", "y");
        assert!(Criteria::default().matches(&e));
        assert!(!Criteria::default().is_active());
    }

    #[test]
    fn unique_only_keeps_first_occurrence() {
        let entries = vec![
            entry("MISC", "Chest", ""),
            entry("CONT", "Chest", ""), // duplicate source
            entry("WEAP", "Sword", ""),
            entry("MISC", "Chest", ""), // duplicate source
        ];
        let c = Criteria {
            unique_only: true,
            ..Default::default()
        };
        // Only the first "Chest" (idx 0) and "Sword" (idx 2).
        assert_eq!(c.apply(&entries), vec![0, 2]);
        // Without the flag, all four are shown.
        assert_eq!(Criteria::default().apply(&entries).len(), 4);
    }

    #[test]
    fn stats_count_unique_and_remaining() {
        let entries = vec![
            entry("MISC", "Chest", "Coffre"), // Chest translated here…
            entry("CONT", "Chest", ""),       // …so the "Chest" unique is done
            entry("WEAP", "Sword", ""),       // Sword not translated anywhere
            entry("WEAP", "Bow", ""),
        ];
        let s = stats(&entries, false);
        assert_eq!(s.total, 4);
        assert_eq!(s.unique, 3); // Chest, Sword, Bow
        assert_eq!(s.translated, 1);
        assert_eq!(s.unique_translated, 1); // Chest
        assert_eq!(s.unique_remaining(), 2); // Sword, Bow
        // Per status (A-15): the sum is the total (T-A15-02); a line never
        // shown in game counts only when shown.
        assert_eq!(s.per_status.iter().sum::<usize>(), 4);
        assert_eq!(s.per_status[status::Status::ToTranslate.index()], 3);
        let mut with_unused = entries.clone();
        with_unused.push(TmEntry {
            unused: true,
            ..entry("TES4", "Author", "")
        });
        assert_eq!(
            stats(&with_unused, false).per_status.iter().sum::<usize>(),
            4
        );
        assert_eq!(
            stats(&with_unused, true).per_status.iter().sum::<usize>(),
            5
        );
        assert_eq!(shown_per_status(&entries, &[0, 2]), {
            let mut a = [0; status::COUNT];
            a[status::Status::ToTranslate.index()] = 1;
            a[status::derive(&entries[0]).index()] += 1;
            a
        });
    }

    /// T-A15-03: a click keeps one status, a second click releases; Ctrl
    /// adds and removes.
    #[test]
    fn status_counter_clicks() {
        use status::Status as S;
        let mut f = [true; status::COUNT];
        click_status(&mut f, S::Ignored, false);
        assert!(f[S::Ignored.index()] && f.iter().filter(|&&b| b).count() == 1);
        click_status(&mut f, S::Ignored, false);
        assert!(f.iter().all(|&b| b), "second click: filter released");
        click_status(&mut f, S::Pending, true);
        assert!(f[S::Pending.index()] && f.iter().filter(|&&b| b).count() == 1);
        click_status(&mut f, S::Validated, true);
        assert_eq!(f.iter().filter(|&&b| b).count(), 2);
        click_status(&mut f, S::Pending, true);
        click_status(&mut f, S::Validated, true);
        assert!(f.iter().all(|&b| b), "nothing left: everything");
        click_status(&mut f, S::Custom, false);
        click_status(&mut f, S::Ignored, false);
        assert!(f[S::Ignored.index()] && !f[S::Custom.index()]);
    }

    #[test]
    fn propagation_fills_duplicates() {
        let mut entries = vec![
            entry("MISC", "Chest", "Coffre"),
            entry("CONT", "Chest", ""),
            entry("WEAP", "Sword", ""),
        ];
        let n = propagate_translation(&mut entries, 0);
        assert_eq!(n, 1); // the second "Chest" filled
        assert_eq!(entries[1].trans, "Coffre");
        assert_eq!(entries[2].trans, ""); // Sword untouched
        assert!(entries[1].is_translated());
    }
}
