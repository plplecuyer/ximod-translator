//! Matrices de remplacement (B-17): named series of substitutions, kept by
//! target language in `assets/data/matrices/<lang>.json` (the user's copy
//! of the same name under `<config>/data/matrices/` wins), applied in one
//! go in the direct or the inverse direction. Pure functions; the editor,
//! the Rechercher et remplacer integration and the undo live in the UI.
//!
//! A rule is an A-11 rule (`find`, `replace`, `mode`, `case`, `whole`)
//! with an optional explicit `inverse`. A Normal or Extended rule inverts
//! by swapping its two texts; a Regex rule only by its explicit inverse.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::replace::{Matcher, Mode, Rule};
use crate::status::{self, Status};
use crate::tm::TmEntry;

/// The file format.
pub const FORMAT: u32 = 1;
/// The folder of the files, under the data folders.
pub const DIR: &str = "matrices";
/// The file of the matrices valid for every language.
pub const ALL: &str = "all";

/// The explicit inverse of a rule.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Inverse {
    pub find: String,
    #[serde(default)]
    pub replace: String,
}

/// One rule of a matrix.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MatrixRule {
    #[serde(flatten)]
    pub rule: Rule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<Inverse>,
}

/// A named series of rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Matrix {
    pub name: String,
    #[serde(default)]
    pub rules: Vec<MatrixRule>,
}

/// The file of one language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixFile {
    pub format: u32,
    #[serde(default)]
    pub matrices: Vec<Matrix>,
}

impl Default for MatrixFile {
    fn default() -> Self {
        MatrixFile {
            format: FORMAT,
            matrices: Vec::new(),
        }
    }
}

/// Which way a matrix is applied (R-B17-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Direct,
    Inverse,
}

/// Why a file or a matrix is refused (the editor words it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// The JSON cannot be read.
    Json(String),
    /// A format this version does not read.
    Format(u32),
    /// A matrix without a name.
    NoName(usize),
    /// Two matrices of the same name.
    DuplicateName(String),
    /// Rule `n` (1-based) of a matrix has an empty pattern.
    EmptyFind { matrix: String, rule: usize },
    /// Rule `n` (1-based) of a matrix does not compile (or its inverse).
    Regex {
        matrix: String,
        rule: usize,
        error: String,
    },
}

/// Where the matrices of a language were read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The user's copy under `<config>/data/matrices/`.
    User,
    /// The shipped file under `assets/data/matrices/`.
    Shipped,
    /// No file for this language.
    None,
}

impl MatrixRule {
    pub fn new(find: &str, replace: &str, mode: Mode) -> MatrixRule {
        MatrixRule {
            rule: Rule {
                find: find.to_string(),
                replace: replace.to_string(),
                mode,
                case_sensitive: false,
                whole: false,
            },
            inverse: None,
        }
    }

    /// The rule of the inverse direction: the explicit inverse when there
    /// is one, the two texts swapped for a Normal or Extended rule; `None`
    /// for a Regex rule without inverse and for a rule that deletes.
    pub fn inverse_rule(&self) -> Option<Rule> {
        if let Some(inv) = &self.inverse
            && !inv.find.is_empty()
        {
            return Some(Rule {
                find: inv.find.clone(),
                replace: inv.replace.clone(),
                mode: self.rule.mode,
                case_sensitive: self.rule.case_sensitive,
                whole: self.rule.whole && self.rule.mode != Mode::Regex,
            });
        }
        match self.rule.mode {
            Mode::Regex => None,
            Mode::Normal | Mode::Extended if self.rule.replace.is_empty() => None,
            Mode::Normal | Mode::Extended => Some(Rule {
                find: self.rule.replace.clone(),
                replace: self.rule.find.clone(),
                mode: self.rule.mode,
                case_sensitive: self.rule.case_sensitive,
                whole: self.rule.whole,
            }),
        }
    }

    /// Whether the inverse comes from the explicit field (the editor's
    /// « explicite ») rather than from swapping the texts.
    pub fn has_explicit_inverse(&self) -> bool {
        self.inverse.as_ref().is_some_and(|i| !i.find.is_empty())
    }
}

