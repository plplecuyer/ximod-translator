//! Source-language detection for files that do not name their language
//! (non-localized plugins, binary bases): a statistical estimate over the
//! source texts, by the Lingua library (75 languages, n-grams of 1 to 5
//! characters, built for short texts).
//!
//! The detector is built once, lazily, on the first call; its models are
//! loaded on demand, so the first detection of a session costs a little more.

use std::sync::OnceLock;

use lingua::{LanguageDetector, LanguageDetectorBuilder};

/// Candidates below this share of confidence are dropped.
const MIN_CONFIDENCE: f64 = 0.10;
/// At most this many candidates are proposed.
const MAX_CANDIDATES: usize = 3;
/// Characters of text fed to the detector, at most (the longest strings
/// first: dialogue and descriptions carry far more signal than item names).
const SAMPLE_CHARS: usize = 20_000;
/// Below this many characters the estimate is not trusted at all.
const MIN_SAMPLE_CHARS: usize = 20;

fn detector() -> &'static LanguageDetector {
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| LanguageDetectorBuilder::from_all_languages().build())
}

/// One detected language: ISO 639-3 code and confidence in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub iso3: String,
    pub confidence: f64,
}

/// Build the text sample from a set of source strings: the longest first,
/// up to [`SAMPLE_CHARS`] characters, one string per line.
pub fn sample<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    sample_n(texts, SAMPLE_CHARS)
}

/// [`sample`] with another size (the page detection, D-09, scores many
/// decodings of the same bytes and needs a smaller one).
pub fn sample_n<'a>(texts: impl IntoIterator<Item = &'a str>, max_chars: usize) -> String {
    let mut v: Vec<&str> = texts
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    v.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    v.dedup();
    let mut out = String::new();
    let mut count = 0usize;
    for t in v {
        if count >= max_chars {
            break;
        }
        count += t.chars().count() + 1;
        out.push_str(t);
        out.push('\n');
    }
    out
}

/// The most likely languages of a sample, best first; empty when the sample
/// is too short or nothing stands out.
pub fn detect(sample_text: &str) -> Vec<Candidate> {
    if sample_text.chars().count() < MIN_SAMPLE_CHARS {
        return Vec::new();
    }
    let values = detector().compute_language_confidence_values(sample_text);
    values
        .into_iter()
        .filter(|(_, c)| *c >= MIN_CONFIDENCE)
        .take(MAX_CANDIDATES)
        .map(|(lang, c)| Candidate {
            iso3: lang.iso_code_639_3().to_string(),
            confidence: c,
        })
        .collect()
}

/// [`sample`] then [`detect`] in one call.
pub fn detect_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<Candidate> {
    detect(&sample(texts))
}

/// The number of languages the detector knows.
pub fn language_count() -> usize {
    lingua::Language::all().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_keeps_the_longest_strings_first() {
        let s = sample([
            "Iron Sword",
            "",
            "  ",
            "A long description of the blade, forged in Whiterun.",
            "Steel",
        ]);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(
            lines[0],
            "A long description of the blade, forged in Whiterun."
        );
        assert_eq!(lines.last(), Some(&"Steel"));
        assert!(detect("short").is_empty(), "too short to trust");
    }

    #[test]
    fn detects_english_and_french_game_text() {
        let en = detect_texts([
            "You have been found guilty of a crime. Pay the fine or go to jail.",
            "The Jarl of Whiterun wants to speak with you about the dragon.",
            "Iron Sword",
            "Leather Armor of Minor Health",
        ]);
        assert_eq!(en.first().map(|c| c.iso3.as_str()), Some("eng"), "{en:?}");
        let fr = detect_texts([
            "Vous avez été reconnu coupable d'un crime. Payez l'amende ou allez en prison.",
            "Le jarl de Blancherive veut vous parler du dragon.",
            "Épée en fer",
            "Armure de cuir de santé mineure",
        ]);
        assert_eq!(fr.first().map(|c| c.iso3.as_str()), Some("fra"), "{fr:?}");
        assert!(fr[0].confidence > 0.5);
        assert_eq!(language_count(), 75);
    }
}
