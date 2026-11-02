//! Where the translation of a line came from (B-09): a short code carried
//! by the line ([`TmEntry::prov`]), saved with the working files (`.ximdb`
//! format 4, pivot JSON, native XML) and stable from one language to the
//! other; the interface words it ([`short_label`], [`full_label`]) and
//! filters on its category ([`ProvFilter`]).
//!
//! Codes: `typed`, `dict:<role>:<level>:<file>`, `fuzzy:<score>`,
//! `online:<service>`, `propagate`, `propagate:<file>` (B-25), `replace`,
//! `replace:<matrix>` (B-17),
//! `dict:<role>:norm:<file>|rep:<codes>|src:<source>` (a line matched on
//! its normalized form, B-19: the reports `n`, `p`, `c`, `!` and the
//! source found, `|` and `%` escaped),
//! `target`, `review`,
//! `history`, `import:<format>`, `rule:<name>`, `lock:<rule>` (a string
//! of a script locked, C-08, with the provenance under the file lock after
//! `>`); a line left ambiguous by the dictionaries (B-08) ends with
//! `|amb:<n>`.

use crate::basedb::FillLevel;
use crate::dicts::Role;
use crate::i18n::I18n;
use crate::normalize::Reports;
use crate::tm::TmEntry;

pub const TYPED: &str = "typed";
pub const PROPAGATE: &str = "propagate";
pub const REPLACE: &str = "replace";
pub const TARGET: &str = "target";
pub const REVIEW: &str = "review";
pub const HISTORY: &str = "history";
/// A line the glossary filled, its terms in the source order (B-15).
pub const RULE_GLOSSARY: &str = "rule:glossary";
/// A line the composition of the glossary's terms filled (B-15, B-16).
pub const RULE_COMPOSE: &str = "rule:compose";
/// A line a pattern filled, followed by its number (B-16).
pub const RULE_PATTERN: &str = "rule:pattern:";
/// A line derived from a parent, followed by `<rule>:<parent EDID>` (B-21).
pub const RULE_DERIVE: &str = "rule:derive:";

/// The provenance of a line a replacement matrix changed (B-17).
pub fn replace_matrix(name: &str) -> String {
    format!("{REPLACE}:{name}")
}

/// The provenance of a line received from another open file (B-25).
pub fn propagate_file(file: &str) -> String {
    format!("{PROPAGATE}:{file}")
}

/// The provenance of a line, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prov {
    /// No translation, or nothing known.
    None,
    Typed,
    Dict {
        role: Role,
        level: FillLevel,
        file: String,
    },
    /// Matched on the normalized form (B-19): the reports, the number of
    /// candidates in `reports.amb` (from the `|amb:` suffix), the source
    /// found in the dictionary.
    Normalized {
        role: Role,
        file: String,
        reports: Reports,
        source: String,
    },
    Fuzzy(u8),
    Online(String),
    Propagate,
    /// Received from another open file (B-25): its name.
    PropagateFile(String),
    Replace,
    /// Changed by a replacement matrix (B-17): its name.
    ReplaceMatrix(String),
    Target,
    Review,
    History,
    Import(String),
    Rule(String),
    /// Locked (C-08): the rule (`vmad-off`, `key`, `path`…).
    Lock(String),
}

/// The code of a dictionary level (`exact`, `text`…).
fn level_code(l: FillLevel) -> String {
    match l {
        FillLevel::Exact => "exact".into(),
        FillLevel::Text => "text".into(),
        FillLevel::TextOnly => "textonly".into(),
        FillLevel::Old => "old".into(),
        FillLevel::FormId => "formid".into(),
        FillLevel::Fuzzy(s) => format!("fuzzy{s}"),
        FillLevel::TargetFile => "target".into(),
        FillLevel::Online => "online".into(),
        FillLevel::Normalized(_) => "norm".into(),
    }
}

