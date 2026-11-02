//! Fichier ▸ Enregistrer une partie… (B-10): the lines of a document chosen
//! by scope, status, record and field, written to a working format without
//! touching the document.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::status::{self, Status};
use crate::tm::{TmEntry, TranslationMemory};

/// The lines the dialog starts from (R-B10-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    Selection,
    View,
    #[default]
    All,
}

/// The choices of the dialog, kept for the session per file (R-B10-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    pub scope: Scope,
    pub statuses: [bool; status::COUNT],
    /// The records kept; `None`: all of them.
    pub records: Option<BTreeSet<String>>,
    /// The field kept; `None`: all of them.
    pub field: Option<String>,
    /// « Lignes sans traduction : conserver la source ».
    pub keep_source: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            scope: Scope::All,
            statuses: [true; status::COUNT],
            records: None,
            field: None,
            keep_source: false,
        }
    }
}

/// The lines written: the scope's lines (in document order) that show in
/// game and pass the statuses, the records and the field.
pub fn lines(entries: &[TmEntry], scope_lines: &[usize], c: &Choices) -> Vec<usize> {
    let mut out: Vec<usize> = scope_lines
        .iter()
        .copied()
        .filter(|&i| {
            let Some(e) = entries.get(i) else {
                return false;
            };
            !e.unused
                && c.statuses[status::derive(e).index()]
                && c.records.as_ref().is_none_or(|r| r.contains(&e.rec_sig))
                && c.field.as_ref().is_none_or(|f| *f == e.field_sig)
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The record signatures of the lines that show in game, the most
/// frequent first (the chips of the dialog).
pub fn records_of(entries: &[TmEntry]) -> Vec<(String, usize)> {
    count_by(
        entries
            .iter()
            .filter(|e| !e.unused)
            .map(|e| e.rec_sig.as_str()),
    )
}

/// The field signatures of the lines of the records kept.
pub fn fields_of(entries: &[TmEntry], records: Option<&BTreeSet<String>>) -> Vec<String> {
    let set: BTreeSet<&str> = entries
        .iter()
        .filter(|e| !e.unused && records.is_none_or(|r| r.contains(&e.rec_sig)))
        .map(|e| e.field_sig.as_str())
        .collect();
    set.into_iter().map(str::to_string).collect()
}

fn count_by<'a>(it: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut m: BTreeMap<&str, usize> = BTreeMap::new();
    for k in it {
        *m.entry(k).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// The preview's breakdown: by record (most first), by status (in the
/// order of the statuses).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakdown {
    pub records: Vec<(String, usize)>,
    pub statuses: Vec<(Status, usize)>,
}

pub fn breakdown(entries: &[TmEntry], lines: &[usize]) -> Breakdown {
    let records = count_by(lines.iter().map(|&i| entries[i].rec_sig.as_str()));
    let mut n = [0usize; status::COUNT];
    for &i in lines {
        n[status::derive(&entries[i]).index()] += 1;
    }
    let statuses = Status::ALL
        .iter()
        .filter(|s| n[s.index()] > 0)
        .map(|s| (*s, n[s.index()]))
        .collect();
    Breakdown { records, statuses }
}

/// Why a file name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A STRINGS file: a partial one does not load in game.
    Strings,
    /// An extension none of the working formats has.
    Unknown(String),
}

/// The formats of Enregistrer sous… but STRINGS (R-B10-02).
pub fn check_path(path: &Path) -> Result<(), Refusal> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if crate::tm::strings::kind_from_extension(&ext).is_some() {
        return Err(Refusal::Strings);
    }
    if crate::tm::output_format(path).is_none() {
        return Err(Refusal::Unknown(ext));
    }
    Ok(())
}

/// The file proposed: « <document>_<suffix>.json » in `dir`.
pub fn default_path(doc_path: &Path, dir: &Path, suffix: &str) -> PathBuf {
    let stem = doc_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mod");
    dir.join(format!("{stem}_{suffix}.json"))
}

/// The memory written: the document's metadata, the lines chosen (with an
/// empty translation replaced by the source when `keep_source`), and, for
/// the pivot JSON, the history of those lines (R-B10-02).
pub fn memory(
    tm: &TranslationMemory,
    lines: &[usize],
    keep_source: bool,
    history: &[crate::history::Entry],
) -> TranslationMemory {
    let entries: Vec<TmEntry> = lines
        .iter()
        .filter_map(|&i| tm.entries.get(i))
        .map(|e| {
            let mut e = e.clone();
            if keep_source && e.trans.is_empty() {
                e.trans = e.source.clone();
            }
            e
        })
        .collect();
    let keys: std::collections::HashSet<crate::history::Key> =
        entries.iter().map(crate::history::key_of).collect();
    TranslationMemory {
        meta: tm.meta.clone(),
        entries,
        history: history
            .iter()
            .filter(|h| keys.contains(&h.k))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(rec: &str, field: &str, src: &str, trans: &str, st: Status) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            source: src.into(),
            trans: trans.into(),
            ..Default::default()
        };
        e.set_translated_flags(!trans.is_empty());
        if !trans.is_empty() {
            status::set(&mut e, st);
        }
        e
    }

    #[test]
    fn lines_breakdown_and_refusals() {
        let mut v = vec![
            e(
                "ARMO",
                "FULL",
                "Iron Helmet",
                "Casque en fer",
                Status::Validated,
            ),
            e("WEAP", "FULL", "Iron Sword", "Épée en fer", Status::Pending),
            e("WEAP", "DESC", "Sharp.", "Tranchante.", Status::Validated),
            e("BOOK", "FULL", "Journal", "", Status::ToTranslate),
            e("ARMO", "FULL", "Unused", "Inutilisé", Status::Validated),
        ];
        v[4].unused = true;
        let all: Vec<usize> = (0..v.len()).collect();
        let mut c = Choices::default();
        assert_eq!(lines(&v, &all, &c), vec![0, 1, 2, 3]);
        c.statuses = [false; status::COUNT];
        c.statuses[Status::Validated.index()] = true;
        c.records = Some(["ARMO".to_string(), "WEAP".to_string()].into());
        assert_eq!(lines(&v, &all, &c), vec![0, 2]);
        c.field = Some("FULL".into());
        assert_eq!(lines(&v, &all, &c), vec![0]);
        let b = breakdown(&v, &[0, 1, 2]);
        assert_eq!(b.records, vec![("WEAP".into(), 2), ("ARMO".into(), 1)]);
        assert_eq!(
            b.statuses,
            vec![(Status::Validated, 2), (Status::Pending, 1)]
        );
        assert_eq!(
            records_of(&v),
            vec![("WEAP".into(), 2), ("ARMO".into(), 1), ("BOOK".into(), 1)]
        );
        assert_eq!(fields_of(&v, None), vec!["DESC", "FULL"]);
        assert_eq!(check_path(Path::new("a.strings")), Err(Refusal::Strings));
        assert_eq!(check_path(Path::new("a.ILSTRINGS")), Err(Refusal::Strings));
        assert_eq!(
            check_path(Path::new("a.txt")),
            Err(Refusal::Unknown("txt".into()))
        );
        assert!(check_path(Path::new("a.sst")).is_ok());
        assert!(check_path(Path::new("a.sst.xml")).is_ok());
        assert_eq!(
            default_path(Path::new("/m/MonMod.esp"), Path::new("/out"), "partie"),
            PathBuf::from("/out/MonMod_partie.json")
        );
        let tm = TranslationMemory {
            entries: v,
            ..Default::default()
        };
        let m = memory(&tm, &[0, 3], true, &[]);
        assert_eq!(m.entries.len(), 2);
        assert_eq!(m.entries[1].trans, "Journal");
        let m = memory(&tm, &[3], false, &[]);
        assert_eq!(m.entries[0].trans, "");
    }
}
