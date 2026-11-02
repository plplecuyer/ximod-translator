//! Localized-plugin support: resolve a plugin's string ids against external
//! STRINGS files (reading) and build a translated set of STRINGS files (writing,
//! CDC output mode 1).
//!
//! A localized plugin (Skyrim SE, Fallout 4, Starfield) stores a 4-byte string id
//! in each translatable field; [`super::reader::extract_strings`] records that id
//! in `TmEntry::idstexte` and leaves `source` empty. Here we fill `source` (and
//! `list_index`) from the sidecar `.STRINGS`/`.DLSTRINGS`/`.ILSTRINGS`, and,
//! symmetrically, emit translated STRINGS files from a translation memory.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};

use crate::tm::strings::{self, StringsKind};
use crate::tm::{TmEntry, TranslationMemory};

/// The three STRINGS files of a plugin/language, indexed by string id. Each id
/// maps to its text and the list (0/1/2) it came from, so the writer can place a
/// translation back into the same file.
#[derive(Debug, Default, Clone)]
pub struct StringsBundle {
    map: HashMap<u32, (String, u8)>,
}

impl StringsBundle {
    /// List the language tokens (lowercased) for which STRINGS exist for this
    /// plugin — scanning its folder and a sibling `Strings/` for loose
    /// `<stem>_<lang>.<KIND>` files, plus any sibling BSA/BA2 whose name matches
    /// the plugin. Used to check whether a requested source language is actually
    /// available before (re)loading a localized plugin. Empty when none is found.
    pub fn available_langs(plugin_path: &Path) -> Vec<String> {
        use std::collections::BTreeSet;
        let mut set: BTreeSet<String> = BTreeSet::new();
        let Some(stem) = plugin_path.file_stem().and_then(|s| s.to_str()) else {
            return Vec::new();
        };
        let dir = plugin_path.parent().unwrap_or_else(|| Path::new("."));

        // Loose STRINGS next to the plugin (and a sibling `Strings/`).
        for d in [dir.to_path_buf(), dir.join("Strings")] {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let Some(name) = fname.to_str() else { continue };
                if let Some((lang, _)) = crate::lang::match_strings_filename(name, stem) {
                    set.insert(lang.to_lowercase());
                }
            }
        }

