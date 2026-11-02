//! B-19 — normalized matching of the dictionaries: the level of the stack,
//! the reports, the provenance, the mode of Appliquer les dictionnaires
//! (T-B19-01 … T-B19-06).

use std::path::PathBuf;
use std::sync::Arc;

use ximod_translator::apply::{self, Mode, Options, Overwrite};
use ximod_translator::basedb::{BaseDb, BaseRow, FillLevel};
use ximod_translator::dict_stack::{DictStack, Level};
use ximod_translator::dicts::Role;
use ximod_translator::games::Game;
use ximod_translator::provenance::{self, Prov, ProvFilter};
use ximod_translator::quality::{self, Check};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

const NNBSP: char = '\u{202f}';

fn row(rec: &str, field: &str, src: &str, tr: &str) -> BaseRow {
    BaseRow {
        rec: rec.into(),
        field: field.into(),
        formid: 0x800 + src.len() as u32,
        edid: format!("Ed{}", src.len()),
        source: src.into(),
        trans: tr.into(),
        ..Default::default()
    }
}

fn entry(rec: &str, field: &str, src: &str) -> TmEntry {
    TmEntry {
        rec_sig: rec.into(),
        field_sig: field.into(),
        formid: 0x900,
        edid: "New".into(),
        source: src.into(),
        ..Default::default()
    }
}

fn db(rows: &[BaseRow]) -> Arc<BaseDb> {
    let mut d = BaseDb::default();
    d.add_rows(rows);
    Arc::new(d)
}

fn stack(dbs: Vec<(Role, Arc<BaseDb>)>, normalize: bool) -> DictStack {
    DictStack {
        levels: dbs
            .into_iter()
            .enumerate()
            .map(|(i, (role, db))| Level {
                role,
                path: PathBuf::from(format!("D{i}_ENG-FRA.ximdb")),
                db,
            })
            .collect(),
        normalize,
    }
}

fn base() -> Arc<BaseDb> {
    db(&[
        row(
            "MESG",
            "DESC",
            "You have 3 days left!",
            &format!("Il vous reste 3 jours{NNBSP}!"),
        ),
        row("MESG", "DESC", "Wait 2 hours.", "Attendez trois heures."),
        row(
            "BOOK",
            "FULL",
            "The Lusty Argonian Maid",
            "La Servante argonienne",
        ),
    ])
}

