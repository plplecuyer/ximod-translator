//! Bethesda STRINGS language names ↔ ISO codes.
//!
//! Localized plugins name their sidecar files `<Plugin>_<Language>.STRINGS`,
//! where `<Language>` is Bethesda's capitalised English name (`English`,
//! `French`, …). Translators, however, pick target languages by ISO code, and
//! this app stores them as ISO 639-3 (`eng`, `fra`). This module maps between the
//! two so exported STRINGS get the right suffix and existing STRINGS are located
//! regardless of the code form used.

use super::strings::{StringsKind, kind_from_extension};

/// `(ISO 639-1, ISO 639-3, Bethesda STRINGS suffix)`. The first block are the
/// languages the games officially ship; the rest follow the same
/// capitalised-name convention and are common in community translations.
pub const STRINGS_LANGUAGES: &[(&str, &str, &str)] = &[
    // Officially shipped by Skyrim / Fallout 4 / 76 / Starfield.
    ("en", "eng", "English"),
    ("fr", "fra", "French"),
    ("de", "deu", "German"),
    ("it", "ita", "Italian"),
    ("es", "spa", "Spanish"),
    ("pl", "pol", "Polish"),
    ("ru", "rus", "Russian"),
    ("ja", "jpn", "Japanese"),
    ("zh", "zho", "Chinese"),
    ("cs", "ces", "Czech"),
    ("pt", "por", "Portuguese"),
    ("ko", "kor", "Korean"),
    // Common community translation languages, same naming convention.
    ("hu", "hun", "Hungarian"),
    ("nl", "nld", "Dutch"),
    ("sv", "swe", "Swedish"),
    ("tr", "tur", "Turkish"),
    ("ar", "ara", "Arabic"),
    ("uk", "ukr", "Ukrainian"),
    ("fi", "fin", "Finnish"),
    ("da", "dan", "Danish"),
    ("no", "nor", "Norwegian"),
    ("el", "ell", "Greek"),
    ("th", "tha", "Thai"),
    ("vi", "vie", "Vietnamese"),
];

/// The Bethesda STRINGS suffix for an ISO code (639-1 or 639-3, any case).
/// Returns `None` for an unknown code (so callers can fall back deliberately).
pub fn bethesda_name(code: &str) -> Option<&'static str> {
    let c = code.trim().to_lowercase();
    STRINGS_LANGUAGES
        .iter()
        .find(|(i1, i3, _)| *i1 == c || *i3 == c)
        .map(|(_, _, name)| *name)
}

/// The ISO 639-1 code for a Bethesda STRINGS suffix (case-insensitive).
pub fn iso_from_bethesda_name(name: &str) -> Option<&'static str> {
    let n = name.trim().to_lowercase();
    STRINGS_LANGUAGES
        .iter()
        .find(|(_, _, bname)| bname.to_lowercase() == n)
        .map(|(i1, _, _)| *i1)
}

/// Whether a filename's language token (e.g. `French`, `fr`, `fra`) refers to the
/// same language as `preferred` (an ISO code or a Bethesda name). Case-insensitive.
pub fn token_matches(token: &str, preferred: &str) -> bool {
    let t = token.trim().to_lowercase();
    let p = preferred.trim().to_lowercase();
    if t == p {
        return true;
    }
    // Resolve both sides to an ISO 639-1 and compare.
    let iso_of = |s: &str| -> Option<&'static str> {
        STRINGS_LANGUAGES
            .iter()
            .find(|(i1, i3, name)| *i1 == s || *i3 == s || name.to_lowercase() == s)
            .map(|(i1, _, _)| *i1)
    };
    match (iso_of(&t), iso_of(&p)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// If `name` is `<stem>_<lang>.<KIND>` (case-insensitive on stem and extension),
/// return the language token (original case) and the STRINGS kind. Used to locate
/// a plugin's sidecar STRINGS without guessing exact casing — important on the
/// case-sensitive filesystems XIMOD also targets (Linux/macOS).
pub fn match_strings_filename(name: &str, stem: &str) -> Option<(String, StringsKind)> {
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    let kind = kind_from_extension(ext)?;
    let base = &name[..dot];
    // The language is the last underscore-separated token; the rest is the base
    // name, compared to the plugin stem **ignoring case and separators** so that
    // e.g. Creation Club STRINGS `ccqdrsse001-survivalmode_english.strings`
    // resolve for a plugin named `ccQDRSSE001SurvivalMode.esl` (dash dropped).
    let us = base.rfind('_')?;
    let (base_name, lang) = (&base[..us], &base[us + 1..]);
    if lang.is_empty() || normalize_name(base_name) != normalize_name(stem) {
        return None;
    }
    Some((lang.to_string(), kind))
}

/// Normalize a plugin/base name for tolerant comparison: keep only ASCII
/// alphanumerics, lowercased (drops dashes, spaces, underscores, case).
pub fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_to_name_accepts_both_code_forms() {
        assert_eq!(bethesda_name("fr"), Some("French"));
        assert_eq!(bethesda_name("fra"), Some("French")); // 639-3 (config form)
        assert_eq!(bethesda_name("ENG"), Some("English")); // case-insensitive
        assert_eq!(bethesda_name("zh"), Some("Chinese"));
        assert_eq!(bethesda_name("xx"), None);
    }

    #[test]
    fn name_to_iso() {
        assert_eq!(iso_from_bethesda_name("French"), Some("fr"));
        assert_eq!(iso_from_bethesda_name("russian"), Some("ru"));
        assert_eq!(iso_from_bethesda_name("Klingon"), None);
    }

    #[test]
    fn token_matching_across_forms() {
        assert!(token_matches("French", "fra"));
        assert!(token_matches("fr", "French"));
        assert!(token_matches("FRENCH", "fr"));
        assert!(!token_matches("German", "fr"));
    }

    #[test]
    fn filename_matching_is_case_tolerant() {
        assert_eq!(
            match_strings_filename("MyMod_French.STRINGS", "MyMod"),
            Some(("French".to_string(), StringsKind::Strings))
        );
        // Lowercase (BSA-extracted) name, lowercase extension.
        assert_eq!(
            match_strings_filename("mymod_english.dlstrings", "MyMod"),
            Some(("english".to_string(), StringsKind::DlStrings))
        );
        assert_eq!(
            match_strings_filename("Other_French.STRINGS", "MyMod"),
            None
        );
        assert_eq!(match_strings_filename("MyMod.esp", "MyMod"), None);
    }

    #[test]
    fn filename_matching_ignores_separators() {
        // Creation Club: STRINGS carry a dash the plugin file name lacks.
        assert_eq!(
            match_strings_filename(
                "ccqdrsse001-survivalmode_english.strings",
                "ccQDRSSE001SurvivalMode"
            ),
            Some(("english".to_string(), StringsKind::Strings))
        );
        // A genuinely different plugin still doesn't match.
        assert_eq!(
            match_strings_filename("othermod_english.strings", "ccQDRSSE001SurvivalMode"),
            None
        );
    }
}
