//! Glossaire de termes par groupe (B-15): the terms of a language pair —
//! original, translation, class, options — organised in groups, in a JSON
//! file shipped read-only (`assets/data/assist/glossary_<SRC>-<DST>.json`)
//! with a user's copy that wins (`data/assist/`), and an optional glossary
//! per game (`glossary_<Game>_<SRC>-<DST>.json`) whose terms come first
//! (decision D-76).
//!
//! A text is cut as for F9 (words, tags, numbers, punctuation, spaces);
//! each run of words is covered by the longest term that starts there —
//! at equal length, the first of the files — and a text is translated only
//! when every word is covered (decision D-77).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::instant::{Kind, Token, tokens};

/// The folder of the assistants' files, shipped and user's.
pub const DIR: &str = "assist";
pub const FORMAT: u32 = 1;

/// The class of a term: where the composition puts it (R-B15-02, R-B16-02).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// The noun: gives the gender and the number.
    Noun,
    /// A complement bound to the noun (« d'apparat », « à masque »).
    Bound,
    /// A relational adjective of a people or a place (« nordique »,
    /// « elfique »), agreed; French puts it next to the noun (B-16).
    People,
    /// An adjective, agreed.
    Adj,
    /// A colour, agreed.
    Color,
    /// A material (« en acier »).
    Material,
    /// An origin (« du Tribunal »).
    Origin,
    /// No class: the composition leaves it in place.
    #[default]
    Free,
}

impl Class {
    pub const ALL: [Class; 8] = [
        Class::Noun,
        Class::Bound,
        Class::People,
        Class::Adj,
        Class::Color,
        Class::Material,
        Class::Origin,
        Class::Free,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Class::Noun => "glossary-class-noun",
            Class::Bound => "glossary-class-bound",
            Class::People => "glossary-class-people",
            Class::Adj => "glossary-class-adj",
            Class::Color => "glossary-class-color",
            Class::Material => "glossary-class-material",
            Class::Origin => "glossary-class-origin",
            Class::Free => "glossary-class-free",
        }
    }
}

/// Gender and number of a noun, or the form of an adjective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    M,
    F,
    Mp,
    Fp,
}

impl Gender {
    pub const ALL: [Gender; 4] = [Gender::M, Gender::F, Gender::Mp, Gender::Fp];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Gender::M => "glossary-gender-m",
            Gender::F => "glossary-gender-f",
            Gender::Mp => "glossary-gender-mp",
            Gender::Fp => "glossary-gender-fp",
        }
    }
}

/// Where a term may match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pos {
    #[default]
    Any,
    /// The term must open the text.
    Start,
    /// The term must close the text.
    End,
}

impl Pos {
    pub const ALL: [Pos; 3] = [Pos::Any, Pos::Start, Pos::End];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Pos::Any => "glossary-pos-any",
            Pos::Start => "glossary-pos-start",
            Pos::End => "glossary-pos-end",
        }
    }
}

/// The forms of an adjective or a colour, in place of `dst`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fp: Option<String>,
}

impl Forms {
    pub fn is_empty(&self) -> bool {
        [&self.m, &self.f, &self.mp, &self.fp]
            .iter()
            .all(|f| f.as_deref().is_none_or(|s| s.trim().is_empty()))
    }

    pub fn slot(&self, g: Gender) -> &Option<String> {
        match g {
            Gender::M => &self.m,
            Gender::F => &self.f,
            Gender::Mp => &self.mp,
            Gender::Fp => &self.fp,
        }
    }

    pub fn slot_mut(&mut self, g: Gender) -> &mut Option<String> {
        match g {
            Gender::M => &mut self.m,
            Gender::F => &mut self.f,
            Gender::Mp => &mut self.mp,
            Gender::Fp => &mut self.fp,
        }
    }

