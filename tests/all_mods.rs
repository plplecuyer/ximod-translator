//! B-25 on real plugins: a mod and two patches with common sources; a
//! translation typed in the mod reaches the patches En attente with the
//! provenance « propagation (<fichier>) », a Validé line is left alone, the
//! other way round works too, an undo is not propagated (T-B25-01/02).

use std::path::Path;

use ximod_translator::plugin;
use ximod_translator::propagate_files::{self, may_receive};
use ximod_translator::provenance;
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn entries() -> Vec<TmEntry> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp");
    plugin::read_file(&p).unwrap().entries
}

/// A translation typed in a cell, as the grid records it.
fn type_in(stack: &mut UndoStack, entries: &mut [TmEntry], i: usize, text: &str) {
    let before = vec![(i, EntryState::capture(&entries[i]))];
    let e = &mut entries[i];
    e.trans = text.into();
    e.prov = provenance::TYPED.into();
    e.set_translated_flags(true);
    status::set(e, Status::Pending);
    stack.push(op_from_before(OpKind::Edit, &before, entries));
}

#[test]
fn propagation_between_a_mod_and_two_patches() {
    let mut mod_ = entries();
    let mut patch_a = entries();
    let mut patch_b = entries();
    let (mut s_mod, mut s_a) = (UndoStack::new(50), UndoStack::new(50));
    // The first source shown in game, and its copies in the patches.
    let i = mod_
        .iter()
        .position(|e| !e.unused && !e.source.trim().is_empty())
        .unwrap();
    let src = mod_[i].source.clone();
    let same = |es: &[TmEntry]| -> Vec<usize> {
        (0..es.len())
            .filter(|&k| !es[k].unused && es[k].source.trim() == src.trim())
            .collect()
    };
    // Patch B already has it Validé.
    for k in same(&patch_b) {
        let e = &mut patch_b[k];
        e.trans = "Déjà validée".into();
        e.set_translated_flags(true);
        status::set(e, Status::Validated);
    }
    assert!(same(&patch_a).iter().all(|&k| may_receive(&patch_a[k])));

    // T-B25-01: typed in the mod.
    type_in(&mut s_mod, &mut mod_, i, "Traduction du mod");
    let log = s_mod.take_log();
    let written = propagate_files::written(&log, &mod_);
    assert_eq!(
        written,
        vec![(src.trim().to_string(), "Traduction du mod".to_string())]
    );
    let op = propagate_files::apply(&mut patch_a, &written, "MonMod.esp").unwrap();
    assert_eq!(op.changes.len(), same(&patch_a).len());
    s_a.push(op);
    for k in same(&patch_a) {
        assert_eq!(patch_a[k].trans, "Traduction du mod");
        assert_eq!(status::derive(&patch_a[k]), Status::Pending);
        assert_eq!(patch_a[k].prov, "propagate:MonMod.esp");
    }
    // T-B25-02: the Validé lines of patch B are not touched.
    assert!(propagate_files::apply(&mut patch_b, &written, "MonMod.esp").is_none());
    assert!(
        same(&patch_b)
            .iter()
            .all(|&k| patch_b[k].trans == "Déjà validée")
    );

    // The propagation itself is not propagated again.
    let log_a = s_a.take_log();
    assert!(propagate_files::written(&log_a, &patch_a).is_empty());

    // The other way: typed in patch A on another common source.
    let j = (i + 1..patch_a.len())
        .find(|&k| {
            !patch_a[k].unused
                && !patch_a[k].source.trim().is_empty()
                && patch_a[k].source.trim() != src.trim()
                && patch_a[k].trans.is_empty()
        })
        .unwrap();
    type_in(&mut s_a, &mut patch_a, j, "Traduction du patch");
    let written = propagate_files::written(&s_a.take_log(), &patch_a);
    let op = propagate_files::apply(&mut mod_, &written, "Patch A.esp").unwrap();
    s_mod.push(op);
    assert_eq!(mod_[j].trans, "Traduction du patch");

    // Undoing in the mod is not propagated (R-B25-02): the patches keep
    // what they received.
    s_mod.undo(&mut mod_).unwrap();
    assert!(propagate_files::written(&s_mod.take_log(), &mod_).is_empty());
    assert!(mod_[j].trans.is_empty());
}

/// 150 000 lines on three files: the propagation of one translation stays
/// well under the frame budget.
#[test]
fn many_lines() {
    let base = entries();
    let line = base.iter().find(|e| !e.unused).unwrap().clone();
    let big: Vec<TmEntry> = (0..50_000)
        .map(|n| {
            let mut e = line.clone();
            e.source = format!("Line {}", n % 5_000);
            e
        })
        .collect();
    let mut b = big.clone();
    let written = vec![("Line 42".to_string(), "Ligne 42".to_string())];
    let start = std::time::Instant::now();
    let op = propagate_files::apply(&mut b, &written, "MonMod.esp").unwrap();
    let took = start.elapsed();
    assert_eq!(op.changes.len(), 10);
    // Debug build.
    assert!(took < std::time::Duration::from_millis(500), "{took:?}");
}
