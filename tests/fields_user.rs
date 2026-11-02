//! T-A06-02/03/04 — a user field file changes the extraction; a corrupt one is
//! reported and ignored; restoring brings the shipped behaviour back. Runs in
//! its own binary because the registry is process-wide.

use std::path::{Path, PathBuf};

use ximod_translator::games::{self, FieldFile, FieldMode, FieldRule, Game};
use ximod_translator::plugin;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn shipped_dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/data")
        .join(sub)
}

fn lines(path: &Path) -> Vec<(String, String, String)> {
    let tm = plugin::reader::read_file(path).unwrap();
    tm.entries
        .iter()
        .map(|e| {
            (
                format!("{}:{}", e.rec_sig, e.field_sig),
                e.edid.clone(),
                ximod_translator::status::derive(e).key().to_string(),
            )
        })
        .collect()
}

#[test]
fn user_file_overrides_then_restores() {
    let tmp = std::env::temp_dir().join(format!("ximod-fields-user-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let games_dir = shipped_dir("games");
    let fields_dir = shipped_dir("fields");
    let (gi, fi) = games::spec::init(&games_dir, &fields_dir, Some(&tmp));
    assert!(gi.is_empty() && fi.is_empty());

    let esp = fixture("skyrimle_riverwood.esp");
    let baseline = lines(&esp);
    assert!(baseline.iter().any(|(s, _, _)| s == "BOOK:FULL"));

    // T-A06-03: BOOK:FULL → ignore; BOOK:DESC → locked by an EDID pattern.
    let user = FieldFile {
        format: 1,
        game: "Skyrim".into(),
        inherits: None,
        tags: vec![],
        fields: {
            let mut f = FieldFile::builtin(Game::Skyrim).unwrap().fields;
            for r in f.iter_mut() {
                if r.sig.as_deref() == Some("BOOK:FULL") {
                    r.mode = FieldMode::Ignore;
                }
            }
            f.push(FieldRule {
                edid: Some("*Note*".into()),
                mode: FieldMode::Locked,
                ..Default::default()
            });
            f
        },
    };
    std::fs::write(tmp.join("Skyrim.json"), user.to_json()).unwrap();
    let issues = games::spec::reload_fields(Game::Skyrim);
    assert!(issues.is_empty());
    let changed = lines(&esp);
    assert!(
        !changed.iter().any(|(s, _, _)| s == "BOOK:FULL"),
        "BOOK:FULL must disappear"
    );
    let notes: Vec<_> = changed
        .iter()
        .filter(|(s, e, _)| s == "BOOK:DESC" && e.contains("Note"))
        .collect();
    assert!(!notes.is_empty());
    assert!(
        notes.iter().all(|(_, _, st)| st == "ignored"),
        "locked lines carry the Ignoré status: {notes:?}"
    );
    assert!(changed.len() < baseline.len());

    // T-A06-04: a corrupt user file is reported; extraction is the shipped one.
    std::fs::write(tmp.join("Skyrim.json"), "{ not json").unwrap();
    let issues = games::spec::reload_fields(Game::Skyrim);
    assert_eq!(issues.len(), 1);
    assert_eq!(lines(&esp), baseline);

    // Restore: no user file → shipped behaviour.
    std::fs::remove_file(tmp.join("Skyrim.json")).unwrap();
    assert!(games::spec::reload_fields(Game::Skyrim).is_empty());
    assert_eq!(lines(&esp), baseline);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn shipped_field_files_match_embedded_copies() {
    for g in Game::ALL {
        let path = shipped_dir("fields").join(format!("{}.json", g.id()));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display()));
        assert_eq!(
            Some(text.as_str()),
            games::fields::builtin_text(*g),
            "{}",
            path.display()
        );
        let f = FieldFile::from_json(&text, Some(g.id())).unwrap();
        assert!(f.validate().is_empty(), "{}", path.display());
    }
}
