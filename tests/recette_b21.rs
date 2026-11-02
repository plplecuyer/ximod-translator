//! B-21 recette on the mods of lot 4 (A.6), run by hand:
//! `XIMOD_RECETTE_B21=<recette_lot4> cargo test --release --test
//! recette_b21 -- --ignored --nocapture`.
//! Apocalypse (scrolls and tomes from the spells and effects translated by
//! the French plugin) and CACO (potions, poisons, draughts from the effects
//! translated by the DSD, the degrees by the glossary), compared with the
//! human translations; then the parents found in the game's base only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::derive::{self, DeriveFile, ParentBases};
use ximod_translator::assist::{
    self, Assistant, DeriveCtx, Glossary, GlossaryFile, NoBases, Options, Patterns, StackBases,
};
use ximod_translator::games::Game;
use ximod_translator::tm::TmEntry;

fn root() -> Option<PathBuf> {
    std::env::var("XIMOD_RECETTE_B21").ok().map(PathBuf::from)
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn plugin(p: &Path) -> Vec<TmEntry> {
    ximod_translator::plugin::read_file_as(p, Some(Game::SkyrimSE))
        .unwrap()
        .entries
}

fn assistant(game_glossary: Option<&Path>) -> Assistant {
    let read = |p: &Path| GlossaryFile::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
    let pair = read(&data("assets/data/assist/glossary_ENG-FRA.json"));
    let game = game_glossary.map(read).unwrap_or_default();
    let p = assist::patterns::shipped_file(&data("assets/data/assist"), "ENG-FRA").unwrap();
    let mut a = Assistant::new(
        Glossary::new(&[(&game, true), (&pair, false)]),
        p.composition.clone(),
    );
    a.patterns = Some(Arc::new(Patterns::new(p)));
    a
}

fn rules() -> DeriveFile {
    let f = derive::shipped_file(&data("assets/data/assist"), "SkyrimSE").unwrap();
    f.validate().unwrap();
    f
}

fn derive_only() -> Options {
    Options {
        glossary: false,
        patterns: false,
        derive: true,
        overwrite: Overwrite::None,
    }
}

/// Apply the derivations, print the counts, the lines that differ from the
/// human translation and return (derived per signature, equal).
fn run(
    name: &str,
    entries: &mut [TmEntry],
    a: &Assistant,
    parent_bases: &dyn ParentBases,
    teaches: &HashMap<u32, u32>,
    human: &dyn Fn(&TmEntry) -> Option<String>,
) -> (BTreeMap<String, usize>, usize) {
    let lines: Vec<usize> = (0..entries.len()).collect();
    let f = rules();
    let ctx = DeriveCtx {
        file: &f,
        lang: "fra",
        open: Vec::new(),
        parent_bases,
        teaches,
    };
    let t = std::time::Instant::now();
    let plan = assist::plan_all(entries, &lines, a, &derive_only(), &NoBases, Some(&ctx));
    let el = t.elapsed();
    let mut per_sig: BTreeMap<String, usize> = BTreeMap::new();
    let mut equal = 0;
    let mut diffs = Vec::new();
    for c in &plan.changes {
        let e = &entries[c.idx];
        *per_sig.entry(e.rec_sig.clone()).or_default() += 1;
        match human(e) {
            Some(h) if h.trim() == c.trans => equal += 1,
            h => diffs.push(format!(
                "{} → {} | {}",
                e.source,
                c.trans,
                h.unwrap_or_default()
            )),
        }
    }
    eprintln!(
        "{name}: {} derived {:?} per rule {:?}, {} untranslated parents, {} not found, {} ambiguous, {equal} equal to the human translation ({el:?})",
        plan.filled(),
        per_sig,
        plan.derived,
        plan.untranslated,
        plan.not_found,
        plan.ambiguous
    );
    for d in diffs.iter().take(60) {
        eprintln!("  {d}");
    }
    assist::commit(entries, &plan);
    (per_sig, equal)
}

type Key = (String, u32, String, i32);

fn key(e: &TmEntry) -> Key {
    (e.rec_sig.clone(), e.formid, e.field_sig.clone(), e.index)
}

#[test]
#[ignore]
fn apocalypse() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B21 not set: skipped");
        return;
    };
    let dir = r.join("Lot 4-2/Apocalypse - Magic of Skyrim");
    let en_path = dir.join("Apocalypse - Magic of Skyrim.esp");
    let mut en = plugin(&en_path);
    let fr: HashMap<Key, String> = plugin(&dir.join(
        "(FR - DSD) Apocalypse - Magic of Skyrim 9.45-4771-9-45-1748024240/08 Clean patch/Apocalypse - Magic of Skyrim.esp",
    ))
    .into_iter()
    .map(|e| (key(&e), e.source))
    .collect();
    // The spells and effects translated (the French plugin as the mod's
    // dictionary), the scrolls and tomes empty.
    for e in en.iter_mut() {
        if matches!(e.rec_sig.as_str(), "SPEL" | "MGEF")
            && let Some(t) = fr.get(&key(e))
        {
            e.trans = t.clone();
        }
    }
    let scrolls = en
        .iter()
        .filter(|e| e.rec_sig == "SCRL" && e.field_sig == "FULL")
        .count();
    let books = en
        .iter()
        .filter(|e| e.rec_sig == "BOOK" && e.field_sig == "FULL")
        .count();
    eprintln!("Apocalypse: {scrolls} SCRL, {books} BOOK");
    let teaches = derive::teaches(&en_path);
    eprintln!("  {} tomes teach a spell", teaches.len());
    let a = assistant(None);
    let human = |e: &TmEntry| fr.get(&key(e)).cloned();
    let (per, equal) = run("Apocalypse", &mut en, &a, &NoBases, &teaches, &human);
    let n: usize = per.values().sum();
    assert!(n >= 320, "{n}");
    assert!(equal >= 315, "{equal}");
    // Every derived line is En attente with its provenance.
    for e in en.iter().filter(|e| e.prov.starts_with("rule:derive:")) {
        assert_eq!(
            ximod_translator::status::derive(e),
            ximod_translator::status::Status::Pending
        );
    }
}

