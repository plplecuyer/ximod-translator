//! Motifs à variables (B-16): a rule « %A% of %B% » → « %A% de %B% »
//! captures fragments of a short text, has each one translated — by the
//! glossary, the dictionaries of the stack, another pattern when it is
//! recursive, or the composition of the glossary's terms — and writes them
//! in the order of the target, with the agreement of the forms (the head
//! variable gives the gender), the articles, the elision and the
//! contractions of the target language at the junctions the pattern
//! creates (R-B16-01 to R-B16-06, decision D-78).
//!
//! The file is `assets/data/assist/patterns_<SRC>-<DST>.json`, with a
//! user's copy that wins; its implicit first pattern is the composition
//! (« motif 1 »); the patterns of the file are numbered from 2.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::compose::{self, Composition, capitalize, first_upper, inner};
use super::glossary::{DIR, Forms, Gender, Glossary, Piece};
use crate::instant::{Kind, tokens};

pub const FORMAT: u32 = 1;

fn sixty() -> usize {
    60
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// How a variable may be resolved (R-B16-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolver {
    /// A term of the glossary covering the whole capture, then the
    /// composition of its terms.
    Glossary,
    /// A term of the glossary that has forms.
    Adjective,
    /// The dictionaries of the stack (text only, then normalized form).
    Bases,
    /// The other patterns (recursive patterns only).
    Patterns,
    /// One pattern, by its number.
    Pattern(usize),
    /// The capture as it is, when each of its words starts with a capital
    /// (a proper name).
    Copy,
}

impl Resolver {
    /// The default order: glossary, adjective, bases, patterns.
    pub const DEFAULT: [Resolver; 4] = [
        Resolver::Glossary,
        Resolver::Adjective,
        Resolver::Bases,
        Resolver::Patterns,
    ];

    pub fn code(self) -> String {
        match self {
            Resolver::Glossary => "glossary".into(),
            Resolver::Adjective => "glossary:adjective".into(),
            Resolver::Bases => "bases".into(),
            Resolver::Patterns => "patterns".into(),
            Resolver::Pattern(n) => format!("pattern:{n}"),
            Resolver::Copy => "copy".into(),
        }
    }

    pub fn parse(s: &str) -> Option<Resolver> {
        Some(match s.trim() {
            "glossary" => Resolver::Glossary,
            "glossary:adjective" => Resolver::Adjective,
            "bases" => Resolver::Bases,
            "patterns" => Resolver::Patterns,
            "copy" => Resolver::Copy,
            other => Resolver::Pattern(other.strip_prefix("pattern:")?.parse().ok()?),
        })
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Resolver::Glossary => "patterns-resolver-glossary",
            Resolver::Adjective => "patterns-resolver-adjective",
            Resolver::Bases => "patterns-resolver-bases",
            Resolver::Patterns | Resolver::Pattern(_) => "patterns-resolver-patterns",
            Resolver::Copy => "patterns-resolver-copy",
        }
    }
}

impl Serialize for Resolver {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.code())
    }
}

impl<'de> Deserialize<'de> for Resolver {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Resolver::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("resolver {s}")))
    }
}

/// One pattern of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub src: String,
    pub dst: String,
    /// Longer texts (characters) are not tried.
    #[serde(default = "sixty", rename = "maxLen")]
    pub max_len: usize,
    /// Its variables may be resolved by other patterns.
    #[serde(default, skip_serializing_if = "is_false")]
    pub recursive: bool,
    /// The resolvers of a variable, in order (all by default).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, Vec<Resolver>>,
    /// The variable whose gender the pattern takes (the last of the
    /// target by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern {
            src: String::new(),
            dst: String::new(),
            max_len: 60,
            recursive: false,
            vars: BTreeMap::new(),
            head: None,
        }
    }
}

/// The resolution of the parent's variable of a derivation (B-21): the
/// capture → its text and whether `:low` may lower it.
pub(crate) type ParentFn<'a> = dyn FnMut(&str) -> Option<(String, bool)> + 'a;

