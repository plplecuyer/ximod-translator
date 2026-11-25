//! The lists of the « Base pays / langues » window (E-11): the countries and
//! the languages filtered and sorted, the languages spoken in a country, the
//! countries where a language is spoken. Read-only views of the reference
//! tables, without the interface.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::{CountriesData, CountryEntry, CountryLanguagesData, CountryNamesData, LanguagesData};

/// Lower case without accents: what the filters compare.
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The name of a country shown in the interface language `loc`: the
/// translated name (CountryNames.json), else the English one.
pub fn country_name<'a>(names: &'a CountryNamesData, c: &'a CountryEntry, loc: &str) -> &'a str {
    names.name_for(&c.a3, loc).unwrap_or(&c.name_en)
}

/// The country's own name in a language: its endonym when the language is
/// official there (Countries.json), else its translated name; empty when
/// neither is known.
pub fn local_name<'a>(names: &'a CountryNamesData, c: &'a CountryEntry, iso3: &str) -> &'a str {
    c.languages
        .iter()
        .find(|l| l.iso3 == iso3)
        .map(|l| l.country_endonym.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| names.name_for(&c.a3, iso3))
        .unwrap_or("")
}

/// The countries a filter keeps (R-P) — an exact ISO 3166 code, two or
/// three letters, or the start of a word of a name: the common and the
/// official name in the interface language, the common and the official
/// English names, the endonyms; without case nor accents — as (alpha-3,
/// shown name). An empty filter sorts them by shown name; otherwise the
/// exact codes come first, then the names that start with the filter, then
/// the others.
pub fn country_rows(
    countries: &CountriesData,
    names: &CountryNamesData,
    loc: &str,
    filter: &str,
) -> Vec<(String, String)> {
    let needle = fold(filter.trim());
    let mut rows: Vec<(u8, String, String, String)> = countries
        .countries
        .iter()
        .filter_map(|c| {
            let rank = if needle.is_empty() {
                Some(0)
            } else {
                country_rank(names, c, loc, &needle)
            }?;
            let n = country_name(names, c, loc).to_string();
            Some((rank, fold(&n), c.a3.clone(), n))
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, _, a3, n)| (a3, n)).collect()
}

/// A tile of « Choisir un pays »: its code, the name under the flag — the
/// common one in the interface language (« France »), else the official
/// one — and its tooltip, the official name and the ISO codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagRow {
    pub a3: String,
    pub label: String,
    pub tip: String,
}

/// The tiles of « Choisir un pays »: filtered as [`country_rows`], sorted
/// by rank then by the name shown under the flag.
pub fn flag_rows(
    countries: &CountriesData,
    names: &CountryNamesData,
    loc: &str,
    filter: &str,
) -> Vec<FlagRow> {
    let needle = fold(filter.trim());
    let mut rows: Vec<(u8, String, FlagRow)> = countries
        .countries
        .iter()
        .filter_map(|c| {
            let rank = if needle.is_empty() {
                Some(0)
            } else {
                country_rank(names, c, loc, &needle)
            }?;
            let official = country_name(names, c, loc);
            let label = names.common_for(&c.a3, loc).unwrap_or(official);
            let codes = if c.a2.is_empty() {
                c.a3.clone()
            } else {
                format!("{} · {}", c.a3, c.a2)
            };
            let tip = if fold(label) == fold(official) {
                format!("{label}\n{codes}")
            } else {
                format!("{label}\n{official}\n{codes}")
            };
            let row = FlagRow {
                a3: c.a3.clone(),
                label: label.to_string(),
                tip,
            };
            Some((rank, fold(label), row))
        })
        .collect();
    rows.sort_by(|a, b| (a.0, &a.1, &a.2.a3).cmp(&(b.0, &b.1, &b.2.a3)));
    rows.into_iter().map(|(_, _, r)| r).collect()
}