#[test]
#[ignore]
fn caco() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B21 not set: skipped");
        return;
    };
    let dir = r.join("Lot 4-2/Complete Alchemy and Cooking Overhaul");
    let mut en = plugin(&dir.join("complete alchemy & cooking overhaul.esp"));
    let dsd_path = dir.join("Complete Alchemy and Cooking Overhaul (French) 149068 3.0FRDSD 2026-07-13T12-55Z bqv3tSkvi/Data/SKSE/Plugins/DynamicStringDistributor/complete alchemy & cooking overhaul.esp/complete alchemy & cooking overhaul.json");
    let dsd = ximod_translator::tm::load(&dsd_path).unwrap();
    let by: HashMap<(String, String), String> = dsd
        .entries
        .iter()
        .map(|e| {
            (
                (format!("{}:{}", e.rec_sig, e.field_sig), e.source.clone()),
                e.trans.clone(),
            )
        })
        .collect();
    let by_src: HashMap<String, String> = dsd
        .entries
        .iter()
        .map(|e| (e.source.clone(), e.trans.clone()))
        .collect();
    for e in en.iter_mut() {
        if e.rec_sig == "MGEF"
            && let Some(t) = by
                .get(&(format!("{}:{}", e.rec_sig, e.field_sig), e.source.clone()))
                .or_else(|| by_src.get(&e.source))
        {
            e.trans = t.clone();
        }
    }
    let derivable: HashSet<usize> = en
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.rec_sig == "ALCH"
                && e.field_sig == "FULL"
                && [
                    " Potion of ",
                    " Poison of ",
                    " Draught of ",
                    " Elixir of ",
                    " Wax of ",
                ]
                .iter()
                .any(|w| format!(" {}", e.source).contains(w))
        })
        .map(|(i, _)| i)
        .collect();
    eprintln!("CACO: {} ALCH « … of … »", derivable.len());
    let a = assistant(Some(&data("tests/data/assist/caco_SkyrimSE_ENG-FRA.json")));
    let human = |e: &TmEntry| by_src.get(&e.source).cloned();
    let teaches = HashMap::new();
    let (per, _) = run(
        "CACO without the base",
        &mut en.clone(),
        &a,
        &NoBases,
        &teaches,
        &human,
    );
    let without = per.get("ALCH").copied().unwrap_or(0);
    let base = r.join("Bases/SkyrimSE_ENG-FRA.ximdb");
    let db = ximod_translator::basedb::BaseDb::from_file(&base).unwrap();
    let sb = StackBases(vec![Arc::new(db)]);
    eprintln!(
        "  base: Healing → {:?}, Regeneration → {:?}",
        sb.lookup("MGEF", "FULL", "Healing"),
        sb.lookup("MGEF", "FULL", "Regeneration")
    );
    let (per, _) = run("CACO with the base", &mut en, &a, &sb, &teaches, &human);
    let with = per.get("ALCH").copied().unwrap_or(0);
    eprintln!("  {without} without the base, {with} with it");
    assert!(with >= 300, "{with}");
    assert!(with > without);
}
