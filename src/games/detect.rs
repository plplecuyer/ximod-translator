//! Installed-game detection across stores (Steam, GOG, GOG Galaxy, Epic, Xbox
//! Game Pass) and manual / folder-scan fallbacks, on Windows, Linux and macOS.
//!
//! See `docs/detection-jeux.md` for the design. The guiding rule is **validation
//! by master file**: a candidate folder is only accepted when the game's master
//! plugin (e.g. `Skyrim.esm`) is present, which rules out stale store entries and
//! makes store-specific ids optional. OS- and store-specific readers are isolated
//! so that any failure degrades to "no candidate" instead of an error.

use std::path::{Path, PathBuf};

use crate::games::Game;

/// Where a detection came from (also traced in `Config.ini`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Steam,
    GogGalaxy,
    GogRegistry,
    Epic,
    Xbox,
    /// Heroic Games Launcher (GOG / Epic / Amazon on Linux, macOS, Windows).
    Heroic,
    /// Lutris (Linux).
    Lutris,
    FolderScan,
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::GogGalaxy => "GogGalaxy",
            Source::GogRegistry => "GogRegistry",
            Source::Epic => "Epic",
            Source::Xbox => "Xbox",
            Source::Heroic => "Heroic",
            Source::Lutris => "Lutris",
            Source::FolderScan => "FolderScan",
            Source::Manual => "Manual",
        }
    }

    /// Lower is more trustworthy (used to deduplicate the same game).
    fn priority(self) -> u8 {
        match self {
            Source::Steam => 0,
            Source::GogGalaxy => 1,
            Source::GogRegistry => 2,
            Source::Epic => 3,
            Source::Xbox => 4,
            Source::Heroic => 5,
            Source::Lutris => 6,
            Source::FolderScan => 7,
            Source::Manual => 8,
        }
    }
}

/// A validated game installation.
#[derive(Debug, Clone)]
pub struct DetectedGame {
    pub game: Game,
    /// Install root (e.g. `…/Skyrim Special Edition`).
    pub root: PathBuf,
    /// Data sub-folder holding the master plugin (`…/Data`, or `…/Data Files`).
    pub data_dir: PathBuf,
    pub source: Source,
}

/// Static per-game identifiers and detection proofs (single source of truth).
struct GameIds {
    game: Game,
    /// Steam AppIDs (several for GOTY / regional variants).
    steam_appids: &'static [u32],
    /// Substrings matched (case-insensitive) against an Epic DisplayName / a
    /// folder name during a scan.
    names: &'static [&'static str],
    /// Data sub-folder ("Data", or "Data Files" for Morrowind).
    data_subdir: &'static str,
    /// Master plugin proving the game is really there.
    master_file: &'static str,
}

const GAME_IDS: &[GameIds] = &[
    GameIds {
        game: Game::Morrowind,
        steam_appids: &[22320],
        names: &["Morrowind"],
        data_subdir: "Data Files",
        master_file: "Morrowind.esm",
    },
    GameIds {
        game: Game::Oblivion,
        steam_appids: &[22330, 22490],
        names: &["Oblivion"],
        data_subdir: "Data",
        master_file: "Oblivion.esm",
    },
    GameIds {
        game: Game::OblivionRD,
        // The Elder Scrolls IV: Oblivion Remastered — its own Steam app, with the
        // original Gamebryo data nested under Content/Dev/ObvData/Data.
        steam_appids: &[2623190],
        names: &["Oblivion Remastered"],
        data_subdir: "OblivionRemastered/Content/Dev/ObvData/Data",
        master_file: "Oblivion.esm",
    },
    GameIds {
        game: Game::Skyrim,
        steam_appids: &[72850],
        names: &["Skyrim"],
        data_subdir: "Data",
        master_file: "Skyrim.esm",
    },
    GameIds {
        game: Game::SkyrimSE,
        steam_appids: &[489830],
        names: &["Skyrim Special Edition", "Skyrim Anniversary"],
        data_subdir: "Data",
        master_file: "Skyrim.esm",
    },
    GameIds {
        game: Game::Fallout3,
        steam_appids: &[22300, 22370],
        names: &["Fallout 3"],
        data_subdir: "Data",
        master_file: "Fallout3.esm",
    },
    GameIds {
        game: Game::FalloutNV,
        steam_appids: &[22380, 22490],
        names: &["Fallout New Vegas", "FalloutNV"],
        data_subdir: "Data",
        master_file: "FalloutNV.esm",
    },
    GameIds {
        game: Game::Fallout4,
        steam_appids: &[377160],
        names: &["Fallout 4"],
        data_subdir: "Data",
        master_file: "Fallout4.esm",
    },
    GameIds {
        game: Game::Fallout76,
        steam_appids: &[1151340],
        names: &["Fallout76", "Fallout 76"],
        data_subdir: "Data",
        master_file: "SeventySix.esm",
    },
    GameIds {
        game: Game::Starfield,
        steam_appids: &[1716740],
        names: &["Starfield"],
        data_subdir: "Data",
        master_file: "Starfield.esm",
    },
];

