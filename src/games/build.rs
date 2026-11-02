//! Build game translation dictionaries from a game's *installed* STRINGS.
//!
//! At startup ximod-translator can ship **no** game dictionaries and instead
//! generate them from the games found on the machine (see `games::detect`).
//! For each localized game we gather every language's STRINGS — base game **and**
//! DLC (see `games::dlc`) — found loose in `<game>/Data/Strings` and/or packed in
//! the interface/localization and DLC BSA/BA2 archives, then write one
//! `assets/bases/jeux/<Jeu>_ENG-<LANG>.ximdb` for **every** English→other-language
//! pair present (not just the configured pair).
//!
//! The pass is **incremental**: a game's dictionaries are skipped when they exist
//! and the STRINGS fingerprint (source names + sizes + mtimes) is unchanged, so
//! relaunching is cheap while still catching newly installed games/languages.
//! Only the localized engines (Skyrim LE/SE/AE, Fallout 4/76, Starfield) ship
//! external STRINGS; older games keep their text inside the plugin and are built
//! via the plugin-pair flow instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::archive::Archive;
use crate::config::AppConfig;
use crate::games::{Game, detect};
use crate::tm::strings::{self, StringsKind};

/// Outcome of a regeneration pass.
#[derive(Debug, Default, Clone)]
pub struct BuildReport {
    /// Games considered (registered with a path).
    pub games: usize,
    /// Dictionaries (re)built this pass.
    pub built: usize,
    /// Dictionaries left untouched (up to date).
    pub skipped: usize,
    /// One human-readable line per game (for the startup diagnostic log).
    pub details: Vec<String>,
}

/// Case-insensitive child lookup (games copied from Windows carry mixed casing).
fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if e.file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            return Some(e.path());
        }
    }
    None
}

/// Split a STRINGS file name `<base>_<lang>.<KIND>` → `(base, lang, kind)`.
fn split_strings(name: &str) -> Option<(String, String, StringsKind)> {
    let dot = name.rfind('.')?;
    let kind = strings::kind_from_extension(&name[dot + 1..])?;
    let base = &name[..dot];
    let us = base.rfind('_')?;
    let (b, lang) = (&base[..us], &base[us + 1..]);
    if b.is_empty() || lang.is_empty() {
        return None;
    }
    Some((b.to_string(), lang.to_string(), kind))
}

/// STRINGS files directly in `dir`, as `(name, path, base, lang, kind)`.
fn strings_files(dir: &Path) -> Vec<(String, PathBuf, String, String, StringsKind)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((base, lang, kind)) = split_strings(name) {
            out.push((name.to_string(), p.clone(), base, lang, kind));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Key identifying one string across languages: `(plugin base, list, id)`.
type Key = (String, u8, i32);

/// A dictionary row: `(rec_sig, field_sig, formid, edid, source, translation)`.
/// Game dictionaries leave the first four empty and match on source text.
type Row = crate::basedb::BaseRow;

/// `(size, mtime_secs)` of a file, `(0, 0)` on error.
fn meta_size_mtime(p: &Path) -> (u64, u64) {
    std::fs::metadata(p)
        .map(|m| {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (m.len(), mt)
        })
        .unwrap_or((0, 0))
}

/// Every language's strings for a game: canonical ISO 639-3 code → map of
/// `(base, list, id) → text`, gathered across all plugins (base game + DLC).
type LangMaps = std::collections::BTreeMap<String, HashMap<Key, String>>;

/// Games whose text lives in external STRINGS (loose or archived). Older engines
/// keep it inline in the plugin and are handled via the plugin-pair flow.
fn is_localized(game: Game) -> bool {
    matches!(
        game,
        Game::Skyrim | Game::SkyrimSE | Game::Fallout4 | Game::Fallout76 | Game::Starfield
    )
}

/// Parse STRINGS `bytes` and file the entries under their language (by ISO 639-3
/// derived from the filename token), so `English`/`en`/`eng` all group together.
fn ingest(langs: &mut LangMaps, base: &str, lang_token: &str, kind: StringsKind, bytes: &[u8]) {
    let Some(iso3) = crate::lang::iso3_from_token(lang_token) else {
        return; // unknown language token
    };
    if let Ok(tm) = strings::load_as_tm(bytes, kind, true) {
        let map = langs.entry(iso3.to_string()).or_default();
        for e in tm.entries {
            map.insert((base.to_lowercase(), e.list_index, e.idstexte), e.source);
        }
    }
}

/// Whether an archive's name suggests it holds STRINGS (interface/localization),
/// so we avoid loading multi-GB texture/mesh archives into memory.
fn archive_maybe_strings(name_lower: &str) -> bool {
    name_lower.contains("interface")
        || name_lower.contains("localization")
        || name_lower.contains("strings")
}

/// Skip archives larger than this (bytes) as a memory safety net: STRINGS-bearing
/// archives (interface/localization, DLC main) are well under it; only texture
/// mega-packs exceed it and they never contain STRINGS.
const ARCHIVE_SIZE_CAP: u64 = 6 * 1024 * 1024 * 1024;

/// Whether to open this archive for STRINGS: an interface/localization archive,
/// or one belonging to a listed DLC (name starts with / contains the DLC base).
fn archive_relevant(name_lower: &str, dlc_lower: &[String]) -> bool {
    archive_maybe_strings(name_lower)
        || dlc_lower
            .iter()
            .any(|d| name_lower.starts_with(d) || name_lower.contains(d))
}

/// The `.bsa`/`.ba2` files directly in `data`, sorted.
fn archives_in(data: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(data) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .as_deref(),
                    Some("bsa") | Some("ba2")
                )
        })
        .collect();
    v.sort();
    v
}

