//! Internationalization.
//!
//! Uses Fluent, loading `assets/locales/<iso639-3>/main.ftl` at runtime (never
//! embedded), exactly like XIMOD Architect. The canonical language table comes
//! from `assets/data/Languages.json`; a small built-in table is kept only as a
//! fallback and to migrate legacy 2-letter locale codes.

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use std::path::PathBuf;
use unic_langid::LanguageIdentifier;

use crate::data::LanguagesData;

/// Fallback ISO 639-1 ↔ ISO 639-3 ↔ display-name table.
/// Interface languages written right to left, held back until the
/// interface can be drawn right to left (E-08, étape 3): their `main.ftl`
/// stays shipped (and editable in the translation editor, E-10), but they
/// are not offered as interface or displayed language, and a configuration
/// that still names one opens in English.
pub const RTL_PENDING: &[&str] = &["ara"];

const LANGUAGE_MAP: &[(&str, &str, &str)] = &[
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
    ("cs", "ces", "Čeština"),
    ("tr", "tur", "Türkçe"),
    ("ar", "ara", "العربية"),
    ("uk", "ukr", "Українська"),
];

/// ISO 639-3 → ISO 639-1 (fallback table), defaulting to "en".
pub fn iso639_3_to_1(code: &str) -> String {
    let lower = code.to_lowercase();
    if lower.len() == 2 && LANGUAGE_MAP.iter().any(|(iso1, _, _)| *iso1 == lower) {
        return lower;
    }
    for (iso1, iso3, _) in LANGUAGE_MAP {
        if *iso3 == lower {
            return iso1.to_string();
        }
    }
    "en".to_string()
}

/// If `code` is a recognized ISO 639-1 or 639-3 language code, return its
/// ISO 639-3 form; otherwise `None`. Used to spot a language token inside a
/// dictionary file name (e.g. `EN` → `eng`, `fra` → `fra`, `Fallout3` → None).
pub fn iso639_to_3(code: &str) -> Option<String> {
    let lower = code.trim().to_lowercase();
    for (iso1, iso3, _) in LANGUAGE_MAP {
        if *iso1 == lower || *iso3 == lower {
            return Some((*iso3).to_string());
        }
    }
    None
}

/// Display name for a locale code (fallback table).
pub fn locale_display_name(locale: &str) -> &'static str {
    let lower = locale.to_lowercase();
    for (iso1, iso3, name) in LANGUAGE_MAP {
        if *iso1 == lower || *iso3 == lower {
            return name;
        }
    }
    ""
}

/// Normalize a locale code to ISO 639-3 (2-letter codes are treated as ISO
/// 639-1 and mapped; anything else is assumed already ISO 639-3).
pub fn normalize_locale(code: &str) -> String {
    let lower = code.trim().to_lowercase();
    if lower.len() == 2 {
        for (iso1, iso3, _) in LANGUAGE_MAP {
            if *iso1 == lower {
                return iso3.to_string();
            }
        }
    }
    lower
}

/// Internationalization manager.
pub struct I18n {
    bundle: Option<FluentBundle<FluentResource>>,
    fallback_bundle: Option<FluentBundle<FluentResource>>,
    current_locale: String,
    locales_dir: PathBuf,
    /// `<config>/locales`: the languages written by the interface
    /// translation editor when `assets/` is read-only (E-10, D-35), read
    /// before the shipped ones.
    user_locales_dir: Option<PathBuf>,
    available_locales: Vec<String>,
    languages: LanguagesData,
}

impl I18n {
    pub fn new() -> Self {
        let locales_dir = Self::get_locales_dir();
        let user_locales_dir = Some(crate::config::paths::get().config_dir.join("locales"));
        let mut available = Self::scan_available_locales(&locales_dir);
        if let Some(u) = &user_locales_dir {
            available.extend(Self::scan_available_locales(u));
            available.sort();
            available.dedup();
        }
        let languages = LanguagesData::load();
        tracing::info!("Locales dir: {locales_dir:?}; available: {available:?}");
        Self {
            bundle: None,
            fallback_bundle: None,
            current_locale: "eng".to_string(),
            locales_dir,
            user_locales_dir,
            available_locales: available,
            languages,
        }
    }