/// A patterns file, format 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternsFile {
    pub format: u32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Composition>,
    /// « de » → « d' » before a vowel or a mute h.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub elision: BTreeMap<String, String>,
    /// The words of an aspirated h (no elision).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aspirated: Vec<String>,
    /// « de le » → « du ».
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub contractions: BTreeMap<String, String>,
    /// An article before a variable takes its gender: « le » → « la »,
    /// « les », nothing before a name without gender (`none`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub articles: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl Default for PatternsFile {
    fn default() -> Self {
        PatternsFile {
            format: FORMAT,
            source: String::new(),
            target: String::new(),
            composition: None,
            elision: BTreeMap::new(),
            aspirated: Vec::new(),
            contractions: BTreeMap::new(),
            articles: BTreeMap::new(),
            patterns: Vec::new(),
            rest: serde_json::Map::new(),
        }
    }
}

/// Which side of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Src,
    Dst,
}

/// Why a file is refused (R-B16-07). `n` is the number of the pattern
/// (from 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    Json(String),
    Format(u32),
    /// No literal and no variable.
    Empty(usize),
    /// A variable of one side missing in the other.
    VarMissing {
        n: usize,
        var: String,
        side: Side,
    },
    MaxLen(usize),
    /// Two patterns call each other (a pattern that calls itself: a = b).
    Cycle {
        a: usize,
        b: usize,
    },
    /// A resolver names a pattern that does not exist.
    BadRef {
        n: usize,
        target: usize,
    },
}

/// The variables of a side, in order, each once: (name, modifier).
pub fn variables(side: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for seg in segments(side) {
        if let Seg::Var(name, m) = seg
            && !out.iter().any(|(n, _)| *n == name)
        {
            out.push((name, m));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Lit(String),
    Var(String, Option<String>),
}

/// « %A% de %B:cap% » cut into literals and variables.
fn segments(s: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('%') else { break };
        let inside = &after[..j];
        let (name, m) = match inside.split_once(':') {
            Some((n, m)) => (n, Some(m.to_string())),
            None => (inside, None),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric()) {
            // Not a variable: the « % » is literal.
            out.push(Seg::Lit(rest[..i + 1].to_string()));
            rest = after;
            continue;
        }
        if i > 0 {
            out.push(Seg::Lit(rest[..i].to_string()));
        }
        out.push(Seg::Var(name.to_string(), m));
        rest = &after[j + 1..];
    }
    if !rest.is_empty() {
        out.push(Seg::Lit(rest.to_string()));
    }
    out
}

impl PatternsFile {
    pub fn parse(text: &str) -> Result<PatternsFile, Invalid> {
        let f: PatternsFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| Invalid::Json(e.to_string()))?;
        if f.format != FORMAT {
            return Err(Invalid::Format(f.format));
        }
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// R-B16-07: each variable on both sides, something to match, a
    /// length from 1 to 500, no cycle between patterns.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.format != FORMAT {
            return Err(Invalid::Format(self.format));
        }
        for (k, p) in self.patterns.iter().enumerate() {
            let n = k + 2;
            if p.src.trim().is_empty() {
                return Err(Invalid::Empty(n));
            }
            if !(1..=500).contains(&p.max_len) {
                return Err(Invalid::MaxLen(n));
            }
            let vs: Vec<String> = variables(&p.src).into_iter().map(|v| v.0).collect();
            let vd: Vec<String> = variables(&p.dst).into_iter().map(|v| v.0).collect();
            if let Some(v) = vs.iter().find(|v| !vd.contains(v)) {
                return Err(Invalid::VarMissing {
                    n,
                    var: v.clone(),
                    side: Side::Dst,
                });
            }
            if let Some(v) = vd.iter().find(|v| !vs.contains(v)) {
                return Err(Invalid::VarMissing {
                    n,
                    var: v.clone(),
                    side: Side::Src,
                });
            }
            for r in p.vars.values().flatten() {
                if let Resolver::Pattern(t) = r
                    && (*t < 2 || *t >= self.patterns.len() + 2)
                {
                    return Err(Invalid::BadRef { n, target: *t });
                }
            }
        }
        // The calls named by `pattern:<n>` resolvers: no cycle.
        let edges = |k: usize| -> Vec<usize> {
            let p = &self.patterns[k];
            if !p.recursive {
                return Vec::new();
            }
            p.vars
                .values()
                .flatten()
                .filter_map(|r| match r {
                    Resolver::Pattern(t) => Some(t - 2),
                    _ => None,
                })
                .collect()
        };
        for start in 0..self.patterns.len() {
            // Depth-first from each pattern: a path back to it is a cycle.
            let mut stack = vec![(start, 0usize)];
            let mut seen = vec![false; self.patterns.len()];
            while let Some((k, _)) = stack.pop() {
                for t in edges(k) {
                    if t == start {
                        return Err(Invalid::Cycle {
                            a: start + 2,
                            b: k + 2,
                        });
                    }
                    if !seen[t] {
                        seen[t] = true;
                        stack.push((t, 0));
                    }
                }
            }
        }
        Ok(())
    }
}