/// A cheap fingerprint of a game's STRINGS sources (names + sizes + mtimes), with
/// NO archive opening — so an unchanged install is detected without extracting.
fn fingerprint_sources(data: &Path, dlc_lower: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(sdir) = child_ci(data, "Strings") {
        for (name, path, _, lang, _) in strings_files(&sdir) {
            if crate::lang::iso3_from_token(&lang).is_none() {
                continue;
            }
            let (size, mtime) = meta_size_mtime(&path);
            lines.push(format!("loose:{name}|{size}|{mtime}"));
        }
    }
    for ap in archives_in(data) {
        let name_lower = ap
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_lowercase())
            .unwrap_or_default();
        if !archive_relevant(&name_lower, dlc_lower) {
            continue;
        }
        let (size, mtime) = meta_size_mtime(&ap);
        if size > ARCHIVE_SIZE_CAP {
            continue;
        }
        let n = ap.file_name().and_then(|n| n.to_str()).unwrap_or("");
        lines.push(format!("bsa:{n}|{size}|{mtime}"));
    }
    lines.sort();
    lines.join("\n")
}

/// Collect **all** languages' STRINGS for a game: loose `Data/Strings` (every
/// plugin base and language) plus the STRINGS packed in the game's
/// interface/localization archives and each listed DLC archive. Entries stay
/// keyed by `(plugin base, list, id)` so the caller can split the base game from
/// each DLC.
fn collect_all_languages(data: &Path, dlc_bases: &[String]) -> LangMaps {
    let mut langs = LangMaps::new();
    let dlc_lower: Vec<String> = dlc_bases.iter().map(|d| d.to_lowercase()).collect();

    // 1) Loose STRINGS (all bases, all languages).
    if let Some(sdir) = child_ci(data, "Strings") {
        for (_, path, base, lang, kind) in strings_files(&sdir) {
            if crate::lang::iso3_from_token(&lang).is_none() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            ingest(&mut langs, &base, &lang, kind, &bytes);
        }
    }

    // 2) Archived STRINGS (base game interface/localization + listed DLC).
    for ap in archives_in(data) {
        let name_lower = ap
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_lowercase())
            .unwrap_or_default();
        if !archive_relevant(&name_lower, &dlc_lower) {
            continue;
        }
        if meta_size_mtime(&ap).0 > ARCHIVE_SIZE_CAP {
            continue;
        }
        let Ok(archive) = Archive::open(&ap) else {
            continue;
        };
        for (idx, entry) in archive.entries().iter().enumerate() {
            let fname = entry
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(entry.path.as_str());
            let Some((base, lang, kind)) = split_strings(fname) else {
                continue;
            };
            if crate::lang::iso3_from_token(&lang).is_none() {
                continue;
            }
            let Ok(bytes) = archive.extract(idx) else {
                continue;
            };
            ingest(&mut langs, &base, &lang, kind, &bytes);
        }
    }
    langs
}

