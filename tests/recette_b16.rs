//! B-16 recette on the mods of lot 4 (A.5), run by hand:
//! `XIMOD_RECETTE_B16=<recette_lot4> cargo test --release --test
//! recette_b16 -- --ignored --nocapture`.
//! Immersive Weapons (237 WEAP:FULL) with a glossary of at most 100 terms
//! and the shipped patterns, compared with the two French DSD
//! translations; then with the Skyrim SE base as the dictionaries of the
//! variables.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::{
    self, Assistant, Bases, Glossary, GlossaryFile, NoBases, Options, Patterns, Refusal, Rule,
    StackBases,
};
use ximod_translator::games::Game;
use ximod_translator::tm::TmEntry;

fn root() -> Option<PathBuf> {
    std::env::var("XIMOD_RECETTE_B16").ok().map(PathBuf::from)
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn assistant(glossary: &Path) -> Assistant {
    let f = GlossaryFile::parse(&std::fs::read_to_string(glossary).unwrap()).unwrap();
    f.validate().unwrap();
    let p = assist::patterns::shipped_file(&data("assets/data/assist"), "ENG-FRA").unwrap();
    p.validate().unwrap();
    let mut a = Assistant::new(Glossary::new(&[(&f, false)]), p.composition.clone());
    a.patterns = Some(Arc::new(Patterns::new(p)));
    a
}

fn dsd(p: &Path) -> HashMap<String, String> {
    ximod_translator::tm::load(p)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| (e.source, e.trans))
        .collect()
}

fn run(
    name: &str,
    entries: &[TmEntry],
    a: &Assistant,
    bases: &dyn Bases,
    human: &[HashMap<String, String>],
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
        bases,
    );
    let el = t.elapsed();
    let same = plan
        .changes
        .iter()
        .filter(|c| {
            human.iter().any(|h| {
                h.get(&entries[c.idx].source)
                    .is_some_and(|x| x.trim() == c.trans)
            })
        })
        .count();
    eprintln!(
        "{name}: {} lines, {} filled ({} composed, {:?} by pattern, {} glossary), {} partial, {same} equal to a DSD translation ({el:?})",
        entries.len(),
        plan.filled(),
        plan.composed,
        plan.per_pattern,
        plan.glossary,
        plan.partial
    );
    let mut missed: BTreeMap<String, usize> = BTreeMap::new();
    for e in entries {
        if let Err(Refusal::Partial(w)) = a.run(&e.source, true, true, bases) {
            for w in w {
                *missed.entry(w).or_default() += 1;
            }
        }
    }
    let mut m: Vec<_> = missed.into_iter().collect();
    m.sort_by_key(|x| std::cmp::Reverse(x.1));
    eprintln!("  missed: {:?}", &m[..m.len().min(40)]);
    for c in &plan.changes {
        let src = &entries[c.idx].source;
        let h: Vec<&str> = human
            .iter()
            .filter_map(|h| h.get(src).map(String::as_str))
            .collect();
        eprintln!("  {src} → {} [{:?}] | {}", c.trans, c.rule, h.join(" | "));
    }
    plan.filled()
}

#[test]
#[ignore]
fn immersive_weapons() {
    let Some(r) = root() else {
        eprintln!("XIMOD_RECETTE_B16 not set: skipped");
        return;
    };
    let iw = r.join("Lot 4-1/Immersive Weapons");
    let entries: Vec<TmEntry> = ximod_translator::plugin::read_file_as(
        &iw.join("Immersive Weapons.esp"),
        Some(Game::SkyrimSE),
    )
    .unwrap()
    .entries
    .into_iter()
    .filter(|e| e.rec_sig == "WEAP" && e.field_sig == "FULL" && !e.unused)
    .collect();
    assert_eq!(entries.len(), 237);
    let human = vec![
        dsd(&iw.join("Immersive Weapons - French-93449-2-0FRDSD01-1734781267/Data/SKSE/Plugins/DynamicStringDistributor/immersive weapons.esp/Immersive Weapons.json")),
        dsd(&iw.join("Immersive Weapons Trad FR DSD-149986-2-1747149345/SKSE/Plugins/DynamicStringDistributor/Immersive Weapons.esp/Immersive Weapons.json")),
    ];
    let a = assistant(&data("tests/data/assist/iw_100_ENG-FRA.json"));
    assert!(a.glossary.len() <= 100, "{}", a.glossary.len());
    for (s, t) in [
        ("Steel Nordic Spear", "Lance nordique en acier"),
        (
            "Antique Honed Silver Sword",
            "Épée antique affûtée en argent",
        ),
        ("Mace of Molag Bal", "Masse de Molag Bal"),
        ("Ice Blade of the Monarch", "Lame glacée du Monarque"),
    ] {
        assert_eq!(a.run(s, true, true, &NoBases).unwrap().text, t);
    }
    let n = run("IW, 100 terms", &entries, &a, &NoBases, &human);
    assert!(n >= 120, "{n}");
    let plan = assist::plan(
        &entries,
        &(0..entries.len()).collect::<Vec<_>>(),
        &a,
        &Options::default(),
        &NoBases,
    );
    assert!(
        plan.changes
            .iter()
            .all(|c| matches!(c.rule, Rule::Compose | Rule::Pattern(_) | Rule::Glossary))
    );
    // With the game's base for the variables.
    let base = r.join("Bases/SkyrimSE_ENG-FRA.ximdb");
    if let Some(db) = ximod_translator::basedb::BaseDb::from_file(&base) {
        run(
            "IW, 100 terms and the Skyrim SE base",
            &entries,
            &a,
            &StackBases(vec![Arc::new(db)]),
            &human,
        );
    }
}

#[test]
#[ignore]
fn immersive_armors_with_the_patterns() {
    let Some(r) = root() else { return };
    let ia = r.join("Lot 4-1/Immersive Armors");
    let entries: Vec<TmEntry> = ximod_translator::plugin::read_file_as(
        &ia.join("Hothtrooper44_ArmorCompilation.esp"),
        Some(Game::SkyrimSE),
    )
    .unwrap()
    .entries
    .into_iter()
    .filter(|e| e.rec_sig == "ARMO" && e.field_sig == "FULL" && !e.unused)
    .collect();
    let human = vec![dsd(&ia.join("Immersive Armors - French-93078-8-1FRDSD-1743524460/Data/SKSE/Plugins/DynamicStringDistributor/Hothtrooper44_ArmorCompilation.esp/Hothtrooper44_ArmorCompilation.json"))];
    let a = assistant(&data("assets/data/assist/glossary_ENG-FRA.json"));
    let n = run(
        "IA, shipped glossary and patterns",
        &entries,
        &a,
        &NoBases,
        &human,
    );
    assert!(n >= 838, "{n}");
}