    /// The form of a gender; a missing one falls back on the nearest
    /// (feminine plural: masculine plural, then feminine, then masculine).
    pub fn get(&self, g: Gender) -> Option<&str> {
        let order: &[Gender] = match g {
            Gender::M => &[Gender::M],
            Gender::F => &[Gender::F, Gender::M],
            Gender::Mp => &[Gender::Mp, Gender::M],
            Gender::Fp => &[Gender::Fp, Gender::Mp, Gender::F, Gender::M],
        };
        order
            .iter()
            .filter_map(|g| self.slot(*g).as_deref())
            .find(|s| !s.trim().is_empty())
    }
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_free(c: &Class) -> bool {
    *c == Class::Free
}

fn is_any(p: &Pos) -> bool {
    *p == Pos::Any
}

/// One term (R-B15-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Term {
    /// One or more words.
    pub src: String,
    /// The translation; empty drops the words (« Light »).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dst: String,
    #[serde(default, skip_serializing_if = "is_free")]
    pub class: Class,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<Gender>,
    #[serde(default, skip_serializing_if = "Forms::is_empty")]
    pub forms: Forms,
    /// Whole words only (« Iron » does not take « Ironhand »).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub whole: bool,
    #[serde(default, skip_serializing_if = "is_any")]
    pub pos: Pos,
    /// Matched and translated as is (proper names, acronyms).
    #[serde(default, skip_serializing_if = "is_false")]
    pub case: bool,
}

impl Default for Term {
    fn default() -> Self {
        Term {
            src: String::new(),
            dst: String::new(),
            class: Class::Free,
            gender: None,
            forms: Forms::default(),
            whole: true,
            pos: Pos::Any,
            case: false,
        }
    }
}

impl Term {
    pub fn new(src: &str, dst: &str, class: Class) -> Term {
        Term {
            src: src.to_string(),
            dst: dst.to_string(),
            class,
            ..Default::default()
        }
    }

    /// The translation for a noun of gender `g` (the forms when there are
    /// some, `dst` otherwise).
    pub fn text(&self, g: Option<Gender>) -> &str {
        if !self.forms.is_empty()
            && let Some(f) = self.forms.get(g.unwrap_or(Gender::M))
        {
            return f;
        }
        &self.dst
    }

    /// « elfique / elfiques »: what the table shows in the Traduit column.
    pub fn shown(&self) -> String {
        if self.forms.is_empty() {
            return self.dst.clone();
        }
        let mut seen: Vec<&str> = Vec::new();
        for g in Gender::ALL {
            if let Some(f) = self
                .forms
                .slot(g)
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                && !seen.contains(&f)
            {
                seen.push(f);
            }
        }
        seen.join(" / ")
    }

    /// The key of the term in its group (unique, R-B15-05).
    fn key(&self) -> (bool, String) {
        let s = self.src.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.case {
            (true, s)
        } else {
            (false, s.to_lowercase())
        }
    }
}

/// A group of terms (« Matériaux »).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub terms: Vec<Term>,
}

/// A glossary file, format 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryFile {
    pub format: u32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub groups: Vec<Group>,
}

impl Default for GlossaryFile {
    fn default() -> Self {
        GlossaryFile {
            format: FORMAT,
            source: String::new(),
            target: String::new(),
            groups: Vec::new(),
        }
    }
}

/// Why a file is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    Json(String),
    Format(u32),
    /// A group without a name (its rank, from 1).
    GroupName(usize),
    DuplicateGroup(String),
    /// A term with an empty original, in its group.
    EmptySrc(String),
    /// A term twice in its group.
    Duplicate {
        src: String,
        group: String,
    },
    /// Forms without the masculine one.
    Forms {
        src: String,
        group: String,
    },
}

impl GlossaryFile {
    pub fn parse(text: &str) -> Result<GlossaryFile, Invalid> {
        let f: GlossaryFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
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

    pub fn term_count(&self) -> usize {
        self.groups.iter().map(|g| g.terms.len()).sum()
    }

    /// R-B15-05: an original is never empty and unique in its group; forms
    /// start with the masculine one.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.format != FORMAT {
            return Err(Invalid::Format(self.format));
        }
        let mut names: Vec<String> = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            let name = g.name.trim();
            if name.is_empty() {
                return Err(Invalid::GroupName(gi + 1));
            }
            if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                return Err(Invalid::DuplicateGroup(name.to_string()));
            }
            names.push(name.to_string());
            if let Some(d) = group_problem(g) {
                return Err(d);
            }
        }
        Ok(())
    }

    /// The group of that name (case ignored), created at the end if absent.
    pub fn group_mut(&mut self, name: &str) -> &mut Group {
        let name = name.trim();
        let i = match self
            .groups
            .iter()
            .position(|g| g.name.trim().eq_ignore_ascii_case(name))
        {
            Some(i) => i,
            None => {
                self.groups.push(Group {
                    name: name.to_string(),
                    terms: Vec::new(),
                });
                self.groups.len() - 1
            }
        };
        &mut self.groups[i]
    }

    /// Add a term to a group (created if needed); refused when its
    /// original is empty or already in the group.
    pub fn add(&mut self, group: &str, term: Term) -> Result<(), Invalid> {
        let gname = group.trim().to_string();
        let g = self.group_mut(&gname);
        if term.src.trim().is_empty() {
            return Err(Invalid::EmptySrc(gname));
        }
        if g.terms.iter().any(|t| t.key() == term.key()) {
            return Err(Invalid::Duplicate {
                src: term.src.trim().to_string(),
                group: g.name.clone(),
            });
        }
        g.terms.push(term);
        Ok(())
    }
}

