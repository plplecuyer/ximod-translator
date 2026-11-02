//! E-12 « recharger en tant que <jeu> »: a plugin re-read with another
//! game's rules keeps its translations by layered matching (R-E12-03).

use std::path::{Path, PathBuf};

use ximod_translator::games::Game;
use ximod_translator::plugin;
use ximod_translator::tm::apply::{MergeOptions, merge};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn oblivion_to_remastered_keeps_translations() {
    let path = fixture("oblivion_magpies.esp");
    let mut doc = plugin::read_file(&path).unwrap();
    assert_eq!(doc.tm_game(), Game::Oblivion);
    for (i, e) in doc.entries.iter_mut().enumerate() {
        e.trans = format!("FR{i}");
    }
    let translated = doc.entries.len();
    assert!(translated > 0);

    let mut reloaded = plugin::read_file_as(&path, Some(Game::OblivionRD)).unwrap();
    assert_eq!(reloaded.meta.game, "OblivionRD");
    let report = merge(&mut reloaded, &doc, MergeOptions::default());
    assert_eq!(report.effective, translated, "{report:?}");
    assert!(reloaded.entries.iter().all(|e| e.trans.starts_with("FR")));

    // The reverse direction, and an explicit `None`, both read the detected game.
    let back = plugin::read_file_as(&path, None).unwrap();
    assert_eq!(back.meta.game, "Oblivion");
    assert_eq!(back.entries.len(), doc.entries.len());
}

#[test]
fn skyrim_le_to_se_same_lines() {
    let path = fixture("skyrimle_riverwood.esp");
    let le = plugin::read_file(&path).unwrap();
    let se = plugin::read_file_as(&path, Some(Game::SkyrimSE)).unwrap();
    assert_eq!(le.meta.game, "Skyrim");
    assert_eq!(se.meta.game, "SkyrimSE");
    let keys = |tm: &ximod_translator::tm::TranslationMemory| -> Vec<(String, String, u32, i32)> {
        tm.entries
            .iter()
            .map(|e| (e.rec_sig.clone(), e.field_sig.clone(), e.formid, e.index))
            .collect()
    };
    assert_eq!(
        keys(&le),
        keys(&se),
        "SkyrimSE inherits Skyrim's field table"
    );
}

trait TmGame {
    fn tm_game(&self) -> Game;
}
impl TmGame for ximod_translator::tm::TranslationMemory {
    fn tm_game(&self) -> Game {
        Game::from_id(&self.meta.game)
    }
}
