//! The structured formats on files (D-08, T-D08-01 / 03 / 04): CSV by
//! columns with the translation in place, XML by paths with its entities,
//! and — when the recette files are present — the `config.json` of Sim
//! Settlements 2 with the MCM Helper format.

use std::path::{Path, PathBuf};

use ximod_translator::textfmt::{Formats, Resolve, TextDoc, TextFormat};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/structured")
        .join(name)
}

/// T-D08-03: the translation column read as Validé and written in place,
/// quotes kept; a missing column is refused.
#[test]
fn csv_columns_with_the_translation_in_place() {
    let formats = Formats::builtin();
    let fmt = formats.get("csv-table").unwrap();
    let path = fixture("table.csv");
    let data = std::fs::read(&path).unwrap();
    let doc = TextDoc::read(&path, fmt).unwrap();
    let s = doc.structured.as_ref().unwrap();
    assert!(s.in_place);
    assert_eq!(doc.keys(), 3, "the empty source is left out");
    assert_eq!(s.stats.read, 4);
    let tm = doc.to_tm(fmt, "table.csv", "");
    assert_eq!(tm.entries[0].trans, "Épée en fer");
    assert_eq!(
        ximod_translator::status::derive(&tm.entries[0]),
        ximod_translator::status::Status::Validated
    );
    assert_eq!(tm.entries[2].source, "Say \"hi\"; then go");
    let mut e = tm.entries.clone();
    e[1].trans = "Arc de chasse".into();
    e[2].trans = "Dis « salut » ; puis va".into();
    let out = String::from_utf8(doc.write(fmt, &e, true)).unwrap();
    assert_eq!(
        out,
        "id;source;traduction\r\nsword;\"Iron Sword\";\"Épée en fer\"\r\nbow;Hunting Bow;Arc de chasse\r\nnote;\"Say \"\"hi\"\"; then go\";\"Dis « salut » ; puis va\"\r\nempty;;\r\n"
    );
    assert_eq!(doc.write(fmt, &tm.entries, true), data);
    let mut named = fmt.clone();
    named.fields.as_mut().unwrap().translation = Some(Resolve::new("column", "traduction"));
    named.fields.as_mut().unwrap().text = Resolve::new("column", "source");
    assert_eq!(TextDoc::read(&path, &named).unwrap().keys(), 3);
    named.fields.as_mut().unwrap().translation = Some(Resolve::new("column", "translation"));
    let err = ximod_translator::textfmt::try_structured(&named, &path).unwrap_err();
    assert!(err.contains("translation"), "{err}");
}

/// T-D08-04: `//string/text()` and `//item/@label`, entities kept.
#[test]
fn xml_paths_keep_the_entities() {
    let mut fmt = TextFormat::new_structured("strings", "Strings", "xml");
    fmt.fields.as_mut().unwrap().text = Resolve::new("path", "//string/text() | //item/@label");
    let path = fixture("strings.xml");
    let data = std::fs::read(&path).unwrap();
    let doc = TextDoc::read(&path, &fmt).unwrap();
    let tm = doc.to_tm(&fmt, "strings.xml", "");
    let rows: Vec<(&str, &str, &str)> = tm
        .entries
        .iter()
        .map(|e| (e.field_sig.as_str(), e.edid.as_str(), e.source.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("string", "greet", "Fish & chips"),
            ("string", "warn", "Value < 10"),
            ("label", "m1", "Open \"map\""),
            ("label", "m2", "Close"),
        ]
    );
    let mut e = tm.entries.clone();
    e[0].trans = "Poisson & frites".into();
    e[2].trans = "Ouvrir « carte » & \"voir\"".into();
    let out = String::from_utf8(doc.write(&fmt, &e, true)).unwrap();
    let expect = String::from_utf8(data.clone())
        .unwrap()
        .replace("Fish &amp; chips", "Poisson &amp; frites")
        .replace(
            "Open &quot;map&quot;",
            "Ouvrir « carte » &amp; &quot;voir&quot;",
        );
    assert_eq!(out, expect);
    assert_eq!(doc.write(&fmt, &tm.entries, true), data);
}

fn ss2() -> Option<(PathBuf, PathBuf)> {
    let root = Path::new("/root/ximod_work/recette_lot4/Lot 4-4/Sim Settlements 2");
    let src = root.join("MCM/Config/SS2/config.json");
    let fr = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path().join("MCM/Config/SS2/config.json"))
        .find(|p| p.is_file())?;
    src.is_file().then_some((src, fr))
}

