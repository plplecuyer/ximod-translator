//! Lignes communes (C-04): the lines that have the same source as a line,
//! in its file, in the other open files (B-25) and in the dictionaries,
//! for the Communes tab of the edit window. A source index per file (the
//! source key → its lines) keeps the look-up well under a frame.

use std::collections::HashMap;

use crate::tm::TmEntry;

/// Lines of files shown at most (R-C04-01).
pub const MAX_LINES: usize = 200;
/// Dictionary pairs shown at most (R-C04-01).
pub const MAX_PAIRS: usize = 20;

/// The key of a source: the text without its outer spaces (as B-25).
pub fn key(source: &str) -> &str {
    source.trim()
}

/// The lines of a file by source key, built once per file version; lines
/// never shown in game and empty sources are left out.
#[derive(Debug, Default, Clone)]
pub struct SourceIndex {
    map: HashMap<String, Vec<u32>>,
}

impl SourceIndex {
    pub fn build(entries: &[TmEntry]) -> Self {
        let mut map: HashMap<String, Vec<u32>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if e.unused {
                continue;
            }
            let k = key(&e.source);
            if k.is_empty() {
                continue;
            }
            match map.get_mut(k) {
                Some(v) => v.push(i as u32),
                None => {
                    map.insert(k.to_string(), vec![i as u32]);
                }
            }
        }
        SourceIndex { map }
    }

    /// The lines of a source key, in file order.
    pub fn lines(&self, key: &str) -> &[u32] {
        self.map.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Distinct sources.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// One file searched: its lines and its index.
pub struct File<'a> {
    pub entries: &'a [TmEntry],
    pub index: &'a SourceIndex,
}

/// A common line: the file (its position in the list given) and the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub file: usize,
    pub idx: usize,
}

/// What the Communes tab shows (R-C04-01).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The lines of the current file, then those of the other files, in
    /// the order of the list (the tabs' order), at most [`MAX_LINES`].
    pub hits: Vec<Hit>,
    /// Lines in the current file (all, not only those shown).
    pub here: usize,
    /// Lines in the other files (all).
    pub others: usize,
}

impl Found {
    /// Lines of files in all.
    pub fn total(&self) -> usize {
        self.here + self.others
    }

    /// Whether some lines are not shown.
    pub fn limited(&self) -> bool {
        self.total() > self.hits.len()
    }
}

/// The lines with the same source as line `line` of `files[cur]`, the
/// line itself left out (R-C04-01). `split_by_plugin`: the current file
/// is the « Tous les mods » synthesis, whose lines of another plugin count
/// as other files.
pub fn gather(files: &[File<'_>], cur: usize, line: usize, split_by_plugin: bool) -> Found {
    let mut out = Found::default();
    let Some(me) = files.get(cur).and_then(|f| f.entries.get(line)) else {
        return out;
    };
    let k = key(&me.source);
    if me.unused || k.is_empty() {
        return out;
    }
    let mut later: Vec<Hit> = Vec::new();
    let mut take = |hit: Hit, here: bool, out: &mut Found| {
        if here {
            out.here += 1;
        } else {
            out.others += 1;
        }
        if here {
            if out.hits.len() < MAX_LINES {
                out.hits.push(hit);
            }
        } else if later.len() < MAX_LINES {
            later.push(hit);
        }
    };
    let f = &files[cur];
    for &i in f.index.lines(k) {
        let i = i as usize;
        if i == line {
            continue;
        }
        let here = !split_by_plugin || f.entries[i].plugin == me.plugin;
        take(Hit { file: cur, idx: i }, here, &mut out);
    }
    for (fi, f) in files.iter().enumerate() {
        if fi == cur {
            continue;
        }
        for &i in f.index.lines(k) {
            take(
                Hit {
                    file: fi,
                    idx: i as usize,
                },
                false,
                &mut out,
            );
        }
    }
    // In the synthesis the other plugins' lines came in file order with
    // the current one's: theirs go after (R-C04-01).
    let room = MAX_LINES.saturating_sub(out.hits.len());
    out.hits.extend(later.into_iter().take(room));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(src: &str, plugin: &str) -> TmEntry {
        TmEntry {
            source: src.into(),
            plugin: plugin.into(),
            ..Default::default()
        }
    }

    #[test]
    fn here_then_other_files() {
        let a = vec![
            e("Iron Sword", "A"),
            e("Steel", "A"),
            e(" Iron Sword ", "A"),
            e("Iron Sword", "A"),
        ];
        let mut b = vec![e("Iron Sword", "B"), e("Iron Sword", "B")];
        b[1].unused = true;
        let (ia, ib) = (SourceIndex::build(&a), SourceIndex::build(&b));
        let files = [
            File {
                entries: &a,
                index: &ia,
            },
            File {
                entries: &b,
                index: &ib,
            },
        ];
        let f = gather(&files, 0, 0, false);
        assert_eq!(f.here, 2);
        assert_eq!(f.others, 1);
        assert_eq!(
            f.hits,
            vec![
                Hit { file: 0, idx: 2 },
                Hit { file: 0, idx: 3 },
                Hit { file: 1, idx: 0 }
            ]
        );
        assert!(!f.limited());
        // From the other file: the current file's lines are « ici ».
        let f = gather(&files, 1, 0, false);
        assert_eq!((f.here, f.others), (0, 3));
        // A source alone.
        let f = gather(&files, 0, 1, false);
        assert_eq!(f.total(), 0);
        assert!(f.hits.is_empty());
    }

    #[test]
    fn synthesis_splits_by_plugin() {
        let all = vec![
            e("Chest", "A"),
            e("Chest", "B"),
            e("Chest", "A"),
            e("Chest", "C"),
        ];
        let ix = SourceIndex::build(&all);
        let files = [File {
            entries: &all,
            index: &ix,
        }];
        let f = gather(&files, 0, 0, true);
        assert_eq!((f.here, f.others), (1, 2));
        // The line of A first, then B and C.
        let idx: Vec<usize> = f.hits.iter().map(|h| h.idx).collect();
        assert_eq!(idx, vec![2, 1, 3]);
    }

    #[test]
    fn limit_and_timing() {
        let big: Vec<TmEntry> = (0..150_000)
            .map(|n| e(&format!("Line {}", n % 500), "A"))
            .collect();
        let start = std::time::Instant::now();
        let ix = SourceIndex::build(&big);
        let built = start.elapsed();
        assert_eq!(ix.len(), 500);
        let files = [File {
            entries: &big,
            index: &ix,
        }];
        let start = std::time::Instant::now();
        let f = gather(&files, 0, 0, false);
        let took = start.elapsed();
        assert_eq!(f.here, 299);
        assert_eq!(f.hits.len(), MAX_LINES);
        assert!(f.limited());
        // Debug build: the release is about ten times faster.
        assert!(took < std::time::Duration::from_millis(50), "{took:?}");
        assert!(built < std::time::Duration::from_secs(3), "{built:?}");
    }
}