fn ids_for(game: Game) -> Option<&'static GameIds> {
    GAME_IDS.iter().find(|g| g.game == game)
}

fn game_from_steam_appid(appid: u32) -> Option<Game> {
    GAME_IDS
        .iter()
        .find(|g| g.steam_appids.contains(&appid))
        .map(|g| g.game)
}

// --------------------------------------------------------------------------
// Validation (the master-file gate) and small path helpers
// --------------------------------------------------------------------------

/// A child of `dir` named `name`, matched case-insensitively (needed on Linux,
/// where Windows game copies carry mixed casing). Exact hit is tried first.
fn find_child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    // `name` may be a nested relative path (e.g. Oblivion Remastered's
    // "OblivionRemastered/Content/Dev/ObvData/Data"); walk each segment
    // case-insensitively so a wrong-case path still resolves.
    let mut cur = dir.to_path_buf();
    for seg in name.split(['/', '\\']).filter(|s| !s.is_empty()) {
        let exact = cur.join(seg);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let mut next = None;
        if let Ok(rd) = std::fs::read_dir(&cur) {
            for entry in rd.flatten() {
                if let Some(n) = entry.file_name().to_str()
                    && n.eq_ignore_ascii_case(seg)
                {
                    next = Some(entry.path());
                    break;
                }
            }
        }
        cur = next?;
    }
    Some(cur)
}

/// If `root` holds `game` (its data sub-folder contains the master plugin),
/// return that data folder. This is the single acceptance gate.
fn master_present(game: Game, root: &Path) -> Option<PathBuf> {
    let ids = ids_for(game)?;
    let data = find_child_ci(root, ids.data_subdir)?;
    find_child_ci(&data, ids.master_file)?;
    Some(data)
}

/// Validate a candidate root against a specific game, returning a `DetectedGame`.
fn validate(game: Game, root: PathBuf, source: Source) -> Option<DetectedGame> {
    let data_dir = master_present(game, &root)?;
    Some(DetectedGame {
        game,
        root,
        data_dir,
        source,
    })
}

/// Validate a root against every supported game (used when the source does not
/// tell us which game a folder holds — GOG registry, folder scan, manual pick).
fn validate_any(root: &Path, source: Source) -> Vec<DetectedGame> {
    GAME_IDS
        .iter()
        .filter_map(|ids| validate(ids.game, root.to_path_buf(), source))
        .collect()
}

// --------------------------------------------------------------------------
// Public entry points
// --------------------------------------------------------------------------

/// A detection source: a plain function returning its candidates.
type SourceFn = fn() -> Vec<DetectedGame>;

/// The detection sources, in order, each with a stable id for progress display.
const STEPS: &[(&str, SourceFn)] = &[
    ("steam", detect_steam),
    ("gog", detect_gog_registry),
    ("gog-galaxy", detect_gog_galaxy),
    ("epic", detect_epic),
    ("xbox", detect_xbox),
    ("heroic", detect_heroic),
    ("lutris", detect_lutris),
    ("scan", detect_scan),
];

/// Detect every installed supported game across all available sources, calling
/// `on_step(done, total, id)` before each source runs (and once at the end with
/// `done == total`). Runs on the calling thread; use it from a worker thread for
/// a responsive UI.
pub fn detect_all_reporting(
    mut on_step: impl FnMut(usize, usize, &'static str),
) -> Vec<DetectedGame> {
    let total = STEPS.len();
    let mut c = Vec::new();
    for (i, (id, f)) in STEPS.iter().enumerate() {
        on_step(i, total, id);
        c.extend(f());
    }
    on_step(total, total, "");
    dedup(c)
}

/// Number of detection sources (for progress display).
pub const STEP_COUNT: usize = STEPS.len();

/// Detect every installed supported game (no progress reporting).
pub fn detect_all() -> Vec<DetectedGame> {
    detect_all_reporting(|_, _, _| {})
}

/// The data sub-folder for a game ("Data", or "Data Files" for Morrowind).
pub fn data_subdir(game: Game) -> &'static str {
    ids_for(game).map(|g| g.data_subdir).unwrap_or("Data")
}

/// The master plugin file name for a game (e.g. `"Skyrim.esm"`).
pub fn master_file(game: Game) -> &'static str {
    ids_for(game).map(|g| g.master_file).unwrap_or("")
}

/// Whether `root` still holds `game` (its data folder contains the master
/// plugin). Used to validate `Config.ini` paths at startup.
pub fn is_game_at(game: Game, root: &Path) -> bool {
    master_present(game, root).is_some()
}

