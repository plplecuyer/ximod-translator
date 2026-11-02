//! Propagation aux lignes de même source (A-12): from a translated line,
//! the other lines of the document with the same source text, listed for
//! the user to check, then given the translation and the status of the
//! origin in one undoable operation.

use std::collections::HashMap;

use crate::status::{self, Status};
use crate::tm::TmEntry;

/// A line that may receive the translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub idx: usize,
    pub checked: bool,
    /// Its translation already equals the origin's.
    pub same: bool,
    /// Another open file (C-04): its position among the documents; `None`
    /// for the origin's own file.
    pub file: Option<usize>,
}

/// The lines of one source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub origin: usize,
    pub source: String,
    pub trans: String,
    pub status: Status,
    pub targets: Vec<Target>,
}

impl Group {
    pub fn checked(&self) -> usize {
        self.targets.iter().filter(|t| t.checked).count()
    }
}

/// The groups of `origins` (R-A12-01, R-A12-03): one per translated origin
/// line, in the order given; an origin without translation, or whose
/// source another origin already carries, gives none. Ignored lines and
/// lines never shown in game are left out; every target is checked except
/// a Validé or Personnalisé line whose translation differs.
pub fn groups(entries: &[TmEntry], origins: &[usize]) -> Vec<Group> {
    build(entries, origins, false)
}

/// As [`groups`], a group kept even without a target in its own file, for
/// the lines of the other open files to join it (C-04, R-C04-03); drop the
/// empty ones with [`drop_empty`] afterwards.
pub fn groups_open(entries: &[TmEntry], origins: &[usize]) -> Vec<Group> {
    build(entries, origins, true)
}

fn build(entries: &[TmEntry], origins: &[usize], keep_empty: bool) -> Vec<Group> {
    let mut by_source: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        // The key of C-04 and B-25: outer spaces left out.
        let k = crate::common::key(&e.source);
        if e.unused || k.is_empty() {
            continue;
        }
        by_source.entry(k).or_default().push(i);
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for &o in origins {
        let Some(e) = entries.get(o) else {
            continue;
        };
        let k = crate::common::key(&e.source);
        if e.unused || e.trans.is_empty() || k.is_empty() || seen.contains(&k) {
            continue;
        }
        let st = status::derive(e);
        if st == Status::Ignored {
            continue;
        }
        seen.push(k);
        let targets: Vec<Target> = by_source
            .get(k)
            .map(|v| v.iter().copied())
            .into_iter()
            .flatten()
            .filter(|&i| i != o)
            .filter_map(|i| {
                let t = &entries[i];
                let ts = status::derive(t);
                if ts == Status::Ignored {
                    return None;
                }
                let same = t.trans == e.trans && ts == st;
                let differs = t.trans != e.trans;
                let checked = !(differs && matches!(ts, Status::Validated | Status::Custom));
                Some(Target {
                    idx: i,
                    checked,
                    same,
                    file: None,
                })
            })
            .collect();
        if targets.is_empty() && !keep_empty {
            continue;
        }
        out.push(Group {
            origin: o,
            source: e.source.clone(),
            trans: e.trans.clone(),
            status: st,
            targets,
        });
    }
    out
}

/// The checking rule of a target (R-A12-01): every line is checked but a
/// Validé or Personnalisé one whose translation differs; `None` for an
/// Ignoré line or one never shown in game.
fn target_of(g: &Group, t: &TmEntry, idx: usize, file: Option<usize>) -> Option<Target> {
    if t.unused {
        return None;
    }
    let ts = status::derive(t);
    if ts == Status::Ignored {
        return None;
    }
    let differs = t.trans != g.trans;
    Some(Target {
        idx,
        checked: !(differs && matches!(ts, Status::Validated | Status::Custom)),
        same: !differs && ts == g.status,
        file,
    })
}

/// Add to each group the lines of another open file (`file`, its position
/// among the documents) whose source has the same key (outer spaces left
/// out, as B-25), with the rule of A-12 (R-C04-03).
pub fn extend(
    groups: &mut [Group],
    file: usize,
    entries: &[TmEntry],
    index: &crate::common::SourceIndex,
) {
    for g in groups.iter_mut() {
        let k = crate::common::key(&g.source);
        for &i in index.lines(k) {
            let i = i as usize;
            if let Some(t) = entries.get(i).and_then(|t| target_of(g, t, i, Some(file))) {
                g.targets.push(t);
            }
        }
    }
}

/// Leave out the groups without a target.
pub fn drop_empty(groups: &mut Vec<Group>) {
    groups.retain(|g| !g.targets.is_empty());
}

/// Whether a line has at least one other line of the same source that
/// does not already carry its translation and status (the proposal after
/// validation, R-A12-02).
pub fn has_differing_duplicates(entries: &[TmEntry], idx: usize) -> bool {
    groups(entries, &[idx])
        .first()
        .is_some_and(|g| g.targets.iter().any(|t| !t.same))
}

/// Copy the translation and the status of each origin on its checked
/// targets of its own file; the touched lines.
pub fn apply(entries: &mut [TmEntry], groups: &[Group]) -> Vec<usize> {
    apply_file(entries, groups, None)
}

