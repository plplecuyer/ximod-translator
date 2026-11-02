//! The exclusion rules (B-18): which lines are identifiers, keys, paths or
//! tags rather than texts, locked Ignoré with the provenance `lock:<id>`.
//!
//! Three levels:
//!
//! * **shipped** and **user** rules — `assets/data/rules/exclusions.json`
//!   and the user's copy `data/rules/exclusions.json`, which replaces it
//!   (A.4.4); it absorbs the former `papyrus_rules.json` (D-73): the
//!   patterns of the scripts' strings and the functions whose arguments
//!   are identifiers (C-06);
//! * **deduced** rules, computed per document and never written: the EDIDs
//!   of the plugin, the keys of its interface files, the names of its
//!   files ([`Deduced`]);
//! * **learned** rules, made from a line set Ignoré by hand
//!   ([`Proposal`]): local ones live in the header of the mod's dictionary
//!   (and the pivot's meta), global ones in the user's copy (D-74).
//!
//! The rules lock only the lines a person has not decided: without
//! translation, À traduire and without provenance; a line unlocked by hand
//! (a status or a translation, provenance « saisie ») is never locked
//! again. They are laid after the dictionaries filled the document, and
//! lifted before ([`release`], [`apply`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};

use crate::status::{self, Status};
use crate::tm::TmEntry;

/// Current format of the file.
pub const FORMAT: u32 = 1;
/// The folder of the file, in `assets/data/` and the user's `data/`.
pub const DIR: &str = "rules";
/// The file name.
pub const FILE: &str = "exclusions.json";
/// The provenance prefix of a lock.
pub const LOCK: &str = crate::vmad_lock::LOCK;
/// The ids of the deduced rules.
pub const DEDUCED_EDID: &str = "edid";
pub const DEDUCED_IFKEY: &str = "ifkey";
pub const DEDUCED_FILE: &str = "file";
/// The id of the keys an engine locks (`LOC_*`, Oblivion Remastered).
pub const LOC_KEYS: &str = "loc-keys";

/// What a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// An expression on the text of the line (Rust syntax, Unicode).
    #[default]
    Regex,
    /// The signature `REC:FLD`, `*` wildcards, alternatives with `|`.
    Field,
    /// The EDID, `*` wildcards, case aside.
    Edid,
    /// A string passed to a script function (C-06).
    Function,
    /// The key of a line of a text file (C-05), `*` wildcards.
    Key,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Regex,
        Kind::Field,
        Kind::Edid,
        Kind::Function,
        Kind::Key,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Kind::Regex => "rules-kind-regex",
            Kind::Field => "rules-kind-field",
            Kind::Edid => "rules-kind-edid",
            Kind::Function => "rules-kind-function",
            Kind::Key => "rules-kind-key",
        }
    }
}

/// Which lines a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The lines of the records and of the text files.
    Text,
    /// The strings of the scripts: `.pex`, VMAD, SCTX.
    Script,
    /// The EDID (rules of kind edid).
    Edid,
    /// The signature (rules of kind field).
    Field,
}

impl Scope {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Scope::Text => "rules-scope-text",
            Scope::Script => "rules-scope-script",
            Scope::Edid => "rules-scope-edid",
            Scope::Field => "rules-scope-field",
        }
    }
}

/// Where a rule comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    #[default]
    Shipped,
    /// The game's sheet (`locksLocKeys`).
    Game,
    Learned,
    User,
}

/// How a learned rule was learned.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Learned {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// The EDID (or the key) of the line set Ignoré by hand.
    pub from: String,
}

fn star() -> String {
    "*".to_string()
}
fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_star(s: &str) -> bool {
    s == "*"
}
fn text_scope() -> Vec<Scope> {
    vec![Scope::Text]
}

/// One rule of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub kind: Kind,
    pub pattern: String,
    /// A second expression the text must also match (kind regex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub also: Option<String>,
    #[serde(default = "text_scope")]
    pub scope: Vec<Scope>,
    /// A game id, or `*`.
    #[serde(default = "star", skip_serializing_if = "is_star")]
    pub game: String,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// The text files a key rule is limited to (`*` wildcards, matched on
    /// the end of the path).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned: Option<Learned>,
}

impl Rule {
    pub fn new(id: &str, kind: Kind, pattern: &str, scope: Vec<Scope>) -> Rule {
        Rule {
            id: id.to_string(),
            kind,
            pattern: pattern.to_string(),
            also: None,
            scope,
            game: star(),
            origin: Origin::User,
            enabled: true,
            comment: String::new(),
            files: Vec::new(),
            learned: None,
        }
    }

    /// Whether the rule applies to documents of `game`.
    pub fn fits(&self, game: &str) -> bool {
        self.game == "*" || self.game.is_empty() || self.game.eq_ignore_ascii_case(game)
    }
}

/// What the deductions apply to, and the fields a generalization by field
/// must leave alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deduce {
    /// The lines the « EDID du plugin » rule applies to: `script` (VMAD,
    /// `.pex`, SCTX), `file` (the lines of text files) — never the
    /// displayed fields of the records.
    #[serde(rename = "edidScopes", default = "edid_scopes")]
    pub edid_scopes: Vec<String>,
    /// The fields shown in game (`REC:FLD`, `*` wildcards).
    #[serde(default)]
    pub displayed: Vec<String>,
}

fn edid_scopes() -> Vec<String> {
    vec!["script".into(), "file".into()]
}