/// The game whose master plugin is `file_name` (e.g. `"Oblivion.esm"` →
/// `Game::Oblivion`), if any. `Skyrim.esm` maps to the first match (Skyrim LE).
/// Used to route a plugin-pair dictionary into the game bases folder.
pub fn game_from_master_file(file_name: &str) -> Option<Game> {
    GAME_IDS
        .iter()
        .find(|g| g.master_file.eq_ignore_ascii_case(file_name))
        .map(|g| g.game)
}

/// Validate a folder the user picked manually against every game. Returns the
/// matches (usually one).
pub fn validate_manual(root: &Path) -> Vec<DetectedGame> {
    validate_any(root, Source::Manual)
}

/// Run detection and merge the results into `cfg` (never overwriting a locked
/// entry), then save `Config.ini`. Returns how many entries were written.
pub fn detect_and_save(cfg: &mut crate::config::AppConfig) -> usize {
    let found = detect_all();
    let mut n = 0;
    for d in &found {
        if cfg.set_game(
            d.game.id(),
            &d.root.to_string_lossy(),
            d.source.as_str(),
            false,
        ) {
            n += 1;
        }
    }
    let _ = cfg.save();
    n
}

/// Keep one entry per game: the most trustworthy source, then the shortest path.
fn dedup(mut cands: Vec<DetectedGame>) -> Vec<DetectedGame> {
    cands.sort_by(|a, b| {
        (a.game.id(), a.source.priority(), a.root.as_os_str().len()).cmp(&(
            b.game.id(),
            b.source.priority(),
            b.root.as_os_str().len(),
        ))
    });
    let mut out: Vec<DetectedGame> = Vec::new();
    for c in cands {
        if !out.iter().any(|k| k.game == c.game) {
            out.push(c);
        }
    }
    out
}

// --------------------------------------------------------------------------
// VDF helpers (Steam) — line-based, sufficient for libraryfolders/appmanifest
// --------------------------------------------------------------------------

fn first_quoted(s: &str) -> Option<String> {
    let a = s.find('"')?;
    let rest = &s[a + 1..];
    let b = rest.find('"')?;
    Some(rest[..b].to_string())
}

/// All values for `key` in a Valve VDF text (key and value on the same line).
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix(&needle)
                .and_then(first_quoted)
                .map(|v| v.replace("\\\\", "\\"))
        })
        .collect()
}

fn vdf_value(text: &str, key: &str) -> Option<String> {
    vdf_values(text, key).into_iter().next()
}

// --------------------------------------------------------------------------
// Steam (all OS)
// --------------------------------------------------------------------------

/// Candidate Steam roots for the current OS.
fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(k) = hkcu.open_subkey("Software\\Valve\\Steam")
            && let Ok(p) = k.get_value::<String, _>("SteamPath")
        {
            roots.push(PathBuf::from(p));
        }
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(k) = hklm.open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
            && let Ok(p) = k.get_value::<String, _>("InstallPath")
        {
            roots.push(PathBuf::from(p));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(pf).join("Steam"));
        }
    }

    #[cfg(not(windows))]
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Application Support/Steam"));
        } else {
            // Linux: native, Flatpak and Snap layouts.
            roots.push(home.join(".steam/steam"));
            roots.push(home.join(".steam/root"));
            roots.push(home.join(".local/share/Steam"));
            roots.push(home.join(".var/app/com.valvesoftware.Steam/data/Steam"));
            roots.push(home.join("snap/steam/common/.local/share/Steam"));
        }
    }

    roots.retain(|p| p.is_dir());
    roots
}

/// Library roots for a Steam install: the entries of `libraryfolders.vdf` plus
/// the Steam root itself.
fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let steamapps =
        find_child_ci(steam_root, "steamapps").unwrap_or_else(|| steam_root.join("steamapps"));
    if let Some(vdf) = find_child_ci(&steamapps, "libraryfolders.vdf")
        && let Ok(text) = std::fs::read_to_string(&vdf)
    {
        for p in vdf_values(&text, "path") {
            libs.push(PathBuf::from(p));
        }
    }
    libs
}

