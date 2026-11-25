//! How each language writes its numbers (B.6, D-124): its decimal
//! separator and its thousands separators, from the CLDR through
//! `Languages.json`. The number check (A-07, D-05, the marks of A-16) reads
//! « 250,000 » in English and « 250 000 » in French as one number each and
//! finds them equal by their value.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The writing of the numbers of a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumStyle {
    /// The decimal separator (`,` in French, `.` in English; `\0` when
    /// unknown).
    pub decimal: char,
    /// The thousands separators (`\0` for a free place), the usual first.
    groups: [char; 4],
}

impl Default for NumStyle {
    fn default() -> Self {
        Self::FALLBACK
    }
}

impl NumStyle {
    /// A language whose writing is unknown: no thousands separator, `.`
    /// or `,` before decimals (the reading before B.6), and no value for
    /// « 250,000 », which may be either.
    pub const FALLBACK: NumStyle = NumStyle {
        decimal: '\0',
        groups: ['\0'; 4],
    };

    /// A writing: its decimal separator and its thousands separators.
    pub fn new(decimal: char, groups: &str) -> NumStyle {
        let mut g = ['\0'; 4];
        for (slot, c) in g.iter_mut().zip(groups.chars()) {
            *slot = c;
        }
        NumStyle { decimal, groups: g }
    }

    /// `c` separates thousands in this language (an ordinary space only
    /// where the language admits it, R-N).
    pub fn is_group(&self, c: char) -> bool {
        c != '\0' && self.groups.contains(&c)
    }

    /// `c` may separate the decimals: the language's own, else `.` or `,`.
    fn is_decimal(&self, c: char) -> bool {
        (c != '\0' && c == self.decimal) || c == '.' || c == ','
    }

    /// The end (byte offset) of the number that starts with the digit at
    /// `start` in `text`: its digits, its thousands — a separator of the
    /// language followed by exactly three digits, after a first group of
    /// at most three — and its decimals.
    pub fn number_end(&self, text: &str, start: usize) -> usize {
        let b = text.as_bytes();
        let mut i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i - start <= 3 {
            while let Some(c) = text[i..].chars().next() {
                let after = i + c.len_utf8();
                if self.is_group(c) && three_digits_at(b, after) {
                    i = after + 3;
                } else {
                    break;
                }
            }
        }
        if let Some(c) = text[i..].chars().next() {
            let after = i + c.len_utf8();
            if self.is_decimal(c) && b.get(after).is_some_and(u8::is_ascii_digit) {
                i = after;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
        i
    }

    /// The value of a number read in this language, written plainly
    /// (« 250000 », « 2.5 »); `None` when it cannot be read as one value —
    /// also when a separator that is not the language's own stands before
    /// three last digits: « 250,000 » in French may be thousands written
    /// in English, never « 250 ».
    pub fn value(&self, number: &str) -> Option<String> {
        let b = number.as_bytes();
        let mut int = String::new();
        let mut frac: Option<String> = None;
        for (i, c) in number.char_indices() {
            let thousands = frac.is_none()
                && !int.is_empty()
                && self.is_group(c)
                && three_digits_at(b, i + c.len_utf8());
            if c.is_ascii_digit() {
                match &mut frac {
                    Some(f) => f.push(c),
                    None => int.push(c),
                }
            } else if thousands {
                continue;
            } else if frac.is_none() && !int.is_empty() && self.is_decimal(c) {
                let after = i + c.len_utf8();
                if c != self.decimal && three_digits_at(b, after) && after + 3 == b.len() {
                    return None;
                }
                frac = Some(String::new());
            } else {
                return None;
            }
        }
        if int.is_empty() {
            return None;
        }
        let int = int.trim_start_matches('0');
        let int = if int.is_empty() { "0" } else { int };
        let frac = frac.unwrap_or_default();
        let frac = frac.trim_end_matches('0');
        Some(if frac.is_empty() {
            int.to_string()
        } else {
            format!("{int}.{frac}")
        })
    }
}

/// Exactly three digits at `at`, no fourth after them.
fn three_digits_at(b: &[u8], at: usize) -> bool {
    b.len() >= at + 3
        && b[at..at + 3].iter().all(u8::is_ascii_digit)
        && !b.get(at + 3).is_some_and(u8::is_ascii_digit)
}

/// The writing of a language (ISO 639-3) from `Languages.json`; the
/// fallback when the table does not give it.
pub fn style(iso3: &str) -> NumStyle {
    static TABLE: OnceLock<HashMap<String, NumStyle>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let data = crate::data::languages();
        data.languages
            .iter()
            .filter_map(|l| {
                let (d, g) = data.number_separators(&l.iso3)?;
                Some((l.iso3.clone(), NumStyle::new(d.chars().next()?, g)))
            })
            .collect()
    });
    table.get(iso3).copied().unwrap_or(NumStyle::FALLBACK)
}

/// The writings of the source and of the translation of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NumPair {
    pub source: NumStyle,
    pub target: NumStyle,
}

