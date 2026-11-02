//! Character-by-character differences between two texts (A-08, decision
//! D-49): Myers on the graphemes through the `similar` crate, then a
//! semantic cleanup — a common run shorter than two graphemes between two
//! changes is absorbed, and a change inside a word or a number grows to
//! the whole word or number — so that two sources that differ by one
//! number show that number alone. Long texts (or a slow comparison) fall
//! back to a word-by-word comparison.

use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, TextDiff};

/// Above this many characters (either side) the comparison is by words.
pub const MAX_CHARS: usize = 5_000;
/// A grapheme comparison slower than this is redone by words.
pub const TIME_LIMIT: Duration = Duration::from_millis(50);

/// The two texts as runs: `(text, differs)`; a differing run of the line
/// is what the line has and the entry has not, and conversely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub line: Vec<(String, bool)>,
    pub entry: Vec<(String, bool)>,
    /// Compared word by word (long text, R-A08-04).
    pub word_mode: bool,
}

impl Diff {
    /// The same comparison read from the entry (⇄ Inverser).
    pub fn swapped(&self) -> Diff {
        Diff {
            line: self.entry.clone(),
            entry: self.line.clone(),
            word_mode: self.word_mode,
        }
    }

    pub fn differs(&self) -> bool {
        self.line.iter().chain(&self.entry).any(|(_, d)| *d)
    }
}

/// A changed region: token ranges in the old (line) and new (entry) texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    old: (usize, usize),
    new: (usize, usize),
}

fn is_word(t: &str) -> bool {
    t.chars().all(|c| c.is_alphanumeric()) && !t.is_empty()
}

/// Compare `line` with `entry` (R-A08-01/03/04).
pub fn compute(line: &str, entry: &str) -> Diff {
    let long = line.chars().count() > MAX_CHARS || entry.chars().count() > MAX_CHARS;
    if !long {
        let start = Instant::now();
        let d = TextDiff::configure()
            .algorithm(Algorithm::Myers)
            .timeout(TIME_LIMIT)
            .diff_graphemes(line, entry);
        if start.elapsed() <= TIME_LIMIT {
            let old: Vec<&str> = d.iter_old_slices().collect();
            let new: Vec<&str> = d.iter_new_slices().collect();
            return from_ops(&old, &new, d.ops(), true, false);
        }
    }
    let d = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_words(line, entry);
    let old: Vec<&str> = d.iter_old_slices().collect();
    let new: Vec<&str> = d.iter_new_slices().collect();
    from_ops(&old, &new, d.ops(), false, true)
}

fn from_ops(old: &[&str], new: &[&str], ops: &[DiffOp], cleanup: bool, word_mode: bool) -> Diff {
    // The changed regions, in order.
    let mut regions: Vec<Region> = Vec::new();
    for op in ops {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => regions.push(Region {
                old: (old_index, old_index + old_len),
                new: (new_index, new_index),
            }),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => regions.push(Region {
                old: (old_index, old_index),
                new: (new_index, new_index + new_len),
            }),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => regions.push(Region {
                old: (old_index, old_index + old_len),
                new: (new_index, new_index + new_len),
            }),
        }
    }
    if cleanup {
        regions = absorb_short_gaps(regions, 2);
        for r in regions.iter_mut() {
            extend_to_words(r, old, new);
        }
        regions = absorb_short_gaps(regions, 1);
    }
    Diff {
        line: runs(old, regions.iter().map(|r| r.old)),
        entry: runs(new, regions.iter().map(|r| r.new)),
        word_mode,
    }
}

/// Merge two regions when the common run between them is shorter than
/// `min` tokens (on both sides).
fn absorb_short_gaps(regions: Vec<Region>, min: usize) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::with_capacity(regions.len());
    for r in regions {
        if let Some(last) = out.last_mut()
            && r.old.0 - last.old.1 < min
            && r.new.0 - last.new.1 < min
        {
            last.old.1 = r.old.1;
            last.new.1 = r.new.1;
        } else {
            out.push(r);
        }
    }
    out
}

