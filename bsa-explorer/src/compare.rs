//! Compare two archives by their file lists: which files were **added**,
//! **removed**, **modified**, or left **unchanged** — like BSA Browser's
//! "Compare archives".
//!
//! Comparison is by **path + decompressed size**, which is fast (index-only, no
//! extraction) and catches the vast majority of real changes. Paths are matched
//! case-insensitively (Bethesda archive paths are case-insensitive). A file whose
//! content changed without changing size is reported as *unchanged* — a rare edge
//! case that only a full content hash would catch.

use crate::archive::ArchiveEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Present in B, absent from A.
    Added,
    /// Present in A, absent from B.
    Removed,
    /// Present in both, different size.
    Modified,
    /// Present in both, same size.
    Unchanged,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Added => "Ajouté",
            Status::Removed => "Supprimé",
            Status::Modified => "Modifié",
            Status::Unchanged => "Inchangé",
        }
    }
}

/// One row of a comparison.
#[derive(Debug, Clone)]
pub struct DiffRow {
    pub status: Status,
    /// Display path (from A when present, else B).
    pub path: String,
    pub size_a: Option<u64>,
    pub size_b: Option<u64>,
}

/// The full comparison of A → B.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub rows: Vec<DiffRow>,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
}

/// Compare archive `a` (left) with archive `b` (right).
pub fn compare(a: &[ArchiveEntry], b: &[ArchiveEntry]) -> Diff {
    use std::collections::HashMap;

    // key = lowercased path; value = (display path, size)
    let index = |entries: &[ArchiveEntry]| -> HashMap<String, (String, u64)> {
        entries
            .iter()
            .map(|e| (e.path.to_lowercase(), (e.path.clone(), e.size)))
            .collect()
    };
    let ma = index(a);
    let mb = index(b);

    let mut rows: Vec<DiffRow> = Vec::new();
    let mut diff = Diff::default();

    // Everything in A: removed or common (modified/unchanged).
    for (key, (path, size_a)) in &ma {
        match mb.get(key) {
            None => {
                rows.push(DiffRow {
                    status: Status::Removed,
                    path: path.clone(),
                    size_a: Some(*size_a),
                    size_b: None,
                });
                diff.removed += 1;
            }
            Some((_, size_b)) => {
                if size_a == size_b {
                    rows.push(DiffRow {
                        status: Status::Unchanged,
                        path: path.clone(),
                        size_a: Some(*size_a),
                        size_b: Some(*size_b),
                    });
                    diff.unchanged += 1;
                } else {
                    rows.push(DiffRow {
                        status: Status::Modified,
                        path: path.clone(),
                        size_a: Some(*size_a),
                        size_b: Some(*size_b),
                    });
                    diff.modified += 1;
                }
            }
        }
    }
    // Everything only in B: added.
    for (key, (path, size_b)) in &mb {
        if !ma.contains_key(key) {
            rows.push(DiffRow {
                status: Status::Added,
                path: path.clone(),
                size_a: None,
                size_b: Some(*size_b),
            });
            diff.added += 1;
        }
    }

    rows.sort_by_key(|r| r.path.to_lowercase());
    diff.rows = rows;
    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ArchiveEntry;

    fn entry(path: &str, size: u64) -> ArchiveEntry {
        ArchiveEntry::for_test(path, size)
    }

    #[test]
    fn classifies_added_removed_modified_unchanged() {
        let a = [
            entry("common/same.dds", 100),
            entry("common/changed.nif", 10),
            entry("only_a.txt", 5),
        ];
        let b = [
            entry("common/same.dds", 100),
            entry("common/changed.nif", 20),
            entry("only_b.txt", 7),
        ];
        let d = compare(&a, &b);
        assert_eq!(d.unchanged, 1);
        assert_eq!(d.modified, 1);
        assert_eq!(d.removed, 1);
        assert_eq!(d.added, 1);
        // Case-insensitive path matching.
        let a2 = [entry("Foo/Bar.DDS", 1)];
        let b2 = [entry("foo/bar.dds", 1)];
        assert_eq!(compare(&a2, &b2).unchanged, 1);
    }
}
