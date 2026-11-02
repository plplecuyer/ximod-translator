//! Exercises the ESP/ESM parser on a synthetic, minimal but well-formed
//! plugin (TES4-family layout with a GRUP, a compressed-free MESG record, an
//! EDID, a FULL and two ITXT subrecords).

use std::path::Path;

use ximod_translator::plugin;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn subrecord(sig: &str, data: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u16).to_le_bytes());
    v.extend_from_slice(data);
    v
}

fn record(sig: &str, formid: u32, flags: u32, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes()); // dataSize
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&formid.to_le_bytes());
    v.extend_from_slice(&[0u8; 8]); // timestamp / version / unknown
    v.extend_from_slice(subs);
    v
}

fn grup(label: &str, body: &[u8]) -> Vec<u8> {
    let total = (24 + body.len()) as u32;
    let mut v = b"GRUP".to_vec();
    v.extend_from_slice(&total.to_le_bytes()); // groupSize incl. header
    v.extend_from_slice(label.as_bytes()); // label (record type for top group)
    v.extend_from_slice(&0i32.to_le_bytes()); // groupType = 0 (top)
    v.extend_from_slice(&[0u8; 8]); // stamp / version / unknown
    v.extend_from_slice(body);
    v
}

fn zstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

// --- TES3 (Morrowind) builders: 16-byte record header, 4-byte subrecord size,
//     no GRUP, no FormID. ---

fn tes3_sub(sig: &str, data: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u32).to_le_bytes()); // u32 size
    v.extend_from_slice(data);
    v
}

fn tes3_record(sig: &str, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes()); // dataSize
    v.extend_from_slice(&0u32.to_le_bytes()); // header1 (unused)
    v.extend_from_slice(&0u32.to_le_bytes()); // flags
    v.extend_from_slice(subs);
    v
}

/// A 300-byte HEDR: version, file type, author[32], description[256], count.
fn tes3_hedr(author: &str, desc: &str) -> Vec<u8> {
    let mut d = vec![0u8; 300];
    d[0..4].copy_from_slice(&1.3f32.to_le_bytes());
    let a = author.as_bytes();
    d[8..8 + a.len()].copy_from_slice(a);
    let de = desc.as_bytes();
    d[40..40 + de.len()].copy_from_slice(de);
    d
}

#[test]
fn parses_synthetic_tes3_with_cell_name_nuance() {
    use ximod_translator::games::Game;

    // TES3 header with a master and author/description in HEDR.
    let hedr = tes3_hedr("Caeris", "A quest mod");
    let mut hdr_subs = tes3_sub("HEDR", &hedr);
    hdr_subs.extend(tes3_sub("MAST", &zstr("Morrowind.esm")));
    hdr_subs.extend(tes3_sub("DATA", &[0u8; 8]));
    let tes3 = tes3_record("TES3", &hdr_subs);

    // A BOOK: NAME (id) + FNAM (title) + TEXT (body).
    let mut book = Vec::new();
    book.extend(tes3_sub("NAME", &zstr("thief_book_01")));
    book.extend(tes3_sub("FNAM", &zstr("Guild Secrets")));
    book.extend(tes3_sub("TEXT", &zstr("Chapter one...")));
    let book = tes3_record("BOOK", &book);

    // A CELL: leading NAME (cell name) then a reference block (FRMR + NAME id).
    // Only the leading NAME must be extracted.
    let mut cell = Vec::new();
    cell.extend(tes3_sub("NAME", &zstr("Balmora, Guild of Thieves")));
    cell.extend(tes3_sub("DATA", &[0u8; 12]));
    cell.extend(tes3_sub("FRMR", &1u32.to_le_bytes()));
    cell.extend(tes3_sub("NAME", &zstr("ref_object_id"))); // reference, NOT a name
    let cell = tes3_record("CELL", &cell);

    // A dialogue topic and one response.
    let dial = tes3_record("DIAL", &tes3_sub("NAME", &zstr("Background")));
    let mut info = Vec::new();
    info.extend(tes3_sub("INAM", &zstr("info_0001")));
    info.extend(tes3_sub("NAME", &zstr("I know of the Thieves Guild.")));
    let info = tes3_record("INFO", &info);

    let mut file = tes3;
    file.extend(book);
    file.extend(cell);
    file.extend(dial);
    file.extend(info);

    let parsed = plugin::parse(&file).unwrap();
    assert_eq!(parsed.game, Game::Morrowind);
    assert_eq!(parsed.header_len, 16);
    assert_eq!(parsed.masters, vec!["Morrowind.esm".to_string()]);
    assert_eq!(parsed.author, "Caeris");

    let tm = plugin::extract_strings(&parsed, "Test.esp");
    assert_eq!(tm.meta.game, "Morrowind");

    let has = |rec: &str, field: &str, src: &str| {
        tm.entries
            .iter()
            .any(|e| e.rec_sig == rec && e.field_sig == field && e.source == src)
    };
    assert!(has("TES3", "CNAM", "Caeris"), "author from HEDR");
    assert!(has("TES3", "SNAM", "A quest mod"), "description from HEDR");
    assert!(has("BOOK", "FNAM", "Guild Secrets"));
    assert!(has("BOOK", "TEXT", "Chapter one..."));
    assert!(has("CELL", "NAME", "Balmora, Guild of Thieves"));
    assert!(has("DIAL", "NAME", "Background"));
    assert!(has("INFO", "NAME", "I know of the Thieves Guild."));

    // The reference-block NAME must NOT be surfaced as a cell name.
    assert!(
        !tm.entries.iter().any(|e| e.source == "ref_object_id"),
        "a CELL reference id must not be extracted as a cell name"
    );
    // No FormID in TES3.
    assert!(tm.entries.iter().all(|e| e.formid == 0));
    // The INFO response is keyed by its INAM id, not by its text.
    let resp = tm.entries.iter().find(|e| e.rec_sig == "INFO").unwrap();
    assert_eq!(resp.edid, "info_0001");
}

