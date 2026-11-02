//! A-04 on a real plugin: the four buttons of the column give a line or a
//! selection its status through the same path as F5–F8, undoable; the
//! impossible statuses of a line without translation are greyed
//! (T-A04-01, 03, 04).

use std::path::{Path, PathBuf};

use ximod_translator::plugin;
use ximod_translator::propagate;
use ximod_translator::status::{self, Status};
use ximod_translator::ui::grid::{self, STATUS_BUTTONS, Selection};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// What the application does on a click of button `st` on line `idx`:
/// the targets of the button, the applicable ones changed, one operation.
fn click(
    tm: &mut ximod_translator::tm::TranslationMemory,
    undo: &mut UndoStack,
    sel: &mut Selection,
    idx: usize,
    st: Status,
) -> usize {
    if !sel.rows.contains(&idx) {
        sel.set_single(idx);
    }
    let targets = grid::button_targets(sel, idx);
    let before: Vec<(usize, EntryState)> = targets
        .iter()
        .filter(|&&i| st.applicable_to(&tm.entries[i]))
        .map(|&i| (i, EntryState::capture(&tm.entries[i])))
        .collect();
    for (i, _) in &before {
        status::set(&mut tm.entries[*i], st);
    }
    let op = op_from_before(OpKind::Status, &before, &mut tm.entries);
    let n = op.changes.len();
    if n > 0 {
        undo.push(op);
    }
    n
}

#[test]
fn buttons_change_a_line_or_a_selection_and_grey_out() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let workable: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused)
        .collect();
    assert!(workable.len() >= 14, "{}", workable.len());
    let mut undo = UndoStack::new(50);
    let mut sel = Selection::default();

    // T-A04-01: a translated line En attente, ✓ clicked: Validé; the
    // click selects it alone first (R-A04-04).
    let a = workable[0];
    tm.entries[a].trans = format!("FR {}", tm.entries[a].source);
    status::set(&mut tm.entries[a], Status::Pending);
    sel.set_single(workable[13]);
    assert_eq!(click(&mut tm, &mut undo, &mut sel, a, Status::Validated), 1);
    assert_eq!(status::derive(&tm.entries[a]), Status::Validated);
    assert_eq!(sel.targets(), vec![a]);
    // The propagation proposal applies in the same conditions as F5: it
    // asks only when the line has differing duplicates.
    let _ = propagate::has_differing_duplicates(&tm.entries, a);

    // T-A04-03: twelve lines selected, Ignoré clicked on one of them: the
    // twelve are Ignoré; one Ctrl+Z gives them back.
    let twelve = &workable[1..13];
    let visible: Vec<usize> = (0..tm.entries.len()).collect();
    sel.clear();
    for &i in twelve {
        sel.click(i, eframe::egui::Modifiers::COMMAND, &visible);
    }
    let before: Vec<Status> = twelve
        .iter()
        .map(|&i| status::derive(&tm.entries[i]))
        .collect();
    assert_eq!(
        click(&mut tm, &mut undo, &mut sel, twelve[4], Status::Ignored),
        12
    );
    assert!(
        twelve
            .iter()
            .all(|&i| status::derive(&tm.entries[i]) == Status::Ignored)
    );
    undo.undo(&mut tm.entries).unwrap();
    let after: Vec<Status> = twelve
        .iter()
        .map(|&i| status::derive(&tm.entries[i]))
        .collect();
    assert_eq!(after, before);

    // T-A04-04: a line without translation: ✓ and ★ greyed, ⋯ and ⊘ not.
    let b = workable[13];
    tm.entries[b].trans.clear();
    tm.entries[b].set_translated_flags(false);
    let enabled: Vec<bool> = STATUS_BUTTONS
        .iter()
        .map(|st| st.applicable_to(&tm.entries[b]))
        .collect();
    assert_eq!(enabled, vec![false, true, false, true]);
    assert_eq!(click(&mut tm, &mut undo, &mut sel, b, Status::Validated), 0);
    // A line never shown in game has no button at all.
    let unused = tm.entries.iter().find(|e| e.unused);
    if let Some(e) = unused {
        assert!(STATUS_BUTTONS.iter().all(|st| !st.applicable_to(e)));
    }
}
