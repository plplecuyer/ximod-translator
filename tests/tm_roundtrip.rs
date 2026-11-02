//! Integration tests for the translation-memory engine, run against real files
//! exported by ESP-ESM Translator and xTranslator.

use std::path::Path;

use ximod_translator::tm::{self, eet, xml};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn eet_binary_roundtrip_is_byte_exact() {
    let bytes = std::fs::read(fixture("follower_chantez.eet")).unwrap();
    let tm = eet::read_bytes(&bytes).unwrap();
    assert!(!tm.entries.is_empty(), "EET should decode entries");
    let (out, mode) = eet::write_bytes(&tm);
    assert_eq!(mode, eet::EetWriteMode::Exact);
    assert_eq!(out, bytes, "EET round-trip must be byte-exact");
}

#[test]
fn eet_binary_roundtrip_second_file() {
    let bytes = std::fs::read(fixture("follower_chantez2.eet")).unwrap();
    let tm = eet::read_bytes(&bytes).unwrap();
    let (out, mode) = eet::write_bytes(&tm);
    assert_eq!(mode, eet::EetWriteMode::Exact);
    assert_eq!(out, bytes);
}

#[test]
fn eetxml_roundtrip_preserves_entries() {
    let src = std::fs::read_to_string(fixture("infirmary_eet.xml")).unwrap();
    let a = xml::read_eetxml(&src).unwrap();
    let out = xml::write_eetxml(&a);
    let b = xml::read_eetxml(&out).unwrap();
    assert_eq!(a.entries.len(), b.entries.len());
    for (x, y) in a.entries.iter().zip(b.entries.iter()) {
        assert_eq!(x.source, y.source);
        assert_eq!(x.trans, y.trans);
        assert_eq!(x.edid, y.edid);
        assert_eq!(x.rec_sig, y.rec_sig);
        assert_eq!(x.field_sig, y.field_sig);
        assert_eq!(x.index, y.index);
        assert_eq!(x.formid, y.formid);
        assert_eq!(x.status_eet, y.status_eet);
    }
}

#[test]
fn sstxml_roundtrip_preserves_entries_and_ids() {
    let src = std::fs::read_to_string(fixture("infirmary_xt.xml")).unwrap();
    let a = xml::read_sstxml(&src).unwrap();
    let out = xml::write_sstxml(&a);
    let b = xml::read_sstxml(&out).unwrap();
    assert_eq!(a.entries.len(), b.entries.len());
    assert_eq!(a.meta.addon, b.meta.addon);
    for (x, y) in a.entries.iter().zip(b.entries.iter()) {
        assert_eq!(x.source, y.source);
        assert_eq!(x.trans, y.trans);
        assert_eq!(x.edid, y.edid);
        assert_eq!(x.rec_sig, y.rec_sig);
        assert_eq!(x.field_sig, y.field_sig);
        assert_eq!(x.index, y.index);
    }
    // The MESG:ITXT group has three instances with indices 1,2,3.
    let itxt: Vec<i32> = a
        .entries
        .iter()
        .filter(|e| e.rec_sig == "MESG" && e.field_sig == "ITXT")
        .map(|e| e.index)
        .collect();
    assert_eq!(itxt, vec![1, 2, 3]);
}

#[test]
fn cross_eetxml_to_sstxml_reconstructs_id_idmax() {
    let src = std::fs::read_to_string(fixture("infirmary_eet.xml")).unwrap();
    let a = xml::read_eetxml(&src).unwrap();
    let sst_xml = xml::write_sstxml(&a);
    // The multi-instance ITXT group must carry id 0,1,2 with idMax 2.
    assert!(sst_xml.contains(r#"<REC id="0" idMax="2">MESG:ITXT</REC>"#));
    assert!(sst_xml.contains(r#"<REC id="2" idMax="2">MESG:ITXT</REC>"#));
    // Single-instance fields must not carry id/idMax.
    assert!(sst_xml.contains("<REC>FLST:FULL</REC>"));

    // And re-reading maps the ids back to indices 1,2,3.
    let b = xml::read_sstxml(&sst_xml).unwrap();
    let itxt: Vec<i32> = b
        .entries
        .iter()
        .filter(|e| e.rec_sig == "MESG" && e.field_sig == "ITXT")
        .map(|e| e.index)
        .collect();
    assert_eq!(itxt, vec![1, 2, 3]);
}

#[test]
fn sst_binary_roundtrip_is_byte_exact() {
    // Guards the 0-based↔1-based index normalization: read (+1) then write (−1)
    // must reproduce the original bytes exactly.
    let bytes = std::fs::read(fixture("lalongue.sst")).unwrap();
    let tm = ximod_translator::tm::sst::read_bytes(&bytes).unwrap();
    let out = ximod_translator::tm::sst::write_bytes(&tm);
    assert_eq!(out, bytes, "SST round-trip must be byte-exact");
}

#[test]
fn pivot_json_roundtrip() {
    let bytes = std::fs::read(fixture("follower_chantez.eet")).unwrap();
    let a = eet::read_bytes(&bytes).unwrap();
    let json = serde_json::to_string(&a).unwrap();
    let b = tm::load_json_or_dsd(&json).unwrap();
    assert_eq!(a.entries.len(), b.entries.len());
    // From the pivot we can rebuild a byte-exact EET (raw bytes preserved).
    let (out, mode) = eet::write_bytes(&b);
    assert_eq!(mode, eet::EetWriteMode::Exact);
    assert_eq!(out, bytes);
}
