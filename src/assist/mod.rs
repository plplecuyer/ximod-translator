//! Assistants hors ligne par règles (étape 2, lot 4): the glossary (B-15)
//! and the composition of its terms; the patterns (B-16) and the
//! derivations (B-21) join them. They apply through « Appliquer les
//! assistants… », never at opening (decision D-80): a line is filled only
//! when its text is entirely covered, En attente, with the provenance
//! `rule:compose`, `rule:pattern:<n>` or `rule:glossary` (decision D-77);
//! locked and Ignoré lines are never touched.

pub mod compose;
pub mod derive;
pub mod glossary;
pub mod patterns;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::apply::Overwrite;
use crate::status::{self, Status};
use crate::tm::TmEntry;

pub use compose::{Composition, NotComposed, Translation};
pub use glossary::{Class, Cover, Gender, Glossary, GlossaryFile, Pos, Term};
pub use patterns::{Bases, How, NoBases, Patterns, PatternsFile, StackBases, Step};

/// The glossary and the composition of a document (its pair, its game).
#[derive(Debug, Clone, Default)]
pub struct Assistant {
    pub glossary: Glossary,
    pub composition: Option<Composition>,
    /// The patterns of the pair (B-16).
    pub patterns: Option<Arc<Patterns>>,
    /// The derivations of the game (B-21).
    pub derive: Option<Arc<derive::DeriveFile>>,
    /// `ENG-FRA`.
    pub pair: String,
    /// `SkyrimSE`.
    pub game: String,
}

/// Why a text gets no translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No term in the text.
    NoTerm,
    /// Words no term covers.
    Partial(Vec<String>),
}

impl Assistant {
    /// The files of the installation: the game's glossary first, then the
    /// pair's; the pair's composition.
    pub fn load(pair: &str, game: &str) -> Assistant {
        Assistant::load_from(&glossary::Dirs::installed(), pair, game)
    }

    pub fn load_from(dirs: &glossary::Dirs, pair: &str, game: &str) -> Assistant {
        let (pair_file, _, _) = dirs.load(&glossary::file_name(pair, None));
        let (game_file, _, _) = dirs.load(&glossary::file_name(pair, Some(game)));
        let glossary = Glossary::new(&[(&game_file, true), (&pair_file, false)]);
        let file = patterns::load_file(&dirs.shipped, dirs.user.as_ref(), pair).map(|(f, _)| f);
        let composition = file.as_ref().and_then(|f| f.composition.clone());
        let patterns = file.map(|f| Arc::new(Patterns::new(f)));
        let derive =
            derive::load_file(&dirs.shipped, dirs.user.as_ref(), game).map(|(f, _)| Arc::new(f));
        Assistant {
            glossary,
            composition,
            patterns,
            derive,
            pair: pair.to_string(),
            game: game.to_string(),
        }
    }

    pub fn new(glossary: Glossary, composition: Option<Composition>) -> Assistant {
        Assistant {
            glossary,
            composition,
            ..Default::default()
        }
    }

    /// The translation of a text by the glossary, composed when `compose`.
    pub fn translate(&self, text: &str, compose: bool) -> Result<Translation, Refusal> {
        let cover = self.glossary.cover(text);
        if cover.terms().is_empty() {
            return Err(Refusal::NoTerm);
        }
        let un = cover.uncovered();
        if !un.is_empty() {
            return Err(Refusal::Partial(
                un.into_iter().map(str::to_string).collect(),
            ));
        }
        let comp = if compose {
            self.composition.as_ref()
        } else {
            None
        };
        compose::translate(&self.glossary, &cover, comp, text).ok_or(Refusal::NoTerm)
    }

    /// The whole chain of an application (R-B16-06): the composition, then
    /// the patterns in their order, then the glossary alone in the source
    /// order.
    pub fn run(
        &self,
        text: &str,
        use_glossary: bool,
        use_patterns: bool,
        bases: &dyn Bases,
    ) -> Result<Outcome, Refusal> {
        let cover = self.glossary.cover(text);
        let complete = cover.complete();
        if use_patterns {
            if complete
                && let Some(t) =
                    compose::translate(&self.glossary, &cover, self.composition.as_ref(), text)
                && t.composed
            {
                return Ok(Outcome {
                    text: t.text.clone(),
                    rule: Rule::Compose,
                    steps: Vec::new(),
                    translation: Some(t),
                });
            }
            if let Some(p) = &self.patterns
                && let Some(m) = p.translate(text, &self.glossary, self.composition.as_ref(), bases)
            {
                return Ok(Outcome {
                    text: m.text,
                    rule: Rule::Pattern(m.n),
                    steps: m.steps,
                    translation: None,
                });
            }
        }
        if use_glossary
            && complete
            && let Some(t) = compose::translate(&self.glossary, &cover, None, text)
        {
            return Ok(Outcome {
                text: t.text.clone(),
                rule: Rule::Glossary,
                steps: Vec::new(),
                translation: Some(t),
            });
        }
        let un = cover.uncovered();
        if cover.terms().is_empty() {
            Err(Refusal::NoTerm)
        } else {
            Err(Refusal::Partial(
                un.into_iter().map(str::to_string).collect(),
            ))
        }
    }
}

