//! C-06 on the recette mods (not in the repository, skipped when absent):
//! the scripts of a mod found loose and in its archives, every script read
//! and written back identical, a translated literal written and read
//! again, the names never lines, the locking rules, the versions not read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ximod_translator::games::Game;
use ximod_translator::pex;
use ximod_translator::script_rules::Rules;
use ximod_translator::scripts::{self, ScriptSet};
use ximod_translator::status::{self, Status};

fn recette(rel: &str) -> Option<PathBuf> {
    let p = Path::new("/root/ximod_work/recette_lot3").join(rel);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("skipped: {} absent", p.display());
        None
    }
}

const SKIPPER: &str = "Lot 3-1/x_Skipper_112699_0.5.3_2026-08-15T18-57Z_t/skipper.esp";

/// T-C06-01/03: Skipper's two loose scripts; a literal of MCM_skipper
/// translated is written, read again translated; the names are not lines;
/// nothing translated, nothing written.
#[test]
fn skipper_scripts_translated_and_written() {
    let Some(plugin) = recette(SKIPPER) else {
        return;
    };
    let set = ScriptSet::discover(&plugin, Game::SkyrimSE);
    let names: Vec<&str> = set.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"MCM_skipper"), "{names:?}");
    let mut tm = set.to_tm(Game::SkyrimSE, "skipper.esp");
    assert!(tm.entries.len() > 100);
    let mcm = set.script("MCM_skipper").unwrap();
    let p = mcm.pex.as_ref().unwrap();
    // A name (a function of the script) is never a line.
    let names_as_lines = tm
        .entries
        .iter()
        .filter(|e| {
            let i = e.index as usize;
            e.edid == "MCM_skipper" && p.ident[i] && !p.is_literal(i)
        })
        .count();
    assert_eq!(names_as_lines, 0);
    assert!(set.translated(&tm, Game::SkyrimSE).unwrap().is_empty());
    // Translate the first literal that is a sentence.
    let i = tm
        .entries
        .iter()
        .position(|e| {
            e.edid == "MCM_skipper" && e.source.contains(' ') && !p.ident[e.index as usize]
        })
        .unwrap();
    let source = tm.entries[i].source.clone();
    tm.entries[i].trans = format!("{source} (FR é)");
    status::set(&mut tm.entries[i], Status::Validated);
    let out = set.translated(&tm, Game::SkyrimSE).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "Scripts/MCM_skipper.pex");
    let q = pex::parse(&out[0].1).unwrap();
    let idx = tm.entries[i].index as usize;
    assert_eq!(
        String::from_utf8_lossy(&q.strings[idx]),
        format!("{source} (FR é)")
    );
    assert_eq!(q.strings.len(), p.strings.len());
    // The translation removed: identical, not written.
    tm.entries[i].trans.clear();
    status::set(&mut tm.entries[i], Status::ToTranslate);
    assert!(set.translated(&tm, Game::SkyrimSE).unwrap().is_empty());
}

