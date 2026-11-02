//! B-18 recette on the mods of lot 4 (A.8), run by hand:
//! `XIMOD_RECETTE_B18=<recette_lot4> XIMOD_RECETTE_LOT3=<recette_lot3>
//! cargo test --release --test recette_b18 -- --ignored --nocapture`.
//! Each mod is compared with its human translation: a line the shipped
//! rules lock that the translator translated is a false positive.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ximod_translator::exclusions::{Compiled, Ctx, Deduced, LineKind, RulesFile, line_kind};
use ximod_translator::games::Game;
use ximod_translator::textfmt::{Formats, TextDoc};
use ximod_translator::tm::TmEntry;

type Key = (String, u32, String, i32);

fn key(e: &TmEntry) -> Key {
    (e.rec_sig.clone(), e.formid, e.field_sig.clone(), e.index)
}

fn plugin(p: &Path, game: Game) -> Vec<TmEntry> {
    ximod_translator::plugin::read_file_as(p, Some(game))
        .unwrap()
        .entries
}

/// The lines the rules lock, and among them those the translator
/// translated (`translated` says whether he did, `None` when unknown).
fn measure(
    name: &str,
    entries: &[TmEntry],
    game: Game,
    path: &str,
    translated: impl Fn(usize, &TmEntry) -> Option<bool>,
) -> (usize, usize) {
    let f = RulesFile::embedded();
    let c = Compiled::new(&f, &[], game.id());
    let d = Deduced::default();
    let off = HashSet::new();
    let ctx = Ctx {
        game: game.id(),
        path,
        deduced: &d,
        off: &off,
        facts: None,
    };
    let t = std::time::Instant::now();
    let mut per: HashMap<String, (usize, usize, Vec<String>)> = HashMap::new();
    let (mut locked, mut fp, mut script) = (0, 0, 0);
    for (i, e) in entries.iter().enumerate() {
        if e.unused {
            continue;
        }
        let Some(id) = c.lock_of(e, &ctx) else {
            continue;
        };
        if line_kind(e) == LineKind::Script {
            script += 1;
            continue;
        }
        locked += 1;
        let slot = per.entry(id).or_default();
        slot.0 += 1;
        if translated(i, e) == Some(true) {
            fp += 1;
            slot.1 += 1;
            if slot.2.len() < 40 {
                slot.2.push(e.source.clone());
            }
        }
    }
    let elapsed = t.elapsed();
    eprintln!(
        "{name}: {} lines, {locked} texts locked, {fp} translated by the translator; {script} script strings locked ({elapsed:?})",
        entries.iter().filter(|e| !e.unused).count()
    );
    let mut ids: Vec<_> = per.into_iter().collect();
    ids.sort();
    for (id, (n, f, ex)) in ids {
        eprintln!("  {id}: {n} locked, {f} translated {ex:?}");
    }
    (locked, fp)
}

fn root() -> Option<PathBuf> {
    std::env::var("XIMOD_RECETTE_B18").ok().map(PathBuf::from)
}

/// The pairs of a DSD translation: original → translation.
fn dsd(p: &Path) -> HashMap<String, String> {
    let tm = ximod_translator::tm::load(p).unwrap();
    tm.entries
        .into_iter()
        .map(|e| (e.source, e.trans))
        .collect()
}

#[test]
#[ignore]
fn fallout4_mods() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B18 not set: skipped");
        return;
    };
    let pairs = |en: &Path, fr: &Path| {
        let en = plugin(en, Game::Fallout4);
        let fr: HashMap<Key, String> = plugin(fr, Game::Fallout4)
            .into_iter()
            .map(|e| (key(&e), e.source))
            .collect();
        let tr: Vec<Option<bool>> = en
            .iter()
            .map(|e| fr.get(&key(e)).map(|t| t.trim() != e.source.trim()))
            .collect();
        (en, tr)
    };
    let ss2 = r.join("Lot 4-4/Sim Settlements 2");
    let fr = std::fs::read_dir(&ss2)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && p.to_string_lossy().contains("Nate"))
        .unwrap()
        .join("SS2.esm");
    let (en, tr) = pairs(&ss2.join("SS2.esm"), &fr);
    let (n, fp) = measure("SS2.esm", &en, Game::Fallout4, "SS2.esm", |i, _| tr[i]);
    assert!(n <= 1900 && fp <= 25, "{n} {fp}");
    // Cheat Terminal: the English original is in the material of lot 3.
    if let Ok(l3) = std::env::var("XIMOD_RECETTE_LOT3") {
        let en = PathBuf::from(l3).join("Lot 3-5/CheatTerminal_FO4_1.78/Data/CheatTerminal.esp");
        let fr = r.join(
            "Lot 4-4/Cheat Terminal - version francaise 75437 1.78 2026-10-15T08-30Z Hbu637Oq2/CheatTerminal.esp",
        );
        let (en, tr) = pairs(&en, &fr);
        let (n, fp) = measure(
            "CheatTerminal.esp",
            &en,
            Game::Fallout4,
            "CheatTerminal.esp",
            |i, _| tr[i],
        );
        assert!(n <= 70 && fp <= 3, "{n} {fp}");
    }
    let sms = r.join("Lot 4-4/Settlement Management Software");
    let (en, tr) = pairs(
        &sms.join("Settlement Management Software (loose files)-12032-3/SettlementManagementSoftware-3/mso_sms.esp"),
        &sms.join("Settlement Management Software - French Translation 3.03-13324-3/mso_sms.esp"),
    );
    let (_, fp) = measure("mso_sms.esp", &en, Game::Fallout4, "mso_sms.esp", |i, _| {
        tr[i]
    });
    assert_eq!(fp, 0);
    // The MCM file of SS2 (T-B18-14).
    let fmts = Formats::builtin();
    let fmt = fmts.get("json").unwrap();
    let cfg = ss2.join("MCM/Config/SS2/config.json");
    let doc = TextDoc::read(&cfg, fmt).unwrap();
    let tm = doc.to_tm(fmt, "config.json", "Fallout4");
    let cfr = fr.with_file_name("MCM/Config/SS2/config.json");
    let frd = TextDoc::read(&cfr, fmt).ok();
    let frv: Vec<String> = frd
        .map(|d| {
            d.pairs
                .iter()
                .filter_map(|&i| d.lines[i].pair.as_ref().map(|p| p.value.clone()))
                .collect()
        })
        .unwrap_or_default();
    let (n, fp) = measure(
        "MCM config.json",
        &tm.entries,
        Game::Fallout4,
        &cfg.to_string_lossy(),
        |i, e| frv.get(i).map(|t| t != &e.source),
    );
    assert!(fp == 0 && n >= 1000, "{n} {fp}");
}

