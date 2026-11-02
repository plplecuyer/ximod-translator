//! Write-engine tests. The founding guardrail (J1) is byte-exact identity:
//! parsing a plugin into the fidelity tree and writing it back must reproduce
//! the original file byte for byte. Later tests exercise translation reinjection
//! (J2) and the full translated-esp output (J4).

use std::path::{Path, PathBuf};

use ximod_translator::plugin;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Parse into the fidelity tree and write back; assert byte-exact identity.
fn assert_roundtrip_identity(path: &Path) {
    let data = std::fs::read(path).unwrap();
    // The reader detects the header length (20 Oblivion / 24 modern).
    let plugin = plugin::parse(&data).unwrap();
    let tree = plugin::parse_tree(&data, plugin.header_len).unwrap();
    let out = tree.write();
    assert_eq!(
        out.len(),
        data.len(),
        "{}: length changed on round-trip",
        path.display()
    );
    assert!(
        out == data,
        "{}: bytes differ on round-trip (identity guardrail failed)",
        path.display()
    );
}

#[test]
fn roundtrip_identity_starfield() {
    let p = fixture("cheatterminal.esm");
    if !p.exists() {
        eprintln!("skipping: cheatterminal.esm not present");
        return;
    }
    assert_roundtrip_identity(&p);
}

#[test]
fn roundtrip_identity_fnv() {
    let p = fixture("lalongue.esp");
    if !p.exists() {
        eprintln!("skipping: lalongue.esp not present");
        return;
    }
    assert_roundtrip_identity(&p);
}

/// Oblivion exercises the **20-byte** header regime (and its 20-byte GRUP
/// headers) — a different code path from the modern 24-byte plugins.
#[test]
fn roundtrip_identity_oblivion() {
    let p = fixture("oblivion_magpies.esp");
    if !p.exists() {
        eprintln!("skipping: oblivion_magpies.esp not present");
        return;
    }
    // Confirm it really is the 20-byte regime before asserting identity.
    let data = std::fs::read(&p).unwrap();
    assert_eq!(plugin::parse(&data).unwrap().header_len, 20);
    assert_roundtrip_identity(&p);
}