impl Default for Deduce {
    fn default() -> Self {
        Deduce {
            edid_scopes: edid_scopes(),
            displayed: Vec::new(),
        }
    }
}

/// The file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesFile {
    pub format: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub deduce: Deduce,
}

impl Default for RulesFile {
    fn default() -> Self {
        RulesFile {
            format: FORMAT,
            rules: Vec::new(),
            functions: Vec::new(),
            deduce: Deduce::default(),
        }
    }
}

/// Why a file or a rule is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    Json(String),
    Format(u32),
    NoId(usize),
    IdDup(String),
    Empty(String),
    Regex { id: String, error: String },
    Sig { id: String, text: String },
    Scope(String),
}

impl RulesFile {
    pub fn parse(text: &str) -> Result<RulesFile, Invalid> {
        let f: RulesFile = serde_json::from_str(text).map_err(|e| Invalid::Json(e.to_string()))?;
        if f.format != FORMAT {
            return Err(Invalid::Format(f.format));
        }
        f.validate()?;
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// The embedded copy of the shipped file.
    pub fn embedded() -> RulesFile {
        RulesFile::parse(include_str!("../assets/data/rules/exclusions.json"))
            .expect("embedded exclusions.json valid")
    }

    /// R-B18-06: unique ids, a pattern, compilable expressions, `AAAA:BBBB`
    /// signatures, a scope.
    pub fn validate(&self) -> Result<(), Invalid> {
        validate_rules(&self.rules, &[])
    }

    pub fn find(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }
}

/// [`RulesFile::validate`] on a list, with the ids of `others` (the file's
/// rules, for the learned local ones) taken.
pub fn validate_rules(rules: &[Rule], others: &[Rule]) -> Result<(), Invalid> {
    let mut seen: HashSet<&str> = others.iter().map(|r| r.id.as_str()).collect();
    for (i, r) in rules.iter().enumerate() {
        if r.id.trim().is_empty() {
            return Err(Invalid::NoId(i + 1));
        }
        if !seen.insert(r.id.as_str()) {
            return Err(Invalid::IdDup(r.id.clone()));
        }
        validate_rule(r)?;
    }
    Ok(())
}

/// One rule.
pub fn validate_rule(r: &Rule) -> Result<(), Invalid> {
    if r.pattern.trim().is_empty() {
        return Err(Invalid::Empty(r.id.clone()));
    }
    if r.scope.is_empty() {
        return Err(Invalid::Scope(r.id.clone()));
    }
    match r.kind {
        Kind::Regex => {
            for p in std::iter::once(&r.pattern).chain(r.also.iter()) {
                Regex::new(p).map_err(|e| Invalid::Regex {
                    id: r.id.clone(),
                    error: e.to_string(),
                })?;
            }
        }
        Kind::Field => {
            for alt in r.pattern.split('|') {
                if !is_sig_glob(alt.trim()) {
                    return Err(Invalid::Sig {
                        id: r.id.clone(),
                        text: alt.trim().to_string(),
                    });
                }
            }
        }
        Kind::Edid | Kind::Function | Kind::Key => {}
    }
    Ok(())
}

/// `AAAA:BBBB`, letters, digits, `_` or `*`.
fn is_sig_glob(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 4
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '*')
    };
    match s.split_once(':') {
        Some((a, b)) => ok(a) && ok(b),
        None => false,
    }
}

/// `pattern` with `*` wildcards against `text`; alternatives with `|`.
fn glob_any(pattern: &str, text: &str, fold: bool) -> bool {
    pattern.split('|').any(|p| {
        let p = p.trim();
        if fold {
            crate::textfmt::glob_match(&p.to_lowercase(), &text.to_lowercase())
        } else {
            crate::textfmt::glob_match(p, text)
        }
    })
}

/// A path pattern against the end of a path.
fn glob_tail(pattern: &str, path: &str) -> bool {
    let p = pattern.replace('\\', "/").to_lowercase();
    let t = path.replace('\\', "/").to_lowercase();
    crate::textfmt::glob_match(&p, &t) || crate::textfmt::glob_match(&format!("*/{p}"), &t)
}

// ------------------------------------------------------------ the files

/// Where the file is: shipped and the user's copy.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub shipped: PathBuf,
    pub user: Option<PathBuf>,
}

/// Which file was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    User,
    Shipped,
}

impl Dirs {
    pub fn installed() -> Dirs {
        let p = crate::config::paths::get();
        Dirs {
            shipped: crate::config::assets_data_dir().join(DIR),
            user: Some(p.data_dir.join(DIR)),
        }
    }

    pub fn user_path(&self) -> Option<PathBuf> {
        self.user.as_ref().map(|u| u.join(FILE))
    }

    pub fn shipped_path(&self) -> PathBuf {
        self.shipped.join(FILE)
    }

    /// The shipped file, else the embedded copy.
    pub fn shipped(&self) -> RulesFile {
        std::fs::read_to_string(self.shipped_path())
            .ok()
            .and_then(|t| RulesFile::parse(&t).ok())
            .unwrap_or_else(RulesFile::embedded)
    }