impl NumPair {
    /// The pair of two languages (ISO 639-3).
    pub fn of(source: &str, target: &str) -> NumPair {
        NumPair {
            source: style(source),
            target: style(target),
        }
    }

    /// A number of the source and one of the translation correspond: the
    /// same writing, or the same value each read in its language (R-N).
    pub fn corresponds(&self, source: &str, trans: &str) -> bool {
        source == trans
            || self
                .source
                .value(source)
                .is_some_and(|v| self.target.value(trans).as_ref() == Some(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng() -> NumStyle {
        NumStyle::new('.', ",")
    }
    fn fra() -> NumStyle {
        NumStyle::new(',', "\u{202f}\u{a0} ")
    }
    fn deu() -> NumStyle {
        NumStyle::new(',', ".")
    }

    fn read(text: &str, s: NumStyle) -> Vec<&str> {
        let mut out = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_digit() {
                let end = s.number_end(text, i);
                out.push(&text[i..end]);
                i = end;
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn a_number_is_read_with_the_separators_of_its_language() {
        assert_eq!(read("250,000 gold", eng()), ["250,000"]);
        assert_eq!(read("250 000 or", fra()), ["250 000"]);
        assert_eq!(read("250\u{202f}000 or", fra()), ["250\u{202f}000"]);
        assert_eq!(read("250.000 Gold", deu()), ["250.000"]);
        assert_eq!(read("1,234,567.5", eng()), ["1,234,567.5"]);
        // A space only where the language admits it.
        assert_eq!(read("250 000 gold", eng()), ["250", "000"]);
        // « 10 20 30 » stays three numbers (T-N).
        assert_eq!(read("10 20 30", fra()), ["10", "20", "30"]);
        // Four digits after the separator: no thousands.
        assert_eq!(read("12 3456", fra()), ["12", "3456"]);
        // A first group of more than three digits takes no thousands.
        assert_eq!(read("12345 678", fra()), ["12345", "678"]);
        // Decimals as before, whatever the language.
        assert_eq!(read("2,5 kg", eng()), ["2,5"]);
        assert_eq!(read("2.5 kg", fra()), ["2.5"]);
        assert_eq!(read("1.000", NumStyle::FALLBACK), ["1.000"]);
    }

    #[test]
    fn the_value_of_a_number() {
        assert_eq!(eng().value("250,000").as_deref(), Some("250000"));
        assert_eq!(fra().value("250 000").as_deref(), Some("250000"));
        assert_eq!(deu().value("250.000").as_deref(), Some("250000"));
        assert_eq!(eng().value("2.5").as_deref(), Some("2.5"));
        assert_eq!(fra().value("2,5").as_deref(), Some("2.5"));
        assert_eq!(fra().value("2,50").as_deref(), Some("2.5"));
        assert_eq!(eng().value("007").as_deref(), Some("7"));
        assert_eq!(eng().value("1.000").as_deref(), Some("1"));
        assert_eq!(deu().value("1.000").as_deref(), Some("1000"));
        assert_eq!(eng().value("2,5").as_deref(), Some("2.5"));
        assert_eq!(eng().value("1.2.3"), None);
        // Thousands or decimals? Not said: no value.
        assert_eq!(NumStyle::FALLBACK.value("250,000"), None);
        assert_eq!(NumStyle::FALLBACK.value("1.000"), None);
        assert_eq!(fra().value("2.500"), None);
        assert_eq!(NumStyle::FALLBACK.value("2,5").as_deref(), Some("2.5"));
        assert_eq!(NumStyle::FALLBACK.value("250").as_deref(), Some("250"));
    }

    /// T-N: the pairs of the specification.
    #[test]
    fn acceptance_t_n_pairs() {
        let en_fr = NumPair {
            source: eng(),
            target: fra(),
        };
        assert!(en_fr.corresponds("250,000", "250 000"));
        assert!(en_fr.corresponds("250000", "250 000"));
        assert!(en_fr.corresponds("2.5", "2,5"));
        assert!(!en_fr.corresponds("250 000", "25 000"));
        assert!(!en_fr.corresponds("250,000", "25 000"));
        let en_de = NumPair {
            source: eng(),
            target: deu(),
        };
        assert!(en_de.corresponds("250,000", "250.000"));
        // Written the same: they correspond, whatever the reading.
        assert!(en_de.corresponds("1.000", "1.000"));
        // « 2,5 » is never « 25 » (D-124).
        assert!(!en_de.corresponds("25", "2,5"));
    }

    /// The table of `Languages.json` gives the CLDR writings (R-N).
    #[test]
    fn the_languages_table_gives_the_writings() {
        assert_eq!(style("eng"), eng());
        assert_eq!(style("fra"), fra());
        assert_eq!(style("deu"), deu());
        let gsw = style("gsw");
        assert_eq!(gsw.decimal, '.');
        assert!(gsw.is_group('\'') && gsw.is_group('\u{2019}'));
        assert_eq!(style("not-a-language"), NumStyle::FALLBACK);
    }
}
