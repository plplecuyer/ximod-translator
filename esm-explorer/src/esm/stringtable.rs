//! Locate and load a localized plugin's STRINGS into a single id→text table.
//!
//! Looks for `<Plugin>_<lang>.STRINGS/DLSTRINGS/ILSTRINGS` both as **loose files**
//! (next to the plugin and in a sibling `Strings/` folder) and inside **BSA/BA2
//! archives** next to the plugin. Archive scanning is deliberately narrow — only
//! an archive whose name matches the plugin exactly, or a localization archive
//! (name contains "localization"/"strings") — so we never crack open multi-GB
//! texture archives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::archive::Archive;
use super::games::Game;
use super::lang::{self, match_strings_filename, token_matches};
use super::strings::{self, StringsKind};

/// Resolved string table (ids are unique across the three files, so one map).
#[derive(Default)]
pub struct StringTable {
    pub map: HashMap<u32, String>,
}

impl StringTable {
    pub fn get(&self, id: u32) -> Option<&str> {
        self.map.get(&id).map(String::as_str)
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Outcome of a load: the table for the chosen language, the languages available,
/// the chosen one, and a short list of the sources used (for the status bar).
pub struct StringsLoad {
    pub table: StringTable,
    pub languages: Vec<String>,
    pub chosen: Option<String>,
    pub sources: Vec<String>,
}

struct LooseCand {
    path: PathBuf,
    kind: StringsKind,
    lang: String,
}
struct ArcCand {
    arc: usize,
    order: usize,
    kind: StringsKind,
    lang: String,
}

/// Whether an archive next to the plugin should be opened to look for STRINGS.
fn archive_is_candidate(plugin_stem_norm: &str, archive_path: &Path) -> bool {
    let stem = archive_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let a = lang::normalize_name(stem);
    let name_lower = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    a == plugin_stem_norm
        || (a.starts_with(plugin_stem_norm)
            && (name_lower.contains("localization") || name_lower.contains("strings")))
}

/// Find and load the STRINGS of a localized plugin. `preferred` is an ISO code or
/// Bethesda name (e.g. "English"); when absent among the available languages the
/// loader falls back to English, else the first found.
pub fn load(plugin_path: &Path, game: Game, preferred: &str) -> StringsLoad {
    let utf8 = game.uses_utf8();
    let stem = plugin_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let stem_norm = lang::normalize_name(&stem);
    let folder = plugin_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    // ---- discover loose candidates (folder + sibling Strings/) ----
    let mut loose: Vec<LooseCand> = Vec::new();
    let mut dirs = vec![folder.clone()];
    if let Ok(rd) = std::fs::read_dir(&folder) {
        for e in rd.flatten() {
            if e.path().is_dir()
                && e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("strings")
            {
                dirs.push(e.path());
            }
        }
    }
    for dir in &dirs {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some((l, k)) = match_strings_filename(&name, &stem) {
                    loose.push(LooseCand {
                        path: e.path(),
                        kind: k,
                        lang: l,
                    });
                }
            }
        }
    }

    // ---- discover archive candidates (narrow filter) ----
    let mut archives: Vec<Archive> = Vec::new();
    let mut arc_cands: Vec<ArcCand> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&folder) {
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                matches!(
                    p.extension()
                        .and_then(|x| x.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("bsa") | Some("ba2")
                )
            })
            .filter(|p| archive_is_candidate(&stem_norm, p))
            .collect();
        paths.sort();
        for p in paths {
            if let Ok(a) = Archive::open(&p) {
                let ai = archives.len();
                for (order, e) in a.entries().iter().enumerate() {
                    if let Some((l, k)) = match_strings_filename(e.file_name(), &stem) {
                        arc_cands.push(ArcCand {
                            arc: ai,
                            order,
                            kind: k,
                            lang: l,
                        });
                    }
                }
                archives.push(a);
            }
        }
    }

    // ---- available languages (deduped by ISO equivalence) ----
    let mut languages: Vec<String> = Vec::new();
    let mut add_lang = |tok: &str| {
        if !languages.iter().any(|l| token_matches(l, tok)) {
            languages.push(tok.to_string());
        }
    };
    for c in &loose {
        add_lang(&c.lang);
    }
    for c in &arc_cands {
        add_lang(&c.lang);
    }

    if languages.is_empty() {
        return StringsLoad {
            table: StringTable::default(),
            languages,
            chosen: None,
            sources: Vec::new(),
        };
    }

    // ---- choose a language ----
    let chosen = languages
        .iter()
        .find(|l| token_matches(l, preferred))
        .or_else(|| languages.iter().find(|l| token_matches(l, "en")))
        .cloned()
        .unwrap_or_else(|| languages[0].clone());

    // ---- load the chosen language's files, merging into one table ----
    let mut table = StringTable::default();
    let mut sources: Vec<String> = Vec::new();
    for c in &loose {
        if !token_matches(&c.lang, &chosen) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&c.path)
            && let Ok(pairs) = strings::parse(&bytes, c.kind, utf8)
        {
            for (id, text) in pairs {
                table.map.insert(id, text);
            }
            if let Some(n) = c.path.file_name().and_then(|s| s.to_str()) {
                sources.push(n.to_string());
            }
        }
    }
    for c in &arc_cands {
        if !token_matches(&c.lang, &chosen) {
            continue;
        }
        if let Ok(bytes) = archives[c.arc].extract(c.order)
            && let Ok(pairs) = strings::parse(&bytes, c.kind, utf8)
        {
            for (id, text) in pairs {
                table.map.insert(id, text);
            }
            sources.push(format!("{} (archive)", archives[c.arc].file_name()));
        }
    }

    StringsLoad {
        table,
        languages,
        chosen: Some(chosen),
        sources,
    }
}
