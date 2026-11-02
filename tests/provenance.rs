//! B-09 on a real plugin: the provenance follows the operations, undo and
//! redo, the filter and the working formats (T-B09-01 to T-B09-06).

use std::path::{Path, PathBuf};

use ximod_translator::basedb::{BaseRow, FillLevel};
use ximod_translator::dicts::Role;
use ximod_translator::filter::Criteria;
use ximod_translator::i18n::I18n;
use ximod_translator::plugin;
use ximod_translator::provenance::{self, ProvFilter};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::{self, TranslationMemory};
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before, record};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn shown(tm: &TranslationMemory, f: ProvFilter) -> Vec<usize> {
    Criteria {
        prov: f,
        ..Default::default()
    }
    .apply(&tm.entries)
}

/// T-B09-01 / T-B09-02: twenty lines from a service are exactly what the
/// « En ligne » filter shows, En attente; five validated stay « en ligne ».
#[test]
fn online_lines_keep_their_provenance_when_validated() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let lines: Vec<usize> = (0..tm.entries.len())
        .filter(|&i| !tm.entries[i].unused && !tm.entries[i].source.trim().is_empty())
        .take(20)
        .collect();
    assert_eq!(lines.len(), 20);
    let mut undo = UndoStack::new(50);
    let code = provenance::online("deepl");
    record(&mut undo, &mut tm.entries, OpKind::Online, None, |es| {
        for &i in &lines {
            es[i].trans = format!("[fr] {}", es[i].source);
            es[i].prov = code.clone();
            status::set(&mut es[i], Status::Pending);
        }
    });
    assert_eq!(shown(&tm, ProvFilter::Online), lines);
    assert!(
        lines
            .iter()
            .all(|&i| status::derive(&tm.entries[i]) == Status::Pending)
    );
    // Validate five (F5): a status operation.
    let five = &lines[..5];
    let before: Vec<(usize, EntryState)> = five
        .iter()
        .map(|&i| (i, EntryState::capture(&tm.entries[i])))
        .collect();
    for &i in five {
        status::set(&mut tm.entries[i], Status::Validated);
    }
    undo.push(op_from_before(OpKind::Status, &before, &mut tm.entries));
    assert_eq!(shown(&tm, ProvFilter::Online), lines);
    let mut i18n = I18n::new();
    i18n.set_locale("fra");
    assert_eq!(
        provenance::tip(&i18n, &tm.entries[five[0]]).unwrap(),
        "Validé — en ligne (DeepL)"
    );
    i18n.set_locale("eng");
    assert_eq!(
        provenance::short_label(&i18n, &tm.entries[five[0]]),
        "online (DeepL)"
    );
}

/// T-B09-04: retouching a dictionary line makes it « saisie »; undo gives
/// its provenance back, redo takes it again.
#[test]
fn typing_over_a_dictionary_line_and_undo() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let i = tm.entries.iter().position(|e| !e.unused).unwrap();
    let dict = provenance::dict(Role::Game, FillLevel::Exact, "Skyrim_ENG-FRA.ximdb");
    tm.entries[i].trans = "Traduction du jeu".into();
    tm.entries[i].prov = dict.clone();
    status::set(&mut tm.entries[i], Status::Validated);
    let mut undo = UndoStack::new(50);
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    tm.entries[i].trans = "Ma traduction".into();
    undo.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
    assert_eq!(tm.entries[i].prov, provenance::TYPED);
    assert_eq!(shown(&tm, ProvFilter::Typed), vec![i]);
    undo.undo(&mut tm.entries).unwrap();
    assert_eq!(tm.entries[i].prov, dict);
    assert_eq!(tm.entries[i].trans, "Traduction du jeu");
    assert_eq!(shown(&tm, ProvFilter::Game), vec![i]);
    undo.redo(&mut tm.entries).unwrap();
    assert_eq!(tm.entries[i].prov, provenance::TYPED);
    // Emptying the translation clears the provenance.
    let before = vec![(i, EntryState::capture(&tm.entries[i]))];
    tm.entries[i].trans.clear();
    undo.push(op_from_before(OpKind::Edit, &before, &mut tm.entries));
    assert!(tm.entries[i].prov.is_empty());
    assert!(shown(&tm, ProvFilter::Empty).contains(&i));
}

/// T-B09-06: a translation equal to the source is « Identique à la
/// source », whatever its stored provenance.
#[test]
fn identical_lines_are_found() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let i = tm.entries.iter().position(|e| !e.unused).unwrap();
    tm.entries[i].trans = tm.entries[i].source.clone();
    tm.entries[i].prov = provenance::TYPED.into();
    assert_eq!(shown(&tm, ProvFilter::Identical), vec![i]);
    assert!(!shown(&tm, ProvFilter::Typed).contains(&i));
    let mut i18n = I18n::new();
    i18n.set_locale("fra");
    assert_eq!(provenance::short_label(&i18n, &tm.entries[i]), "identique");
    assert_eq!(
        provenance::full_label(&i18n, &tm.entries[i]).as_deref(),
        Some("saisie")
    );
}

/// The pivot JSON, the native XML and the mod's dictionary carry the
/// provenance and the review (A.4.2); SST and EET do not.
#[test]
fn working_formats_keep_provenance_and_review() {
    let mut tm = plugin::read_file(&fixture("skyrimle_riverwood.esp")).unwrap();
    let i = tm.entries.iter().position(|e| !e.unused).unwrap();
    tm.entries[i].trans = "Bonjour".into();
    tm.entries[i].prov = provenance::fuzzy(91);
    tm.entries[i].review = "Salut".into();
    let dir = std::env::temp_dir().join(format!("ximod-prov-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Pivot JSON.
    let json = dir.join("doc.json");
    tm::save(&tm, &json).unwrap();
    let back = tm::load(&json).unwrap();
    assert_eq!(back.entries[i].prov, "fuzzy:91");
    assert_eq!(back.entries[i].review, "Salut");
    // Native XML.
    let xml = tm::xml::write_ximodxml(&tm);
    let back = tm::xml::read_ximodxml(&xml).unwrap();
    assert_eq!(back.entries[i].prov, "fuzzy:91");
    assert_eq!(back.entries[i].review, "Salut");
    // The xTranslator XML has no place for them.
    let sst = tm::xml::write_sstxml(&tm);
    assert!(!sst.contains("fuzzy:91"));
    // The mod's dictionary row.
    let row = BaseRow::from_entry(&tm.entries[i], true);
    assert_eq!(row.prov, "fuzzy:91");
    assert_eq!(row.review, "Salut");
    // A file opened with translations and no provenance: « importée ».
    let mut sst_like = tm.clone();
    sst_like.entries[i].prov.clear();
    provenance::mark_imported(&mut sst_like.entries, "sst");
    assert_eq!(shown(&sst_like, ProvFilter::Import), vec![i]);
    let _ = std::fs::remove_dir_all(&dir);
}
