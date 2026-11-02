//! Spell checking (CDC §8.3), the cross-platform replacement for xTranslator's
//! `LibHunspell.pas`.
//!
//! The reference binds the native `libhunspell.dll` (Windows-only, a shipped C++
//! DLL). To stay multiplatform and free of any proprietary/native dependency we
//! use [`spellbook`], a pure-Rust reimplementation of Hunspell that reads the
//! very same `.aff`/`.dic` dictionaries (OpenOffice / LibreOffice / Mozilla).
//! It is licensed MPL-2.0, exactly like Hunspell itself.
//!
//! The public surface mirrors the Pascal `THunspellChecker`: initialize from an
//! affix + dictionary pair, ask whether a word is spelled correctly, and get
//! suggestions — plus everything a translator needs on top: dictionary discovery
//! by folder, resolution from the target language code, a persistent personal
//! dictionary, a per-session ignore list, and a tokenizer (see [`tokenize`])
//! that understands Bethesda tags and placeholders.

pub mod tokenize;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use tokenize::Token;

/// A misspelled word found in a string, with its byte span `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misspelling {
    pub word: String,
    pub start: usize,
    pub end: usize,
}

/// A discovered dictionary on disk: a `.dic` with its matching `.aff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictInfo {
    /// File stem, e.g. `"fr_FR"` or `"en_US"`.
    pub code: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
}

impl DictInfo {
    /// The language part before the region, lowercased (`"fr_FR"` → `"fr"`).
    pub fn base(&self) -> String {
        let lower = self.code.to_lowercase();
        lower.split(['_', '-']).next().unwrap_or(&lower).to_string()
    }
}

/// Locate the dictionaries directory (`assets/dictionaries`), trying the
/// production layout (next to the executable, macOS bundle) then the dev one.
pub fn dictionaries_dir() -> Option<PathBuf> {
    let rel = PathBuf::from("assets").join("dictionaries");
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&rel));
        candidates.push(dir.join("dictionaries"));
        if let Some(up) = dir.parent() {
            candidates.push(up.join("Resources").join(&rel));
            candidates.push(up.join("Resources").join("dictionaries"));
        }
    }
    candidates.push(rel);
    candidates.push(PathBuf::from("dictionaries"));
    candidates.into_iter().find(|p| p.is_dir())
}

/// Scan a directory for every `*.dic` that has a matching `*.aff` (the Pascal
/// `FindDictEx`), sorted by code. Missing directory → empty list.
pub fn scan_dictionaries(dir: &Path) -> Vec<DictInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let dic = entry.path();
        if dic
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("dic"))
            != Some(true)
        {
            continue;
        }
        let aff = dic.with_extension("aff");
        if !aff.is_file() {
            continue;
        }
        if let Some(code) = dic.file_stem().and_then(|s| s.to_str()) {
            out.push(DictInfo {
                code: code.to_string(),
                aff,
                dic,
            });
        }
    }
    out.sort_by_key(|d| d.code.to_lowercase());
    out
}

/// The installed dictionaries (empty when the folder is absent).
pub fn available_dictionaries() -> Vec<DictInfo> {
    dictionaries_dir()
        .map(|d| scan_dictionaries(&d))
        .unwrap_or_default()
}

