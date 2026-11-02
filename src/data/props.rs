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

/// The countries a filter keeps — on the shown name, the English name, the
/// endonyms and the code, without case nor accents — as (alpha-3, shown
/// name), sorted by shown name.
pub fn country_rows(
    countries: &CountriesData,
    names: &CountryNamesData,
    loc: &str,
    filter: &str,
) -> Vec<(String, String)> {
    let needle = fold(filter.trim());
    let mut rows: Vec<(String, String, String)> = countries
        .countries
        .iter()
        .filter(|c| {
            needle.is_empty()
                || fold(country_name(names, c, loc)).contains(&needle)
                || fold(&c.name_en).contains(&needle)
                || fold(&c.a3).contains(&needle)
                || c.languages
                    .iter()
                    .any(|l| fold(&l.country_endonym).contains(&needle))
        })
        .map(|c| {
            let n = country_name(names, c, loc).to_string();
            (fold(&n), c.a3.clone(), n)
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, a3, n)| (a3, n)).collect()
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
