//! Transformations of a translation (A-13): upper case, lower case, title
//! case, first letter in capital, spaces at the ends, a prefix or a suffix
//! added or removed.
//!
//! The text is cut into segments first: engine tags (`<Alias=…>`, `<mag>`,
//! any `<…>`), numbers, `%s`-like placeholders and line breaks are kept as
//! they are; the transformation applies to the text between them.

use crate::tags::{self, TagSet};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// The commands of Transformer ▸ (R-A13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Upper,
    Lower,
    Title,
    Sentence,
    Trim,
    AffixAdd,
    AffixRemove,
}

impl Command {
    /// The commands in the order of the menu.
    pub const ALL: [Command; 7] = [
        Command::Upper,
        Command::Lower,
        Command::Title,
        Command::Sentence,
        Command::Trim,
        Command::AffixAdd,
        Command::AffixRemove,
    ];

    /// The message key of the command's name.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Command::Upper => "transform-upper",
            Command::Lower => "transform-lower",
            Command::Title => "transform-title",
            Command::Sentence => "transform-sentence",
            Command::Trim => "transform-trim",
            Command::AffixAdd => "transform-affix-add",
            Command::AffixRemove => "transform-affix-remove",
        }
    }

    /// The commands that need the prefix / suffix dialog.
    pub fn needs_dialog(self) -> bool {
        matches!(self, Command::AffixAdd | Command::AffixRemove)
    }
}

/// A piece of a text: words to transform, or a segment kept as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Kept(&'a str),
}

/// Cut a text into its pieces (R-A13-02).
pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut spans: Vec<(usize, usize)> = tags::tags(text, &TagSet::any())
        .into_iter()
        .map(|s| (s.start, s.end))
        .collect();
    spans.extend(
        tags::numbers(text, &TagSet::any())
            .into_iter()
            .map(|s| (s.start, s.end)),
    );
    // `%s`, `%d`, `%1`: placeholders of formatted strings.
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'%' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j > i + 1 {
                spans.push((i, j));
            }
        }
        if *b == b'\n' || *b == b'\r' {
            spans.push((i, i + 1));
        }
    }
    spans.sort_unstable();
    let mut out = Vec::new();
    let mut at = 0;
    for (s, e) in spans {
        if s < at {
            // Inside a span already kept (a number in a tag).
            if e > at {
                out.push(Piece::Kept(&text[at..e]));
                at = e;
            }
            continue;
        }
        if s > at {
            out.push(Piece::Text(&text[at..s]));
        }
        out.push(Piece::Kept(&text[s..e]));
        at = e;
    }
    if at < text.len() {
        out.push(Piece::Text(&text[at..]));
    }
    out
}

/// Apply `f` to the text pieces and put the whole together again.
fn map_text(text: &str, mut f: impl FnMut(&str) -> String) -> String {
    pieces(text)
        .into_iter()
        .map(|p| match p {
            Piece::Text(t) => f(t),
            Piece::Kept(k) => k.to_string(),
        })
        .collect()
}

/// UPPER CASE (Rust's full Unicode mappings: « ß » → « SS »).
pub fn upper(text: &str) -> String {
    map_text(text, str::to_uppercase)
}

/// lower case.
pub fn lower(text: &str) -> String {
    map_text(text, str::to_lowercase)
}

/// The first letter of the text in capital, nothing else touched: after a
/// kept segment or an opening punctuation.
pub fn sentence(text: &str) -> String {
    let mut done = false;
    map_text(text, |t| {
        if done {
            return t.to_string();
        }
        let mut out = String::with_capacity(t.len());
        for c in t.chars() {
            if !done && c.is_alphabetic() {
                out.extend(c.to_uppercase());
                done = true;
            } else {
                out.push(c);
            }
        }
        out
    })
}

/// Spaces, tabulations and non-breaking spaces at both ends removed; line
/// breaks kept (R-A13-06).
pub fn trim(text: &str) -> String {
    let blank = |c: char| matches!(c, ' ' | '\t' | '\u{a0}' | '\u{202f}' | '\u{2007}');
    text.trim_start_matches(blank)
        .trim_end_matches(blank)
        .to_string()
}

/// The words a title leaves in lower case and those kept as written, for
/// one language.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleRules {
    pub lower: Vec<String>,
    pub keep: Vec<String>,
}