/// `patterns_ENG-FRA.json`.
pub fn file_name(pair: &str) -> String {
    format!("patterns_{pair}.json")
}

fn embedded(name: &str) -> Option<&'static str> {
    Some(match name {
        "patterns_ENG-FRA.json" => include_str!("../../assets/data/assist/patterns_ENG-FRA.json"),
        "patterns_ENG-ZHO.json" => include_str!("../../assets/data/assist/patterns_ENG-ZHO.json"),
        _ => return None,
    })
}

/// The shipped file of a pair (the file, else the embedded copy).
pub fn shipped_file(shipped: &Path, pair: &str) -> Option<PatternsFile> {
    let name = file_name(pair);
    std::fs::read_to_string(shipped.join(&name))
        .ok()
        .and_then(|t| PatternsFile::parse(&t).ok())
        .or_else(|| embedded(&name).and_then(|t| PatternsFile::parse(t).ok()))
}

/// The patterns file of a pair: the user's copy when present and valid,
/// else the shipped one; with whether it is the user's.
pub fn load_file(
    shipped: &Path,
    user: Option<&PathBuf>,
    pair: &str,
) -> Option<(PatternsFile, bool)> {
    let name = file_name(pair);
    if let Some(u) = user
        && let Ok(text) = std::fs::read_to_string(u.join(&name))
    {
        match PatternsFile::parse(&text).and_then(|f| f.validate().map(|_| f)) {
            Ok(f) => return Some((f, true)),
            Err(e) => tracing::warn!("{}: {e:?}", u.join(&name).display()),
        }
    }
    shipped_file(shipped, pair).map(|f| (f, false))
}

/// Write the user's copy of a pair, validated first.
pub fn save(user: &Path, pair: &str, f: &PatternsFile) -> Result<PathBuf, Invalid> {
    f.validate()?;
    let p = user.join(file_name(pair));
    let _ = std::fs::create_dir_all(user);
    crate::config::paths::write_atomic(&p, f.to_json().as_bytes())
        .map_err(|e| Invalid::Json(e.to_string()))?;
    Ok(p)
}

/// The user's folder of the assistants' files.
pub fn user_dir() -> PathBuf {
    crate::config::paths::get().data_dir.join(DIR)
}

// ----------------------------------------------------------------------
// Bases
// ----------------------------------------------------------------------

/// The dictionaries a variable may be looked up in (R-B16-05).
pub trait Bases {
    fn lookup(&self, text: &str) -> Option<String>;
}

/// No dictionary.
pub struct NoBases;

impl Bases for NoBases {
    fn lookup(&self, _: &str) -> Option<String> {
        None
    }
}