/// Extra Steam library roots found by probing drives directly — a robust
/// fallback when the registry / `libraryfolders.vdf` can't be read (e.g. the app
/// runs elevated, so `HKCU` is the wrong hive, or Steam lives on another drive).
/// A folder is a library root when it contains a `steamapps` sub-folder.
fn extra_steam_libraries() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let candidates = [
            "SteamLibrary",
            "Steam",
            "SteamGames",
            "Games\\SteamLibrary",
            "Games\\Steam",
            "Program Files (x86)\\Steam",
            "Program Files\\Steam",
        ];
        let mut out = Vec::new();
        for d in b'A'..=b'Z' {
            let drive = format!("{}:\\", d as char);
            for c in candidates {
                let root = PathBuf::from(&drive).join(c);
                if find_child_ci(&root, "steamapps").is_some_and(|p| p.is_dir()) {
                    out.push(root);
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// All Steam library roots to scan: those declared via the registry +
/// `libraryfolders.vdf`, plus the drive-probe fallback, de-duplicated.
fn all_steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in steam_roots() {
        libs.extend(steam_libraries(&root));
    }
    libs.extend(extra_steam_libraries());
    libs.sort();
    libs.dedup();
    libs
}

fn detect_steam() -> Vec<DetectedGame> {
    let mut out = Vec::new();
    for lib in all_steam_libraries() {
        let sa = find_child_ci(&lib, "steamapps").unwrap_or_else(|| lib.join("steamapps"));
        if !sa.is_dir() {
            continue;
        }
        let common = find_child_ci(&sa, "common").unwrap_or_else(|| sa.join("common"));
        for ids in GAME_IDS {
            for &appid in ids.steam_appids {
                let manifest = sa.join(format!("appmanifest_{appid}.acf"));
                let Ok(text) = std::fs::read_to_string(&manifest) else {
                    continue;
                };
                let installdir = vdf_value(&text, "installdir").unwrap_or_default();
                let dir = if installdir.is_empty() {
                    None
                } else {
                    Some(
                        find_child_ci(&common, &installdir)
                            .unwrap_or_else(|| common.join(&installdir)),
                    )
                };
                let candidate = dir.unwrap_or_else(|| common.join(ids.names[0]));
                if let Some(g) = game_from_steam_appid(appid)
                    && let Some(d) = validate(g, candidate, Source::Steam)
                {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// A human-readable diagnostic of Steam detection, for the startup log.
pub fn diagnostics() -> Vec<String> {
    let mut d = Vec::new();
    let roots = steam_roots();
    d.push(format!("steam roots (registry/known): {}", roots.len()));
    for r in &roots {
        d.push(format!("  root: {}", r.display()));
    }
    let libs = all_steam_libraries();
    d.push(format!(
        "steam libraries (incl. drive probe): {}",
        libs.len()
    ));
    for lib in &libs {
        let sa = find_child_ci(lib, "steamapps").unwrap_or_else(|| lib.join("steamapps"));
        let mut found = Vec::new();
        if sa.is_dir() {
            for ids in GAME_IDS {
                for &appid in ids.steam_appids {
                    if sa.join(format!("appmanifest_{appid}.acf")).is_file() {
                        found.push(format!("{}({appid})", ids.game.id()));
                    }
                }
            }
        }
        d.push(format!(
            "  lib: {} — steamapps {} — manifests: [{}]",
            lib.display(),
            if sa.is_dir() { "ok" } else { "MISSING" },
            found.join(", ")
        ));
    }
    // Launchers (S-08): which configuration folders exist on this machine.
    let heroic = heroic_config_dirs();
    d.push(format!("heroic config folders: {}", heroic.len()));
    for h in &heroic {
        d.push(format!("  heroic: {}", h.display()));
    }
    let lutris = lutris_config_dirs();
    d.push(format!("lutris config folders: {}", lutris.len()));
    for l in &lutris {
        d.push(format!("  lutris: {}", l.display()));
    }
    d.push(format!(
        "gog galaxy / lutris sqlite reader: {}",
        if cfg!(feature = "gog-galaxy") {
            "compiled in"
        } else {
            "not compiled (feature gog-galaxy)"
        }
    ));
    d
}

// --------------------------------------------------------------------------
// GOG — Windows registry
// --------------------------------------------------------------------------

fn detect_gog_registry() -> Vec<DetectedGame> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let mut out = Vec::new();
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for base in [
            "SOFTWARE\\WOW6432Node\\GOG.com\\Games",
            "SOFTWARE\\GOG.com\\Games",
        ] {
            let Ok(games) = hklm.open_subkey(base) else {
                continue;
            };
            for sub in games.enum_keys().flatten() {
                if let Ok(k) = games.open_subkey(&sub)
                    && let Ok(path) = k.get_value::<String, _>("path")
                {
                    out.extend(validate_any(Path::new(&path), Source::GogRegistry));
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

// --------------------------------------------------------------------------
// GOG Galaxy — SQLite database (feature "gog-galaxy")
// --------------------------------------------------------------------------

fn detect_gog_galaxy() -> Vec<DetectedGame> {
    #[cfg(feature = "gog-galaxy")]
    {
        let mut out = Vec::new();
        for db in gog_galaxy_dbs() {
            out.extend(read_gog_galaxy_db(&db));
        }
        out
    }
    #[cfg(not(feature = "gog-galaxy"))]
    {
        Vec::new()
    }
}

#[cfg(feature = "gog-galaxy")]
fn gog_galaxy_dbs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    if let Some(pd) = std::env::var_os("ProgramData") {
        v.push(PathBuf::from(pd).join("GOG.com/Galaxy/storage/galaxy-2.0.db"));
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = dirs::home_dir() {
        v.push(home.join("Library/Application Support/GOG.com/Galaxy/storage/galaxy-2.0.db"));
    }
    v.retain(|p| p.is_file());
    v
}

#[cfg(feature = "gog-galaxy")]
fn read_gog_galaxy_db(db: &Path) -> Vec<DetectedGame> {
    // Open read-only/immutable so a running Galaxy does not block us. The schema
    // is undocumented and version-dependent, so we probe for an installation
    // path column and validate every path by master file anyway.
    let uri = format!("file:{}?mode=ro&immutable=1", db.display());
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Known layouts across Galaxy versions store the install path either in
    // InstalledBaseProducts.installationPath or in a separate InstalledProducts.
    for sql in [
        "SELECT installationPath FROM InstalledBaseProducts",
        "SELECT installationPath FROM InstalledProducts",
    ] {
        if let Ok(mut stmt) = conn.prepare(sql)
            && let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0))
        {
            for p in rows.flatten() {
                out.extend(validate_any(Path::new(&p), Source::GogGalaxy));
            }
        }
    }
    out
}

// --------------------------------------------------------------------------
// Epic Games — JSON manifests
// --------------------------------------------------------------------------

fn epic_manifest_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    if let Some(pd) = std::env::var_os("ProgramData") {
        v.push(PathBuf::from(pd).join("Epic/EpicGamesLauncher/Data/Manifests"));
    }
    #[cfg(not(windows))]
    if let Some(home) = dirs::home_dir()
        && cfg!(target_os = "macos")
    {
        v.push(home.join("Library/Application Support/Epic/EpicGamesLauncher/Data/Manifests"));
        v.push(PathBuf::from(
            "/Users/Shared/Epic/EpicGamesLauncher/Data/Manifests",
        ));
    }
    v.retain(|p| p.is_dir());
    v
}

/// `(InstallLocation, DisplayName)` from an Epic `.item` manifest.
fn parse_epic_manifest(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let loc = v.get("InstallLocation")?.as_str()?.to_string();
    let name = v
        .get("DisplayName")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if loc.is_empty() {
        None
    } else {
        Some((loc, name))
    }
}

fn game_from_epic_name(name: &str) -> Option<Game> {
    let low = name.to_lowercase();
    // Prefer the most specific (longest) matching name so "Skyrim Special
    // Edition" wins over the "Skyrim" substring.
    let mut best: Option<(usize, Game)> = None;
    for ids in GAME_IDS {
        for n in ids.names {
            let nl = n.to_lowercase();
            if low.contains(&nl) && best.map(|(l, _)| nl.len() > l).unwrap_or(true) {
                best = Some((nl.len(), ids.game));
            }
        }
    }
    best.map(|(_, g)| g)
}

fn detect_epic() -> Vec<DetectedGame> {
    let mut out = Vec::new();
    for dir in epic_manifest_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("item") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Some((loc, name)) = parse_epic_manifest(&text) else {
                continue;
            };
            let root = Path::new(&loc);
            // Prefer the game named in the manifest; else try all games.
            match game_from_epic_name(&name) {
                Some(g) => out.extend(validate(g, root.to_path_buf(), Source::Epic)),
                None => out.extend(validate_any(root, Source::Epic)),
            }
        }
    }
    out
}

// --------------------------------------------------------------------------
// Heroic Games Launcher — GOG / Epic / Amazon on Linux, macOS, Windows (S-08)
// --------------------------------------------------------------------------

/// Heroic configuration folders: native, Flatpak, macOS and Windows layouts.
fn heroic_config_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            v.push(home.join("Library/Application Support/heroic"));
        } else if cfg!(windows) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                v.push(PathBuf::from(appdata).join("heroic"));
            }
        } else {
            v.push(home.join(".config/heroic"));
            v.push(home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"));
        }
    }
    v.retain(|p| p.is_dir());
    v
}

/// `(install_path, title)` of the games in Heroic's GOG store file
/// (`gog_store/installed.json`: `{"installed":[{"install_path":…,"appName":…,"platform":…}]}`).
pub fn parse_heroic_gog_installed(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let items = v
        .get("installed")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|it| {
            let path = it.get("install_path").and_then(|p| p.as_str())?.to_string();
            let title = it
                .get("title")
                .or_else(|| it.get("appName"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            (!path.is_empty()).then_some((path, title))
        })
        .collect()
}

/// `(install_path, title)` of the games in Legendary's file (Epic through
/// Heroic: `legendaryConfig/legendary/installed.json`, a map app → entry).
pub fn parse_legendary_installed(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(map) = v.as_object() else {
        return Vec::new();
    };
    map.values()
        .filter_map(|it| {
            let path = it.get("install_path").and_then(|p| p.as_str())?.to_string();
            let title = it
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            (!path.is_empty()).then_some((path, title))
        })
        .collect()
}

/// `(path, id)` of the games in Nile's file (Amazon through Heroic:
/// `nile_config/nile/installed.json`, an array of `{"id":…,"path":…}`).
pub fn parse_nile_installed(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|it| {
                    let path = it.get("path").and_then(|p| p.as_str())?.to_string();
                    let id = it
                        .get("id")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    (!path.is_empty()).then_some((path, id))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Validate a launcher's `(install_path, title)` pairs: the title picks the
/// game when it names one, else every game is tried on the folder. A Windows
/// game run through Wine / Proton is validated on its real folder like any
/// other.
fn validate_launcher_entries(entries: &[(String, String)], source: Source) -> Vec<DetectedGame> {
    let mut out = Vec::new();
    for (path, title) in entries {
        let root = Path::new(path);
        match game_from_epic_name(title) {
            Some(g) => match validate(g, root.to_path_buf(), source) {
                Some(d) => out.push(d),
                None => out.extend(validate_any(root, source)),
            },
            None => out.extend(validate_any(root, source)),
        }
    }
    out
}

/// Heroic's games from one configuration folder (testable with fixtures).
pub fn detect_heroic_in(config_dir: &Path) -> Vec<DetectedGame> {
    let mut entries: Vec<(String, String)> = Vec::new();
    if let Ok(t) = std::fs::read_to_string(config_dir.join("gog_store/installed.json")) {
        entries.extend(parse_heroic_gog_installed(&t));
    }
    if let Ok(t) =
        std::fs::read_to_string(config_dir.join("legendaryConfig/legendary/installed.json"))
    {
        entries.extend(parse_legendary_installed(&t));
    }
    if let Ok(t) = std::fs::read_to_string(config_dir.join("nile_config/nile/installed.json")) {
        entries.extend(parse_nile_installed(&t));
    }
    validate_launcher_entries(&entries, Source::Heroic)
}

fn detect_heroic() -> Vec<DetectedGame> {
    heroic_config_dirs()
        .iter()
        .flat_map(|d| detect_heroic_in(d))
        .collect()
}

// --------------------------------------------------------------------------
// Lutris — Linux (S-08)
// --------------------------------------------------------------------------

/// Lutris configuration folders holding `games/*.yml` (native, Flatpak).
fn lutris_config_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(target_os = "linux")
        && let Some(home) = dirs::home_dir()
    {
        v.push(home.join(".config/lutris"));
        v.push(home.join(".var/app/net.lutris.Lutris/config/lutris"));
    }
    v.retain(|p| p.is_dir());
    v
}

/// Lutris data folders holding `pga.db` (native, Flatpak).
#[cfg_attr(not(feature = "gog-galaxy"), allow(dead_code))]
fn lutris_data_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(target_os = "linux")
        && let Some(home) = dirs::home_dir()
    {
        v.push(home.join(".local/share/lutris"));
        v.push(home.join(".var/app/net.lutris.Lutris/data/lutris"));
    }
    v.retain(|p| p.is_dir());
    v
}

/// The candidate game folders described by a Lutris game YAML: the folder of
/// `exe` (absolute, or relative to `prefix` / `working_dir`), plus
/// `working_dir` and `directory` when given. No YAML crate: the keys are
/// read line by line, which is all these files need.
pub fn lutris_yaml_roots(yaml: &str) -> Vec<PathBuf> {
    let mut exe: Option<String> = None;
    let mut prefix: Option<String> = None;
    let mut working_dir: Option<String> = None;
    let mut directory: Option<String> = None;
    for line in yaml.lines() {
        let t = line.trim();
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "exe" => exe = Some(value.to_string()),
            "prefix" => prefix = Some(value.to_string()),
            "working_dir" => working_dir = Some(value.to_string()),
            "directory" => directory = Some(value.to_string()),
            _ => {}
        }
    }
    let expand = |p: &str| -> PathBuf {
        if let Some(rest) = p.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(rest);
        }
        PathBuf::from(p)
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(e) = &exe {
        let exe_path = if Path::new(e).is_absolute() {
            PathBuf::from(e)
        } else if let Some(base) = working_dir.as_deref().or(prefix.as_deref()) {
            expand(base).join(e)
        } else {
            PathBuf::from(e)
        };
        if let Some(parent) = exe_path.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    for d in [&working_dir, &directory].into_iter().flatten() {
        roots.push(expand(d));
    }
    roots.dedup();
    roots
}

/// Lutris' games from one configuration folder (testable with fixtures).
pub fn detect_lutris_in(config_dir: &Path) -> Vec<DetectedGame> {
    let mut out = Vec::new();
    let games = config_dir.join("games");
    let Ok(rd) = std::fs::read_dir(&games) else {
        return out;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("yml") | Some("yaml")
        ) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for root in lutris_yaml_roots(&text) {
            // The executable may sit one level below the game root
            // (`Skyrim/SkyrimSE.exe` → root) or in the root itself.
            out.extend(validate_any(&root, Source::Lutris));
            if let Some(up) = root.parent() {
                out.extend(validate_any(up, Source::Lutris));
            }
        }
    }
    out
}

/// Lutris' games from its SQLite database (`pga.db`, table `games`:
/// `directory`), when the SQLite reader is compiled in.
#[cfg(feature = "gog-galaxy")]
fn detect_lutris_db(db: &Path) -> Vec<DetectedGame> {
    let uri = format!("file:{}?mode=ro&immutable=1", db.display());
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT directory FROM games WHERE directory IS NOT NULL AND directory != ''")
        && let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0))
    {
        for p in rows.flatten() {
            out.extend(validate_any(Path::new(&p), Source::Lutris));
        }
    }
    out
}