fn level_from(s: &str) -> Option<FillLevel> {
    Some(match s {
        "exact" => FillLevel::Exact,
        "text" => FillLevel::Text,
        "textonly" => FillLevel::TextOnly,
        "old" => FillLevel::Old,
        "formid" => FillLevel::FormId,
        "target" => FillLevel::TargetFile,
        "online" => FillLevel::Online,
        "norm" => FillLevel::Normalized(Reports::default()),
        _ => FillLevel::Fuzzy(s.strip_prefix("fuzzy")?.parse().ok()?),
    })
}

/// `dict:<role>:<level>:<file>` — a line filled by a dictionary.
pub fn dict(role: Role, level: FillLevel, file: &str) -> String {
    match level {
        FillLevel::Fuzzy(s) => fuzzy(s),
        FillLevel::TargetFile => TARGET.to_string(),
        FillLevel::Online => online(file),
        FillLevel::Normalized(r) => {
            let code = format!("dict:{}:norm:{file}|rep:{}", role.key(), r.codes());
            with_amb(code, r)
        }
        _ => format!("dict:{}:{}:{file}", role.key(), level_code(level)),
    }
}

fn with_amb(code: String, r: Reports) -> String {
    if r.amb >= 2 {
        format!("{code}|amb:{}", r.amb)
    } else {
        code
    }
}

/// `%` and `|` escaped: the source found is the last field of a code.
fn escape(s: &str) -> String {
    s.replace('%', "%25").replace('|', "%7C")
}

fn unescape(s: &str) -> String {
    s.replace("%7C", "|").replace("%25", "%")
}

/// The code of a line matched on its normalized form (B-19): the level,
/// the reports, the source found, then the ambiguity mark.
pub fn normalized(role: Role, file: &str, reports: Reports, source: &str) -> String {
    let code = format!(
        "dict:{}:norm:{file}|rep:{}|src:{}",
        role.key(),
        reports.codes(),
        escape(source)
    );
    with_amb(code, reports)
}

/// `fuzzy:<score>` — the heuristic (a suggestion used, F9).
pub fn fuzzy(score: u8) -> String {
    format!("fuzzy:{score}")
}

/// `online:<service>` — an online service's answer.
pub fn online(service: &str) -> String {
    format!("online:{service}")
}

/// `import:<format>` — the translation read with the file opened.
pub fn import(format: &str) -> String {
    format!("import:{format}")
}

/// The code without the ambiguity suffix, and the number of candidates
/// that suffix gives (B-08).
pub fn split_ambiguous(code: &str) -> (&str, Option<u32>) {
    match code.rsplit_once("|amb:") {
        Some((base, n)) => (base, n.parse().ok()),
        None => (code, None),
    }
}

/// Decode a code; an unknown one reads as [`Prov::None`].
pub fn parse(code: &str) -> Prov {
    let (code, amb) = split_ambiguous(code);
    // A normalized match (B-19).
    if let Some((head, rest)) = code.split_once("|rep:") {
        let (codes, source) = match rest.split_once("|src:") {
            Some((c, s)) => (c, unescape(s)),
            None => (rest, String::new()),
        };
        let mut reports = Reports::from_codes(codes);
        reports.amb = amb.unwrap_or(0).min(255) as u8;
        return match parse(head) {
            Prov::Dict { role, file, .. } => Prov::Normalized {
                role,
                file,
                reports,
                source,
            },
            _ => Prov::None,
        };
    }
    match code {
        "" => return Prov::None,
        TYPED => return Prov::Typed,
        PROPAGATE => return Prov::Propagate,
        REPLACE => return Prov::Replace,
        TARGET => return Prov::Target,
        REVIEW => return Prov::Review,
        HISTORY => return Prov::History,
        _ => {}
    }
    let Some((kind, rest)) = code.split_once(':') else {
        return Prov::None;
    };
    match kind {
        "dict" => {
            let mut it = rest.splitn(3, ':');
            let (Some(role), Some(level), Some(file)) = (it.next(), it.next(), it.next()) else {
                return Prov::None;
            };
            match (Role::from_key(role), level_from(level)) {
                (Some(role), Some(level)) => Prov::Dict {
                    role,
                    level,
                    file: file.to_string(),
                },
                _ => Prov::None,
            }
        }
        "fuzzy" => rest.parse().map(Prov::Fuzzy).unwrap_or(Prov::None),
        "online" => Prov::Online(rest.to_string()),
        "import" => Prov::Import(rest.to_string()),
        "rule" => Prov::Rule(rest.to_string()),
        "lock" => Prov::Lock(rest.split('>').next().unwrap_or(rest).to_string()),
        "propagate" => Prov::PropagateFile(rest.to_string()),
        "replace" => Prov::ReplaceMatrix(rest.to_string()),
        _ => Prov::None,
    }
}