/// The dictionaries of a stack, in order: the text-only pair or the most
/// frequent translation of the text, then the normalized form (B-19).
pub struct StackBases(pub Vec<std::sync::Arc<crate::basedb::BaseDb>>);

impl Bases for StackBases {
    fn lookup(&self, text: &str) -> Option<String> {
        let text = text.trim();
        for db in &self.0 {
            if let Some((t, _)) = db.text_candidates(text).first() {
                return Some(t.trim().to_string());
            }
        }
        let form = crate::normalize::form(text)?;
        let e = crate::tm::TmEntry {
            source: text.to_string(),
            ..Default::default()
        };
        self.0
            .iter()
            .find_map(|db| db.lookup_normalized(&e, &form))
            .map(|h| h.trans.trim().to_string())
    }
}

// ----------------------------------------------------------------------
// Matching and resolution
// ----------------------------------------------------------------------

/// A token of a text with its place, the possessive « 's » apart.
#[derive(Debug, Clone)]
struct Tok {
    kind: Kind,
    key: String,
    start: usize,
    end: usize,
}

fn key_of(s: &str) -> String {
    s.to_lowercase().replace('’', "'")
}

/// The solid tokens of a text: « Duke's » gives « Duke », « ' », « s ».
fn solid(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut at = 0;
    for t in tokens(text) {
        let (start, end) = (at, at + t.text.len());
        at = end;
        if t.kind == Kind::Space {
            continue;
        }
        if t.kind == Kind::Word {
            for ap in ["'s", "’s"] {
                if let Some(stem) = t.text.strip_suffix(ap)
                    && !stem.is_empty()
                {
                    let s1 = start + stem.len();
                    out.push(Tok {
                        kind: Kind::Word,
                        key: key_of(stem),
                        start,
                        end: s1,
                    });
                    out.push(Tok {
                        kind: Kind::Punct,
                        key: "'".into(),
                        start: s1,
                        end: s1 + ap.len() - 1,
                    });
                    out.push(Tok {
                        kind: Kind::Word,
                        key: "s".into(),
                        start: end - 1,
                        end,
                    });
                    break;
                }
            }
            if out.last().is_some_and(|l: &Tok| l.end == end) {
                continue;
            }
        }
        out.push(Tok {
            kind: t.kind,
            key: key_of(&t.text),
            start,
            end,
        });
    }
    out
}

/// A pattern compiled: its source as literal keys and variables.
#[derive(Debug, Clone)]
enum Item {
    Lit(Vec<String>),
    Var(String),
}

#[derive(Debug, Clone)]
struct Compiled {
    items: Vec<Item>,
    dst: Vec<Seg>,
    head: Option<String>,
}

impl Compiled {
    fn new(p: &Pattern) -> Compiled {
        let mut items = Vec::new();
        for seg in segments(&p.src) {
            match seg {
                Seg::Lit(s) => {
                    let keys: Vec<String> = solid(&s).into_iter().map(|t| t.key).collect();
                    if !keys.is_empty() {
                        items.push(Item::Lit(keys));
                    }
                }
                Seg::Var(n, _) => items.push(Item::Var(n)),
            }
        }
        let dst = segments(&p.dst);
        let head = p.head.clone().or_else(|| {
            dst.iter().rev().find_map(|s| match s {
                Seg::Var(n, _) => Some(n.clone()),
                _ => None,
            })
        });
        Compiled { items, dst, head }
    }
}

