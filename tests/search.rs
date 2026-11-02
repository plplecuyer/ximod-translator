//! A-19 on a real plugin and with the shipped presets: the advanced search
//! applied to the grid with the filter bar, the groups compared, a preset
//! saved and found again, an invalid expression, 50 000 lines (T-A19-01 to
//! T-A19-05).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ximod_translator::filter::{Criteria, SearchFilter};
use ximod_translator::plugin;
use ximod_translator::search::{self, F_FIELD, F_RECORD, F_SOURCE, F_TRANS, Groups, Preset, Spec};
use ximod_translator::status::{self, Status};

fn root(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(p)
}

fn shipped() -> Vec<Preset> {
    search::load(
        Some(&root("assets/data/search_presets.json")),
        Path::new("/nonexistent/search_presets.json"),
    )
}

fn preset(name: &str) -> Spec {
    shipped().into_iter().find(|p| p.name == name).unwrap().spec
}

#[test]
fn advanced_search_on_a_plugin() {
    let mut tm = plugin::read_file(&root("tests/fixtures/skyrimle_riverwood.esp")).unwrap();
    let workable: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused)
        .collect();
    for (n, &i) in workable.iter().enumerate() {
        let e = &mut tm.entries[i];
        e.trans = format!("{} fer {n}", e.source);
        status::set(e, Status::Validated);
    }
    // T-A19-01: GRUP « CELL », Champ « FULL », Traduction « fer » (the
    // fixture has no armour), with a status of the filter bar.
    let mut spec = Spec::default();
    spec.texts[F_RECORD] = "cell".into();
    spec.texts[F_FIELD] = "FULL".into();
    spec.texts[F_TRANS] = "fer".into();
    let c = Arc::new(search::compile(&spec).unwrap());
    let found = search::find(&c, &tm.entries, None);
    let cells: Vec<usize> = workable
        .iter()
        .copied()
        .filter(|&i| tm.entries[i].rec_sig == "CELL")
        .collect();
    assert_eq!(found.lines, cells);
    let crit = Criteria {
        search: Some(SearchFilter(c.clone())),
        ..Default::default()
    };
    assert!(crit.is_active());
    assert_eq!(crit.apply(&tm.entries), cells);
    let first = cells[0];
    status::set(&mut tm.entries[first], Status::Pending);
    let mut statuses = [false; status::COUNT];
    statuses[Status::Pending.index()] = true;
    let crit = Criteria {
        search: Some(SearchFilter(c)),
        statuses,
        ..Default::default()
    };
    assert_eq!(crit.apply(&tm.entries), vec![first]);

    // T-A19-02: the shipped « Nombres différents » on « 3 days » / « 5 jours ».
    let a = workable[0];
    let b = workable[1];
    tm.entries[a].source = "You have 3 days left".into();
    tm.entries[a].trans = "Il reste 5 jours".into();
    tm.entries[b].source = "12 gold".into();
    tm.entries[b].trans = "12 pièces".into();
    let mut s = preset("numbers-differ");
    assert_eq!(s.groups, Groups::Differ);
    let f = search::find(&search::compile(&s).unwrap(), &tm.entries, None);
    assert!(f.lines.contains(&a) && !f.lines.contains(&b));
    assert_eq!(f.pairs[0], ("3".to_string(), "5".to_string()));
    s.groups = Groups::Same;
    let same = search::compile(&s).unwrap().apply(&tm.entries, None);
    assert!(same.contains(&b) && !same.contains(&a));

    // The other shipped presets.
    let c = workable[2];
    tm.entries[c].source = "Take <Alias=Item> now".into();
    tm.entries[c].trans = "Prenez-le".into();
    let tags = search::compile(&preset("tags-missing")).unwrap();
    assert!(tags.apply(&tm.entries, None).contains(&c));
    let d = workable[3];
    tm.entries[d].trans = tm.entries[d].source.clone();
    let same = search::compile(&preset("trans-equals-source")).unwrap();
    assert_eq!(same.apply(&tm.entries, None), vec![d]);
    let no_edid = search::compile(&preset("no-edid")).unwrap();
    assert!(
        no_edid
            .apply(&tm.entries, None)
            .iter()
            .all(|&i| tm.entries[i].edid.is_empty())
    );

    // T-A19-03: a preset saved, read again.
    let dir = std::env::temp_dir().join(format!("ximod-search-{}", std::process::id()));
    let user = dir.join("data").join(search::PRESETS_FILE);
    let mut ps = shipped();
    ps.push(Preset {
        name: "Mes nombres".into(),
        key: String::new(),
        spec: s.clone(),
        shipped: false,
    });
    search::save_user(&ps, &user).unwrap();
    let again = search::load(Some(&root("assets/data/search_presets.json")), &user);
    assert_eq!(again.len(), 5);
    assert_eq!(again.last().unwrap().spec, s);
    let _ = std::fs::remove_dir_all(&dir);

    // T-A19-04: « [ » in Source in .* mode is the error of that field.
    let mut bad = Spec::default();
    bad.texts[F_SOURCE] = "[".into();
    bad.regex[F_SOURCE] = true;
    assert!(search::compile(&bad).unwrap_err()[F_SOURCE].is_some());
}

/// T-A19-05: 50 000 lines, two expressions with groups.
#[test]
fn fifty_thousand_lines() {
    let tm = plugin::read_file(&root("tests/fixtures/skyrimle_riverwood.esp")).unwrap();
    let base = tm.entries[1].clone();
    let entries: Vec<_> = (0..50_000)
        .map(|i| {
            let mut e = base.clone();
            e.source = format!("You have {i} days left in the {} realm", i % 7);
            e.trans = format!(
                "Il vous reste {} jours dans le royaume {}",
                i + i % 3,
                i % 7
            );
            e
        })
        .collect();
    let s = preset("numbers-differ");
    let c = search::compile(&s).unwrap();
    let start = Instant::now();
    let f = search::find(&c, &entries, None);
    let took = start.elapsed();
    assert_eq!(f.lines.len(), (0..50_000).filter(|i| i % 3 != 0).count());
    // Debug build; the release build is about ten times faster.
    assert!(took < Duration::from_secs(3), "{took:?}");
    eprintln!("50 000 lines: {took:?}");
}