/// Synthetic minimal plugin (TES4 header + a top GRUP of one MESG record) must
/// also round-trip identically — exercises the nested-GRUP size recomputation.
#[test]
fn roundtrip_identity_synthetic() {
    // Build the same shape as the parser test.
    fn sub(sig: &str, data: &[u8]) -> Vec<u8> {
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
    fn record(sig: &str, formid: u32, subs: &[u8]) -> Vec<u8> {
        let mut v = sig.as_bytes().to_vec();
        v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // flags
        v.extend_from_slice(&formid.to_le_bytes());
        v.extend_from_slice(&[0u8; 8]); // version tail
        v.extend_from_slice(subs);
        v
    }

    let mut subs = Vec::new();
    subs.extend(sub("EDID", &zstr("MyMsg")));
    subs.extend(sub("FULL", &zstr("Hello")));
    let mesg = record("MESG", 0x0000_0ABC, &subs);

    let tes4 = record("TES4", 0, &sub("HEDR", &[0u8; 12]));
    let mut group = b"GRUP".to_vec();
    let total = (24 + mesg.len()) as u32;
    group.extend_from_slice(&total.to_le_bytes());
    group.extend_from_slice(b"MESG");
    group.extend_from_slice(&0i32.to_le_bytes());
    group.extend_from_slice(&[0u8; 8]);
    group.extend_from_slice(&mesg);

    let mut file = tes4;
    file.extend(group);

    let tree = plugin::parse_tree(&file, 24).unwrap();
    assert_eq!(tree.write(), file);
}

// --- J2 + J4: translation reinjection and full translated esp ---

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
fn rec24(sig: &str, formid: u32, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes()); // flags (offset 8)
    v.extend_from_slice(&formid.to_le_bytes()); // formid (offset 12)
    v.extend_from_slice(&[0u8; 4]); // timestamp / VC (offset 16-19)
    v.extend_from_slice(&40u16.to_le_bytes()); // form version 40 -> Skyrim LE (offset 20)
    v.extend_from_slice(&[0u8; 2]); // unknown (offset 22-23)
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

/// Build a minimal non-localized Skyrim-LE plugin: TES4 header (Skyrim master) +
/// a GRUP of two BOOK records, each with EDID + FULL.
fn synthetic_skyrim_le() -> Vec<u8> {
    let tes4 = {
        let mut subs = sub_u16("HEDR", &[0u8; 12]);
        subs.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24("TES4", 0, &subs)
    };
    let book_a = {
        let mut s = sub_u16("EDID", &zstr("BookA"));
        s.extend(sub_u16("FULL", &zstr("Hello")));
        rec24("BOOK", 0x0000_0ABC, &s)
    };
    let book_b = {
        let mut s = sub_u16("EDID", &zstr("BookB"));
        s.extend(sub_u16("FULL", &zstr("World")));
        rec24("BOOK", 0x0000_0DEF, &s)
    };
    let mut body = book_a;
    body.extend(book_b);
    let mut file = tes4;
    file.extend(top_grup("BOOK", &body));
    file
}

#[test]
fn reinject_translation_changes_only_target_record() {
    use ximod_translator::games::Game;

    let src = synthetic_skyrim_le();
    let parsed = plugin::parse(&src).unwrap();
    assert_eq!(parsed.game, Game::Skyrim, "form version 40 -> Skyrim LE");
    assert!(!parsed.localized);

    // Extract, then translate only BookA's FULL, with an accented, longer string
    // (exercises Windows-1252 encoding and a data-size increase / GRUP resize).
    let mut tm = plugin::extract_strings(&parsed, "Test.esp");
    for e in &mut tm.entries {
        if e.formid == 0x0000_0ABC && e.field_sig == "FULL" {
            e.trans = "Bonjour à tous".into();
        }
    }

    let out = plugin::write_full_esp(&src, &tm).unwrap();
    assert_ne!(out, src, "the translated record must change the bytes");

    // Re-parse the output and confirm the translation landed and the other
    // record is intact.
    let reparsed = plugin::parse(&out).unwrap();
    let tm2 = plugin::extract_strings(&reparsed, "Test.esp");
    let a = tm2
        .entries
        .iter()
        .find(|e| e.formid == 0x0000_0ABC && e.field_sig == "FULL")
        .unwrap();
    assert_eq!(
        a.source, "Bonjour à tous",
        "translation reinjected & re-read"
    );
    let b = tm2
        .entries
        .iter()
        .find(|e| e.formid == 0x0000_0DEF && e.field_sig == "FULL")
        .unwrap();
    assert_eq!(b.source, "World", "untranslated record unchanged");
}

#[test]
fn empty_translation_yields_identity() {
    // A TM with no effective translations must reproduce the input exactly.
    let src = synthetic_skyrim_le();
    let parsed = plugin::parse(&src).unwrap();
    let tm = plugin::extract_strings(&parsed, "Test.esp"); // trans all empty
    let out = plugin::write_full_esp(&src, &tm).unwrap();
    assert_eq!(out, src, "no effective translation -> identity");
}

/// Real FNV mod: its dictionary's translations equal the source (an en→en
/// fixture), so `is_translated()` is false for all — the full-esp output must be
/// byte-identical to the input, proving reinjection never disturbs untouched data.
#[test]
fn full_esp_identity_on_real_fnv() {
    use ximod_translator::tm;

    let esp = fixture("lalongue.esp");
    let sst = fixture("lalongue.sst");
    if !esp.exists() || !sst.exists() {
        eprintln!("skipping: FNV fixtures not present");
        return;
    }
    let src = std::fs::read(&esp).unwrap();
    let mut plug = plugin::read_file(&esp).unwrap();
    let dict = tm::load(&sst).unwrap();
    tm::apply::merge(&mut plug, &dict, tm::apply::MergeOptions::default());

    let out = plugin::write_full_esp(&src, &plug).unwrap();
    assert_eq!(out, src, "identical translations must not change any byte");
}

/// End-to-end on a real Skyrim LE plugin (nested GRUPs, 696 records): fabricate a
/// distinct translation for every extracted string, write the full esp, and prove
/// (a) it re-parses cleanly — structural validity — and (b) every translation was
/// reinjected and reads back correctly.
#[test]
fn full_esp_translates_real_skyrim_le() {
    let esp = fixture("skyrimle_riverwood.esp");
    if !esp.exists() {
        eprintln!("skipping: skyrimle_riverwood.esp not present");
        return;
    }
    let src = std::fs::read(&esp).unwrap();
    let parsed = plugin::parse(&src).unwrap();
    assert_eq!(parsed.header_len, 24);
    assert!(!parsed.localized);

    let mut tm = plugin::extract_strings(&parsed, "ARW.esp");
    let original_count = tm.entries.len();
    assert!(original_count > 0);
    // Prefix each source with "FR:" so every entry is an effective translation.
    for e in &mut tm.entries {
        e.trans = format!("FR:{}", e.source);
    }

    let out = plugin::write_full_esp(&src, &tm).unwrap();
    assert_ne!(out, src);

    // Structural validity: the produced plugin must re-parse without error and
    // yield the same number of translatable strings.
    let reparsed = plugin::parse(&out).unwrap();
    let tm2 = plugin::extract_strings(&reparsed, "ARW.esp");
    assert_eq!(
        tm2.entries.len(),
        original_count,
        "no strings lost or gained"
    );
    assert!(
        tm2.entries.iter().all(|e| e.source.starts_with("FR:")),
        "every string must carry its reinjected translation"
    );
}

// --- J5: mode 3, override patch ---

#[test]
fn override_patch_is_minimal_and_masters_the_original() {
    use ximod_translator::games::Game;

    let src = synthetic_skyrim_le();
    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "Original.esp");
    // Translate only BookA.
    for e in &mut tm.entries {
        if e.formid == 0x0000_0ABC && e.field_sig == "FULL" {
            e.trans = "Bonjour".into();
        }
    }

    let patch = plugin::write_override_esp(&src, &tm, "Original.esp").unwrap();
    let reparsed = plugin::parse(&patch).unwrap();

    // Masters: the original's masters + the original itself, appended last.
    assert_eq!(
        reparsed.masters,
        vec!["Skyrim.esm".to_string(), "Original.esp".to_string()]
    );
    assert_eq!(reparsed.game, Game::Skyrim);

    // Only the translated record is present (BookA), not BookB.
    let tm2 = plugin::extract_strings(&reparsed, "Patch.esp");
    let books: Vec<_> = tm2.entries.iter().filter(|e| e.rec_sig == "BOOK").collect();
    assert_eq!(books.len(), 1, "only the translated record is carried");
    assert_eq!(books[0].formid, 0x0000_0ABC);
    assert_eq!(books[0].source, "Bonjour");
    // BookB (untranslated) must not appear at all.
    assert!(
        !tm2.entries.iter().any(|e| e.formid == 0x0000_0DEF),
        "untranslated record must be excluded from the override"
    );
}