/// Title Case (R-A13-04): each word gets a capital and the rest in lower
/// case, except the words of `rules.lower` — unless first in the text or
/// after « : . ! ? — » or a line break — and the acronyms of `rules.keep`
/// written in capitals.
pub fn title(text: &str, rules: &TitleRules) -> String {
    let lower: HashSet<String> = rules.lower.iter().map(|w| w.to_lowercase()).collect();
    let keep: HashSet<&str> = rules.keep.iter().map(String::as_str).collect();
    let mut start = true;
    let mut out = String::with_capacity(text.len());
    for p in pieces(text) {
        let t = match p {
            Piece::Kept(k) => {
                if k == "\n" || k == "\r" {
                    start = true;
                }
                out.push_str(k);
                continue;
            }
            Piece::Text(t) => t,
        };
        let chars: Vec<char> = t.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if !c.is_alphanumeric() {
                if matches!(c, ':' | '.' | '!' | '?' | '—') {
                    start = true;
                }
                out.push(c);
                i += 1;
                continue;
            }
            let from = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            let word: String = chars[from..i].iter().collect();
            let lw = word.to_lowercase();
            // « l' », « d' »: the word with the apostrophe that follows.
            let elided = chars
                .get(i)
                .is_some_and(|a| *a == '\'' || *a == '’')
                .then(|| format!("{lw}'"));
            let is_lower =
                lower.contains(&lw) || elided.as_ref().is_some_and(|e| lower.contains(e));
            let acronym = word.chars().count() >= 2
                && word.chars().all(|c| !c.is_lowercase())
                && keep.contains(word.as_str());
            if acronym {
                out.push_str(&word);
            } else if is_lower && !start {
                out.push_str(&lw);
            } else {
                let mut cs = lw.chars();
                if let Some(f) = cs.next() {
                    out.extend(f.to_uppercase());
                    out.push_str(cs.as_str());
                }
            }
            start = false;
        }
    }
    out
}

/// Apply one of the five direct commands; `None` for the prefix /
/// suffix ones (see [`affix_add`] and [`affix_remove`]).
pub fn apply(cmd: Command, text: &str, rules: &TitleRules) -> Option<String> {
    Some(match cmd {
        Command::Upper => upper(text),
        Command::Lower => lower(text),
        Command::Title => title(text, rules),
        Command::Sentence => sentence(text),
        Command::Trim => trim(text),
        Command::AffixAdd | Command::AffixRemove => return None,
    })
}

/// Add a prefix and a suffix (R-A13-07), as typed, spaces included; with
/// `no_repeat`, a text that already starts with the prefix or ends with the
/// suffix is left (`None`).
pub fn affix_add(text: &str, prefix: &str, suffix: &str, no_repeat: bool) -> Option<String> {
    if prefix.is_empty() && suffix.is_empty() {
        return None;
    }
    if no_repeat
        && ((!prefix.is_empty() && text.starts_with(prefix))
            || (!suffix.is_empty() && text.ends_with(suffix)))
    {
        return None;
    }
    Some(format!("{prefix}{text}{suffix}"))
}

/// Remove the prefix and / or the suffix when present; `None` when neither
/// is.
pub fn affix_remove(text: &str, prefix: &str, suffix: &str) -> Option<String> {
    let mut t = text;
    let mut changed = false;
    if !prefix.is_empty()
        && let Some(r) = t.strip_prefix(prefix)
    {
        t = r;
        changed = true;
    }
    if !suffix.is_empty()
        && let Some(r) = t.strip_suffix(suffix)
    {
        t = r;
        changed = true;
    }
    changed.then(|| t.to_string())
}

/// The exception lists of every language (D-112): `assets/data/text/
/// title_case.json`, overridden language by language by the user's copy
/// `<config>/data/text/title_case.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleLists {
    pub langs: BTreeMap<String, TitleRules>,
}

const SHIPPED: &str = include_str!("../../assets/data/text/title_case.json");

impl TitleLists {
    /// Read a `title_case.json` (format 1): `{ "format": 1, "<lang>":
    /// { "lower": [...], "keep": [...] } }`.
    pub fn parse(text: &str) -> Result<TitleLists, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or_else(|| "not an object".to_string())?;
        if obj.get("format").and_then(|f| f.as_u64()) != Some(1) {
            return Err("format".to_string());
        }
        let list = |v: Option<&serde_json::Value>| -> Vec<String> {
            v.and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut langs = BTreeMap::new();
        for (k, v) in obj {
            if k == "format" {
                continue;
            }
            langs.insert(
                k.to_ascii_lowercase(),
                TitleRules {
                    lower: list(v.get("lower")),
                    keep: list(v.get("keep")),
                },
            );
        }
        Ok(TitleLists { langs })
    }

    /// The JSON of the lists.
    pub fn to_json(&self) -> String {
        let mut m = serde_json::Map::new();
        m.insert("format".into(), 1.into());
        for (k, r) in &self.langs {
            m.insert(
                k.clone(),
                serde_json::json!({ "lower": r.lower, "keep": r.keep }),
            );
        }
        let mut s = serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap_or_default();
        s.push('\n');
        s
    }

