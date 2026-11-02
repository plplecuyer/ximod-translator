//! A-10 on a real plugin: the filter row combines its fields with AND and
//! with the filter bar; an invalid expression is reported and ignored
//! (T-A10-01 to T-A10-05).

use std::path::{Path, PathBuf};

use ximod_translator::filter::{ColumnFilters, Criteria, FilterCol};
use ximod_translator::plugin;
use ximod_translator::status::{self, Status};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn column_filters_combine_with_and() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    for e in tm.entries.iter_mut().filter(|e| !e.unused) {
        e.trans = format!("{} en fer", e.source);
        status::set(e, Status::Validated);
    }
    let cell_full: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| {
            let e = &tm.entries[i];
            !e.unused && e.rec_sig == "CELL" && e.field_sig == "FULL"
        })
        .collect();
    assert!(!cell_full.is_empty());

    // T-A10-01: « ARMO » under GRUP and « FULL » under Champ (the fixture
    // has no armour: CELL names instead); case does not matter.
    let mut cols = ColumnFilters::default();
    *cols.get_mut(FilterCol::Record) = "cell".into();
    *cols.get_mut(FilterCol::Field) = "FULL".into();
    let crit = Criteria {
        columns: cols.clone(),
        ..Default::default()
    };
    assert!(crit.is_active());
    assert_eq!(crit.apply(&tm.entries), cell_full);

    // T-A10-02: the eraser empties every field; every line is back.
    cols.clear();
    assert!(cols.is_empty());
    let all = Criteria::default().apply(&tm.entries);
    let crit = Criteria {
        columns: cols.clone(),
        ..Default::default()
    };
    assert!(!crit.is_active());
    assert_eq!(crit.apply(&tm.entries), all);

    // T-A10-03: .* mode, « ^Iron » under Source (« ^Riverwood » here) and
    // « fer » under Traduction.
    *cols.get_mut(FilterCol::Source) = "^Riverwood".into();
    *cols.get_mut(FilterCol::Trans) = "fer".into();
    let crit = Criteria {
        columns: cols.clone(),
        use_regex: true,
        ..Default::default()
    };
    let got = crit.apply(&tm.entries);
    assert!(!got.is_empty());
    for &i in &got {
        assert!(tm.entries[i].source.starts_with("Riverwood"));
        assert!(tm.entries[i].trans.contains("fer"));
    }
    let expected: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| tm.entries[i].source.starts_with("Riverwood"))
        .collect();
    assert_eq!(got, expected);

    // T-A10-04: « [ » under Source in .* mode: that field is in error and
    // ignored, the Traduction field still acts.
    *cols.get_mut(FilterCol::Source) = "[".into();
    let compiled = cols.compile(true);
    assert!(compiled.errors[FilterCol::Source.index()].is_some());
    assert!(compiled.errors[FilterCol::Trans.index()].is_none());
    assert_eq!(compiled.matchers.len(), 1);
    let crit = Criteria {
        columns: cols.clone(),
        use_regex: true,
        ..Default::default()
    };
    assert_eq!(crit.apply(&tm.entries), all, "every line has « fer »");

    // T-A10-05: a column filter and the Pending status of the bar apply
    // together.
    let pending = all[0];
    status::set(&mut tm.entries[pending], Status::Pending);
    let mut cols = ColumnFilters::default();
    *cols.get_mut(FilterCol::Trans) = "fer".into();
    let mut statuses = [false; status::COUNT];
    statuses[Status::Pending.index()] = true;
    let crit = Criteria {
        columns: cols,
        statuses,
        ..Default::default()
    };
    assert_eq!(crit.apply(&tm.entries), vec![pending]);

    // The EDID field takes a FormID too, like « EDID ou FormID ».
    let e = &tm.entries[all[1]];
    let mut cols = ColumnFilters::default();
    *cols.get_mut(FilterCol::Edid) = format!("{:08X}", e.formid);
    let crit = Criteria {
        columns: cols,
        ..Default::default()
    };
    assert!(crit.apply(&tm.entries).contains(&all[1]));
}