/// The captures of the variables, as token ranges; the leftmost literal
/// wins (the shortest capture first).
fn match_items(
    items: &[Item],
    toks: &[Tok],
    i: usize,
    pos: usize,
    caps: &mut Vec<(String, usize, usize)>,
) -> bool {
    let Some(item) = items.get(i) else {
        return pos == toks.len();
    };
    match item {
        Item::Lit(keys) => {
            let n = keys.len();
            pos + n <= toks.len()
                && keys
                    .iter()
                    .zip(&toks[pos..pos + n])
                    .all(|(k, t)| *k == t.key)
                && match_items(items, toks, i + 1, pos + n, caps)
        }
        Item::Var(name) => {
            let last = i + 1 == items.len();
            let lens: Vec<usize> = if last {
                vec![toks.len().saturating_sub(pos)]
            } else {
                (1..=toks.len().saturating_sub(pos)).collect()
            };
            for len in lens {
                if len == 0 {
                    continue;
                }
                let span = &toks[pos..pos + len];
                if !span.iter().any(|t| t.kind == Kind::Word) {
                    continue;
                }
                caps.push((name.clone(), pos, pos + len));
                if match_items(items, toks, i + 1, pos + len, caps) {
                    return true;
                }
                caps.pop();
            }
            false
        }
    }
}

/// How a variable was resolved (the path of the essay).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    Glossary,
    Adjective,
    Bases,
    /// By the pattern of that number.
    Pattern(usize),
    /// By the composition of the glossary's terms.
    Compose,
    Copy,
    /// By the translated name of a parent record (B-21).
    Parent,
}

/// One variable of a pattern, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub var: String,
    pub capture: String,
    pub how: How,
    pub text: String,
}

/// A capture translated.
#[derive(Debug, Clone)]
struct Resolved {
    text: String,
    gender: Option<Gender>,
    forms: Option<Forms>,
    /// Lowered when placed inside the result (not a proper name).
    lowerable: bool,
    /// May take a small letter (`%A:low%`).
    low_ok: bool,
    how: How,
    /// The steps below, for a capture resolved by a pattern.
    steps: Vec<Step>,
}

/// A text translated by a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub text: String,
    /// The number of the pattern (from 2).
    pub n: usize,
    pub gender: Option<Gender>,
    pub steps: Vec<Step>,
}

/// The patterns of a pair, compiled, with the rules of the target language.
#[derive(Debug, Clone, Default)]
pub struct Patterns {
    pub file: PatternsFile,
    compiled: Vec<Compiled>,
}

const VOWELS: &str = "aeiouyàâäéèêëîïôöûüùœæ";

impl Patterns {
    pub fn new(file: PatternsFile) -> Patterns {
        let compiled = file.patterns.iter().map(Compiled::new).collect();
        Patterns { file, compiled }
    }

    pub fn len(&self) -> usize {
        self.compiled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.compiled.is_empty()
    }