    fn get_locales_dir() -> PathBuf {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join("assets").join("locales"));
            candidates.push(dir.join("locales"));
            if let Some(up) = dir.parent() {
                let res = up.join("Resources");
                candidates.push(res.join("assets").join("locales"));
                candidates.push(res.join("locales"));
            }
        }
        candidates.push(PathBuf::from("assets").join("locales"));
        candidates.push(PathBuf::from("locales"));
        candidates
            .iter()
            .find(|p| p.is_dir())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("assets/locales"))
    }

    fn scan_available_locales(locales_dir: &PathBuf) -> Vec<String> {
        let mut locales = Vec::new();
        if let Ok(entries) = std::fs::read_dir(locales_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path.join("main.ftl").exists()
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    let code = normalize_locale(name);
                    if !RTL_PENDING.contains(&code.as_str()) {
                        locales.push(code);
                    }
                }
            }
        }
        locales.sort();
        locales
    }

    /// Set the current locale (ISO 639-1 or ISO 639-3).
    pub fn set_locale(&mut self, locale: &str) {
        let mut iso3 = normalize_locale(locale);
        if RTL_PENDING.contains(&iso3.as_str()) {
            iso3 = "eng".to_string();
        }
        self.bundle = None;
        if let Some(bundle) = self.load_locale_bundle(&iso3) {
            self.bundle = Some(bundle);
            self.current_locale = iso3.clone();
        }
        if iso3 != "eng" {
            if self.fallback_bundle.is_none() {
                self.fallback_bundle = self.load_locale_bundle("eng");
            }
            if self.bundle.is_some() {
                let missing = self.missing_keys(&iso3).len();
                if missing > 0 {
                    tracing::debug!("locale {iso3}: {missing} key(s) shown in English");
                }
            }
        } else {
            self.fallback_bundle = None;
        }
        if self.bundle.is_none()
            && iso3 != "eng"
            && let Some(bundle) = self.load_locale_bundle("eng")
        {
            self.bundle = Some(bundle);
            self.current_locale = "eng".to_string();
        }
    }

    /// The `main.ftl` of a locale: the user's copy first (E-10), then the
    /// shipped one.
    pub fn locale_file(&self, iso3: &str) -> PathBuf {
        if let Some(u) = &self.user_locales_dir {
            let p = u.join(iso3).join("main.ftl");
            if p.is_file() {
                return p;
            }
        }
        self.locales_dir.join(iso3).join("main.ftl")
    }

    /// Read Languages.json again (endonym or font changed by E-10).
    pub fn reload_languages(&mut self) {
        self.languages = LanguagesData::load();
    }

    /// The folder of the user's locales (E-10).
    pub fn user_locales_dir(&self) -> Option<&std::path::Path> {
        self.user_locales_dir.as_deref()
    }

    /// Scan the locale folders again (a language was saved by E-10) and
    /// reload the current locale.
    pub fn rescan(&mut self) {
        let mut available = Self::scan_available_locales(&self.locales_dir);
        if let Some(u) = &self.user_locales_dir {
            available.extend(Self::scan_available_locales(u));
        }
        available.sort();
        available.dedup();
        self.available_locales = available;
        let cur = self.current_locale.clone();
        self.fallback_bundle = None;
        self.set_locale(&cur);
    }

    /// The message ids of a locale file (`key = …` lines).
    pub fn message_ids(content: &str) -> Vec<String> {
        content
            .lines()
            .filter_map(|l| {
                let (k, _) = l.split_once(" =")?;
                let first = k.chars().next()?;
                (first.is_ascii_alphabetic()
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .then(|| k.to_string())
            })
            .collect()
    }

    /// The keys of the reference locale (English) that `iso3` lacks: they
    /// are shown in English (R-E07-02).
    pub fn missing_keys(&self, iso3: &str) -> Vec<String> {
        let reference = std::fs::read_to_string(self.locale_file("eng")).unwrap_or_default();
        let own = std::fs::read_to_string(self.locale_file(iso3)).unwrap_or_default();
        let have: std::collections::HashSet<String> = Self::message_ids(&own).into_iter().collect();
        Self::message_ids(&reference)
            .into_iter()
            .filter(|k| !have.contains(k))
            .collect()
    }

    fn load_locale_bundle(&self, iso3: &str) -> Option<FluentBundle<FluentResource>> {
        let main_ftl = self.locale_file(iso3);
        let content = match std::fs::read_to_string(&main_ftl) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to read {main_ftl:?}: {e}");
                return None;
            }
        };
        let iso1 = self
            .languages
            .iso3_to_iso1(iso3)
            .map(|s| s.to_string())
            .unwrap_or_else(|| iso639_3_to_1(iso3));
        let lang_id: LanguageIdentifier = iso1.parse().unwrap_or_else(|_| "en".parse().unwrap());
        let resource = match FluentResource::try_new(content) {
            Ok(res) => res,
            Err((res, errors)) => {
                for err in &errors {
                    tracing::warn!("Fluent parse error in {iso3}: {err:?}");
                }
                res
            }
        };
        let mut bundle = FluentBundle::new(vec![lang_id]);
        // Allow plain text without Unicode isolation marks around arguments.
        bundle.set_use_isolating(false);
        if let Err(errors) = bundle.add_resource(resource) {
            for err in errors {
                tracing::warn!("Fluent bundle error in {iso3}: {err:?}");
            }
        }
        Some(bundle)
    }

    pub fn current_locale(&self) -> &str {
        &self.current_locale
    }

    pub fn available_locales(&self) -> &[String] {
        &self.available_locales
    }

    pub fn locales_dir(&self) -> &std::path::Path {
        &self.locales_dir
    }

    /// True when the code is a known ISO 639-3 language (present in Languages.json).
    pub fn has_language(&self, iso3: &str) -> bool {
        self.languages.contains(iso3)
    }

    /// The language table (Languages.json).
    pub fn languages(&self) -> &LanguagesData {
        &self.languages
    }

    /// Font covering a language's script (path relative to `assets/fonts/`).
    pub fn font_for(&self, iso3: &str) -> Option<&str> {
        self.languages.font_for(iso3)
    }

    /// Human-readable name for a locale code.
    /// ISO 639-3 → ISO 639-1 from the language table (`None` without a
    /// two-letter code).
    pub fn iso3_to_iso1(&self, code: &str) -> Option<String> {
        self.languages.iso3_to_iso1(code).map(str::to_string)
    }

    pub fn display_name(&self, code: &str) -> String {
        if let Some(name) = self.languages.display_name(code) {
            return name.to_string();
        }
        let builtin = locale_display_name(code);
        if !builtin.is_empty() {
            return builtin.to_string();
        }
        code.to_string()
    }

    /// Translate a key.
    pub fn t(&self, key: &str) -> String {
        if crate::l10n::has_msg(key) {
            return self.localize(key);
        }
        self.t_with_args(key, None)
    }

    /// Translate a key with Fluent arguments.
    pub fn t_with_args(&self, key: &str, args: Option<&FluentArgs>) -> String {
        // Arguments holding a message built away from the interface (an
        // error of a reader…) are translated first (crate::l10n).
        let localized;
        let args = match args {
            Some(a)
                if a.iter().any(
                    |(_, v)| matches!(v, FluentValue::String(s) if crate::l10n::has_msg(s)),
                ) =>
            {
                let mut b = FluentArgs::new();
                for (name, v) in a.iter() {
                    match v {
                        FluentValue::String(s) if crate::l10n::has_msg(s) => {
                            b.set(name.to_string(), self.localize(s));
                        }
                        other => b.set(name.to_string(), other.clone()),
                    }
                }
                localized = b;
                Some(&localized)
            }
            other => other,
        };
        if let Some(bundle) = &self.bundle
            && let Some(msg) = bundle.get_message(key)
            && let Some(pattern) = msg.value()
        {
            let mut errors = vec![];
            return bundle
                .format_pattern(pattern, args, &mut errors)
                .to_string();
        }
        if let Some(fallback) = &self.fallback_bundle
            && let Some(msg) = fallback.get_message(key)
            && let Some(pattern) = msg.value()
        {
            let mut errors = vec![];
            return fallback
                .format_pattern(pattern, args, &mut errors)
                .to_string();
        }
        key.to_string()
    }

    /// Translate the messages encoded in a text (crate::l10n): a message
    /// built away from the interface — an error of a reader, of an archive,
    /// of a service — becomes a sentence in the interface language; the rest
    /// of the text is kept. Numeric values are passed as numbers (plurals).
    pub fn localize(&self, text: &str) -> String {
        if !crate::l10n::has_msg(text) {
            return text.to_string();
        }
        let mut out = String::new();
        for piece in crate::l10n::pieces(text) {
            match piece {
                crate::l10n::Piece::Text(t) => out.push_str(&t),
                crate::l10n::Piece::Msg { key, args } => {
                    let mut fa = FluentArgs::new();
                    for (name, value) in args {
                        let value = self.localize(&value);
                        match value.parse::<i64>() {
                            Ok(n) => fa.set(name, n),
                            Err(_) => fa.set(name, value),
                        }
                    }
                    out.push_str(&self.t_with_args(&key, Some(&fa)));
                }
            }
        }
        out
    }

    /// « a — b »: two parts of a message (status line, title).
    pub fn dash(&self, a: &str, b: &str) -> String {
        self.t_args("join-dash", &[("a", a), ("b", b)])
    }

    /// « a: b » (« a : b » in French): a subject and what is said of it.
    pub fn colon(&self, a: &str, b: &str) -> String {
        self.t_args("join-colon", &[("a", a), ("b", b)])
    }

    /// « a (b) »: a name and a detail.
    pub fn paren(&self, a: &str, b: &str) -> String {
        self.t_args("join-paren", &[("a", a), ("b", b)])
    }

    /// « a · b »: items of a short list.
    pub fn dot(&self, a: &str, b: &str) -> String {
        self.t_args("join-dot", &[("a", a), ("b", b)])
    }

    /// « a, b, c »: the items of a list.
    pub fn list(&self, items: &[String]) -> String {
        let mut it = items.iter();
        let Some(first) = it.next() else {
            return String::new();
        };
        it.fold(first.clone(), |acc, x| {
            self.t_args("join-comma", &[("a", &acc), ("b", x)])
        })
    }

    /// A size in bytes, with the unit of the language (« 1.2 MB », « 1,2 Mo »).
    pub fn size(&self, bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * 1024;
        if bytes >= MB {
            self.t_arg("size-mb", "n", &format!("{:.1}", bytes as f64 / MB as f64))
        } else if bytes >= KB {
            self.t_arg("size-kb", "n", &format!("{:.1}", bytes as f64 / KB as f64))
        } else {
            self.t_arg("size-bytes", "n", &bytes.to_string())
        }
    }

    /// Translate with a single string argument.
    pub fn t_arg(&self, key: &str, name: &str, value: &str) -> String {
        let mut args = FluentArgs::new();
        args.set(name, value.to_string());
        self.t_with_args(key, Some(&args))
    }

    /// Translate a key with several string arguments.
    pub fn t_args(&self, key: &str, pairs: &[(&str, &str)]) -> String {
        let mut args = FluentArgs::new();
        for (name, value) in pairs {
            args.set(name.to_string(), value.to_string());
        }
        self.t_with_args(key, Some(&args))
    }

    /// Translate with a numeric argument named `num` (shared with Architect's
    /// messages, F-03).
    pub fn t_num(&self, key: &str, num: i64) -> String {
        let mut args = FluentArgs::new();
        args.set("num", num);
        self.t_with_args(key, Some(&args))
    }

    /// Every language the data files know (ISO 639-3), for the source /
    /// target lists of the FOMOD window.
    pub fn language_codes(&self) -> Vec<String> {
        self.languages
            .languages
            .iter()
            .map(|l| l.iso3.clone())
            .collect()
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize_locale("fr"), "fra");
        assert_eq!(normalize_locale("eng"), "eng");
        assert_eq!(normalize_locale("nso"), "nso"); // preserved
    }

    #[test]
    fn test_iso_fallback() {
        assert_eq!(iso639_3_to_1("fra"), "fr");
        assert_eq!(locale_display_name("fra"), "Français");
    }
}

