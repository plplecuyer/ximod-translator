//! S-08 — game detection through Heroic (GOG / Epic / Amazon) and Lutris,
//! on simulated launcher configurations (`tests/fixtures/launchers/`) whose
//! paths are rewritten to a temporary tree holding the games' master files.

use std::path::{Path, PathBuf};

use ximod_translator::games::Game;
use ximod_translator::games::detect::{self, Source};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/launchers")
        .join(name)
}

/// Copy a launcher fixture into `dst`, replacing `__GAMES__` / `__PREFIX__`.
/// The paths are written with forward slashes: the fixtures are JSON and
/// YAML, where a Windows backslash would be an escape character, and the
/// launchers themselves write their paths that way on Windows.
fn instantiate(src: &Path, dst: &Path, games: &Path, prefix: &Path) {
    let slashes = |p: &Path| p.display().to_string().replace('\\', "/");
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let out = dst.join(entry.file_name());
        if p.is_dir() {
            std::fs::create_dir_all(&out).unwrap();
            instantiate(&p, &out, games, prefix);
        } else {
            let text = std::fs::read_to_string(&p)
                .unwrap()
                .replace("__GAMES__", &slashes(games))
                .replace("__PREFIX__", &slashes(prefix));
            std::fs::write(&out, text).unwrap();
        }
    }
}

fn make_game(games: &Path, folder: &str, data: &str, master: &str) {
    let d = games.join(folder).join(data);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(master), b"TES4").unwrap();
}

#[test]
fn heroic_finds_gog_and_epic_games_and_skips_the_rest() {
    let tmp = std::env::temp_dir().join(format!("ximod-launchers-heroic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let games = tmp.join("Games");
    make_game(&games, "Skyrim Special Edition", "Data", "Skyrim.esm");
    make_game(&games, "Fallout 4", "Data", "Fallout4.esm");
    make_game(&games, "Amazon Game", "Data", "Nothing.esm"); // no known master
    let cfg = tmp.join("heroic");
    std::fs::create_dir_all(&cfg).unwrap();
    instantiate(&fixture("heroic"), &cfg, &games, &tmp);

    let mut found = detect::detect_heroic_in(&cfg);
    found.sort_by_key(|d| d.game.id());
    let ids: Vec<(Game, Source)> = found.iter().map(|d| (d.game, d.source)).collect();
    assert_eq!(
        ids,
        vec![
            (Game::Fallout4, Source::Heroic),
            (Game::SkyrimSE, Source::Heroic)
        ]
    );
    assert_eq!(
        found[1].data_dir.canonicalize().unwrap(),
        games
            .join("Skyrim Special Edition")
            .join("Data")
            .canonicalize()
            .unwrap()
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn lutris_yaml_roots_and_detection() {
    // Roots from the YAML alone.
    let roots = detect::lutris_yaml_roots(
        "game:\n  exe: /games/Skyrim/SkyrimSE.exe\n  working_dir: /games/Skyrim\n",
    );
    assert_eq!(roots, vec![PathBuf::from("/games/Skyrim")]);
    let roots = detect::lutris_yaml_roots(
        "game:\n  exe: drive_c/GOG Games/Morrowind/Morrowind.exe\n  prefix: /home/p/Games/mw\n",
    );
    assert_eq!(
        roots,
        vec![PathBuf::from(
            "/home/p/Games/mw/drive_c/GOG Games/Morrowind"
        )]
    );
    assert!(detect::lutris_yaml_roots("game: {}\n").is_empty());

    // Detection on the fixtures: Morrowind under a Wine prefix (Data Files),
    // Starfield by absolute path (T-S08-01 in spirit).
    let tmp = std::env::temp_dir().join(format!("ximod-launchers-lutris-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let games = tmp.join("Games");
    let prefix = tmp.join("prefix-mw");
    make_game(
        &prefix.join("drive_c").join("GOG Games"),
        "Morrowind",
        "Data Files",
        "Morrowind.esm",
    );
    make_game(&games, "Starfield", "Data", "Starfield.esm");
    let cfg = tmp.join("lutris");
    std::fs::create_dir_all(&cfg).unwrap();
    instantiate(&fixture("lutris"), &cfg, &games, &prefix);

    let mut found = detect::detect_lutris_in(&cfg);
    found.sort_by_key(|d| d.game.id());
    found.dedup_by_key(|d| d.game);
    let ids: Vec<Game> = found.iter().map(|d| d.game).collect();
    assert_eq!(ids, vec![Game::Morrowind, Game::Starfield]);
    assert!(found.iter().all(|d| d.source == Source::Lutris));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn launcher_files_parse() {
    let gog = std::fs::read_to_string(fixture("heroic/gog_store/installed.json")).unwrap();
    let e = detect::parse_heroic_gog_installed(&gog);
    assert_eq!(e.len(), 2);
    assert!(e[0].1.contains("Skyrim Special Edition"));
    let leg = std::fs::read_to_string(fixture("heroic/legendaryConfig/legendary/installed.json"))
        .unwrap();
    let e = detect::parse_legendary_installed(&leg);
    assert_eq!(
        e,
        vec![("__GAMES__/Fallout 4".to_string(), "Fallout 4".to_string())]
    );
    let nile = std::fs::read_to_string(fixture("heroic/nile_config/nile/installed.json")).unwrap();
    assert_eq!(detect::parse_nile_installed(&nile).len(), 1);
    assert!(detect::parse_heroic_gog_installed("garbage").is_empty());
}
