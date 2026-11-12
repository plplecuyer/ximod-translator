//! Sorting the grid by a column (A-14, D-110).
//!
//! The sorted order is a **snapshot**: a rank is computed for every line of
//! the document when a header is clicked, and the view orders its lines by
//! that rank. Editing a line afterwards does not move it — a line validated
//! in a sort by status stays where it is until the next click. The rank
//! covers the whole document, so a change of filter does not recompute it.
//! The sort belongs to the view only: exports and the command line keep the
//! order of the file.

use crate::status::{self, Status};
use crate::tm::TmEntry;
use std::collections::HashMap;

/// The columns a header click sorts (R-A14-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortCol {
    Status,
    File,
    Record,
    Field,
    Edid,
    Source,
    Trans,
    Review,
    Prov,
    Comment,
}

/// The direction of a sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// The sort of a view: its column, direction and the rank of each line of
/// the document (`rank[i]` is the position of line `i`).
#[derive(Debug, Clone, PartialEq)]
pub struct Sort {
    pub col: SortCol,
    pub dir: SortDir,
    pub rank: Vec<u32>,
    /// Changes with each sort, for the view cache.
    pub serial: u64,
}

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Sort {
    /// Rank `entries` by `col` in `dir`; `texts` gives the displayed text
    /// of the Provenance or Commentaire column per line (the raw code / the
    /// author's comment otherwise).
    pub fn new(entries: &[TmEntry], col: SortCol, dir: SortDir, texts: Option<&[String]>) -> Self {
        Sort {
            col,
            dir,
            rank: rank(entries, col, dir, texts),
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Whether the rank still describes a document of `n` lines.
    pub fn fits(&self, n: usize) -> bool {
        self.rank.len() == n
    }

    /// Order the lines of a view by their rank (a line out of the rank goes
    /// last, in the order of the file).
    pub fn order(&self, visible: &mut [usize]) {
        visible.sort_by_key(|&i| (self.rank.get(i).copied().unwrap_or(u32::MAX), i));
    }
}

/// The working order of the statuses: what remains to be done first
/// (R-A14-01).
pub fn status_order(s: Status) -> u8 {
    match s {
        Status::ToTranslate => 0,
        Status::Pending => 1,
        Status::BySimilarity => 2,
        Status::ModifiedProposal => 3,
        Status::Custom => 4,
        Status::Validated => 5,
        Status::Ignored => 6,
    }
}

/// The sort after a click on `clicked` (R-A14-02): ascending, then
/// descending, then the order of the file (`None`); another column starts
/// ascending.
pub fn next_dir(current: Option<(SortCol, SortDir)>, clicked: SortCol) -> Option<SortDir> {
    match current {
        Some((c, SortDir::Asc)) if c == clicked => Some(SortDir::Desc),
        Some((c, SortDir::Desc)) if c == clicked => None,
        _ => Some(SortDir::Asc),
    }
}

/// The text column `col` shows for line `i` (`texts`: the displayed
/// Provenance or Commentaire, the raw code / the author's comment
/// otherwise); empty for the status.
fn text_of<'a>(i: usize, e: &'a TmEntry, col: SortCol, texts: Option<&'a [String]>) -> &'a str {
    let shown = |raw: &'a str| -> &'a str {
        texts
            .and_then(|t| t.get(i))
            .map(String::as_str)
            .unwrap_or(raw)
    };
    match col {
        SortCol::Status => "",
        SortCol::File => &e.plugin,
        SortCol::Record => &e.rec_sig,
        SortCol::Field => &e.field_sig,
        SortCol::Edid => &e.edid,
        SortCol::Source => &e.source,
        SortCol::Trans => &e.trans,
        SortCol::Review => &e.review,
        SortCol::Prov => shown(&e.prov),
        SortCol::Comment => shown(&e.comment),
    }
}

/// Lower case without accents, as the text filter compares (a fast path
/// for plain ASCII).
fn folded(s: &str) -> String {
    if s.is_ascii() {
        s.to_ascii_lowercase()
    } else {
        crate::journal::fold(s)
    }
}

/// The rank of every line (stable: equal lines keep the order of the file,
/// in both directions). The key of a line: for the status, its working
/// order; for a text, empty after the others, then the text without case or
/// accents, then as written, then (EDID) the FormID.
pub fn rank(entries: &[TmEntry], col: SortCol, dir: SortDir, texts: Option<&[String]>) -> Vec<u32> {
    // Each line gets an ordinal — of its status, or of its text among the
    // distinct texts, sorted once — so that the lines sort on integers:
    // comparing strings line by line is far too slow on a file full of
    // duplicates (56 915 lines of SS2 must sort in well under 100 ms).
    let ordinal: Vec<u32> = match col {
        SortCol::Status => entries
            .iter()
            .map(|e| {
                let st = if e.unused {
                    Status::Ignored
                } else {
                    status::derive(e)
                };
                u32::from(status_order(st))
            })
            .collect(),
        _ => {
            let mut ids: HashMap<&str, u32> = HashMap::new();
            let mut distinct: Vec<&str> = Vec::new();
            let line_ids: Vec<u32> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let t = text_of(i, e, col, texts);
                    match ids.get(t) {
                        Some(&k) => k,
                        None => {
                            let k = distinct.len() as u32;
                            ids.insert(t, k);
                            distinct.push(t);
                            k
                        }
                    }
                })
                .collect();
            let mut by_text: Vec<(u8, String, &str, u32)> = distinct
                .iter()
                .enumerate()
                .map(|(k, t)| (u8::from(t.trim().is_empty()), folded(t), *t, k as u32))
                .collect();
            by_text.sort_unstable();
            let mut place = vec![0u32; distinct.len()];
            for (pos, (.., k)) in by_text.iter().enumerate() {
                place[*k as usize] = pos as u32;
            }
            line_ids.iter().map(|&k| place[k as usize]).collect()
        }
    };
    let mut keyed: Vec<(u32, u32, u32)> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let num = if col == SortCol::Edid { e.formid } else { 0 };
            (ordinal[i], num, i as u32)
        })
        .collect();
    // The line breaks the last ties: a total order, so unstable is stable.
    keyed.sort_unstable_by(|a, b| {
        let o = (a.0, a.1).cmp(&(b.0, b.1));
        let o = if dir == SortDir::Desc { o.reverse() } else { o };
        o.then(a.2.cmp(&b.2))
    });
    let mut rank = vec![0u32; entries.len()];
    for (pos, &(_, _, i)) in keyed.iter().enumerate() {
        rank[i as usize] = pos as u32;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, trans: &str) -> TmEntry {
        TmEntry {
            source: source.into(),
            trans: trans.into(),
            ..TmEntry::default()
        }
    }

    fn sorted(entries: &[TmEntry], col: SortCol, dir: SortDir) -> Vec<usize> {
        let s = Sort::new(entries, col, dir, None);
        let mut v: Vec<usize> = (0..entries.len()).collect();
        s.order(&mut v);
        v
    }

    #[test]
    fn the_cycle_is_asc_desc_file_order() {
        use SortCol::*;
        assert_eq!(next_dir(None, Source), Some(SortDir::Asc));
        assert_eq!(
            next_dir(Some((Source, SortDir::Asc)), Source),
            Some(SortDir::Desc)
        );
        assert_eq!(next_dir(Some((Source, SortDir::Desc)), Source), None);
        assert_eq!(
            next_dir(Some((Source, SortDir::Desc)), Edid),
            Some(SortDir::Asc)
        );
    }

    #[test]
    fn case_and_accents_do_not_separate_texts() {
        let e = vec![
            entry("bound sword", ""),
            entry("Zèle", ""),
            entry("Bound Sword", ""),
            entry("épée", ""),
            entry("BOUND SWORD", ""),
            entry("Epee", ""),
        ];
        assert_eq!(
            sorted(&e, SortCol::Source, SortDir::Asc),
            vec![4, 2, 0, 5, 3, 1]
        );
        // « BOUND », « Bound », « bound » are next to each other, the ties
        // broken as written; the reverse keeps them together too.
        let d = sorted(&e, SortCol::Source, SortDir::Desc);
        assert_eq!(d[0], 1);
        let pos: Vec<usize> = [0, 2, 4]
            .iter()
            .map(|i| d.iter().position(|x| x == i).unwrap())
            .collect();
        assert_eq!(pos.iter().max().unwrap() - pos.iter().min().unwrap(), 2);
    }

    #[test]
    fn empty_translations_come_last_in_ascending_order() {
        let e = vec![
            entry("a", ""),
            entry("b", "zz"),
            entry("c", "aa"),
            entry("d", " "),
        ];
        assert_eq!(sorted(&e, SortCol::Trans, SortDir::Asc), vec![2, 1, 0, 3]);
    }

    #[test]
    fn status_puts_the_work_to_do_first() {
        let e = vec![entry("x", "done"), entry("y", ""), entry("z", "")];
        let s = sorted(&e, SortCol::Status, SortDir::Asc);
        assert_eq!(status::derive(&e[s[0]]), Status::ToTranslate);
        let s = sorted(&e, SortCol::Status, SortDir::Desc);
        assert_eq!(status::derive(&e[s[2]]), Status::ToTranslate);
        // Ties in the order of the file, both ways.
        let to_do: Vec<usize> = sorted(&e, SortCol::Status, SortDir::Desc)
            .into_iter()
            .filter(|i| e[*i].trans.is_empty())
            .collect();
        assert_eq!(to_do, vec![1, 2]);
    }

    #[test]
    fn the_sort_is_stable_on_a_thousand_random_lines() {
        // A small deterministic generator: four distinct sources.
        let mut x: u32 = 12345;
        let e: Vec<TmEntry> = (0..1000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let s = ["Alpha", "alpha", "Beta", "gamma"][(x >> 16) as usize % 4];
                entry(s, "")
            })
            .collect();
        for dir in [SortDir::Asc, SortDir::Desc] {
            let v = sorted(&e, SortCol::Source, dir);
            for w in v.windows(2) {
                let (a, b) = (&e[w[0]], &e[w[1]]);
                if a.source == b.source {
                    assert!(w[0] < w[1], "stable");
                }
            }
            let mut seen = v.clone();
            seen.sort_unstable();
            assert_eq!(seen, (0..1000).collect::<Vec<_>>());
        }
    }

    #[test]
    fn display_texts_win_over_raw_codes() {
        let mut a = entry("a", "");
        a.prov = "zz".into();
        let mut b = entry("b", "");
        b.prov = "aa".into();
        let texts = vec!["Alpha".to_string(), "Beta".to_string()];
        let s = Sort::new(
            &[a.clone(), b.clone()],
            SortCol::Prov,
            SortDir::Asc,
            Some(&texts),
        );
        let mut v = vec![1, 0];
        s.order(&mut v);
        assert_eq!(v, vec![0, 1]);
        let s = Sort::new(&[a, b], SortCol::Prov, SortDir::Asc, None);
        let mut v = vec![0, 1];
        s.order(&mut v);
        assert_eq!(v, vec![1, 0]);
    }

    #[test]
    fn an_outdated_rank_puts_unknown_lines_last() {
        let e = vec![entry("b", ""), entry("a", "")];
        let s = Sort::new(&e, SortCol::Source, SortDir::Asc, None);
        assert!(s.fits(2));
        assert!(!s.fits(3));
        let mut v = vec![2, 0, 1];
        s.order(&mut v);
        assert_eq!(v, vec![1, 0, 2]);
    }
}