/// Which assistant translated a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// The composition of the glossary's terms (« motif 1 »).
    Compose,
    /// A pattern, by its number (from 2).
    Pattern(usize),
    /// The glossary alone, in the source order.
    Glossary,
    /// A derivation from a parent (B-21): the provenance names the rule
    /// and the parent.
    Derive,
}

impl Rule {
    /// The provenance code of the line (B-09).
    pub fn prov(self) -> String {
        match self {
            Rule::Compose => crate::provenance::RULE_COMPOSE.to_string(),
            Rule::Pattern(n) => format!("{}{n}", crate::provenance::RULE_PATTERN),
            Rule::Glossary => crate::provenance::RULE_GLOSSARY.to_string(),
            Rule::Derive => crate::provenance::RULE_DERIVE.to_string(),
        }
    }
}

/// A text translated by an assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    pub rule: Rule,
    /// The variables of a pattern, resolved (the path of the essay).
    pub steps: Vec<Step>,
    /// The glossary's translation (composition or glossary alone).
    pub translation: Option<Translation>,
}

/// The settings of an application (R-B15-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub glossary: bool,
    /// The patterns: the composition by classes (the implicit first one)
    /// and those of the file.
    pub patterns: bool,
    /// The derivations of the game (B-21).
    pub derive: bool,
    pub overwrite: Overwrite,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            glossary: true,
            patterns: true,
            derive: true,
            overwrite: Overwrite::None,
        }
    }
}

/// One line the application would fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub idx: usize,
    pub trans: String,
    pub rule: Rule,
    /// The provenance code the line gets.
    pub prov: String,
}

/// What an application would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Lines assembled in the source order (`rule:glossary`).
    pub glossary: usize,
    /// Lines composed (`rule:compose`).
    pub composed: usize,
    /// Lines per pattern of the file (`rule:pattern:<n>`).
    pub per_pattern: BTreeMap<usize, usize>,
    /// Lines derived, per rule of derivation (B-21).
    pub derived: BTreeMap<String, usize>,
    /// Lines a derivation rule matched whose parent has no translation.
    pub untranslated: usize,
    /// Lines a derivation rule matched whose parent was not found.
    pub not_found: usize,
    /// Derived lines whose parent has several translations (B-08).
    pub ambiguous: usize,
    /// Lines with terms and words no term covers: left as they are.
    pub partial: usize,
    /// Lines that had a translation and get another one.
    pub overwritten: usize,
    /// Locked (B-18) or Ignoré lines of the scope, never touched.
    pub locked: usize,
}

impl Plan {
    pub fn filled(&self) -> usize {
        self.changes.len()
    }

    /// Lines filled by the patterns of the file.
    pub fn patterned(&self) -> usize {
        self.per_pattern.values().sum()
    }

    /// Lines filled by the derivations.
    pub fn derived_n(&self) -> usize {
        self.derived.values().sum()
    }
}

/// What the derivations need besides the document (B-21).
pub struct DeriveCtx<'a> {
    pub file: &'a derive::DeriveFile,
    /// The target language (ISO 639-3).
    pub lang: &'a str,
    /// The entries of the other open files (same pair).
    pub open: Vec<&'a [TmEntry]>,
    pub parent_bases: &'a dyn derive::ParentBases,
    /// book FormID → spell FormID, for the tomes.
    pub teaches: &'a HashMap<u32, u32>,
}

/// Whether a line may receive a translation (and is not locked).
fn eligible(e: &TmEntry, opts: &Options, p: &mut Plan) -> Option<(bool, Status)> {
    if e.unused {
        return None;
    }
    let st = status::derive(e);
    if st == Status::Ignored {
        p.locked += 1;
        return None;
    }
    let has = !e.trans.is_empty();
    if has && !opts.overwrite.allows(st) {
        return None;
    }
    if !e.source.chars().any(char::is_alphabetic) {
        return None;
    }
    Some((has, st))
}

/// Compute what the glossary and the patterns would fill among `lines`.
pub fn plan(
    entries: &[TmEntry],
    lines: &[usize],
    a: &Assistant,
    opts: &Options,
    bases: &dyn Bases,
) -> Plan {
    plan_all(entries, lines, a, opts, bases, None)
}