/// The first problem of a group's terms.
pub fn group_problem(g: &Group) -> Option<Invalid> {
    let mut keys: Vec<(bool, String)> = Vec::new();
    for t in &g.terms {
        if t.src.trim().is_empty() {
            return Some(Invalid::EmptySrc(g.name.clone()));
        }
        let k = t.key();
        if keys.contains(&k) {
            return Some(Invalid::Duplicate {
                src: t.src.trim().to_string(),
                group: g.name.clone(),
            });
        }
        keys.push(k);
        if !t.forms.is_empty() && t.forms.m.as_deref().is_none_or(|m| m.trim().is_empty()) {
            return Some(Invalid::Forms {
                src: t.src.clone(),
                group: g.name.clone(),
            });
        }
    }
    None
}

/// The terms of a CSV or TSV file of two columns « original ; traduit »
/// (a header line is skipped when its first cell says so). The separator
/// is the first of tab, semicolon, comma found on the first line.
pub fn read_table(text: &str) -> Vec<(String, String)> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or("");
    let sep = ['\t', ';', ','].into_iter().find(|c| first.contains(*c));
    let Some(sep) = sep else {
        return Vec::new();
    };
    let unquote = |s: &str| {
        let s = s.trim();
        s.strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .map(|s| s.replace("\"\"", "\""))
            .unwrap_or_else(|| s.to_string())
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut cells = line.splitn(2, sep);
        let a = unquote(cells.next().unwrap_or(""));
        let b = unquote(cells.next().unwrap_or(""));
        if i == 0 && is_header(&a) {
            continue;
        }
        out.push((a, b));
    }
    out
}

fn is_header(cell: &str) -> bool {
    let c = cell.trim().to_lowercase();
    [
        "original", "source", "src", "term", "terme", "english", "eng",
    ]
    .contains(&c.as_str())
}

// ----------------------------------------------------------------------
// Files
// ----------------------------------------------------------------------

/// `glossary_ENG-FRA.json`, `glossary_SkyrimSE_ENG-FRA.json`.
pub fn file_name(pair: &str, game: Option<&str>) -> String {
    match game {
        Some(g) => format!("glossary_{g}_{pair}.json"),
        None => format!("glossary_{pair}.json"),
    }
}

fn embedded(name: &str) -> Option<&'static str> {
    Some(match name {
        "glossary_ENG-FRA.json" => include_str!("../../assets/data/assist/glossary_ENG-FRA.json"),
        "glossary_ENG-ZHO.json" => include_str!("../../assets/data/assist/glossary_ENG-ZHO.json"),
        _ => return None,
    })
}

/// Which copy was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Shipped,
    /// No file: an empty glossary.
    None,
}

/// The folders of the assistants' files.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub shipped: PathBuf,
    pub user: Option<PathBuf>,
}

impl Dirs {
    pub fn installed() -> Dirs {
        let p = crate::config::paths::get();
        Dirs {
            shipped: crate::config::assets_data_dir().join(DIR),
            user: Some(p.data_dir.join(DIR)),
        }
    }

    pub fn user_path(&self, name: &str) -> Option<PathBuf> {
        self.user.as_ref().map(|u| u.join(name))
    }

    pub fn shipped_path(&self, name: &str) -> PathBuf {
        self.shipped.join(name)
    }

    /// The shipped file (the file, else the embedded copy).
    pub fn shipped(&self, name: &str) -> Option<GlossaryFile> {
        std::fs::read_to_string(self.shipped_path(name))
            .ok()
            .and_then(|t| GlossaryFile::parse(&t).ok())
            .or_else(|| embedded(name).and_then(|t| GlossaryFile::parse(t).ok()))
    }