/// Pick the dictionary matching a language, given its ISO 639-3 code (e.g.
/// `"fra"`). Tries the 639-3 code, its 639-1 code (`"fr"`) and any regional
/// variant (`fr_FR`, `fr-CA`). An exact-code match wins over a base match.
pub fn resolve_for_language<'a>(iso3: &str, dicts: &'a [DictInfo]) -> Option<&'a DictInfo> {
    let iso3 = iso3.to_lowercase();
    let iso1 = crate::i18n::iso639_3_to_1(&iso3);
    let keys: Vec<String> = {
        let mut k = vec![iso3.clone()];
        // `iso639_3_to_1` falls back to "en" for unknown codes; only trust the
        // 2-letter code when it is a genuine mapping (not that default guess).
        let genuine = iso1 != "en" || matches!(iso3.as_str(), "eng" | "en");
        if genuine && iso1 != iso3 {
            k.push(iso1);
        }
        k
    };
    // 1) Exact code match (case-insensitive): code == key.
    for d in dicts {
        let lc = d.code.to_lowercase();
        if keys.contains(&lc) {
            return Some(d);
        }
    }
    // 2) Base-language match: fr_FR / fr-CA → "fr". Several regional
    //    dictionaries may share a base (en_AU, en_GB, en_US…): prefer the
    //    region of `preferred_region`, then the region that repeats the
    //    language (fr_FR, de_DE), then a bare code (ar), then the first one
    //    in alphabetical order, so the choice never depends on the folder's
    //    reading order.
    let mut candidates: Vec<&DictInfo> =
        dicts.iter().filter(|d| keys.contains(&d.base())).collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|d| d.code.to_lowercase());
    let base = candidates[0].base();
    let region_of = |d: &DictInfo| -> String {
        let lower = d.code.to_lowercase();
        lower
            .split(['_', '-'])
            .nth(1)
            .map(|r| r.to_uppercase())
            .unwrap_or_default()
    };
    if let Some(pref) = preferred_region(&base)
        && let Some(d) = candidates.iter().find(|d| region_of(d) == pref)
    {
        return Some(d);
    }
    if let Some(d) = candidates
        .iter()
        .find(|d| region_of(d) == base.to_uppercase())
    {
        return Some(d);
    }
    if let Some(d) = candidates.iter().find(|d| region_of(d).is_empty()) {
        return Some(d);
    }
    candidates.first().copied()
}

/// The regional dictionary to prefer when a language has several
/// (`en` → `en_US`, `pt` → `pt_BR`, `sv` → `sv_SE`…). Languages whose usual
/// dictionary repeats the language code (fr_FR, de_DE, it_IT…) need no entry.
fn preferred_region(base: &str) -> Option<&'static str> {
    Some(match base {
        "en" => "US",
        "pt" => "BR",
        "sv" => "SE",
        "da" => "DK",
        "cs" => "CZ",
        "el" => "GR",
        "ko" => "KR",
        "ja" => "JP",
        "zh" => "CN",
        "uk" => "UA",
        "be" => "BY",
        "sr" => "RS",
        "bs" => "BA",
        "sl" => "SI",
        "et" => "EE",
        "nb" | "nn" | "no" => "NO",
        "ca" | "eu" | "gl" | "an" => "ES",
        "ga" => "IE",
        "cy" | "gd" => "GB",
        "ar" => "SA",
        "fa" => "IR",
        "he" => "IL",
        "hi" | "bn" | "ta" | "te" | "mr" | "gu" | "pa" => "IN",
        "vi" => "VN",
        "ms" => "MY",
        "sw" => "KE",
        "af" => "ZA",
        _ => return None,
    })
}

/// A ready-to-use spell checker for one language.
pub struct SpellChecker {
    dict: spellbook::Dictionary,
    /// Dictionary code actually loaded (`"fr_FR"`).
    code: String,
    /// Target-language ISO 639-3 this checker serves.
    language: String,
    /// User-added words (lowercased), also merged into `dict`.
    personal: HashSet<String>,
    /// Words ignored for this session only (lowercased).
    ignored: HashSet<String>,
    /// Where the personal dictionary is persisted, if known.
    personal_path: Option<PathBuf>,
}

impl SpellChecker {
    /// Build a checker from an affix + dictionary pair already in memory. Used
    /// by tests and by [`Self::from_files`].
    pub fn from_sources(
        aff: &str,
        dic: &str,
        code: impl Into<String>,
        language: impl Into<String>,
    ) -> Result<Self, String> {
        let dict = spellbook::Dictionary::new(aff, dic)
            .map_err(|e| crate::l10n!("err-spell-dict", error = e))?;
        Ok(Self {
            dict,
            code: code.into(),
            language: language.into(),
            personal: HashSet::new(),
            ignored: HashSet::new(),
            personal_path: None,
        })
    }

    /// Build a checker by reading a dictionary pair from disk.
    pub fn from_files(info: &DictInfo, language: &str) -> Result<Self, String> {
        let aff = std::fs::read_to_string(&info.aff)
            .map_err(|e| crate::l10n!("err-read-path", path = info.aff.display(), error = e))?;
        let dic = std::fs::read_to_string(&info.dic)
            .map_err(|e| crate::l10n!("err-read-path", path = info.dic.display(), error = e))?;
        Self::from_sources(&aff, &dic, info.code.clone(), language)
    }

