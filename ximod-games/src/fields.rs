//! Translatable fields per game, as data (A-06).
//!
//! What is extracted as a translatable line, and under which constraints, is
//! described by one JSON file per game, `assets/data/fields/<id>.json`
//! (shipped, also embedded in this crate as the last-resort copy) that a file
//! of the same name in the user's `data/fields/` folder replaces entirely
//! (R-A06-01). A file may `inherits` another one: its entries complete or
//! replace the parent's, signature by signature (R-A06-08).
//!
//! An entry is either a **signature** rule (`"sig": "WEAP:FULL"`) or a
//! **pattern** rule (`"edid": "LOC_FN_*"` on the record's editor id, or
//! `"value": "LOC_*"` on the field's own text). Signature rules decide whether
//! a field is extracted; pattern rules can only *downgrade* what a signature
//! rule allowed (R-A06-02). The modes:
//!
//! * `translate` — extracted and editable;
//! * `locked` — extracted, shown, locked by default (status Ignoré), the user
//!   may unlock it;
//! * `ignore` — never extracted.
//!
//! A signature rule may carry a context `rule` (R-A06-03) — `edidPrefix`,
//! `edidGlob`, `fieldEquals`, `fieldNotEquals`, `valuePrefix` — which must be
//! satisfied for the entry to apply; otherwise the field is ignored.
//!
//! Structural rules that describe the *format* rather than a translation
//! choice (a TES3 `CELL:NAME` only before the first `FRMR`, the TES4 header's
//! `CNAM`/`SNAM` always inline) stay in the engines.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Game;

/// Current file format: 2 admits the record joker `*:VMAD` (C-08); a
/// file of format 1 is read as before.
pub const FIELDS_FORMAT: u32 = 2;

/// The record part of a signature rule that applies to every record (C-08).
pub const ANY_RECORD: &str = "*";

/// What happens to a field (R-A06-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FieldMode {
    #[default]
    Translate,
    /// Extracted, shown, but never translated (status Ignoré): a key the
    /// game resolves elsewhere (`LOC_*`), a value kept verbatim…
    Locked,
    /// Extracted with the status Ignoré and flagged « never shown in game »:
    /// listed in red, hidden by default (plugin header, editor-only names).
    Unused,
    /// Not extracted at all.
    Ignore,
}

impl FieldMode {
    pub const ALL: [FieldMode; 4] = [
        FieldMode::Translate,
        FieldMode::Locked,
        FieldMode::Unused,
        FieldMode::Ignore,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FieldMode::Translate => "translate",
            FieldMode::Locked => "locked",
            FieldMode::Unused => "unused",
            FieldMode::Ignore => "ignore",
        }
    }

    /// Ordering used when a pattern rule downgrades a signature rule:
    /// `Translate` < `Locked` < `Unused` < `Ignore`.
    fn rank(self) -> u8 {
        match self {
            FieldMode::Translate => 0,
            FieldMode::Locked => 1,
            FieldMode::Unused => 2,
            FieldMode::Ignore => 3,
        }
    }
}

/// Context condition of a signature rule (R-A06-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContextRule {
    /// The record's editor id starts with `value`.
    EdidPrefix { value: String },
    /// The record's editor id matches the `*` pattern.
    EdidGlob { value: String },
    /// Another field of the record (its text) equals `value`.
    FieldEquals { field: String, value: String },
    /// Another field of the record (its text) differs from `value`.
    FieldNotEquals { field: String, value: String },
    /// The field's own text starts with `value`.
    ValuePrefix { value: String },
}

impl ContextRule {
    /// Kind names, for the editor's combo.
    pub const KINDS: [&'static str; 5] = [
        "edidPrefix",
        "edidGlob",
        "fieldEquals",
        "fieldNotEquals",
        "valuePrefix",
    ];