/// (Re)generate the game dictionaries. For every localized game, ALL
/// English→other-language pairs found in its installed STRINGS (base game + the
/// DLC listed in `dlc`) are written to `jeux_dir` as
/// `<GameId>_ENG-<LANG>.ximdb`. STRINGS are read from a loose `Data/Strings`
/// folder and/or the game's interface/localization and DLC BSA/BA2 archives. The
/// pass is incremental (a game's outputs are skipped when its STRINGS fingerprint
/// is unchanged). Non-localized games have no STRINGS and are left for the
/// plugin-pair flow. Calls `on_step(done, total, label)` per game and once at the
/// end. Returns a [`BuildReport`].
pub fn regenerate(
    cfg: &AppConfig,
    jeux_dir: &Path,
    dlc: &crate::games::dlc::DlcMap,
    mut on_step: impl FnMut(usize, usize, &str),
) -> BuildReport {
    let mut report = BuildReport::default();

    // Stable order for deterministic progress.
    let entries: Vec<(&String, &String)> = cfg
        .games
        .iter()
        .filter(|(_, e)| !e.path.trim().is_empty())
        .map(|(id, e)| (id, &e.path))
        .collect();
    let total = entries.len();
    report.games = total;
    let _ = std::fs::create_dir_all(jeux_dir);

    let no_dlc: Vec<String> = Vec::new();
    for (i, (id, path)) in entries.iter().enumerate() {
        let game = Game::from_id(id);
        on_step(i, total, game.display_name());

        if !is_localized(game) {
            report.details.push(format!(
                "{id}: no external STRINGS (old engine) — build via plugin pair"
            ));
            continue;
        }

        let root = PathBuf::from(path.trim());
        let Some(data) = child_ci(&root, detect::data_subdir(game)) else {
            report
                .details
                .push(format!("{id}: no data folder under {}", root.display()));
            continue;
        };

        let safe: String = id
            .chars()
            .map(|c| if std::path::is_separator(c) { '_' } else { c })
            .collect();
        let dlc_bases = dlc.get(id.as_str()).unwrap_or(&no_dlc);
        let dlc_lower: Vec<String> = dlc_bases.iter().map(|d| d.to_lowercase()).collect();
        // base(lowercased) → DLC display name (nice casing from the JSON).
        let dlc_display: HashMap<String, String> = dlc_bases
            .iter()
            .map(|d| (d.to_lowercase(), d.clone()))
            .collect();

        // Each game's dictionaries live in their own sub-folder: jeux/<GameId>/.
        let gdir = jeux_dir.join(&safe);
        let stamp = gdir.join(".sources.stamp");
        let fp = fingerprint_sources(&data, &dlc_lower);

        // Migrate any dictionaries left at the jeux root by older versions.
        migrate_root_dicts(jeux_dir, &safe, &gdir);

        // Incremental: skip the whole game (no archive reading) when its sources
        // are unchanged and at least one dictionary already exists.
        let up_to_date = std::fs::read_to_string(&stamp).ok().as_deref() == Some(fp.as_str())
            && dir_has_ximdb(&gdir);
        if up_to_date {
            report.skipped += 1;
            report.details.push(format!("{id}: up to date"));
            continue;
        }

        let langs = collect_all_languages(&data, dlc_bases);
        let Some(eng) = langs.get("eng").filter(|m| !m.is_empty()) else {
            report.details.push(format!(
                "{id}: no English STRINGS found (source); languages present: [{}]",
                langs.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
            continue;
        };
        // The localizable fields of the game, from its masters (C-09,
        // decision D-62): written in the user's data folder, they take
        // the place of the shipped table.
        report
            .details
            .push(derive_lstrings(game, id, &data, eng, &cfg_data_dir()));
        let targets: Vec<&String> = langs.keys().filter(|l| l.as_str() != "eng").collect();
        if targets.is_empty() {
            report.details.push(format!(
                "{id}: only English STRINGS present → no pair to build ({} entries)",
                eng.len()
            ));
            continue;
        }

        let _ = std::fs::create_dir_all(&gdir);
        // The folder is auto-managed: clear previous auto dictionaries so removed
        // languages/DLC don't linger.
        clear_ximdb(&gdir);

        let mut built_here: Vec<String> = Vec::new();
        for tgt in &targets {
            let tmap = &langs[*tgt];
            let up = tgt.to_uppercase();

            // Split rows by plugin base: base game vs each DLC → separate files.
            let mut buckets: std::collections::BTreeMap<String, Vec<Row>> =
                std::collections::BTreeMap::new();
            for ((base, li, tid), s) in eng {
                if let Some(t) = tmap.get(&(base.clone(), *li, *tid))
                    && !t.is_empty()
                    && t != s
                {
                    let out_name = dlc_display
                        .get(base)
                        .cloned()
                        .unwrap_or_else(|| id.to_string());
                    buckets
                        .entry(out_name)
                        .or_default()
                        .push(crate::basedb::BaseRow {
                            source: s.clone(),
                            trans: t.clone(),
                            ..Default::default()
                        });
                }
            }
            for (out_name, rows) in buckets {
                if rows.is_empty() {
                    continue;
                }
                let n = rows.len();
                let safe_name: String = out_name
                    .chars()
                    .map(|c| if std::path::is_separator(c) { '_' } else { c })
                    .collect();
                let out = gdir.join(format!("{safe_name}_ENG-{up}.ximdb"));
                if crate::basedb::write_ximdb(rows, tgt, &out).is_ok() {
                    report.built += 1;
                    built_here.push(format!("{safe_name}_ENG-{up}({n})"));
                } else {
                    report
                        .details
                        .push(format!("{id}: write failed for {}", out.display()));
                }
            }
        }
        let _ = std::fs::write(&stamp, &fp);
        report.details.push(format!(
            "{id}: {} English entries → {}",
            eng.len(),
            if built_here.is_empty() {
                "no differing pairs".to_string()
            } else {
                built_here.join(", ")
            }
        ));
    }
    on_step(total, total, "");
    report
}

/// The user's data folder (the derived tables go there).
fn cfg_data_dir() -> PathBuf {
    crate::config::paths::get().data_dir.clone()
}

/// Derive the table of the localizable fields from the first master of
/// the game found in `data`, with the identifiers of its English STRINGS
/// (R-C09-01), and write it under `user_dir/lstrings/`. Returns the line
/// of the report.
fn derive_lstrings(
    game: Game,
    id: &str,
    data: &Path,
    eng: &HashMap<Key, String>,
    user_dir: &Path,
) -> String {
    let reg = crate::games::registry();
    let masters = reg.spec(game).masters.clone();
    for master in &masters {
        let Some(path) = child_ci(data, master) else {
            continue;
        };
        let base = Path::new(master)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mut ids: [std::collections::HashSet<u32>; 3] = Default::default();
        for (b, list, sid) in eng.keys() {
            if *b == base && *sid > 0 {
                ids[(*list as usize).min(2)].insert(*sid as u32);
            }
        }
        if ids.iter().all(|s| s.is_empty()) {
            return format!("{id}: no STRINGS ids for {master} — localizable fields not derived");
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        // SAFETY: read-only mapping of the game's master.
        let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
            continue;
        };
        let date = crate::backup::today();
        return match crate::lstrings::derive(game, master, &map[..], &ids, &date) {
            Ok(table) => {
                let out = crate::lstrings::Table::user_path(game, user_dir);
                if let Some(dir) = out.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&out, table.to_json()) {
                    Ok(()) => format!(
                        "{id}: {} localizable fields derived from {master}",
                        table.len()
                    ),
                    Err(e) => format!("{id}: localizable fields not written ({e})"),
                }
            }
            Err(e) => format!("{id}: localizable fields not derived from {master} ({e})"),
        };
    }
    format!("{id}: no master found — shipped table of localizable fields kept")
}

/// Whether `dir` directly contains at least one `.ximdb` file.
fn dir_has_ximdb(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("ximdb"))
            })
        })
        .unwrap_or(false)
}