/// Real Morrowind mod (Thieves Guild Quest Revisions) cross-checked against its
/// ESP-ESM Translator dictionary — xTranslator does not support Morrowind, so
/// the EET is the authoritative reference. Extraction must match the EET's
/// non-script fields exactly (script text is deferred to V2, like elsewhere).
#[test]
fn morrowind_matches_eet_dictionary() {
    use std::collections::HashSet;
    use ximod_translator::tm;

    let esp = fixture("morrowind_thieves.esp");
    let eet = fixture("morrowind_thieves.eet");
    if !esp.exists() || !eet.exists() {
        eprintln!("skipping: Morrowind fixtures not present");
        return;
    }

    let tm_plug = plugin::read_file(&esp).unwrap();
    assert_eq!(tm_plug.meta.game, "Morrowind");
    assert_eq!(
        tm_plug.meta.masters,
        vec![
            "Morrowind.esm".to_string(),
            "Tribunal.esm".to_string(),
            "Bloodmoon.esm".to_string()
        ]
    );
    // Field-for-field parity with the EET's non-script pairs established by
    // cross-analysis: 361 strings (dominated by the 267 INFO responses).
    assert_eq!(tm_plug.entries.len(), 361);
    assert_eq!(
        tm_plug
            .entries
            .iter()
            .filter(|e| e.rec_sig == "INFO" && e.field_sig == "NAME")
            .count(),
        267
    );
    // The CELL leading-name nuance: exactly the 4 named cells, not the many
    // reference ids that also use a NAME subrecord.
    assert_eq!(
        tm_plug
            .entries
            .iter()
            .filter(|e| e.rec_sig == "CELL" && e.field_sig == "NAME")
            .count(),
        4
    );

    // Every extracted string must exist in the EET dictionary (by rec/field/src).
    let dict = tm::load(&eet).unwrap();
    let key = |e: &ximod_translator::tm::TmEntry| {
        (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone())
    };
    let dict_keys: HashSet<_> = dict.entries.iter().map(&key).collect();
    let missing = tm_plug
        .entries
        .iter()
        .filter(|e| !dict_keys.contains(&key(e)))
        .count();
    assert_eq!(
        missing, 0,
        "every extracted string must be in the EET dictionary"
    );
}

#[test]
fn extracts_full_and_indexed_itxt() {
    // MESG record with EDID, FULL and two ITXT fields.
    let mut subs = Vec::new();
    subs.extend(subrecord("EDID", &zstr("MyMsg")));
    subs.extend(subrecord("FULL", &zstr("Hello")));
    subs.extend(subrecord("ITXT", &zstr("One")));
    subs.extend(subrecord("ITXT", &zstr("Two")));
    let mesg = record("MESG", 0x0000_0ABC, 0, &subs);

    // TES4 header record (non-localized) + a top GRUP of MESG records.
    let tes4 = record("TES4", 0, 0, &subrecord("HEDR", &[0u8; 12]));
    let mut file = tes4;
    file.extend(grup("MESG", &mesg));

    let parsed = plugin::parse(&file).unwrap();
    assert!(!parsed.localized);

    let tm = plugin::extract_strings(&parsed, "Test.esp");
    // FULL + 2 ITXT = 3 translatable strings.
    assert_eq!(tm.entries.len(), 3);

    let full = tm.entries.iter().find(|e| e.field_sig == "FULL").unwrap();
    assert_eq!(full.source, "Hello");
    assert_eq!(full.edid, "MyMsg");
    assert_eq!(full.formid, 0x0000_0ABC);
    assert_eq!(full.rec_sig, "MESG");

    let itxt: Vec<(&str, i32)> = tm
        .entries
        .iter()
        .filter(|e| e.field_sig == "ITXT")
        .map(|e| (e.source.as_str(), e.index))
        .collect();
    assert_eq!(itxt, vec![("One", 1), ("Two", 2)]);

    assert_eq!(tm.meta.plugin_name, "Test.esp");
}

