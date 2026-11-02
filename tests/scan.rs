//! Analyser un dossier (D-07, T-D07-01 / 05) on the recette folder when it
//! is present: the files taken, archives left, the stack applied without
//! writing anything.

use std::path::Path;

use ximod_translator::headless::Kind;
use ximod_translator::scan::{self, Options, Scanner, State};
use ximod_translator::textfmt::Formats;

#[test]
fn the_recette_folder() {
    let root = Path::new("/root/ximod_work/recette_lot4");
    if !root.is_dir() {
        eprintln!("recette folder absent: skipped");
        return;
    }
    let formats = Formats::builtin();
    let opts = Options::default();
    let files = scan::list(root, opts, &formats);
    let count = |k: Kind| files.iter().filter(|f| f.1 == k).count();
    let plugins = count(Kind::Plugin);
    let dsd = count(Kind::Dsd);
    let texts = count(Kind::Text);
    eprintln!(
        "plugins {plugins} dsd {dsd} texts {texts} dict {}",
        count(Kind::Dictionary)
    );
    assert_eq!(plugins, 22);
    assert!(files.iter().all(|f| {
        let e =
            f.0.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
        !matches!(e.as_str(), "7z" | "zip" | "rar" | "bsa" | "ba2")
    }));
    let json = files
        .iter()
        .filter(|f| {
            f.0.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .count();
    eprintln!("json {json}");
    // The Immersive Weapons translations are sources of dictionary.
    let cfg = ximod_translator::config::AppConfig::default();
    let mods = cfg.mod_base_dir();
    let listing = |d: &Path| -> Vec<String> {
        let mut v: Vec<String> = walk(d);
        v.sort();
        v
    };
    let before = listing(&mods);
    let mut sc = Scanner::new(cfg.clone(), root, "SkyrimSE", "eng", "fra");
    for (p, k) in files
        .iter()
        .filter(|f| f.0.to_string_lossy().contains("Immersive Weapons"))
    {
        let r = sc.analyze(p, *k);
        eprintln!(
            "{} {:?} {:?} lines {} filled {} left {} locked {}",
            r.rel, r.state, r.dict, r.lines, r.filled, r.left, r.locked
        );
        if *k == Kind::Dsd {
            assert_eq!(r.state, State::Source);
        }
    }
    // T-D07-05: the analysis writes no dictionary of the mod.
    assert_eq!(listing(&mods), before);
}

fn walk(d: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p.display().to_string());
            }
        }
    }
    out
}

/// T-D07-02 on the two French DSD of Immersive Weapons, when present.
#[test]
fn the_two_immersive_weapons_dsd() {
    use ximod_translator::compare::{self, Category, Mode, Prefer};
    let dir = Path::new("/root/ximod_work/recette_lot4/Lot 4-1/Immersive Weapons");
    let a = dir.join("Immersive Weapons - French-93449-2-0FRDSD01-1734781267/Data/SKSE/Plugins/DynamicStringDistributor/immersive weapons.esp/Immersive Weapons.json");
    let b = dir.join("Immersive Weapons Trad FR DSD-149986-2-1747149345/SKSE/Plugins/DynamicStringDistributor/Immersive Weapons.esp/Immersive Weapons.json");
    if !a.is_file() || !b.is_file() {
        eprintln!("recette files absent: skipped");
        return;
    }
    let ta = compare::read_side(&a, "eng").unwrap();
    let tb = compare::read_side(&b, "eng").unwrap();
    for (mode, fold) in [
        (Mode::Record, true),
        (Mode::Record, false),
        (Mode::Text, true),
    ] {
        let r = compare::compare(&ta, &tb, mode, fold);
        eprintln!(
            "{mode:?} fold {fold}: a {} b {} common {} same {} diff {} onlyA {} onlyB {}",
            r.a_lines,
            r.b_lines,
            r.common(),
            r.count(Category::Same),
            r.count(Category::Diff),
            r.count(Category::OnlyA),
            r.count(Category::OnlyB)
        );
    }
    let r = compare::compare(&ta, &tb, Mode::Record, true);
    let (rows, amb) = compare::merge(&r, Prefer::A, "x.ximdb");
    eprintln!("merge {} amb {amb}", rows.len());
}
