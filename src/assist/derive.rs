//! Dérivation d'objets (B-21): the derived objects of a game — the scroll
//! « Scroll of Fireball », the tome « Spell Tome: Fireball », the potion
//! « Weak Potion of Fortify One-handed » — written from the translated
//! name of their parent record by a rule of the game's file
//! `assets/data/assist/derive_<Game>.json` (user's copy in `data/assist/`):
//! the source pattern matched on the line, `%A%` looked up, case folded,
//! among the sources of the parent lines — of the document, of the other
//! open files, of the stack's dictionaries — and the target pattern of the
//! target language written with the rules of the patterns (B-16). The tome
//! takes the spell it teaches (DATA) first (decision D-79). The parent's
//! translation may lose a listed prefix (« Invocation - ») and the rule
//! may have other variables, resolved like those of a pattern, with the
//! gender of the noun it writes (decision D-82).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::glossary::{DIR, Gender};
use super::patterns::{Resolver, Step, variables};
use super::{Assistant, Bases};
use crate::tm::TmEntry;

pub const FORMAT: u32 = 1;

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// A value for every target language, or one per language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PerLang<T> {
    All(T),
    Langs(BTreeMap<String, T>),
}

impl<T> PerLang<T> {
    /// The value for a target language (ISO 639-3, lowercase).
    pub fn get(&self, lang: &str) -> Option<&T> {
        match self {
            PerLang::All(v) => Some(v),
            PerLang::Langs(m) => m.get(&lang.to_ascii_lowercase()),
        }
    }
}

/// One signature or several, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Sigs {
    One(String),
    Many(Vec<String>),
}