/// Whether a translation is « identique » (R-B09-05): not empty and equal
/// to the source once the outer spaces are trimmed.
pub fn is_identical(e: &TmEntry) -> bool {
    let t = e.trans.trim();
    !t.is_empty() && t == e.source.trim()
}

/// Settle the provenance of a line an operation just changed (R-B09-02):
/// an emptied translation loses it; a translation changed without the
/// operation naming its provenance gets `default`.
pub fn settle(e: &mut TmEntry, before_trans: &str, before_prov: &str, default: Option<&str>) {
    if e.trans.is_empty() {
        // A lock says why the line is held, translation or not (C-08).
        if !matches!(parse(&e.prov), Prov::Lock(_)) {
            e.prov.clear();
        }
        return;
    }
    if e.trans != before_trans
        && e.prov == before_prov
        && let Some(d) = default
    {
        e.prov = d.to_string();
    }
    // A decided line is no longer ambiguous (B-08, R-B08-05): Validé or
    // Personnalisé.
    if split_ambiguous(&e.prov).1.is_some()
        && matches!(
            crate::status::derive(e),
            crate::status::Status::Validated | crate::status::Status::Custom
        )
    {
        e.prov = split_ambiguous(&e.prov).0.to_string();
    }
}

/// The lines of a file just opened that carry a translation without a
/// provenance: « importée » with the format they were read from.
pub fn mark_imported(entries: &mut [TmEntry], source_format: &str) {
    let code = import(source_format);
    for e in entries.iter_mut() {
        if !e.trans.is_empty() && e.prov.is_empty() {
            e.prov = code.clone();
        }
    }
}

/// The choices of the « Provenance : » list of the filter bar (R-B09-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProvFilter {
    #[default]
    All,
    Typed,
    Game,
    Mod,
    Personal,
    OtherDict,
    Fuzzy,
    /// Matched on the normalized form (B-19).
    Normalized,
    Online,
    Propagate,
    Replace,
    Review,
    History,
    Import,
    /// Filled by an assistant: glossary, composition (B-15).
    Rule,
    /// Locked by a rule or the file's VMAD switch (C-08, D-63).
    Locked,
    Identical,
    Empty,
}