    /// The user's copy when present and valid, else the shipped one; the
    /// error of an invalid copy comes back with the shipped one.
    pub fn load(&self, name: &str) -> (GlossaryFile, Origin, Option<Invalid>) {
        let mut err = None;
        if let Some(p) = self.user_path(name)
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            match GlossaryFile::parse(&text).and_then(|f| f.validate().map(|_| f)) {
                Ok(f) => return (f, Origin::User, None),
                Err(e) => {
                    tracing::warn!("{}: {e:?}", p.display());
                    err = Some(e);
                }
            }
        }
        match self.shipped(name) {
            Some(f) => (f, Origin::Shipped, err),
            None => (GlossaryFile::default(), Origin::None, err),
        }
    }

    /// Write the user's copy, validated first.
    pub fn save(&self, name: &str, f: &GlossaryFile) -> Result<PathBuf, Invalid> {
        f.validate()?;
        let p = self
            .user_path(name)
            .ok_or_else(|| Invalid::Json("no user folder".into()))?;
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        crate::config::paths::write_atomic(&p, f.to_json().as_bytes())
            .map_err(|e| Invalid::Json(e.to_string()))?;
        Ok(p)
    }

    /// Remove the user's copy (« Restaurer la version livrée »).
    pub fn restore(&self, name: &str) -> std::io::Result<()> {
        match self.user_path(name) {
            Some(p) if p.is_file() => std::fs::remove_file(p),
            _ => Ok(()),
        }
    }
}

// ----------------------------------------------------------------------
// Matching
// ----------------------------------------------------------------------

/// A term of the merged glossary, with where it comes from.
#[derive(Debug, Clone)]
pub struct Entry {
    pub term: Term,
    pub group: String,
    /// From the game's glossary.
    pub game: bool,
    /// The solid tokens of the original: lowercase, or as is for a term
    /// whose case is kept.
    keys: Vec<String>,
}

/// The terms of the pair (and of the game), compiled for matching.
#[derive(Debug, Clone, Default)]
pub struct Glossary {
    entries: Vec<Entry>,
    /// By lowercase first token.
    by_first: HashMap<String, Vec<usize>>,
    /// Single-word terms that may match inside a word (`whole` false).
    parts: Vec<usize>,
}

fn solid(text: &str) -> Vec<Token> {
    tokens(text.trim())
        .into_iter()
        .filter(|t| t.kind != Kind::Space)
        .collect()
}

impl Glossary {
    /// The files in priority order: the game's first (D-76).
    pub fn new(files: &[(&GlossaryFile, bool)]) -> Glossary {
        let mut g = Glossary::default();
        for (f, game) in files {
            for grp in &f.groups {
                for t in &grp.terms {
                    g.push(t.clone(), &grp.name, *game);
                }
            }
        }
        g
    }