    /// The user's copy when present and valid, else the shipped file; the
    /// error of an invalid copy comes back with the shipped one.
    pub fn load(&self) -> (RulesFile, Source, Option<Invalid>) {
        if let Some(p) = self.user_path()
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            match RulesFile::parse(&text) {
                Ok(f) => return (f, Source::User, None),
                Err(e) => {
                    tracing::warn!("{}: {e:?}", p.display());
                    return (self.shipped(), Source::Shipped, Some(e));
                }
            }
        }
        (self.shipped(), Source::Shipped, None)
    }

    /// Write the user's copy (validated first: an invalid file is never
    /// written).
    pub fn save(&self, f: &RulesFile) -> Result<PathBuf, Invalid> {
        f.validate()?;
        let p = self
            .user_path()
            .ok_or_else(|| Invalid::Json("no user folder".into()))?;
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        crate::config::paths::write_atomic(&p, f.to_json().as_bytes())
            .map_err(|e| Invalid::Json(e.to_string()))?;
        Ok(p)
    }

    /// Remove the user's copy.
    pub fn restore(&self) -> std::io::Result<()> {
        match self.user_path() {
            Some(p) if p.is_file() => std::fs::remove_file(p),
            _ => Ok(()),
        }
    }

    /// D-73: a user's copy of the former `data/scripts/papyrus_rules.json`
    /// is merged once into the user's copy of the exclusions (its patterns
    /// as script rules, its functions added), then renamed `.bak`. Returns
    /// the number of patterns and functions taken.
    pub fn migrate_papyrus(&self) -> Option<usize> {
        let user = self.user.as_ref()?;
        let old = user
            .parent()?
            .join("scripts")
            .join(crate::script_rules::FILE);
        let text = std::fs::read_to_string(&old).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
        let (mut f, _, _) = self.load();
        let mut n = 0;
        if let Some(list) = parsed.get("lockPatterns").and_then(|v| v.as_array()) {
            for p in list {
                let (Some(id), Some(re)) = (
                    p.get("id").and_then(|v| v.as_str()),
                    p.get("regex").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                if Regex::new(re).is_err() {
                    continue;
                }
                match f
                    .rules
                    .iter_mut()
                    .find(|r| r.id == id && r.scope.contains(&Scope::Script))
                {
                    Some(r) if r.pattern == re => {}
                    Some(r) => {
                        r.pattern = re.to_string();
                        n += 1;
                    }
                    None => {
                        let mut id = id.to_string();
                        while f.find(&id).is_some() {
                            id.push('+');
                        }
                        f.rules
                            .push(Rule::new(&id, Kind::Regex, re, vec![Scope::Script]));
                        n += 1;
                    }
                }
            }
        }
        if let Some(list) = parsed.get("lockFunctions").and_then(|v| v.as_array()) {
            for v in list.iter().filter_map(|v| v.as_str()) {
                if !f.functions.iter().any(|x| x == v) {
                    f.functions.push(v.to_string());
                    n += 1;
                }
            }
        }
        self.save(&f).ok()?;
        let _ = std::fs::rename(&old, old.with_extension("json.bak"));
        Some(n)
    }
}

// ------------------------------------------------------------ the lines

/// What a line is, for the scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A field of a record.
    Record,
    /// A line of a text file (C-05).
    File,
    /// A string of a script: `.pex`, VMAD, SCTX.
    Script,
}

pub fn line_kind(e: &TmEntry) -> LineKind {
    if e.field_sig == crate::vmad_lock::FIELD
        || e.field_sig == "SCTX"
        || (e.rec_sig == crate::scripts::REC && e.field_sig == crate::scripts::FIELD)
    {
        LineKind::Script
    } else if crate::textfmt::is_text_field(&e.field_sig) {
        LineKind::File
    } else {
        LineKind::Record
    }
}

/// Whether the rules may lock a line: nobody decided it (R-B18-04).
pub fn is_subject(e: &TmEntry) -> bool {
    !e.unused && e.trans.is_empty() && e.prov.is_empty() && status::derive(e) == Status::ToTranslate
}

/// The rule of a line's lock, `None` when no rule holds it (the file lock
/// of VMAD is not a rule).
pub fn lock_rule(e: &TmEntry) -> Option<&str> {
    crate::vmad_lock::lock_rule(&e.prov).filter(|r| *r != crate::vmad_lock::RULE_OFF)
}

/// The deductions of a document (R-B18-02), built at its opening.
#[derive(Debug, Clone, Default)]
pub struct Deduced {
    /// The EDIDs of the plugin, in lower case.
    pub edids: HashSet<String>,
    /// The keys of the mod's interface files (`$Key`).
    pub keys: HashSet<String>,
    /// The names of the mod's files, in lower case.
    pub files: HashSet<String>,
}

/// At most this many entries of the mod's folder are listed.
const FILES_MAX: usize = 50_000;

impl Deduced {
    /// The EDIDs of the entries, the keys of the interface files next to
    /// `plugin`, the files of its folder (not the game's Data folder).
    pub fn of(entries: &[TmEntry], plugin: Option<&Path>, interface: &[PathBuf]) -> Deduced {
        let mut d = Deduced::default();
        for e in entries {
            if !e.edid.is_empty() && line_kind(e) == LineKind::Record {
                d.edids.insert(e.edid.to_lowercase());
            }
        }
        for p in interface {
            if let Ok(data) = std::fs::read(p) {
                let (text, _) = crate::textfmt::decode(&data, "auto");
                for line in text.lines() {
                    if let Some((k, _)) = line.split_once('\t')
                        && k.starts_with('$')
                    {
                        d.keys.insert(k.trim().to_string());
                    }
                }
            }
        }
        if let Some(dir) = plugin.and_then(Path::parent)
            && !dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("data"))
        {
            let mut stack = vec![(dir.to_path_buf(), 0)];
            let mut seen = 0usize;
            while let Some((p, depth)) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&p) else {
                    continue;
                };
                for ent in rd.flatten() {
                    seen += 1;
                    if seen > FILES_MAX {
                        return d;
                    }
                    let path = ent.path();
                    if path.is_dir() {
                        if depth < 6 {
                            stack.push((path, depth + 1));
                        }
                    } else if let Some(n) = path.file_name().and_then(|n| n.to_str())
                        && n.contains('.')
                    {
                        d.files.insert(n.to_lowercase());
                    }
                }
            }
        }
        d
    }
}