    pub fn kind(&self) -> &'static str {
        match self {
            ContextRule::EdidPrefix { .. } => "edidPrefix",
            ContextRule::EdidGlob { .. } => "edidGlob",
            ContextRule::FieldEquals { .. } => "fieldEquals",
            ContextRule::FieldNotEquals { .. } => "fieldNotEquals",
            ContextRule::ValuePrefix { .. } => "valuePrefix",
        }
    }

    /// Build a rule from its kind and parameters (editor); `None` for an
    /// unknown kind.
    pub fn from_parts(kind: &str, field: &str, value: &str) -> Option<ContextRule> {
        let value = value.to_string();
        Some(match kind {
            "edidPrefix" => ContextRule::EdidPrefix { value },
            "edidGlob" => ContextRule::EdidGlob { value },
            "fieldEquals" => ContextRule::FieldEquals {
                field: field.to_string(),
                value,
            },
            "fieldNotEquals" => ContextRule::FieldNotEquals {
                field: field.to_string(),
                value,
            },
            "valuePrefix" => ContextRule::ValuePrefix { value },
            _ => return None,
        })
    }

    /// `(field, value)` parameters (editor).
    pub fn parts(&self) -> (&str, &str) {
        match self {
            ContextRule::EdidPrefix { value }
            | ContextRule::EdidGlob { value }
            | ContextRule::ValuePrefix { value } => ("", value),
            ContextRule::FieldEquals { field, value }
            | ContextRule::FieldNotEquals { field, value } => (field, value),
        }
    }

    /// One-line rendering for the editor's table.
    pub fn summary(&self) -> String {
        match self {
            ContextRule::EdidPrefix { value } => format!("EDID ^{value}"),
            ContextRule::EdidGlob { value } => format!("EDID ~{value}"),
            ContextRule::FieldEquals { field, value } => format!("{field} = {value}"),
            ContextRule::FieldNotEquals { field, value } => format!("{field} ≠ {value}"),
            ContextRule::ValuePrefix { value } => format!("texte ^{value}"),
        }
    }

    fn satisfied(&self, ctx: &FieldCtx) -> bool {
        match self {
            ContextRule::EdidPrefix { value } => ctx.edid.starts_with(value.as_str()),
            ContextRule::EdidGlob { value } => glob_match(value, ctx.edid),
            ContextRule::FieldEquals { field, value } => {
                (ctx.other)(field).as_deref() == Some(value.as_str())
            }
            ContextRule::FieldNotEquals { field, value } => {
                (ctx.other)(field).as_deref() != Some(value.as_str())
            }
            ContextRule::ValuePrefix { value } => ctx.value.starts_with(value.as_str()),
        }
    }
}

/// One entry of a fields file (R-A06-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FieldRule {
    /// `"WEAP:FULL"` — signature rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
    /// `"LOC_FN_*"` — pattern rule on the record's editor id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edid: Option<String>,
    /// `"LOC_*"` — pattern rule on the field's own text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub mode: FieldMode,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub non_empty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
    /// Whether the engine shows a line break in this field: `None` while
    /// unknown (the editors then let Enter through), `Some(false)` when the
    /// field is single-line (A-05 fills the known values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<ContextRule>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl FieldRule {
    /// A plain `translate` signature rule.
    pub fn translate(rec: &str, fld: &str) -> FieldRule {
        FieldRule {
            sig: Some(format!("{rec}:{fld}")),
            mode: FieldMode::Translate,
            ..Default::default()
        }
    }

    /// The key an entry replaces in its parent (R-A06-08): the signature, or
    /// the pattern prefixed by its kind.
    pub fn key(&self) -> String {
        if let Some(s) = &self.sig {
            format!("sig:{s}")
        } else if let Some(e) = &self.edid {
            format!("edid:{e}")
        } else if let Some(v) = &self.value {
            format!("value:{v}")
        } else {
            String::new()
        }
    }

    /// `(record, field)` of a signature rule.
    pub fn split_sig(&self) -> Option<(&str, &str)> {
        let s = self.sig.as_deref()?;
        let (r, f) = s.split_once(':')?;
        Some((r, f))
    }

    pub fn is_pattern(&self) -> bool {
        self.sig.is_none() && (self.edid.is_some() || self.value.is_some())
    }
}

/// A fields file (`assets/data/fields/<id>.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldFile {
    pub format: u32,
    pub game: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    /// Engine tags compared between source and translation (A-07): tag
    /// names, a trailing `*` matching any continuation (`Alias*` covers
    /// `<Alias=…>` and `<Alias.Name=…>`). Empty: every `<…>` tag counts. A
    /// child file's non-empty list replaces its parent's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub fields: Vec<FieldRule>,
}