impl Matrix {
    /// The 0-based index of the first rule that has no inverse, which
    /// keeps the matrix to the direct direction.
    pub fn blocking_inverse(&self) -> Option<usize> {
        self.rules.iter().position(|r| r.inverse_rule().is_none())
    }

    /// The compiled rules of a direction, in application order (the
    /// inverse goes backwards). The error names the 1-based rule.
    pub fn compile(&self, dir: Direction) -> Result<Vec<Matcher>, (usize, String)> {
        let mut out = Vec::with_capacity(self.rules.len());
        match dir {
            Direction::Direct => {
                for (i, r) in self.rules.iter().enumerate() {
                    out.push(r.rule.compile().map_err(|e| (i + 1, e))?);
                }
            }
            Direction::Inverse => {
                for (i, r) in self.rules.iter().enumerate().rev() {
                    let inv = r.inverse_rule().ok_or_else(|| (i + 1, String::new()))?;
                    out.push(inv.compile().map_err(|e| (i + 1, e))?);
                }
            }
        }
        Ok(out)
    }

    /// A matrix made of the rules of an A-11 rules file.
    pub fn from_rules(name: &str, rules: &[Rule]) -> Matrix {
        Matrix {
            name: name.to_string(),
            rules: rules
                .iter()
                .map(|r| MatrixRule {
                    rule: r.clone(),
                    inverse: None,
                })
                .collect(),
        }
    }
}

/// The rules applied in order to a text: `(new text, occurrences)`.
pub fn apply_text(matchers: &[Matcher], text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut n = 0;
    for m in matchers {
        let (t, k) = m.replace_all(&out);
        if k > 0 {
            out = t;
            n += k;
        }
    }
    (out, n)
}

/// Whether a line may receive a matrix: a translation, not locked, shown.
pub fn applicable(e: &TmEntry) -> bool {
    !e.trans.is_empty() && !e.unused && status::derive(e) != Status::Ignored
}

/// What a matrix would change: `(line, new translation)` for the lines of
/// `lines` whose translation changes.
pub fn plan(entries: &[TmEntry], lines: &[usize], matchers: &[Matcher]) -> Vec<(usize, String)> {
    lines
        .iter()
        .filter_map(|&i| {
            let e = entries.get(i)?;
            if !applicable(e) {
                return None;
            }
            let (t, n) = apply_text(matchers, &e.trans);
            (n > 0 && t != e.trans).then_some((i, t))
        })
        .collect()
}

/// How many of `texts` a second application of the matrix still changes
/// (R-B17-04): 0 for an idempotent matrix.
pub fn not_idempotent(matchers: &[Matcher], texts: &[&str]) -> usize {
    texts
        .iter()
        .filter(|t| {
            let (once, _) = apply_text(matchers, t);
            let (twice, _) = apply_text(matchers, &once);
            twice != once
        })
        .count()
}

impl MatrixFile {
    pub fn parse(text: &str) -> Result<MatrixFile, Invalid> {
        let f: MatrixFile = serde_json::from_str(text).map_err(|e| Invalid::Json(e.to_string()))?;
        if f.format != FORMAT {
            return Err(Invalid::Format(f.format));
        }
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Check names and rules (R-B17-05): a file that fails is never saved.
    pub fn validate(&self) -> Result<(), Invalid> {
        let mut names: Vec<String> = Vec::new();
        for (k, m) in self.matrices.iter().enumerate() {
            let name = m.name.trim();
            if name.is_empty() {
                return Err(Invalid::NoName(k + 1));
            }
            let low = name.to_lowercase();
            if names.contains(&low) {
                return Err(Invalid::DuplicateName(name.to_string()));
            }
            names.push(low);
            for (i, r) in m.rules.iter().enumerate() {
                if r.rule.find.is_empty() {
                    return Err(Invalid::EmptyFind {
                        matrix: name.to_string(),
                        rule: i + 1,
                    });
                }
                let check = |rule: &Rule| {
                    rule.compile().map(|_| ()).map_err(|error| Invalid::Regex {
                        matrix: name.to_string(),
                        rule: i + 1,
                        error,
                    })
                };
                check(&r.rule)?;
                if let Some(inv) = r.inverse_rule() {
                    check(&inv)?;
                }
            }
        }
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&Matrix> {
        self.matrices.iter().find(|m| m.name == name)
    }
}

/// The shipped files, embedded: the fallback when the assets folder is
/// missing (tests, a moved executable).
fn embedded(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "fra" => include_str!("../assets/data/matrices/fra.json"),
        "deu" => include_str!("../assets/data/matrices/deu.json"),
        "spa" => include_str!("../assets/data/matrices/spa.json"),
        "eng" => include_str!("../assets/data/matrices/eng.json"),
        ALL => include_str!("../assets/data/matrices/all.json"),
        _ => return None,
    })
}

