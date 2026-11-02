//! Per-game DLC plugin bases, listed in a user-editable JSON file.
//!
//! Localized games (Skyrim LE/SE/AE, Fallout 4/76, Starfield) keep the STRINGS
//! of each DLC either loose in `Data/Strings` (picked up automatically) or packed
//! inside the DLC's own BSA/BA2. Those DLC archives are NOT named
//! interface/localization, so the startup builder needs to know which extra
//! archives to open. This list provides those DLC plugin base names, grouped by
//! game, in `assets/data/GamesDLC.json` — the user can add newly released DLC or
//! Creation Club content by editing that file.

use std::collections::BTreeMap;
use std::path::Path;

/// DLC plugin bases per game id (e.g. `"Fallout4" → ["DLCRobot", …]`).
pub type DlcMap = BTreeMap<String, Vec<String>>;

/// The file name, kept in `assets/data`.
pub const FILE_NAME: &str = "GamesDLC.json";

/// The built-in defaults written on first run (official expansions). Community
/// Creation Club content can be added by the user.
pub fn defaults() -> DlcMap {
    let mut m = DlcMap::new();
    let put = |m: &mut DlcMap, k: &str, v: &[&str]| {
        m.insert(k.to_string(), v.iter().map(|s| s.to_string()).collect());
    };
    // Skyrim (Legendary) and Skyrim Special/Anniversary Edition share the three
    // official expansions; AE adds Creation Club content the user can append.
    put(
        &mut m,
        "Skyrim",
        &["Dawnguard", "HearthFires", "Dragonborn"],
    );
    put(
        &mut m,
        "SkyrimSE",
        &["Dawnguard", "HearthFires", "Dragonborn"],
    );
    put(
        &mut m,
        "Fallout4",
        &[
            "DLCRobot",      // Automatron
            "DLCworkshop01", // Wasteland Workshop
            "DLCCoast",      // Far Harbor
            "DLCworkshop02", // Contraptions Workshop
            "DLCworkshop03", // Vault-Tec Workshop
            "DLCNukaWorld",  // Nuka-World
        ],
    );
    // Fallout 76 folds its updates into the base SeventySix archives — no discrete
    // DLC plugins; left empty (add entries here if a future build ships them).
    put(&mut m, "Fallout76", &[]);
    put(&mut m, "Starfield", &["ShatteredSpace"]);
    m
}

/// Load `GamesDLC.json` from `dir`, creating it with [`defaults`] when absent.
/// A malformed file is ignored (falls back to defaults) rather than failing the
/// launch. Returns the per-game DLC bases.
pub fn load_or_create(dir: &Path) -> DlcMap {
    let path = dir.join(FILE_NAME);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(m) = serde_json::from_str::<DlcMap>(&text) {
            return m;
        }
        // Unreadable/invalid JSON: keep the user's file, use defaults in memory.
        return defaults();
    }
    let def = defaults();
    if let Ok(text) = serde_json::to_string_pretty(&def) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, text);
    }
    def
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_default_then_reads_it() {
        let dir = std::env::temp_dir().join("ximod_dlc_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // First call creates the file from defaults.
        let m1 = load_or_create(&dir);
        assert!(dir.join(FILE_NAME).is_file());
        assert!(m1.get("Fallout4").unwrap().iter().any(|d| d == "DLCCoast"));
        assert!(
            m1.get("Starfield")
                .unwrap()
                .iter()
                .any(|d| d == "ShatteredSpace")
        );

        // Second call reads the (now existing) file.
        let m2 = load_or_create(&dir);
        assert_eq!(m1, m2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