impl FieldFile {
    /// Parse a file, checking its format and (when given) that `game` matches
    /// the expected id (case-insensitive).
    pub fn from_json(text: &str, expected_game: Option<&str>) -> Result<FieldFile, String> {
        let f: FieldFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !(1..=FIELDS_FORMAT).contains(&f.format) {
            return Err(format!(
                "unsupported format {} (expected {FIELDS_FORMAT})",
                f.format
            ));
        }
        if let Some(exp) = expected_game
            && !f.game.eq_ignore_ascii_case(exp)
        {
            return Err(format!(
                "game '{}' does not match the file name ({exp})",
                f.game
            ));
        }
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("fields serialize")
    }

    /// The embedded copy of the shipped file for a game (`None` for
    /// `Game::Unknown`).
    pub fn builtin(game: Game) -> Option<FieldFile> {
        let text = builtin_text(game)?;
        Some(FieldFile::from_json(text, Some(game.id())).expect("embedded fields file valid"))
    }

    /// Validate the file for the editor (R-A06-05). Line numbers are 1-based
    /// indexes into `fields`.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (i, r) in self.fields.iter().enumerate() {
            let line = i + 1;
            let mut push = |reason: ValidationReason| issues.push(ValidationIssue { line, reason });
            let n_keys = r.sig.is_some() as u8 + r.edid.is_some() as u8 + r.value.is_some() as u8;
            if n_keys == 0 {
                push(ValidationReason::MissingKey);
            } else if n_keys > 1 {
                push(ValidationReason::SeveralKeys);
            }
            if let Some(s) = &r.sig {
                let sig4 = |t: &str| {
                    t.len() == 4 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                };
                let ok = s
                    .split_once(':')
                    .is_some_and(|(r, f)| (r == ANY_RECORD || sig4(r)) && sig4(f));
                if !ok {
                    push(ValidationReason::BadSignature(s.clone()));
                }
            }
            for p in [&r.edid, &r.value].into_iter().flatten() {
                if p.trim().is_empty() {
                    push(ValidationReason::EmptyPattern);
                }
            }
            if r.max_chars == Some(0) || r.max_bytes == Some(0) {
                push(ValidationReason::ZeroMax);
            }
            if r.is_pattern() && r.rule.is_some() {
                push(ValidationReason::RuleOnPattern);
            }
            if let Some(rule) = &r.rule {
                let (f, v) = rule.parts();
                let needs_field = matches!(
                    rule,
                    ContextRule::FieldEquals { .. } | ContextRule::FieldNotEquals { .. }
                );
                if v.is_empty() || (needs_field && f.len() != 4) {
                    push(ValidationReason::BadRule);
                }
            }
            let key = r.key();
            if !key.is_empty() && !seen.insert(key.clone()) {
                push(ValidationReason::Duplicate(key));
            }
        }
        issues
    }
}

/// A validation problem (R-A06-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// 1-based index of the entry in `fields`.
    pub line: usize,
    pub reason: ValidationReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationReason {
    /// Neither `sig`, `edid` nor `value`.
    MissingKey,
    /// More than one of `sig`, `edid`, `value`.
    SeveralKeys,
    /// Not `AAAA:BBBB`.
    BadSignature(String),
    EmptyPattern,
    ZeroMax,
    /// A context rule on a pattern entry.
    RuleOnPattern,
    /// Empty value, or a field condition without a 4-letter field.
    BadRule,
    Duplicate(String),
}

impl ValidationReason {
    /// i18n key of the message (the argument, when any, is the offending text).
    pub fn i18n_key(&self) -> &'static str {
        match self {
            ValidationReason::MissingKey => "fields-err-missing-key",
            ValidationReason::SeveralKeys => "fields-err-several-keys",
            ValidationReason::BadSignature(_) => "fields-err-bad-sig",
            ValidationReason::EmptyPattern => "fields-err-empty-pattern",
            ValidationReason::ZeroMax => "fields-err-zero-max",
            ValidationReason::RuleOnPattern => "fields-err-rule-on-pattern",
            ValidationReason::BadRule => "fields-err-bad-rule",
            ValidationReason::Duplicate(_) => "fields-err-duplicate",
        }
    }

    pub fn arg(&self) -> &str {
        match self {
            ValidationReason::BadSignature(s) | ValidationReason::Duplicate(s) => s,
            _ => "",
        }
    }
}

/// What the reader knows about a field when asking for its mode.
pub struct FieldCtx<'a> {
    /// The record's editor id (empty when it has none).
    pub edid: &'a str,
    /// The field's own decoded text (empty for a localized string id).
    pub value: &'a str,
    /// Text of another field of the same record, by signature.
    pub other: &'a dyn Fn(&str) -> Option<String>,
}