        // STRINGS packed in a sibling BSA/BA2 (Creation Club, many mods).
        let stem_norm = crate::lang::normalize_name(stem);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext != "bsa" && ext != "ba2" {
                    continue;
                }
                let astem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !crate::lang::normalize_name(astem).starts_with(&stem_norm) {
                    continue;
                }
                if let Ok(archive) = crate::archive::Archive::open(&p) {
                    for e in archive.entries() {
                        if let Some((lang, _)) =
                            crate::lang::match_strings_filename(e.file_name(), stem)
                        {
                            set.insert(lang.to_lowercase());
                        }
                    }
                }
            }
        }
        set.into_iter().collect()
    }

    /// Build from the raw bytes of the three files (any may be empty/absent).
    pub fn from_files(
        strings_bytes: Option<&[u8]>,
        dlstrings_bytes: Option<&[u8]>,
        ilstrings_bytes: Option<&[u8]>,
        utf8: bool,
    ) -> Result<StringsBundle> {
        let mut map = HashMap::new();
        for (bytes, kind) in [
            (strings_bytes, StringsKind::Strings),
            (dlstrings_bytes, StringsKind::DlStrings),
            (ilstrings_bytes, StringsKind::IlStrings),
        ] {
            if let Some(b) = bytes {
                for (id, text) in strings::parse(b, kind, utf8)? {
                    map.insert(id, (text, kind.list_index()));
                }
            }
        }
        Ok(StringsBundle { map })
    }

    /// Locate and load the three loose STRINGS files next to a plugin by
    /// **scanning** its directory (and a sibling `Strings/`) case-insensitively
    /// for `<stem>_<lang>.<KIND>` files — so casing and the code form (`French`
    /// vs `fr`) don't matter, which matters on Linux/macOS.
    ///
    /// `preferred` (an ISO code or a Bethesda name) selects which language to load
    /// when several are present; otherwise the first language found is used.
    pub fn from_plugin_path(
        plugin_path: &Path,
        preferred: &str,
        utf8: bool,
    ) -> Result<StringsBundle> {
        use std::collections::HashMap;

        let stem = plugin_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-plugin-no-name")))?;
        let dir = plugin_path.parent().unwrap_or_else(|| Path::new("."));

        // language token (lowercased) → kind → file bytes
        let mut found: HashMap<String, HashMap<StringsKind, Vec<u8>>> = HashMap::new();
        for d in [dir.to_path_buf(), dir.join("Strings")] {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let Some(name) = fname.to_str() else { continue };
                if let Some((lang, kind)) = crate::lang::match_strings_filename(name, stem)
                    && let Ok(bytes) = std::fs::read(entry.path())
                {
                    found
                        .entry(lang.to_lowercase())
                        .or_default()
                        .insert(kind, bytes);
                }
            }
        }
        if found.is_empty() {
            bail!(crate::l10n!("err-strings-none", plugin = stem));
        }

        // Choose the language: the preferred one if present, else any.
        let chosen = found
            .keys()
            .find(|tok| crate::lang::token_matches(tok, preferred))
            .cloned()
            .or_else(|| found.keys().next().cloned())
            .unwrap();
        let files = &found[&chosen];
        Self::from_files(
            files.get(&StringsKind::Strings).map(|v| v.as_slice()),
            files.get(&StringsKind::DlStrings).map(|v| v.as_slice()),
            files.get(&StringsKind::IlStrings).map(|v| v.as_slice()),
            utf8,
        )
    }

    /// Locate and load the three STRINGS from a sibling **BSA/BA2 archive** next
    /// to the plugin — the Creation Club (and many mods) ship their STRINGS inside
    /// `<plugin>.bsa` under a `strings/` folder rather than loose on disk. Only
    /// archives whose name matches the plugin (ignoring case/separators) are
    /// opened, and inside them only the `<plugin>_<lang>.<KIND>` entries are read.
    ///
    /// `preferred` selects the language when several are present.
    pub fn from_archives(plugin_path: &Path, preferred: &str, utf8: bool) -> Result<StringsBundle> {
        let stem = plugin_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-plugin-no-name")))?;
        let dir = plugin_path.parent().unwrap_or_else(|| Path::new("."));
        let stem_norm = crate::lang::normalize_name(stem);

        // Sibling archives whose (normalized) stem starts with the plugin's — so
        // `X.bsa` and `X - Textures.bsa` are considered, but not unrelated ones.
        let mut archives: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext != "bsa" && ext != "ba2" {
                    continue;
                }
                let astem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if crate::lang::normalize_name(astem).starts_with(&stem_norm) {
                    archives.push(p);
                }
            }
        }
        archives.sort();

        let mut found: HashMap<String, HashMap<StringsKind, Vec<u8>>> = HashMap::new();
        for apath in &archives {
            let Ok(archive) = crate::archive::Archive::open(apath) else {
                continue;
            };
            for (i, e) in archive.entries().iter().enumerate() {
                if let Some((lang, kind)) = crate::lang::match_strings_filename(e.file_name(), stem)
                    && let Ok(bytes) = archive.extract(i)
                {
                    found
                        .entry(lang.to_lowercase())
                        .or_default()
                        .insert(kind, bytes);
                }
            }
        }
        if found.is_empty() {
            bail!(crate::l10n!("err-strings-none-archive", plugin = stem));
        }

        let chosen = found
            .keys()
            .find(|tok| crate::lang::token_matches(tok, preferred))
            .cloned()
            .or_else(|| found.keys().next().cloned())
            .unwrap();
        let files = &found[&chosen];
        Self::from_files(
            files.get(&StringsKind::Strings).map(|v| v.as_slice()),
            files.get(&StringsKind::DlStrings).map(|v| v.as_slice()),
            files.get(&StringsKind::IlStrings).map(|v| v.as_slice()),
            utf8,
        )
    }

    /// The text for a string id, if present in any of the three files.
    pub fn get(&self, id: u32) -> Option<&str> {
        self.map.get(&id).map(|(t, _)| t.as_str())
    }

    /// Whether the bundle holds any strings at all.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Fill `source` and `list_index` on every localized entry from this bundle.
    /// Returns how many entries were resolved.
    pub fn resolve(&self, tm: &mut TranslationMemory) -> usize {
        let mut n = 0;
        for e in &mut tm.entries {
            if e.idstexte >= 0
                && let Some((text, list)) = self.map.get(&(e.idstexte as u32))
            {
                e.source = text.clone();
                e.list_index = *list;
                n += 1;
            }
        }
        n
    }
}

