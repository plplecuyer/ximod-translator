//! T-S04-01..04 — the export policy through the real writers (S-04): STRINGS
//! from a localized memory, override patch, DSD and full plugin from
//! `skyrimle_riverwood.esp`, with a mix of statuses.

use std::path::{Path, PathBuf};

use ximod_translator::export::ExportPolicy;
use ximod_translator::plugin;
use ximod_translator::status::{self, Status};
use ximod_translator::tm::strings::StringsKind;
use ximod_translator::tm::{TmEntry, TranslationMemory};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Translate every line, alternating Validé / Traduit par similarité / En attente.
fn translate_mixed(tm: &mut TranslationMemory) -> (usize, usize, usize) {
    let (mut v, mut s, mut p) = (0, 0, 0);
    for (i, e) in tm.entries.iter_mut().enumerate() {
        e.trans = format!("FR{i}");
        let st = match i % 3 {
            0 => {
                v += 1;
                Status::Validated
            }
            1 => {
                s += 1;
                Status::BySimilarity
            }
            _ => {
                p += 1;
                Status::Pending
            }
        };
        status::set(e, st);
    }
    (v, s, p)
}

fn translated_count(tm: &TranslationMemory) -> usize {
    tm.entries.iter().filter(|e| e.is_translated()).count()
}

#[test]
fn override_patch_and_full_plugin_follow_the_policy() {
    let path = fixture("skyrimle_riverwood.esp");
    let src = std::fs::read(&path).unwrap();
    let mut tm = plugin::read_file(&path).unwrap();
    // The header lines (FormID 0) are never patched: keep them out of the mix.
    tm.entries.retain(|e| e.formid != 0);
    let (v, s, _p) = translate_mixed(&mut tm);
    assert!(v > 0 && s > 0);

    // T-S04-03: validated only → the patch carries the validated lines only.
    let policy = ExportPolicy::validated_only();
    let counts = policy.counts(&tm);
    assert_eq!(counts.total, v);
    let filtered = policy.filtered(&tm);
    assert_eq!(translated_count(&filtered), v);
    let patch = plugin::write_override_esp(&src, &filtered, "skyrimle_riverwood.esp").unwrap();
    let back = plugin::extract_strings(&plugin::parse(&patch).unwrap(), "patch");
    let translated: Vec<&TmEntry> = back
        .entries
        .iter()
        .filter(|e| e.source.starts_with("FR"))
        .collect();
    assert_eq!(translated.len(), v, "only validated lines in the patch");
    // Records whose only translated field is not validated are absent.
    let excluded_formids: std::collections::HashSet<u32> = tm
        .entries
        .iter()
        .filter(|e| !policy.exportable(e))
        .map(|e| e.formid)
        .filter(|f| {
            !tm.entries
                .iter()
                .any(|e| e.formid == *f && policy.exportable(e))
        })
        .collect();
    assert!(!excluded_formids.is_empty());
    assert!(
        back.entries
            .iter()
            .all(|e| !excluded_formids.contains(&e.formid))
    );

    // T-S04-02: including « par similarité » adds those lines.
    let policy2 = policy.with(Status::BySimilarity, true);
    assert_eq!(policy2.counts(&tm).total, v + s);
    let patch2 =
        plugin::write_override_esp(&src, &policy2.filtered(&tm), "skyrimle_riverwood.esp").unwrap();
    let back2 = plugin::extract_strings(&plugin::parse(&patch2).unwrap(), "patch");
    assert_eq!(
        back2
            .entries
            .iter()
            .filter(|e| e.source.starts_with("FR"))
            .count(),
        v + s
    );

    // Full plugin: excluded lines keep their source text.
    let (full, _) = plugin::write_delocalized_esp(&src, &filtered, None).unwrap();
    let back_full = plugin::extract_strings(&plugin::parse(&full).unwrap(), "full");
    let originals: Vec<&TmEntry> = back_full
        .entries
        .iter()
        .filter(|e| !e.source.starts_with("FR"))
        .collect();
    assert_eq!(
        back_full
            .entries
            .iter()
            .filter(|e| e.source.starts_with("FR"))
            .count(),
        v
    );
    assert!(originals.iter().all(|e| {
        e.formid == 0
            || tm
                .entries
                .iter()
                .any(|o| o.formid == e.formid && o.source == e.source)
    }));
}

#[test]
fn dsd_carries_only_exported_lines() {
    let path = fixture("skyrimle_riverwood.esp");
    let mut tm = plugin::read_file(&path).unwrap();
    tm.entries.retain(|e| e.formid != 0);
    let (v, s, _) = translate_mixed(&mut tm);
    let policy = ExportPolicy::validated_only();
    let (_, n) = ximod_translator::tm::dsd::to_json_bytes(&policy.filtered(&tm));
    assert_eq!(n, v); // T-S04-04
    let (_, n2) = ximod_translator::tm::dsd::to_json_bytes(
        &policy.with(Status::BySimilarity, true).filtered(&tm),
    );
    assert_eq!(n2, v + s);
}

#[test]
fn strings_fall_back_on_source_for_excluded_lines() {
    // T-S04-01 on a synthetic localized memory: ids 1..=6 in the STRINGS list.
    let mut tm = TranslationMemory {
        meta: Default::default(),
        entries: vec![],
        history: Vec::new(),
    };
    for i in 1..=6u32 {
        tm.entries.push(TmEntry {
            rec_sig: "WEAP".into(),
            field_sig: "FULL".into(),
            formid: 0x1000 + i,
            edid: format!("W{i}"),
            source: format!("Sword {i}"),
            idstexte: i as i32,
            list_index: 0,
            ..Default::default()
        });
    }
    let (v, s, p) = translate_mixed(&mut tm);
    assert_eq!((v, s, p), (2, 2, 2));
    let policy = ExportPolicy::validated_only();
    let files = plugin::build_strings_files(&policy.filtered(&tm), true);
    let (kind, bytes) = files
        .iter()
        .find(|(k, _)| *k == StringsKind::Strings)
        .unwrap();
    let parsed = ximod_translator::tm::strings::parse(bytes, *kind, true).unwrap();
    assert_eq!(parsed.len(), 6, "every id is emitted");
    let translated = parsed.iter().filter(|(_, t)| t.starts_with("FR")).count();
    assert_eq!(translated, 2);
    let sources = parsed
        .iter()
        .filter(|(_, t)| t.starts_with("Sword"))
        .count();
    assert_eq!(sources, 4, "excluded ids carry the source text");

    let all = ExportPolicy::optional_statuses().fold(policy, |p, st| p.with(st, true));
    let files = plugin::build_strings_files(&all.filtered(&tm), true);
    let (kind, bytes) = files
        .iter()
        .find(|(k, _)| *k == StringsKind::Strings)
        .unwrap();
    let parsed = ximod_translator::tm::strings::parse(bytes, *kind, true).unwrap();
    assert_eq!(
        parsed.iter().filter(|(_, t)| t.starts_with("FR")).count(),
        6
    );
}
