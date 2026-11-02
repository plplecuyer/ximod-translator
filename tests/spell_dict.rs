//! Integration test for the spell checker against the bundled `en_US`
//! dictionary in `assets/dictionaries/`. Exercises the on-disk pipeline
//! (discovery → resolution → file loading → check/suggest) that the unit tests
//! (which use inline dictionaries) cannot.

use std::path::Path;

use ximod_translator::spell::{self, SpellChecker};

fn dict_dir() -> &'static Path {
    Path::new("assets/dictionaries")
}

#[test]
fn discovers_and_resolves_bundled_en_us() {
    let dicts = spell::scan_dictionaries(dict_dir());
    assert!(
        dicts.iter().any(|d| d.code == "en_US"),
        "en_US should be discovered among {:?}",
        dicts.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    // "eng" resolves to en_US via the 639-3 → 639-1 → region logic.
    let info = spell::resolve_for_language("eng", &dicts).expect("eng resolves");
    assert_eq!(info.code, "en_US");
}

#[test]
fn checks_real_text_and_ignores_noise() {
    let dicts = spell::scan_dictionaries(dict_dir());
    let info = spell::resolve_for_language("eng", &dicts).expect("eng resolves");
    let checker = SpellChecker::from_files(info, "eng").expect("loads");

    // A realistic mod string: engine tag, placeholder, possessive and a compound
    // must NOT be flagged; only the two genuine misspellings should be.
    let text =
        "The dragon's <Alias=Player> hoard has %d gold, a well-known swrod and recieved fame.";
    let miss: Vec<String> = checker
        .check_text(text)
        .into_iter()
        .map(|m| m.word)
        .collect();
    assert_eq!(miss, ["swrod", "recieved"]);

    // Suggestions point at the right words.
    assert!(checker.suggest("swrod").iter().any(|s| s == "sword"));
    assert!(checker.suggest("recieved").iter().any(|s| s == "received"));
}

#[test]
fn personal_and_ignore_take_effect() {
    let dicts = spell::scan_dictionaries(dict_dir());
    let info = spell::resolve_for_language("eng", &dicts).expect("eng resolves");
    let mut checker = SpellChecker::from_files(info, "eng").expect("loads");

    assert!(!checker.accepts("Dwemer"));
    checker.add_word("Dwemer"); // no path attached → in-memory only
    assert!(checker.accepts("Dwemer"));

    assert!(!checker.accepts("Falmer"));
    checker.ignore("Falmer");
    assert!(checker.accepts("Falmer"));
}