/// Regression test on a real Starfield master (CheatTerminal.esm). Its
/// extractable strings must match, field for field, the authoritative
/// xTranslator dictionary for the same mod (8716 strings, dominated by the
/// TMLM terminal menus).
#[test]
fn extracts_real_starfield_esm() {
    let path = fixture("cheatterminal.esm");
    if !path.exists() {
        eprintln!("skipping: {} not present", path.display());
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let parsed = plugin::parse(&data).unwrap();
    assert_eq!(parsed.masters, vec!["Starfield.esm".to_string()]);
    assert!(!parsed.localized, "this plugin stores strings inline");

    let tm = plugin::extract_strings(&parsed, "CheatTerminal.esm");
    assert_eq!(tm.meta.game, "Starfield");
    // Matches xTranslator's count for this mod exactly, plus the scripts'
    // string properties (C-08, locked by default).
    let vmad = tm.entries.iter().filter(|e| e.field_sig == "VMAD").count();
    assert_eq!(vmad, 37);
    assert_eq!(tm.entries.len() - vmad, 8716);

    let itxt = tm
        .entries
        .iter()
        .filter(|e| e.rec_sig == "TMLM" && e.field_sig == "ITXT")
        .count();
    assert_eq!(itxt, 7249, "TMLM terminal-menu item text");

    // No empty sources should survive extraction on a non-localized plugin.
    assert!(tm.entries.iter().all(|e| !e.source.is_empty()));
}

/// Fallout New Vegas mod (La Longue Carabine) — a small, fully-translated pair
/// bundled as a fixture. Locks in the FNV engine (24-byte headers), the IMOD
/// (item-mod) record, and exact parity with its xTranslator dictionary.
#[test]
fn fallout_nv_matches_xtranslator_dictionary() {
    use std::collections::HashSet;
    use ximod_translator::tm;

    let esp = fixture("lalongue.esp");
    let sst = fixture("lalongue.sst");
    if !esp.exists() || !sst.exists() {
        eprintln!("skipping: FNV fixtures not present");
        return;
    }

    let tm_plug = plugin::read_file(&esp).unwrap();
    assert_eq!(tm_plug.meta.masters, vec!["FalloutNV.esm".to_string()]);
    assert_eq!(tm_plug.meta.game, "FalloutNV");

    let dict = tm::load(&sst).unwrap();

    let key = |e: &ximod_translator::tm::TmEntry| {
        (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone())
    };
    let plug_keys: HashSet<_> = tm_plug.entries.iter().map(&key).collect();
    let dict_keys: HashSet<_> = dict.entries.iter().map(&key).collect();

    // Exact parity, both directions.
    assert_eq!(
        plug_keys, dict_keys,
        "FNV extraction must match the dictionary"
    );
    // Including the Fallout-specific IMOD record.
    assert!(
        tm_plug
            .entries
            .iter()
            .any(|e| e.rec_sig == "IMOD" && e.field_sig == "FULL")
    );
}

/// Merging the FNV dictionary onto the plugin must match every string and,
/// crucially, must NOT cross-match the two multi-instance MESG:ITXT fields
/// (the bug that motivated 0-based↔1-based index normalization).
#[test]
fn merge_fnv_dictionary_onto_plugin() {
    use ximod_translator::tm::{
        self,
        apply::{MergeOptions, merge},
    };

    let esp = fixture("lalongue.esp");
    let sst = fixture("lalongue.sst");
    if !esp.exists() || !sst.exists() {
        eprintln!("skipping: FNV fixtures not present");
        return;
    }
    let mut plug = plugin::read_file(&esp).unwrap();
    let dict = tm::load(&sst).unwrap();
    let report = merge(&mut plug, &dict, MergeOptions::default());

    assert_eq!(report.total, 11);
    assert_eq!(report.matched, 11);
    assert_eq!(report.unmatched(), 0);

    // Each MESG:ITXT must carry ITS OWN text, not the sibling's.
    for e in plug
        .entries
        .iter()
        .filter(|e| e.rec_sig == "MESG" && e.field_sig == "ITXT")
    {
        assert_eq!(e.trans, e.source, "ITXT instance must not be cross-matched");
    }
    let attach = plug
        .entries
        .iter()
        .find(|e| e.source == "Attach/Detach Scope")
        .unwrap();
    assert_eq!(attach.trans, "Attach/Detach Scope");
}

/// Cross-validation on a real Skyrim mod (Requiem) against its populated
/// xTranslator dictionary. The big fixtures are NOT shipped (they would bloat
/// the archive); drop `requiem.esp` and `requiem.sst` into `tests/fixtures/` to
/// enable it. It asserts that virtually every string the plugin yields is
/// present in the authoritative dictionary, and that no field is under-covered.
#[test]
fn skyrim_requiem_matches_xtranslator_dictionary() {
    use std::collections::{HashMap, HashSet};
    use ximod_translator::tm;

    let esp = fixture("requiem.esp");
    let sst = fixture("requiem.sst");
    if !esp.exists() || !sst.exists() {
        eprintln!("skipping: drop requiem.esp and requiem.sst into tests/fixtures/ to run");
        return;
    }

    let plug = plugin::read_file(&esp).unwrap();
    let dict = tm::load(&sst).unwrap();

    let key = |e: &ximod_translator::tm::TmEntry| {
        (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone())
    };
    let dict_keys: HashSet<_> = dict.entries.iter().map(&key).collect();
    let plug_keys: HashSet<_> = plug.entries.iter().map(&key).collect();

    let common = plug_keys.intersection(&dict_keys).count();
    let ratio = common as f64 / plug_keys.len() as f64;
    assert!(
        ratio > 0.98,
        "plugin↔dictionary coverage too low: {:.1}%",
        ratio * 100.0
    );

    // No (record:field) that the dictionary translates should be missed entirely.
    let mut plug_fields: HashMap<(String, String), usize> = HashMap::new();
    for e in &plug.entries {
        *plug_fields
            .entry((e.rec_sig.clone(), e.field_sig.clone()))
            .or_default() += 1;
    }
    let mut dict_fields: HashMap<(String, String), usize> = HashMap::new();
    for e in &dict.entries {
        *dict_fields
            .entry((e.rec_sig.clone(), e.field_sig.clone()))
            .or_default() += 1;
    }
    for (field, &n) in &dict_fields {
        if n > 5 {
            assert!(
                plug_fields.get(field).copied().unwrap_or(0) > 0,
                "field {field:?} present {n}× in dictionary but never extracted"
            );
        }
    }
}

/// Parity: the memory-mapped streaming reader (`read_file`, which skips
/// non-translatable record types) must produce exactly the same entries and
/// metadata as the classic `parse` + `extract_strings` path, across every format
/// regime (Oblivion 20-byte header, Morrowind TES3, FNV, Skyrim LE, Starfield).
#[test]
fn streaming_reader_matches_classic_extraction() {
    let fixtures = [
        "oblivion_magpies.esp",
        "morrowind_thieves.esp",
        "lalongue.esp",
        "skyrimle_riverwood.esp",
        "cheatterminal.esm",
    ];
    for name in fixtures {
        let path = fixture(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("read {name}"));
        let classic = plugin::extract_strings(&plugin::parse(&bytes).unwrap(), name);
        let streamed = plugin::read_file(&path).unwrap();

        assert_eq!(
            streamed.meta.game, classic.meta.game,
            "game mismatch on {name}"
        );
        assert_eq!(
            streamed.meta.masters, classic.meta.masters,
            "masters mismatch on {name}"
        );
        assert_eq!(
            streamed.entries.len(),
            classic.entries.len(),
            "entry count mismatch on {name}"
        );
        for (i, (a, b)) in streamed
            .entries
            .iter()
            .zip(classic.entries.iter())
            .enumerate()
        {
            assert_eq!(a.formid, b.formid, "formid @{i} on {name}");
            assert_eq!(a.edid, b.edid, "edid @{i} on {name}");
            assert_eq!(a.rec_sig, b.rec_sig, "rec_sig @{i} on {name}");
            assert_eq!(a.field_sig, b.field_sig, "field_sig @{i} on {name}");
            assert_eq!(a.source, b.source, "source @{i} on {name}");
            assert_eq!(a.idstexte, b.idstexte, "idstexte @{i} on {name}");
            assert_eq!(a.index, b.index, "index @{i} on {name}");
        }
        assert!(
            !streamed.entries.is_empty(),
            "no entries extracted from {name}"
        );
    }
}
