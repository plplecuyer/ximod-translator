//! B-19 recette on the versions of Skipper (T-B19-04), run by hand:
//! `XIMOD_RECETTE_B19=<folder of the Skipper versions> cargo test --test
//! recette_b19 -- --ignored --nocapture`. The Traditional Chinese
//! translation of 0.3.0 (scripts and JSON files) becomes a text-only
//! dictionary; 0.5.3 is matched against it, then a copy of 0.5.3 whose
//! numbers, final punctuation or case changed on twenty lines.
//! `XIMOD_RECETTE_B19_OUT=<file.ximdb>` also writes the dictionary.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ximod_translator::basedb::{BaseDb, BaseRow, write_ximdb};
use ximod_translator::dict_stack::{DictStack, Level};
use ximod_translator::dicts::Role;
use ximod_translator::provenance::{self, Prov};
use ximod_translator::status::{self, Status};
use ximod_translator::textfmt::{Formats, TextDoc};
use ximod_translator::tm::TmEntry;

const SKIPS: [&str; 4] = [
    "College of Winterhold",
    "The Companions",
    "Main Quest",
    "Thieves Guild",
];

fn find(root: &Path, prefix: &str) -> PathBuf {
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("{prefix} not found"))
}

/// The texts of a version: the values of its JSON files, the literals of
/// its MCM script.
fn texts(dir: &Path) -> (Vec<String>, Vec<String>) {
    let fmts = Formats::builtin();
    let fmt = fmts.get("json").unwrap();
    let mut json = Vec::new();
    for s in SKIPS {
        let p = dir.join(format!(
            "skse/plugins/StorageUtilData/skipper/skips/{s}.json"
        ));
        if !p.is_file() {
            continue;
        }
        let doc = TextDoc::read(&p, fmt).unwrap();
        for &i in &doc.pairs {
            if let Some(pair) = &doc.lines[i].pair {
                json.push(pair.value.clone());
            }
        }
    }
    let data = std::fs::read(dir.join("scripts/MCM_skipper.pex")).unwrap();
    let pex = ximod_translator::pex::parse(&data).unwrap();
    let script = pex
        .literals()
        .map(|i| String::from_utf8_lossy(&pex.strings[i]).into_owned())
        .collect();
    (json, script)
}

/// A text worth translating: a letter and a space (not an identifier).
fn is_text(s: &str) -> bool {
    s.chars().any(char::is_alphabetic) && s.trim().contains(' ')
}

fn entries(texts: &[String]) -> Vec<TmEntry> {
    texts
        .iter()
        .filter(|s| is_text(s))
        .map(|s| TmEntry {
            rec_sig: "JSON".into(),
            field_sig: "TXT".into(),
            source: s.clone(),
            ..Default::default()
        })
        .collect()
}

/// Change the numbers, else the final punctuation, else the case of the
/// first letter.
fn derive(s: &str, k: usize) -> String {
    if let Some(p) = s.find(|c: char| c.is_ascii_digit()) {
        let d = s.as_bytes()[p] - b'0';
        let mut out = s.to_string();
        out.replace_range(p..p + 1, &(((d + 1) % 10).to_string()));
        return out;
    }
    if k.is_multiple_of(2) {
        match s.trim_end().strip_suffix('.') {
            Some(body) => format!("{body}!"),
            None => format!("{}.", s.trim_end()),
        }
    } else {
        let mut c = s.chars();
        match c.next() {
            Some(f) if f.is_uppercase() => f.to_lowercase().chain(c).collect(),
            Some(f) => f.to_uppercase().chain(c).collect(),
            None => String::new(),
        }
    }
}

