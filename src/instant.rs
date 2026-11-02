//! Traduction instantanée (B-14): a piece of the source — or all of it —
//! translated by assembling the fragments the active dictionaries know,
//! text-only and personal pairs first. The whole text is tried first, then
//! a greedy pass takes, at each point, the longest known fragment that
//! starts there; a word no fragment covers is copied and counted unknown;
//! tags, numbers and punctuation are copied.

use std::collections::HashMap;
use std::sync::Arc;

use crate::basedb::BaseDb;
use crate::dicts::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Word,
    Tag,
    Num,
    Punct,
    Space,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: Kind,
    pub(crate) text: String,
    /// What is compared: lowercase for words and tags, as is otherwise.
    pub(crate) key: String,
}

/// The tokens of a text: words, tags, numbers, punctuation, spaces (the
/// cutting of F9, shared by the glossary, B-15).
pub(crate) fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let push = |out: &mut Vec<Token>, kind: Kind, s: String| {
        let key = match kind {
            Kind::Word | Kind::Tag => s.to_lowercase(),
            _ => s.clone(),
        };
        out.push(Token { kind, text: s, key });
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            let start = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            push(&mut out, Kind::Space, chars[start..i].iter().collect());
        } else if c == '<' {
            // A tag of the engine, up to its « > » (or the end of the text).
            let start = i;
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            push(&mut out, Kind::Tag, chars[start..i].iter().collect());
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | ',' | '%'))
            {
                i += 1;
            }
            push(&mut out, Kind::Num, chars[start..i].iter().collect());
        } else if c.is_alphanumeric() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || matches!(chars[i], '\'' | '’' | '-' | '_')
                        && i + 1 < chars.len()
                        && chars[i + 1].is_alphanumeric())
            {
                i += 1;
            }
            push(&mut out, Kind::Word, chars[start..i].iter().collect());
        } else {
            i += 1;
            push(&mut out, Kind::Punct, c.to_string());
        }
    }
    out
}

/// A known pair, tokenised.
#[derive(Debug, Clone)]
struct Entry {
    keys: Vec<String>,
    source: String,
    trans: String,
}

/// The fragments of one dictionary, by first token.
#[derive(Debug, Default)]
pub struct FragmentIndex {
    by_first: HashMap<String, Vec<Entry>>,
    len: usize,
}

impl FragmentIndex {
    /// Every current row of the dictionary, signed or not.
    pub fn build(db: &BaseDb) -> FragmentIndex {
        let mut ix = FragmentIndex::default();
        for (src, tr) in db.fragment_pairs() {
            ix.add(src, tr);
        }
        ix
    }