    /// Discover the installed dictionaries and load the one for `iso3`, then
    /// merge the persistent personal dictionary. Returns `None` when no matching
    /// dictionary is installed.
    pub fn load_for_language(iso3: &str) -> Option<Self> {
        let dicts = available_dictionaries();
        let info = resolve_for_language(iso3, &dicts)?;
        let mut checker = Self::from_files(info, iso3).ok()?;
        checker.attach_personal(personal_dict_path(iso3));
        Some(checker)
    }

    /// The loaded dictionary code (`"fr_FR"`).
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The target language served (ISO 639-3).
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Attach a personal-dictionary file and load whatever words it holds.
    pub fn attach_personal(&mut self, path: Option<PathBuf>) {
        self.personal_path = path;
        if let Some(p) = &self.personal_path
            && let Ok(content) = std::fs::read_to_string(p)
        {
            for line in content.lines() {
                let w = line.trim();
                if !w.is_empty() {
                    self.register_personal(w);
                }
            }
        }
    }

    /// Add a word to the checker's personal set + the live dictionary (no I/O).
    fn register_personal(&mut self, word: &str) {
        self.personal.insert(word.to_lowercase());
        let _ = self.dict.add(word);
    }

    /// Try the word as written, then lowercased (so `SWORD` / `Sword` match a
    /// lowercase dictionary entry).
    fn dict_ok(&self, w: &str) -> bool {
        if self.dict.check(w) {
            return true;
        }
        let lower = w.to_lowercase();
        lower != w && self.dict.check(&lower)
    }

    /// Whether a bare word is spelled correctly, handling elisions and
    /// hyphenated compounds by splitting: `l'épée`, `well-known`, `qu'il`,
    /// `sword's` are accepted when their significant parts are. Contraction /
    /// elision particles (≤ 2 letters, e.g. `l'`, `d'`, `qu'`, `'s`) don't need
    /// to be words themselves.
    pub fn is_correct(&self, word: &str) -> bool {
        if word.chars().filter(|c| c.is_alphabetic()).count() < 2 {
            return true;
        }
        if self.dict_ok(word) {
            return true;
        }
        let segs: Vec<&str> = word
            .split(['\'', '\u{2019}', '-'])
            .filter(|s| !s.is_empty())
            .collect();
        if segs.len() <= 1 {
            return false;
        }
        let mut any_significant = false;
        for seg in segs {
            if self.dict_ok(seg) {
                any_significant = true; // a real word (any length)
            } else if seg.chars().count() <= 2 {
                continue; // short unknown → elision / contraction particle
            } else {
                return false; // a significant segment that is misspelled
            }
        }
        any_significant
    }

    /// Whether the checker accepts the word: correctly spelled, in the personal
    /// dictionary, or ignored this session.
    pub fn accepts(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        if self.ignored.contains(&lower) || self.personal.contains(&lower) {
            return true;
        }
        self.is_correct(word)
    }

    /// Spelling suggestions for a word (capped, best first).
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.dict.suggest(word, &mut out);
        out.truncate(7);
        out
    }

    /// Ignore a word for the rest of the session.
    pub fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.to_lowercase());
    }

    /// Add a word to the personal dictionary (in memory and, if a path is set,
    /// appended to disk). Returns whether it was newly added.
    pub fn add_word(&mut self, word: &str) -> bool {
        let word = word.trim();
        if word.is_empty() {
            return false;
        }
        let fresh = self.personal.insert(word.to_lowercase());
        let _ = self.dict.add(word);
        if fresh && let Some(path) = &self.personal_path {
            use std::io::Write;
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{word}");
            }
        }
        fresh
    }

    /// Every misspelled word in `text`, in order, with byte spans.
    pub fn check_text(&self, text: &str) -> Vec<Misspelling> {
        tokenize::tokens(text)
            .into_iter()
            .filter(|t| !self.accepts(&t.word))
            .map(|t| Misspelling {
                word: t.word,
                start: t.start,
                end: t.end,
            })
            .collect()
    }
}

