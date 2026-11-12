//! B-11 on a real plugin: an autosaved state of `skyrimle_riverwood.esp`
//! written in the background, found at the next start, and resumed onto the
//! freshly read file (T-B11-01); a normal close leaves nothing (T-B11-02).

use std::path::{Path, PathBuf};
use std::time::Duration;

use ximod_translator::backup::{Autosave, AutosaveEvent, Manifest, resume_into};
use ximod_translator::config::OutputMode;
use ximod_translator::doc_settings::DocSettings;
use ximod_translator::plugin;
use ximod_translator::status::{self, Status};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn crash_then_resume_restores_the_work() {
    let dir = std::env::temp_dir().join(format!("ximod-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = fixture("skyrimle_riverwood.esp");
    let mut tm = plugin::read_file(&src).unwrap();
    let mut n = 0;
    for (i, e) in tm.entries.iter_mut().enumerate() {
        if i % 2 == 0 {
            e.trans = format!("FR{i}");
            status::set(
                e,
                if i % 4 == 0 {
                    Status::Validated
                } else {
                    Status::Pending
                },
            );
            n += 1;
        }
    }
    let settings = DocSettings {
        game: "Skyrim".into(),
        source: "eng".into(),
        target: "fra".into(),
        output: Some(OutputMode::OverridePatch),
        use_personal: true,
    };
    let mut auto = Autosave::new(dir.clone(), 5);
    auto.spawn(tm.clone(), Manifest::new(&src, &settings, n), 3);
    // Up to 30 s: the suite runs in parallel on a loaded machine; a failed
    // write is shown with its error.
    let mut events = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while events.is_empty() && std::time::Instant::now() < deadline {
        events.extend(auto.poll());
        if events.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AutosaveEvent::Done { .. })),
        "{events:?}"
    );

    // "Crash": no discard. The next start finds the manifest with the settings.
    let found = Autosave::pending(&dir);
    assert_eq!(found.items.len(), 1);
    let (m, state) = &found.items[0];
    assert_eq!(m.settings, settings);
    assert_eq!(m.modified, n);
    assert!(!m.source_changed());

    // Resume onto a freshly read document, as the application does.
    let saved = Autosave::load_state(state).unwrap();
    let mut fresh = plugin::read_file(&src).unwrap();
    let r = resume_into(&mut fresh.entries, &saved);
    assert_eq!((r.matched, r.unmatched), (n, 0));
    assert_eq!(
        fresh.entries, tm.entries,
        "the resumed document equals the autosaved one"
    );

    // Normal close: nothing left (T-B11-02).
    auto.discard(&src).unwrap();
    assert!(Autosave::pending(&dir).items.is_empty());
    assert_eq!(std::fs::read_dir(&dir).unwrap().flatten().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