impl ProvFilter {
    pub const ALL: [ProvFilter; 18] = [
        ProvFilter::All,
        ProvFilter::Typed,
        ProvFilter::Game,
        ProvFilter::Mod,
        ProvFilter::Personal,
        ProvFilter::OtherDict,
        ProvFilter::Fuzzy,
        ProvFilter::Normalized,
        ProvFilter::Online,
        ProvFilter::Propagate,
        ProvFilter::Replace,
        ProvFilter::Review,
        ProvFilter::History,
        ProvFilter::Import,
        ProvFilter::Rule,
        ProvFilter::Locked,
        ProvFilter::Identical,
        ProvFilter::Empty,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            ProvFilter::All => "filter-prov-all",
            ProvFilter::Typed => "filter-prov-typed",
            ProvFilter::Game => "filter-prov-game",
            ProvFilter::Mod => "filter-prov-mod",
            ProvFilter::Personal => "filter-prov-personal",
            ProvFilter::OtherDict => "filter-prov-other",
            ProvFilter::Fuzzy => "filter-prov-fuzzy",
            ProvFilter::Normalized => "filter-prov-normalized",
            ProvFilter::Online => "filter-prov-online",
            ProvFilter::Propagate => "filter-prov-propagate",
            ProvFilter::Replace => "filter-prov-replace",
            ProvFilter::Review => "filter-prov-review",
            ProvFilter::History => "filter-prov-history",
            ProvFilter::Import => "filter-prov-import",
            ProvFilter::Rule => "filter-prov-rule",
            ProvFilter::Locked => "filter-prov-locked",
            ProvFilter::Identical => "filter-prov-identical",
            ProvFilter::Empty => "filter-prov-empty",
        }
    }

    /// The category of a line: « identique » and « sans traduction » first,
    /// whatever the stored provenance (R-B09-05).
    pub fn of(e: &TmEntry) -> Option<ProvFilter> {
        if matches!(parse(&e.prov), Prov::Lock(_)) {
            return Some(ProvFilter::Locked);
        }
        if e.trans.is_empty() {
            return Some(ProvFilter::Empty);
        }
        if is_identical(e) {
            return Some(ProvFilter::Identical);
        }
        Some(match parse(&e.prov) {
            Prov::None | Prov::Lock(_) => return None,
            Prov::Rule(_) => ProvFilter::Rule,
            Prov::Typed => ProvFilter::Typed,
            Prov::Dict { role, .. } => match role {
                Role::Game => ProvFilter::Game,
                Role::Mod => ProvFilter::Mod,
                Role::Personal => ProvFilter::Personal,
                Role::TextOnly | Role::External | Role::Backups => ProvFilter::OtherDict,
            },
            Prov::Fuzzy(_) => ProvFilter::Fuzzy,
            Prov::Normalized { .. } => ProvFilter::Normalized,
            Prov::Online(_) => ProvFilter::Online,
            Prov::Propagate | Prov::PropagateFile(_) => ProvFilter::Propagate,
            Prov::Replace | Prov::ReplaceMatrix(_) => ProvFilter::Replace,
            Prov::Review => ProvFilter::Review,
            Prov::History => ProvFilter::History,
            Prov::Import(_) | Prov::Target => ProvFilter::Import,
        })
    }

    pub fn matches(self, e: &TmEntry) -> bool {
        self == ProvFilter::All || ProvFilter::of(e) == Some(self)
    }
}

/// The name shown for an online service id.
fn service_name(i18n: &I18n, id: &str) -> String {
    match crate::online::Service::from_id(id) {
        Some(s) => s.short_label(i18n),
        None => id.to_string(),
    }
}

/// The name shown for an import format code.
fn format_name(code: &str) -> String {
    match code {
        "eetxml" | "sstxml" | "ximodxml" | "xml" => "XML".to_string(),
        "" => "?".to_string(),
        other => other.to_uppercase(),
    }
}

fn level_short_key(l: FillLevel) -> &'static str {
    match l {
        FillLevel::Exact => "prov-level-exact",
        FillLevel::Text => "prov-level-text",
        FillLevel::TextOnly => "prov-level-textonly",
        FillLevel::Old => "prov-level-old",
        _ => "prov-level-formid",
    }
}