    /// The first pattern that matches `text` and whose variables all
    /// resolve (R-B16-04).
    pub fn translate(
        &self,
        text: &str,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
    ) -> Option<Matched> {
        self.try_all(text, glossary, comp, bases, 0, None, None)
            .map(|(n, r)| Matched {
                text: r.text,
                n,
                gender: r.gender,
                steps: r.steps,
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn try_all(
        &self,
        text: &str,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
        depth: usize,
        exclude: Option<usize>,
        only: Option<usize>,
    ) -> Option<(usize, Resolved)> {
        let toks = solid(text);
        if toks.is_empty() {
            return None;
        }
        let len = text.trim().chars().count();
        for (k, c) in self.compiled.iter().enumerate() {
            if Some(k) == exclude || only.is_some_and(|o| o != k) {
                continue;
            }
            let p = &self.file.patterns[k];
            if len > p.max_len || c.items.is_empty() {
                continue;
            }
            let mut caps = Vec::new();
            if !match_items(&c.items, &toks, 0, 0, &mut caps) {
                continue;
            }
            if let Some(r) = self.apply(k, &caps, &toks, text, glossary, comp, bases, depth) {
                return Some((k + 2, r));
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &self,
        k: usize,
        caps: &[(String, usize, usize)],
        toks: &[Tok],
        text: &str,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
        depth: usize,
    ) -> Option<Resolved> {
        let p = &self.file.patterns[k];
        let c = &self.compiled[k];
        let mut r = self.build(
            p, c, caps, toks, text, glossary, comp, bases, depth, k, None, None,
        )?;
        r.how = How::Pattern(k + 2);
        Some(r)
    }

    /// A text derived from a parent (B-21): the source pattern `src`
    /// matched on `text`, the variable `parent_var` resolved by `parent`
    /// (its translation, and whether it may take a small letter), the
    /// others by their resolvers; `gender` is that of the noun the target
    /// writes. The rules of the target language apply as for a pattern.
    #[allow(clippy::too_many_arguments)]
    pub fn derive_with(
        &self,
        src: &str,
        dst: &str,
        vars: &BTreeMap<String, Vec<Resolver>>,
        text: &str,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
        parent_var: &str,
        parent: &mut dyn FnMut(&str) -> Option<(String, bool)>,
        gender: Option<Gender>,
    ) -> Option<(String, Vec<Step>)> {
        let p = Pattern {
            src: src.to_string(),
            dst: dst.to_string(),
            max_len: 500,
            recursive: true,
            vars: vars.clone(),
            head: None,
        };
        let c = Compiled::new(&p);
        let toks = solid(text);
        if toks.is_empty() || c.items.is_empty() {
            return None;
        }
        let mut caps = Vec::new();
        if !match_items(&c.items, &toks, 0, 0, &mut caps) {
            return None;
        }
        let r = self.build(
            &p,
            &c,
            &caps,
            &toks,
            text,
            glossary,
            comp,
            bases,
            0,
            usize::MAX,
            Some((parent_var, parent)),
            gender,
        )?;
        Some((r.text, r.steps))
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        p: &Pattern,
        c: &Compiled,
        caps: &[(String, usize, usize)],
        toks: &[Tok],
        text: &str,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
        depth: usize,
        k: usize,
        mut fixed: Option<(&str, &mut ParentFn<'_>)>,
        gender: Option<Gender>,
    ) -> Option<Resolved> {
        let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
        let mut steps = Vec::new();
        for (name, a, b) in caps {
            let capture = text[toks[*a].start..toks[*b - 1].end].to_string();
            if let Some((var, f)) = fixed.as_mut()
                && *var == name.as_str()
            {
                let (t, low_ok) = f(&capture)?;
                steps.push(Step {
                    var: name.clone(),
                    capture: capture.clone(),
                    how: How::Parent,
                    text: t.clone(),
                });
                resolved.insert(
                    name.clone(),
                    Resolved {
                        text: t,
                        gender: None,
                        forms: None,
                        lowerable: false,
                        low_ok,
                        how: How::Parent,
                        steps: Vec::new(),
                    },
                );
                continue;
            }
            let list: Vec<Resolver> = p
                .vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| Resolver::DEFAULT.to_vec());
            let r = self.resolve(
                &capture,
                &list,
                p.recursive,
                k,
                glossary,
                comp,
                bases,
                depth,
            )?;
            steps.push(Step {
                var: name.clone(),
                capture: capture.clone(),
                how: r.how.clone(),
                text: r.text.clone(),
            });
            steps.extend(r.steps.iter().cloned());
            resolved.insert(name.clone(), r);
        }
        let head_gender = gender.or_else(|| {
            c.head
                .as_ref()
                .and_then(|h| resolved.get(h))
                .and_then(|r| r.gender)
        });
        // The segments of the result: literals and the variables' texts.
        let mut out: Vec<(String, Option<&Resolved>)> = Vec::new();
        for seg in &c.dst {
            match seg {
                Seg::Lit(s) => out.push((s.clone(), None)),
                Seg::Var(name, m) => {
                    let r = resolved.get(name)?;
                    let mods: Vec<&str> = m
                        .as_deref()
                        .map(|m| m.split(':').collect())
                        .unwrap_or_default();
                    let forced = mods.iter().find_map(|m| match *m {
                        "m" => Some(Gender::M),
                        "f" => Some(Gender::F),
                        "mp" => Some(Gender::Mp),
                        "fp" => Some(Gender::Fp),
                        _ => None,
                    });
                    let mut t = match &r.forms {
                        Some(f) => {
                            let g = forced
                                .or(gender)
                                .or(if c.head.as_deref() == Some(name) {
                                    r.gender
                                } else {
                                    head_gender
                                })
                                .unwrap_or(Gender::M);
                            f.get(g).unwrap_or(&r.text).to_string()
                        }
                        None => r.text.clone(),
                    };
                    let at_start = out.iter().all(|(s, _)| s.trim().is_empty());
                    if !at_start && r.lowerable {
                        t = inner(&t, false);
                    }
                    if mods.contains(&"low") && r.low_ok {
                        t = inner(&t, false);
                    }
                    if mods.contains(&"cap") {
                        t = capitalize(&t);
                    }
                    out.push((t, Some(r)));
                }
            }
        }
        let mut s = self.junctions(&out);
        s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let first_lowerable = out
            .iter()
            .find(|(t, _)| !t.trim().is_empty())
            .is_none_or(|(_, r)| r.is_none_or(|r| r.lowerable));
        if first_upper(text) && first_lowerable {
            s = capitalize(&s);
        }
        if s.is_empty() {
            return None;
        }
        Some(Resolved {
            text: s,
            gender: head_gender,
            forms: None,
            lowerable: first_lowerable,
            low_ok: first_lowerable,
            how: How::Pattern(k.wrapping_add(2)),
            steps,
        })
    }

    /// Articles, elision and contractions where a literal of the pattern
    /// meets a variable (R-B16-03): the article takes the variable's gender
    /// (« le » → « la », nothing before a name without gender), then the
    /// elision before a vowel or a mute h (« de Aevar » → « d'Aevar »),
    /// else the contractions of the literal (« de le » → « du ») and of the
    /// junction (« de » + « le Duc » → « du Duc », « de » + « d'Épervine »
    /// → « d'Épervine »).
    fn junctions(&self, out: &[(String, Option<&Resolved>)]) -> String {
        let f = &self.file;
        let mut s = String::new();
        for (text, r) in out {
            let Some(r) = r else {
                s.push_str(text);
                continue;
            };
            let trimmed = s.trim_end();
            if trimmed.len() == s.len() || trimmed.is_empty() {
                // Glued to what precedes, or at the start: as it is.
                s.push_str(text);
                continue;
            }
            let mut words: Vec<String> = trimmed.split(' ').map(str::to_string).collect();
            let mut var_text = text.clone();
            let first_word =
                |t: &str| -> String { t.split(' ').next().unwrap_or("").to_lowercase() };
            if let Some(last) = words.last().cloned()
                && let Some(map) = f.articles.get(&last.to_lowercase())
            {
                let key = match r.gender {
                    Some(Gender::M) => "m",
                    Some(Gender::F) => "f",
                    Some(Gender::Mp) => "mp",
                    Some(Gender::Fp) => "fp",
                    None => "none",
                };
                if let Some(rep) = map.get(key) {
                    words.pop();
                    if !rep.is_empty() {
                        words.push(rep.clone());
                    }
                }
            }
            let fw = first_word(&var_text);
            let elides = |w: &str| -> Option<String> {
                let el = f.elision.get(&w.to_lowercase())?;
                let c = fw.chars().next()?;
                let bare: String = fw.chars().filter(|c| c.is_alphabetic()).collect();
                let mute_h = c == 'h' && !f.aspirated.iter().any(|a| a.to_lowercase() == bare);
                (VOWELS.contains(c) || mute_h).then(|| el.clone())
            };
            if let Some(last) = words.last().cloned()
                && let Some(el) = elides(&last)
            {
                words.pop();
                var_text = format!("{el}{var_text}");
            } else {
                if words.len() >= 2 {
                    let pair = format!(
                        "{} {}",
                        words[words.len() - 2].to_lowercase(),
                        words[words.len() - 1].to_lowercase()
                    );
                    if let Some(rep) = f.contractions.get(&pair) {
                        words.truncate(words.len() - 2);
                        words.push(rep.clone());
                    }
                }
                if let Some(last) = words.last().map(|w| w.to_lowercase()) {
                    for (k, rep) in &f.contractions {
                        let Some((a, b)) = k.split_once(' ') else {
                            continue;
                        };
                        if a != last {
                            continue;
                        }
                        if b.ends_with('\'') {
                            if fw.starts_with(b) || fw.starts_with(&b.replace('\'', "’")) {
                                words.pop();
                                break;
                            }
                        } else if fw == b {
                            words.pop();
                            var_text = format!("{rep}{}", &var_text[b.len()..]);
                            break;
                        }
                    }
                }
            }
            let mut joined = words.join(" ");
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(&var_text);
            s = joined;
        }
        s
    }

    /// Resolve a capture with its resolvers, in order (R-B16-05).
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &self,
        capture: &str,
        list: &[Resolver],
        recursive: bool,
        caller: usize,
        glossary: &Glossary,
        comp: Option<&Composition>,
        bases: &dyn Bases,
        depth: usize,
    ) -> Option<Resolved> {
        let single = || {
            let cover = glossary.cover(capture);
            let solid: Vec<&Piece> = cover
                .pieces
                .iter()
                .filter(|p| !matches!(p, Piece::Space(_)))
                .collect();
            match solid.as_slice() {
                [Piece::Term { entry, .. }] => Some(glossary.entry(*entry).term.clone()),
                _ => None,
            }
        };
        for r in list {
            match r {
                Resolver::Glossary | Resolver::Adjective => {
                    if let Some(t) = single() {
                        let has_forms = !t.forms.is_empty();
                        if *r == Resolver::Adjective && !has_forms {
                            continue;
                        }
                        let text = t.text(t.gender).to_string();
                        if text.trim().is_empty() {
                            continue;
                        }
                        return Some(Resolved {
                            text,
                            gender: t.gender,
                            forms: has_forms.then(|| t.forms.clone()),
                            lowerable: !t.case,
                            low_ok: !t.case,
                            how: if *r == Resolver::Adjective {
                                How::Adjective
                            } else {
                                How::Glossary
                            },
                            steps: Vec::new(),
                        });
                    }
                }
                Resolver::Bases => {
                    if let Some(t) = bases.lookup(capture)
                        && !t.is_empty()
                    {
                        return Some(Resolved {
                            text: t,
                            gender: None,
                            forms: None,
                            lowerable: false,
                            low_ok: false,
                            how: How::Bases,
                            steps: Vec::new(),
                        });
                    }
                }
                Resolver::Patterns | Resolver::Pattern(_) => {
                    if !recursive || depth >= 3 {
                        continue;
                    }
                    let only = match r {
                        Resolver::Pattern(n) => Some(n - 2),
                        _ => None,
                    };
                    if let Some((_, res)) = self.try_all(
                        capture,
                        glossary,
                        comp,
                        bases,
                        depth + 1,
                        Some(caller),
                        only,
                    ) {
                        return Some(res);
                    }
                }
                Resolver::Copy => {
                    let ok = capture
                        .split_whitespace()
                        .all(|w| w.chars().next().is_some_and(char::is_uppercase));
                    if ok {
                        return Some(Resolved {
                            text: capture.trim().to_string(),
                            gender: None,
                            forms: None,
                            lowerable: false,
                            low_ok: false,
                            how: How::Copy,
                            steps: Vec::new(),
                        });
                    }
                }
            }
        }
        // Last, the glossary word by word (the composition).
        if list.contains(&Resolver::Glossary) {
            let cover = glossary.cover(capture);
            if cover.complete()
                && let Some(t) = compose::translate(glossary, &cover, comp, capture)
            {
                return Some(Resolved {
                    text: t.text,
                    gender: t.gender,
                    forms: None,
                    lowerable: !t.case_kept,
                    low_ok: !t.case_kept,
                    how: How::Compose,
                    steps: Vec::new(),
                });
            }
        }
        None
    }
}
