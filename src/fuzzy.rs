//! Heuristique par mots (B-01): a text becomes its list of normalised
//! words, two texts are compared by a Levenshtein distance on those lists,
//! and an inverted index finds the close entries of a dictionary without
//! comparing them all. Serves the Vocabulaire tab (suggestions of the
//! current line) and the « Approchant » mode of the application dialog.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// The token of every engine tag `<…>` and of every number.
const TAG: u64 = 1;
const NUM: u64 = 2;

/// The words of a text as the heuristic sees them: lower case, tags and
/// numbers replaced by one token each, punctuation dropped.
pub fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            let w = std::mem::take(cur);
            out.push(if w.chars().all(|c| c.is_ascii_digit()) {
                "<num>".to_string()
            } else {
                w
            });
        }
    };
    while let Some(c) = chars.next() {
        if c == '<' {
            flush(&mut cur, &mut out);
            // A tag: everything to the closing bracket.
            let mut closed = false;
            for d in chars.by_ref() {
                if d == '>' {
                    closed = true;
                    break;
                }
            }
            if closed {
                out.push("<tag>".to_string());
            }
            continue;
        }
        if c.is_alphanumeric() || c == '\'' && !cur.is_empty() {
            cur.extend(c.to_lowercase());
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn hash_word(w: &str) -> u64 {
    match w {
        "<tag>" => TAG,
        "<num>" => NUM,
        _ => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            w.hash(&mut h);
            h.finish().max(3)
        }
    }
}

/// The word hashes of a text.
pub fn tokens(text: &str) -> Vec<u64> {
    words(text).iter().map(|w| hash_word(w)).collect()
}

/// Levenshtein distance on word lists (insert, delete, replace = 1).
pub fn distance(a: &[u64], b: &[u64]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &wa) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &wb) in b.iter().enumerate() {
            let cost = usize::from(wa != wb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Score in percent: 1 − distance / max(len a, len b).
pub fn score(dist: usize, la: usize, lb: usize) -> u8 {
    let m = la.max(lb);
    if m == 0 {
        return 100;
    }
    ((1.0 - dist as f64 / m as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

/// Whether a distance is within the threshold: `dist ≤ pct % × len`.
pub fn within(dist: usize, len: usize, pct: u8) -> bool {
    dist as f64 <= pct as f64 / 100.0 * len as f64
}

/// One entry of the index.
#[derive(Debug, Clone)]
pub struct Entry {
    pub words: Vec<u64>,
    pub source: String,
    pub trans: String,
    pub rec: String,
    pub field: String,
    /// From the old data of a mod's dictionary (B-07).
    pub old: bool,
}

/// The closest entries of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub score: u8,
    pub distance: usize,
    pub source: String,
    pub trans: String,
    pub rec: String,
    pub field: String,
    pub old: bool,
    /// The words of the suggestion that the text does not have.
    pub differing: Vec<String>,
}

pub struct FuzzyIndex {
    entries: Vec<Entry>,
    /// Word → entries containing it, except the frequent words.
    by_word: HashMap<u64, Vec<u32>>,
    stop: std::collections::HashSet<u64>,
}

impl FuzzyIndex {
    /// Build from (record, field, source, translation, old) tuples;
    /// `progress(done, total)` every 4096 entries, false to cancel.
    pub fn build<I>(
        rows: I,
        total: usize,
        mut progress: impl FnMut(usize, usize) -> bool,
    ) -> Option<FuzzyIndex>
    where
        I: IntoIterator<Item = (String, String, String, String, bool)>,
    {
        let mut entries: Vec<Entry> = Vec::with_capacity(total);
        let mut by_word: HashMap<u64, Vec<u32>> = HashMap::new();
        for (i, (rec, field, source, trans, old)) in rows.into_iter().enumerate() {
            if i % 4096 == 0 && !progress(i, total) {
                return None;
            }
            if trans.is_empty() {
                continue;
            }
            let ws = tokens(&source);
            if ws.is_empty() {
                continue;
            }
            let id = entries.len() as u32;
            let mut seen = std::collections::HashSet::new();
            for &w in &ws {
                if seen.insert(w) {
                    by_word.entry(w).or_default().push(id);
                }
            }
            entries.push(Entry {
                words: ws,
                source,
                trans,
                rec,
                field,
                old,
            });
        }
        // Words present in more than 5 % of the entries do not select
        // candidates (they still count in the distance).
        let limit = (entries.len() / 20).max(50);
        let mut stop = std::collections::HashSet::new();
        for (w, ids) in &by_word {
            if ids.len() > limit {
                stop.insert(*w);
            }
        }
        for w in &stop {
            by_word.remove(w);
        }
        progress(total, total);
        Some(FuzzyIndex {
            entries,
            by_word,
            stop,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries within the threshold, best first, at most `max`.
    pub fn suggest(&self, text: &str, pct: u8, max: usize) -> Vec<Suggestion> {
        let ws = tokens(text);
        if ws.is_empty() {
            return Vec::new();
        }
        let len = ws.len();
        let max_dist = (pct as f64 / 100.0 * len as f64).floor() as usize;
        // Candidates: share a rare word, with a compatible length.
        let mut counts: HashMap<u32, u32> = HashMap::new();
        let mut rare = 0usize;
        for &w in &ws {
            if self.stop.contains(&w) {
                continue;
            }
            rare += 1;
            if let Some(ids) = self.by_word.get(&w) {
                for &id in ids {
                    *counts.entry(id).or_default() += 1;
                }
            }
        }
        // A text made only of frequent words: compare with every entry of
        // a compatible length (rare case, short texts).
        let candidates: Vec<u32> = if rare == 0 {
            (0..self.entries.len() as u32).collect()
        } else {
            // At least len − max_dist shared words are needed for a match.
            let need = (len.saturating_sub(max_dist))
                .saturating_sub(ws.iter().filter(|w| self.stop.contains(w)).count())
                .max(1) as u32;
            counts
                .into_iter()
                .filter(|(_, c)| *c >= need)
                .map(|(id, _)| id)
                .collect()
        };
        let mut out: Vec<Suggestion> = Vec::new();
        for id in candidates {
            let e = &self.entries[id as usize];
            if e.words.len().abs_diff(len) > max_dist {
                continue;
            }
            let d = distance(&ws, &e.words);
            if d > max_dist {
                continue;
            }
            let differing: Vec<String> = {
                let have: std::collections::HashSet<u64> = ws.iter().copied().collect();
                words(&e.source)
                    .into_iter()
                    .filter(|w| !have.contains(&hash_word(w)))
                    .collect()
            };
            out.push(Suggestion {
                score: score(d, len, e.words.len()),
                distance: d,
                source: e.source.clone(),
                trans: e.trans.clone(),
                rec: e.rec.clone(),
                field: e.field.clone(),
                old: e.old,
                differing,
            });
        }
        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(a.old.cmp(&b.old))
                .then(a.source.cmp(&b.source))
        });
        out.dedup_by(|a, b| a.source == b.source && a.trans == b.trans);
        out.truncate(max);
        out
    }

    /// The best entry within the threshold, if any (the batch mode).
    pub fn best(&self, text: &str, pct: u8) -> Option<Suggestion> {
        self.suggest(text, pct, 1).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_normalised() {
        assert_eq!(
            words("You're finally awake, prisoner."),
            vec!["you're", "finally", "awake", "prisoner"]
        );
        assert_eq!(
            words("Deal 25 points of <Alias=Victim> damage!"),
            vec!["deal", "<num>", "points", "of", "<tag>", "damage"]
        );
        assert_eq!(
            words("Iron Dagger of Burning"),
            vec!["iron", "dagger", "of", "burning"]
        );
        assert_eq!(
            distance(&tokens("Iron Dagger"), &tokens("Iron Dagger of Burning")),
            2
        );
        assert_eq!(
            distance(
                &tokens("You're finally awake."),
                &tokens("You're finally awake, prisoner.")
            ),
            1
        );
        assert_eq!(score(1, 3, 4), 75);
        assert!(within(1, 4, 25) && !within(1, 4, 20));
    }

    fn index() -> FuzzyIndex {
        let rows = vec![
            (
                "INFO".to_string(),
                "NAM1".to_string(),
                "You're finally awake.".to_string(),
                "Tu es enfin réveillé.".to_string(),
                false,
            ),
            (
                "WEAP".to_string(),
                "FULL".to_string(),
                "Iron Dagger".to_string(),
                "Dague en fer".to_string(),
                false,
            ),
            (
                "WEAP".to_string(),
                "FULL".to_string(),
                "Iron Dagger of Fire".to_string(),
                "Dague en fer de feu".to_string(),
                true,
            ),
            (
                "WEAP".to_string(),
                "FULL".to_string(),
                "Steel Sword".to_string(),
                "Épée en acier".to_string(),
                false,
            ),
            (
                "BOOK".to_string(),
                "FULL".to_string(),
                "Empty".to_string(),
                String::new(),
                false,
            ),
        ];
        FuzzyIndex::build(rows, 5, |_, _| true).unwrap()
    }

    /// T-B01-01: the one-word insertion at 75 %, retained at 25 %, not at 20 %.
    #[test]
    fn suggestions_follow_the_threshold() {
        let ix = index();
        assert_eq!(ix.len(), 4, "the untranslated row is left out");
        let s = ix.suggest("You're finally awake, prisoner.", 25, 8);
        assert_eq!(s.len(), 1);
        assert_eq!(
            (s[0].score, s[0].distance, s[0].differing.len()),
            (75, 1, 0)
        );
        assert!(
            ix.suggest("You're finally awake, prisoner.", 20, 8)
                .is_empty()
        );
        // Two words, 50 %: one word of difference at most.
        let s = ix.suggest("Iron Dagger", 50, 8);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].source.as_str(), s[0].score), ("Iron Dagger", 100));
        // Three words, 34 %: both daggers, the suggestion's extra word listed.
        let s = ix.suggest("Iron Dagger of", 34, 8);
        let got: Vec<(&str, u8)> = s.iter().map(|x| (x.source.as_str(), x.score)).collect();
        assert_eq!(got, vec![("Iron Dagger of Fire", 75), ("Iron Dagger", 67)]);
        assert_eq!(s[0].differing, vec!["fire"]);
    }

    /// T-B01-02: « Iron Dagger of Burning » — the old « of Fire » at 75 % before « Iron Dagger » at 50 %.
    #[test]
    fn closer_first_old_data_included() {
        let ix = index();
        let s = ix.suggest("Iron Dagger of Burning", 50, 8);
        let got: Vec<(&str, u8, bool)> = s
            .iter()
            .map(|x| (x.source.as_str(), x.score, x.old))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Iron Dagger of Fire", 75, true),
                ("Iron Dagger", 50, false)
            ]
        );
        assert_eq!(
            ix.best("Iron Dagger of Burning", 30).map(|x| x.score),
            Some(75)
        );
        assert!(ix.best("Iron Dagger of Burning", 20).is_none());
        assert!(ix.suggest("", 50, 8).is_empty());
    }

    /// Frequent words never select candidates but a text of frequent words
    /// still finds its entries; the build can be cancelled.
    #[test]
    fn stop_words_and_cancel() {
        let mut rows: Vec<(String, String, String, String, bool)> = (0..2000)
            .map(|i| {
                (
                    "WEAP".into(),
                    "FULL".into(),
                    format!("Iron thing w{i}"),
                    format!("Chose en fer {i}"),
                    false,
                )
            })
            .collect();
        rows.push((
            "WEAP".into(),
            "FULL".into(),
            "Iron".into(),
            "Fer".into(),
            false,
        ));
        let ix = FuzzyIndex::build(rows.clone(), rows.len(), |_, _| true).unwrap();
        assert!(ix.stop.contains(&hash_word("iron")));
        let s = ix.suggest("Iron", 50, 3);
        assert_eq!(s[0].source, "Iron");
        let s = ix.suggest("Iron thing w7", 34, 3);
        assert_eq!(s[0].source, "Iron thing w7");
        assert_eq!(s[0].score, 100);
        assert!(FuzzyIndex::build(rows.clone(), rows.len(), |_, _| false).is_none());
    }
}