impl Sigs {
    pub fn list(&self) -> Vec<&str> {
        match self {
            Sigs::One(s) => vec![s.as_str()],
            Sigs::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// A derivation rule (R-B21-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    /// The derived object, `REC:FLD`.
    pub target: String,
    /// The source pattern (`%A%` is the parent's name).
    pub src: String,
    /// The parent records whose translated name resolves `%A%`, in order.
    pub parent: Sigs,
    /// `teaches`: the spell a tome teaches (BOOK DATA) first.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The target pattern, per target language.
    pub dst: PerLang<String>,
    /// Prefixes or suffixes taken off the parent's translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip: Option<PerLang<Vec<String>>>,
    /// The resolvers of the other variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, Vec<Resolver>>,
    /// The gender of the noun the rule writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<PerLang<Gender>>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl Rule {
    pub fn parents(&self) -> Vec<&str> {
        self.parent.list()
    }

    /// The target pattern for a language; `None`: the rule is inactive
    /// for that target.
    pub fn dst_for(&self, lang: &str) -> Option<&str> {
        self.dst
            .get(lang)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
    }

    pub fn strip_for(&self, lang: &str) -> &[String] {
        self.strip
            .as_ref()
            .and_then(|s| s.get(lang))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn gender_for(&self, lang: &str) -> Option<Gender> {
        self.gender.as_ref().and_then(|g| g.get(lang)).copied()
    }
}

/// A derivation file, format 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeriveFile {
    pub format: u32,
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for DeriveFile {
    fn default() -> Self {
        DeriveFile {
            format: FORMAT,
            game: String::new(),
            rules: Vec::new(),
        }
    }
}

/// Why a file is refused (R-B21-06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    Json(String),
    Format(u32),
    /// A rule without an id (its rank, from 1).
    NoId(usize),
    DuplicateId(String),
    /// A signature that is not `AAAA:BBBB`.
    Sig {
        id: String,
        sig: String,
    },
    EmptySrc(String),
    /// A variable of a target pattern missing in the source pattern.
    VarMissing {
        id: String,
        var: String,
        lang: String,
    },
}

/// `AAAA:BBBB`: four letters, digits or `_` on each side.
pub fn is_sig(s: &str) -> bool {
    let ok = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    matches!(s.split_once(':'), Some((a, b)) if ok(a) && ok(b))
}

impl DeriveFile {
    pub fn parse(text: &str) -> Result<DeriveFile, Invalid> {
        let f: DeriveFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
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

    pub fn validate(&self) -> Result<(), Invalid> {
        if self.format != FORMAT {
            return Err(Invalid::Format(self.format));
        }
        let mut ids: HashSet<&str> = HashSet::new();
        for (k, r) in self.rules.iter().enumerate() {
            let id = r.id.trim();
            if id.is_empty() {
                return Err(Invalid::NoId(k + 1));
            }
            if !ids.insert(id) {
                return Err(Invalid::DuplicateId(id.to_string()));
            }
            for s in std::iter::once(r.target.as_str()).chain(r.parents()) {
                if !is_sig(s) {
                    return Err(Invalid::Sig {
                        id: id.to_string(),
                        sig: s.to_string(),
                    });
                }
            }
            if r.src.trim().is_empty() {
                return Err(Invalid::EmptySrc(id.to_string()));
            }
            let vs: Vec<String> = variables(&r.src).into_iter().map(|v| v.0).collect();
            let dsts: Vec<(String, &str)> = match &r.dst {
                PerLang::All(d) => vec![(String::new(), d.as_str())],
                PerLang::Langs(m) => m.iter().map(|(l, d)| (l.clone(), d.as_str())).collect(),
            };
            for (lang, d) in dsts {
                if let Some((v, _)) = variables(d).into_iter().find(|(v, _)| !vs.contains(v)) {
                    return Err(Invalid::VarMissing {
                        id: id.to_string(),
                        var: v,
                        lang,
                    });
                }
            }
        }
        Ok(())
    }

    /// The rules that act for a target language.
    pub fn active(&self, lang: &str) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(move |r| r.enabled && r.dst_for(lang).is_some())
    }
}

/// `derive_SkyrimSE.json`.
pub fn file_name(game: &str) -> String {
    format!("derive_{game}.json")
}

fn embedded(name: &str) -> Option<&'static str> {
    Some(match name {
        "derive_Skyrim.json" => include_str!("../../assets/data/assist/derive_Skyrim.json"),
        "derive_SkyrimSE.json" => include_str!("../../assets/data/assist/derive_SkyrimSE.json"),
        "derive_Fallout4.json" => include_str!("../../assets/data/assist/derive_Fallout4.json"),
        "derive_Oblivion.json" => include_str!("../../assets/data/assist/derive_Oblivion.json"),
        "derive_Starfield.json" => include_str!("../../assets/data/assist/derive_Starfield.json"),
        _ => return None,
    })
}

/// The shipped file of a game (the file, else the embedded copy).
pub fn shipped_file(shipped: &Path, game: &str) -> Option<DeriveFile> {
    let name = file_name(game);
    std::fs::read_to_string(shipped.join(&name))
        .ok()
        .and_then(|t| DeriveFile::parse(&t).ok())
        .or_else(|| embedded(&name).and_then(|t| DeriveFile::parse(t).ok()))
}

/// The file of a game: the user's copy when present and valid, else the
/// shipped one; with whether it is the user's.
pub fn load_file(shipped: &Path, user: Option<&PathBuf>, game: &str) -> Option<(DeriveFile, bool)> {
    let name = file_name(game);
    if let Some(u) = user
        && let Ok(text) = std::fs::read_to_string(u.join(&name))
    {
        match DeriveFile::parse(&text).and_then(|f| f.validate().map(|_| f)) {
            Ok(f) => return Some((f, true)),
            Err(e) => tracing::warn!("{}: {e:?}", u.join(&name).display()),
        }
    }
    shipped_file(shipped, game).map(|f| (f, false))
}

/// Write the user's copy of a game, validated first.
pub fn save(user: &Path, game: &str, f: &DeriveFile) -> Result<PathBuf, Invalid> {
    f.validate()?;
    let p = user.join(file_name(game));
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
// The parents
// ----------------------------------------------------------------------

/// Where a parent line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    Document,
    OpenFile,
}

/// A parent line: its translation (empty when it has none) and its EDID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cand {
    pub trans: String,
    pub edid: String,
    pub origin: Origin,
}

/// The parent lines of the document and of the other open files, by
/// signature and case-folded source, and by FormID for the spells a tome
/// teaches.
#[derive(Debug, Clone, Default)]
pub struct Parents {
    by_sig: HashMap<String, HashMap<String, Vec<Cand>>>,
    by_formid: HashMap<(String, u32), Cand>,
}

fn sig_of(e: &TmEntry) -> String {
    format!("{}:{}", e.rec_sig, e.field_sig)
}

impl Parents {
    /// Add the lines of a file whose signature is in `sigs`; `trans` gives
    /// the translation to read (the planned one, else the line's).
    pub fn add(
        &mut self,
        entries: &[TmEntry],
        origin: Origin,
        sigs: &HashSet<String>,
        trans: &dyn Fn(usize, &TmEntry) -> String,
    ) {
        for (i, e) in entries.iter().enumerate() {
            if e.unused {
                continue;
            }
            let sig = sig_of(e);
            if !sigs.contains(&sig) {
                continue;
            }
            let c = Cand {
                trans: trans(i, e).trim().to_string(),
                edid: e.edid.clone(),
                origin,
            };
            if origin == Origin::Document && e.formid != 0 {
                self.by_formid.insert((sig.clone(), e.formid), c.clone());
            }
            self.by_sig
                .entry(sig)
                .or_default()
                .entry(e.source.trim().to_lowercase())
                .or_default()
                .push(c);
        }
    }

