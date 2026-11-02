//! Recette de l'étape 2 (A.4.5 du cadre) : les fichiers de travail de
//! l'étape 1 s'ouvrent sans perte. Un dictionnaire du mod de format 2
//! (lignes de sept éléments : la ligne et son statut, en-tête réduit à
//! `v` et `lang`) et un pivot sans les champs ajoutés à l'étape 2
//! (provenance, relecture, historique, règles, VMAD) sont écrits comme
//! l'étape 1 les écrivait, puis relus : mêmes lignes, mêmes traductions,
//! mêmes statuts ; le dictionnaire réécrit passe au format 4 sans rien
//! perdre.
//!
//! Le test sur Apocalypse (2 940 lignes) se joue à la main :
//! `XIMOD_RECETTE_D06=<recette_lot4> cargo test --release --test
//! compat_e1 -- --ignored --nocapture`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use ximod_translator::basedb::{self, BaseRow};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::{self, TmEntry, TranslationMemory};

const CLI: &str = env!("CARGO_BIN_EXE_ximod-cli");

/// A dictionary of format 4 rewritten as the first stage wrote it.
fn to_format2(v4: &Path, out: &Path) {
    let v: Value = serde_json::from_slice(&std::fs::read(v4).unwrap()).unwrap();
    let rows: Vec<Value> = v["e"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| Value::Array(r.as_array().unwrap().iter().take(7).cloned().collect()))
        .collect();
    // `v` and `lang` first, as the writer of the first stage put them.
    let f = format!(
        "{{\"v\":2,\"lang\":{},\"e\":{}}}",
        v["lang"],
        serde_json::to_string(&rows).unwrap()
    );
    std::fs::write(out, f).unwrap();
}

