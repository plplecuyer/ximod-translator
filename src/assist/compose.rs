//! The composition of the glossary's terms (R-B16-02, decision D-81),
//! needed by B-15: a text the glossary covers entirely is rewritten in the
//! order of the target — the terms ranked by class (`composition.order` of
//! the pair's patterns file), source order within a class, free terms in
//! their place — the noun giving the gender that adjectives and colours
//! take, the terms joined by `joiner` (a space in French, nothing in
//! Chinese). Without a noun the text is not composed: the terms are
//! assembled in the source order.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::glossary::{Class, Gender, Glossary, Piece};

fn space() -> String {
    " ".to_string()
}

fn sixty() -> usize {
    60
}

/// Terms of one class that follow each other are merged: « en ébonite »
/// and « en os de dragon » give « en ébonite et os de dragon ».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merge {
    /// Taken off the terms after the first.
    #[serde(default)]
    pub prefix: String,
    /// Between the merged terms.
    #[serde(rename = "and")]
    pub and: String,
}

/// The `composition` section of a patterns file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composition {
    /// The classes in the order of the target; empty keeps the source order.
    #[serde(default)]
    pub order: Vec<Class>,
    #[serde(default = "space")]
    pub joiner: String,
    /// Longer texts (characters) are not composed.
    #[serde(default = "sixty", rename = "maxLen")]
    pub max_len: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub merge: BTreeMap<Class, Merge>,
}

impl Default for Composition {
    fn default() -> Self {
        Composition {
            order: Vec::new(),
            joiner: space(),
            max_len: sixty(),
            merge: BTreeMap::new(),
        }
    }
}

/// The composition of a pair: the user's patterns file, else the shipped
/// one; `None` when the pair has none.
pub fn load(shipped: &std::path::Path, user: Option<&PathBuf>, pair: &str) -> Option<Composition> {
    super::patterns::load_file(shipped, user, pair).and_then(|(f, _)| f.composition)
}

/// A translation by the glossary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub text: String,
    /// By the composition (`rule:compose`), else assembled in the source
    /// order (`rule:glossary`).
    pub composed: bool,
    /// The classes in the order of the result (the path of the essay).
    pub classes: Vec<Class>,
    /// The gender of the noun, when one was found.
    pub gender: Option<Gender>,
    /// Why the composition was not used, when it was tried.
    pub not_composed: Option<NotComposed>,
    /// The term opening the result keeps its case (a proper name): the
    /// result is not lowered when a pattern places it inside a text.
    pub case_kept: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotComposed {
    /// No term of class noun (`patterns-no-noun`).
    NoNoun,
    /// Longer than `maxLen`.
    TooLong,
    /// A tag, a number or punctuation among the terms.
    Literal,
}

pub(crate) fn first_upper(s: &str) -> bool {
    s.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| c.is_uppercase())
}

pub(crate) fn capitalize(s: &str) -> String {
    match s.char_indices().find(|(_, c)| c.is_alphabetic()) {
        Some((i, c)) => {
            let mut r = String::with_capacity(s.len());
            r.push_str(&s[..i]);
            r.extend(c.to_uppercase());
            r.push_str(&s[i + c.len_utf8()..]);
            r
        }
        None => s.to_string(),
    }
}

/// A term placed after the first one loses the capital of a word that
/// opens a text (« Épée » → « épée »), unless its case is kept or it looks
/// like an acronym (two capitals).
pub(crate) fn inner(text: &str, case: bool) -> String {
    let mut it = text.chars();
    match (it.next(), it.next()) {
        (Some(a), Some(b)) if !case && a.is_uppercase() && b.is_lowercase() => {
            let mut r: String = a.to_lowercase().collect();
            r.push_str(&text[a.len_utf8()..]);
            r
        }
        _ => text.to_string(),
    }
}

/// The separators a composition drops between terms (« Hood - Black »,
/// « Bandana, Tied »).
fn is_separator(s: &str) -> bool {
    matches!(s, "," | "-" | "–" | "—")
}