#[test]
#[ignore]
fn skipper_versions() {
    let Ok(root) = std::env::var("XIMOD_RECETTE_B19") else {
        eprintln!("XIMOD_RECETTE_B19 not set: skipped");
        return;
    };
    let root = PathBuf::from(root);
    let (en_json, en_script) = texts(&find(&root, "Skipper 0.3.0"));
    let (zh_json, zh_script) = texts(&find(&root, "Skipper - Traditional Chinese"));
    let (new_json, new_script) = texts(&find(&root, "Skipper 112699 0.5.3"));
    assert_eq!(en_json.len(), zh_json.len());
    assert_eq!(en_script.len(), zh_script.len());
    // The dictionary: the pairs the translator changed.
    let mut rows: Vec<BaseRow> = Vec::new();
    for (s, t) in en_json
        .iter()
        .zip(&zh_json)
        .chain(en_script.iter().zip(&zh_script))
    {
        if s != t && is_text(s) && !rows.iter().any(|r| &r.source == s) {
            rows.push(BaseRow {
                source: s.clone(),
                trans: t.clone(),
                ..Default::default()
            });
        }
    }
    if let Ok(out) = std::env::var("XIMOD_RECETTE_B19_OUT") {
        write_ximdb(rows.clone(), "zho", Path::new(&out)).unwrap();
    }
    let mut db = BaseDb::default();
    db.add_rows(&rows);
    let stack = DictStack {
        levels: vec![Level {
            role: Role::TextOnly,
            path: PathBuf::from("skipper_0.3.0_ENG-ZHO.ximdb"),
            db: Arc::new(db),
        }],
        normalize: true,
    };
    let old: std::collections::HashSet<&String> = en_json.iter().chain(&en_script).collect();

    // 0.5.3: the identical texts come back, nothing new is normalized.
    for (what, list) in [("JSON", &new_json), ("script", &new_script)] {
        let mut es = entries(list);
        let r = stack.auto_translate_entries(&mut es, &mut Vec::new());
        let same = es.iter().filter(|e| old.contains(&e.source)).count();
        let filled = es.iter().filter(|e| !e.trans.is_empty()).count();
        let new: Vec<&str> = es
            .iter()
            .filter(|e| !old.contains(&e.source))
            .map(|e| e.source.as_str())
            .collect();
        eprintln!(
            "0.5.3 {what}: {} texts, {same} already in 0.3.0, {filled} filled, {} normalized, {} new",
            es.len(),
            r.normalized,
            new.len()
        );
        for n in &new {
            eprintln!("  new: {n}");
        }
        assert_eq!(r.normalized, 0, "no false match");
        assert!(
            es.iter()
                .filter(|e| !old.contains(&e.source))
                .all(|e| e.trans.is_empty())
        );
    }

    // A copy of 0.5.3 with twenty lines changed: all found, En attente.
    let known: Vec<&String> = new_json
        .iter()
        .chain(&new_script)
        .filter(|s| is_text(s) && rows.iter().any(|r| &&r.source == s))
        .collect();
    let picked: Vec<String> = known
        .iter()
        .step_by((known.len() / 20).max(1))
        .take(20)
        .enumerate()
        .map(|(k, s)| derive(s, k))
        .collect();
    let mut es = entries(&picked);
    let r = stack.auto_translate_entries(&mut es, &mut Vec::new());
    eprintln!("derived: {} lines, {} normalized", es.len(), r.normalized);
    for e in &es {
        let reports = match provenance::parse(&e.prov) {
            Prov::Normalized { reports, .. } => reports.codes(),
            _ => "-".into(),
        };
        eprintln!("  [{reports}] {} → {}", e.source, e.trans);
        assert_eq!(status::derive(e), Status::Pending);
    }
    assert_eq!(r.normalized, es.len());
}

/// The vanilla base on real translations: each French DSD file of the
/// recette is emptied, filled by the base, and the lines the normalized
/// tier filled are compared with the human translation.
/// `XIMOD_RECETTE_B19_BASE=<base.ximdb> XIMOD_RECETTE_B19_DSD=<a.json;b.json>`.
#[test]
#[ignore]
fn vanilla_base_on_human_translations() {
    let (Ok(base), Ok(list)) = (
        std::env::var("XIMOD_RECETTE_B19_BASE"),
        std::env::var("XIMOD_RECETTE_B19_DSD"),
    ) else {
        eprintln!("XIMOD_RECETTE_B19_BASE / _DSD not set: skipped");
        return;
    };
    let db = Arc::new(BaseDb::from_file(Path::new(&base)).unwrap());
    let stack = DictStack {
        levels: vec![Level {
            role: Role::Game,
            path: PathBuf::from(&base),
            db,
        }],
        normalize: true,
    };
    let (mut all, mut same) = (0, 0);
    for f in list.split(';').filter(|s| !s.is_empty()) {
        let human = ximod_translator::tm::load(Path::new(f)).unwrap();
        let mut es: Vec<TmEntry> = human
            .entries
            .iter()
            .map(|e| TmEntry {
                trans: String::new(),
                ..e.clone()
            })
            .collect();
        for e in &mut es {
            status::set(e, Status::ToTranslate);
            e.prov.clear();
        }
        let r = stack.auto_translate_entries(&mut es, &mut Vec::new());
        let name = Path::new(f).file_name().unwrap().to_string_lossy();
        let mut ok = 0;
        let mut shown = 0;
        let mut seen: std::collections::HashSet<String> = Default::default();
        for (e, h) in es.iter().zip(&human.entries) {
            if !matches!(provenance::parse(&e.prov), Prov::Normalized { .. }) {
                continue;
            }
            if e.trans.trim() == h.trans.trim() {
                ok += 1;
            } else if shown < 12 && seen.insert(e.source.clone()) {
                shown += 1;
                let src = match provenance::parse(&e.prov) {
                    Prov::Normalized {
                        source, reports, ..
                    } => {
                        format!("{source} [{}]", reports.codes())
                    }
                    _ => String::new(),
                };
                eprintln!(
                    "  {} | base: {src}\n     → {}\n     human: {}",
                    e.source, e.trans, h.trans
                );
            }
        }
        eprintln!(
            "{name}: {} lines, {} filled, {} normalized, {ok} equal to the human translation",
            es.len(),
            r.total(),
            r.normalized
        );
        all += r.normalized;
        same += ok;
    }
    eprintln!("total: {all} normalized, {same} equal to the human translation");
}