/// The words of a decoded provenance; `full` for the tooltip and the
/// Enregistrement tab, the short form for the column.
fn words(i18n: &I18n, p: &Prov, full: bool) -> String {
    match p {
        Prov::None => i18n.t("prov-none"),
        Prov::Typed => i18n.t("prov-typed"),
        Prov::Propagate => i18n.t("prov-propagate"),
        Prov::PropagateFile(f) => i18n.t_arg("prov-propagate-file", "file", f),
        Prov::Replace => i18n.t("prov-replace"),
        Prov::ReplaceMatrix(name) => i18n.t_arg("prov-replace-matrix", "name", name),
        Prov::Target => i18n.t("prov-target"),
        Prov::Review => i18n.t("prov-review"),
        Prov::History => i18n.t("prov-history"),
        Prov::Dict { role, level, file } => {
            let role_s = i18n.t(role.short_key());
            if full {
                let level_s = i18n.t(level.i18n_key());
                i18n.t_args(
                    "prov-dict-full",
                    &[("role", &role_s), ("file", file), ("level", &level_s)],
                )
            } else {
                let level_s = i18n.t(level_short_key(*level));
                i18n.t_args("prov-dict-short", &[("role", &role_s), ("level", &level_s)])
            }
        }
        Prov::Normalized {
            file,
            reports,
            source,
            ..
        } => {
            if full {
                let list: Vec<String> = reports.keys().iter().map(|k| i18n.t(k)).collect();
                let list = if list.is_empty() {
                    i18n.t("prov-report-none")
                } else {
                    list.join(", ")
                };
                i18n.t_args(
                    "prov-normalized-long",
                    &[("file", file), ("source", source), ("reports", &list)],
                )
            } else {
                let list: Vec<String> = reports
                    .keys()
                    .iter()
                    .map(|k| i18n.t(&format!("{k}-short")))
                    .collect();
                if list.is_empty() {
                    i18n.t("prov-normalized-bare")
                } else {
                    i18n.t_arg("prov-normalized", "reports", &list.join(", "))
                }
            }
        }
        Prov::Fuzzy(s) => i18n.t_arg("prov-fuzzy", "score", &s.to_string()),
        Prov::Online(id) => i18n.t_arg("prov-online", "service", &service_name(i18n, id)),
        Prov::Import(f) => i18n.t_arg("prov-import", "format", &format_name(f)),
        Prov::Rule(n) => match n.as_str() {
            "glossary" => i18n.t("prov-rule-glossary"),
            "compose" => i18n.t("prov-rule-compose"),
            p if p.starts_with("derive:") => {
                let rest = &p["derive:".len()..];
                let (id, parent) = rest.split_once(':').unwrap_or((rest, ""));
                i18n.t_args("prov-rule-derive", &[("id", id), ("parent", parent)])
            }
            p if p.starts_with("pattern:") => {
                i18n.t_arg("prov-rule-pattern", "n", &p["pattern:".len()..])
            }
            _ => i18n.t_arg("prov-rule", "name", n),
        },
        Prov::Lock(rule) => i18n.t_arg("prov-lock", "rule", &lock_words(i18n, rule)),
    }
}

/// The words of a locking rule (C-08): the shipped ones are translated, a
/// rule of the user's file shows its id.
pub fn lock_words(i18n: &I18n, rule: &str) -> String {
    // An argument of a function whose arguments are identifiers (C-06).
    if let Some(f) = rule.strip_prefix("function:") {
        return i18n.t_arg("lock-function", "fn", f);
    }
    let key = match rule {
        "vmad-off" => "lock-vmad-off",
        "key" | "key-text" => "lock-key",
        "path" | "path-text" => "lock-path",
        "noletter" => "lock-noletter",
        "word" => "lock-word",
        "ident" => "lock-ident",
        "compiled" => "lock-compiled",
        // The exclusion rules (B-18).
        "hex" => "lock-hex",
        "ident-text" => "lock-ident-text",
        "url" => "lock-url",
        "tag-only" => "lock-tag-only",
        "mcm-config" | "json-ids" => "lock-key-file",
        "edid" => "lock-edid",
        "ifkey" => "lock-ifkey",
        "file" => "lock-file",
        _ => return rule.to_string(),
    };
    i18n.t(key)
}

/// The column's text (R-B09-04): « identique » for a translation equal to
/// the source, the short provenance otherwise; empty for a line without
/// translation.
pub fn short_label(i18n: &I18n, e: &TmEntry) -> String {
    // A locked line says so, translation or not (C-08).
    if let p @ Prov::Lock(_) = parse(&e.prov) {
        return words(i18n, &p, false);
    }
    if e.trans.is_empty() {
        return String::new();
    }
    if is_identical(e) {
        return i18n.t("prov-identical");
    }
    with_ambiguity(i18n, words(i18n, &parse(&e.prov), false), &e.prov)
}