/// T-B19-01 / T-B19-02: numbers, final punctuation and case carried over;
/// En attente, provenance « normalisée ».
#[test]
fn numbers_punctuation_and_case() {
    let s = stack(vec![(Role::Mod, base())], true);
    let mut es = vec![
        entry("MESG", "DESC", "You have 5 days left."),
        entry("MESG", "DESC", "you have 7 days left"),
        entry("MESG", "DESC", "You have 3 days and 2 nights"),
        entry("MESG", "DESC", "You have 3 days left!"),
    ];
    let mut origins = Vec::new();
    let r = s.auto_translate_entries(&mut es, &mut origins);
    assert_eq!(es[0].trans, "Il vous reste 5 jours.");
    assert_eq!(status::derive(&es[0]), Status::Pending);
    match provenance::parse(&es[0].prov) {
        Prov::Normalized {
            role,
            file,
            reports,
            source,
        } => {
            assert_eq!(role, Role::Mod);
            assert_eq!(file, "D0_ENG-FRA.ximdb");
            assert_eq!(reports.codes(), "np");
            assert_eq!(source, "You have 3 days left!");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(ProvFilter::of(&es[0]), Some(ProvFilter::Normalized));
    assert_eq!(es[1].trans, "il vous reste 7 jours");
    assert!(matches!(provenance::parse(&es[1].prov),
        Prov::Normalized { reports, .. } if reports.codes() == "npc"));
    // Another sentence: not matched (T-B19-02).
    assert!(es[2].trans.is_empty());
    // The same text: the text level, never the normalized one.
    assert_eq!(status::derive(&es[3]), Status::BySimilarity);
    assert_eq!(r.normalized, 2);
    assert!(origins.iter().any(|(_, o)| o.level == FillLevel::Text));
}

/// T-B19-03: numbers the translation does not show are not carried; the
/// quality check Nombres flags the line.
#[test]
fn numbers_not_carried_are_flagged() {
    let s = stack(vec![(Role::Game, base())], true);
    let mut es = vec![entry("MESG", "DESC", "Wait 5 hours.")];
    s.auto_translate_entries(&mut es, &mut Vec::new());
    assert_eq!(es[0].trans, "Attendez trois heures.");
    assert!(matches!(provenance::parse(&es[0].prov),
        Prov::Normalized { reports, .. } if reports.codes() == "!"));
    let rep = quality::run(&es, &[0], &[Check::Numbers], Game::SkyrimSE, |_, _| true);
    assert_eq!(rep.lines().len(), 1);
}

/// T-B19-05: the setting off, nothing at opening; the Normalisé mode of
/// Appliquer les dictionnaires finds the lines.
#[test]
fn setting_off_and_apply_mode() {
    let s = stack(vec![(Role::Mod, base())], false);
    let mut es = vec![
        entry("MESG", "DESC", "You have 5 days left."),
        entry("BOOK", "FULL", "the lusty argonian maid"),
    ];
    let r = s.auto_translate_entries(&mut es, &mut Vec::new());
    assert_eq!(r.normalized, 0);
    assert!(es.iter().all(|e| e.trans.is_empty()));
    let levels = vec![apply::Level {
        name: "D0_ENG-FRA.ximdb".into(),
        role: Role::Mod,
        db: base(),
        fuzzy: None,
    }];
    // Strict without the setting: nothing.
    let strict = Options {
        normalize: false,
        ..Default::default()
    };
    let p = apply::plan(&es, &[0, 1], &levels, &strict, |_, _| true);
    assert!(p.changes.is_empty());
    let opts = Options {
        mode: Mode::Normalized,
        overwrite: Overwrite::None,
        ..Default::default()
    };
    let p = apply::plan(&es, &[0, 1], &levels, &opts, |_, _| true);
    assert_eq!(p.normalized, 2);
    assert_eq!(p.filled(), 2);
    let touched = apply::commit(&mut es, &p);
    assert_eq!(touched, vec![0, 1]);
    assert_eq!(es[0].trans, "Il vous reste 5 jours.");
    assert_eq!(es[1].trans, "la Servante argonienne");
    assert!(es.iter().all(|e| status::derive(e) == Status::Pending));
    assert!(matches!(provenance::parse(&es[1].prov),
        Prov::Normalized { source, reports, .. }
            if source == "The Lusty Argonian Maid" && reports.codes() == "c"));
    // Strict with the setting: the stack's rule, normalized included.
    let mut fresh = vec![entry("MESG", "DESC", "You have 9 days left.")];
    let p = apply::plan(&fresh, &[0], &levels, &Options::default(), |_, _| true);
    apply::commit(&mut fresh, &p);
    assert_eq!(fresh[0].trans, "Il vous reste 9 jours.");
}

/// A form shared by sources whose adapted translations differ is ambiguous
/// (B-08): En attente, marked with the number of candidates.
#[test]
fn ambiguous_forms() {
    let d = db(&[
        row("WEAP", "FULL", "Iron Sword", "Épée en fer"),
        row("WEAP", "FULL", "Iron sword!", "Épée de fer !"),
    ]);
    let s = stack(vec![(Role::Game, d)], true);
    let mut es = vec![entry("WEAP", "FULL", "IRON SWORD.")];
    s.auto_translate_entries(&mut es, &mut Vec::new());
    assert!(!es[0].trans.is_empty());
    assert_eq!(provenance::split_ambiguous(&es[0].prov).1, Some(2));
    assert_eq!(status::derive(&es[0]), Status::Pending);
}

/// Locked (Ignoré), unused and translated lines are not filled; a personal
/// dictionary gives En attente, not Personnalisé (R-B19-06).
#[test]
fn what_is_not_filled() {
    let s = stack(vec![(Role::Personal, base())], true);
    let mut ignored = entry("MESG", "DESC", "You have 5 days left.");
    status::set(&mut ignored, Status::Ignored);
    let mut unused = entry("MESG", "DESC", "You have 6 days left.");
    unused.unused = true;
    let mut es = vec![
        ignored,
        unused,
        entry("MESG", "DESC", "You have 8 days left."),
    ];
    s.auto_translate_entries(&mut es, &mut Vec::new());
    assert!(es[0].trans.is_empty() && es[1].trans.is_empty());
    assert_eq!(status::derive(&es[2]), Status::Pending);
}

/// The old data of the mod's dictionary get the level too (R-B19-06).
#[test]
fn old_data_are_normalized() {
    let dir = std::env::temp_dir().join(format!("ximod-norm-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("M_ENG-FRA.ximdb");
    ximod_translator::basedb::write_ximdb_full(
        vec![row("MISC", "FULL", "Gold", "Or")],
        vec![ximod_translator::basedb::OldRow {
            row: row("MESG", "DESC", "Rest 4 hours.", "Reposez-vous 4 heures."),
            date: "2026-10-01".into(),
        }],
        "fra",
        &path,
        0,
    )
    .unwrap();
    let d = Arc::new(BaseDb::from_file(&path).unwrap());
    assert_eq!(d.old_rows(), 1);
    let s = stack(vec![(Role::Mod, d)], true);
    let mut es = vec![entry("MESG", "DESC", "Rest 6 hours!")];
    s.auto_translate_entries(&mut es, &mut Vec::new());
    assert_eq!(es[0].trans, "Reposez-vous 6 heures!");
    assert_eq!(status::derive(&es[0]), Status::Pending);
    let _ = std::fs::remove_dir_all(dir);
}

/// T-B19-06: 50 000 lines, five bases — the normalized level costs little
/// at opening (measured in release; the debug build only reports it).
#[test]
fn cost_at_opening() {
    let mk = |k: usize| {
        let rows: Vec<BaseRow> = (0..20_000)
            .map(|i| {
                row(
                    "MESG",
                    "DESC",
                    &format!("Base {k} message number {i}, take {} steps!", i % 7),
                    &format!("Message {i} de la base {k}, faites {} pas !", i % 7),
                )
            })
            .collect();
        db(&rows)
    };
    let dbs: Vec<(Role, Arc<BaseDb>)> = (0..5).map(|k| (Role::Game, mk(k))).collect();
    let lines: Vec<TmEntry> = (0..50_000)
        .map(|i| {
            entry(
                "MESG",
                "DESC",
                &format!("Plugin line {i} says something new, {} times.", i % 9),
            )
        })
        .collect();
    let time = |normalize: bool| {
        let s = stack(dbs.clone(), normalize);
        let mut es = lines.clone();
        // The indexes are built on first use: once before measuring.
        s.auto_translate_entries(&mut es.clone(), &mut Vec::new());
        let t = std::time::Instant::now();
        s.auto_translate_entries(&mut es, &mut Vec::new());
        t.elapsed()
    };
    let off = time(false);
    let on = time(true);
    let extra = on.saturating_sub(off);
    eprintln!("normalized tier: {extra:?} more ({off:?} → {on:?})");
    if !cfg!(debug_assertions) {
        assert!(extra.as_millis() < 300, "{extra:?}");
    }
}