/// The resolved table of one game: the merged rules, ready to answer.
#[derive(Debug, Clone, Default)]
pub struct FieldTable {
    /// Engine tags of the game (A-07), from the nearest file of the chain
    /// that lists any.
    tags: Vec<String>,
    by_sig: HashMap<(String, String), FieldRule>,
    patterns: Vec<FieldRule>,
    /// Records that have at least one non-ignored signature rule.
    records: HashSet<String>,
    /// Field signatures that carry text somewhere in the table.
    string_fields: HashSet<String>,
    /// Fields of a joker rule `*:XXXX` (C-08): any record may have them.
    any_record: HashSet<String>,
    /// Where the rules came from (for the startup log and the editor).
    pub source: FieldSource,
}

/// Origin of a game's table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FieldSource {
    #[default]
    Builtin,
    Shipped(PathBuf),
    User(PathBuf),
}

/// A file that could not be used (R-A06-07).
#[derive(Debug, Clone)]
pub struct FieldLoadIssue {
    pub game: Game,
    pub path: PathBuf,
    pub reason: String,
}

impl FieldTable {
    /// Build a table from an already-merged list of rules.
    pub fn from_rules(rules: &[FieldRule], source: FieldSource) -> FieldTable {
        let mut t = FieldTable {
            source,
            ..Default::default()
        };
        for r in rules {
            if let Some((rec, fld)) = r.split_sig() {
                if r.mode != FieldMode::Ignore {
                    if rec == ANY_RECORD {
                        // A structured field (VMAD), not a string: neither
                        // the record list nor the string signatures.
                        t.any_record.insert(fld.to_string());
                    } else {
                        t.records.insert(rec.to_string());
                        t.string_fields.insert(fld.to_string());
                    }
                }
                t.by_sig
                    .insert((rec.to_string(), fld.to_string()), r.clone());
            } else if r.is_pattern() {
                t.patterns.push(r.clone());
            }
        }
        t
    }

    /// Resolve a game's table (R-A06-01/07/08): the user file when present and
    /// valid, else the shipped file, else the embedded copy; parents by
    /// `inherits` the same way. Invalid files are reported and skipped.
    pub fn load(
        shipped_dir: &Path,
        user_dir: Option<&Path>,
        game: Game,
    ) -> (FieldTable, Vec<FieldLoadIssue>) {
        let mut issues = Vec::new();
        let table = Self::resolve(game, |g| load_one(shipped_dir, user_dir, g, &mut issues));
        (table, issues)
    }

    /// The table made only of the embedded files (inheritance resolved).
    pub fn builtin(game: Game) -> FieldTable {
        Self::resolve(game, |g| {
            FieldFile::builtin(g).map(|f| (f, FieldSource::Builtin))
        })
    }

    /// The files a game's table is made of, child first (the game's own file)
    /// then its parents (R-A06-08), each with where it came from. The editor
    /// uses it to tell inherited entries from the game's own.
    pub fn load_chain(
        shipped_dir: &Path,
        user_dir: Option<&Path>,
        game: Game,
    ) -> (Vec<ChainLink>, Vec<FieldLoadIssue>) {
        let mut issues = Vec::new();
        let chain = follow_chain(game, |g| load_one(shipped_dir, user_dir, g, &mut issues));
        (chain, issues)
    }

    /// Merge a chain (child first) into one rule list: parents first, children
    /// replacing by key; first-seen order kept.
    pub fn merge_chain(chain: &[ChainLink]) -> Vec<FieldRule> {
        let mut merged: BTreeMap<String, FieldRule> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for link in chain.iter().rev() {
            for r in &link.file.fields {
                let k = r.key();
                if merged.insert(k.clone(), r.clone()).is_none() {
                    order.push(k);
                }
            }
        }
        order
            .iter()
            .filter_map(|k| merged.get(k).cloned())
            .collect()
    }

    /// Follow the `inherits` chain with `loader`, then merge.
    fn resolve(
        game: Game,
        loader: impl FnMut(Game) -> Option<(FieldFile, FieldSource)>,
    ) -> FieldTable {
        let chain = follow_chain(game, loader);
        let rules = Self::merge_chain(&chain);
        let source = chain.first().map(|l| l.source.clone()).unwrap_or_default();
        let mut t = FieldTable::from_rules(&rules, source);
        t.tags = chain
            .iter()
            .map(|l| &l.file.tags)
            .find(|t| !t.is_empty())
            .cloned()
            .unwrap_or_default();
        t
    }

