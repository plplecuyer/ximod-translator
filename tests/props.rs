//! Base pays / langues (E-11): the lists of the window on the reference
//! tables shipped in assets/data.

use ximod_translator::data::props::{self, fold};
use ximod_translator::data::{
    CountriesData, CountryLanguagesData, CountryNamesData, LanguagesData,
};

struct Tables {
    countries: CountriesData,
    spoken: CountryLanguagesData,
    names: CountryNamesData,
    langs: LanguagesData,
}

fn tables() -> Tables {
    let t = Tables {
        countries: CountriesData::load(),
        spoken: CountryLanguagesData::load(),
        names: CountryNamesData::load(),
        langs: LanguagesData::load(),
    };
    assert!(!t.countries.is_empty() && !t.langs.is_empty() && !t.names.is_empty());
    t
}

#[test]
fn a_language_and_the_countries_where_it_is_spoken() {
    // T-E11-01 (critère): the language, its codes, its font, its countries.
    let t = tables();
    let rows = props::language_rows(&t.langs, "fr");
    let first = &t.langs.languages[rows[0]];
    assert_eq!((first.iso3.as_str(), first.iso1.as_str()), ("fra", "fr"));
    assert_eq!(first.name, "Français");
    assert!(first.font.contains("NotoSans"));
    assert!(rows.len() > 10, "{}", rows.len());
    let fr = props::spoken_in("fra", &t.langs, &t.countries, &t.spoken, &t.names, "fra");
    assert!(fr.len() >= 60, "{}", fr.len());
    // The official countries first.
    let n_off = fr.iter().take_while(|s| s.official).count();
    assert!(n_off >= 30 && fr[n_off..].iter().all(|s| !s.official));
    for a3 in ["FRA", "BEL", "CAN", "CHE", "SEN"] {
        assert!(fr[..n_off].iter().any(|s| s.a3 == a3), "{a3}");
    }
    // The local name in the language itself.
    let che = fr.iter().find(|s| s.a3 == "CHE").unwrap();
    assert_eq!(che.local, "Confédération suisse");
    // In another interface language, the names change, not the list.
    let de = props::spoken_in("fra", &t.langs, &t.countries, &t.spoken, &t.names, "deu");
    assert_eq!(de.len(), fr.len());
    assert!(de.iter().any(|s| s.a3 == "CHE" && s.name != che.name));
}