/// « … · ambiguë (n) » after the words of a marked line (B-08).
fn with_ambiguity(i18n: &I18n, label: String, prov: &str) -> String {
    match split_ambiguous(prov).1 {
        Some(n) => i18n.t_args(
            "prov-ambiguous",
            &[("provenance", &label), ("n", &n.to_string())],
        ),
        None => label,
    }
}

/// The whole provenance as stored (tooltip, Enregistrement tab); `None`
/// without translation or provenance.
pub fn full_label(i18n: &I18n, e: &TmEntry) -> Option<String> {
    if let p @ Prov::Lock(_) = parse(&e.prov) {
        return Some(words(i18n, &p, true));
    }
    if e.trans.is_empty() {
        return None;
    }
    match parse(&e.prov) {
        Prov::None => None,
        p => Some(with_ambiguity(i18n, words(i18n, &p, true), &e.prov)),
    }
}

/// « Statut — provenance » for the tooltip of the status cell.
pub fn tip(i18n: &I18n, e: &TmEntry) -> Option<String> {
    let full = full_label(i18n, e)?;
    let status = i18n.t(crate::status::derive(e).legend_key());
    Some(i18n.t_args("origin-tip", &[("status", &status), ("provenance", &full)]))
}

/// Whether a dictionary file gave the current translation of a line (the
/// counts of the dictionary tree, R-B02-04).
pub fn from_file(e: &TmEntry, name: &str) -> bool {
    !e.trans.is_empty()
        && matches!(parse(&e.prov),
            Prov::Dict { file, .. } | Prov::Normalized { file, .. } if file == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(src: &str, tr: &str, prov: &str) -> TmEntry {
        TmEntry {
            source: src.into(),
            trans: tr.into(),
            prov: prov.into(),
            ..Default::default()
        }
    }

    #[test]
    fn codes_round_trip() {
        let c = dict(Role::Game, FillLevel::Exact, "Skyrim_ENG-FRA.ximdb");
        assert_eq!(c, "dict:game:exact:Skyrim_ENG-FRA.ximdb");
        assert_eq!(
            parse(&c),
            Prov::Dict {
                role: Role::Game,
                level: FillLevel::Exact,
                file: "Skyrim_ENG-FRA.ximdb".into()
            }
        );
        // A file name with a colon-free path part and an ambiguity suffix.
        let amb = format!("{}|amb:3", dict(Role::Mod, FillLevel::Text, "a b.ximdb"));
        assert_eq!(split_ambiguous(&amb).1, Some(3));
        assert!(matches!(
            parse(&amb),
            Prov::Dict {
                role: Role::Mod,
                ..
            }
        ));
        assert_eq!(parse(&fuzzy(91)), Prov::Fuzzy(91));
        assert_eq!(parse(&online("deepl")), Prov::Online("deepl".into()));
        assert_eq!(parse(&import("sst")), Prov::Import("sst".into()));
        assert_eq!(parse("typed"), Prov::Typed);
        assert_eq!(parse("rule:x"), Prov::Rule("x".into()));
        assert_eq!(parse(""), Prov::None);
        assert_eq!(parse("nonsense"), Prov::None);
        assert_eq!(parse("dict:nope:exact:f"), Prov::None);
        // A dictionary level that is not one gives the matching code.
        assert_eq!(dict(Role::External, FillLevel::Fuzzy(80), "f"), "fuzzy:80");
        assert_eq!(
            dict(Role::External, FillLevel::Online, "deepl"),
            "online:deepl"
        );
        assert_eq!(dict(Role::External, FillLevel::TargetFile, "f"), TARGET);
    }

    #[test]
    fn settle_follows_the_rules() {
        // Typing over a dictionary line: « saisie » (R-B09-02).
        let mut e = entry("Hat", "Chapeau !", "dict:game:exact:S.ximdb");
        settle(&mut e, "Chapeau", "dict:game:exact:S.ximdb", Some(TYPED));
        assert_eq!(e.prov, TYPED);
        // An operation that set its own provenance keeps it.
        let mut e = entry("Hat", "Chapeau", "online:deepl");
        settle(&mut e, "", "", Some(TYPED));
        assert_eq!(e.prov, "online:deepl");
        // Emptied: no provenance.
        let mut e = entry("Hat", "", "typed");
        settle(&mut e, "Chapeau", "typed", Some(TYPED));
        assert!(e.prov.is_empty());
        // A status change (same text) keeps the provenance.
        let mut e = entry("Hat", "Chapeau", "online:deepl");
        settle(&mut e, "Chapeau", "online:deepl", Some(TYPED));
        assert_eq!(e.prov, "online:deepl");
    }

    #[test]
    fn categories_and_identical() {
        // T-B09-06: equal to the source → « identique », whatever stored.
        let e = entry("Whiterun ", " Whiterun", "online:deepl");
        assert!(is_identical(&e));
        assert_eq!(ProvFilter::of(&e), Some(ProvFilter::Identical));
        assert!(!ProvFilter::Online.matches(&e));
        assert!(ProvFilter::Identical.matches(&e));
        assert_eq!(ProvFilter::of(&entry("a", "", "")), Some(ProvFilter::Empty));
        let g = entry("a", "b", "dict:game:text:S.ximdb");
        assert_eq!(ProvFilter::of(&g), Some(ProvFilter::Game));
        let p = entry("a", "b", "dict:personal:exact:p.ximdb");
        assert_eq!(ProvFilter::of(&p), Some(ProvFilter::Personal));
        let x = entry("a", "b", "dict:textonly:textonly:t.ximdb");
        assert_eq!(ProvFilter::of(&x), Some(ProvFilter::OtherDict));
        assert_eq!(
            ProvFilter::of(&entry("a", "b", "target")),
            Some(ProvFilter::Import)
        );
        assert!(ProvFilter::All.matches(&entry("a", "b", "")));
        assert!(from_file(&g, "S.ximdb"));
        assert!(!from_file(&g, "T.ximdb"));
    }

    #[test]
    fn normalized_codes() {
        let r = Reports {
            bits: Reports::NUMBERS | Reports::PUNCT,
            amb: 0,
        };
        let c = normalized(Role::Mod, "M_ENG-FRA.ximdb", r, "You have 3 days | left!");
        assert_eq!(
            c,
            "dict:mod:norm:M_ENG-FRA.ximdb|rep:np|src:You have 3 days %7C left!"
        );
        assert_eq!(
            parse(&c),
            Prov::Normalized {
                role: Role::Mod,
                file: "M_ENG-FRA.ximdb".into(),
                reports: r,
                source: "You have 3 days | left!".into(),
            }
        );
        let e = entry("a", "b", &c);
        assert_eq!(ProvFilter::of(&e), Some(ProvFilter::Normalized));
        assert!(from_file(&e, "M_ENG-FRA.ximdb"));
        // Ambiguous: the mark comes last, the count reads back.
        let amb = normalized(Role::Game, "S.ximdb", Reports { bits: 0, amb: 3 }, "x");
        assert!(amb.ends_with("|amb:3"));
        assert!(matches!(parse(&amb), Prov::Normalized { reports, .. } if reports.amb == 3));
        assert_eq!(
            dict(Role::Game, FillLevel::Normalized(r), "S.ximdb"),
            "dict:game:norm:S.ximdb|rep:np"
        );
    }

    #[test]
    fn imported_lines_are_marked() {
        let mut es = vec![
            entry("a", "b", ""),
            entry("c", "", ""),
            entry("d", "e", "typed"),
        ];
        mark_imported(&mut es, "sst");
        assert_eq!(es[0].prov, "import:sst");
        assert!(es[1].prov.is_empty());
        assert_eq!(es[2].prov, "typed");
    }
}
