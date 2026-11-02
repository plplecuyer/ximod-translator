//! End-to-end localized-plugin flow: a plugin whose fields hold string ids, its
//! external STRINGS resolved into sources, translated, and written back out as
//! translated STRINGS files (output mode 1).

use ximod_translator::plugin::{self, StringsBundle, build_strings_files};
use ximod_translator::tm::strings::{self, StringsKind};

/// The scanner locates STRINGS regardless of case or code form (French vs fr) and
/// picks the preferred language when several are present.
#[test]
fn from_plugin_path_scans_case_insensitively() {
    let dir = std::env::temp_dir().join("ximod_strings_scan_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Two languages, deliberately mixed case; English lowercase (BSA-extracted).
    std::fs::write(
        dir.join("MyMod_French.STRINGS"),
        strings::write(&[(1u32, "Épée".into())], StringsKind::Strings, true),
    )
    .unwrap();
    std::fs::write(
        dir.join("mymod_english.strings"),
        strings::write(&[(1u32, "Sword".into())], StringsKind::Strings, true),
    )
    .unwrap();

    let plugin_path = dir.join("MyMod.esp"); // need not exist; only stem/dir used
    // Prefer French, given as the ISO 639-3 code the app stores.
    let bundle = StringsBundle::from_plugin_path(&plugin_path, "fra", true).unwrap();
    assert_eq!(bundle.get(1), Some("Épée"), "preferred language chosen");

    // Prefer English via its 639-1 code.
    let bundle_en = StringsBundle::from_plugin_path(&plugin_path, "en", true).unwrap();
    assert_eq!(bundle_en.get(1), Some("Sword"));

    let _ = std::fs::remove_dir_all(&dir);
}

fn sub_u16(sig: &str, data: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u16).to_le_bytes());
    v.extend_from_slice(data);
    v
}
fn zstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}
fn rec24(sig: &str, formid: u32, flags: u32, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&formid.to_le_bytes());
    v.extend_from_slice(&[0u8; 4]);
    v.extend_from_slice(&44u16.to_le_bytes()); // form version 44 = Skyrim SE
    v.extend_from_slice(&[0u8; 2]);
    v.extend_from_slice(subs);
    v
}
fn top_grup(label: &str, body: &[u8]) -> Vec<u8> {
    let mut v = b"GRUP".to_vec();
    v.extend_from_slice(&((24 + body.len()) as u32).to_le_bytes());
    v.extend_from_slice(label.as_bytes());
    v.extend_from_slice(&0i32.to_le_bytes());
    v.extend_from_slice(&[0u8; 8]);
    v.extend_from_slice(body);
    v
}

#[test]
fn localized_resolve_translate_and_emit_strings() {
    const LOCALIZED: u32 = 0x0000_0080;

    // A localized SSE plugin: WEAP with FULL (id 1) and DESC (id 2), 4-byte ids.
    let tes4 = {
        let mut s = sub_u16("HEDR", &[0u8; 12]);
        s.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24("TES4", 0, LOCALIZED, &s)
    };
    let weap = {
        let mut s = sub_u16("EDID", &zstr("IronSword"));
        s.extend(sub_u16("FULL", &1u32.to_le_bytes()));
        s.extend(sub_u16("DESC", &2u32.to_le_bytes()));
        rec24("WEAP", 0x0100_0801, 0, &s)
    };
    let mut src = tes4;
    src.extend(top_grup("WEAP", &weap));

    // Its English STRINGS: FULL in .STRINGS, DESC in .DLSTRINGS.
    let strings_en = strings::write(&[(1, "Iron Sword".into())], StringsKind::Strings, true);
    let dlstrings_en = strings::write(
        &[(2, "A basic sword.".into())],
        StringsKind::DlStrings,
        true,
    );

    // Parse the plugin — localized, so sources start empty (just ids).
    let parsed = plugin::parse(&src).unwrap();
    assert!(parsed.localized);
    let mut tm = plugin::extract_strings(&parsed, "MyMod.esp");
    assert_eq!(tm.entries.len(), 2);
    assert!(
        tm.entries
            .iter()
            .all(|e| e.source.is_empty() && e.idstexte >= 0)
    );

    // Resolve the ids against the STRINGS bundle.
    let bundle =
        StringsBundle::from_files(Some(&strings_en), Some(&dlstrings_en), None, true).unwrap();
    let n = bundle.resolve(&mut tm);
    assert_eq!(n, 2, "both ids resolved");
    let full = tm.entries.iter().find(|e| e.field_sig == "FULL").unwrap();
    assert_eq!(full.source, "Iron Sword");
    assert_eq!(full.list_index, 0, "FULL came from .STRINGS");
    let desc = tm.entries.iter().find(|e| e.field_sig == "DESC").unwrap();
    assert_eq!(desc.source, "A basic sword.");
    assert_eq!(desc.list_index, 1, "DESC came from .DLSTRINGS");

    // Translate and emit the localized set (mode 1).
    for e in &mut tm.entries {
        e.trans = match e.field_sig.as_str() {
            "FULL" => "Épée de fer".into(),
            "DESC" => "Une épée simple.".into(),
            _ => e.source.clone(),
        };
    }
    let files = build_strings_files(&tm, true);
    assert_eq!(files.len(), 2, "one .STRINGS and one .DLSTRINGS");

    // Verify the translations landed in the right files, keyed by the same ids.
    for (kind, bytes) in &files {
        let entries = strings::parse(bytes, *kind, true).unwrap();
        match kind {
            StringsKind::Strings => {
                assert_eq!(entries, vec![(1u32, "Épée de fer".to_string())]);
            }
            StringsKind::DlStrings => {
                assert_eq!(entries, vec![(2u32, "Une épée simple.".to_string())]);
            }
            _ => panic!("unexpected file {kind:?}"),
        }
    }
}

