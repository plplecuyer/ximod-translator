//! E-06 on a real plugin: the export over the open plugin keeps the
//! original in backups/, the copies rotate, the option and the forced case,
//! a backups folder that cannot be written (T-E06-01 to T-E06-04).

use std::path::{Path, PathBuf};

use ximod_translator::backup::{self, ExportWriteError};
use ximod_translator::plugin;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp")
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ximod-e06-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The translated plugin of the fixture, as Exporter la traduction… writes
/// it in Full plugin.
fn translated(src: &[u8]) -> Vec<u8> {
    let mut tm = plugin::read_file(&fixture()).unwrap();
    for e in tm.entries.iter_mut().filter(|e| !e.unused) {
        e.trans = format!("{} (fr)", e.source);
        e.set_translated_flags(true);
    }
    plugin::write_delocalized_esp(src, &tm, None).unwrap().0
}

/// T-E06-01: Full plugin on the open plugin's own path.
#[test]
fn open_plugin_overwritten_keeps_its_original() {
    let d = dir("self");
    let open = d.join("MonMod.esp");
    std::fs::copy(fixture(), &open).unwrap();
    let original = std::fs::read(&open).unwrap();
    let bytes = translated(&original);
    assert_ne!(bytes, original);
    // The option off: the open plugin is kept all the same.
    let forced = backup::same_file(&open, &d.join(".").join("MonMod.esp"));
    assert!(forced);
    let kept = backup::write_export(&open, &bytes, false, forced, 5)
        .unwrap()
        .unwrap();
    assert_eq!(kept.parent().unwrap(), d.join("backups"));
    let name = kept.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with("MonMod_") && name.ends_with(".esp"),
        "{name}"
    );
    assert_eq!(name.len(), "MonMod_20261025-161203.esp".len());
    assert_eq!(std::fs::read(&kept).unwrap(), original);
    assert_eq!(std::fs::read(&open).unwrap(), bytes);
    // The new plugin reads back.
    assert!(plugin::read_file(&open).is_ok());
    let _ = std::fs::remove_dir_all(&d);
}

/// T-E06-02: six exports with N = 5 leave five copies; another plugin's
/// copies are not touched.
#[test]
fn six_exports_keep_five_copies() {
    let d = dir("rot");
    let out = d.join("MonMod.esp");
    std::fs::write(&out, b"v0").unwrap();
    std::fs::create_dir_all(d.join("backups")).unwrap();
    let other = d.join("backups").join("MonMod_Patch_20200101-000000.esp");
    std::fs::write(&other, b"patch").unwrap();
    let mut kept = Vec::new();
    for i in 1..=6 {
        let k = backup::write_export(&out, format!("v{i}").as_bytes(), true, false, 5)
            .unwrap()
            .unwrap();
        kept.push(k);
    }
    let copies = backup::original_copies(&out);
    assert_eq!(copies.len(), 5);
    // The oldest (the content « v0 ») is gone; the others are in order.
    assert!(!kept[0].exists());
    let contents: Vec<Vec<u8>> = copies.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(
        contents,
        (1..=5)
            .map(|i| format!("v{i}").into_bytes())
            .collect::<Vec<_>>()
    );
    assert!(other.exists());
    assert_eq!(std::fs::read(&out).unwrap(), b"v6");
    let _ = std::fs::remove_dir_all(&d);
}

/// T-E06-03: the option off, a previous output is overwritten without a
/// copy; a new path copies nothing.
#[test]
fn option_off_and_new_path() {
    let d = dir("off");
    let out = d.join("MonMod_patch.esp");
    std::fs::write(&out, b"old").unwrap();
    assert!(
        backup::write_export(&out, b"new", false, false, 5)
            .unwrap()
            .is_none()
    );
    assert!(!d.join("backups").exists());
    let fresh = d.join("Nouveau.esp");
    assert!(
        backup::write_export(&fresh, b"x", true, false, 5)
            .unwrap()
            .is_none()
    );
    assert!(!d.join("backups").exists());
    // A text file written by its own writer: the copy before.
    let txt = d.join("menu_french.txt");
    std::fs::write(&txt, "a=b").unwrap();
    let k = backup::keep_before_export(&txt, true, false)
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read_to_string(k).unwrap(), "a=b");
    let _ = std::fs::remove_dir_all(&d);
}

/// T-E06-04: backups/ cannot be made (a file has its name): the export
/// stops, the existing file is intact.
#[test]
fn backups_not_writable_stops_the_export() {
    let d = dir("ro");
    let out = d.join("MonMod.esp");
    std::fs::write(&out, b"original").unwrap();
    std::fs::write(d.join("backups"), b"not a folder").unwrap();
    match backup::write_export(&out, b"new", true, false, 5) {
        Err(ExportWriteError::Keep(_)) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&out).unwrap(), b"original");
    let _ = std::fs::remove_dir_all(&d);
}