    /// The engine tags compared by A-07 (see [`FieldFile::tags`]).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Table with explicit tags (tests, editors).
    pub fn with_tags(mut self, tags: Vec<String>) -> FieldTable {
        self.tags = tags;
        self
    }

    /// The mode of a field in context: signature rule → context condition →
    /// pattern downgrades (R-A06-02/03). Absent ⇒ `Ignore`.
    pub fn mode(&self, rec: &str, fld: &str, ctx: &FieldCtx) -> FieldMode {
        let Some(rule) = self.rule(rec, fld) else {
            return FieldMode::Ignore;
        };
        let mut mode = rule.mode;
        if mode == FieldMode::Ignore {
            return mode;
        }
        if let Some(cond) = &rule.rule
            && !cond.satisfied(ctx)
        {
            return FieldMode::Ignore;
        }
        for p in &self.patterns {
            let hit = match (&p.edid, &p.value) {
                (Some(g), _) => glob_match(g, ctx.edid),
                (None, Some(g)) => glob_match(g, ctx.value),
                _ => false,
            };
            if hit && p.mode.rank() > mode.rank() {
                mode = p.mode;
            }
        }
        mode
    }

    /// Whether a record type may produce lines at all (lets the reader skip
    /// whole records without decompressing them).
    pub fn record_may_have_fields(&self, rec: &str) -> bool {
        self.records.contains(rec)
    }

    /// Whether a signature rule lists the field with a non-`ignore` mode
    /// (context and patterns aside).
    pub fn is_translatable(&self, rec: &str, fld: &str) -> bool {
        self.rule(rec, fld)
            .is_some_and(|r| r.mode != FieldMode::Ignore)
    }

    /// The signature rule of a field (limits for A-05), if any.
    pub fn limits(&self, rec: &str, fld: &str) -> Option<&FieldRule> {
        self.rule(rec, fld)
    }

    /// The rule of `rec:fld`, else the joker `*:fld` (C-08).
    fn rule(&self, rec: &str, fld: &str) -> Option<&FieldRule> {
        self.by_sig
            .get(&(rec.to_string(), fld.to_string()))
            .or_else(|| self.by_sig.get(&(ANY_RECORD.to_string(), fld.to_string())))
    }

    /// The fields of the joker rules (`*:VMAD`): any record may carry them,
    /// which a reader that skips records by type must look for (C-08).
    pub fn any_record_fields(&self) -> &HashSet<String> {
        &self.any_record
    }

    /// Whether a field signature carries text somewhere in this table (used by
    /// delocalization to tell a stray lstring from a FormID).
    pub fn is_string_field(&self, fld: &str) -> bool {
        self.string_fields.contains(fld)
    }

    /// Every signature rule, sorted (for tests and the editor).
    pub fn signatures(&self) -> Vec<&FieldRule> {
        let mut v: Vec<&FieldRule> = self.by_sig.values().collect();
        v.sort_by(|a, b| a.sig.cmp(&b.sig));
        v
    }
}

/// One file of an inheritance chain.
#[derive(Debug, Clone)]
pub struct ChainLink {
    pub game: Game,
    pub file: FieldFile,
    pub source: FieldSource,
}

fn follow_chain(
    game: Game,
    mut loader: impl FnMut(Game) -> Option<(FieldFile, FieldSource)>,
) -> Vec<ChainLink> {
    let mut chain: Vec<ChainLink> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut current = Some(game);
    while let Some(g) = current {
        if !visited.insert(g.id().to_lowercase()) {
            break; // inheritance cycle
        }
        let Some((file, source)) = loader(g) else {
            break;
        };
        current = file
            .inherits
            .as_deref()
            .map(Game::from_id)
            .filter(|p| *p != Game::Unknown);
        chain.push(ChainLink {
            game: g,
            file,
            source,
        });
    }
    chain
}

