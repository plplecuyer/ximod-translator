//! A-01 on a real memory: applying a `.ximdb` to `skyrimle_riverwood.esp`
//! then undoing gives back the original entries, structurally; redoing gives
//! back the applied state. Then a chain of small operations (cell edits,
//! status changes) undone in order (T-A01-02) and the capacity (T-A01-05).

use std::path::{Path, PathBuf};

use ximod_translator::basedb::{BaseDb, BaseRow, write_ximdb};
use ximod_translator::plugin;
use ximod_translator::status::{self, Status};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before, record};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn apply_base_undo_redo_is_exact() {
    let tm0 = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let mut tm = tm0.clone();
    // A base translating every BOOK:FULL line of the file.
    let rows: Vec<BaseRow> = tm
        .entries
        .iter()
        .filter(|e| e.rec_sig == "BOOK" && e.field_sig == "FULL")
        .map(|e| BaseRow {
            rec: e.rec_sig.clone(),
            field: e.field_sig.clone(),
            formid: e.formid,
            edid: e.edid.clone(),
            source: e.source.clone(),
            trans: format!("FR {}", e.source),
            status: None,
            ..Default::default()
        })
        .collect();
    let n_rows = rows.len();
    assert!(n_rows > 0);
    let dir = std::env::temp_dir().join(format!("ximod-undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("base_ENG-FRA.ximdb");
    write_ximdb(rows, "fra", &path).unwrap();
    let db = BaseDb::from_file(&path).unwrap();

    let mut stack = UndoStack::new(50);
    let filled = record(
        &mut stack,
        &mut tm.entries,
        OpKind::ApplyBase,
        Some("base".into()),
        |es| db.auto_translate_entries(es),
    );
    assert_eq!(filled, n_rows);
    let applied = tm.clone();
    assert_ne!(applied.entries, tm0.entries);
    assert_eq!(
        stack.peek_undo(),
        Some((OpKind::ApplyBase, Some("base"), n_rows))
    );

    let r = stack.undo(&mut tm.entries).unwrap();
    assert_eq!(r.count(), n_rows);
    assert_eq!(
        tm.entries, tm0.entries,
        "undo restores the original entries exactly"
    );
    stack.redo(&mut tm.entries).unwrap();
    assert_eq!(
        tm.entries, applied.entries,
        "redo restores the applied state exactly"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edits_and_statuses_undo_in_order() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let mut stack = UndoStack::new(10);
    // T-A01-02: type in a cell, then change the status of 12 lines, Ctrl+Z twice.
    let before = vec![(1, EntryState::capture(&tm.entries[1]))];
    tm.entries[1].trans = "Épée en fer".into();
    stack.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
    let idxs: Vec<usize> = (2..14).collect();
    let before: Vec<(usize, EntryState)> = idxs
        .iter()
        .map(|i| (*i, EntryState::capture(&tm.entries[*i])))
        .collect();
    for i in &idxs {
        status::set(&mut tm.entries[*i], Status::Validated);
    }
    stack.push(op_from_before(OpKind::Status, &before, &mut tm.entries));
    assert_eq!(stack.len(), 2);
    let r = stack.undo(&mut tm.entries).unwrap();
    assert_eq!((r.kind, r.count()), (OpKind::Status, 12));
    assert!(
        idxs.iter()
            .all(|i| status::derive(&tm.entries[*i]) != Status::Validated)
    );
    assert_eq!(tm.entries[1].trans, "Épée en fer");
    let r = stack.undo(&mut tm.entries).unwrap();
    assert_eq!((r.kind, r.first_idx()), (OpKind::Edit, Some(1)));
    assert_eq!(tm.entries[1].trans, "");
    assert!(stack.undo(&mut tm.entries).is_none());

    // T-A01-05: depth 10, twelve edits → ten undoable.
    let mut stack = UndoStack::new(10);
    for i in 0..12 {
        record(&mut stack, &mut tm.entries, OpKind::Edit, None, |es| {
            es[0].trans = format!("v{i}")
        });
    }
    let mut n = 0;
    while stack.undo(&mut tm.entries).is_some() {
        n += 1;
    }
    assert_eq!(n, 10);
    assert_eq!(tm.entries[0].trans, "v1");
}