/// Translate a covered text (R-B15-03): `None` unless every word is
/// covered. With a composition, the text is composed when it can be.
pub fn translate(
    glossary: &Glossary,
    cover: &super::glossary::Cover,
    comp: Option<&Composition>,
    source: &str,
) -> Option<Translation> {
    if !cover.complete() {
        return None;
    }
    let terms = cover.terms();
    let gender = terms
        .iter()
        .rev()
        .map(|&i| &glossary.entry(i).term)
        .find(|t| t.class == Class::Noun)
        .map(|t| t.gender.unwrap_or(Gender::M));
    let has_noun = gender.is_some();
    let mut not_composed = None;
    if let Some(c) = comp {
        match compose(glossary, cover, c, source, gender) {
            Ok(t) => return Some(t),
            Err(why) => not_composed = Some(why),
        }
    }
    // Assembled in the source order.
    let joiner = comp.map(|c| c.joiner.as_str());
    let mut out = String::new();
    let mut opener_case = None;
    let mut last_term = false;
    let mut pending_space: Option<&str> = None;
    for p in &cover.pieces {
        match p {
            Piece::Space(s) => {
                if pending_space.is_none() {
                    pending_space = Some(s);
                }
            }
            Piece::Term { entry, .. } => {
                let t = &glossary.entry(*entry).term;
                let text = t.text(gender);
                if text.is_empty() {
                    continue;
                }
                if opener_case.is_none() {
                    opener_case = Some(t.case);
                }
                if let Some(s) = pending_space.take()
                    && !out.is_empty()
                {
                    out.push_str(if last_term { joiner.unwrap_or(s) } else { s });
                }
                if out.is_empty() {
                    out.push_str(text);
                } else {
                    out.push_str(&inner(text, t.case));
                }
                last_term = true;
            }
            Piece::Lit(s) | Piece::Word(s) => {
                if let Some(sp) = pending_space.take()
                    && !out.is_empty()
                {
                    out.push_str(sp);
                }
                out.push_str(s);
                last_term = false;
            }
        }
    }
    if out.trim().is_empty() {
        return None;
    }
    if first_upper(source) && opener_case == Some(false) {
        out = capitalize(&out);
    }
    Some(Translation {
        text: out,
        composed: false,
        classes: terms
            .iter()
            .map(|&i| glossary.entry(i).term.class)
            .collect(),
        gender: if has_noun { gender } else { None },
        not_composed,
        case_kept: opener_case.unwrap_or(false),
    })
}

fn compose(
    glossary: &Glossary,
    cover: &super::glossary::Cover,
    c: &Composition,
    source: &str,
    gender: Option<Gender>,
) -> Result<Translation, NotComposed> {
    if source.trim().chars().count() > c.max_len {
        return Err(NotComposed::TooLong);
    }
    let Some(gender) = gender else {
        return Err(NotComposed::NoNoun);
    };
    let pieces = &cover.pieces;
    let first = pieces
        .iter()
        .position(|p| matches!(p, Piece::Term { .. }))
        .ok_or(NotComposed::NoNoun)?;
    let last = pieces
        .iter()
        .rposition(|p| matches!(p, Piece::Term { .. }))
        .ok_or(NotComposed::NoNoun)?;
    let text_of = |p: &Piece| match p {
        Piece::Space(s) | Piece::Lit(s) | Piece::Word(s) => s.clone(),
        Piece::Term { source, .. } => source.clone(),
    };
    let prefix: String = pieces[..first].iter().map(text_of).collect();
    let suffix: String = pieces[last + 1..].iter().map(text_of).collect();
    let mut slots: Vec<usize> = Vec::new();
    for p in &pieces[first..=last] {
        match p {
            Piece::Term { entry, .. } => slots.push(*entry),
            Piece::Space(_) => {}
            Piece::Lit(s) if is_separator(s) => {}
            _ => return Err(NotComposed::Literal),
        }
    }
    // The classed terms ranked by class, in the places classed terms had;
    // free terms stay where they are.
    let rank = |cl: Class| {
        c.order
            .iter()
            .position(|o| *o == cl)
            .unwrap_or(c.order.len())
    };
    let mut classed: Vec<usize> = slots
        .iter()
        .copied()
        .filter(|&i| glossary.entry(i).term.class != Class::Free)
        .collect();
    if !c.order.is_empty() {
        classed.sort_by_key(|&i| rank(glossary.entry(i).term.class));
    }
    let mut it = classed.into_iter();
    let ordered: Vec<usize> = slots
        .iter()
        .map(|&i| {
            if glossary.entry(i).term.class == Class::Free {
                i
            } else {
                it.next().unwrap_or(i)
            }
        })
        .collect();
    // The texts, agreed, with the merges of a class.
    let mut parts: Vec<String> = Vec::new();
    let mut classes: Vec<Class> = Vec::new();
    let mut prev: Option<Class> = None;
    let mut opener_case = None;
    for &i in &ordered {
        let t = &glossary.entry(i).term;
        let text = t.text(Some(gender)).trim();
        if text.is_empty() {
            continue;
        }
        if opener_case.is_none() {
            opener_case = Some(t.case);
        }
        if prev == Some(t.class)
            && let Some(m) = c.merge.get(&t.class)
            && let Some(last) = parts.last_mut()
        {
            let rest = text.strip_prefix(m.prefix.as_str()).unwrap_or(text);
            last.push_str(&m.and);
            last.push_str(rest);
            continue;
        }
        parts.push(if parts.is_empty() {
            text.to_string()
        } else {
            inner(text, t.case)
        });
        classes.push(t.class);
        prev = Some(t.class);
    }
    if parts.is_empty() {
        return Err(NotComposed::NoNoun);
    }
    let mut core = parts.join(&c.joiner);
    if first_upper(source) && opener_case == Some(false) && !prefix.chars().any(char::is_alphabetic)
    {
        core = capitalize(&core);
    }
    Ok(Translation {
        text: format!("{prefix}{core}{suffix}"),
        composed: true,
        classes,
        gender: Some(gender),
        not_composed: None,
        case_kept: opener_case.unwrap_or(false),
    })
}