/// Delocalization: turn a localized plugin into a standalone esp — string ids
/// become inline text (translation when present, resolved original otherwise),
/// the localized flag is cleared, and the result reads back with no STRINGS.
#[test]
fn delocalize_localized_plugin_to_standalone_esp() {
    const LOCALIZED: u32 = 0x0000_0080;

    let tes4 = {
        let mut s = sub_u16("HEDR", &[0u8; 12]);
        s.extend(sub_u16("CNAM", &zstr("SomeAuthor"))); // inline, must survive
        s.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24("TES4", 0, LOCALIZED, &s)
    };
    let weap = {
        let mut s = sub_u16("EDID", &zstr("IronSword"));
        s.extend(sub_u16("FULL", &1u32.to_le_bytes()));
        s.extend(sub_u16("DESC", &2u32.to_le_bytes()));
        rec24("WEAP", 0x0100_0801, 0, &s)
    };
    let mut src = tes4;
    src.extend(top_grup("WEAP", &weap));

    let strings_en = strings::write(&[(1, "Iron Sword".into())], StringsKind::Strings, true);
    let dlstrings_en = strings::write(
        &[(2, "A basic sword.".into())],
        StringsKind::DlStrings,
        true,
    );

    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "MyMod.esp");
    let bundle =
        StringsBundle::from_files(Some(&strings_en), Some(&dlstrings_en), None, true).unwrap();
    bundle.resolve(&mut tm);
    // Translate FULL, leave DESC untranslated (should inline the original).
    for e in &mut tm.entries {
        if e.field_sig == "FULL" {
            e.trans = "Épée de fer".into();
        }
    }

    let (out, report) = plugin::write_delocalized_esp(&src, &tm, Some(&bundle)).unwrap();
    assert!(report.is_complete(), "no residual lstrings expected here");
    assert_eq!(report.table_inlined, 2);

    // The output is no longer localized.
    let re = plugin::parse(&out).unwrap();
    assert!(!re.localized, "localized flag must be cleared");
    // The header author survived inline.
    assert_eq!(re.author, "SomeAuthor");

    // Re-extract WITHOUT any STRINGS: sources are now inline text.
    let tm2 = plugin::extract_strings(&re, "MyMod.esp");
    let full = tm2.entries.iter().find(|e| e.field_sig == "FULL").unwrap();
    assert_eq!(full.source, "Épée de fer", "translated FULL inlined");
    assert!(full.idstexte < 0, "no longer an id");
    let desc = tm2.entries.iter().find(|e| e.field_sig == "DESC").unwrap();
    assert_eq!(
        desc.source, "A basic sword.",
        "untranslated DESC keeps original"
    );
}

/// Partial delocalization: a localized string field **outside** the translatable
/// table (here a `FULL` on an unknown record type) still holds a string id after
/// the table fields are inlined. Clearing the localized flag would break it, so
/// the delocalizer inlines it from the bundle (untranslated) and reports it.
#[test]
fn delocalize_inlines_lstring_outside_table_and_reports_it() {
    const LOCALIZED: u32 = 0x0000_0080;

    let tes4 = {
        let mut s = sub_u16("HEDR", &[0u8; 12]);
        s.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24("TES4", 0, LOCALIZED, &s)
    };
    // Table field: WEAP:FULL (id 1).
    let weap = {
        let mut s = sub_u16("EDID", &zstr("IronSword"));
        s.extend(sub_u16("FULL", &1u32.to_le_bytes()));
        rec24("WEAP", 0x0100_0801, 0, &s)
    };
    // Non-table lstring: a FULL on an unknown record "XYZW" (id 3).
    let xyzw = {
        let mut s = sub_u16("EDID", &zstr("Mystery"));
        s.extend(sub_u16("FULL", &3u32.to_le_bytes()));
        rec24("XYZW", 0x0100_0802, 0, &s)
    };
    let mut src = tes4;
    src.extend(top_grup("WEAP", &weap));
    src.extend(top_grup("XYZW", &xyzw));

    let strings_en = strings::write(
        &[(1u32, "Iron Sword".into()), (3u32, "Extra Text".into())],
        StringsKind::Strings,
        true,
    );

    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "MyMod.esp");
    // Only the table field is extracted.
    assert_eq!(tm.entries.len(), 1);
    let bundle = StringsBundle::from_files(Some(&strings_en), None, None, true).unwrap();
    bundle.resolve(&mut tm);
    for e in &mut tm.entries {
        e.trans = "Épée".into();
    }

    let (out, report) = plugin::write_delocalized_esp(&src, &tm, Some(&bundle)).unwrap();
    assert_eq!(report.table_inlined, 1);
    assert_eq!(
        report.extra_inlined, 1,
        "the out-of-table lstring was inlined"
    );
    assert!(report.unresolved.is_empty());
    assert!(!report.is_complete(), "partial conversion is flagged");

    // The unknown record's FULL now holds inline text, not an id.
    let re = plugin::parse(&out).unwrap();
    assert!(!re.localized);
    let xyzw_rec = re.records.iter().find(|r| r.sig == "XYZW").unwrap();
    let full = xyzw_rec
        .subrecords
        .iter()
        .find(|s| s.sig == "FULL")
        .unwrap();
    assert_eq!(full.as_zstring(), "Extra Text");
}