fn detect_lutris() -> Vec<DetectedGame> {
    #[allow(unused_mut)]
    let mut out: Vec<DetectedGame> = lutris_config_dirs()
        .iter()
        .flat_map(|d| detect_lutris_in(d))
        .collect();
    #[cfg(feature = "gog-galaxy")]
    for d in lutris_data_dirs() {
        let db = d.join("pga.db");
        if db.is_file() {
            out.extend(detect_lutris_db(&db));
        }
    }
    out
}

// --------------------------------------------------------------------------
// Xbox Game Pass PC — .GamingRoot (Windows, best effort)
// --------------------------------------------------------------------------

/// The relative games folder encoded in a drive's `.GamingRoot` file
/// (magic `RGBX`, a u32, then a NUL-terminated UTF-16LE path). Only used by the
/// Windows Xbox reader (and the tests), hence the allow on other targets.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn parse_gaming_root(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 8 || &bytes[0..4] != b"RGBX" {
        return None;
    }
    let u16s: Vec<u16> = bytes[8..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    let s = String::from_utf16(&u16s).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

fn detect_xbox() -> Vec<DetectedGame> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let marker = PathBuf::from(&drive).join(".GamingRoot");
            let Ok(bytes) = std::fs::read(&marker) else {
                continue;
            };
            let Some(rel) = parse_gaming_root(&bytes) else {
                continue;
            };
            let games_dir = PathBuf::from(&drive).join(rel);
            let Ok(rd) = std::fs::read_dir(&games_dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                // MS Store games keep their files under a "Content" sub-folder.
                let content = find_child_ci(&p, "Content").unwrap_or(p.clone());
                out.extend(validate_any(&content, Source::Xbox));
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

// --------------------------------------------------------------------------
// Bounded folder scan (CD/DVD, manual installs)
// --------------------------------------------------------------------------

/// Common install roots to scan (shallow), per OS.
fn scan_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(var) {
                v.push(PathBuf::from(p));
            }
        }
        for letter in b'C'..=b'H' {
            let d = PathBuf::from(format!("{}:\\", letter as char));
            v.push(d.join("Games"));
            v.push(d.join("GOG Games"));
        }
    }

    #[cfg(not(windows))]
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            v.push(PathBuf::from("/Applications"));
            v.push(home.join("Applications"));
        } else {
            v.push(home.join("Games"));
            v.push(home.join("GOG Games"));
            v.push(PathBuf::from("/opt"));
            v.push(home.join(".wine/drive_c/Program Files"));
            v.push(home.join(".wine/drive_c/Program Files (x86)"));
            v.push(home.join(".wine/drive_c/Games"));
        }
    }

    v.retain(|p| p.is_dir());
    v
}

