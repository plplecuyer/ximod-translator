//! B-12 on a real plugin: the identity written in the header of the mod's
//! dictionary, recognised or not after the plugin changed, a dictionary of
//! before B-12, a renamed plugin's dictionary moved with its companions
//! (T-B12-01 to T-B12-04).

use std::path::{Path, PathBuf};

use ximod_translator::backup;
use ximod_translator::basedb::{self, BaseRow};
use ximod_translator::plugin_id::{self, Ident};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp")
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ximod-b12-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rows() -> Vec<BaseRow> {
    vec![BaseRow {
        rec: "CONT".into(),
        field: "FULL".into(),
        formid: 0x1234,
        edid: "Chest".into(),
        source: "Chest".into(),
        trans: "Coffre".into(),
        ..Default::default()
    }]
}

/// T-B12-01 and T-B12-02: written for the plugin, recognised while it is
/// unchanged, not after it changed.
#[test]
fn identity_written_and_compared() {
    let d = dir("id");
    let plugin = d.join("MonMod.esp");
    std::fs::copy(fixture(), &plugin).unwrap();
    let id = Ident {
        written: 1_790_000_000,
        ..plugin_id::of_file(&plugin).unwrap()
    };
    assert_eq!(id.plugin, "MonMod.esp");
    assert_eq!(id.size, std::fs::metadata(&plugin).unwrap().len());
    let dict = d.join("MonMod_ENG-FRA.ximdb");
    basedb::write_ximdb_ident(rows(), Vec::new(), "fra", &dict, 0, Some(&id)).unwrap();
    // The header is at the start of the file, the rows still read.
    let text = std::fs::read_to_string(&dict).unwrap();
    assert!(text.starts_with("{\"v\":4,\"lang\":\"fra\",\"plugin\":\"MonMod.esp\""));
    let back = basedb::peek_ident(&dict).unwrap();
    assert_eq!(back, id);
    assert_eq!(basedb::read_rows_and_old(&dict).unwrap().1.len(), 1);
    // Unchanged: recognised.
    assert!(plugin_id::of_file(&plugin).unwrap().same_content(&back));
    // Another version: one byte more.
    let mut bytes = std::fs::read(&plugin).unwrap();
    bytes.push(0);
    std::fs::write(&plugin, &bytes).unwrap();
    let now = plugin_id::of_file(&plugin).unwrap();
    assert!(!now.same_content(&back));
    assert_ne!(now.crc_hex(), back.crc_hex());
    let _ = std::fs::remove_dir_all(&d);
}

/// T-B12-04: a dictionary written before B-12 has no identity; a write
/// that does not give one keeps the one already there.
#[test]
fn dictionary_without_identity() {
    let d = dir("old");
    let dict = d.join("MonMod_ENG-FRA.ximdb");
    basedb::write_ximdb(rows(), "fra", &dict).unwrap();
    assert!(basedb::peek_ident(&dict).is_none());
    basedb::write_ximdb_full(rows(), Vec::new(), "fra", &dict, 0).unwrap();
    assert!(basedb::peek_ident(&dict).is_none());
    let id = Ident {
        plugin: "MonMod.esp".into(),
        size: 42,
        crc: 0xDEAD_BEEF,
        written: 0,
    };
    basedb::write_ximdb_ident(rows(), Vec::new(), "fra", &dict, 0, Some(&id)).unwrap();
    // The dictionary editor rewrites it: the identity stays.
    basedb::write_ximdb_full(rows(), Vec::new(), "fra", &dict, 0).unwrap();
    assert_eq!(basedb::peek_ident(&dict), Some(id));
    let _ = std::fs::remove_dir_all(&d);
}

/// T-B12-03: the dictionary of a renamed plugin moves with its history
/// and its copies.
#[test]
fn renamed_plugin_dictionary_moves_with_companions() {
    let d = dir("ren");
    let old = d.join("MonMod_ENG-FRA.ximdb");
    let new = d.join("MonMod Redux_ENG-FRA.ximdb");
    basedb::write_ximdb(rows(), "fra", &old).unwrap();
    std::fs::write(old.with_extension("ximhist"), "{}\n").unwrap();
    std::fs::create_dir_all(d.join("backups")).unwrap();
    let copy = d
        .join("backups")
        .join("MonMod_ENG-FRA_20261020-101010.ximdb");
    std::fs::write(&copy, "{}").unwrap();
    backup::rename_with_companions(&old, &new).unwrap();
    assert!(!old.exists() && new.is_file());
    assert!(new.with_extension("ximhist").is_file());
    assert!(!copy.exists());
    assert!(
        d.join("backups")
            .join("MonMod Redux_ENG-FRA_20261020-101010.ximdb")
            .is_file()
    );
    // Never over an existing dictionary.
    basedb::write_ximdb(rows(), "fra", &old).unwrap();
    assert!(backup::rename_with_companions(&old, &new).is_err());
    let _ = std::fs::remove_dir_all(&d);
}