/// Compute what the assistants would fill among `lines`: the glossary and
/// the patterns first, then the derivations, whose parents include the
/// lines just planned (R-B21-05).
pub fn plan_all(
    entries: &[TmEntry],
    lines: &[usize],
    a: &Assistant,
    opts: &Options,
    bases: &dyn Bases,
    derive: Option<&DeriveCtx>,
) -> Plan {
    let mut p = Plan::default();
    let mut partial: HashSet<usize> = HashSet::new();
    if opts.glossary || opts.patterns {
        let mut cache: HashMap<&str, Result<Outcome, Refusal>> = HashMap::new();
        for &i in lines {
            let Some(e) = entries.get(i) else { continue };
            let Some((has, st)) = eligible(e, opts, &mut p) else {
                continue;
            };
            let r = cache
                .entry(e.source.as_str())
                .or_insert_with(|| a.run(&e.source, opts.glossary, opts.patterns, bases));
            match r {
                Ok(t) => {
                    if has && e.trans == t.text && st == Status::Pending {
                        continue;
                    }
                    if has {
                        p.overwritten += 1;
                    }
                    match t.rule {
                        Rule::Compose => p.composed += 1,
                        Rule::Glossary => p.glossary += 1,
                        Rule::Pattern(n) => *p.per_pattern.entry(n).or_default() += 1,
                        Rule::Derive => {}
                    }
                    p.changes.push(Change {
                        idx: i,
                        trans: t.text.clone(),
                        rule: t.rule,
                        prov: t.rule.prov(),
                    });
                }
                Err(Refusal::Partial(_)) => {
                    p.partial += 1;
                    partial.insert(i);
                }
                Err(Refusal::NoTerm) => {}
            }
        }
    }
    if opts.derive
        && let Some(ctx) = derive
    {
        let planned: HashMap<usize, &str> = p
            .changes
            .iter()
            .map(|c| (c.idx, c.trans.as_str()))
            .collect();
        let sigs = derive::parent_sigs(ctx.file, ctx.lang);
        let mut parents = derive::Parents::default();
        parents.add(entries, derive::Origin::Document, &sigs, &|i, e| {
            planned
                .get(&i)
                .map(|t| t.to_string())
                .unwrap_or_else(|| e.trans.clone())
        });
        for other in &ctx.open {
            parents.add(other, derive::Origin::OpenFile, &sigs, &|_, e| {
                e.trans.clone()
            });
        }
        let d = derive::Deriver {
            file: ctx.file,
            lang: ctx.lang,
            parents: &parents,
            parent_bases: ctx.parent_bases,
            assistant: a,
            bases,
            teaches: ctx.teaches,
        };
        // A derived object takes its derivation rather than the pattern or
        // the glossary of the first pass: the derivation knows its parent,
        // strips it and follows it.
        let filled: HashMap<usize, usize> = p
            .changes
            .iter()
            .enumerate()
            .map(|(k, c)| (c.idx, k))
            .collect();
        let locked_before = p.locked;
        let mut replaced: Vec<(usize, Change)> = Vec::new();
        for &i in lines {
            let Some(e) = entries.get(i) else { continue };
            let first = filled.get(&i).copied();
            let (has, st) = match first {
                Some(_) => (!e.trans.is_empty(), status::derive(e)),
                None => match eligible(e, opts, &mut p) {
                    Some(x) => x,
                    None => continue,
                },
            };
            match d.derive(e, None) {
                Some(derive::Derived::Text {
                    text,
                    prov,
                    rule,
                    ambiguous,
                    ..
                }) => {
                    if first.is_none() && has && e.trans == text && st == Status::Pending {
                        continue;
                    }
                    if has && first.is_none() {
                        p.overwritten += 1;
                    }
                    if partial.remove(&i) {
                        p.partial -= 1;
                    }
                    *p.derived.entry(rule).or_default() += 1;
                    let prov = if ambiguous > 0 {
                        p.ambiguous += 1;
                        crate::ambiguity::mark(&prov, ambiguous)
                    } else {
                        prov
                    };
                    let c = Change {
                        idx: i,
                        trans: text,
                        rule: Rule::Derive,
                        prov,
                    };
                    match first {
                        Some(k) => replaced.push((k, c)),
                        None => p.changes.push(c),
                    }
                }
                Some(derive::Derived::Untranslated { .. }) if first.is_none() => {
                    p.untranslated += 1
                }
                Some(derive::Derived::NotFound { .. }) if first.is_none() => p.not_found += 1,
                _ => {}
            }
        }
        for (k, c) in replaced {
            match p.changes[k].rule {
                Rule::Compose => p.composed -= 1,
                Rule::Glossary => p.glossary -= 1,
                Rule::Pattern(n) => {
                    if let Some(x) = p.per_pattern.get_mut(&n) {
                        *x -= 1;
                        if *x == 0 {
                            p.per_pattern.remove(&n);
                        }
                    }
                }
                Rule::Derive => {}
            }
            p.changes[k] = c;
        }
        // The locked lines were counted by the first pass.
        if opts.glossary || opts.patterns {
            p.locked = locked_before;
        }
    }
    p
}

/// Write a plan into the entries: En attente, provenance « règle :
/// composition », « règle : motif <n> » or « règle : glossaire ». Returns the lines changed.
pub fn commit(entries: &mut [TmEntry], plan: &Plan) -> Vec<usize> {
    let mut touched = Vec::with_capacity(plan.changes.len());
    for c in &plan.changes {
        let Some(e) = entries.get_mut(c.idx) else {
            continue;
        };
        e.trans = c.trans.clone();
        e.prov = c.prov.clone();
        e.set_translated_flags(true);
        status::set(e, Status::Pending);
        touched.push(c.idx);
    }
    touched
}