/// The context of a document.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    pub game: &'a str,
    /// The document's path, for the `files` of the key rules.
    pub path: &'a str,
    pub deduced: &'a Deduced,
    /// The rules turned off for this document.
    pub off: &'a HashSet<String>,
    /// The facts of a scripts document (C-06).
    pub facts: Option<&'a crate::scripts::Facts>,
}

enum Matcher {
    Regex { also: Option<Regex> },
    Field,
    Edid,
    Key { files: Vec<String> },
}

struct CRule {
    id: String,
    pattern: String,
    scope: Vec<Scope>,
    matcher: Matcher,
}

/// The rules of a document, compiled once (R-B18-08): the expressions of
/// each scope in a [`RegexSet`].
pub struct Compiled {
    rules: Vec<CRule>,
    text: (RegexSet, Vec<usize>),
    script: (RegexSet, Vec<usize>),
    functions: Vec<String>,
    edid_script: bool,
    edid_file: bool,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("rules", &self.rules.len())
            .finish()
    }
}

impl Compiled {
    /// The enabled rules of `file` and `local` (the learned ones of the
    /// mod) that fit `game`. An expression that does not compile is left
    /// out (the file is validated before it is written).
    pub fn new(file: &RulesFile, local: &[Rule], game: &str) -> Compiled {
        let mut rules = Vec::new();
        let (mut tp, mut ti, mut sp, mut si) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut functions = file.functions.clone();
        for r in file.rules.iter().chain(local) {
            if !r.enabled || !r.fits(game) {
                continue;
            }
            let matcher = match r.kind {
                Kind::Regex => {
                    if Regex::new(&r.pattern).is_err() {
                        continue;
                    }
                    let also = match &r.also {
                        Some(a) => match Regex::new(a) {
                            Ok(re) => Some(re),
                            Err(_) => continue,
                        },
                        None => None,
                    };
                    let k = rules.len();
                    if r.scope.contains(&Scope::Text) {
                        tp.push(r.pattern.clone());
                        ti.push(k);
                    }
                    if r.scope.contains(&Scope::Script) {
                        sp.push(r.pattern.clone());
                        si.push(k);
                    }
                    Matcher::Regex { also }
                }
                Kind::Field => Matcher::Field,
                Kind::Edid => Matcher::Edid,
                Kind::Key => Matcher::Key {
                    files: r.files.clone(),
                },
                Kind::Function => {
                    functions.push(r.pattern.clone());
                    continue;
                }
            };
            rules.push(CRule {
                id: r.id.clone(),
                pattern: r.pattern.clone(),
                scope: r.scope.clone(),
                matcher,
            });
        }
        let set = |p: &[String]| RegexSet::new(p).unwrap_or_else(|_| RegexSet::empty());
        Compiled {
            text: (set(&tp), ti),
            script: (set(&sp), si),
            rules,
            functions,
            edid_script: file.deduce.edid_scopes.iter().any(|s| s == "script"),
            edid_file: file.deduce.edid_scopes.iter().any(|s| s == "file"),
        }
    }

    /// Whether the string arguments of `function` are identifiers (C-06).
    pub fn locks_function(&self, function: &str) -> bool {
        self.functions.iter().any(|f| match f.strip_suffix('*') {
            Some(prefix) => function.starts_with(prefix),
            None => f.eq_ignore_ascii_case(function),
        })
    }

