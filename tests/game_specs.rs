//! S-05 parity: the shipped game sheets must equal the built-in ones, and every
//! selectable game must have a sheet (T-S05, `GameSpec::builtin` ↔ JSON).
use std::path::Path;
use ximod_translator::games::{Game, GameRegistry, GameSpec};

fn games_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/games")
}

#[test]
fn shipped_sheets_match_builtin() {
    for g in Game::ALL {
        let path = games_dir().join(format!("{}.json", g.id()));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let spec = GameSpec::from_json(&text, Some(g.id()))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            spec,
            GameSpec::builtin(*g),
            "{} differs from the built-in sheet",
            path.display()
        );
    }
}

#[test]
fn registry_loads_shipped_sheets_without_issue() {
    let (reg, issues) = GameRegistry::load(&games_dir());
    assert!(issues.is_empty(), "{issues:?}");
    assert!(reg.spec(Game::Starfield).utf8);
    assert_eq!(reg.spec(Game::Oblivion).header_len, 20);
}

/// The Sortie list follows the game: an output the game cannot receive is
/// greyed and emptied (Oblivion and the Fallout 3 family have no STRINGS,
/// DSD is for Skyrim SE alone); an unknown game and the text / scripts
/// modes are never restricted.
#[test]
fn outputs_follow_the_game() {
    use ximod_translator::config::OutputMode as M;
    let fits = |g: Game| -> Vec<M> {
        M::variants()
            .iter()
            .copied()
            .filter(|m| m.fits(g))
            .collect()
    };
    let old = vec![M::OverridePatch, M::FullPlugin];
    for g in [
        Game::Oblivion,
        Game::OblivionRD,
        Game::Fallout3,
        Game::FalloutNV,
    ] {
        assert_eq!(fits(g), old, "{g:?}");
        assert!(!GameSpec::builtin(g).localized_strings, "{g:?}");
    }
    assert_eq!(fits(Game::Morrowind), vec![M::FullPlugin]);
    assert_eq!(fits(Game::SkyrimSE), M::variants().to_vec());
    for g in [Game::Skyrim, Game::Fallout4, Game::Starfield] {
        assert!(
            fits(g).contains(&M::Strings) && !fits(g).contains(&M::Dsd),
            "{g:?}"
        );
    }
    assert_eq!(fits(Game::Unknown), M::variants().to_vec());
    for g in Game::ALL {
        assert!(M::Text.fits(*g) && M::Scripts.fits(*g));
    }
}