#[cfg(test)]
mod locale_files_tests {
    use super::I18n;
    use fluent_bundle::{FluentBundle, FluentResource};

    /// T-E07-04: every shipped locale parses without error, has exactly the
    /// keys of the English reference and the same variables in each.
    #[test]
    fn every_locale_is_complete_and_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
        let reference = std::fs::read_to_string(dir.join("eng/main.ftl")).unwrap();
        let ref_ids = I18n::message_ids(&reference);
        // id -> sorted variables of its message (continuation lines included).
        let vars = |text: &str| -> std::collections::HashMap<String, Vec<String>> {
            let mut out: std::collections::HashMap<String, Vec<String>> = Default::default();
            let mut cur: Option<String> = None;
            for l in text.lines() {
                if let Some(id) = I18n::message_ids(l).pop() {
                    cur = Some(id);
                } else if !l.starts_with(' ') {
                    cur = None;
                }
                let Some(id) = &cur else { continue };
                let entry = out.entry(id.clone()).or_default();
                for (p, _) in l.match_indices("{ $") {
                    let v: String = l[p + 3..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    if !entry.contains(&v) {
                        entry.push(v);
                    }
                }
                entry.sort();
            }
            out
        };
        let ref_vars = vars(&reference);
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path().join("main.ftl");
            if !p.is_file() {
                continue;
            }
            count += 1;
            let code = entry.file_name().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&p).unwrap();
            let res = FluentResource::try_new(text.clone())
                .unwrap_or_else(|(_, e)| panic!("{code}: parse errors {e:?}"));
            let mut bundle: FluentBundle<FluentResource> =
                FluentBundle::new(vec!["en".parse().unwrap()]);
            bundle
                .add_resource(res)
                .unwrap_or_else(|e| panic!("{code}: bundle errors {e:?}"));
            let ids = I18n::message_ids(&text);
            let own_vars = vars(&text);
            assert_eq!(ids.len(), ref_ids.len(), "{code}: key count");
            for id in &ref_ids {
                assert!(bundle.has_message(id), "{code}: missing {id}");
                assert_eq!(
                    own_vars.get(id),
                    ref_vars.get(id),
                    "{code}: variables of {id}"
                );
            }
            assert!(
                text.contains(&format!("# @language = {code}")),
                "{code}: header"
            );
        }
        assert!(count >= 33, "{count} locales");
    }

    /// D-35: a language saved in `<config>/locales` is read before the
    /// shipped one and appears in the list after a rescan.
    #[test]
    fn user_locales_come_first() {
        let tmp = std::env::temp_dir().join(format!("ximod_userloc_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("deu")).unwrap();
        std::fs::create_dir_all(tmp.join("xxa")).unwrap();
        std::fs::write(tmp.join("deu/main.ftl"), "menu-file = Datei (Benutzer)\n").unwrap();
        std::fs::write(tmp.join("xxa/main.ftl"), "menu-file = Xa\n").unwrap();
        let mut i = I18n::new();
        i.user_locales_dir = Some(tmp.clone());
        i.rescan();
        assert!(i.available_locales().iter().any(|c| c == "xxa"));
        i.set_locale("deu");
        assert_eq!(i.t("menu-file"), "Datei (Benutzer)");
        // Missing keys of the user copy fall back to English.
        assert_eq!(i.t("btn-close"), "Close");
        assert_eq!(
            i.locale_file("fra"),
            i.locales_dir().join("fra").join("main.ftl")
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Messages built away from the interface (crate::l10n) are translated
    /// in the current language, nested ones and numbers included.
    #[test]
    fn encoded_messages_are_translated() {
        let mut i = I18n::new();
        i.set_locale("fra");
        let inner = crate::l10n!("err-bsa-unreadable", error = "bad header");
        assert_eq!(i.localize(&inner), "BSA illisible : bad header");
        let shown = i.t_arg("err-open", "error", &inner);
        assert_eq!(shown, "Erreur d'ouverture : BSA illisible : bad header");
        assert_eq!(
            i.t(&crate::l10n!("err-plugin-short")),
            "Fichier trop court pour être un plugin"
        );
        assert_eq!(i.localize("plain text"), "plain text");
        assert_eq!(i.colon("a", "b"), "a : b");
        assert_eq!(i.size(1536), "1.5 Ko");
        i.set_locale("eng");
        assert_eq!(i.localize(&inner), "Unreadable BSA: bad header");
        assert_eq!(i.colon("a", "b"), "a: b");
        assert_eq!(
            crate::l10n::strip(&format!("{} x", crate::l10n!("comment-template-name"))),
            "x"
        );
    }

    /// Arabic stays shipped but is neither offered nor selectable until the
    /// interface is drawn right to left (E-08).
    #[test]
    fn rtl_locales_are_held_back() {
        let mut i = I18n::new();
        assert!(i.locales_dir().join("ara").join("main.ftl").is_file());
        assert!(!i.available_locales().iter().any(|c| c == "ara"));
        assert!(i.available_locales().iter().any(|c| c == "fra"));
        i.set_locale("ara");
        assert_eq!(i.current_locale(), "eng");
        i.set_locale("ar");
        assert_eq!(i.current_locale(), "eng");
    }

    /// R-E07-02: a key missing from a locale is shown in English.
    #[test]
    fn missing_key_falls_back_to_english() {
        let mut i = I18n::new();
        i.set_locale("deu");
        assert_eq!(i.current_locale(), "deu");
        assert_ne!(i.t("menu-file"), "menu-file");
        assert!(i.missing_keys("deu").is_empty());
        assert_eq!(i.t("no-such-key-anywhere"), "no-such-key-anywhere");
        assert!(I18n::message_ids("a-b = x\n  cont\n# c\nZz = y\n").len() == 2);
    }
}

#[cfg(test)]
mod undo_message_tests {
    use super::I18n;

    #[test]
    fn undo_done_variants() {
        let mut i = I18n::new();
        i.set_locale("fra");
        let a = i.t_args(
            "undo-done",
            &[("count", "12"), ("hidden", "0"), ("more", "no")],
        );
        assert_eq!(a, "12 lignes annulées");
        let b = i.t_args(
            "undo-done",
            &[("count", "12"), ("hidden", "3"), ("more", "yes")],
        );
        assert_eq!(b, "12 lignes annulées, dont 3 hors du filtre");
        assert_eq!(
            i.t_arg("op-apply-base", "name", "jeu"),
            "Application de la base jeu"
        );
    }
}
