//! B-08 on a real plugin: two dictionaries translating the same source
//! differently leave the line En attente and marked; the mark is filtered,
//! ends with Utiliser or a decision, never applies to a record match, and
//! survives a save in the mod's dictionary (T-B08-01 to T-B08-05).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ximod_translator::ambiguity;
use ximod_translator::basedb::{BaseDb, BaseRow, write_ximdb};
use ximod_translator::dict_stack::{DictStack, Level};
use ximod_translator::dicts::Role;
use ximod_translator::filter::Criteria;
use ximod_translator::plugin;
use ximod_translator::status::{self, Status};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn row(rec: &str, field: &str, src: &str, trans: &str) -> BaseRow {
    BaseRow {
        rec: rec.into(),
        field: field.into(),
        source: src.into(),
        trans: trans.into(),
        ..Default::default()
    }
}

fn level(dir: &Path, name: &str, role: Role, rows: Vec<BaseRow>) -> Level {
    let path = dir.join(name);
    write_ximdb(rows, "ENG-FRA", &path).unwrap();
    let db = BaseDb::from_file(&path).unwrap();
    Level {
        role,
        path,
        db: Arc::new(db),
    }
}

#[test]
fn ambiguous_sources_are_marked_filtered_and_decided() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let dir = std::env::temp_dir().join(format!("ximod-amb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The plugin has two « Chest » and three « Knapsack » containers, one
    // « Riverwood Home » cell; « Chest » is translated two ways across the
    // two dictionaries, « Knapsack » one way, « Riverwood Home » by its
    // own record in the game dictionary.
    let home = tm
        .entries
        .iter()
        .find(|e| e.source == "Riverwood Home" && e.rec_sig == "CELL")
        .unwrap();
    let mut home_row = BaseRow::from_entry(home, false);
    home_row.trans = "Maison de Rivebois".into();
    let a = level(
        &dir,
        "a_ENG-FRA.ximdb",
        Role::External,
        vec![
            row("CONT", "FULL", "Chest", "Coffre"),
            row("CONT", "FULL", "Knapsack", "Sac à dos"),
        ],
    );
    let b = level(
        &dir,
        "b_ENG-FRA.ximdb",
        Role::Game,
        vec![
            row("MISC", "FULL", "Chest", "Malle"),
            row("CONT", "FULL", "Knapsack", "Sac à dos "),
            home_row,
        ],
    );
    let stack = DictStack {
        levels: vec![a, b],
        normalize: true,
    };
    let mut undo = UndoStack::new(50);
    let before: Vec<(usize, EntryState)> = tm
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (i, EntryState::capture(e)))
        .collect();
    let mut origins = Vec::new();
    let report = stack.auto_translate_entries(&mut tm.entries, &mut origins);
    undo.push(op_from_before(OpKind::ApplyBase, &before, &mut tm.entries));

    // T-B08-01: « Chest » En attente, marked, provenance « ambiguë (2) »,
    // filled by the first dictionary.
    let chests: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| tm.entries[i].source == "Chest")
        .collect();
    assert_eq!(chests.len(), 2);
    for &i in &chests {
        let e = &tm.entries[i];
        assert_eq!(e.trans, "Coffre");
        assert_eq!(status::derive(e), Status::Pending);
        assert_eq!(ambiguity::count_of(e), Some(2));
        assert_eq!(e.prov, "dict:external:text:a_ENG-FRA.ximdb|amb:2");
    }
    assert_eq!(report.ambiguous, 2);
    // « Knapsack »: one translation (spaces apart), not ambiguous.
    let sack = tm.entries.iter().find(|e| e.source == "Knapsack").unwrap();
    assert_eq!(status::derive(sack), Status::BySimilarity);
    assert!(!ambiguity::is_marked(sack));

    // T-B08-02: the Ambiguës filter shows these lines alone.
    let crit = Criteria {
        ambiguous_only: true,
        ..Default::default()
    };
    assert_eq!(crit.apply(&tm.entries), chests);

    // T-B08-04: « Riverwood Home » found by its record: not marked, even
    // though another candidate text could exist.
    let home = tm
        .entries
        .iter()
        .find(|e| e.source == "Riverwood Home" && e.rec_sig == "CELL")
        .unwrap();
    assert_eq!(home.trans, "Maison de Rivebois");
    assert!(!ambiguity::is_marked(home));

    // The candidates of the Vocabulaire tab: « Coffre » (a) and « Malle » (b).
    let cands = stack.candidates("Chest");
    assert_eq!(cands.len(), 2);
    assert_eq!(cands[0].trans, "Coffre");
    assert_eq!(cands[1].trans, "Malle");
    assert_eq!(cands[1].dicts[0].0, "b_ENG-FRA.ximdb");
    assert_eq!(cands[1].count(), 1);

    // T-B08-05: saved in the mod's dictionary with its provenance and
    // read again, the mark is still there.
    let mod_dict = dir.join("riverwood_ENG-FRA.ximdb");
    let rows: Vec<BaseRow> = tm
        .entries
        .iter()
        .filter(|e| !e.unused)
        .map(|e| BaseRow::from_entry(e, true))
        .collect();
    write_ximdb(rows, "ENG-FRA", &mod_dict).unwrap();
    let mut reopened = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let mod_level = Level {
        role: Role::Mod,
        path: mod_dict.clone(),
        db: Arc::new(BaseDb::from_file(&mod_dict).unwrap()),
    };
    let stack2 = DictStack {
        levels: vec![mod_level],
        normalize: true,
    };
    stack2.auto_translate_entries(&mut reopened.entries, &mut Vec::new());
    for &i in &chests {
        let e = &reopened.entries[i];
        assert_eq!(status::derive(e), Status::Pending);
        assert_eq!(ambiguity::count_of(e), Some(2), "{}", e.prov);
    }

    // T-B08-03: Utiliser « Malle »: the mark goes, the translation is
    // « Malle », provenance of the chosen dictionary.
    let i = chests[0];
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    let e = &mut tm.entries[i];
    e.trans = cands[1].trans.clone();
    e.prov = cands[1].prov();
    status::set(e, Status::Pending);
    undo.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
    let e = &tm.entries[i];
    assert_eq!(e.trans, "Malle");
    assert!(!ambiguity::is_marked(e));
    assert_eq!(e.prov, "dict:game:text:b_ENG-FRA.ximdb");
    // Validating the other one decides it too (R-B08-05); Ctrl+Z brings
    // the mark back.
    let j = chests[1];
    let before = vec![(j, EntryState::capture(&tm.entries[j]))];
    status::set(&mut tm.entries[j], Status::Validated);
    undo.push(op_from_before(OpKind::Status, &before, &mut tm.entries));
    assert!(!ambiguity::is_marked(&tm.entries[j]));
    assert_eq!(tm.entries[j].prov, "dict:external:text:a_ENG-FRA.ximdb");
    undo.undo(&mut tm.entries).unwrap();
    assert!(ambiguity::is_marked(&tm.entries[j]));
    let _ = std::fs::remove_dir_all(&dir);
}
