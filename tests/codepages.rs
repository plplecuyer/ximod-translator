//! D-09 on real plugins: an Oblivion plugin in Windows-1251 (made from the
//! English fixture by putting Russian text in its fields) is read in
//! Cyrillic once the page is detected, rewritten identical when nothing is
//! modified, and a Russian translation comes back from the written file
//! (T-D09-01/02); a Skyrim LE plugin in Polish follows 1250 (T-D09-03); the
//! writing page refuses what it cannot hold.

use std::path::{Path, PathBuf};

use ximod_translator::codepage::{self, Page, Pages};
use ximod_translator::plugin;
use ximod_translator::quality;
use ximod_translator::tm::encoding::{Codec, with_input, with_output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ximod-d09-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const RU: &[&str] = &[
    "Железный меч стражника",
    "Дневник охотника, страница третья",
    "Добро пожаловать, путник, в наш город",
    "Морозная пещера у реки",
    "Будучи неуязвимым, нельзя притвориться мертвым",
    "У меня не получается сломать этот замок",
];
const PL: &[&str] = &[
    "Żelazny miecz strażnika miejskiego",
    "Dziennik łowcy, strona trzecia",
    "Witaj, wędrowcze, w naszym mieście",
    "Mroźna jaskinia nad rzeką Biała Grań",
    "Posiadłość Śnieżnych-Stóp jest zamknięta",
    "Koszary straży są pełne żołnierzy",
];

/// Put texts of `pool`, in `page`, in the FULL and DESC fields of a
/// plugin, each cut or padded to the field's length (the file keeps its
/// layout; the texts stay words of the language).
fn transcode(bytes: &[u8], pool: &[&str], page: Page) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut k = 0usize;
    let mut i = 0usize;
    while i + 6 < out.len() {
        if &out[i..i + 4] == b"FULL" || &out[i..i + 4] == b"DESC" {
            let len = u16::from_le_bytes([out[i + 4], out[i + 5]]) as usize;
            let start = i + 6;
            if len >= 8 && start + len <= out.len() && out[start + len - 1] == 0 {
                let text = pool[k % pool.len()];
                k += 1;
                let mut enc = page.encode(text);
                // Cut on a character boundary (single-byte pages) and pad
                // with spaces to the field's length, NUL kept.
                enc.truncate(len - 1);
                while enc.len() < len - 1 {
                    enc.push(b' ');
                }
                out[start..start + len - 1].copy_from_slice(&enc);
                i = start + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A plugin of an older game in `page`, written in a scratch folder.
fn make(tag: &str, fixture_name: &str, pool: &[&str], page: Page) -> (PathBuf, Vec<u8>) {
    let src = std::fs::read(fixture(fixture_name)).unwrap();
    let bytes = transcode(&src, pool, page);
    assert_ne!(bytes, src, "the fixture has text fields to transcode");
    let d = dir(tag);
    let p = d.join(fixture_name);
    std::fs::write(&p, &bytes).unwrap();
    (p, bytes)
}

fn pages() -> Pages {
    Pages::parse(include_str!("../assets/data/codepages.json"))
}

#[test]
fn oblivion_russian_detected_and_rewritten_identical() {
    let (p, bytes) = make("ru", "oblivion_magpies.esp", RU, Page::Win1251);
    // Read as before D-09: the fallback gives Latin gibberish.
    let tm = plugin::read_file(&p).unwrap();
    let texts: Vec<&str> = tm
        .entries
        .iter()
        .map(|e| e.source.as_str())
        .filter(|s| s.chars().any(|c| c as u32 >= 0x80))
        .collect();
    assert!(!texts.is_empty());
    assert!(texts.iter().all(|s| codepage::from_win1252(s)));
    // T-D09-02: the detection says Cyrillic and Russian.
    let (page, lang, _) = codepage::detect(&texts, &pages()).unwrap();
    assert_eq!(page, Page::Win1251);
    assert_eq!(lang, "rus");
    // Read again in that page: Cyrillic.
    let codec = Codec {
        page,
        strict: false,
    };
    let tm = plugin::read_file_with(&p, None, Some(codec)).unwrap();
    assert!(
        tm.entries
            .iter()
            .any(|e| e.source.starts_with("Железный меч")),
        "{:?}",
        tm.entries
            .iter()
            .map(|e| &e.source)
            .take(5)
            .collect::<Vec<_>>()
    );
    // T-D09-01: rewritten without modification, identical byte for byte.
    let out = with_output(Some(Page::Win1251), || plugin::write_full_esp(&bytes, &tm)).unwrap();
    assert_eq!(out, bytes);
    // A Russian translation written in 1251 and read back.
    let mut tm2 = tm.clone();
    let i = tm2
        .entries
        .iter()
        .position(|e| e.source.starts_with("Железный меч"))
        .unwrap();
    tm2.entries[i].trans = "Стальной кинжал".into();
    tm2.entries[i].set_translated_flags(true);
    let out = with_output(Some(Page::Win1251), || plugin::write_full_esp(&bytes, &tm2)).unwrap();
    assert_ne!(out, bytes);
    assert!(
        out.windows(15)
            .any(|w| w == Page::Win1251.encode("Стальной кинжал")),
        "the translation is in 1251"
    );
    let p2 = p.with_extension("ru.esp");
    std::fs::write(&p2, &out).unwrap();
    let back = plugin::read_file_with(&p2, None, Some(codec)).unwrap();
    assert!(back.entries.iter().any(|e| e.source == "Стальной кинжал"));
    // The same file read in the wrong page is not Cyrillic.
    let wrong = with_input(
        Some(Codec {
            page: Page::Win1250,
            strict: true,
        }),
        || plugin::read_file(&p2).unwrap(),
    );
    assert!(!wrong.entries.iter().any(|e| e.source == "Стальной кинжал"));
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn skyrim_le_polish_in_1250() {
    let (p, bytes) = make("pl", "skyrimle_riverwood.esp", PL, Page::Win1250);
    let tm = plugin::read_file(&p).unwrap();
    let texts: Vec<&str> = tm
        .entries
        .iter()
        .map(|e| e.source.as_str())
        .filter(|s| s.chars().any(|c| c as u32 >= 0x80) && codepage::from_win1252(s))
        .collect();
    let (page, lang, _) = codepage::detect(&texts, &pages()).unwrap();
    assert_eq!((page, lang.as_str()), (Page::Win1250, "pol"));
    let codec = Codec {
        page,
        strict: false,
    };
    let tm = plugin::read_file_with(&p, None, Some(codec)).unwrap();
    assert!(
        tm.entries
            .iter()
            .any(|e| e.source.contains("Żelazny miecz"))
    );
    // T-D09-03: a Polish translation written in 1250 keeps its letters.
    let mut tm2 = tm.clone();
    let i = tm2
        .entries
        .iter()
        .position(|e| e.source.contains("Żelazny miecz"))
        .unwrap();
    tm2.entries[i].trans = "Złoty miecz".into();
    tm2.entries[i].set_translated_flags(true);
    let out = with_output(Some(Page::Win1250), || plugin::write_full_esp(&bytes, &tm2)).unwrap();
    let p2 = p.with_extension("pl.esp");
    std::fs::write(&p2, &out).unwrap();
    let back = plugin::read_file_with(&p2, None, Some(codec)).unwrap();
    assert!(back.entries.iter().any(|e| e.source == "Złoty miecz"));
    // The same translation cannot go in Windows-1252: the quality check
    // names the letters, and the export would count the line (R-D09-05).
    let lines: Vec<usize> = (0..tm2.entries.len()).collect();
    let report = quality::run_with_page(
        &tm2.entries,
        &lines,
        &[quality::Check::Encoding],
        ximod_translator::games::Game::Skyrim,
        Page::Win1252,
        |_, _| true,
    );
    let f = report
        .findings
        .iter()
        .find(|f| f.line == i)
        .expect("the line is found");
    assert!(
        matches!(&f.detail, quality::Detail::Encoding(chars, page) if chars == "ł" && *page == "windows-1252")
    );
    assert!(Page::Win1252.unencodable("Złoty miecz") == vec![(1, 'ł')]);
    // In 1250 nothing is reported.
    let report = quality::run_with_page(
        &tm2.entries,
        &lines,
        &[quality::Check::Encoding],
        ximod_translator::games::Game::Skyrim,
        Page::Win1250,
        |_, _| true,
    );
    assert!(report.findings.is_empty());
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

/// The pages a file forces are kept in the header of its mod dictionary
/// (R-D09-04), beside the plugin's identity.
#[test]
fn dictionary_header_keeps_the_pages() {
    use ximod_translator::basedb::{self, BaseRow, HeaderExtra};
    let d = dir("hdr");
    let dict = d.join("MonMod_ENG-RUS.ximdb");
    let rows = vec![BaseRow {
        rec: "WEAP".into(),
        field: "FULL".into(),
        formid: 1,
        edid: "Sword".into(),
        source: "Sword".into(),
        trans: "Меч".into(),
        ..Default::default()
    }];
    let extra = HeaderExtra {
        enc_in: Some("windows-1251".into()),
        enc_out: Some("windows-1251".into()),
        vmad: None,
        ..Default::default()
    };
    basedb::write_ximdb_with(rows.clone(), Vec::new(), "rus", &dict, 0, None, &extra).unwrap();
    assert_eq!(basedb::peek_extra(&dict), extra);
    assert_eq!(basedb::read_rows_and_old(&dict).unwrap().1.len(), 1);
    // A write that does not know them keeps them.
    basedb::write_ximdb_full(rows, Vec::new(), "rus", &dict, 0).unwrap();
    assert_eq!(basedb::peek_extra(&dict), extra);
    let _ = std::fs::remove_dir_all(&d);
}

/// The recette plugins of the lot (not in the repository): Maskar's
/// Oblivion Overhaul in Russian (Windows-1251, 7.5 MB, 1 215 scripts) and
/// Open Cities Skyrim in Polish (Windows-1250). Skipped when absent.
#[test]
fn recette_plugins_if_present() {
    let root = Path::new("/root/ximod_work/recette_lot3");
    let cases = [
        (
            "Lot 3-4/x_MOO493revision4-2-47698-rev4-2-155827992/Maskar's Oblivion Overhaul.esp",
            Page::Win1251,
            "rus",
            "Бандит",
        ),
        (
            "Lot 3-4/x_polish/Data/Open Cities Skyrim.esp",
            Page::Win1250,
            "pol",
            "Biała Grań",
        ),
    ];
    for (rel, page, lang, word) in cases {
        let p = root.join(rel);
        if !p.is_file() {
            eprintln!("skipped: {} absent", p.display());
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        // The detector's models load at the first call of the session.
        let _ = ximod_translator::langdetect::detect(
            "The guard greets the traveller at the gate of the city.",
        );
        let start = std::time::Instant::now();
        let tm = plugin::read_file(&p).unwrap();
        let texts: Vec<&str> = tm
            .entries
            .iter()
            .map(|e| e.source.as_str())
            .filter(|s| s.chars().any(|c| c as u32 >= 0x80) && codepage::from_win1252(s))
            .collect();
        let t0 = std::time::Instant::now();
        let got = codepage::detect(&texts, &pages()).unwrap();
        let took = t0.elapsed();
        assert_eq!((got.0, got.1.as_str()), (page, lang), "{rel}: {got:?}");
        // 300 ms in release (R-D09 test 6); the debug build is slower.
        let budget = if cfg!(debug_assertions) { 3_000 } else { 300 };
        assert!(took.as_millis() < budget, "{rel}: detection in {took:?}");
        let codec = Codec {
            page,
            strict: false,
        };
        let tm = plugin::read_file_with(&p, None, Some(codec)).unwrap();
        assert!(tm.entries.iter().any(|e| e.source.contains(word)), "{rel}");
        let out = with_output(Some(page), || plugin::write_full_esp(&bytes, &tm)).unwrap();
        assert_eq!(out.len(), bytes.len(), "{rel}");
        assert!(out == bytes, "{rel}: rewritten identical");
        eprintln!(
            "{rel}: {} lines, detection {:?}, whole {:?}",
            tm.entries.len(),
            took,
            start.elapsed()
        );
    }
}
