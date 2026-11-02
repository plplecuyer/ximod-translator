//! A-20: the concordance in dictionaries written on disk — a term found in
//! two dictionaries with their records, the mod's dictionary alone, a
//! million rows, the pair taken into a line and undone, a term too short
//! (T-A20-01 to T-A20-05).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ximod_translator::basedb::{BaseDb, BaseRow, write_ximdb};
use ximod_translator::concord::{self, Level, Options};
use ximod_translator::dicts::Role;
use ximod_translator::plugin;
use ximod_translator::provenance;
use ximod_translator::status::{self, Status};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn row(rec: &str, field: &str, src: &str, trans: &str) -> BaseRow {
    BaseRow {
        rec: rec.into(),
        field: field.into(),
        source: src.into(),
        trans: trans.into(),
        ..Default::default()
    }
}

fn level(dir: &Path, name: &str, role: Role, rows: Vec<BaseRow>) -> Level {
    let path = dir.join(name);
    write_ximdb(rows, "ENG-FRA", &path).unwrap();
    Level {
        name: name.trim_end_matches(".ximdb").into(),
        role,
        db: Arc::new(BaseDb::from_file(&path).unwrap()),
    }
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ximod-concord-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stack(dir: &Path) -> Vec<Level> {
    vec![
        level(
            dir,
            "MonMod_ENG-FRA.ximdb",
            Role::Mod,
            vec![row(
                "CELL",
                "FULL",
                "Moon Sugar Refinery",
                "Raffinerie de sucrelune",
            )],
        ),
        level(
            dir,
            "Skyrim_ENG-FRA.ximdb",
            Role::Game,
            vec![
                row(
                    "MISC",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                row(
                    "CONT",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                row(
                    "ALCH",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                // Two rows of the same record and field (two potions).
                row(
                    "ALCH",
                    "FULL",
                    "Satchel of Moon Sugar",
                    "Sacoche de sucrelune",
                ),
                row("INGR", "FULL", "Moon Sugar", "Sucrelune"),
                row(
                    "INFO",
                    "NAM1",
                    "I need some moon sugar.",
                    "Il me faut du sucrelune.",
                ),
                row("BOOK", "FULL", "Sugarcane Farming", "Culture de la canne"),
                row("WEAP", "FULL", "Iron Sword", "Épée en fer"),
            ],
        ),
    ]
}

/// T-A20-01: « Sugar » selected in « Satchel of Moon Sugar »: the pairs of
/// both dictionaries, with dictionary, record and number of rows.
#[test]
fn term_found_in_two_dictionaries() {
    let dir = temp("two");
    let levels = stack(&dir);
    let source = "Satchel of Moon Sugar";
    // The word under a cursor in « Sugar » is the term too.
    assert_eq!(concord::word_at(source, 18).as_deref(), Some("Sugar"));
    let re = concord::matcher("Sugar", false).unwrap();
    let no = AtomicBool::new(false);
    let f = concord::search(&levels, &re, Options::default(), &no).unwrap();
    let got: Vec<(&str, &str, &str, u32)> = f
        .pairs
        .iter()
        .map(|p| {
            (
                p.source.as_str(),
                p.dict.as_str(),
                p.record.as_str(),
                p.count,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("Moon Sugar Refinery", "MonMod_ENG-FRA", "CELL:FULL", 1),
            ("I need some moon sugar.", "Skyrim_ENG-FRA", "INFO:NAM1", 1),
            ("Moon Sugar", "Skyrim_ENG-FRA", "INGR:FULL", 1),
            ("Satchel of Moon Sugar", "Skyrim_ENG-FRA", "ALCH:FULL", 4),
            ("Sugarcane Farming", "Skyrim_ENG-FRA", "BOOK:FULL", 1),
        ]
    );
    assert!(!f.limited);
    assert_eq!(f.scanned, 9);
    // The term highlighted, whatever its case.
    assert_eq!(
        concord::hits(&re, "I need some moon sugar."),
        vec![(17, 22)]
    );
    // Whole word: « Sugarcane » goes.
    let re = concord::matcher("Sugar", true).unwrap();
    let f = concord::search(&levels, &re, Options::default(), &no).unwrap();
    assert_eq!(f.pairs.len(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-A20-02: « Dictionnaire du mod seul ».
#[test]
fn mod_dictionary_only() {
    let dir = temp("mod");
    let levels = stack(&dir);
    let re = concord::matcher("sucrelune", false).unwrap();
    let no = AtomicBool::new(false);
    let opts = Options {
        in_trans: true,
        mod_only: true,
        ..Default::default()
    };
    let f = concord::search(&levels, &re, opts, &no).unwrap();
    assert_eq!(f.pairs.len(), 1);
    assert_eq!(f.pairs[0].dict, "MonMod_ENG-FRA");
    assert_eq!(f.scanned, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-A20-03: a million rows; a search cancelled stops at once.
#[test]
fn a_million_rows() {
    let rows: Vec<BaseRow> = (0..1_000_000)
        .map(|i| {
            row(
                ["MISC", "WEAP", "ARMO", "BOOK"][i % 4],
                "FULL",
                &format!("Item number {i} of the {} realm", i % 97),
                &format!("Objet numéro {i} du royaume {}", i % 97),
            )
        })
        .collect();
    let mut db = BaseDb::default();
    db.add_rows(&rows);
    drop(rows);
    let levels = vec![Level {
        name: "big".into(),
        role: Role::Game,
        db: Arc::new(db),
    }];
    let re = concord::matcher("number 4242", false).unwrap();
    let no = AtomicBool::new(false);
    let start = Instant::now();
    let f = concord::search(
        &levels,
        &re,
        Options {
            in_trans: true,
            ..Default::default()
        },
        &no,
    )
    .unwrap();
    let took = start.elapsed();
    assert_eq!(f.scanned, 1_000_000);
    // « number 4242 », « number 42420 » to « number 42429 », « number 424200 »…
    assert_eq!(f.pairs.len(), 111);
    // Debug build (unoptimised regex); the release build is about ten
    // times faster (under 0.2 s).
    assert!(took < Duration::from_secs(8), "{took:?}");
    eprintln!("1 000 000 rows: {took:?}");
    // Cancelled: no result.
    let yes = AtomicBool::new(true);
    let start = Instant::now();
    assert!(concord::search(&levels, &re, Options::default(), &yes).is_none());
    assert!(start.elapsed() < Duration::from_millis(500));
}

/// T-A20-04: « sucrelune » selected in the translation, replaced by the
/// pair's translation; Ctrl+Z gives it back.
#[test]
fn pair_taken_into_the_line_and_undone() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp");
    let mut tm = plugin::read_file(&fixture).unwrap();
    let i = tm.entries.iter().position(|e| !e.unused).unwrap();
    tm.entries[i].trans = "Sacoche de sucrelune".into();
    status::set(&mut tm.entries[i], Status::Validated);
    let mut undo = UndoStack::new(50);
    let chars: Vec<char> = tm.entries[i].trans.chars().collect();
    // « sucrelune » selected: characters 11 to 20.
    let (text, cursor) = concord::insert(&chars, 11, 20, "Sucrelune");
    assert_eq!(text, "Sacoche de Sucrelune");
    assert_eq!(cursor, 20);
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    let e = &mut tm.entries[i];
    e.trans = text;
    e.prov = provenance::TYPED.into();
    status::set(e, Status::Pending);
    undo.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
    undo.undo(&mut tm.entries).unwrap();
    assert_eq!(tm.entries[i].trans, "Sacoche de sucrelune");
    assert_eq!(status::derive(&tm.entries[i]), Status::Validated);
    // No selection: at the cursor.
    let (text, _) = concord::insert(&chars, 20, 20, " (Moon Sugar)");
    assert_eq!(text, "Sacoche de sucrelune (Moon Sugar)");
}

/// T-A20-05: a term of one character is not searched.
#[test]
fn term_too_short() {
    assert!(concord::matcher("S", false).is_none());
    assert!(concord::matcher("  S ", true).is_none());
    assert!(concord::matcher("é", false).is_none());
    assert!(concord::matcher("Su", false).is_some());
}