    fn push(&mut self, term: Term, group: &str, game: bool) {
        let toks = solid(&term.src);
        if toks.is_empty() {
            return;
        }
        let keys: Vec<String> = toks
            .iter()
            .map(|t| {
                if term.case {
                    t.text.clone()
                } else {
                    t.key.clone()
                }
            })
            .collect();
        let first = toks[0].key.clone();
        let idx = self.entries.len();
        if !term.whole && toks.len() == 1 && toks[0].kind == Kind::Word {
            self.parts.push(idx);
        }
        self.by_first.entry(first).or_default().push(idx);
        self.entries.push(Entry {
            term,
            group: group.to_string(),
            game,
            keys,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, i: usize) -> &Entry {
        &self.entries[i]
    }

    /// The longest term matching the solid tokens from `at`; ties: the
    /// first in the files.
    fn longest_at(
        &self,
        toks: &[&Token],
        at: usize,
        first_word: usize,
        last_word: usize,
    ) -> Option<(usize, usize)> {
        let list = self.by_first.get(&toks[at].key)?;
        let mut best: Option<(usize, usize)> = None;
        for &i in list {
            let e = &self.entries[i];
            let n = e.keys.len();
            if at + n > toks.len() {
                continue;
            }
            let same = e.keys.iter().zip(&toks[at..at + n]).all(|(k, t)| {
                if e.term.case {
                    *k == t.text
                } else {
                    *k == t.key
                }
            });
            if !same {
                continue;
            }
            let pos_ok = match e.term.pos {
                Pos::Any => true,
                Pos::Start => at == first_word,
                Pos::End => at + n - 1 == last_word,
            };
            if pos_ok && best.is_none_or(|(_, bn)| n > bn) {
                best = Some((i, n));
            }
        }
        best
    }

    /// A word cut into pieces that terms matching inside words cover
    /// (`whole` false): the longest at each point.
    fn split_word(&self, word: &str) -> Option<Vec<(usize, String)>> {
        if self.parts.is_empty() {
            return None;
        }
        let chars: Vec<char> = word.chars().collect();
        let lower: Vec<char> = word.to_lowercase().chars().collect();
        if lower.len() != chars.len() {
            return None;
        }
        let mut out = Vec::new();
        let mut at = 0;
        while at < chars.len() {
            let mut best: Option<(usize, usize)> = None;
            for &i in &self.parts {
                let e = &self.entries[i];
                let k: Vec<char> = e.keys[0].chars().collect();
                let n = k.len();
                if n == 0 || at + n > chars.len() || e.term.pos != Pos::Any {
                    continue;
                }
                let hay = if e.term.case {
                    &chars[at..at + n]
                } else {
                    &lower[at..at + n]
                };
                if hay == k.as_slice() && best.is_none_or(|(_, bn)| n > bn) {
                    best = Some((i, n));
                }
            }
            let (i, n) = best?;
            out.push((i, chars[at..at + n].iter().collect()));
            at += n;
        }
        Some(out)
    }

    /// Cover a text with terms (R-B15-03).
    pub fn cover(&self, text: &str) -> Cover {
        let toks = tokens(text);
        let solid_ix: Vec<usize> = (0..toks.len())
            .filter(|&i| toks[i].kind != Kind::Space)
            .collect();
        let solid: Vec<&Token> = solid_ix.iter().map(|&i| &toks[i]).collect();
        let words: Vec<usize> = (0..solid.len())
            .filter(|&s| solid[s].kind == Kind::Word)
            .collect();
        let (first_word, last_word) = (
            words.first().copied().unwrap_or(usize::MAX),
            words.last().copied().unwrap_or(usize::MAX),
        );
        let mut pieces = Vec::new();
        let mut s = 0;
        let mut t = 0;
        while t < toks.len() {
            if toks[t].kind == Kind::Space {
                pieces.push(Piece::Space(toks[t].text.clone()));
                t += 1;
                continue;
            }
            if let Some((i, n)) = self.longest_at(&solid, s, first_word, last_word) {
                let end = solid_ix[s + n - 1];
                let source: String = toks[t..=end].iter().map(|k| k.text.as_str()).collect();
                pieces.push(Piece::Term { entry: i, source });
                s += n;
                t = end + 1;
                continue;
            }
            let tk = &toks[t];
            match tk.kind {
                Kind::Word => match self.split_word(&tk.text) {
                    Some(parts) => {
                        for (i, source) in parts {
                            pieces.push(Piece::Term { entry: i, source });
                        }
                    }
                    None => pieces.push(Piece::Word(tk.text.clone())),
                },
                _ => pieces.push(Piece::Lit(tk.text.clone())),
            }
            s += 1;
            t += 1;
        }
        Cover { pieces }
    }

    /// The terms that cover parts of `text`, in reading order, each once
    /// (the Vocabulaire tab, R-B15-04).
    pub fn terms_in(&self, text: &str) -> Vec<&Entry> {
        let mut out: Vec<&Entry> = Vec::new();
        for p in self.cover(text).pieces {
            if let Piece::Term { entry, .. } = p {
                let e = &self.entries[entry];
                if !out.iter().any(|o| std::ptr::eq(*o, e)) {
                    out.push(e);
                }
            }
        }
        out
    }
}

/// A piece of a covered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// A term (its index in the glossary) and the words it covers.
    Term {
        entry: usize,
        source: String,
    },
    /// A word no term covers.
    Word(String),
    /// A tag, a number or punctuation: copied.
    Lit(String),
    Space(String),
}

/// A text cut into terms and the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cover {
    pub pieces: Vec<Piece>,
}

impl Cover {
    /// The words no term covers.
    pub fn uncovered(&self) -> Vec<&str> {
        self.pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Word(w) => Some(w.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The glossary indexes of the terms, in source order.
    pub fn terms(&self) -> Vec<usize> {
        self.pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Term { entry, .. } => Some(*entry),
                _ => None,
            })
            .collect()
    }

    /// Every word covered, and at least one term.
    pub fn complete(&self) -> bool {
        self.uncovered().is_empty() && !self.terms().is_empty()
    }
}