/// Every recette script: read, written back identical without change, a
/// translation of every literal read again; the archives listed (T-C06-04,
/// T-C06-07).
#[test]
fn every_recette_script_round_trips() {
    let cases = [
        (SKIPPER, Game::SkyrimSE),
        (
            "Lot 3-1/x_MENU_MCM_VANILLA-178821-0-0-1-1777500611/Menu MCM Vanilla.esp",
            Game::SkyrimSE,
        ),
        (
            "Lot 3-2/x_CMRQL_2.41_Installer-39575-2-41-16438757/CMRQL_2_41_Installer/Files/CMRQL.esp",
            Game::Fallout4,
        ),
        (
            "Lot 3-5/CheatTerminal_FO4_1.78/Data/CheatTerminal.esp",
            Game::Fallout4,
        ),
        (
            "Lot 3-5/CheatTerminal_Starfield_1.08/CheatTerminal.esm",
            Game::Starfield,
        ),
        (
            "Lot 3-6/QuickStart_SE_1.2.2/Quick Start - SE.esp",
            Game::SkyrimSE,
        ),
    ];
    for (rel, game) in cases {
        let Some(plugin) = recette(rel) else {
            continue;
        };
        let t = std::time::Instant::now();
        let set = ScriptSet::discover(&plugin, game);
        let took = t.elapsed();
        assert!(!set.scripts.is_empty(), "{rel}");
        let mut both = 0;
        for s in &set.scripts {
            let p = s
                .pex
                .as_ref()
                .unwrap_or_else(|e| panic!("{rel} {}: {e}", s.rel));
            assert_eq!(pex::write(&s.bytes, p, &HashMap::new()).unwrap(), s.bytes);
            let t: HashMap<usize, Vec<u8>> = p.literals().map(|i| (i, b"T".to_vec())).collect();
            let q = pex::parse(&pex::write(&s.bytes, p, &t).unwrap()).unwrap();
            assert!(q.literals().all(|i| q.strings[i] == b"T"), "{}", s.rel);
            // The names are intact.
            for i in 0..p.strings.len() {
                if p.ident[i] {
                    assert_eq!(q.strings[i], p.strings[i]);
                }
            }
            both += p.literals().filter(|i| p.ident[*i]).count();
        }
        let sources = set.scripts.iter().filter(|s| s.source.is_some()).count();
        eprintln!(
            "{rel}: {} scripts ({} in archives, {sources} sources, {both} shared), {took:?}",
            set.scripts.len(),
            set.scripts
                .iter()
                .filter(|s| s.origin.archive_name().is_some())
                .count()
        );
    }
}

/// T-C06-02: a string that is a name and a literal is locked « identifiant
/// du script »; unlocked and translated, the writing adds a string at the
/// end and the name stays.
#[test]
fn a_shared_string_is_locked_then_appended() {
    let Some(plugin) = recette(SKIPPER) else {
        return;
    };
    let set = ScriptSet::discover(&plugin, Game::SkyrimSE);
    let mut tm = set.to_tm(Game::SkyrimSE, "skipper.esp");
    let facts = set.facts();
    scripts::apply(&mut tm.entries, &facts, &Rules::builtin());
    let Some(i) = tm.entries.iter().position(|e| e.prov == "lock:ident") else {
        eprintln!("no shared string in Skipper");
        return;
    };
    assert_eq!(status::derive(&tm.entries[i]), Status::Ignored);
    let (name, idx) = (tm.entries[i].edid.clone(), tm.entries[i].index as usize);
    tm.entries[i].trans = "Traduit".into();
    status::set(&mut tm.entries[i], Status::Validated);
    let out = set.translated(&tm, Game::SkyrimSE).unwrap();
    let p = set.script(&name).unwrap().pex.as_ref().unwrap();
    let q = pex::parse(&out[0].1).unwrap();
    assert_eq!(q.strings[idx], p.strings[idx]);
    assert_eq!(q.strings.len(), p.strings.len() + 1);
    assert_eq!(q.strings.last().unwrap(), b"Traduit");
}

