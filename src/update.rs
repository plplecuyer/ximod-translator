//! Mise à jour du mod (B-07): the document compared with its own
//! dictionary — current rows and old data — into lines added, removed,
//! changed (same record, text changed) and unchanged.

use std::collections::{HashMap, HashSet};

use crate::basedb::{BaseRow, OldRow, row_key};
use crate::tm::TmEntry;

type Key = (String, String, u32, String, String);
type IdKey = (String, String, u32, String);

/// A line whose text changed since the dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub idx: usize,
    pub old_source: String,
    pub old_trans: String,
    pub old_status: Option<String>,
}

/// A row of the dictionary the document no longer has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub row: BaseRow,
    /// Empty for a current row (it leaves at the next write).
    pub date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comparison {
    pub added: Vec<usize>,
    pub removed: Vec<Removed>,
    pub changed: Vec<Changed>,
    pub same: usize,
    /// Current rows of the dictionary.
    pub dict_rows: usize,
}

fn entry_key(e: &TmEntry) -> Key {
    (
        e.rec_sig.clone(),
        e.field_sig.clone(),
        e.formid,
        e.edid.to_lowercase(),
        e.source.clone(),
    )
}

fn id_of(k: &Key) -> IdKey {
    (k.0.clone(), k.1.clone(), k.2, k.3.clone())
}

pub fn compare(entries: &[TmEntry], rows: &[BaseRow], old: &[OldRow]) -> Comparison {
    let mut c = Comparison {
        dict_rows: rows.len(),
        ..Default::default()
    };
    let current: HashSet<Key> = rows.iter().map(row_key).collect();
    let old_keys: HashSet<Key> = old.iter().map(|o| row_key(&o.row)).collect();
    // By identifier: the current rows first, the old ones for what is left.
    let mut by_id: HashMap<IdKey, &BaseRow> = HashMap::new();
    for r in rows.iter().filter(|r| r.formid != 0 && !r.edid.is_empty()) {
        by_id.entry(id_of(&row_key(r))).or_insert(r);
    }
    for o in old
        .iter()
        .filter(|o| o.row.formid != 0 && !o.row.edid.is_empty())
    {
        by_id.entry(id_of(&row_key(&o.row))).or_insert(&o.row);
    }
    let mut doc_keys: HashSet<Key> = HashSet::new();
    for (i, e) in entries.iter().enumerate() {
        if e.unused {
            continue;
        }
        let k = entry_key(e);
        if current.contains(&k) || old_keys.contains(&k) {
            c.same += 1;
            doc_keys.insert(k);
            continue;
        }
        if e.formid != 0
            && !e.edid.is_empty()
            && let Some(r) = by_id.get(&id_of(&k))
            && r.source != e.source
        {
            c.changed.push(Changed {
                idx: i,
                old_source: r.source.clone(),
                old_trans: r.trans.clone(),
                old_status: r.status.clone(),
            });
            doc_keys.insert(k);
            continue;
        }
        c.added.push(i);
        doc_keys.insert(k);
    }
    // What the dictionary has and the document no longer has: the ids of
    // the changed lines are not « removed », their text moved.
    let changed_ids: HashSet<IdKey> = c
        .changed
        .iter()
        .filter_map(|ch| entries.get(ch.idx))
        .map(|e| id_of(&entry_key(e)))
        .collect();
    for r in rows {
        let k = row_key(r);
        if !doc_keys.contains(&k) && !changed_ids.contains(&id_of(&k)) {
            c.removed.push(Removed {
                row: r.clone(),
                date: String::new(),
            });
        }
    }
    for o in old {
        let k = row_key(&o.row);
        if !doc_keys.contains(&k) && !changed_ids.contains(&id_of(&k)) {
            c.removed.push(Removed {
                row: o.row.clone(),
                date: o.date.clone(),
            });
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rec: &str, field: &str, formid: u32, edid: &str, src: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            ..Default::default()
        }
    }
    fn row(rec: &str, field: &str, formid: u32, edid: &str, src: &str, tr: &str) -> BaseRow {
        BaseRow {
            rec: rec.into(),
            field: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            trans: tr.into(),
            status: Some("validated".into()),
            ..Default::default()
        }
    }

    /// T-B07-02: added, removed (current and old), changed, same.
    #[test]
    fn comparison_sorts_the_lines() {
        let rows = vec![
            row("INFO", "NAM1", 1, "A", "Hello.", "Bonjour."),
            row("INFO", "NAM1", 2, "B", "Bye.", "Au revoir."),
            row("WEAP", "FULL", 3, "C", "Sword", "Épée"),
        ];
        let old = vec![
            OldRow {
                row: row("INFO", "NAM1", 4, "D", "Gone.", "Parti."),
                date: "2026-10-02".into(),
            },
            OldRow {
                row: row("INFO", "NAM1", 5, "E", "Back.", "De retour."),
                date: "2026-09-01".into(),
            },
        ];
        let entries = vec![
            entry("INFO", "NAM1", 1, "A", "Hello."),       // same
            entry("INFO", "NAM1", 2, "B", "Bye, friend."), // changed
            entry("INFO", "NAM1", 5, "E", "Back."),        // same (came back from the old data)
            entry("INFO", "NAM1", 6, "F", "New line."),    // added
            {
                let mut e = entry("TES4", "CNAM", 0, "", "DEFAULT");
                e.unused = true;
                e
            },
        ];
        let c = compare(&entries, &rows, &old);
        assert_eq!(c.same, 2);
        assert_eq!(c.added, vec![3]);
        assert_eq!(c.changed.len(), 1);
        assert_eq!(
            (
                c.changed[0].idx,
                c.changed[0].old_source.as_str(),
                c.changed[0].old_trans.as_str()
            ),
            (1, "Bye.", "Au revoir.")
        );
        let removed: Vec<(&str, &str)> = c
            .removed
            .iter()
            .map(|r| (r.row.edid.as_str(), r.date.as_str()))
            .collect();
        assert_eq!(
            removed,
            vec![("C", ""), ("D", "2026-10-02")],
            "the changed id is not removed, the sword is, the old D stays"
        );
        assert_eq!(c.dict_rows, 3);
    }
}