    /// The shipped lists (the file, else the copy built in).
    pub fn shipped() -> TitleLists {
        std::fs::read_to_string(crate::config::assets_data_dir().join("text/title_case.json"))
            .ok()
            .and_then(|t| TitleLists::parse(&t).ok())
            .or_else(|| TitleLists::parse(SHIPPED).ok())
            .unwrap_or_default()
    }

    /// The user's copy, where the settings write.
    pub fn user_path() -> PathBuf {
        crate::config::paths::get()
            .data_dir
            .join("text")
            .join("title_case.json")
    }

    /// The lists in force: the shipped ones, each language of the user's
    /// copy replacing the shipped one.
    pub fn load() -> TitleLists {
        let mut lists = TitleLists::shipped();
        if let Some(user) = std::fs::read_to_string(Self::user_path())
            .ok()
            .and_then(|t| TitleLists::parse(&t).ok())
        {
            lists.langs.extend(user.langs);
        }
        lists
    }

    /// The rules of a language; none for a language without list.
    pub fn rules(&self, lang: &str) -> TitleRules {
        self.langs
            .get(&lang.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// Write the rules of `lang` into the user's copy (the other languages
    /// of the copy kept).
    pub fn save_user(lang: &str, rules: TitleRules) -> std::io::Result<()> {
        let path = Self::user_path();
        let mut user = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| TitleLists::parse(&t).ok())
            .unwrap_or_default();
        user.langs.insert(lang.to_ascii_lowercase(), rules);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::config::paths::write_atomic(&path, user.to_json().as_bytes())
    }
}

/// A list typed in the settings: words separated by commas or blanks.
pub fn parse_words(text: &str) -> Vec<String> {
    text.split([',', ';', '\n', ' ', '\t'])
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fra() -> TitleRules {
        TitleLists::parse(SHIPPED).unwrap().rules("fra")
    }

    #[test]
    fn tags_numbers_placeholders_and_breaks_are_kept() {
        assert_eq!(
            upper("Invoque une épée liée pendant <mag> secondes."),
            "INVOQUE UNE ÉPÉE LIÉE PENDANT <mag> SECONDES."
        );
        assert_eq!(
            upper("<Alias=Player> a 25 pièces, %s reste\nici"),
            "<Alias=Player> A 25 PIÈCES, %s RESTE\nICI"
        );
        assert_eq!(lower("STRASSE <Global=X>"), "strasse <Global=X>");
        assert_eq!(upper("straße"), "STRASSE");
    }

    #[test]
    fn title_case_follows_the_french_exceptions() {
        let r = fra();
        assert_eq!(title("Le pouvoir du savoir", &r), "Le Pouvoir du Savoir");
        assert_eq!(title("Choix de la Valkyrie", &r), "Choix de la Valkyrie");
        assert_eq!(title("Marque de l'Atronach", &r), "Marque de l'Atronach");
        assert_eq!(
            title("de la forge : le marteau", &r),
            "De la Forge : Le Marteau"
        );
        assert_eq!(title("Couronne IV du roi", &r), "Couronne IV du Roi");
        // A language without list has no exception.
        assert_eq!(
            title("le pouvoir du savoir", &TitleRules::default()),
            "Le Pouvoir Du Savoir"
        );
        let mut with_sword = r.clone();
        with_sword.lower.push("épée".into());
        assert_eq!(
            title("parchemin de l'épée liée", &with_sword),
            "Parchemin de l'épée Liée"
        );
    }

    #[test]
    fn sentence_trim_and_affixes() {
        assert_eq!(sentence("« épée liée »"), "« Épée liée »");
        assert_eq!(sentence("<Alias=Player> arrive"), "<Alias=Player> Arrive");
        assert_eq!(trim("  Épée\u{a0}\t"), "Épée");
        assert_eq!(trim("Livre \n"), "Livre \n");
        assert_eq!(trim("a  b"), "a  b");
        let t = "Épée en fer";
        let added = affix_add(t, "[Apo] ", "", true).unwrap();
        assert_eq!(added, "[Apo] Épée en fer");
        assert_eq!(affix_add(&added, "[Apo] ", "", true), None);
        assert_eq!(affix_remove(&added, "[Apo] ", "").unwrap(), t);
        assert_eq!(affix_remove(t, "[Apo] ", ""), None);
        assert_eq!(affix_add(t, "", " (2)", false).unwrap(), "Épée en fer (2)");
    }

    #[test]
    fn the_lists_round_trip_and_cover_the_interface() {
        let l = TitleLists::parse(SHIPPED).unwrap();
        for code in ["fra", "eng", "deu", "spa", "ita"] {
            assert!(!l.rules(code).lower.is_empty(), "{code}");
        }
        assert_eq!(TitleLists::parse(&l.to_json()).unwrap(), l);
        assert!(TitleLists::parse("{\"format\": 2}").is_err());
        assert_eq!(parse_words("de, du ;la\nle"), vec!["de", "du", "la", "le"]);
    }
}