#[test]
#[ignore]
fn skyrim_mods() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B18 not set: skipped");
        return;
    };
    let caco = r.join("Lot 4-2/Complete Alchemy and Cooking Overhaul");
    let en = plugin(
        &caco.join("complete alchemy & cooking overhaul.esp"),
        Game::SkyrimSE,
    );
    let d = dsd(&caco.join("Complete Alchemy and Cooking Overhaul (French) 149068 3.0FRDSD 2026-07-13T12-55Z bqv3tSkvi/Data/SKSE/Plugins/DynamicStringDistributor/complete alchemy & cooking overhaul.esp/complete alchemy & cooking overhaul.json"));
    let (_, fp) = measure("CACO", &en, Game::SkyrimSE, "caco.esp", |_, e| {
        d.get(&e.source).map(|t| t.trim() != e.source.trim())
    });
    assert_eq!(fp, 0);
    let rfm = r.join("Lot 4-5/Reforging - To the Masses");
    let en = plugin(&rfm.join("RFM.esm"), Game::SkyrimSE);
    let d = dsd(&rfm.join("Reforging - To the Masses (Weapons Expansion) (FR)-141539-2-1FRDSD-1739297944/Data/SKSE/Plugins/DynamicStringDistributor/RFM.esm/RFM.json"));
    let (n, _) = measure("RFM.esm", &en, Game::SkyrimSE, "RFM.esm", |_, e| {
        d.get(&e.source).map(|t| t.trim() != e.source.trim())
    });
    assert_eq!(n, 0);
    // Skipper 0.5.3, the JSON files of the steps (T-B18-11).
    let fmts = Formats::builtin();
    let fmt = fmts.get("json").unwrap();
    let dir = r.join("Lot 4-3/Skipper 112699 0.5.3 2026-08-15T18-57Z tSBIcEWhC/skse/plugins/StorageUtilData/skipper/skips");
    let mut all = Vec::new();
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        let doc = TextDoc::read(&f.path(), fmt).unwrap();
        let tm = doc.to_tm(fmt, "x.json", "SkyrimSE");
        let (n, _) = measure(
            &f.file_name().to_string_lossy(),
            &tm.entries,
            Game::SkyrimSE,
            &f.path().to_string_lossy(),
            |_, _| None,
        );
        all.push((n, tm.entries.len()));
    }
    let (n, t): (usize, usize) = all.iter().fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    eprintln!("Skipper JSON: {n} of {t} locked");
}

#[test]
#[ignore]
fn caco_missed() {
    let Some(r) = root() else { return };
    let caco = r.join("Lot 4-2/Complete Alchemy and Cooking Overhaul");
    let en = plugin(
        &caco.join("complete alchemy & cooking overhaul.esp"),
        Game::SkyrimSE,
    );
    let d = dsd(&caco.join("Complete Alchemy and Cooking Overhaul (French) 149068 3.0FRDSD 2026-07-13T12-55Z bqv3tSkvi/Data/SKSE/Plugins/DynamicStringDistributor/complete alchemy & cooking overhaul.esp/complete alchemy & cooking overhaul.json"));
    let f = RulesFile::embedded();
    let c = Compiled::new(&f, &[], "SkyrimSE");
    let dd = Deduced::default();
    let off = HashSet::new();
    let ctx = Ctx {
        game: "SkyrimSE",
        path: "c.esp",
        deduced: &dd,
        off: &off,
        facts: None,
    };
    let mut seen = HashSet::new();
    for e in &en {
        if e.unused
            || line_kind(e) == LineKind::Script
            || e.source.contains(' ')
            || e.field_sig != "FULL"
        {
            continue;
        }
        let tr = d.get(&e.source).map(|t| t.trim() != e.source.trim());
        if tr != Some(true) && c.lock_of(e, &ctx).is_none() && seen.insert(e.source.clone()) {
            eprintln!("missed: {} {}:{}", e.source, e.rec_sig, e.field_sig);
        }
    }
}