/// The file name of a language (`fra.json`, `all.json`).
pub fn file_name(lang: &str) -> String {
    format!("{}.json", lang.to_ascii_lowercase())
}

/// The folders the matrices are read from: `(shipped, user)`.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub shipped: PathBuf,
    pub user: Option<PathBuf>,
}

impl Dirs {
    /// The folders of the installation.
    pub fn installed() -> Dirs {
        let p = crate::config::paths::get();
        Dirs {
            shipped: crate::config::assets_data_dir().join(DIR),
            user: Some(p.data_dir.join(DIR)),
        }
    }

    pub fn user_path(&self, lang: &str) -> Option<PathBuf> {
        self.user.as_ref().map(|u| u.join(file_name(lang)))
    }

    pub fn shipped_path(&self, lang: &str) -> PathBuf {
        self.shipped.join(file_name(lang))
    }

    /// The shipped matrices of a language (the file, else the embedded
    /// copy), empty when there are none.
    pub fn shipped(&self, lang: &str) -> MatrixFile {
        let text = std::fs::read_to_string(self.shipped_path(lang))
            .ok()
            .or_else(|| embedded(lang).map(str::to_string));
        text.and_then(|t| MatrixFile::parse(&t).ok())
            .unwrap_or_default()
    }

    /// The matrices of a language: the user's copy when present and
    /// valid, else the shipped ones; with where they came from.
    pub fn load(&self, lang: &str) -> (MatrixFile, Origin) {
        if let Some(p) = self.user_path(lang)
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            match MatrixFile::parse(&text) {
                Ok(f) if f.validate().is_ok() => return (f, Origin::User),
                Ok(_) => tracing::warn!("{}: invalid matrices, shipped file used", p.display()),
                Err(e) => tracing::warn!("{}: {e:?}", p.display()),
            }
        }
        let shipped_exists =
            self.shipped_path(lang).is_file() || embedded(&lang.to_ascii_lowercase()).is_some();
        let f = self.shipped(lang);
        let origin = if shipped_exists {
            Origin::Shipped
        } else {
            Origin::None
        };
        (f, origin)
    }

    /// Save the user's copy of a language, after validation.
    pub fn save(&self, lang: &str, file: &MatrixFile) -> Result<PathBuf, Invalid> {
        file.validate()?;
        let p = self
            .user_path(lang)
            .ok_or_else(|| Invalid::Json(String::new()))?;
        crate::config::paths::write_atomic(&p, file.to_json().as_bytes())
            .map_err(|e| Invalid::Json(e.to_string()))?;
        Ok(p)
    }

    /// Remove the user's copy: the shipped matrices apply again.
    pub fn restore(&self, lang: &str) -> std::io::Result<()> {
        match self.user_path(lang) {
            Some(p) if p.is_file() => std::fs::remove_file(p),
            _ => Ok(()),
        }
    }

    /// The languages that have matrices (shipped or user), `all` first.
    pub fn languages(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |dir: &Path| {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("json")
                        && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                    {
                        let stem = stem.to_ascii_lowercase();
                        if !out.contains(&stem) {
                            out.push(stem);
                        }
                    }
                }
            }
        };
        add(&self.shipped);
        if let Some(u) = &self.user {
            add(u);
        }
        for l in ["fra", "deu", "spa", "eng"] {
            if !out.iter().any(|o| o == l) {
                out.push(l.to_string());
            }
        }
        out.retain(|l| l != ALL);
        out.sort();
        out.insert(0, ALL.to_string());
        out
    }

    /// The matrices offered for a target language: its own, then the
    /// common ones; `(file language, matrix)`.
    pub fn offered(&self, target: &str) -> Vec<(String, Matrix)> {
        let mut out = Vec::new();
        let t = target.trim().to_ascii_lowercase();
        if !t.is_empty() && t != ALL {
            for m in self.load(&t).0.matrices {
                out.push((t.clone(), m));
            }
        }
        for m in self.load(ALL).0.matrices {
            out.push((ALL.to_string(), m));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(trans: &str) -> TmEntry {
        let mut e = TmEntry {
            source: "src".into(),
            trans: trans.into(),
            ..Default::default()
        };
        e.set_translated_flags(true);
        e
    }

    #[test]
    fn inverse_of_each_mode() {
        let n = MatrixRule::new("...", "…", Mode::Normal);
        assert_eq!(n.inverse_rule().unwrap().find, "…");
        let del = MatrixRule::new("  ", "", Mode::Normal);
        assert!(del.inverse_rule().is_none(), "a deletion has no inverse");
        let mut re = MatrixRule::new("\"([^\"]+)\"", "« $1 »", Mode::Regex);
        assert!(re.inverse_rule().is_none());
        re.inverse = Some(Inverse {
            find: "« ([^»]+) »".into(),
            replace: "\"$1\"".into(),
        });
        assert!(re.has_explicit_inverse());
        let m = Matrix {
            name: "q".into(),
            rules: vec![n, re],
        };
        assert!(m.blocking_inverse().is_none());
        let d = m.compile(Direction::Direct).unwrap();
        let i = m.compile(Direction::Inverse).unwrap();
        let (t, _) = apply_text(&d, "Il dit \"oui\"...");
        assert_eq!(t, "Il dit « oui »…");
        assert_eq!(apply_text(&i, &t).0, "Il dit \"oui\"...");
    }

    #[test]
    fn plan_skips_locked_and_empty() {
        let mut locked = entry("a...");
        status::set(&mut locked, Status::Ignored);
        let es = vec![entry("a..."), locked, entry(""), entry("b")];
        let m = Matrix {
            name: "x".into(),
            rules: vec![MatrixRule::new("...", "…", Mode::Normal)],
        };
        let c = m.compile(Direction::Direct).unwrap();
        let p = plan(&es, &[0, 1, 2, 3], &c);
        assert_eq!(p, vec![(0, "a…".to_string())]);
    }

    #[test]
    fn validation_names_the_rule() {
        let f = MatrixFile {
            format: FORMAT,
            matrices: vec![Matrix {
                name: "Bad".into(),
                rules: vec![
                    MatrixRule::new("a", "b", Mode::Normal),
                    MatrixRule::new("(x", "y", Mode::Regex),
                ],
            }],
        };
        match f.validate() {
            Err(Invalid::Regex { rule, .. }) => assert_eq!(rule, 2),
            other => panic!("{other:?}"),
        }
        let dup = MatrixFile {
            format: FORMAT,
            matrices: vec![
                Matrix {
                    name: "A".into(),
                    rules: vec![],
                },
                Matrix {
                    name: "a".into(),
                    rules: vec![],
                },
            ],
        };
        assert!(matches!(dup.validate(), Err(Invalid::DuplicateName(_))));
    }

    #[test]
    fn shipped_files_are_valid_and_idempotent() {
        for lang in ["fra", "deu", "spa", "eng", ALL] {
            let f = MatrixFile::parse(embedded(lang).unwrap()).unwrap();
            f.validate().unwrap();
            for m in &f.matrices {
                let c = m.compile(Direction::Direct).unwrap();
                let samples = [
                    "Bonjour ! Qui êtes-vous ? Attention : « danger ».",
                    "Il dit \"oui\"... puis s'en va ; voilà !",
                    "<font face='$MAIN_Font'>Prix : 10:30</font>",
                    "Quoi ?! Vraiment ?",
                    "¿Qué? ¡Hola! Bien.",
                ];
                assert_eq!(not_idempotent(&c, &samples), 0, "{lang}: {}", m.name);
            }
        }
    }
}
