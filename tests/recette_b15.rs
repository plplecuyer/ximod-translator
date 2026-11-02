//! B-15 recette on the mods of lot 4 (A.5), run by hand:
//! `XIMOD_RECETTE_B15=<recette_lot4> cargo test --release --test
//! recette_b15 -- --ignored --nocapture`.
//! Immersive Armors (865 ARMO:FULL) with a small glossary (12 materials
//! and the equipment types) and a glossary of at most 100 terms, compared
//! with the French DSD translation; Immersive Weapon Integration with the
//! Chinese glossary, compared with its CHS translation.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::{self, Assistant, Glossary, GlossaryFile, Options, Refusal};
use ximod_translator::games::Game;
use ximod_translator::tm::TmEntry;

fn root() -> Option<PathBuf> {
    std::env::var("XIMOD_RECETTE_B15").ok().map(PathBuf::from)
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn plugin(p: &Path) -> Vec<TmEntry> {
    ximod_translator::plugin::read_file_as(p, Some(Game::SkyrimSE))
        .unwrap()
        .entries
}

fn assistant(glossary: &Path, pair: &str) -> Assistant {
    let f = GlossaryFile::parse(&std::fs::read_to_string(glossary).unwrap()).unwrap();
    f.validate().unwrap();
    eprintln!("{}: {} terms", glossary.display(), f.term_count());
    let comp = assist::compose::load(&data("assets/data/assist"), None, pair);
    Assistant::new(Glossary::new(&[(&f, false)]), comp)
}

/// Fill the lines, print the coverage, the words missed and a sample.
fn run(
    name: &str,
    entries: &mut [TmEntry],
    a: &Assistant,
    human: &HashMap<String, String>,
) -> usize {
    let lines: Vec<usize> = (0..entries.len()).collect();
    let t = std::time::Instant::now();
    let plan = assist::plan(
        entries,
        &lines,
        a,
        &Options {
            glossary: true,
            patterns: true,
            derive: false,
            overwrite: Overwrite::None,
        },
        &assist::NoBases,
    );
    let el = t.elapsed();
    let mut missed: BTreeMap<String, usize> = BTreeMap::new();
    for e in entries.iter() {
        if let Err(Refusal::Partial(w)) = a.translate(&e.source, true) {
            for w in w {
                *missed.entry(w).or_default() += 1;
            }
        }
    }
    assist::commit(entries, &plan);
    let same = plan
        .changes
        .iter()
        .filter(|c| {
            human
                .get(&entries[c.idx].source)
                .is_some_and(|h| h.trim() == c.trans)
        })
        .count();
    eprintln!(
        "{name}: {} lines, {} filled ({} composed, {} glossary), {} partial, {same} equal to the human translation ({el:?})",
        entries.len(),
        plan.filled(),
        plan.composed,
        plan.glossary,
        plan.partial
    );
    let mut m: Vec<_> = missed.into_iter().collect();
    m.sort_by_key(|x| std::cmp::Reverse(x.1));
    eprintln!("  missed: {:?}", &m[..m.len().min(40)]);
    let mut seen = std::collections::HashSet::new();
    for c in plan.changes.iter() {
        let e = &entries[c.idx];
        if seen.insert(e.source.clone()) && seen.len() <= 400 {
            eprintln!(
                "  {} → {} | {}",
                e.source,
                c.trans,
                human.get(&e.source).map(String::as_str).unwrap_or("")
            );
        }
    }
    plan.filled()
}

fn dsd(p: &Path) -> HashMap<String, String> {
    ximod_translator::tm::load(p)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| (e.source, e.trans))
        .collect()
}

#[test]
#[ignore]
fn immersive_armors() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B15 not set: skipped");
        return;
    };
    let ia = r.join("Lot 4-1/Immersive Armors");
    let all = plugin(&ia.join("Hothtrooper44_ArmorCompilation.esp"));
    let armo: Vec<TmEntry> = all
        .into_iter()
        .filter(|e| e.rec_sig == "ARMO" && e.field_sig == "FULL" && !e.unused)
        .collect();
    assert_eq!(armo.len(), 865);
    let human = dsd(&ia.join("Immersive Armors - French-93078-8-1FRDSD-1743524460/Data/SKSE/Plugins/DynamicStringDistributor/Hothtrooper44_ArmorCompilation.esp/Hothtrooper44_ArmorCompilation.json"));
    let small = assistant(&data("tests/data/assist/ia_small_ENG-FRA.json"), "ENG-FRA");
    let n = run(
        "IA, 12 materials and the types",
        &mut armo.clone(),
        &small,
        &human,
    );
    assert!(n >= 300, "{n}");
    let full = assistant(&data("tests/data/assist/ia_100_ENG-FRA.json"), "ENG-FRA");
    assert!(full.glossary.len() <= 100);
    let mut lines = armo.clone();
    let n = run("IA, 100 terms", &mut lines, &full, &human);
    assert!(n >= 700, "{n}");
    // Every line filled is En attente, provenance « règle : … »; no line
    // partially covered is filled.
    for e in &lines {
        if !e.trans.is_empty() {
            assert!(e.prov.starts_with("rule:"));
            assert!(full.translate(&e.source, true).is_ok());
        }
    }
    let shipped = assistant(&data("assets/data/assist/glossary_ENG-FRA.json"), "ENG-FRA");
    run("IA, shipped glossary", &mut armo.clone(), &shipped, &human);
}

#[test]
#[ignore]
fn immersive_weapon_integration_zho() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B15 not set: skipped");
        return;
    };
    let iwi = r.join("Lot 4-1/Immersive Weapon Integration");
    let en = plugin(&iwi.join("ImmersiveIntegrationSE.esp"));
    let zh = plugin(&iwi.join(
        "Immersive Weapon Integration - CHS-159151-0-96-1757830289/ImmersiveIntegrationSE.esp",
    ));
    let key = |e: &TmEntry| (e.rec_sig.clone(), e.formid, e.field_sig.clone());
    let zk: HashMap<_, _> = zh.iter().map(|e| (key(e), e.source.clone())).collect();
    let mut weap: Vec<TmEntry> = en
        .into_iter()
        .filter(|e| e.rec_sig == "WEAP" && e.field_sig == "FULL" && !e.unused)
        .collect();
    let human: HashMap<String, String> = weap
        .iter()
        .filter_map(|e| zk.get(&key(e)).map(|z| (e.source.clone(), z.clone())))
        .collect();
    let a = assistant(&data("assets/data/assist/glossary_ENG-ZHO.json"), "ENG-ZHO");
    assert_eq!(
        a.translate("Nord Steel Warhammer", true).unwrap().text,
        "诺德钢制战锤"
    );
    run("IWI, ZHO", &mut weap, &a, &human);
}