/// Depth-bounded scan: test every visited folder (down to `depth` levels) for a
/// master file, so a game sitting a level or two under a common root is found
/// without walking the whole disk.
fn scan_dir(dir: &Path, depth: usize, out: &mut Vec<DetectedGame>) {
    out.extend(validate_any(dir, Source::FolderScan));
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            scan_dir(&p, depth - 1, out);
        }
    }
}

fn detect_scan() -> Vec<DetectedGame> {
    let mut out = Vec::new();
    for root in scan_roots() {
        scan_dir(&root, 2, &mut out);
    }
    out
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_extracts_values() {
        let text = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        let paths = vdf_values(text, "path");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], "C:\\Program Files (x86)\\Steam");
        assert_eq!(paths[1], "D:\\SteamLibrary");
    }

    #[test]
    fn appmanifest_installdir() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"489830\"\n\t\"installdir\"\t\t\"Skyrim Special Edition\"\n}\n";
        assert_eq!(
            vdf_value(acf, "installdir"),
            Some("Skyrim Special Edition".into())
        );
    }

    #[test]
    fn steam_appid_maps_to_game() {
        assert_eq!(game_from_steam_appid(489830), Some(Game::SkyrimSE));
        assert_eq!(game_from_steam_appid(377160), Some(Game::Fallout4));
        assert_eq!(game_from_steam_appid(1716740), Some(Game::Starfield));
        assert_eq!(game_from_steam_appid(1), None);
    }

    #[test]
    fn epic_manifest_parsed() {
        let json = r#"{"InstallLocation":"C:\\Games\\Skyrim Special Edition","DisplayName":"Skyrim Special Edition","AppName":"abc"}"#;
        let (loc, name) = parse_epic_manifest(json).unwrap();
        assert!(loc.ends_with("Skyrim Special Edition"));
        assert_eq!(game_from_epic_name(&name), Some(Game::SkyrimSE));
    }

    #[test]
    fn gaming_root_parsed() {
        // "RGBX" + u32(1) + "XboxGames" UTF-16LE + NUL
        let mut bytes = b"RGBX".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for u in "XboxGames".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(parse_gaming_root(&bytes).as_deref(), Some("XboxGames"));
        assert_eq!(parse_gaming_root(b"nope"), None);
    }

    #[test]
    fn game_from_master_maps_old_games() {
        assert_eq!(game_from_master_file("Oblivion.esm"), Some(Game::Oblivion));
        assert_eq!(
            game_from_master_file("MORROWIND.ESM"),
            Some(Game::Morrowind)
        );
        assert_eq!(game_from_master_file("Fallout3.esm"), Some(Game::Fallout3));
        assert_eq!(
            game_from_master_file("FalloutNV.esm"),
            Some(Game::FalloutNV)
        );
        assert_eq!(game_from_master_file("SomeMod.esp"), None);
    }

    #[test]
    fn oblivion_remastered_nested_data_path() {
        let root = std::env::temp_dir().join("ximod_oblivionrd_test");
        let _ = std::fs::remove_dir_all(&root);
        let inst = root.join("Oblivion Remastered");
        let data = inst.join("OblivionRemastered/Content/Dev/ObvData/Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Oblivion.esm"), b"x").unwrap();
        // Recognized as the remaster via its nested data path…
        assert!(is_game_at(Game::OblivionRD, &inst));
        // …and NOT as the original Oblivion (no top-level Data/Oblivion.esm).
        assert!(!is_game_at(Game::Oblivion, &inst));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_game_at_validates_paths() {
        let root = std::env::temp_dir().join("ximod_isgameat_test");
        let _ = std::fs::remove_dir_all(&root);
        let inst = root.join("Skyrim Special Edition");
        std::fs::create_dir_all(inst.join("Data")).unwrap();
        std::fs::write(inst.join("Data").join("Skyrim.esm"), b"x").unwrap();
        // Correct path holds the game; a stale/wrong path does not.
        assert!(is_game_at(Game::SkyrimSE, &inst));
        assert!(!is_game_at(Game::SkyrimSE, &root.join("Old Wrong Path")));
        // Right folder, wrong game (master mismatch) → false.
        assert!(!is_game_at(Game::Fallout4, &inst));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn master_file_gate_and_dedup() {
        // Build a fake install tree and check validation + case-insensitive data.
        let root = std::env::temp_dir().join("ximod_detect_test");
        let _ = std::fs::remove_dir_all(&root);
        let inst = root.join("Skyrim Special Edition");
        std::fs::create_dir_all(inst.join("Data")).unwrap();
        std::fs::write(inst.join("Data").join("Skyrim.esm"), b"x").unwrap();

        // SkyrimSE and Skyrim share Skyrim.esm → validate_any returns both.
        let hits = validate_any(&inst, Source::FolderScan);
        assert!(hits.iter().any(|h| h.game == Game::SkyrimSE));
        assert!(hits.iter().any(|h| h.game == Game::Skyrim));

        // A folder without the master file is rejected.
        assert!(validate(Game::Fallout4, inst.clone(), Source::FolderScan).is_none());

        // Dedup keeps one per game, preferring the better source.
        let mut cands = vec![
            DetectedGame {
                game: Game::SkyrimSE,
                root: inst.clone(),
                data_dir: inst.join("Data"),
                source: Source::FolderScan,
            },
            DetectedGame {
                game: Game::SkyrimSE,
                root: inst.clone(),
                data_dir: inst.join("Data"),
                source: Source::Steam,
            },
        ];
        cands = dedup(cands);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].source, Source::Steam);

        let _ = std::fs::remove_dir_all(&root);
    }
}