/// A pivot as the first stage wrote it: without the fields of stage 2.
fn to_stage1_pivot(tm: &TranslationMemory, out: &Path) {
    let mut v = serde_json::to_value(tm).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("history");
    for k in [
        "rules",
        "rules_off",
        "vmad",
        "enc_in",
        "enc_out",
        "text_format",
    ] {
        o["meta"].as_object_mut().unwrap().remove(k);
    }
    for e in o["entries"].as_array_mut().unwrap() {
        let e = e.as_object_mut().unwrap();
        e.remove("prov");
        e.remove("review");
    }
    std::fs::write(out, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

type Key = (String, String, u32, String, i32);

fn key(e: &TmEntry) -> Key {
    (
        e.rec_sig.clone(),
        e.field_sig.clone(),
        e.formid,
        e.edid.clone(),
        e.index,
    )
}

fn by_key(tm: &TranslationMemory) -> HashMap<Key, (String, Status)> {
    tm.entries
        .iter()
        .map(|e| (key(e), (e.trans.clone(), status::derive(e))))
        .collect()
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ximod-compat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Write a mod's dictionary of format 4 for `plugin` (translations
/// « FR: … », statuses mixed), in `mods/<game>/`.
fn v4_dict(plugin: &Path, out: &Path) -> TranslationMemory {
    let mut tm = ximod_translator::plugin::read_file(plugin).unwrap();
    for (i, e) in tm.entries.iter_mut().enumerate() {
        if e.unused || e.source.trim().is_empty() {
            continue;
        }
        e.trans = format!("FR: {}", e.source);
        status::set(
            e,
            [Status::Validated, Status::Pending, Status::Custom][i % 3],
        );
        e.prov = "manual".into();
    }
    let n = ximod_translator::mod_dict::write(&ximod_translator::mod_dict::Write {
        entries: &tm.entries,
        rules: Vec::new(),
        rules_off: Vec::new(),
        enc_in: None,
        enc_out: None,
        lang: "fra",
        out,
        ident: None,
        backup_copies: 0,
        old_months: 0,
    })
    .unwrap();
    assert!(n > 0);
    tm
}

fn check_dict(plugin: &Path, cfg: &Path, v4: &Path, name: &str) {
    // The command line opens the plugin with each dictionary in its place.
    let run = |dir: &Path| {
        let mods = cfg.join("data/bases/mods");
        let target = mods.join(dir.file_name().unwrap());
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).unwrap();
        for f in std::fs::read_dir(dir).unwrap().flatten() {
            std::fs::copy(f.path(), target.join(f.file_name())).unwrap();
        }
        let o = Command::new(CLI)
            .arg("--config")
            .arg(cfg)
            .args(["--json", "apply", plugin.to_str().unwrap(), "--write-dict"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let j: Value = serde_json::from_slice(&o.stdout).unwrap();
        let rewritten = target.join(name);
        (j, basedb::ximdb_to_tm(&rewritten).unwrap(), rewritten)
    };
    let game = v4.parent().unwrap().file_name().unwrap().to_owned();
    // Format 4 as it is.
    let d4 = cfg.parent().unwrap().join("d4").join(&game);
    std::fs::create_dir_all(&d4).unwrap();
    std::fs::copy(v4, d4.join(name)).unwrap();
    // The same, as the first stage wrote it.
    let d2 = cfg.parent().unwrap().join("d2").join(&game);
    std::fs::create_dir_all(&d2).unwrap();
    to_format2(v4, &d2.join(name));
    let head = String::from_utf8_lossy(&std::fs::read(d2.join(name)).unwrap()[..20]).to_string();
    assert!(head.starts_with("{\"v\":2,"), "{head}");
    let rows2 = basedb::ximdb_to_tm(&d2.join(name)).unwrap();
    let rows4 = basedb::ximdb_to_tm(&d4.join(name)).unwrap();
    assert_eq!(rows2.entries.len(), rows4.entries.len());
    let (j4, re4, _) = run(&d4);
    let (j2, re2, path2) = run(&d2);
    eprintln!(
        "{name}: format 4 — lines {} left {} locked {}; format 2 — lines {} left {} locked {}",
        j4["lines"], j4["left"], j4["locked"], j2["lines"], j2["left"], j2["locked"]
    );
    for k in ["lines", "left", "locked", "filledAtOpening"] {
        assert_eq!(j2[k], j4[k], "{k}");
    }
    // The translations and statuses are those of the first stage's file;
    // rewritten, it is of format 4 with the same rows.
    let (a, b) = (by_key(&re2), by_key(&re4));
    assert_eq!(a.len(), b.len());
    for (k, v) in &a {
        assert_eq!(Some(v), b.get(k), "{k:?}");
    }
    let head = String::from_utf8_lossy(&std::fs::read(&path2).unwrap()[..8]).to_string();
    assert!(head.starts_with("{\"v\":4"), "{head}");
    eprintln!(
        "  rewritten as format 4: {} rows, translations and statuses equal",
        re2.entries.len()
    );
}

fn check_pivot(tm: &TranslationMemory, dir: &Path) {
    let p = dir.join("travail.json");
    to_stage1_pivot(tm, &p);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("\"prov\"") && !text.contains("\"history\""));
    let back = tm::load(&p).unwrap();
    assert_eq!(back.entries.len(), tm.entries.len());
    for (a, b) in back.entries.iter().zip(&tm.entries) {
        assert_eq!(key(a), key(b));
        assert_eq!(a.source, b.source);
        assert_eq!(a.trans, b.trans);
        assert_eq!(status::derive(a), status::derive(b));
        assert_eq!(a.comment, b.comment);
    }
    assert_eq!(back.meta.game, tm.meta.game);
    // Saved again by this version, it reads back the same.
    let p2 = dir.join("travail2.json");
    tm::save(&back, &p2).unwrap();
    let again = tm::load(&p2).unwrap();
    assert_eq!(by_key(&again), by_key(&back));
    eprintln!(
        "pivot of stage 1: {} entries read back, translations and statuses equal",
        back.entries.len()
    );
}

fn config(root: &Path) -> PathBuf {
    let cfg = root.join("cfg");
    std::fs::create_dir_all(cfg.join("data/bases/mods")).unwrap();
    std::fs::write(
        cfg.join("Config.ini"),
        "[General]\nLocale=fra\n[Translation]\nSourceLanguage=eng\nTargetLanguage=fra\n",
    )
    .unwrap();
    cfg
}

#[test]
fn stage1_files_open_without_loss() {
    let root = temp("fixture");
    let cfg = config(&root);
    let plugin = root.join("skyrim_dialogue.esp");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrim_dialogue.esp"),
        &plugin,
    )
    .unwrap();
    let v4dir = root.join("v4").join("Skyrim");
    std::fs::create_dir_all(&v4dir).unwrap();
    let name = "skyrim_dialogue_ENG-FRA.ximdb";
    let tm = v4_dict(&plugin, &v4dir.join(name));
    check_dict(&plugin, &cfg, &v4dir.join(name), name);
    check_pivot(&tm, &root);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore]
fn stage1_files_of_apocalypse() {
    let Ok(r) = std::env::var("XIMOD_RECETTE_D06") else {
        eprintln!("XIMOD_RECETTE_D06 not set: skipped");
        return;
    };
    let src = PathBuf::from(r).join("Lot 4-2/Apocalypse - Magic of Skyrim");
    let root = temp("apocalypse");
    let cfg = config(&root);
    let plugin = root.join("Apocalypse - Magic of Skyrim.esp");
    std::fs::copy(src.join("Apocalypse - Magic of Skyrim.esp"), &plugin).unwrap();
    // The translator's work: the French plugin, line by line, statuses
    // mixed as a work in progress.
    let fr = ximod_translator::plugin::read_file(&src.join(
        "(FR - DSD) Apocalypse - Magic of Skyrim 9.45-4771-9-45-1748024240/08 Clean patch/Apocalypse - Magic of Skyrim.esp",
    ))
    .unwrap();
    let frk: HashMap<Key, String> = fr
        .entries
        .iter()
        .map(|e| (key(e), e.source.clone()))
        .collect();
    let mut tm = ximod_translator::plugin::read_file(&plugin).unwrap();
    for (i, e) in tm.entries.iter_mut().enumerate() {
        if let Some(t) = frk.get(&key(e))
            && *t != e.source
        {
            e.trans = t.clone();
            status::set(
                e,
                if i % 4 == 0 {
                    Status::Pending
                } else {
                    Status::Validated
                },
            );
        }
    }
    let v4dir = root.join("v4").join("SkyrimSE");
    std::fs::create_dir_all(&v4dir).unwrap();
    let name = "Apocalypse - Magic of Skyrim_ENG-FRA.ximdb";
    ximod_translator::mod_dict::write(&ximod_translator::mod_dict::Write {
        entries: &tm.entries,
        rules: Vec::new(),
        rules_off: Vec::new(),
        enc_in: None,
        enc_out: None,
        lang: "fra",
        out: &v4dir.join(name),
        ident: None,
        backup_copies: 0,
        old_months: 0,
    })
    .unwrap();
    check_dict(&plugin, &cfg, &v4dir.join(name), name);
    check_pivot(&tm, &root);
    // The game bases of the first stage (format 2), when they are there.
    let base = std::env::var("XIMOD_E1_BASE").ok().map(PathBuf::from);
    if let Some(b) = base.filter(|b| b.is_file()) {
        let t = std::time::Instant::now();
        let db = basedb::FileDbCache::default().get(&b).unwrap();
        eprintln!(
            "game base of stage 1 {}: {} rows read in {:.1} s",
            b.file_name().unwrap().to_string_lossy(),
            db.rows(),
            t.elapsed().as_secs_f32()
        );
        assert!(db.rows() > 0);
    }
    let _ = BaseRow::default();
}