/// T-D08-01 (critère), on the recette files when present: 568 text
/// values, of which 18 empty helps are left out — 550 lines, 1 089 values
/// left out —, 548 identifiers (one control twice); the 528 French values paired; written
/// back, only those 528 values differ.
#[test]
fn sim_settlements_mcm_helper() {
    let Some((src, fr)) = ss2() else {
        eprintln!("recette files absent: skipped");
        return;
    };
    let formats = Formats::builtin();
    let cands = formats.candidates(&src, &std::fs::read(&src).unwrap());
    let fmt = &formats.formats[formats.pick_for(&src, &cands).expect("the pattern decides")];
    assert_eq!(fmt.id, "mcm-helper");
    let doc = TextDoc::read(&src, fmt).unwrap();
    let s = doc.structured.as_ref().unwrap();
    assert_eq!(doc.keys(), 550);
    assert_eq!(s.stats.read, 1639);
    assert_eq!(s.stats.excluded, 1089);
    // The file repeats one control (`bAutosaveBeforeAssaults:Usability`,
    // its text and its help): its rank distinguishes the two.
    assert_eq!(s.stats.dup_lines, 4);
    assert_eq!(s.stats.unique_ids, 548);
    let by: Vec<(&str, usize)> = s
        .stats
        .by_field
        .iter()
        .map(|(f, n)| (f.as_str(), *n))
        .collect();
    let get = |f: &str| by.iter().find(|x| x.0 == f).map(|x| x.1);
    assert_eq!(get("text"), Some(270));
    assert_eq!(get("help"), Some(193));
    assert_eq!(get("options"), Some(81));
    assert_eq!(get("pageDisplayName"), Some(6));
    // The French copy, paired by field and identifier.
    let copy = TextDoc::read(&fr, fmt).unwrap();
    let values = copy.values_by_key();
    let mut tm = doc.to_tm(fmt, "config.json", "Fallout4");
    let mut changed = 0;
    for e in tm.entries.iter_mut() {
        let key = ximod_translator::textfmt::entry_key(&e.field_sig, &e.edid);
        if let Some(v) = values.get(&key).and_then(|l| l.get((e.index - 1) as usize))
            && *v != e.source
        {
            e.trans = v.clone();
            changed += 1;
        }
    }
    assert_eq!(changed, 528);
    let out = String::from_utf8(doc.write(fmt, &tm.entries, true)).unwrap();
    let orig = std::fs::read_to_string(&src).unwrap();
    // The structure is the same: every value read again, the non-texts
    // identical, the texts the translations.
    let back = ximod_translator::textfmt::structured::parse(&out, fmt).unwrap();
    assert_eq!(back.items.len(), 550);
    let diff = back
        .items
        .iter()
        .zip(&s.items)
        .filter(|(a, b)| a.value != b.value)
        .count();
    assert_eq!(diff, 528);
    // Outside the values, byte for byte.
    let strip = |t: &str, items: &[ximod_translator::textfmt::structured::Item]| {
        let mut o = String::new();
        let mut at = 0;
        for it in items {
            o.push_str(&t[at..it.span.0]);
            at = it.span.1;
        }
        o.push_str(&t[at..]);
        o
    };
    assert_eq!(strip(&out, &back.items), strip(&orig, &s.items));
}

/// T-D08-06: a file changed outside the application since it was read is
/// refused until it is read again; T-D08-01: a translated copy pairs by
/// field, identifier and rank for the review (values left as the source
/// aside).
#[test]
fn changed_files_and_review_copies() {
    let dir = std::env::temp_dir().join(format!("ximod_struct_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("strings.json");
    std::fs::write(
        &src,
        r#"[{"id":"a","text":"Sword"},{"id":"b","text":"Bow"},{"id":"c","text":"OK"}]"#,
    )
    .unwrap();
    let formats = Formats::builtin();
    let fmt = formats.get("skse-strings").unwrap();
    let doc = TextDoc::read(&src, fmt).unwrap();
    assert!(!doc.changed_since(&src));
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        &src,
        r#"[{"id":"a","text":"Sword!"},{"id":"b","text":"Bow"},{"id":"c","text":"OK"}]"#,
    )
    .unwrap();
    assert!(doc.changed_since(&src));
    assert!(!TextDoc::read(&src, fmt).unwrap().changed_since(&src));
    let copy = dir.join("strings_fr.json");
    std::fs::write(
        &copy,
        r#"[{"id":"a","text":"Épée"},{"id":"b","text":"Arc"},{"id":"c","text":"OK"}]"#,
    )
    .unwrap();
    let tm = doc.to_tm(fmt, "strings.json", "");
    let paired = ximod_translator::textfmt::pair_copy(fmt, &tm.entries, &copy).unwrap();
    let got: Vec<(&str, &str)> = paired
        .entries
        .iter()
        .map(|e| (e.edid.as_str(), e.trans.as_str()))
        .collect();
    assert_eq!(got, vec![("a", "Épée"), ("b", "Arc")]);
    let _ = std::fs::remove_dir_all(&dir);
}