/// An ESM original must yield an ESM override patch (the plugin tier is
/// preserved) — matching how mod authors ship translation patches. Mirrors the
/// real Starfield "…French.esm" example (flags keep bit 0 set).
#[test]
fn override_patch_preserves_esm_tier() {
    // A synthetic Skyrim-LE plugin, but flagged ESM on its TES4 record.
    let mut src = synthetic_skyrim_le();
    // TES4 flags live at offset 8 of the file (sig[4] + size[4]).
    src[8] |= 0x01; // set ESM bit
    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "Original.esm");
    for e in &mut tm.entries {
        if e.formid == 0x0000_0ABC && e.field_sig == "FULL" {
            e.trans = "Bonjour".into();
        }
    }
    let patch = plugin::write_override_esp(&src, &tm, "Original.esm").unwrap();
    // The patch's TES4 flags must still carry the ESM bit.
    let flags = u32::from_le_bytes([patch[8], patch[9], patch[10], patch[11]]);
    assert_eq!(
        flags & 0x01,
        0x01,
        "ESM tier must be preserved on the patch"
    );
}

/// Full TES4 record builder with explicit flags and form version.
fn rec24_full(sig: &str, formid: u32, flags: u32, form_ver: u16, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&formid.to_le_bytes());
    v.extend_from_slice(&[0u8; 4]); // timestamp / VC
    v.extend_from_slice(&form_ver.to_le_bytes());
    v.extend_from_slice(&[0u8; 2]);
    v.extend_from_slice(subs);
    v
}

