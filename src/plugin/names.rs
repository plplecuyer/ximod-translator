//! The name index of a master file (R-C01-02, decision D-19): the light
//! context pass of `Skyrim.esm` and the like — NPCs with their gender and
//! voice, quests with their aliases, topics, voice types, factions,
//! keywords… — resolved through the master's STRINGS when it is localized,
//! and kept on disk under `cache/names/` keyed by the file's size and date.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::context::{ContextMap, Family};

/// `<config>/cache/names`.
pub fn cache_dir() -> PathBuf {
    crate::config::paths::get()
        .config_dir
        .join("cache")
        .join("names")
}

/// The cache file of a master for a language, or `None` when the master
/// cannot be stat'ed.
pub fn cache_path(master: &Path, lang: &str) -> Option<PathBuf> {
    let meta = std::fs::metadata(master).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = master.file_name()?.to_str()?.replace(['.', ' '], "_");
    Some(cache_dir().join(format!("{stem}_{}_{mtime}_{lang}.json", meta.len())))
}

/// The cached index, when it exists and reads.
pub fn load_cached(master: &Path, lang: &str) -> Option<ContextMap> {
    let p = cache_path(master, lang)?;
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&text).ok()
}

/// Build the index of a master (light pass, STRINGS resolved) and cache it.
pub fn build(
    master: &Path,
    family: Family,
    lang: &str,
    utf8: bool,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<ContextMap> {
    let mut map = ContextMap::read_file_with(master, family, true, progress)?;
    if map.localized
        && let Ok(bundle) = super::StringsBundle::from_plugin_path(master, lang, utf8)
            .or_else(|_| super::StringsBundle::from_archives(master, lang, utf8))
    {
        map.resolve_strings(&bundle);
    }
    // The replies are never kept for a master.
    map.infos.clear();
    map.scenes.clear();
    map.refs.clear();
    if let Some(p) = cache_path(master, lang) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(&map) {
            let _ = crate::config::paths::write_atomic(&p, &json);
        }
    }
    Ok(map)
}

/// The index of a master: the cache, else a fresh build.
pub fn load_or_build(
    master: &Path,
    family: Family,
    lang: &str,
    utf8: bool,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<ContextMap> {
    if let Some(m) = load_cached(master, lang) {
        return Ok(m);
    }
    build(master, family, lang, utf8, progress)
}

/// Where a master may be: next to the plugin, then the game's Data folder.
pub fn find_master(name: &str, plugin_dir: &Path, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut dirs: Vec<&Path> = vec![plugin_dir];
    dirs.extend(data_dirs.iter().map(PathBuf::as_path));
    for d in dirs {
        let p = d.join(name);
        if p.is_file() {
            return Some(p);
        }
        // Case-insensitive match (Linux installs).
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file()
                {
                    return Some(e.path());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_follows_size_and_date() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrim_dialogue.esp");
        let c = cache_path(&p, "eng").unwrap();
        let name = c.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("skyrim_dialogue_esp_"), "{name}");
        assert!(name.ends_with("_eng.json"));
        assert!(cache_path(Path::new("/nowhere/Skyrim.esm"), "eng").is_none());
        let dir = p.parent().unwrap();
        // Compared through `canonicalize`: on Windows the file system is
        // case-insensitive, so the upper-case name is found as is and the
        // returned path differs from `p` in case and separators only.
        let found = find_master("SKYRIM_DIALOGUE.ESP", dir, &[]).expect("found");
        assert_eq!(found.canonicalize().unwrap(), p.canonicalize().unwrap());
        assert!(find_master("Skyrim.esm", dir, &[]).is_none());
    }
}
