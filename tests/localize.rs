//! C-09 on real plugins (cheatterminal.esm is the Starfield Cheat Terminal): a non-localized plugin localized with the shipped
//! table — identifiers 1 to n in file order, an empty field 0, two fields of
//! the same text two identifiers —, then delocalized from its own STRINGS:
//! the original comes back byte for byte (T-C09-02/04); the localized
//! plugin reopens with every line; the derivation of a table from a
//! localized master (T-C09-03).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ximod_translator::games::Game;
use ximod_translator::lstrings::{self, Table};
use ximod_translator::plugin::{self, StringsBundle};
use ximod_translator::tm::strings::{self, StringsKind};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn table(game: &str) -> Table {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/data/lstrings")
        .join(format!("{game}.json"));
    Table::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The STRINGS of a conversion, as the export writes them, read back.
fn bundle(lists: &[(StringsKind, Vec<(u32, String)>)], utf8: bool) -> StringsBundle {
    let get = |k: StringsKind| {
        lists
            .iter()
            .find(|(kind, _)| *kind == k)
            .map(|(_, l)| strings::write(l, k, utf8))
    };
    let (s, d, i) = (
        get(StringsKind::Strings),
        get(StringsKind::DlStrings),
        get(StringsKind::IlStrings),
    );
    StringsBundle::from_files(s.as_deref(), d.as_deref(), i.as_deref(), utf8).unwrap()
}

/// Every record of a plugin: signature, FormID, flags without the
/// compression bit, and its fields inflated.
fn normalized(bytes: &[u8]) -> Vec<(String, u32, u32, Vec<u8>)> {
    use std::io::Read;
    let mut tree = plugin::parse_tree(bytes, 24).unwrap();
    tree.records_mut()
        .into_iter()
        .map(|r| {
            let flags = r.flags();
            let body = if flags & 0x0004_0000 != 0 {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(&r.data[4..])
                    .read_to_end(&mut out)
                    .unwrap();
                out
            } else {
                r.data.clone()
            };
            let flags = flags & !0x0004_0000 & !0x80;
            (r.sig_str(), r.formid(), flags, body)
        })
        .collect()
}

#[test]
fn starfield_localize_then_delocalize_is_identical() {
    let src = std::fs::read(fixture("cheatterminal.esm")).unwrap();
    let tm = plugin::read_file(&fixture("cheatterminal.esm")).unwrap();
    let out = lstrings::localize(&src, &tm, &table("Starfield"), Game::Starfield).unwrap();
    let r = &out.report;
    assert!(r.ids > 100, "{r:?}");
    // The header says Localized; the ids run 1..=n, each once.
    let parsed = plugin::parse(&out.plugin).unwrap();
    assert!(parsed.localized);
    let mut all: Vec<u32> = out
        .target
        .iter()
        .flat_map(|(_, l)| l.iter().map(|(id, _)| *id))
        .collect();
    all.sort_unstable();
    assert_eq!(all, (1..=r.ids).collect::<Vec<_>>());
    // The source lists carry the original texts.
    let src_texts: HashSet<&str> = out
        .source
        .iter()
        .flat_map(|(_, l)| l.iter().map(|(_, t)| t.as_str()))
        .collect();
    assert!(
        tm.entries
            .iter()
            // The scripts' strings stay in the plugin (C-08).
            .filter(|e| !e.source.is_empty() && e.rec_sig != "TES4" && e.field_sig != "VMAD")
            .all(|e| src_texts.contains(e.source.as_str())),
        "every line's text is in the STRINGS"
    );
    // T-C09-02: delocalized from its own STRINGS, the original comes back —
    // record for record, the compressed ones written uncompressed by both
    // conversions.
    let b = bundle(&out.source, true);
    let (back, rep) = plugin::write_delocalized_esp(&out.plugin, &tm, Some(&b)).unwrap();
    assert!(
        rep.unresolved.iter().all(|(_, _, id)| *id == 0),
        "{:?}",
        rep.unresolved
    );
    assert_eq!(rep.extra_inlined, 0, "no flag nor counter inlined");
    assert_eq!(normalized(&back), normalized(&src));

    // The localized plugin reopens: every line, with its text, from the
    // STRINGS next to it.
    let d = std::env::temp_dir().join(format!("ximod-c09-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Strings")).unwrap();
    let p = d.join("cheatterminal.esm");
    std::fs::write(&p, &out.plugin).unwrap();
    for (kind, list) in &out.source {
        let name = format!("cheatterminal_English.{}", kind.extension());
        std::fs::write(
            d.join("Strings").join(name),
            strings::write(list, *kind, true),
        )
        .unwrap();
    }
    let mut tm2 = plugin::read_file(&p).unwrap();
    let bundle = StringsBundle::from_plugin_path(&p, "english", true).unwrap();
    let n = bundle.resolve(&mut tm2);
    assert!(n > 0);
    let before: Vec<(&str, &str, i32, &str)> = tm
        .entries
        .iter()
        .filter(|e| e.rec_sig != "TES4")
        .map(|e| {
            (
                e.rec_sig.as_str(),
                e.field_sig.as_str(),
                e.index,
                e.source.as_str(),
            )
        })
        .collect();
    let after: Vec<(&str, &str, i32, &str)> = tm2
        .entries
        .iter()
        .filter(|e| e.rec_sig != "TES4")
        .map(|e| {
            (
                e.rec_sig.as_str(),
                e.field_sig.as_str(),
                e.index,
                e.source.as_str(),
            )
        })
        .collect();
    assert_eq!(before, after);
    let _ = std::fs::remove_dir_all(&d);
}

/// The translation of an exported line goes in the target STRINGS; the
/// others keep their text; two fields of the same text have two ids.
#[test]
fn translations_and_identifiers() {
    let src = std::fs::read(fixture("cheatterminal.esm")).unwrap();
    let mut tm = plugin::read_file(&fixture("cheatterminal.esm")).unwrap();
    let i = tm
        .entries
        .iter()
        .position(|e| e.rec_sig != "TES4" && !e.source.is_empty() && !e.unused)
        .unwrap();
    let source = tm.entries[i].source.clone();
    tm.entries[i].trans = format!("{source} (FR)");
    tm.entries[i].set_translated_flags(true);
    let out = lstrings::localize(&src, &tm, &table("Starfield"), Game::Starfield).unwrap();
    let target: Vec<&String> = out
        .target
        .iter()
        .flat_map(|(_, l)| l.iter().map(|(_, t)| t))
        .collect();
    assert!(target.iter().any(|t| **t == format!("{source} (FR)")));
    // Two fields of the same text: two identifiers.
    let mut seen: std::collections::HashMap<&str, u32> = Default::default();
    let mut twice = false;
    for (_, l) in &out.source {
        for (_, t) in l {
            let c = seen.entry(t.as_str()).or_insert(0);
            *c += 1;
            twice |= *c > 1;
        }
    }
    assert!(twice, "the fixture has repeated texts");
}

/// A plugin already localized is refused.
#[test]
fn already_localized_is_refused() {
    let src = std::fs::read(fixture("cheatterminal.esm")).unwrap();
    let tm = plugin::read_file(&fixture("cheatterminal.esm")).unwrap();
    let out = lstrings::localize(&src, &tm, &table("Starfield"), Game::Starfield).unwrap();
    assert!(lstrings::localize(&out.plugin, &tm, &table("Starfield"), Game::Starfield).is_err());
}

/// T-C09-03: the derivation of the table from a localized master. A master
/// made by localizing the fixture with the shipped table gives back the
/// same lists for the fields it has.
#[test]
fn derivation_from_a_localized_master() {
    let src = std::fs::read(fixture("cheatterminal.esm")).unwrap();
    let tm = plugin::read_file(&fixture("cheatterminal.esm")).unwrap();
    let shipped = table("Starfield");
    let out = lstrings::localize(&src, &tm, &shipped, Game::Starfield).unwrap();
    let mut ids: [HashSet<u32>; 3] = Default::default();
    for (kind, list) in &out.source {
        ids[kind.list_index() as usize].extend(list.iter().map(|(id, _)| *id));
    }
    let derived = lstrings::derive(
        Game::Starfield,
        "cheatterminal.esm",
        &out.plugin,
        &ids,
        "2026-10-27",
    )
    .unwrap();
    assert!(!derived.is_empty());
    for (sig, f) in &derived.fields {
        let (rec, fld) = sig.split_once(':').unwrap();
        assert_eq!(
            shipped.list(rec, fld, "s"),
            Some(f.list),
            "{sig} derived in {:?}",
            f.list
        );
    }
    // The fields the master uses are all found again.
    for (rec, fld) in [("TERM", "FULL"), ("MESG", "DESC")] {
        if derived.fields.contains_key(&format!("{rec}:{fld}")) {
            assert_eq!(derived.list(rec, fld, ""), shipped.list(rec, fld, ""));
        }
    }
    assert!(
        derived.fields.contains_key("TERM:FULL"),
        "{:?}",
        derived.fields.keys()
    );
    let back = Table::parse(&derived.to_json()).unwrap();
    assert_eq!(back, derived);
}

/// The recette plugins of the lot (not in the repository): Skyrim SE,
/// Fallout 4 and Starfield mods, localized then delocalized from their own
/// STRINGS, record for record identical. Skipped when absent.
#[test]
fn recette_plugins_if_present() {
    let root = Path::new("/root/ximod_work/recette_lot3");
    let cases = [
        (
            "Lot 3-1/x_MENU_MCM_VANILLA-178821-0-0-1-1777500611/Menu MCM Vanilla.esp",
            "SkyrimSE",
        ),
        (
            "Lot 3-1/x_Skipper_112699_0.5.3_2026-08-15T18-57Z_t/skipper.esp",
            "SkyrimSE",
        ),
        (
            "Lot 3-6/QuickStart_SE_1.2.2/Quick Start - SE.esp",
            "SkyrimSE",
        ),
        (
            "Lot 3-2/x_CMRQL_2.41_Installer-39575-2-41-16438757/CMRQL_2_41_Installer/Files/CMRQL.esp",
            "Fallout4",
        ),
        (
            "Lot 3-5/CheatTerminal_FO4_1.78/Data/CheatTerminal.esp",
            "Fallout4",
        ),
        (
            "Lot 3-5/CheatTerminal_Starfield_1.08/CheatTerminal.esm",
            "Starfield",
        ),
    ];
    for (rel, game) in cases {
        let p = root.join(rel);
        if !p.is_file() {
            eprintln!("skipped: {} absent", p.display());
            continue;
        }
        let g = Game::from_id(game);
        let src = std::fs::read(&p).unwrap();
        let tm = plugin::read_file_as(&p, Some(g)).unwrap();
        let start = std::time::Instant::now();
        let out = lstrings::localize(&src, &tm, &table(game), g).unwrap();
        let took = start.elapsed();
        let b = bundle(&out.source, true);
        let (back, rep) = plugin::write_delocalized_esp(&out.plugin, &tm, Some(&b)).unwrap();
        assert!(
            rep.unresolved.iter().all(|(_, _, id)| *id == 0),
            "{rel}: {:?}",
            &rep.unresolved[..rep.unresolved.len().min(5)]
        );
        assert_eq!(normalized(&back), normalized(&src), "{rel}");
        eprintln!(
            "{rel}: {} ids, lists {:?}, unknown {:?}, {:?}",
            out.report.ids, out.report.per_list, out.report.unknown, took
        );
    }
}