/// Delete every `.ximdb` directly in `dir` (auto-managed game folder).
fn clear_ximdb(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("ximdb"))
            {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// Remove dictionaries an older version wrote flat in `jeux_dir` for this game
/// (`<GameId>_*.ximdb` and their `.stamp`), now superseded by the `<GameId>/`
/// sub-folder layout.
fn migrate_root_dicts(jeux_dir: &Path, safe_id: &str, gdir: &Path) {
    let Ok(rd) = std::fs::read_dir(jeux_dir) else {
        return;
    };
    let prefix = format!("{safe_id}_");
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() || p == gdir {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with(&prefix) && (name.ends_with(".ximdb") || name.ends_with(".ximdb.stamp"))
        {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, GameEntry};
    use crate::tm::model::{TmEntry, TmMeta, TranslationMemory};

    /// Write a `<base>_<Lang>.STRINGS` file with the given (id → text) entries.
    fn write_strings(dir: &Path, base: &str, lang: &str, entries: &[(u32, &str)]) {
        let tm = TranslationMemory::new(
            entries
                .iter()
                .map(|(id, text)| TmEntry {
                    source: text.to_string(),
                    idstexte: *id as i32,
                    icon: -1,
                    list_index: 0,
                    index: 1,
                    ..Default::default()
                })
                .collect(),
            TmMeta::default(),
        );
        let bytes = strings::write_from_tm(&tm, StringsKind::Strings, true);
        std::fs::write(dir.join(format!("{base}_{lang}.STRINGS")), bytes).unwrap();
    }

    fn cfg_with_game(root: &Path) -> AppConfig {
        let mut c = AppConfig {
            source_language: "eng".into(),
            target_language: "fra".into(),
            ..AppConfig::default()
        };
        c.games.insert(
            "SkyrimSE".into(),
            GameEntry {
                path: root.display().to_string(),
                source: "Steam".into(),
                locked: false,
            },
        );
        c
    }

    #[test]
    fn builds_then_skips_incrementally() {
        let base = std::env::temp_dir().join("ximod_build_test");
        let _ = std::fs::remove_dir_all(&base);
        let strings = base.join("game/Data/Strings");
        std::fs::create_dir_all(&strings).unwrap();
        write_strings(
            &strings,
            "Skyrim",
            "English",
            &[(1, "Sword"), (2, "Shield")],
        );
        write_strings(
            &strings,
            "Skyrim",
            "French",
            &[(1, "Épée"), (2, "Bouclier")],
        );

        let cfg = cfg_with_game(&base.join("game"));
        let jeux = base.join("bases/jeux");

        // First pass builds one dictionary, inside the game's sub-folder.
        let r1 = regenerate(&cfg, &jeux, &crate::games::dlc::DlcMap::new(), |_, _, _| {});
        assert_eq!(r1.built, 1);
        assert_eq!(r1.skipped, 0);
        let out = jeux.join("SkyrimSE").join("SkyrimSE_ENG-FRA.ximdb");
        assert!(out.is_file());
        assert!(jeux.join("SkyrimSE").join(".sources.stamp").is_file());

        // The dictionary translates Sword → Épée.
        let tm = crate::basedb::ximdb_to_tm(&out).unwrap();
        assert!(
            tm.entries
                .iter()
                .any(|e| e.source == "Sword" && e.trans == "Épée")
        );

        // Second pass skips the whole game (unchanged sources).
        let r2 = regenerate(&cfg, &jeux, &crate::games::dlc::DlcMap::new(), |_, _, _| {});
        assert_eq!(r2.built, 0);
        assert_eq!(r2.skipped, 1);

        // Changing a STRINGS file (different byte length → new fingerprint)
        // triggers a rebuild.
        write_strings(
            &strings,
            "Skyrim",
            "French",
            &[(1, "Grande épée à deux mains"), (2, "Bouclier")],
        );
        let r3 = regenerate(&cfg, &jeux, &crate::games::dlc::DlcMap::new(), |_, _, _| {});
        assert_eq!(r3.built, 1);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn builds_all_pairs_with_separate_dlc_dicts() {
        let base = std::env::temp_dir().join("ximod_build_allpairs");
        let _ = std::fs::remove_dir_all(&base);
        let strings = base.join("game/Data/Strings");
        std::fs::create_dir_all(&strings).unwrap();
        // Base game in three languages.
        write_strings(
            &strings,
            "Skyrim",
            "English",
            &[(1, "Sword"), (2, "Shield")],
        );
        write_strings(
            &strings,
            "Skyrim",
            "French",
            &[(1, "Épée"), (2, "Bouclier")],
        );
        write_strings(
            &strings,
            "Skyrim",
            "German",
            &[(1, "Schwert"), (2, "Schild")],
        );
        // A DLC (loose STRINGS), English + French only → its own dictionary.
        write_strings(&strings, "Dawnguard", "English", &[(10, "Crossbow")]);
        write_strings(&strings, "Dawnguard", "French", &[(10, "Arbalète")]);

        let cfg = cfg_with_game(&base.join("game"));
        let jeux = base.join("bases/jeux");
        let mut dlc = crate::games::dlc::DlcMap::new();
        dlc.insert("SkyrimSE".into(), vec!["Dawnguard".into()]);
        let r = regenerate(&cfg, &jeux, &dlc, |_, _, _| {});

        // FRA → base + Dawnguard (2 dicts); DEU → base only (1 dict) = 3.
        assert_eq!(r.built, 3);
        let g = jeux.join("SkyrimSE");
        let fra = g.join("SkyrimSE_ENG-FRA.ximdb");
        let deu = g.join("SkyrimSE_ENG-DEU.ximdb");
        let dg_fra = g.join("Dawnguard_ENG-FRA.ximdb");
        assert!(fra.is_file() && deu.is_file() && dg_fra.is_file());
        // No ENG-ENG, and no German Dawnguard (absent language).
        assert!(!g.join("SkyrimSE_ENG-ENG.ximdb").is_file());
        assert!(!g.join("Dawnguard_ENG-DEU.ximdb").is_file());

        // Base FRA dict has the base game text but NOT the DLC text.
        let tf = crate::basedb::ximdb_to_tm(&fra).unwrap();
        assert!(
            tf.entries
                .iter()
                .any(|e| e.source == "Sword" && e.trans == "Épée")
        );
        assert!(!tf.entries.iter().any(|e| e.source == "Crossbow"));
        // The DLC dict carries the DLC text.
        let td = crate::basedb::ximdb_to_tm(&dg_fra).unwrap();
        assert!(
            td.entries
                .iter()
                .any(|e| e.source == "Crossbow" && e.trans == "Arbalète")
        );
        // German base dict maps Sword → Schwert.
        let tde = crate::basedb::ximdb_to_tm(&deu).unwrap();
        assert!(
            tde.entries
                .iter()
                .any(|e| e.source == "Sword" && e.trans == "Schwert")
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn single_language_is_skipped() {
        let base = std::env::temp_dir().join("ximod_build_single");
        let _ = std::fs::remove_dir_all(&base);
        let strings = base.join("game/Data/Strings");
        std::fs::create_dir_all(&strings).unwrap();
        // Only the source language present → no source→target dictionary.
        write_strings(&strings, "Skyrim", "English", &[(1, "Sword")]);
        let cfg = cfg_with_game(&base.join("game"));
        let jeux = base.join("bases/jeux");
        let r = regenerate(&cfg, &jeux, &crate::games::dlc::DlcMap::new(), |_, _, _| {});
        assert_eq!(r.built, 0);
        assert!(
            !jeux
                .join("SkyrimSE")
                .join("SkyrimSE_ENG-FRA.ximdb")
                .is_file()
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