/// Build the translated STRINGS files (output mode 1) from a translation memory
/// whose entries carry string ids. Every referenced id is emitted — with its
/// translation when effective, otherwise the original source — into the file it
/// belongs to (by `list_index`). Returns `(kind, bytes)` for each non-empty file.
pub fn build_strings_files(tm: &TranslationMemory, utf8: bool) -> Vec<(StringsKind, Vec<u8>)> {
    // Per-list id → text, preferring a translated value over the source.
    let mut lists: [HashMap<u32, String>; 3] = Default::default();
    let mut translated: [HashMap<u32, bool>; 3] = Default::default();

    for e in &tm.entries {
        if e.idstexte < 0 {
            continue; // not a localized string
        }
        let id = e.idstexte as u32;
        let slot = (e.list_index.min(2)) as usize;
        let text = pick_text(e);
        let is_tr = e.is_translated();
        // Prefer a translated value; don't let an untranslated duplicate clobber it.
        let already_tr = *translated[slot].get(&id).unwrap_or(&false);
        if !lists[slot].contains_key(&id) || (is_tr && !already_tr) {
            lists[slot].insert(id, text);
            translated[slot].insert(id, is_tr);
        }
    }

    let mut out = Vec::new();
    for (slot, kind) in [
        StringsKind::Strings,
        StringsKind::DlStrings,
        StringsKind::IlStrings,
    ]
    .into_iter()
    .enumerate()
    {
        if lists[slot].is_empty() {
            continue;
        }
        // Deterministic order by id.
        let mut entries: Vec<(u32, String)> = lists[slot].drain().collect();
        entries.sort_by_key(|(id, _)| *id);
        out.push((kind, strings::write(&entries, kind, utf8)));
    }
    out
}

fn pick_text(e: &TmEntry) -> String {
    if e.is_translated() {
        e.trans.clone()
    } else {
        e.source.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_langs_lists_loose_strings() {
        let dir = std::env::temp_dir().join("ximod_avail_langs_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Sidecar STRINGS for two languages, plus an unrelated file.
        for name in [
            "MyMod_English.strings",
            "MyMod_English.dlstrings",
            "MyMod_French.strings",
            "MyMod.esp",
            "Other_German.strings",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let langs = StringsBundle::available_langs(&dir.join("MyMod.esp"));
        assert!(langs.iter().any(|l| l == "english"));
        assert!(langs.iter().any(|l| l == "french"));
        // `Other_German` belongs to a different plugin stem → not listed.
        assert!(!langs.iter().any(|l| l == "german"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn available_langs_empty_when_none() {
        let dir = std::env::temp_dir().join("ximod_avail_langs_none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("MyMod.esp"), b"x").unwrap();
        let langs = StringsBundle::available_langs(&dir.join("MyMod.esp"));
        assert!(langs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