    pub fn get(&self, sig: &str, name: &str) -> &[Cand] {
        self.by_sig
            .get(sig)
            .and_then(|m| m.get(&name.trim().to_lowercase()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn by_formid(&self, sig: &str, formid: u32) -> Option<&Cand> {
        self.by_formid.get(&(sig.to_string(), formid))
    }
}

/// The signatures the active rules look up.
pub fn parent_sigs(f: &DeriveFile, lang: &str) -> HashSet<String> {
    f.active(lang)
        .flat_map(|r| r.parents().into_iter().map(str::to_string))
        .collect()
}

/// The dictionaries a parent may be looked up in: by signature when the
/// dictionary has them, by text only otherwise (R-B21-02).
pub trait ParentBases {
    fn lookup(&self, rec: &str, field: &str, text: &str) -> Option<String>;
}

impl ParentBases for super::NoBases {
    fn lookup(&self, _: &str, _: &str, _: &str) -> Option<String> {
        None
    }
}

impl ParentBases for super::StackBases {
    fn lookup(&self, rec: &str, field: &str, text: &str) -> Option<String> {
        let e = TmEntry {
            rec_sig: rec.to_string(),
            field_sig: field.to_string(),
            source: text.trim().to_string(),
            ..Default::default()
        };
        self.0.iter().find_map(|db| {
            db.lookup_text(&e)
                .or_else(|| db.lookup_text_only(&e))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
    }
}

/// The spells the tomes of a plugin teach (BOOK DATA, flag 0x04): book
/// FormID → spell FormID.
pub fn teaches(path: &Path) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    let Ok(data) = std::fs::read(path) else {
        return out;
    };
    let Ok(plugin) = crate::plugin::reader::parse(&data) else {
        return out;
    };
    for r in plugin.records.iter().filter(|r| r.sig == "BOOK") {
        if let Some(d) = r.subrecords.iter().find(|s| s.sig == "DATA")
            && d.data.len() >= 8
            && d.data[0] & 0x04 != 0
        {
            let spell = u32::from_le_bytes([d.data[4], d.data[5], d.data[6], d.data[7]]);
            if spell != 0 {
                out.insert(r.formid, spell);
            }
        }
    }
    out
}

// ----------------------------------------------------------------------
// Derivation
// ----------------------------------------------------------------------

/// What became of a line a rule matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derived {
    /// Derived: the text, the provenance, the number of the parent's
    /// distinct translations when there are several (B-08), 0 otherwise.
    Text {
        text: String,
        prov: String,
        rule: String,
        parent: String,
        ambiguous: usize,
        steps: Vec<Step>,
    },
    /// A parent was found without a translation.
    Untranslated { rule: String, parent: String },
    /// No parent of that name.
    NotFound { rule: String },
}

/// Everything a derivation needs.
pub struct Deriver<'a> {
    pub file: &'a DeriveFile,
    pub lang: &'a str,
    pub parents: &'a Parents,
    pub parent_bases: &'a dyn ParentBases,
    /// For the other variables and the rules of the target language.
    pub assistant: &'a Assistant,
    pub bases: &'a dyn Bases,
    /// book FormID → spell FormID.
    pub teaches: &'a HashMap<u32, u32>,
}

/// The parent found for a capture.
enum Found {
    Text {
        trans: String,
        edid: String,
        amb: usize,
    },
    Untranslated(String),
    None,
}

/// The provenance code of a derived line (B-09).
pub fn prov(rule: &str, parent: &str) -> String {
    format!("{}{rule}:{parent}", crate::provenance::RULE_DERIVE)
}

/// Strip the listed prefixes and suffixes off a translation.
fn strip(t: &str, list: &[String]) -> String {
    let mut s = t.to_string();
    for p in list {
        if let Some(r) = s.strip_prefix(p.as_str()) {
            s = r.to_string();
        }
        if let Some(r) = s.strip_suffix(p.as_str()) {
            s = r.to_string();
        }
    }
    s.trim().to_string()
}

impl Deriver<'_> {
    /// Derive a line by the first active rule whose target and source
    /// pattern match it; `only`: that rule only. `None` when no rule
    /// matches the line.
    pub fn derive(&self, e: &TmEntry, only: Option<&str>) -> Option<Derived> {
        let sig = sig_of(e);
        let mut last: Option<Derived> = None;
        for r in self.file.active(self.lang) {
            if r.target != sig || only.is_some_and(|o| o != r.id) {
                continue;
            }
            let Some(dst) = r.dst_for(self.lang) else {
                continue;
            };
            match self.with_rule(r, dst, e) {
                Some(d @ Derived::Text { .. }) => return Some(d),
                Some(d) if last.is_none() => last = Some(d),
                _ => {}
            }
        }
        last
    }

    fn find(&self, r: &Rule, e: &TmEntry, capture: &str) -> Found {
        // The spell a tome teaches, by FormID.
        if r.reference.as_deref() == Some("teaches")
            && let Some(spell) = self.teaches.get(&e.formid)
        {
            for p in r.parents() {
                if let Some(c) = self.parents.by_formid(p, *spell)
                    && !c.trans.is_empty()
                {
                    return Found::Text {
                        trans: c.trans.clone(),
                        edid: c.edid.clone(),
                        amb: 0,
                    };
                }
            }
        }
        let mut untranslated: Option<String> = None;
        for p in r.parents() {
            let cands = self.parents.get(p, capture);
            if let Some(first) = cands.iter().find(|c| !c.trans.is_empty()) {
                let mut distinct: Vec<&str> = cands
                    .iter()
                    .filter(|c| !c.trans.is_empty())
                    .map(|c| c.trans.as_str())
                    .collect();
                distinct.sort_unstable();
                distinct.dedup();
                return Found::Text {
                    trans: first.trans.clone(),
                    edid: first.edid.clone(),
                    amb: if distinct.len() >= 2 {
                        distinct.len()
                    } else {
                        0
                    },
                };
            }
            if untranslated.is_none()
                && let Some(c) = cands.first()
            {
                untranslated = Some(c.edid.clone());
            }
            let (rec, fld) = p.split_once(':').unwrap_or((p, ""));
            if let Some(t) = self.parent_bases.lookup(rec, fld, capture) {
                return Found::Text {
                    trans: t,
                    edid: capture.trim().to_string(),
                    amb: 0,
                };
            }
        }
        match untranslated {
            Some(edid) => Found::Untranslated(edid),
            None => Found::None,
        }
    }

    fn with_rule(&self, r: &Rule, dst: &str, e: &TmEntry) -> Option<Derived> {
        let patterns = self.assistant.patterns.as_deref();
        let default = super::Patterns::default();
        let p = patterns.unwrap_or(&default);
        let strip_list = r.strip_for(self.lang);
        let glossary = &self.assistant.glossary;
        let mut state: Option<Found> = None;
        let mut parent = |capture: &str| -> Option<(String, bool)> {
            let f = self.find(r, e, capture);
            let out = match &f {
                Found::Text { trans, .. } => {
                    let t = strip(trans, strip_list);
                    // A proper name (a term whose case is kept) keeps its
                    // capital under `:low`.
                    let first = t.split_whitespace().next().unwrap_or("");
                    let proper = glossary.entries().iter().any(|g| {
                        g.term.case && g.term.text(None).split_whitespace().next() == Some(first)
                    });
                    (!t.is_empty()).then_some((t, !proper))
                }
                _ => None,
            };
            state = Some(f);
            out
        };
        let res = p.derive_with(
            &r.src,
            dst,
            &r.vars,
            &e.source,
            glossary,
            self.assistant.composition.as_ref(),
            self.bases,
            "A",
            &mut parent,
            r.gender_for(self.lang),
        );
        match (res, state) {
            (Some((text, steps)), Some(Found::Text { edid, amb, .. })) => Some(Derived::Text {
                prov: prov(&r.id, &edid),
                text,
                rule: r.id.clone(),
                parent: edid,
                ambiguous: amb,
                steps,
            }),
            // A rule without %A% (« même nom » impossible without a
            // parent) or another variable left unresolved.
            (_, Some(Found::Untranslated(edid))) => Some(Derived::Untranslated {
                rule: r.id.clone(),
                parent: edid,
            }),
            (_, Some(Found::None)) => Some(Derived::NotFound { rule: r.id.clone() }),
            _ => None,
        }
    }
}

/// The rule and the parent EDID of a derived line, from its provenance.
pub fn derived_from(prov: &str) -> Option<(&str, &str)> {
    let rest = crate::provenance::split_ambiguous(prov)
        .0
        .strip_prefix(crate::provenance::RULE_DERIVE)?;
    rest.split_once(':')
}

/// Following the parent (R-B21-04): the derived lines still En attente
/// whose parent is among the `changed` lines, with their new text when it
/// differs. A line validated or typed by hand no longer follows (its
/// status or provenance changed).
pub fn refollow(
    d: &Deriver,
    entries: &[TmEntry],
    changed: &[usize],
) -> Vec<(usize, String, String)> {
    let parents: HashSet<&str> = changed
        .iter()
        .filter_map(|&i| entries.get(i))
        .filter(|e| !e.edid.is_empty())
        .map(|e| e.edid.as_str())
        .collect();
    if parents.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let Some((rule, parent)) = derived_from(&e.prov) else {
            continue;
        };
        if !parents.contains(parent) || crate::status::derive(e) != crate::status::Status::Pending {
            continue;
        }
        if let Some(Derived::Text { text, prov, .. }) = d.derive(e, Some(rule))
            && text != e.trans
        {
            out.push((i, text, prov));
        }
    }
    out
}
