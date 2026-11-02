//! Concordance in the dictionaries (A-20): a term looked for in the
//! sources (and the translations) of the active dictionaries of a file;
//! the pairs found, one per distinct (source, translation) and dictionary,
//! with the record and field of the first row and the number of rows.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::basedb::BaseDb;
use crate::dicts::Role;

/// At most this many pairs are listed (R-A20-02).
pub const MAX_PAIRS: usize = 500;
/// A term needs two characters at least.
pub const MIN_CHARS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub whole_word: bool,
    /// Look in the translations too (on by default in the window).
    pub in_trans: bool,
    /// Only the mod's own dictionary.
    pub mod_only: bool,
}

/// One dictionary of the search: its file name, role and index.
#[derive(Clone)]
pub struct Level {
    pub name: String,
    pub role: Role,
    pub db: Arc<BaseDb>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub source: String,
    pub trans: String,
    /// The dictionary's file name.
    pub dict: String,
    /// « REC:FIELD » of the first row; empty for a text-only pair.
    pub record: String,
    /// Rows of the dictionary that carry the pair.
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub pairs: Vec<Pair>,
    /// Rows read in the dictionaries.
    pub scanned: usize,
    /// More than [`MAX_PAIRS`] pairs matched.
    pub limited: bool,
    pub secs: f32,
}

/// The matcher of a term: case-insensitive « contains », or whole words;
/// `None` for a term too short.
pub fn matcher(term: &str, whole_word: bool) -> Option<regex::Regex> {
    let t = term.trim();
    if t.chars().count() < MIN_CHARS {
        return None;
    }
    let pat = if whole_word {
        format!(r"\b{}\b", regex::escape(t))
    } else {
        regex::escape(t)
    };
    regex::RegexBuilder::new(&pat)
        .case_insensitive(true)
        .build()
        .ok()
}

/// Search the levels (R-A20-01/02/03); `None` when `cancel` was raised.
pub fn search(
    levels: &[Level],
    re: &regex::Regex,
    opts: Options,
    cancel: &AtomicBool,
) -> Option<Found> {
    let start = Instant::now();
    let mut found = Found::default();
    for level in levels {
        if opts.mod_only && level.role != Role::Mod {
            continue;
        }
        // (source, translation) → index in this level's pairs.
        let mut seen: HashMap<(String, String), usize> = HashMap::new();
        let mut pairs: Vec<Pair> = Vec::new();
        let mut stopped = false;
        let mut visited = 0usize;
        let already = found.pairs.len();
        let mut limited = false;
        found.scanned += level.db.for_each_pair(|rec, field, src, t, rows| {
            visited += 1;
            if visited.is_multiple_of(16_384) && cancel.load(Ordering::Relaxed) {
                stopped = true;
                return false;
            }
            if !(re.is_match(src) || (opts.in_trans && re.is_match(t))) {
                return true;
            }
            let key = (src.trim().to_string(), t.trim().to_string());
            let record = if rec.is_empty() {
                String::new()
            } else {
                format!("{rec}:{field}")
            };
            match seen.get(&key) {
                Some(&i) => {
                    let p = &mut pairs[i];
                    p.count += rows;
                    // The smallest record, whatever the order of the index.
                    if !record.is_empty() && (p.record.is_empty() || record < p.record) {
                        p.record = record;
                    }
                }
                None => {
                    if already + pairs.len() >= MAX_PAIRS {
                        limited = true;
                        return true;
                    }
                    seen.insert(key, pairs.len());
                    pairs.push(Pair {
                        source: src.to_string(),
                        trans: t.to_string(),
                        dict: level.name.clone(),
                        record,
                        count: rows,
                    });
                }
            }
            true
        });
        if stopped || cancel.load(Ordering::Relaxed) {
            return None;
        }
        found.limited |= limited;
        pairs.sort_by(|a, b| {
            a.source
                .to_lowercase()
                .cmp(&b.source.to_lowercase())
                .then_with(|| a.trans.cmp(&b.trans))
        });
        found.pairs.extend(pairs);
    }
    found.secs = start.elapsed().as_secs_f32();
    Some(found)
}

/// The word of `text` at character `at` (R-A20: the term under the
/// cursor), `None` outside a word.
pub fn word_at(text: &str, at: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '-';
    let mut a = at.min(chars.len());
    // A cursor right after a word belongs to it.
    if (a == chars.len() || !is_w(chars[a])) && a > 0 && is_w(chars[a - 1]) {
        a -= 1;
    }
    if a >= chars.len() || !is_w(chars[a]) {
        return None;
    }
    let mut s = a;
    while s > 0 && is_w(chars[s - 1]) {
        s -= 1;
    }
    let mut e = a;
    while e < chars.len() && is_w(chars[e]) {
        e += 1;
    }
    Some(chars[s..e].iter().collect())
}