    /// The ids of the rules that match a line, in the order of the file:
    /// the facts of a script first (`ident`, `function:<call>`), then the
    /// rules, then the deductions. The rules off for the document are left
    /// out.
    pub fn matches(&self, e: &TmEntry, ctx: &Ctx) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(e, ctx, &mut |id| {
            out.push(id.to_string());
            true
        });
        out
    }

    /// The first rule that matches a line.
    pub fn lock_of(&self, e: &TmEntry, ctx: &Ctx) -> Option<String> {
        let mut first = None;
        self.walk(e, ctx, &mut |id| {
            first = Some(id.to_string());
            false
        });
        first
    }

    /// Call `f` with each matching rule id until it returns false.
    fn walk(&self, e: &TmEntry, ctx: &Ctx, f: &mut dyn FnMut(&str) -> bool) {
        let kind = line_kind(e);
        let on = |id: &str| !ctx.off.contains(id);
        // The facts of a scripts document (C-06).
        if kind == LineKind::Script
            && let Some(fact) = ctx.facts.and_then(|fs| fs.get(&(e.edid.clone(), e.index)))
        {
            if fact.0 && on("ident") && !f("ident") {
                return;
            }
            for c in &fact.1 {
                let bare = c.rsplit('.').next().unwrap_or(c);
                if (self.locks_function(c) || self.locks_function(bare))
                    && on("functions")
                    && !f(&format!("function:{c}"))
                {
                    return;
                }
            }
        }
        let text = e.source.as_str();
        let (set, idx) = match kind {
            LineKind::Script => &self.script,
            _ => &self.text,
        };
        let hits: HashSet<usize> = set.matches(text).into_iter().map(|i| idx[i]).collect();
        let sig = format!("{}:{}", e.rec_sig, e.field_sig);
        for (k, r) in self.rules.iter().enumerate() {
            if !on(&r.id) {
                continue;
            }
            let ok = match &r.matcher {
                Matcher::Regex { also } => {
                    hits.contains(&k) && also.as_ref().is_none_or(|a| a.is_match(text))
                }
                Matcher::Field => kind != LineKind::Script && glob_any(&r.pattern, &sig, false),
                Matcher::Edid => {
                    kind == LineKind::Record
                        && !e.edid.is_empty()
                        && glob_any(&r.pattern, &e.edid, true)
                }
                Matcher::Key { files } => {
                    kind == LineKind::File
                        && glob_any(&r.pattern, &e.edid, false)
                        && (files.is_empty() || files.iter().any(|g| glob_tail(g, ctx.path)))
                }
            };
            let _ = &r.scope;
            if ok && !f(&r.id) {
                return;
            }
        }
        // The deductions (R-B18-02).
        let d = ctx.deduced;
        let edid_ok = match kind {
            LineKind::Script => self.edid_script,
            LineKind::File => self.edid_file,
            LineKind::Record => false,
        };
        if edid_ok
            && on(DEDUCED_EDID)
            && d.edids.contains(&text.trim().to_lowercase())
            && !f(DEDUCED_EDID)
        {
            return;
        }
        if on(DEDUCED_IFKEY) && d.keys.contains(text.trim()) && !f(DEDUCED_IFKEY) {
            return;
        }
        if on(DEDUCED_FILE) && !d.files.is_empty() {
            let t = text.trim().replace('\\', "/").to_lowercase();
            let name = t.rsplit('/').next().unwrap_or(&t);
            if !name.contains(' ') && d.files.contains(name) {
                f(DEDUCED_FILE);
            }
        }
    }
}

/// Lift the rule locks of the lines without translation before the
/// dictionaries fill them (the file lock of VMAD is the switch's).
pub fn release(entries: &mut [TmEntry]) {
    for e in entries.iter_mut() {
        if e.trans.is_empty() && lock_rule(e).is_some() && !crate::vmad_lock::is_file_locked(e) {
            status::set(e, Status::ToTranslate);
            e.prov.clear();
        }
    }
}

/// Lay the locks after the dictionaries filled the lines: each line
/// nobody decided that a rule matches is Ignoré with `lock:<id>`.
/// Returns the number of lines locked.
pub fn apply(entries: &mut [TmEntry], c: &Compiled, ctx: &Ctx) -> usize {
    let mut n = 0;
    for e in entries.iter_mut() {
        if !is_subject(e) {
            continue;
        }
        if let Some(id) = c.lock_of(e, ctx) {
            status::set(e, Status::Ignored);
            e.prov = format!("{LOCK}{id}");
            n += 1;
        }
    }
    n
}

/// Lift the locks of the rules in `ids` (a rule turned off or removed):
/// the lines become À traduire for the dictionaries. Returns their indexes.
pub fn unlock_rules(entries: &mut [TmEntry], ids: &HashSet<String>) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, e) in entries.iter_mut().enumerate() {
        if e.trans.is_empty()
            && let Some(r) = lock_rule(e)
            && (ids.contains(r) || (r.starts_with("function:") && ids.contains("functions")))
            && !crate::vmad_lock::is_file_locked(e)
        {
            status::set(e, Status::ToTranslate);
            e.prov.clear();
            out.push(i);
        }
    }
    out
}

/// The figures of the Règles actives tab: per rule, the lines it matches
/// among the locked lines and an example; the lines locked; the lines a
/// rule matches that a person unlocked (« à la main »).
#[derive(Debug, Clone, Default)]
pub struct Counts {
    pub per_rule: HashMap<String, (usize, String)>,
    pub locked: usize,
    pub hand: usize,
}

/// Count a document's locks (with the rules off counted as if on, so the
/// tab shows what turning one back on would do).
pub fn counts(entries: &[TmEntry], c: &Compiled, ctx: &Ctx) -> Counts {
    let none = HashSet::new();
    let all = Ctx { off: &none, ..*ctx };
    let mut out = Counts::default();
    for e in entries {
        if e.unused || !e.trans.is_empty() && lock_rule(e).is_none() {
            continue;
        }
        let locked = lock_rule(e);
        if locked.is_some() {
            out.locked += 1;
        }
        let typed = e.prov == crate::provenance::TYPED;
        if locked.is_none() && !typed && !is_subject(e) {
            continue;
        }
        let ids = c.matches(e, &all);
        if ids.is_empty() {
            if let Some(r) = locked {
                let r = if r.starts_with("function:") {
                    "functions"
                } else {
                    r
                };
                let slot = out.per_rule.entry(r.to_string()).or_default();
                slot.0 += 1;
                if slot.1.is_empty() {
                    slot.1 = e.source.clone();
                }
            }
            continue;
        }
        if typed {
            out.hand += 1;
            continue;
        }
        for id in ids {
            let id = if id.starts_with("function:") {
                "functions".to_string()
            } else {
                id
            };
            let slot = out.per_rule.entry(id).or_default();
            slot.0 += 1;
            if slot.1.is_empty() {
                slot.1 = e.source.clone();
            }
        }
    }
    out
}

// ------------------------------------------------------ the learned rules

/// What a line set Ignoré by hand may be generalized to (R-B18-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    TextLocal,
    PrefixLocal,
    PrefixGlobal,
    FieldLocal,
    FieldGlobal,
    None,
}

