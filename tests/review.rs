//! A-17 on a real plugin: a reviewed copy loaded into the Relecture column,
//! accepted line by line and undone (T-A17-01 to T-A17-05).

use std::path::{Path, PathBuf};

use ximod_translator::filter::Criteria;
use ximod_translator::plugin;
use ximod_translator::review::{self, LoadOptions};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::{self, TranslationMemory};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The plugin with every line translated and validated.
fn translated() -> TranslationMemory {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    for e in tm.entries.iter_mut().filter(|e| !e.unused) {
        e.trans = format!("FR {}", e.source);
        status::set(e, Status::Validated);
    }
    tm
}

#[test]
fn reviewed_copy_loaded_accepted_and_undone() {
    let mut tm = translated();
    let workable: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused)
        .collect();
    assert!(workable.len() >= 22, "{}", workable.len());
    // The reviewer's copy: 20 lines changed (37 in the specification:
    // the fixture has 28 lines), saved as a pivot JSON.
    let mut copy = tm.clone();
    let changed: Vec<usize> = workable[..20].to_vec();
    for &i in &changed {
        copy.entries[i].trans = format!("Relu {}", copy.entries[i].source);
    }
    let dir = std::env::temp_dir().join(format!("ximod-review-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("relu.json");
    tm::save(&copy, &path).unwrap();

    // T-A17-01: loaded into the review, the translations do not move.
    let read = review::read_copy(&path).unwrap();
    let plan = review::plan(&tm.entries, &read, LoadOptions::default());
    assert_eq!(plan.diff, 20);
    assert_eq!(plan.changes.len(), 20);
    let mut undo = UndoStack::new(50);
    let before: Vec<(usize, EntryState)> = plan
        .changes
        .iter()
        .map(|(i, _)| (*i, EntryState::capture(&tm.entries[*i])))
        .collect();
    let trans_before: Vec<String> = tm.entries.iter().map(|e| e.trans.clone()).collect();
    review::commit(&mut tm.entries, &plan);
    undo.push(op_from_before(OpKind::Review, &before, &mut tm.entries));
    assert_eq!(
        tm.entries
            .iter()
            .map(|e| e.trans.clone())
            .collect::<Vec<_>>(),
        trans_before
    );
    let diff_view = Criteria {
        review_diff: true,
        ..Default::default()
    }
    .apply(&tm.entries);
    assert_eq!(diff_view, changed);

    // T-A17-02: ten accepted, Validé with the provenance « relecture »;
    // one Ctrl+Z gives the ten back.
    let ten = &changed[..10];
    let before: Vec<(usize, EntryState)> = ten
        .iter()
        .map(|&i| (i, EntryState::capture(&tm.entries[i])))
        .collect();
    assert_eq!(review::accept(&mut tm.entries, ten).len(), 10);
    undo.push(op_from_before(
        OpKind::ReviewAccept,
        &before,
        &mut tm.entries,
    ));
    for &i in ten {
        assert!(tm.entries[i].trans.starts_with("Relu "));
        assert_eq!(status::derive(&tm.entries[i]), Status::Validated);
        assert_eq!(tm.entries[i].prov, "review");
    }
    undo.undo(&mut tm.entries).unwrap();
    for &i in ten {
        assert!(tm.entries[i].trans.starts_with("FR "));
        assert!(!tm.entries[i].review.is_empty(), "the review stays loaded");
    }

    // T-A17-03: a review typed on a Validé line changes neither its
    // status nor its translation nor its provenance.
    let i = workable[21];
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    let (t0, p0) = (tm.entries[i].trans.clone(), tm.entries[i].prov.clone());
    tm.entries[i].review = "Ma relecture".into();
    undo.push(op_from_before(OpKind::Review, &before, &mut tm.entries));
    assert_eq!(status::derive(&tm.entries[i]), Status::Validated);
    assert_eq!(
        (tm.entries[i].trans.clone(), tm.entries[i].prov.clone()),
        (t0, p0)
    );

    // T-A17-04: saved and reopened, the reviews are there (pivot JSON).
    let saved = dir.join("travail.json");
    tm::save(&tm, &saved).unwrap();
    let back = tm::load(&saved).unwrap();
    assert_eq!(review::count(&back.entries), 21);

    // T-A17-05: STRINGS and SST carry no review.
    let sst = dir.join("travail.sst");
    tm::save(&tm, &sst).unwrap();
    assert_eq!(review::count(&tm::load(&sst).unwrap().entries), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
