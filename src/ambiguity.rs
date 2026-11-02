//! Ambiguous sources (B-08): a source text the active dictionaries
//! translate in several ways. A line filled at a text level by such a
//! source is marked (its provenance ends with `|amb:<n>`), left En
//! attente, and its candidates are offered; the mark goes when the user
//! decides (typing, Utiliser, Validé or Personnalisé).

use crate::basedb::{BaseDb, FillLevel};
use crate::dicts::Role;
use crate::provenance::split_ambiguous;
use crate::tm::TmEntry;

/// One candidate translation of a source: the text and, per dictionary
/// that has it, the file name, its role and the number of rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub trans: String,
    pub dicts: Vec<(String, Role, u32)>,
}

impl Candidate {
    /// Rows across the dictionaries.
    pub fn count(&self) -> u32 {
        self.dicts.iter().map(|(_, _, n)| n).sum()
    }

    /// The provenance of a line that takes this candidate: its first
    /// dictionary, at the text level.
    pub fn prov(&self) -> String {
        match self.dicts.first() {
            Some((file, role, _)) => crate::provenance::dict(*role, FillLevel::Text, file),
            None => String::new(),
        }
    }
}

/// Whether a fill level can be ambiguous: the levels that identify the
/// record never are (R-B08-01).
pub fn level_can_be(level: FillLevel) -> bool {
    matches!(level, FillLevel::Text | FillLevel::TextOnly)
}

/// The distinct translations of `source` across `levels` (file name,
/// dictionary), in stack order then by number of rows; two translations
/// are the same once trimmed.
pub fn candidates<'a>(
    source: &str,
    levels: impl IntoIterator<Item = (&'a str, Role, &'a BaseDb)>,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for (name, role, db) in levels {
        for (t, n) in db.text_candidates(source) {
            match out.iter_mut().find(|c| c.trans.trim() == t.trim()) {
                Some(c) => c.dicts.push((name.to_string(), role, *n)),
                None => out.push(Candidate {
                    trans: t.clone(),
                    dicts: vec![(name.to_string(), role, *n)],
                }),
            }
        }
    }
    out
}

/// Whether a personal dictionary among `levels` knows `source`: the
/// user's own choice, never ambiguous (R-B08-03).
pub fn personal_decides<'a>(
    source: &str,
    levels: impl IntoIterator<Item = (Role, &'a BaseDb)>,
) -> bool {
    levels
        .into_iter()
        .any(|(role, db)| role == Role::Personal && !db.text_candidates(source).is_empty())
}

/// The provenance code of a line left ambiguous among `n` candidates.
pub fn mark(prov: &str, n: usize) -> String {
    format!("{}|amb:{n}", split_ambiguous(prov).0)
}

/// The number of candidates a line is marked with, `None` when it is
/// not ambiguous.
pub fn count_of(e: &TmEntry) -> Option<u32> {
    if e.trans.is_empty() {
        return None;
    }
    split_ambiguous(&e.prov).1
}

pub fn is_marked(e: &TmEntry) -> bool {
    count_of(e).is_some()
}

/// The provenance without its mark.
pub fn strip(prov: &str) -> String {
    split_ambiguous(prov).0.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basedb::BaseRow;

    fn row(rec: &str, field: &str, src: &str, trans: &str) -> BaseRow {
        BaseRow {
            rec: rec.into(),
            field: field.into(),
            source: src.into(),
            trans: trans.into(),
            ..Default::default()
        }
    }

    #[test]
    fn candidates_merge_across_dictionaries_and_within_one() {
        let mut a = BaseDb::default();
        a.add_rows(&[
            row("MISC", "FULL", "Key", "Clé"),
            row("KEYM", "FULL", "Key", "Clé "),
            row("MESG", "DESC", "Key", "Touche"),
        ]);
        let mut b = BaseDb::default();
        b.add_rows(&[
            row("MISC", "FULL", "Key", "Touche"),
            row("", "", "Key", "Clef"),
        ]);
        let g = Role::Game;
        let c = candidates("Key", [("a.ximdb", g, &a), ("b.ximdb", g, &b)]);
        let texts: Vec<&str> = c.iter().map(|c| c.trans.trim()).collect();
        assert_eq!(texts, vec!["Clé", "Touche", "Clef"]);
        assert_eq!(c[0].count(), 2);
        assert_eq!(
            c[1].dicts,
            vec![("a.ximdb".to_string(), g, 1), ("b.ximdb".to_string(), g, 1)]
        );
        assert_eq!(c[2].prov(), "dict:game:text:b.ximdb");
        // Two translations of the same key in one dictionary: both kept.
        let mut d = BaseDb::default();
        d.add_rows(&[
            row("MISC", "FULL", "Key", "Clé"),
            row("MISC", "FULL", "Key", "Touche"),
        ]);
        assert_eq!(candidates("Key", [("d", g, &d)]).len(), 2);
        assert_eq!(candidates("Door", [("d", g, &d)]).len(), 0);
        assert!(level_can_be(FillLevel::Text));
        assert!(!level_can_be(FillLevel::Exact));
        assert_eq!(
            mark("dict:game:text:a.ximdb", 2),
            "dict:game:text:a.ximdb|amb:2"
        );
        assert_eq!(strip(&mark("typed", 3)), "typed");
    }
}