fn load_one(
    shipped_dir: &Path,
    user_dir: Option<&Path>,
    game: Game,
    issues: &mut Vec<FieldLoadIssue>,
) -> Option<(FieldFile, FieldSource)> {
    let name = format!("{}.json", game.id());
    if let Some(ud) = user_dir {
        let path = ud.join(&name);
        if path.is_file() {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| FieldFile::from_json(&t, Some(game.id())))
            {
                Ok(f) => return Some((f, FieldSource::User(path))),
                Err(reason) => issues.push(FieldLoadIssue { game, path, reason }),
            }
        }
    }
    let path = shipped_dir.join(&name);
    if path.is_file() {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| FieldFile::from_json(&t, Some(game.id())))
        {
            Ok(f) => return Some((f, FieldSource::Shipped(path))),
            Err(reason) => issues.push(FieldLoadIssue { game, path, reason }),
        }
    }
    FieldFile::builtin(game).map(|f| (f, FieldSource::Builtin))
}

/// The embedded shipped files (kept in sync with `assets/data/fields`; a test
/// checks it).
pub fn builtin_text(game: Game) -> Option<&'static str> {
    macro_rules! f {
        ($n:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../assets/data/fields/",
                $n,
                ".json"
            ))
        };
    }
    Some(match game {
        Game::Morrowind => f!("Morrowind"),
        Game::Oblivion => f!("Oblivion"),
        Game::OblivionRD => f!("OblivionRD"),
        Game::Skyrim => f!("Skyrim"),
        Game::SkyrimSE => f!("SkyrimSE"),
        Game::Fallout3 => f!("Fallout3"),
        Game::FalloutNV => f!("FalloutNV"),
        Game::Fallout4 => f!("Fallout4"),
        Game::Fallout76 => f!("Fallout76"),
        Game::Starfield => f!("Starfield"),
        Game::Unknown => return None,
    })
}