    pub fn add(&mut self, source: &str, trans: &str) {
        let keys: Vec<String> = tokens(source.trim())
            .into_iter()
            .filter(|t| t.kind != Kind::Space)
            .map(|t| t.key)
            .collect();
        let Some(first) = keys.first().cloned() else {
            return;
        };
        let list = self.by_first.entry(first).or_default();
        if list.iter().any(|e| e.keys == keys) {
            return;
        }
        list.push(Entry {
            keys,
            source: source.trim().to_string(),
            trans: trans.trim().to_string(),
        });
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The longest entry whose keys match `keys` from `at`: (entry, number
    /// of keys consumed).
    fn longest_at(&self, keys: &[&str], at: usize) -> Option<(&Entry, usize)> {
        let list = self.by_first.get(keys[at])?;
        let mut best: Option<(&Entry, usize)> = None;
        for e in list {
            let n = e.keys.len();
            if at + n > keys.len() {
                continue;
            }
            if e.keys.iter().zip(&keys[at..at + n]).all(|(a, b)| a == b)
                && best.is_none_or(|(_, bn)| n > bn)
            {
                best = Some((e, n));
            }
        }
        best
    }
}

/// One dictionary able to give fragments, in the order they are consulted.
pub struct Level {
    pub name: String,
    pub role: Role,
    pub index: Arc<FragmentIndex>,
}

/// Text-only and personal dictionaries first, then the others in their
/// order (R-B14-02).
pub fn order_levels(mut levels: Vec<Level>) -> Vec<Level> {
    let rank = |l: &Level| match l.role {
        Role::TextOnly => 0,
        Role::Personal => 1,
        _ => 2,
    };
    levels.sort_by_key(rank);
    levels
}

/// A fragment used by a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: String,
    pub trans: String,
    pub origin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    pub fragments: Vec<Found>,
    pub unknown: Vec<String>,
}

impl Outcome {
    /// Share of the source, in characters, that the fragments covered (the
    /// score of the provenance « heuristique », B-09).
    pub fn coverage(&self) -> u8 {
        let known: usize = self
            .fragments
            .iter()
            .map(|f| f.source.chars().count())
            .sum();
        let unknown: usize = self.unknown.iter().map(|u| u.chars().count()).sum();
        if known + unknown == 0 {
            return 0;
        }
        ((known * 100) / (known + unknown)).min(100) as u8
    }
}

/// Translate `text` with the fragments of `levels` (R-B14-02). The result
/// keeps the spacing of the source around what it replaces.
pub fn translate(text: &str, levels: &[Level]) -> Outcome {
    let toks = tokens(text);
    let solid: Vec<usize> = (0..toks.len())
        .filter(|&i| toks[i].kind != Kind::Space)
        .collect();
    let keys: Vec<&str> = solid.iter().map(|&i| toks[i].key.as_str()).collect();
    let mut out = Outcome::default();
    let mut s = 0; // index in `solid`
    let mut t = 0; // index in `toks`
    while t < toks.len() {
        if toks[t].kind == Kind::Space {
            out.text.push_str(&toks[t].text);
            t += 1;
            continue;
        }
        // The longest fragment here, across the levels (ties: first level).
        let mut best: Option<(&Entry, usize, &str)> = None;
        for l in levels {
            if let Some((e, n)) = l.index.longest_at(&keys, s)
                && best.is_none_or(|(_, bn, _)| n > bn)
            {
                best = Some((e, n, l.name.as_str()));
            }
        }
        match best {
            Some((e, n, origin)) => {
                out.text.push_str(&e.trans);
                out.fragments.push(Found {
                    source: e.source.clone(),
                    trans: e.trans.clone(),
                    origin: origin.to_string(),
                });
                s += n;
                // Past the last consumed token; the space after it is
                // emitted by the loop.
                t = solid[s - 1] + 1;
            }
            None => {
                let tk = &toks[t];
                out.text.push_str(&tk.text);
                if tk.kind == Kind::Word {
                    out.unknown.push(tk.text.clone());
                }
                s += 1;
                t += 1;
            }
        }
    }
    out
}

/// The fragments the levels know in `text`, in reading order, each once
/// (the Vocabulaire tab, R-B14-05).
pub fn known_fragments(text: &str, levels: &[Level]) -> Vec<Found> {
    let toks = tokens(text);
    let solid: Vec<usize> = (0..toks.len())
        .filter(|&i| toks[i].kind != Kind::Space)
        .collect();
    let keys: Vec<&str> = solid.iter().map(|&i| toks[i].key.as_str()).collect();
    let mut out: Vec<Found> = Vec::new();
    for s in 0..keys.len() {
        for l in levels {
            if let Some(list) = l.index.by_first.get(keys[s]) {
                for e in list {
                    let n = e.keys.len();
                    if s + n <= keys.len()
                        && e.keys.iter().zip(&keys[s..s + n]).all(|(a, b)| a == b)
                        && !out
                            .iter()
                            .any(|f| f.source == e.source && f.trans == e.trans)
                    {
                        out.push(Found {
                            source: e.source.clone(),
                            trans: e.trans.clone(),
                            origin: l.name.clone(),
                        });
                    }
                }
            }
        }
    }
    out
}

fn first_alpha(s: &str) -> Option<(usize, char)> {
    s.char_indices().find(|(_, c)| c.is_alphabetic())
}

fn set_first_case(s: &str, upper: bool) -> String {
    match first_alpha(s) {
        Some((i, c)) => {
            let mut r = String::with_capacity(s.len());
            r.push_str(&s[..i]);
            if upper {
                r.extend(c.to_uppercase());
            } else {
                r.extend(c.to_lowercase());
            }
            r.push_str(&s[i + c.len_utf8()..]);
            r
        }
        None => s.to_string(),
    }
}

/// The case of the first letter (R-B14-02, T-B14-02): at the start of the
/// translation, that of the source; inserted further, a fragment taken
/// from the start of the source loses its sentence capital; otherwise the
/// fragment keeps its own case.
pub fn adjust_case(
    result: &str,
    source: &str,
    dest_at_start: bool,
    source_at_start: bool,
) -> String {
    if dest_at_start {
        match first_alpha(source) {
            Some((_, c)) => set_first_case(result, c.is_uppercase()),
            None => result.to_string(),
        }
    } else if source_at_start {
        set_first_case(result, false)
    } else {
        result.to_string()
    }
}

/// Whether the position `at` (in chars) of `text` starts a sentence: the
/// start of the text, or only spaces since a « . », « ! », « ? », « … » or
/// a line break.
pub fn at_sentence_start(text: &str, at: usize) -> bool {
    let before: Vec<char> = text.chars().take(at).collect();
    for c in before.iter().rev() {
        if c.is_whitespace() {
            if *c == '\n' {
                return true;
            }
            continue;
        }
        return matches!(c, '.' | '!' | '?' | '…' | ':');
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(pairs: &[(&str, &str)]) -> Arc<FragmentIndex> {
        let mut ix = FragmentIndex::default();
        for (s, t) in pairs {
            ix.add(s, t);
        }
        Arc::new(ix)
    }

    fn levels() -> Vec<Level> {
        order_levels(vec![
            Level {
                name: "Skyrim_ENG-FRA.ximdb".into(),
                role: Role::Game,
                index: index(&[
                    ("Steel Sword", "Épée en acier"),
                    ("Iron Dagger", "Dague en fer"),
                    ("Legendary", "Légendaire"),
                    ("Press E to", "Appuyez sur E pour"),
                    ("the chest", "le coffre"),
                ]),
            },
            Level {
                name: "termes_ENG-FRA.ximdb".into(),
                role: Role::TextOnly,
                index: index(&[("of Burning", "de feu")]),
            },
        ])
    }

    /// T-B14-01: the whole source from two fragments.
    #[test]
    fn two_fragments() {
        let lv = levels();
        assert_eq!(lv[0].role, Role::TextOnly, "text-only first");
        let o = translate("Steel Sword of Burning", &lv);
        assert_eq!(o.text, "Épée en acier de feu");
        assert_eq!(o.fragments.len(), 2);
        assert!(o.unknown.is_empty());
        assert_eq!(o.fragments[1].origin, "termes_ENG-FRA.ximdb");
    }

    /// T-B14-03: an unknown word is copied and counted.
    #[test]
    fn unknown_word_is_copied() {
        let o = translate("Press E to open the chest.", &levels());
        assert_eq!(o.text, "Appuyez sur E pour open le coffre.");
        assert_eq!(o.fragments.len(), 2);
        assert_eq!(o.unknown, vec!["open".to_string()]);
    }

    /// Case-insensitive match, tags and numbers copied, nothing known.
    #[test]
    fn tags_numbers_and_case() {
        let lv = levels();
        let o = translate("<Alias=Player> steel sword 3", &lv);
        assert_eq!(o.text, "<Alias=Player> Épée en acier 3");
        assert!(o.unknown.is_empty());
        let o = translate("Nothing here", &lv);
        assert_eq!(o.text, "Nothing here");
        assert!(o.fragments.is_empty());
        assert_eq!(o.unknown.len(), 2);
    }

    /// T-B14-04: greedy on « Iron Dagger Legendary ».
    #[test]
    fn greedy_longest_first() {
        let o = translate("Iron Dagger Legendary", &levels());
        assert_eq!(o.text, "Dague en fer Légendaire");
        let f = known_fragments("Iron Dagger Legendary of Burning", &levels());
        let s: Vec<&str> = f.iter().map(|f| f.source.as_str()).collect();
        assert_eq!(s, vec!["Iron Dagger", "Legendary", "of Burning"]);
    }

    /// T-B14-02: the sentence capital follows the destination.
    #[test]
    fn case_follows_the_destination() {
        assert_eq!(
            adjust_case("Épée en acier", "Steel Sword", false, true),
            "épée en acier"
        );
        assert_eq!(
            adjust_case("épée en acier", "Steel Sword", true, true),
            "Épée en acier"
        );
        assert_eq!(adjust_case("Dwemer", "Dwemer", false, false), "Dwemer");
        assert_eq!(adjust_case("Épée", "steel", true, true), "épée");
        assert!(at_sentence_start("Une. ", 5));
        assert!(!at_sentence_start("Une ", 4));
        assert!(at_sentence_start("", 0));
        assert!(at_sentence_start("Ligne\n", 6));
    }
}
