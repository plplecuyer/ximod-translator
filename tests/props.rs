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