/// `*`-only glob match (case-sensitive, `*` matches any run including empty).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((b'*', rest)) => (0..=t.len()).any(|i| rec(rest, &t[i..])),
            Some((c, rest)) => t.first() == Some(c) && rec(rest, &t[1..]),
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn ctx<'a>(edid: &'a str, value: &'a str) -> FieldCtx<'a> {
        FieldCtx {
            edid,
            value,
            other: &none,
        }
    }

    #[test]
    fn glob() {
        assert!(glob_match("LOC_FN_*", "LOC_FN_Abc"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*c", "abbbc"));
        assert!(!glob_match("a*c", "abbbd"));
        assert!(glob_match("abc", "abc"));
        assert!(!glob_match("abc", "ab"));
    }

    #[test]
    fn signature_context_and_patterns() {
        let rules = vec![
            FieldRule::translate("WEAP", "FULL"),
            FieldRule {
                sig: Some("GMST:DATA".into()),
                rule: Some(ContextRule::EdidPrefix { value: "s".into() }),
                ..Default::default()
            },
            FieldRule {
                sig: Some("MSTT:FULL".into()),
                mode: FieldMode::Ignore,
                ..Default::default()
            },
            FieldRule {
                value: Some("LOC_*".into()),
                mode: FieldMode::Locked,
                ..Default::default()
            },
            FieldRule {
                edid: Some("Dbg*".into()),
                mode: FieldMode::Ignore,
                ..Default::default()
            },
        ];
        let t = FieldTable::from_rules(&rules, FieldSource::Builtin);
        assert_eq!(
            t.mode("WEAP", "FULL", &ctx("Sword", "Épée")),
            FieldMode::Translate
        );
        assert_eq!(
            t.mode("WEAP", "DESC", &ctx("Sword", "x")),
            FieldMode::Ignore
        );
        assert_eq!(
            t.mode("GMST", "DATA", &ctx("sHello", "x")),
            FieldMode::Translate
        );
        assert_eq!(
            t.mode("GMST", "DATA", &ctx("fHello", "x")),
            FieldMode::Ignore
        );
        assert_eq!(t.mode("MSTT", "FULL", &ctx("a", "x")), FieldMode::Ignore);
        assert_eq!(
            t.mode("WEAP", "FULL", &ctx("Sword", "LOC_FN_Sword")),
            FieldMode::Locked
        );
        assert_eq!(
            t.mode("WEAP", "FULL", &ctx("DbgSword", "x")),
            FieldMode::Ignore
        );
        assert!(t.record_may_have_fields("WEAP"));
        assert!(!t.record_may_have_fields("MSTT"));
        assert!(!t.record_may_have_fields("REFR"));
        assert!(t.is_string_field("FULL"));
        assert!(!t.is_string_field("EDID"));
    }

    #[test]
    fn field_conditions() {
        let other = |f: &str| {
            if f == "DNAM" {
                Some("yes".to_string())
            } else {
                None
            }
        };
        let c = FieldCtx {
            edid: "",
            value: "",
            other: &other,
        };
        assert!(
            ContextRule::FieldEquals {
                field: "DNAM".into(),
                value: "yes".into()
            }
            .satisfied(&c)
        );
        assert!(
            !ContextRule::FieldEquals {
                field: "DNAM".into(),
                value: "no".into()
            }
            .satisfied(&c)
        );
        assert!(
            ContextRule::FieldNotEquals {
                field: "DNAM".into(),
                value: "no".into()
            }
            .satisfied(&c)
        );
        assert!(
            ContextRule::FieldNotEquals {
                field: "XXXX".into(),
                value: "no".into()
            }
            .satisfied(&c)
        );
        assert!(
            ContextRule::EdidGlob {
                value: "*Note*".into()
            }
            .satisfied(&ctx("MyNote1", ""))
        );
    }

    #[test]
    fn validation() {
        let f = FieldFile {
            format: 1,
            game: "Skyrim".into(),
            inherits: None,
            tags: vec![],
            fields: vec![
                FieldRule::translate("WEAP", "FULL"),
                FieldRule::translate("WEAP", "FULL"),
                FieldRule {
                    sig: Some("WEAPFULL".into()),
                    ..Default::default()
                },
                FieldRule::default(),
                FieldRule {
                    sig: Some("ARMO:FULL".into()),
                    edid: Some("x".into()),
                    ..Default::default()
                },
                FieldRule {
                    sig: Some("BOOK:FULL".into()),
                    max_chars: Some(0),
                    ..Default::default()
                },
                FieldRule {
                    sig: Some("MISC:FULL".into()),
                    rule: Some(ContextRule::FieldEquals {
                        field: "DN".into(),
                        value: "x".into(),
                    }),
                    ..Default::default()
                },
                FieldRule {
                    edid: Some("LOC_*".into()),
                    rule: Some(ContextRule::EdidPrefix { value: "a".into() }),
                    ..Default::default()
                },
            ],
        };
        let issues = f.validate();
        let reasons: Vec<(usize, &ValidationReason)> =
            issues.iter().map(|i| (i.line, &i.reason)).collect();
        assert!(reasons.contains(&(2, &ValidationReason::Duplicate("sig:WEAP:FULL".into()))));
        assert!(reasons.contains(&(3, &ValidationReason::BadSignature("WEAPFULL".into()))));
        assert!(reasons.contains(&(4, &ValidationReason::MissingKey)));
        assert!(reasons.contains(&(5, &ValidationReason::SeveralKeys)));
        assert!(reasons.contains(&(6, &ValidationReason::ZeroMax)));
        assert!(reasons.contains(&(7, &ValidationReason::BadRule)));
        assert!(reasons.contains(&(8, &ValidationReason::RuleOnPattern)));
        assert!(
            FieldFile::builtin(Game::SkyrimSE)
                .unwrap()
                .validate()
                .is_empty()
        );
    }

    #[test]
    fn json_round_trip_and_format_check() {
        let f = FieldFile::builtin(Game::Starfield).unwrap();
        let back = FieldFile::from_json(&f.to_json(), Some("Starfield")).unwrap();
        assert_eq!(f, back);
        assert!(FieldFile::from_json(&f.to_json(), Some("Skyrim")).is_err());
        assert!(
            FieldFile::from_json("{\"format\":9,\"game\":\"Starfield\",\"fields\":[]}", None)
                .is_err()
        );
        assert!(FieldFile::from_json("not json", None).is_err());
    }

    #[test]
    fn newline_is_tri_state() {
        // Absent → unknown (editors let Enter through); true / false kept.
        let f = FieldFile::builtin(Game::Skyrim).unwrap();
        let mut j: serde_json::Value = serde_json::from_str(&f.to_json()).unwrap();
        j["fields"] = serde_json::json!([
            {"sig": "WEAP:FULL", "mode": "translate", "newline": false},
            {"sig": "BOOK:DESC", "mode": "translate", "newline": true},
            {"sig": "MESG:DESC", "mode": "translate"}
        ]);
        let back = FieldFile::from_json(&j.to_string(), Some("Skyrim")).unwrap();
        let by = |sig: &str| {
            back.fields
                .iter()
                .find(|r| r.sig.as_deref() == Some(sig))
                .unwrap()
                .newline
        };
        assert_eq!(by("WEAP:FULL"), Some(false));
        assert_eq!(by("BOOK:DESC"), Some(true));
        assert_eq!(by("MESG:DESC"), None);
        // An unknown flag is not written back.
        assert!(!back.to_json().contains("MESG:DESC\",\n      \"newline"));
    }

    #[test]
    fn builtin_every_game() {
        for g in Game::ALL {
            let f = FieldFile::builtin(*g).unwrap_or_else(|| panic!("{g:?}"));
            assert!(f.validate().is_empty(), "{g:?}");
        }
        assert!(FieldFile::builtin(Game::Unknown).is_none());
    }

    #[test]
    fn load_with_inheritance_and_user_override() {
        let tmp = std::env::temp_dir().join(format!("ximod-fields-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let shipped = tmp.join("shipped");
        let user = tmp.join("user");
        std::fs::create_dir_all(&shipped).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        // Shipped Skyrim: WEAP:FULL translate; shipped SkyrimSE inherits and
        // ignores it, adds ARMO:DESC.
        let sky = FieldFile {
            format: 1,
            game: "Skyrim".into(),
            inherits: None,
            tags: vec![],
            fields: vec![
                FieldRule::translate("WEAP", "FULL"),
                FieldRule::translate("BOOK", "FULL"),
            ],
        };
        let sse = FieldFile {
            format: 1,
            game: "SkyrimSE".into(),
            inherits: Some("Skyrim".into()),
            tags: vec![],
            fields: vec![
                FieldRule {
                    sig: Some("WEAP:FULL".into()),
                    mode: FieldMode::Ignore,
                    ..Default::default()
                },
                FieldRule::translate("ARMO", "DESC"),
            ],
        };
        std::fs::write(shipped.join("Skyrim.json"), sky.to_json()).unwrap();
        std::fs::write(shipped.join("SkyrimSE.json"), sse.to_json()).unwrap();
        let (t, issues) = FieldTable::load(&shipped, Some(&user), Game::SkyrimSE);
        assert!(issues.is_empty());
        assert_eq!(t.mode("WEAP", "FULL", &ctx("", "")), FieldMode::Ignore);
        assert_eq!(t.mode("BOOK", "FULL", &ctx("", "")), FieldMode::Translate);
        assert_eq!(t.mode("ARMO", "DESC", &ctx("", "")), FieldMode::Translate);
        assert!(matches!(t.source, FieldSource::Shipped(_)));

        // A valid user file replaces the shipped SkyrimSE entirely (parent kept).
        let user_sse = FieldFile {
            format: 1,
            game: "SkyrimSE".into(),
            inherits: Some("Skyrim".into()),
            tags: vec![],
            fields: vec![FieldRule::translate("MISC", "FULL")],
        };
        std::fs::write(user.join("SkyrimSE.json"), user_sse.to_json()).unwrap();
        let (t, issues) = FieldTable::load(&shipped, Some(&user), Game::SkyrimSE);
        assert!(issues.is_empty());
        assert_eq!(t.mode("WEAP", "FULL", &ctx("", "")), FieldMode::Translate); // from the parent again
        assert_eq!(t.mode("ARMO", "DESC", &ctx("", "")), FieldMode::Ignore);
        assert_eq!(t.mode("MISC", "FULL", &ctx("", "")), FieldMode::Translate);
        assert!(matches!(t.source, FieldSource::User(_)));

        // A corrupt user file is reported and the shipped one used (R-A06-07).
        std::fs::write(user.join("SkyrimSE.json"), "{ corrupt").unwrap();
        let (t, issues) = FieldTable::load(&shipped, Some(&user), Game::SkyrimSE);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].game, Game::SkyrimSE);
        assert_eq!(t.mode("ARMO", "DESC", &ctx("", "")), FieldMode::Translate);

        // No file anywhere: the embedded copy.
        let (t, issues) = FieldTable::load(&tmp.join("nowhere"), None, Game::Morrowind);
        assert!(issues.is_empty());
        assert_eq!(t.source, FieldSource::Builtin);
        assert_eq!(t.mode("BOOK", "FNAM", &ctx("", "")), FieldMode::Translate);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