/// The proposal made after a line was set Ignoré by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub line: usize,
    /// `REC:FLD`, or the key of a text-file line.
    pub sig: String,
    pub text: String,
    /// Other lines of the same text that the rule would lock.
    pub same_text: usize,
    /// The prefix chosen (at first the longest shared by more lines than
    /// the text), and its lines.
    pub prefix: Option<(String, usize)>,
    /// Every prefix worth proposing, longest first, each shared by more
    /// lines than the longer one.
    pub prefixes: Vec<(String, usize)>,
    /// Other lines of the same field (key) the rule would lock.
    pub same_field: usize,
    /// The field is shown in game: no generalization by field.
    pub displayed: bool,
    pub kind: LineKind,
}

impl Proposal {
    /// Whether the proposal is worth a dialog: two other lines share the
    /// field or the text (R-B18-03); a displayed field still opens it, its
    /// choices greyed, for the text and the prefix.
    pub fn worth(&self) -> bool {
        self.same_text >= 2 || self.same_field >= 2
    }
}

fn sig_of(e: &TmEntry) -> String {
    match line_kind(e) {
        LineKind::File => e.edid.clone(),
        _ => format!("{}:{}", e.rec_sig, e.field_sig),
    }
}

/// The candidate prefixes of a text: its first words, three characters
/// at least, the text itself left out.
fn prefixes(text: &str) -> Vec<&str> {
    let t = text.trim();
    let mut out = Vec::new();
    for (i, c) in t.char_indices() {
        if (c.is_whitespace() || (!c.is_alphanumeric() && i > 0))
            && t[..i].trim_end().chars().count() >= 3
        {
            let p = t[..i].trim_end();
            if out.last() != Some(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// The proposal for line `idx` (just set Ignoré by hand).
pub fn propose(entries: &[TmEntry], idx: usize, file: &RulesFile) -> Option<Proposal> {
    let e = entries.get(idx)?;
    let kind = line_kind(e);
    let sig = sig_of(e);
    let text = e.source.trim();
    if text.is_empty() {
        return None;
    }
    let others = || {
        entries
            .iter()
            .enumerate()
            .filter(move |(i, x)| *i != idx && is_subject(x) && line_kind(x) == kind)
            .map(|(_, x)| x)
    };
    let same_text = others().filter(|x| x.source.trim() == text).count();
    let same_field = others().filter(|x| sig_of(x) == sig).count();
    let mut found: Vec<(String, usize)> = Vec::new();
    for p in prefixes(text).into_iter().rev() {
        let n = others()
            .filter(|x| x.source.trim_start().starts_with(p))
            .count();
        if n > same_text && n >= 2 && found.last().is_none_or(|(_, m)| n > *m) {
            found.push((p.to_string(), n));
        }
    }
    let prefix = found.first().cloned();
    let displayed = kind == LineKind::Record
        && file
            .deduce
            .displayed
            .iter()
            .any(|g| glob_any(g, &sig, false));
    Some(Proposal {
        line: idx,
        sig,
        text: text.to_string(),
        same_text,
        prefix,
        prefixes: found,
        same_field,
        displayed,
        kind,
    })
}

/// An id not used by `taken`.
pub fn free_id(base: &str, taken: &[&Rule]) -> String {
    let slug: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "rule".to_string()
    } else {
        slug
    };
    let mut id = format!("learned-{slug}");
    let mut k = 2;
    while taken.iter().any(|r| r.id == id) {
        id = format!("learned-{slug}-{k}");
        k += 1;
    }
    id
}

/// The rule a choice makes, learned today from `from`.
pub fn learned_rule(
    p: &Proposal,
    choice: &Choice,
    from: &str,
    date: &str,
    game: &str,
    taken: &[&Rule],
) -> Option<Rule> {
    let scope = vec![if p.kind == LineKind::Script {
        Scope::Script
    } else {
        Scope::Text
    }];
    let (kind, pattern, base, global) = match choice {
        Choice::TextLocal => (
            Kind::Regex,
            format!("^{}$", regex::escape(&p.text)),
            p.text.as_str(),
            false,
        ),
        Choice::PrefixLocal | Choice::PrefixGlobal => {
            let (pre, _) = p.prefix.as_ref()?;
            (
                Kind::Regex,
                format!("^{}", regex::escape(pre)),
                pre.as_str(),
                *choice == Choice::PrefixGlobal,
            )
        }
        Choice::FieldLocal | Choice::FieldGlobal => {
            if p.displayed {
                return None;
            }
            let kind = if p.kind == LineKind::File {
                Kind::Key
            } else {
                Kind::Field
            };
            (
                kind,
                p.sig.clone(),
                p.sig.as_str(),
                *choice == Choice::FieldGlobal,
            )
        }
        Choice::None => return None,
    };
    let mut r = Rule::new(&free_id(base, taken), kind, &pattern, scope);
    r.origin = Origin::Learned;
    r.learned = Some(Learned {
        date: date.to_string(),
        from: from.to_string(),
    });
    if global && matches!(choice, Choice::FieldGlobal) {
        r.game = game.to_string();
    }
    Some(r)
}

/// Whether a choice makes a global rule.
pub fn is_global(choice: &Choice) -> bool {
    matches!(choice, Choice::PrefixGlobal | Choice::FieldGlobal)
}

// ------------------------------------------------------------ suggestions

/// A rule the application suggests (R-B18-07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggestion {
    /// A field the game's base never translates: `(sig, rows)`.
    FieldBase(String, usize),
    /// Lines Ignoré by hand that share a field or a pattern: the rule and
    /// the number of lines.
    Group(Rule, usize),
}

/// The fields of the document that the game's dictionaries never
/// translate: more than fifty rows with source = translation, none other.
pub fn suggest_fields<'a>(
    entries: &[TmEntry],
    bases: impl IntoIterator<Item = &'a crate::basedb::BaseDb>,
) -> Vec<Suggestion> {
    let sigs: HashSet<(String, String)> = entries
        .iter()
        .filter(|e| line_kind(e) == LineKind::Record)
        .map(|e| (e.rec_sig.clone(), e.field_sig.clone()))
        .collect();
    let mut stats: HashMap<(String, String), (usize, usize)> = HashMap::new();
    for db in bases {
        db.for_each_pair(|rec, field, src, t, rows| {
            if !rec.is_empty() && sigs.contains(&(rec.to_string(), field.to_string())) {
                let s = stats
                    .entry((rec.to_string(), field.to_string()))
                    .or_default();
                if src.trim() == t.trim() {
                    s.0 += rows as usize;
                } else {
                    s.1 += rows as usize;
                }
            }
            true
        });
    }
    let mut out: Vec<Suggestion> = stats
        .into_iter()
        .filter(|(_, (same, diff))| *same > 50 && *diff == 0)
        .map(|((r, f), (same, _))| Suggestion::FieldBase(format!("{r}:{f}"), same))
        .collect();
    out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    out
}

/// The lines Ignoré by hand (not by a rule), grouped by field and by a
/// simple pattern — the first word, the `$` form, the extension; a group
/// of ten lines or more is suggested.
pub fn suggest_groups(entries: &[TmEntry], taken: &[&Rule]) -> Vec<Suggestion> {
    let hand: Vec<&TmEntry> = entries
        .iter()
        .filter(|e| {
            !e.unused
                && status::derive(e) == Status::Ignored
                && lock_rule(e).is_none()
                && !crate::vmad_lock::is_file_locked(e)
        })
        .collect();
    let mut by_field: HashMap<String, usize> = HashMap::new();
    let mut by_prefix: HashMap<String, usize> = HashMap::new();
    let mut by_ext: HashMap<String, usize> = HashMap::new();
    for e in &hand {
        *by_field.entry(sig_of(e)).or_default() += 1;
        if let Some(w) = e.source.split_whitespace().next()
            && w.chars().count() >= 3
            && e.source.trim() != w
        {
            *by_prefix.entry(w.to_string()).or_default() += 1;
        }
        if let Some((_, ext)) = e.source.trim().rsplit_once('.')
            && (2..=4).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
        {
            *by_ext.entry(ext.to_lowercase()).or_default() += 1;
        }
    }
    let mut out = Vec::new();
    let add = |kind: Kind, pattern: String, base: &str, n: usize, out: &mut Vec<Suggestion>| {
        let mut r = Rule::new(&free_id(base, taken), kind, &pattern, vec![Scope::Text]);
        r.origin = Origin::User;
        out.push(Suggestion::Group(r, n));
    };
    let mut keys: Vec<_> = by_field.into_iter().filter(|(_, n)| *n >= 10).collect();
    keys.sort();
    for (sig, n) in keys {
        let kind = if sig.contains(':') {
            Kind::Field
        } else {
            Kind::Key
        };
        add(kind, sig.clone(), &sig, n, &mut out);
    }
    let mut keys: Vec<_> = by_prefix.into_iter().filter(|(_, n)| *n >= 10).collect();
    keys.sort();
    for (w, n) in keys {
        add(
            Kind::Regex,
            format!("^{}\\s", regex::escape(&w)),
            &w,
            n,
            &mut out,
        );
    }
    let mut keys: Vec<_> = by_ext.into_iter().filter(|(_, n)| *n >= 10).collect();
    keys.sort();
    for (x, n) in keys {
        add(
            Kind::Regex,
            format!("(?i)\\.{}$", regex::escape(&x)),
            &x,
            n,
            &mut out,
        );
    }
    out
}

/// Everything a document needs to lay its locks, owned (it is built
/// before the document's lines are borrowed for the operation).
pub struct Plan {
    pub compiled: Compiled,
    pub deduced: std::sync::Arc<Deduced>,
    pub off: HashSet<String>,
    pub game: String,
    pub path: String,
    pub facts: Option<std::sync::Arc<crate::scripts::Facts>>,
}

impl Plan {
    pub fn ctx(&self) -> Ctx<'_> {
        Ctx {
            game: &self.game,
            path: &self.path,
            deduced: &self.deduced,
            off: &self.off,
            facts: self.facts.as_deref(),
        }
    }

    /// [`release`].
    pub fn release(&self, entries: &mut [TmEntry]) {
        release(entries);
    }

    /// [`apply`]; returns the number of lines locked.
    pub fn lay(&self, entries: &mut [TmEntry]) -> usize {
        apply(entries, &self.compiled, &self.ctx())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(rec: &str, field: &str, src: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            source: src.into(),
            edid: "Ed".into(),
            ..Default::default()
        }
    }

    fn lock(c: &Compiled, e: &TmEntry) -> Option<String> {
        let d = Deduced::default();
        let off = HashSet::new();
        c.lock_of(
            e,
            &Ctx {
                game: "Fallout4",
                path: "/x/MCM/Config/SS2/config.json",
                deduced: &d,
                off: &off,
                facts: None,
            },
        )
    }

    #[test]
    fn shipped_file() {
        let f = RulesFile::embedded();
        let c = Compiled::new(&f, &[], "SkyrimSE");
        let t = |s: &str| lock(&c, &line("WEAP", "FULL", s));
        let s = |s: &str| lock(&c, &line("QUST", "VMAD", s));
        // The scripts keep their former rules (T-B18-07).
        assert_eq!(s("$SKI_Pages").as_deref(), Some("key"));
        assert_eq!(s("Data/Interface/menu.swf").as_deref(), Some("path"));
        assert_eq!(s("weaponSwing").as_deref(), Some("word"));
        assert_eq!(s("Skip the introduction?"), None);
        // The texts of the records, with the revised rules (A.8).
        assert_eq!(
            t("xCACO_FireGrenadeEffect01Impact").as_deref(),
            Some("ident-text")
        );
        assert_eq!(t("ITMPotionUse").as_deref(), Some("ident-text"));
        assert_eq!(t("GhoulSynthGen2Eyes").as_deref(), Some("ident-text"));
        assert_eq!(t("MrHandy"), None);
        assert_eq!(t("PowerArmor"), None);
        assert_eq!(t("SPECIAL"), None);
        assert_eq!(t("Themes/Tags"), None);
        assert_eq!(t("</font>").as_deref(), Some("tag-only"));
        assert_eq!(t("<Unknown>"), None);
        assert_eq!(t("<Building Empty>"), None);
        assert_eq!(t("<Token.Title=Settler1> : <Token.Name=Settler1>"), None);
        assert_eq!(t("<Alias=Player> <br>").as_deref(), Some("tag-only"));
        assert_eq!(t("+6 Food"), None);
        assert_eq!(t("45 Mag"), None);
        assert_eq!(t("100").as_deref(), Some("noletter"));
        assert_eq!(t("0001A332").as_deref(), Some("hex"));
        assert_eq!(t("DECADE"), None);
        assert_eq!(t("Textures\\Armor\\iron.dds").as_deref(), Some("path-text"));
        assert_eq!(t("meshes/a/b.nif").as_deref(), Some("path-text"));
        assert_eq!(t("https://www.nexusmods.com").as_deref(), Some("url"));
        assert_eq!(t("$Akaviri Samurai Armor").as_deref(), Some("key-text"));
        assert_eq!(t("$100").as_deref(), Some("noletter"));
        assert_eq!(t("Ash Yam"), None);
        assert_eq!(t("Help"), None);
        // MCM config.json: the technical keys, in that file only.
        let mut k = line("JSON", crate::textfmt::FIELD, "Sim Settlements");
        k.edid = "sourceType".into();
        assert_eq!(lock(&c, &k).as_deref(), Some("mcm-config"));
        k.edid = "text".into();
        assert_eq!(lock(&c, &k), None);
        k.edid = "QuestID".into();
        assert_eq!(lock(&c, &k).as_deref(), Some("json-ids"));
        assert!(c.locks_function("Debug.Trace"));
        assert!(!c.locks_function("Debug.Notification"));
    }

    #[test]
    fn validation() {
        let mut f = RulesFile::embedded();
        f.rules
            .push(Rule::new("bad", Kind::Regex, "(", vec![Scope::Text]));
        assert!(matches!(f.validate(), Err(Invalid::Regex { id, .. }) if id == "bad"));
        f.rules.pop();
        f.rules
            .push(Rule::new("key", Kind::Regex, "x", vec![Scope::Text]));
        assert_eq!(f.validate(), Err(Invalid::IdDup("key".into())));
        f.rules.pop();
        f.rules.push(Rule::new(
            "sig",
            Kind::Field,
            "WEAPON:FULL",
            vec![Scope::Field],
        ));
        assert!(matches!(f.validate(), Err(Invalid::Sig { .. })));
        f.rules.pop();
        f.rules.push(Rule::new("noscope", Kind::Regex, "x", vec![]));
        assert_eq!(f.validate(), Err(Invalid::Scope("noscope".into())));
    }

    #[test]
    fn prefixes_and_proposal() {
        assert_eq!(
            prefixes("zzz TEMPLATE - DO NOT USE"),
            vec![
                "zzz",
                "zzz TEMPLATE",
                "zzz TEMPLATE -",
                "zzz TEMPLATE - DO",
                "zzz TEMPLATE - DO NOT"
            ]
        );
        let mut es: Vec<TmEntry> = (0..12)
            .map(|_| line("ARMO", "FULL", "zzz TEMPLATE - DO NOT USE"))
            .collect();
        es.push(line("ARMO", "FULL", "zzz Test Blade"));
        es.push(line("ARMO", "FULL", "zzz Test Blade"));
        es.push(line("ARMO", "FULL", "Iron Helmet"));
        status::set(&mut es[0], Status::Ignored);
        let f = RulesFile::embedded();
        let p = propose(&es, 0, &f).unwrap();
        assert_eq!(p.same_text, 11);
        assert_eq!(p.prefix, Some(("zzz".into(), 13)));
        assert!(p.displayed);
        assert!(p.worth());
        let r = learned_rule(&p, &Choice::TextLocal, "Ed", "2026-10-28", "Fallout4", &[]).unwrap();
        assert_eq!(r.pattern, "^zzz TEMPLATE \\- DO NOT USE$");
        assert_eq!(r.id, "learned-zzz-template-do-not");
        assert!(learned_rule(&p, &Choice::FieldLocal, "Ed", "d", "g", &[]).is_none());
    }
}