/// `piece` in place of the characters `a..b` of a text (D-53): the new
/// text and the cursor after the piece.
pub fn insert(chars: &[char], a: usize, b: usize, piece: &str) -> (String, usize) {
    let a = a.min(chars.len());
    let b = b.clamp(a, chars.len());
    let mut out: String = chars[..a].iter().collect();
    out.push_str(piece);
    out.extend(&chars[b..]);
    (out, a + piece.chars().count())
}

/// The byte ranges of the term in a text, for the highlight.
pub fn hits(re: &regex::Regex, text: &str) -> Vec<(usize, usize)> {
    re.find_iter(text).map(|m| (m.start(), m.end())).collect()
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

    fn level(name: &str, role: Role, rows: &[BaseRow]) -> Level {
        let mut db = BaseDb::default();
        db.add_rows(rows);
        Level {
            name: name.into(),
            role,
            db: Arc::new(db),
        }
    }

    #[test]
    fn pairs_found_counted_and_limited() {
        let game = level(
            "Skyrim_ENG-FRA.ximdb",
            Role::Game,
            &[
                row(
                    "MISC",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                row(
                    "CONT",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                row("INGR", "FULL", "Moon Sugar", "Sucrelune"),
                row("INFO", "NAM1", "Sugarless", "Sans sucre"),
                row("BOOK", "FULL", "Jarl Balgruuf", "Jarl Balgruuf"),
            ],
        );
        let modd = level(
            "MonMod_ENG-FRA.ximdb",
            Role::Mod,
            &[row(
                "CELL",
                "FULL",
                "Moon Sugar Refinery",
                "Raffinerie de sucrelune",
            )],
        );
        let levels = vec![game, modd];
        let no = AtomicBool::new(false);
        let re = matcher("sugar", false).unwrap();
        let f = search(&levels, &re, Options::default(), &no).unwrap();
        let srcs: Vec<&str> = f.pairs.iter().map(|p| p.source.as_str()).collect();
        assert_eq!(
            srcs,
            vec![
                "Moon Sugar",
                "Satchel of Moon Sugar",
                "Sugarless",
                "Moon Sugar Refinery"
            ]
        );
        assert_eq!(f.pairs[1].count, 2);
        assert_eq!(f.pairs[3].dict, "MonMod_ENG-FRA.ximdb");
        // Whole word: « Sugarless » goes.
        let re = matcher("sugar", true).unwrap();
        let f = search(&levels, &re, Options::default(), &no).unwrap();
        assert_eq!(f.pairs.len(), 3);
        // The mod's dictionary alone.
        let f = search(
            &levels,
            &re,
            Options {
                mod_only: true,
                ..Default::default()
            },
            &no,
        )
        .unwrap();
        assert_eq!(f.pairs.len(), 1);
        // In the translations: « sucrelune » finds pairs by their translation.
        let re = matcher("sucrelune", false).unwrap();
        assert!(
            search(&levels, &re, Options::default(), &no)
                .unwrap()
                .pairs
                .is_empty()
        );
        let f = search(
            &levels,
            &re,
            Options {
                in_trans: true,
                ..Default::default()
            },
            &no,
        )
        .unwrap();
        assert_eq!(f.pairs.len(), 3);
        // Too short; cancelled.
        assert!(matcher("a", false).is_none());
        let yes = AtomicBool::new(true);
        assert!(
            search(
                &levels,
                &matcher("jarl", false).unwrap(),
                Options::default(),
                &yes
            )
            .is_none()
        );
        // The limit.
        let many: Vec<BaseRow> = (0..MAX_PAIRS + 20)
            .map(|i| row("MISC", "FULL", &format!("Gold {i}"), &format!("Or {i}")))
            .collect();
        let big = vec![level("big", Role::Game, &many)];
        let f = search(
            &big,
            &matcher("gold", false).unwrap(),
            Options::default(),
            &no,
        )
        .unwrap();
        assert!(f.limited);
        assert_eq!(f.pairs.len(), MAX_PAIRS);
        assert_eq!(f.scanned, MAX_PAIRS + 20);
    }

    #[test]
    fn word_under_the_cursor() {
        assert_eq!(
            word_at("Satchel of Moon Sugar", 12).as_deref(),
            Some("Moon")
        );
        assert_eq!(
            word_at("Satchel of Moon Sugar", 15).as_deref(),
            Some("Moon")
        );
        assert_eq!(
            word_at("Satchel of Moon Sugar", 21).as_deref(),
            Some("Sugar")
        );
        assert_eq!(word_at("a  b", 2), None);
        assert_eq!(word_at("l'arbre", 3).as_deref(), Some("l'arbre"));
    }

    #[test]
    fn piece_inserted() {
        let c: Vec<char> = "Sacoche de sucre".chars().collect();
        assert_eq!(
            insert(&c, 11, 16, "sucrelune"),
            ("Sacoche de sucrelune".into(), 20)
        );
        assert_eq!(insert(&c, 16, 16, " !"), ("Sacoche de sucre !".into(), 18));
        assert_eq!(insert(&c, 0, 0, "La "), ("La Sacoche de sucre".into(), 3));
    }
}