/// An ESL (light) original must produce a valid ESL override patch: masters
/// `[base…, original]`, the ESL flag preserved, and **FormIDs unchanged** — the
/// original's own light record (self-index high byte) and a base-game override
/// both keep their FormID, because a light plugin's in-file records use the same
/// master-count self-index as a regular plugin.
#[test]
fn override_patch_supports_esl_original() {
    const ESL: u32 = 0x0000_0200;
    // ESL header (form version 44 = Skyrim SE), one master.
    let tes4 = {
        let mut subs = sub_u16("HEDR", &[0u8; 12]);
        subs.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24_full("TES4", 0, ESL, 44, &subs)
    };
    // Own new light record: high byte 1 = self (1 master), object 0x801 ≤ 0xFFF.
    let own = {
        let mut s = sub_u16("EDID", &zstr("OwnBook"));
        s.extend(sub_u16("FULL", &zstr("Hello")));
        rec24_full("BOOK", 0x0100_0801, 0, 44, &s)
    };
    // Override of a base-game record (high byte 0 = Skyrim.esm).
    let over = {
        let mut s = sub_u16("EDID", &zstr("BaseOver"));
        s.extend(sub_u16("FULL", &zstr("World")));
        rec24_full("BOOK", 0x0001_2345, 0, 44, &s)
    };
    let mut body = own;
    body.extend(over);
    let mut src = tes4;
    src.extend(top_grup("BOOK", &body));

    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "Foo.esl");
    for e in &mut tm.entries {
        if e.field_sig == "FULL" {
            e.trans = format!("FR:{}", e.source);
        }
    }

    let patch = plugin::write_override_esp(&src, &tm, "Foo.esl").unwrap();
    let re = plugin::parse(&patch).unwrap();

    // Masters: base + original appended; ESL flag preserved.
    assert_eq!(
        re.masters,
        vec!["Skyrim.esm".to_string(), "Foo.esl".to_string()]
    );
    let flags = u32::from_le_bytes([patch[8], patch[9], patch[10], patch[11]]);
    assert_eq!(flags & ESL, ESL, "ESL tier must be preserved");

    // FormIDs unchanged (no remap) and both records translated.
    let tm2 = plugin::extract_strings(&re, "Patch.esl");
    let own2 = tm2
        .entries
        .iter()
        .find(|e| e.formid == 0x0100_0801)
        .unwrap();
    assert_eq!(
        own2.source, "FR:Hello",
        "light own record kept & translated"
    );
    let over2 = tm2
        .entries
        .iter()
        .find(|e| e.formid == 0x0001_2345)
        .unwrap();
    assert_eq!(over2.source, "FR:World", "base override kept & translated");
}

/// Real Skyrim LE mod: translate only the single `REFR:FULL` (a placed reference
/// inside a CELL). The override must (a) re-parse, (b) carry the **parent CELL**
/// so the reference has valid context, and (c) master the original.
#[test]
fn override_patch_carries_parent_cell_on_real_skyrim_le() {
    let esp = fixture("skyrimle_riverwood.esp");
    if !esp.exists() {
        eprintln!("skipping: skyrimle_riverwood.esp not present");
        return;
    }
    let src = std::fs::read(&esp).unwrap();
    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "ARW.esp");

    // Translate only the placed-reference name(s).
    let mut refr_formid = None;
    for e in &mut tm.entries {
        if e.rec_sig == "REFR" && e.field_sig == "FULL" {
            e.trans = format!("FR:{}", e.source);
            refr_formid = Some(e.formid);
        }
    }
    let refr_formid = refr_formid.expect("the mod has a REFR:FULL");

    let patch =
        plugin::write_override_esp(&src, &tm, "Amazing Riverwood Starter Home.esp").unwrap();
    let reparsed = plugin::parse(&patch).unwrap();

    // Masters end with the original plugin.
    assert_eq!(
        reparsed.masters.last().unwrap(),
        "Amazing Riverwood Starter Home.esp"
    );
    // Much smaller than the source (only a handful of records).
    assert!(patch.len() < src.len() / 2, "override must be minimal");

    let tm2 = plugin::extract_strings(&reparsed, "Patch.esp");
    // The translated REFR is present with its translation.
    let refr = tm2
        .entries
        .iter()
        .find(|e| e.rec_sig == "REFR" && e.field_sig == "FULL")
        .expect("REFR carried into the patch");
    assert!(refr.source.starts_with("FR:"));
    assert_eq!(refr.formid, refr_formid);

    // The parent CELL was carried for context (a CELL record is present).
    assert!(
        reparsed.records.iter().any(|r| r.sig == "CELL"),
        "the parent CELL must be carried so the reference is valid"
    );
}