/// T-C06-05: a rule removed from the user's copy applies no more; since
/// B-18 the copy is the user's exclusions file, and a former
/// papyrus_rules.json is merged into it once (D-73).
#[test]
fn the_users_copy_replaces_the_rules() {
    use ximod_translator::exclusions::{Dirs, RulesFile};
    let tmp = std::env::temp_dir().join(format!("ximod-c06-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dirs = Dirs {
        shipped: Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/rules"),
        user: Some(tmp.join("rules")),
    };
    let r = Rules::from_exclusions(&dirs.load().0);
    assert_eq!(r.lock_of("Default"), Some("word"));
    let mut f: RulesFile = dirs.load().0;
    f.rules.iter_mut().find(|r| r.id == "word").unwrap().enabled = false;
    dirs.save(&f).unwrap();
    let r = Rules::from_exclusions(&dirs.load().0);
    assert_eq!(r.lock_of("Default"), None);
    assert_eq!(r.lock_of("$X"), Some("key"));
    // The former file: merged, renamed .bak.
    dirs.restore().unwrap();
    std::fs::create_dir_all(tmp.join("scripts")).unwrap();
    std::fs::write(
        tmp.join("scripts/papyrus_rules.json"),
        r#"{ "format": 1, "lockFunctions": ["MyLib.Log"], "lockPatterns": [ { "id": "mine", "regex": "^zz" } ] }"#,
    )
    .unwrap();
    assert_eq!(dirs.migrate_papyrus(), Some(2));
    assert!(tmp.join("scripts/papyrus_rules.json.bak").is_file());
    assert!(!tmp.join("scripts/papyrus_rules.json").exists());
    let r = Rules::from_exclusions(&dirs.load().0);
    assert_eq!(r.lock_of("zz test"), Some("mine"));
    assert!(r.locks_function("MyLib.Log"));
    assert!(r.locks_function("Debug.Trace"));
    assert_eq!(dirs.migrate_papyrus(), None);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// T-C06-06: a forged version 9.9 is not read; a truncated file is
/// unreadable; neither gives a line.
#[test]
fn unknown_version_and_truncated_files() {
    let Some(plugin) = recette(SKIPPER) else {
        return;
    };
    let src = plugin
        .with_file_name("scripts")
        .join("skipperpausescript.pex");
    let bytes = std::fs::read(&src).unwrap();
    let dir = std::env::temp_dir().join(format!("ximod-c06v-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut forged = bytes.clone();
    forged[4] = 9;
    forged[5] = 9;
    std::fs::write(dir.join("forged.pex"), &forged).unwrap();
    std::fs::write(dir.join("cut.pex"), &bytes[..bytes.len() / 2]).unwrap();
    let a = ScriptSet::single(&dir.join("forged.pex")).unwrap();
    assert_eq!(
        a.scripts[0].pex.as_ref().err(),
        Some(&pex::PexError::Version(9, 9))
    );
    assert!(a.to_tm(Game::SkyrimSE, "").entries.is_empty());
    let b = ScriptSet::single(&dir.join("cut.pex")).unwrap();
    assert_eq!(
        b.scripts[0].pex.as_ref().err(),
        Some(&pex::PexError::Truncated)
    );
    assert!(
        b.translated(&b.to_tm(Game::SkyrimSE, ""), Game::SkyrimSE)
            .unwrap()
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-C07-05: the translation copied into the .psc source on option: the
/// literal replaced between its quotes, the source at its own place.
#[test]
fn the_source_takes_the_translation() {
    let Some(plugin) =
        recette("Lot 3-1/x_MENU_MCM_VANILLA-178821-0-0-1-1777500611/Menu MCM Vanilla.esp")
    else {
        return;
    };
    let set = ScriptSet::discover(&plugin, Game::SkyrimSE);
    let mut tm = set.to_tm(Game::SkyrimSE, "Menu MCM Vanilla.esp");
    assert!(set.translated_sources(&tm, Game::SkyrimSE).is_empty());
    let i = tm
        .entries
        .iter()
        .position(|e| e.source.starts_with("Description for the example"))
        .unwrap();
    tm.entries[i].trans = "Description du \"bouton\" d'exemple.".into();
    status::set(&mut tm.entries[i], Status::Validated);
    let out = set.translated_sources(&tm, Game::SkyrimSE);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "scripts/source/MenuMCMVanilla_ConfigMenu.psc");
    let text = String::from_utf8(out[0].1.clone()).unwrap();
    assert!(text.contains("SetInfoText(\"Description du \\\"bouton\\\" d'exemple.\")"));
    assert!(!text.contains("Description for the example button"));
}
