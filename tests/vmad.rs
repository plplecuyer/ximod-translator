//! C-08 on real plugins: the VMAD lines of cheatterminal.esm (the Starfield
//! Cheat Terminal, 296 blocks), locked by default and written back
//! identical; a translated string property written, the rest of the file
//! identical; the structures of Fallout 4 (CMRQL) and the string arrays of
//! Skyrim (Skipper) when the recette files are there; a block of an unknown
//! version kept as it is (T-C08-01 to 05).

use std::path::{Path, PathBuf};

use ximod_translator::plugin::{self, vmad};
use ximod_translator::script_rules::Rules;
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TranslationMemory;
use ximod_translator::vmad_lock;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn vmad_lines(tm: &TranslationMemory) -> Vec<usize> {
    tm.entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.field_sig == "VMAD")
        .map(|(i, _)| i)
        .collect()
}

/// Every VMAD block of a plugin: (record, FormID, bytes).
fn blocks(bytes: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
    let p = plugin::parse(bytes).unwrap();
    p.records
        .iter()
        .flat_map(|r| {
            r.subrecords
                .iter()
                .filter(|s| s.sig == "VMAD")
                .map(|s| (r.sig.clone(), r.formid, s.data.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// T-C08-01: opened, written back without change, identical; the VMAD
/// lines are Ignoré, « verrouillée : VMAD désactivé ».
#[test]
fn locked_by_default_and_identical_without_change() {
    let path = fixture("cheatterminal.esm");
    let src = std::fs::read(&path).unwrap();
    let all = blocks(&src);
    assert_eq!(all.len(), 296);
    for (rec, fid, b) in &all {
        assert!(vmad::parse(rec, b).is_ok(), "{rec} {fid:08X}");
    }
    let tm = plugin::read_file(&path).unwrap();
    assert!(tm.meta.vmad_unread.is_empty(), "{:?}", tm.meta.vmad_unread);
    let lines = vmad_lines(&tm);
    assert!(lines.len() > 20, "{}", lines.len());
    for &i in &lines {
        let e = &tm.entries[i];
        assert_eq!(status::derive(e), Status::Ignored);
        assert_eq!(vmad_lock::lock_rule(&e.prov), Some("vmad-off"));
        assert!(e.index >= 1);
        assert!(e.comment.contains("vmad-comment"), "{}", e.comment);
    }
    let out = plugin::write_full_esp(&src, &tm).unwrap();
    assert_eq!(out, src, "identical byte for byte");
}

/// T-C08-02: the switch on, the lines without rule are À traduire; a
/// translated and validated property is written, the plugin read again
/// shows it, and every other byte is the same.
#[test]
fn a_translated_property_is_written_and_nothing_else() {
    let path = fixture("cheatterminal.esm");
    let src = std::fs::read(&path).unwrap();
    let mut tm = plugin::read_file(&path).unwrap();
    let rules = Rules::builtin();
    vmad_lock::switch(&mut tm.entries, true, &rules);
    let lines = vmad_lines(&tm);
    let free: Vec<usize> = lines
        .iter()
        .copied()
        .filter(|&i| status::derive(&tm.entries[i]) == Status::ToTranslate)
        .collect();
    let locked = lines.len() - free.len();
    assert!(locked > 0, "file names and words are locked by rule");
    // A locked one by rule says which.
    let l = lines
        .iter()
        .find(|&&i| status::derive(&tm.entries[i]) == Status::Ignored)
        .unwrap();
    assert!(
        matches!(
            vmad_lock::lock_rule(&tm.entries[*l].prov),
            Some("path" | "word" | "key" | "noletter")
        ),
        "{}",
        tm.entries[*l].prov
    );
    // Translate one line (unlocked by rule or not: by hand).
    let i = free.first().copied().unwrap_or(lines[0]);
    let (formid, index, source) = {
        let e = &tm.entries[i];
        (e.formid, e.index, e.source.clone())
    };
    let trans = format!("{source} — traduit é");
    tm.entries[i].trans = trans.clone();
    status::set(&mut tm.entries[i], Status::Validated);
    tm.entries[i].prov.clear();
    let out = plugin::write_full_esp(&src, &tm).unwrap();
    assert_ne!(out, src);
    let back = plugin::extract_strings(&plugin::parse(&out).unwrap(), "x.esm");
    let e = back
        .entries
        .iter()
        .find(|e| e.formid == formid && e.field_sig == "VMAD" && e.index == index)
        .unwrap();
    assert_eq!(e.source, trans);
    // Every other line reads the same.
    let key = |t: &TranslationMemory| -> Vec<(u32, String, i32, String)> {
        t.entries
            .iter()
            .filter(|e| !(e.formid == formid && e.field_sig == "VMAD" && e.index == index))
            .map(|e| (e.formid, e.field_sig.clone(), e.index, e.source.clone()))
            .collect()
    };
    assert_eq!(key(&back), key(&tm));
    // Byte comparison outside the changed record: every other block same.
    let before = blocks(&src);
    let after = blocks(&out);
    assert_eq!(before.len(), after.len());
    let changed: Vec<_> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a != b)
        .map(|(a, _)| a.1)
        .collect();
    assert_eq!(changed, vec![formid]);
}

/// T-C08-05: a block of version 9 is not read, kept as it is, counted.
#[test]
fn an_unknown_version_is_kept() {
    let path = fixture("cheatterminal.esm");
    let mut src = std::fs::read(&path).unwrap();
    // Find the first uncompressed VMAD subrecord in the file and set its
    // version to 9 (a version field is a u16 right after the 6-byte header).
    let at = src
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"VMAD")
        .map(|(i, _)| i)
        .find(|&i| src.len() > i + 8 && src[i + 6] == 6 && src[i + 7] == 0)
        .expect("a VMAD subrecord in clear");
    src[at + 6] = 9;
    let dir = std::env::temp_dir().join(format!("ximod-c08-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("cheatterminal.esm");
    std::fs::write(&p, &src).unwrap();
    let mut tm = plugin::read_file(&p).unwrap();
    assert_eq!(tm.meta.vmad_unread.len(), 1, "{:?}", tm.meta.vmad_unread);
    assert!(tm.meta.vmad_unread[0].1.contains("version 9"));
    // Everything translated and validated: the unread block stays as it is.
    vmad_lock::switch(&mut tm.entries, true, &Rules::builtin());
    for e in tm.entries.iter_mut().filter(|e| e.field_sig == "VMAD") {
        e.trans = format!("{}!", e.source);
        status::set(e, Status::Validated);
    }
    let out = plugin::write_full_esp(&src, &tm).unwrap();
    let forged: Vec<_> = blocks(&out)
        .into_iter()
        .filter(|(_, _, b)| b.first() == Some(&9))
        .collect();
    assert_eq!(forged.len(), 1);
    let orig: Vec<_> = blocks(&src)
        .into_iter()
        .filter(|(_, _, b)| b.first() == Some(&9))
        .collect();
    assert_eq!(forged, orig);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-C08-04 and the Skyrim arrays, on the recette files when present:
/// one line per element of a string array and per string member of a
/// structure; every block read; a write of all the lines translated reads
/// back with the translations, the structure intact.
#[test]
fn recette_plugins_if_present() {
    let root = Path::new("/root/ximod_work/recette_lot3");
    let cases = [
        (
            "Lot 3-1/x_Skipper_112699_0.5.3_2026-08-15T18-57Z_t/skipper.esp",
            "Pages[2]",
        ),
        (
            "Lot 3-2/x_CMRQL_2.41_Installer-39575-2-41-16438757/CMRQL_2_41_Installer/Files/CMRQL.esp",
            "DeadCountGroups[1].description",
        ),
        ("Lot 3-4/x_polish/Data/Open Cities Skyrim.esp", ""),
        (
            "Lot 3-5/CheatTerminal_FO4_1.78/Data/CheatTerminal.esp",
            "dlcName",
        ),
        (
            "Lot 3-5/CheatTerminal_Starfield_1.08/CheatTerminal.esm",
            "FileName",
        ),
        ("Lot 3-6/QuickStart_SE_1.2.2/Quick Start - SE.esp", ""),
    ];
    for (rel, expect) in cases {
        let p = root.join(rel);
        if !p.is_file() {
            eprintln!("skipped: {} absent", p.display());
            continue;
        }
        let src = std::fs::read(&p).unwrap();
        let mut tm = plugin::read_file(&p).unwrap();
        assert!(
            tm.meta.vmad_unread.is_empty(),
            "{rel}: {:?}",
            tm.meta.vmad_unread
        );
        let n = vmad_lines(&tm).len();
        if !expect.is_empty() {
            assert!(
                tm.entries
                    .iter()
                    .any(|e| e.field_sig == "VMAD" && e.comment.contains(expect)),
                "{rel}: {expect}"
            );
        }
        // Unchanged: identical.
        assert_eq!(plugin::write_full_esp(&src, &tm).unwrap(), src, "{rel}");
        // Everything translated.
        vmad_lock::switch(&mut tm.entries, true, &Rules::builtin());
        for e in tm.entries.iter_mut().filter(|e| e.field_sig == "VMAD") {
            e.trans = format!("«{}»", e.source);
            status::set(e, Status::Validated);
        }
        let out = plugin::write_full_esp(&src, &tm).unwrap();
        let back = plugin::extract_strings(&plugin::parse(&out).unwrap(), "x.esm");
        assert!(back.meta.vmad_unread.is_empty(), "{rel}");
        let got: Vec<&str> = back
            .entries
            .iter()
            .filter(|e| e.field_sig == "VMAD")
            .map(|e| e.source.as_str())
            .collect();
        assert_eq!(got.len(), n, "{rel}");
        assert!(got.iter().all(|s| s.starts_with('«')), "{rel}");
        eprintln!("{rel}: {n} VMAD lines");
    }
}

/// T-C08-03 (the lines' side): the switch as an undoable operation — on,
/// the rule locks say why; undone, the file lock is back exactly; redone,
/// the same; the state follows.
#[test]
fn the_switch_is_undoable() {
    use ximod_translator::undo::{OpKind, UndoStack, record};
    let mut tm = plugin::read_file(&fixture("cheatterminal.esm")).unwrap();
    let rules = Rules::builtin();
    let mut undo = UndoStack::new(50);
    let before = tm.entries.clone();
    record(&mut undo, &mut tm.entries, OpKind::VmadOn, None, |es| {
        vmad_lock::switch(es, true, &rules)
    });
    assert_eq!(vmad_lock::state(&tm.entries), Some(true));
    let locked: Vec<&str> = tm
        .entries
        .iter()
        .filter(|e| e.field_sig == "VMAD" && status::derive(e) == Status::Ignored)
        .map(|e| e.prov.as_str())
        .collect();
    assert!(!locked.is_empty());
    assert!(locked.iter().all(|p| p.starts_with("lock:")), "{locked:?}");
    let after = tm.entries.clone();
    undo.undo(&mut tm.entries).unwrap();
    assert_eq!(tm.entries, before);
    assert_eq!(vmad_lock::state(&tm.entries), Some(false));
    undo.redo(&mut tm.entries).unwrap();
    assert_eq!(tm.entries, after);
}