#[test]
fn a_country_and_its_languages() {
    // T-E11-02: Suisse, its four official languages, the others.
    let t = tables();
    let rows = props::country_rows(&t.countries, &t.names, "fra", "suisse");
    assert_eq!(rows[0].0, "CHE");
    // The filter ignores accents and case, and finds the endonyms.
    assert!(
        props::country_rows(&t.countries, &t.names, "fra", "CONFEDERATION")
            .iter()
            .any(|r| r.0 == "CHE")
    );
    assert!(
        props::country_rows(&t.countries, &t.names, "fra", "svizra")
            .iter()
            .any(|r| r.0 == "CHE")
    );
    assert!(
        props::country_rows(&t.countries, &t.names, "fra", "che")
            .iter()
            .any(|r| r.0 == "CHE")
    );
    let che = t.countries.by_a3("CHE").unwrap();
    let off: Vec<&str> = che.languages.iter().map(|l| l.iso3.as_str()).collect();
    assert_eq!(off, ["deu", "fra", "ita", "roh"]);
    let others = props::spoken_languages(&t.spoken, che);
    assert!(!others.is_empty() && others.iter().all(|l| !off.contains(l)));
    // Romansh: Switzerland, official, its own name.
    let roh = props::spoken_in("roh", &t.langs, &t.countries, &t.spoken, &t.names, "fra");
    assert_eq!(roh[0].a3, "CHE");
    assert!(roh[0].official);
    assert_eq!(roh[0].local, "Confederaziun svizra");
    // Sorted by the name shown, every country once.
    let all = props::country_rows(&t.countries, &t.names, "fra", "");
    assert_eq!(all.len(), t.countries.countries.len());
    let keys: Vec<String> = all.iter().map(|r| fold(&r.1)).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn without_the_names_the_english_ones() {
    // T-E11-04: no CountryNames.json → English names, nothing breaks.
    let t = tables();
    let none = CountryNamesData::default();
    let rows = props::country_rows(&t.countries, &none, "fra", "swiss");
    assert_eq!(
        rows[0],
        ("CHE".to_string(), "Swiss Confederation".to_string())
    );
    let fr = props::spoken_in("fra", &t.langs, &t.countries, &t.spoken, &none, "fra");
    assert!(
        fr.iter()
            .any(|s| s.a3 == "CHE" && s.local == "Confédération suisse")
    );
    assert!(props::language_rows(&LanguagesData::default(), "fr").is_empty());
    assert_eq!(fold("Élysée ÇA"), "elysee ca");
}

/// T-P (B.6): the common name, the official one, the English one and the
/// ISO codes find a country; a filter is found at the start of a word.
#[test]
fn acceptance_t_p_a_country_by_its_common_name() {
    let t = tables();
    let find = |f: &str| props::country_rows(&t.countries, &t.names, "fra", f);
    for f in ["France", "france", "FR", "FRA", "République", "fran"] {
        let rows = find(f);
        assert!(rows.iter().any(|r| r.0 == "FRA"), "{f}");
    }
    // An exact code first, then a name starting with the filter.
    for f in ["FR", "fra", "France"] {
        assert_eq!(find(f)[0].0, "FRA", "{f}");
    }
    for f in ["Allemagne", "Germany", "DE", "deu", "allem"] {
        let rows = find(f);
        assert_eq!(rows[0].0, "DEU", "{f}");
    }
    // A word inside a name: « Ivoire » finds Côte d’Ivoire.
    assert!(find("ivoire").iter().any(|r| r.0 == "CIV"));
    // Not inside a word any more: « ance » finds no France.
    assert!(!find("ance").iter().any(|r| r.0 == "FRA"));
    // In another interface language, its common name.
    let de = props::country_rows(&t.countries, &t.names, "deu", "Frankreich");
    assert_eq!(de[0].0, "FRA");
    // Every country has its two-letter code and its common English name.
    assert!(
        t.countries
            .countries
            .iter()
            .all(|c| c.a2.len() == 2 && !c.common_en.is_empty())
    );
    // Japanese: a word without spaces is found inside the name.
    let ja = props::country_rows(&t.countries, &t.names, "jpn", "フランス");
    assert_eq!(ja[0].0, "FRA");
}

#[test]
fn a_word_start() {
    assert_eq!(props::word_start_rank("cote d’ivoire", "cote"), Some(1));
    assert_eq!(props::word_start_rank("cote d’ivoire", "ivoire"), Some(2));
    assert_eq!(props::word_start_rank("saint-pierre", "pierre"), Some(2));
    assert_eq!(props::word_start_rank("france", "ance"), None);
    assert_eq!(props::word_start_rank("法兰西共和国", "共和"), Some(2));
}

/// Chinese in the broad sense (zho) and Mandarin (cmn) have names of their
/// own: the Cible list of China shows « 中文 » and « 普通话 ».
#[test]
fn chinese_and_mandarin_are_told_apart() {
    let t = tables();
    assert_eq!(t.langs.display_name("zho"), Some("中文"));
    assert_eq!(t.langs.display_name("cmn"), Some("普通话"));
    let chn = t.countries.by_a3("CHN").unwrap();
    let names: Vec<&str> = chn
        .languages
        .iter()
        .filter_map(|l| t.langs.display_name(&l.iso3))
        .collect();
    assert_eq!(names, ["中文", "普通话"]);
}

/// Signalé le 18 novembre 2026 : l'Algérie avait ses deux noms officiels
/// inversés (l'arabe en tifinagh, le tamazight en arabe) ; le Sahara
/// occidental, le Soudan et le Pakistan aussi. Le nom en arabe ou en
/// ourdou s'écrit en alphabet arabe, le nom en tamazight en tifinagh.
#[test]
fn each_official_name_is_in_its_language() {
    let t = tables();
    let arabic = |s: &str| s.chars().any(|c| ('\u{600}'..='\u{6ff}').contains(&c));
    let tifinagh = |s: &str| s.chars().any(|c| ('\u{2d30}'..='\u{2d7f}').contains(&c));
    for c in &t.countries.countries {
        for l in &c.languages {
            let n = &l.country_endonym;
            if n.is_empty() {
                continue;
            }
            match l.iso3.as_str() {
                "ara" | "urd" | "fas" => assert!(arabic(n), "{} {} {n}", c.a3, l.iso3),
                "tzm" => assert!(tifinagh(n), "{} {} {n}", c.a3, l.iso3),
                "eng" | "fra" | "spa" | "por" => {
                    assert!(!arabic(n) && !tifinagh(n), "{} {} {n}", c.a3, l.iso3)
                }
                _ => {}
            }
        }
    }
    let dza = t.countries.by_a3("DZA").unwrap();
    let ara = dza.languages.iter().find(|l| l.iso3 == "ara").unwrap();
    assert_eq!(
        ara.country_endonym,
        "الجمهورية الجزائرية الديمقراطية الشعبية"
    );
}

/// Demandé le 18 novembre 2026 : deux langues ne portent jamais le même
/// nom — une macrolangue garde le sien, la langue précise prend le sien
/// (« Melayu » / « Bahasa Malaysia », « isiNdebele » / « isiNdebele
/// seSewula », « العربية » / « العربية الفصحى »…).
#[test]
fn every_language_has_a_name_of_its_own() {
    let t = tables();
    let mut seen: std::collections::HashMap<&str, &str> = Default::default();
    for l in &t.langs.languages {
        if let Some(other) = seen.insert(l.name.as_str(), l.iso3.as_str()) {
            panic!("« {} » : {} et {}", l.name, other, l.iso3);
        }
    }
    assert_eq!(t.langs.display_name("msa"), Some("Melayu"));
    assert_eq!(t.langs.display_name("zsm"), Some("Bahasa Malaysia"));
    assert_eq!(t.langs.display_name("nde"), Some("isiNdebele"));
    assert_eq!(t.langs.display_name("nbl"), Some("isiNdebele seSewula"));
    assert_eq!(t.langs.display_name("ara"), Some("العربية"));
    assert_eq!(t.langs.display_name("arb"), Some("العربية الفصحى"));
}

/// Choisir un pays (recette du lot 2) : sous le drapeau le nom courant dans
/// la langue de l'interface, dans l'infobulle le nom officiel et les codes ;
/// le portugais de l'interface est celui du Portugal.
#[test]
fn the_flags_show_the_common_names() {
    let t = tables();
    let rows = props::flag_rows(&t.countries, &t.names, "fra", "");
    let fr = rows.iter().find(|r| r.a3 == "FRA").unwrap();
    assert_eq!(fr.label, "France");
    assert!(
        fr.tip.contains("République française") && fr.tip.contains("FRA · FR"),
        "{}",
        fr.tip
    );
    // Sorted by the name shown: Allemagne before France, Corée du Sud
    // among the C.
    let pos = |a3: &str| rows.iter().position(|r| r.a3 == a3).unwrap();
    assert!(pos("DEU") < pos("FRA"));
    assert_eq!(
        rows.iter().find(|r| r.a3 == "KOR").unwrap().label,
        "Corée du Sud"
    );
    assert!(pos("CMR") < pos("KOR") && pos("KOR") < pos("CRI"));
    // Every tile has a name, in each interface language.
    for loc in ["eng", "deu", "jpn", "zho", "ara", "nob", "nor", "por"] {
        let rows = props::flag_rows(&t.countries, &t.names, loc, "");
        assert_eq!(rows.len(), t.countries.countries.len(), "{loc}");
        assert!(rows.iter().all(|r| !r.label.is_empty()), "{loc}");
    }
    let pt = props::flag_rows(&t.countries, &t.names, "por", "");
    assert_eq!(
        pt.iter().find(|r| r.a3 == "CYM").unwrap().label,
        "Ilhas Caimão"
    );
    // The filter of country_rows: « fran », « FR » find France first.
    for f in ["fran", "FR", "France"] {
        assert_eq!(
            props::flag_rows(&t.countries, &t.names, "fra", f)[0].a3,
            "FRA",
            "{f}"
        );
    }
}
