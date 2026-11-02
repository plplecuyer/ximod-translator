//! B-10 on a real plugin: a part chosen by statuses and records written in
//! the pivot JSON, the document untouched; a selection written in SST and
//! loaded into the review; nothing to write; STRINGS refused (T-B10-01 to
//! T-B10-04).

use std::path::{Path, PathBuf};

use ximod_translator::history::{self, Entry};
use ximod_translator::plugin;
use ximod_translator::review::{self, LoadOptions};
use ximod_translator::savepart::{self, Choices, Refusal};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::{self, TranslationMemory};

fn doc() -> TranslationMemory {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp");
    let mut tm = plugin::read_file(&p).unwrap();
    // Statuses in turn: Validé, En attente, Personnalisé, À traduire.
    for (n, e) in tm.entries.iter_mut().enumerate() {
        if e.unused {
            continue;
        }
        match n % 4 {
            0 => {
                e.trans = format!("{} (fr)", e.source);
                status::set(e, Status::Validated);
            }
            1 => {
                e.trans = format!("{} (fr)", e.source);
                status::set(e, Status::Pending);
            }
            2 => {
                e.trans = format!("{} (fr)", e.source);
                status::set(e, Status::Custom);
            }
            _ => {}
        }
        e.set_translated_flags(!e.trans.is_empty());
    }
    tm
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ximod-savepart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// T-B10-01: Validé and En attente, the two most frequent records, the
/// whole document, in the pivot JSON with the history of those lines.
#[test]
fn statuses_and_records_in_json() {
    let tm = doc();
    let before = serde_json::to_string(&tm).unwrap();
    let recs = savepart::records_of(&tm.entries);
    assert!(recs.len() >= 2, "{recs:?}");
    let kept: std::collections::BTreeSet<String> =
        recs.iter().take(2).map(|(r, _)| r.clone()).collect();
    let mut c = Choices {
        statuses: [false; status::COUNT],
        ..Default::default()
    };
    c.statuses[Status::Validated.index()] = true;
    c.statuses[Status::Pending.index()] = true;
    c.records = Some(kept.clone());
    let all: Vec<usize> = (0..tm.entries.len()).collect();
    let lines = savepart::lines(&tm.entries, &all, &c);
    let expected: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| {
            let e = &tm.entries[i];
            !e.unused
                && kept.contains(&e.rec_sig)
                && matches!(status::derive(e), Status::Validated | Status::Pending)
        })
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(lines, expected);
    let b = savepart::breakdown(&tm.entries, &lines);
    assert_eq!(b.records.iter().map(|(_, n)| n).sum::<usize>(), lines.len());
    assert!(
        b.statuses
            .iter()
            .all(|(s, _)| matches!(s, Status::Validated | Status::Pending))
    );
    // A history for one written line and one other line.
    let mk = |i: usize| Entry {
        t: "2026-10-26T10:00:00".into(),
        k: history::key_of(&tm.entries[i]),
        s: String::new(),
        b: String::new(),
        a: tm.entries[i].trans.clone(),
        sb: String::new(),
        sa: String::new(),
        p: String::new(),
        op: "edit".into(),
        arg: None,
        u: false,
        rb: None,
        ra: None,
    };
    let other = all
        .iter()
        .copied()
        .find(|i| !lines.contains(i) && !tm.entries[*i].unused)
        .unwrap();
    let hist = vec![mk(lines[0]), mk(other)];
    let part = savepart::memory(&tm, &lines, false, &hist);
    let path = temp("part.json");
    tm::save(&part, &path).unwrap();
    let back = tm::load(&path).unwrap();
    assert_eq!(back.entries.len(), lines.len());
    for (e, &i) in back.entries.iter().zip(&lines) {
        assert_eq!(e.source, tm.entries[i].source);
        assert_eq!(e.trans, tm.entries[i].trans);
        assert_eq!(status::derive(e), status::derive(&tm.entries[i]));
    }
    assert_eq!(back.history.len(), 1);
    // The document is untouched.
    assert_eq!(serde_json::to_string(&tm).unwrap(), before);
    let _ = std::fs::remove_file(&path);
}

/// T-B10-02: a selection of 12 lines in SST, loaded into the review.
#[test]
fn selection_in_sst_loaded_into_the_review() {
    let mut tm = doc();
    let selection: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused && !tm.entries[i].trans.is_empty())
        .take(12)
        .collect();
    assert_eq!(selection.len(), 12);
    let lines = savepart::lines(&tm.entries, &selection, &Choices::default());
    assert_eq!(lines, selection);
    // The reviewer corrects the part.
    let mut part = savepart::memory(&tm, &lines, false, &[]);
    for e in &mut part.entries {
        e.trans = format!("{} — relu", e.trans);
    }
    let path = temp("part.sst");
    tm::save(&part, &path).unwrap();
    let copy = review::read_copy(&path).unwrap();
    assert_eq!(copy.entries.len(), 12);
    let plan = review::plan(&tm.entries, &copy, LoadOptions::default());
    assert_eq!(plan.matched, 12);
    let changed = review::commit(&mut tm.entries, &plan);
    assert_eq!(changed.len(), 12);
    assert!(
        selection
            .iter()
            .all(|&i| tm.entries[i].review.ends_with("— relu"))
    );
    let _ = std::fs::remove_file(&path);
}

/// T-B10-03 and T-B10-04: an empty view writes nothing; STRINGS refused;
/// « conserver la source ».
#[test]
fn nothing_to_write_strings_refused_source_kept() {
    let tm = doc();
    assert!(savepart::lines(&tm.entries, &[], &Choices::default()).is_empty());
    let c = Choices {
        statuses: [false; status::COUNT],
        ..Default::default()
    };
    let all: Vec<usize> = (0..tm.entries.len()).collect();
    assert!(savepart::lines(&tm.entries, &all, &c).is_empty());
    for name in ["part.strings", "part.DLSTRINGS", "part.ilstrings"] {
        assert_eq!(savepart::check_path(Path::new(name)), Err(Refusal::Strings));
    }
    for name in [
        "part.json",
        "part.sst",
        "part.eet",
        "part.dsd",
        "part.csv",
        "part.xml",
    ] {
        assert!(savepart::check_path(Path::new(name)).is_ok(), "{name}");
    }
    // An untranslated line kept: its source, or nothing.
    let i = all
        .iter()
        .copied()
        .find(|&i| !tm.entries[i].unused && tm.entries[i].trans.is_empty())
        .unwrap();
    assert_eq!(
        savepart::memory(&tm, &[i], true, &[]).entries[0].trans,
        tm.entries[i].source
    );
    assert!(
        savepart::memory(&tm, &[i], false, &[]).entries[0]
            .trans
            .is_empty()
    );
}