/// Grow a change to the whole word or number it sits in: the common
/// token at its edge and the changed token next to it are both word
/// characters (R-A08-03).
fn extend_to_words(r: &mut Region, old: &[&str], new: &[&str]) {
    // Left edge: the common token before the change is old[r.old.0 - 1]
    // (equal to new[r.new.0 - 1]).
    loop {
        if r.old.0 == 0 || r.new.0 == 0 {
            break;
        }
        let edge = old[r.old.0 - 1];
        let first = if r.old.0 < r.old.1 {
            Some(old[r.old.0])
        } else if r.new.0 < r.new.1 {
            Some(new[r.new.0])
        } else {
            None
        };
        match first {
            Some(f) if is_word(edge) && is_word(f) => {
                r.old.0 -= 1;
                r.new.0 -= 1;
            }
            _ => break,
        }
    }
    loop {
        if r.old.1 >= old.len() || r.new.1 >= new.len() {
            break;
        }
        let edge = old[r.old.1];
        let last = if r.old.0 < r.old.1 {
            Some(old[r.old.1 - 1])
        } else if r.new.0 < r.new.1 {
            Some(new[r.new.1 - 1])
        } else {
            None
        };
        match last {
            Some(l) if is_word(edge) && is_word(l) => {
                r.old.1 += 1;
                r.new.1 += 1;
            }
            _ => break,
        }
    }
}

/// The tokens as runs `(text, changed)`, the changed ranges given in order.
fn runs(tokens: &[&str], changed: impl Iterator<Item = (usize, usize)>) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut pos = 0;
    let mut push = |range: (usize, usize), flag: bool| {
        if range.0 < range.1 {
            out.push((tokens[range.0..range.1].concat(), flag));
        }
    };
    for (s, e) in changed {
        push((pos, s), false);
        push((s, e), true);
        pos = e;
    }
    push((pos, tokens.len()), false);
    out
}

/// The text of a changed run with its blanks made visible (R-A08-02):
/// the space « · », the line break « ↵ », the tabulation « → » (given by
/// the caller, so the glyphs follow the font).
pub fn visible_blanks(text: &str, space: &str, newline: &str, tab: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ' ' => s.push_str(space),
            '\n' => s.push_str(newline),
            '\r' => {}
            '\t' => s.push_str(tab),
            c => s.push(c),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(side: &[(String, bool)]) -> Vec<String> {
        side.iter()
            .filter(|(_, d)| *d)
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// T-A08-01: only the number and the punctuation differ.
    #[test]
    fn a_number_and_a_punctuation_mark() {
        let d = compute("You have 3 days left!", "You have 5 days left.");
        assert!(!d.word_mode);
        assert_eq!(changed(&d.line), vec!["3", "!"]);
        assert_eq!(changed(&d.entry), vec!["5", "."]);
        let whole: String = d.line.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(whole, "You have 3 days left!");
    }

    /// T-A08-02: swapped, the sides and the colours change.
    #[test]
    fn swapped_reads_from_the_entry() {
        let d = compute("12 gold", "120 gold").swapped();
        assert_eq!(changed(&d.line), vec!["120"]);
        assert_eq!(changed(&d.entry), vec!["12"]);
    }

    /// T-A08-03: the whole number, not the « 0 » alone; and a word.
    #[test]
    fn a_change_grows_to_the_whole_word() {
        let d = compute("12 gold", "120 gold");
        assert_eq!(changed(&d.line), vec!["12"]);
        assert_eq!(changed(&d.entry), vec!["120"]);
        let d = compute("Open the door", "Open the doors");
        assert_eq!(changed(&d.line), vec!["door"]);
        assert_eq!(changed(&d.entry), vec!["doors"]);
        // A short common run between two changes is absorbed.
        let d = compute("abc", "xbz");
        assert_eq!(changed(&d.line), vec!["abc"]);
        assert_eq!(changed(&d.entry), vec!["xbz"]);
        assert!(!compute("Same", "Same").differs());
    }

    /// T-A08-04: a long text is compared by words.
    #[test]
    fn a_long_text_goes_by_words() {
        let a = "word ".repeat(1_200);
        let b = format!("{a}end");
        let start = Instant::now();
        let d = compute(&a, &b);
        assert!(d.word_mode);
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(changed(&d.entry), vec!["end"]);
    }

    #[test]
    fn blanks_are_made_visible() {
        assert_eq!(visible_blanks("a b\nc\td", "·", "↵", "→"), "a·b↵c→d");
    }
}