/// Path of the persistent personal dictionary for a language
/// (`dict/personal_<iso3>.dic` in the configuration folder, E-05).
pub fn personal_dict_path(iso3: &str) -> Option<PathBuf> {
    Some(
        crate::config::paths::get()
            .dict_dir
            .join(format!("personal_{iso3}.dic")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny self-contained dictionary — no external files needed.
    fn checker() -> SpellChecker {
        let aff = "SET UTF-8\n";
        let dic = "8\nhello\nsword\népée\nknown\nwell\nil\ncan\nParis\n";
        SpellChecker::from_sources(aff, dic, "test", "und").unwrap()
    }

    #[test]
    fn basic_check_and_suggest() {
        let c = checker();
        assert!(c.is_correct("hello"));
        assert!(!c.is_correct("swrod"));
        assert_eq!(c.suggest("swrod"), vec!["sword"]);
    }

    #[test]
    fn case_insensitive_variants() {
        let c = checker();
        assert!(c.is_correct("Hello")); // capitalized
        assert!(c.is_correct("SWORD")); // all caps
    }

    #[test]
    fn elisions_and_compounds() {
        let c = checker();
        assert!(c.is_correct("l'épée")); // elision l' + épée
        assert!(c.is_correct("qu'il")); // qu' + il
        assert!(c.is_correct("well-known")); // compound
        assert!(!c.is_correct("swrod's")); // real misspelling with possessive
    }

    #[test]
    fn ignore_and_personal() {
        let mut c = checker();
        assert!(!c.accepts("Skyrim"));
        c.ignore("Skyrim");
        assert!(c.accepts("Skyrim"));

        assert!(!c.accepts("Dwemer"));
        assert!(c.add_word("Dwemer"));
        assert!(c.accepts("Dwemer"));
        assert!(c.accepts("dwemer")); // case-insensitive personal match
    }

    #[test]
    fn check_text_ignores_tags_and_finds_errors() {
        let c = checker();
        let miss = c.check_text("hello <Alias=X> swrod and %d gold");
        let words: Vec<&str> = miss.iter().map(|m| m.word.as_str()).collect();
        assert_eq!(words, ["swrod", "and", "gold"]); // "and"/"gold" absent from tiny dict
        // Spans point back at the real substring.
        let m = &miss[0];
        assert_eq!(
            &"hello <Alias=X> swrod and %d gold"[m.start..m.end],
            "swrod"
        );
    }

    #[test]
    fn dict_info_base() {
        let d = DictInfo {
            code: "fr_FR".into(),
            aff: "x".into(),
            dic: "y".into(),
        };
        assert_eq!(d.base(), "fr");
    }

    #[test]
    fn resolve_prefers_exact_then_base() {
        let dicts = vec![
            DictInfo {
                code: "en_US".into(),
                aff: "".into(),
                dic: "".into(),
            },
            DictInfo {
                code: "fr_FR".into(),
                aff: "".into(),
                dic: "".into(),
            },
        ];
        // "fra" → iso1 "fr" → base match fr_FR.
        assert_eq!(resolve_for_language("fra", &dicts).unwrap().code, "fr_FR");
        // "eng" → en_US.
        assert_eq!(resolve_for_language("eng", &dicts).unwrap().code, "en_US");
        // Unknown language → none.
        assert!(resolve_for_language("zzz", &dicts).is_none());
    }

    /// The full Hunspell set of the repository (several regions per language,
    /// read in any order): the preferred region wins, then the region that
    /// repeats the language, then a bare code.
    #[test]
    fn resolve_among_regional_variants() {
        let info = |c: &str| DictInfo {
            code: c.into(),
            aff: "".into(),
            dic: "".into(),
        };
        let dicts: Vec<DictInfo> = [
            "en_ZA",
            "en_AU",
            "en_GB",
            "en_US",
            "en_CA",
            "de_AT_frami",
            "de_CH_frami",
            "de_DE_frami",
            "es_MX",
            "es_ES",
            "es_AR",
            "pt_PT",
            "pt_BR",
            "ar",
            "sv_SE",
            "sv_FI",
            "ca",
            "ca-valencia",
            "nb_NO",
            "fr_FR",
            "fr",
        ]
        .into_iter()
        .map(info)
        .collect();
        let pick = |l: &str| resolve_for_language(l, &dicts).map(|d| d.code.as_str());
        assert_eq!(pick("eng"), Some("en_US"));
        assert_eq!(pick("deu"), Some("de_DE_frami"));
        assert_eq!(pick("spa"), Some("es_ES"));
        assert_eq!(pick("por"), Some("pt_BR"));
        assert_eq!(pick("ara"), Some("ar"));
        assert_eq!(pick("swe"), Some("sv_SE"));
        // An exact code still wins over everything.
        assert_eq!(pick("fr"), Some("fr"));
    }
}