/// How well a country answers a folded filter: 0 for an exact code, 1 for
/// a name that starts with it, 2 for a word of a name; `None` otherwise.
fn country_rank(names: &CountryNamesData, c: &CountryEntry, loc: &str, needle: &str) -> Option<u8> {
    if fold(&c.a3) == needle || (!c.a2.is_empty() && fold(&c.a2) == needle) {
        return Some(0);
    }
    let mut texts: Vec<&str> = vec![country_name(names, c, loc), &c.name_en, &c.common_en];
    texts.extend(names.common_for(&c.a3, loc));
    texts.extend(c.languages.iter().map(|l| l.country_endonym.as_str()));
    texts
        .iter()
        .filter(|t| !t.is_empty())
        .filter_map(|t| word_start_rank(&fold(t), needle))
        .min()
}

/// 1 when `hay` starts with `needle`, 2 when a word of it does (after a
/// space, a hyphen, an apostrophe…, or at any ideograph or kana, scripts
/// without spaces); `None` otherwise.
pub fn word_start_rank(hay: &str, needle: &str) -> Option<u8> {
    if needle.is_empty() || hay.starts_with(needle) {
        return Some(1);
    }
    let mut prev: Option<char> = None;
    for (i, c) in hay.char_indices() {
        let start = prev.is_some_and(|p| !p.is_alphanumeric()) || is_unspaced(c);
        if start && hay[i..].starts_with(needle) {
            return Some(2);
        }
        prev = Some(c);
    }
    None
}

/// A letter of a script written without spaces between words.
fn is_unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// The languages a filter keeps — on the name and the ISO 639-3 / 639-1
/// codes — as indices into the table, sorted by name; an exact code first.
pub fn language_rows(langs: &LanguagesData, filter: &str) -> Vec<usize> {
    let needle = fold(filter.trim());
    let mut rows: Vec<(u8, String, usize)> = langs
        .languages
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if needle.is_empty() {
                return Some((1, fold(&e.name), i));
            }
            let exact = e.iso3 == needle || (!e.iso1.is_empty() && e.iso1 == needle);
            let hit = exact
                || fold(&e.name).contains(&needle)
                || e.iso3.contains(&needle)
                || e.iso1.contains(&needle);
            hit.then(|| (if exact { 0 } else { 1 }, fold(&e.name), i))
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, _, i)| i).collect()
}

/// The languages spoken in a country besides its official ones
/// (CountryLanguages.json), in the file's order.
pub fn spoken_languages<'a>(spoken: &'a CountryLanguagesData, c: &CountryEntry) -> Vec<&'a str> {
    spoken
        .languages_for(&c.a3)
        .iter()
        .map(String::as_str)
        .filter(|iso3| !c.languages.iter().any(|l| l.iso3 == *iso3))
        .collect()
}

/// A country where a language is spoken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpokenIn {
    pub a3: String,
    /// In the interface language.
    pub name: String,
    /// In the language itself (may be empty).
    pub local: String,
    pub official: bool,
}

/// The countries where a language is spoken: those of Languages.json, those
/// where it is official (Countries.json) and those of CountryLanguages.json;
/// the official ones first, then by name.
pub fn spoken_in(
    iso3: &str,
    langs: &LanguagesData,
    countries: &CountriesData,
    spoken: &CountryLanguagesData,
    names: &CountryNamesData,
    loc: &str,
) -> Vec<SpokenIn> {
    let mut a3s: Vec<&str> = langs
        .countries_for(iso3)
        .iter()
        .map(String::as_str)
        .collect();
    for c in &countries.countries {
        if c.languages.iter().any(|l| l.iso3 == iso3)
            || spoken.languages_for(&c.a3).iter().any(|l| l == iso3)
        {
            a3s.push(&c.a3);
        }
    }
    a3s.sort_unstable();
    a3s.dedup();
    let mut out: Vec<SpokenIn> = a3s
        .into_iter()
        .filter_map(|a3| countries.by_a3(a3))
        .map(|c| SpokenIn {
            a3: c.a3.clone(),
            name: country_name(names, c, loc).to_string(),
            local: local_name(names, c, iso3).to_string(),
            official: c.languages.iter().any(|l| l.iso3 == iso3),
        })
        .collect();
    out.sort_by_cached_key(|s| (!s.official, fold(&s.name)));
    out
}