/// As [`apply`] for the targets of one file: `None` the origin's, or
/// another open file (C-04).
pub fn apply_file(entries: &mut [TmEntry], groups: &[Group], file: Option<usize>) -> Vec<usize> {
    let mut touched = Vec::new();
    for g in groups {
        for t in g.targets.iter().filter(|t| t.checked && t.file == file) {
            let Some(e) = entries.get_mut(t.idx) else {
                continue;
            };
            e.trans = g.trans.clone();
            e.set_translated_flags(!e.trans.is_empty());
            status::set(e, g.status);
            touched.push(t.idx);
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(i: u32, src: &str, tr: &str) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: "INFO".into(),
            field_sig: "NAM1".into(),
            formid: i,
            edid: format!("E{i}"),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        };
        e.set_translated_flags(!tr.is_empty());
        e
    }

    /// T-A12-01: twenty « Hello. », one validated; nineteen targets, one
    /// unchecked, eighteen applied.
    #[test]
    fn twenty_lines_one_validated() {
        let mut entries: Vec<TmEntry> = (0..20).map(|i| entry(i, "Hello.", "")).collect();
        entries[0].trans = "Bonjour.".into();
        entries[0].set_translated_flags(true);
        status::set(&mut entries[0], Status::Validated);
        // One Ignoré, one never shown: left out.
        status::set(&mut entries[5], Status::Ignored);
        entries[6].unused = true;
        // One Validé with another translation: unchecked.
        entries[7].trans = "Salut.".into();
        entries[7].set_translated_flags(true);
        status::set(&mut entries[7], Status::Validated);
        entries.push(entry(99, "Bye.", ""));
        let g = groups(&entries, &[0]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].targets.len(), 17);
        assert_eq!(g[0].checked(), 16);
        assert!(!g[0].targets.iter().find(|t| t.idx == 7).unwrap().checked);
        assert!(has_differing_duplicates(&entries, 0));
        let mut g = g;
        g[0].targets[0].checked = false;
        let touched = apply(&mut entries, &g);
        assert_eq!(touched.len(), 15);
        for &i in &touched {
            assert_eq!(entries[i].trans, "Bonjour.");
            assert_eq!(status::derive(&entries[i]), Status::Validated);
        }
        assert_eq!(entries[7].trans, "Salut.");
        assert_eq!(entries[5].trans, "");
        // No duplicate for the last line; none for an untranslated origin.
        assert!(groups(&entries, &[20]).is_empty());
        assert!(groups(&entries, &[1]).is_empty() || entries[1].trans == "Bonjour.");
        // Everything in place: no differing duplicate left (but the two kept).
        assert!(has_differing_duplicates(&entries, 0));
    }

    /// R-A12-03: a selection groups by source, once per source.
    #[test]
    fn selection_groups_by_source() {
        let mut entries = vec![
            entry(1, "A", "a"),
            entry(2, "A", ""),
            entry(3, "B", "b"),
            entry(4, "B", ""),
            entry(5, "A", "a2"),
            entry(6, "C", "c"),
        ];
        for e in entries.iter_mut() {
            if !e.trans.is_empty() {
                status::set(e, Status::Pending);
            }
        }
        let g = groups(&entries, &[0, 2, 4, 5]);
        let srcs: Vec<&str> = g.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(srcs, vec!["A", "B"], "A once, C has no duplicate");
        assert_eq!(g[0].targets.len(), 2);
        assert!(
            g[0].targets.iter().all(|t| t.checked),
            "Pending differing: checked"
        );
        let touched = apply(&mut entries, &g);
        assert_eq!(touched, vec![1, 4, 3]);
        assert_eq!(entries[4].trans, "a");
    }

    /// C-04: a line alone in its file, two lines of the same source in
    /// another open file; one operation per file.
    #[test]
    fn other_files_join_the_group() {
        let mut mine = vec![entry(1, "Chest", "Coffre"), entry(2, "Barrel", "")];
        status::set(&mut mine[0], Status::Validated);
        assert!(groups(&mine, &[0]).is_empty());
        let mut g = groups_open(&mine, &[0, 1]);
        assert_eq!(g.len(), 1, "an origin without translation gives none");
        let mut patch = vec![
            entry(7, " Chest", ""),
            entry(8, "Chest", "Malle"),
            entry(9, "Barrel", ""),
        ];
        status::set(&mut patch[1], Status::Validated);
        let ix = crate::common::SourceIndex::build(&patch);
        extend(&mut g, 3, &patch, &ix);
        drop_empty(&mut g);
        assert_eq!(g[0].targets.len(), 2);
        assert!(g[0].targets.iter().all(|t| t.file == Some(3)));
        assert!(g[0].targets[0].checked && !g[0].targets[1].checked);
        // Nothing in the origin's own file.
        assert!(apply(&mut mine, &g).is_empty());
        assert_eq!(apply_file(&mut patch, &g, Some(3)), vec![0]);
        assert_eq!(patch[0].trans, "Coffre");
        assert_eq!(status::derive(&patch[0]), Status::Validated);
        assert_eq!(patch[1].trans, "Malle");
    }
}
