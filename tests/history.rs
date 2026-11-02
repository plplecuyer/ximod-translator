//! A-18 on a real plugin: the operations leave dated entries, written with
//! the mod's dictionary and read again; the previous translation; the
//! autosave carries the entries of the session (T-A18-01, 02, 04, 05).

use std::path::{Path, PathBuf};

use ximod_translator::history::{self, History, key_of};
use ximod_translator::plugin;
use ximod_translator::tm::{self, TranslationMemory};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Type `text` on line `i`, as a cell edit.
fn type_on(tm: &mut TranslationMemory, undo: &mut UndoStack, i: usize, text: &str) {
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    tm.entries[i].trans = text.into();
    tm.entries[i].set_translated_flags(true);
    undo.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
}

#[test]
fn entries_saved_reopened_and_previous_translation() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let lines: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused)
        .take(3)
        .collect();
    let mut undo = UndoStack::new(50);
    let mut h = History::default();
    // T-A18-01: three lines changed, saved, read again.
    for (n, &i) in lines.iter().enumerate() {
        type_on(&mut tm, &mut undo, i, &format!("Première {n}"));
    }
    h.record(&undo.take_log(), &tm.entries, 1000);
    assert_eq!(h.entries.len(), 3);
    assert!(
        h.entries
            .iter()
            .all(|e| e.op == "edit" && e.t.contains('T'))
    );
    let dir = std::env::temp_dir().join(format!("ximod-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dict = dir.join("skyrimle_riverwood_ENG-FRA.ximdb");
    let file = history::file_for(&dict);
    assert_eq!(file.extension().unwrap(), "ximhist");
    h.save(&file, 1000).unwrap();
    let mut reopened = History::default();
    reopened.load(&file);
    assert_eq!(reopened.entries.len(), 3);
    assert_eq!(reopened.unsaved, 0);

    // T-A18-02: two lines changed twice; their previous translation is the
    // first one, and an undo entry does not count.
    for &i in &lines[..2] {
        type_on(&mut tm, &mut undo, i, "Seconde");
    }
    reopened.record(&undo.take_log(), &tm.entries, 1000);
    for (n, &i) in lines[..2].iter().enumerate() {
        let e = &tm.entries[i];
        let prev = reopened.previous_of(&key_of(e), &e.trans).unwrap();
        assert_eq!(prev.b, format!("Première {n}"));
    }
    undo.undo(&mut tm.entries).unwrap();
    reopened.record(&undo.take_log(), &tm.entries, 1000);
    let last = reopened.entries.last().unwrap();
    assert!(last.u, "the undo is an entry of its own");
    // Undone, the line is back on its first translation: its previous
    // one is the empty text it had.
    let e = &tm.entries[lines[1]];
    assert_eq!(e.trans, "Première 1");
    assert_eq!(reopened.previous_of(&key_of(e), &e.trans).unwrap().b, "");

    // T-A18-04: the autosave carries the unsaved entries; a resumption
    // finds them.
    let mut snap = tm.clone();
    snap.history = reopened.pending().to_vec();
    assert_eq!(snap.history.len(), 3);
    let state = dir.join("autosave.json");
    tm::save(&snap, &state).unwrap();
    let back = tm::load(&state).unwrap();
    let mut after_crash = History::default();
    after_crash.load(&file);
    after_crash.merge_unsaved(back.history);
    assert_eq!(after_crash.entries.len(), 6);
    assert_eq!(after_crash.unsaved, 3);

    // T-A18-05: an entry made before the source changed is « source
    // différente ».
    let i = lines[0];
    let old = after_crash
        .of_key(&key_of(&tm.entries[i]))
        .last()
        .unwrap()
        .clone();
    tm.entries[i].source.push_str(" (v2)");
    assert_ne!(old.s, tm.entries[i].source);
    let _ = std::fs::remove_dir_all(&dir);
}
