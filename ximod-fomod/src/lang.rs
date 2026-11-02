//! Language codes used by the translation documents and package names.

const LANGUAGE_MAP: &[(&str, &str, &str)] = &[
    // (ISO 639-1, ISO 639-3, Display Name)
    ("en", "eng", "English"),
    ("fr", "fra", "Français"),
    ("de", "deu", "Deutsch"),
    ("es", "spa", "Español"),
    ("it", "ita", "Italiano"),
    ("pt", "por", "Português"),
    ("ru", "rus", "Русский"),
    ("zh", "zho", "中文"),
    ("ja", "jpn", "日本語"),
    ("ko", "kor", "한국어"),
    ("pl", "pol", "Polski"),
    ("nl", "nld", "Nederlands"),
    ("sv", "swe", "Svenska"),
    ("da", "dan", "Dansk"),
    ("no", "nor", "Norsk"),
    ("fi", "fin", "Suomi"),
    ("cs", "ces", "Čeština"),
    ("hu", "hun", "Magyar"),
    ("tr", "tur", "Türkçe"),
    ("ar", "ara", "العربية"),
    ("he", "heb", "עברית"),
    ("th", "tha", "ไทย"),
    ("vi", "vie", "Tiếng Việt"),
    ("uk", "ukr", "Українська"),
    ("el", "ell", "Ελληνικά"),
    ("ro", "ron", "Română"),
    ("bg", "bul", "Български"),
    ("hr", "hrv", "Hrvatski"),
    ("sk", "slk", "Slovenčina"),
    ("sl", "slv", "Slovenščina"),
    ("et", "est", "Eesti"),
    ("lv", "lav", "Latviešu"),
    ("lt", "lit", "Lietuvių"),
    ("ga", "gle", "Gaeilge"),
    ("mt", "mlt", "Malti"),
    ("rm", "roh", "Rumantsch"),
];

/// ISO 639-3 (or 639-1) → ISO 639-1; `en` for anything unknown.
pub fn iso639_3_to_1(code: &str) -> String {
    let code_lower = code.to_lowercase();
    if code_lower.len() == 2 && LANGUAGE_MAP.iter().any(|(iso1, _, _)| *iso1 == code_lower) {
        return code_lower;
    }
    for (iso1, iso3, _) in LANGUAGE_MAP {
        if *iso3 == code_lower {
            return iso1.to_string();
        }
    }
    "en".to_string()
}

/// ISO 639-1 (or 639-3) → ISO 639-3; the input when unknown.
pub fn iso639_1_to_3(code: &str) -> String {
    let code_lower = code.to_lowercase();
    if code_lower.len() == 3 && LANGUAGE_MAP.iter().any(|(_, iso3, _)| *iso3 == code_lower) {
        return code_lower;
    }
    for (iso1, iso3, _) in LANGUAGE_MAP {
        if *iso1 == code_lower {
            return iso3.to_string();
        }
    }
    code_lower
}

/// The endonym of a language (« Français »), « Unknown » otherwise.
pub fn locale_display_name(locale: &str) -> &'static str {
    let locale_lower = locale.to_lowercase();
    for (iso1, iso3, name) in LANGUAGE_MAP {
        if *iso1 == locale_lower || *iso3 == locale_lower {
            return name;
        }
    }
    "Unknown"
}
