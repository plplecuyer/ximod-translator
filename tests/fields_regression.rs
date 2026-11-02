//! T-A06-01 — non-regression of the extraction after the translatable-field
//! tables moved from Rust constants to `assets/data/fields/<game>.json`.
//!
//! Every fixture plugin is extracted and the resulting lines (record, field,
//! FormID, EDID, instance, string id, status, source) are compared with the
//! listing captured **before** the change, kept in
//! `tests/fixtures/expected/<fixture>.txt`. Run with `XIMOD_UPDATE_EXPECTED=1`
//! to regenerate a listing after an intentional change of a shipped file.

use std::path::{Path, PathBuf};

use ximod_translator::plugin;

const FIXTURES: &[&str] = &[
    "skyrimle_riverwood.esp",
    "oblivion_magpies.esp",
    "morrowind_thieves.esp",
    "cheatterminal.esm",
    "lalongue.esp",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn listing(path: &Path) -> String {
    let tm = plugin::reader::read_file(path).expect("fixture readable");
    let mut out = String::new();
    out.push_str(&format!("game={}\n", tm.meta.game));
    for e in &tm.entries {
        let status = ximod_translator::status::derive(e).key();
        out.push_str(&format!(
            "{}:{}\t{:08X}\t{}\t{}\t{}\t{}\t{}\n",
            e.rec_sig,
            e.field_sig,
            e.formid,
            e.edid,
            e.index,
            e.idstexte,
            status,
            e.source.replace('\n', "\\n").replace('\r', "\\r")
        ));
    }
    out
}

#[test]
fn extraction_matches_captured_listings() {
    let update = std::env::var("XIMOD_UPDATE_EXPECTED")
        .map(|v| v == "1")
        .unwrap_or(false);
    let expected_dir = fixture("expected");
    let mut failures = Vec::new();
    for name in FIXTURES {
        let path = fixture(name);
        if !path.is_file() {
            continue;
        }
        let got = listing(&path);
        let exp_path = expected_dir.join(format!("{name}.txt"));
        if update {
            std::fs::create_dir_all(&expected_dir).unwrap();
            std::fs::write(&exp_path, &got).unwrap();
            continue;
        }
        let exp = std::fs::read_to_string(&exp_path).unwrap_or_else(|_| {
            panic!(
                "missing listing {} (run with XIMOD_UPDATE_EXPECTED=1)",
                exp_path.display()
            )
        });
        if got != exp {
            let got_lines: Vec<&str> = got.lines().collect();
            let exp_lines: Vec<&str> = exp.lines().collect();
            let first = got_lines
                .iter()
                .zip(exp_lines.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got_lines.len().min(exp_lines.len()));
            failures.push(format!(
                "{name}: {} line(s) expected, {} got; first difference at line {}:\n  expected: {}\n  got:      {}",
                exp_lines.len(),
                got_lines.len(),
                first + 1,
                exp_lines.get(first).unwrap_or(&"<none>"),
                got_lines.get(first).unwrap_or(&"<none>")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
