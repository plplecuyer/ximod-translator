//! Per-game specification sheets (`assets/data/games/<id>.json`).
//!
//! A [`GameSpec`] describes what the engine needs to know about one title:
//! its plugin extensions and master files, the form-version range used for
//! detection, the record-header regime, text encoding, where external STRINGS
//! live, the applicable output modes and the translatable-field file (A-06).
//!
//! The sheets shipped in `assets/data/games/` are the reference. Built-in
//! defaults ([`GameSpec::builtin`]) reproduce them exactly so that tests and a
//! damaged installation keep working; a parity test guards that equality.
//! Adding a game of an existing engine is then a matter of data: a JSON sheet
//! plus its field file (S-05, R-S05-01).

use crate::Game;
use crate::fields::{FieldLoadIssue, FieldTable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// Which plugin engine reads / writes the game's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    /// TES4 container: Oblivion → Starfield (localized plugins included).
    Tes4,
    /// TES3 container: Morrowind.
    Tes3,
    /// Key / value text files (MCM, INI, JSON…): no game of its own.
    Text,
}

/// How the game names its language-specific STRINGS files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LangStyle {
    /// `<plugin>_French.strings` (Skyrim, Fallout 4).
    Name,
    /// `<plugin>_fr.strings` — to be confirmed on the game files (Starfield).
    Iso2,
}

/// Output modes a game can produce. Mirrors the application's `OutputMode`
/// without depending on it (this crate has no GUI / app dependency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputModeId {
    Strings,
    Hybrid,
    OverridePatch,
    FullPlugin,
    Dsd,
}

/// One game's sheet. Field names are the JSON keys (camelCase on disk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSpec {
    /// Schema version of the sheet (currently 1).
    pub format: u32,
    /// Stable identifier, equal to [`Game::id`] and to the file stem.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    pub engine: EngineKind,
    /// Plugin extensions, lowercase, without dot.
    pub plugin_extensions: Vec<String>,
    /// Master files that identify the game (case-insensitive).
    pub masters: Vec<String>,
    /// Inclusive form-version range, when the engine has one.
    #[serde(default)]
    pub form_versions: Option<(u16, u16)>,
    /// Record / GRUP header length in bytes (16, 20 or 24).
    pub header_len: u8,
    /// Text encoding: UTF-8 (true) or Windows-1252 (false).
    pub utf8: bool,
    /// Whether external STRINGS files exist for this game.
    pub localized_strings: bool,
    /// Sub-folder of `Data` holding STRINGS files.
    pub strings_dir: String,
    pub strings_lang_style: LangStyle,
    /// Archive extension, lowercase, without dot.
    pub archive_extension: String,
    /// Whether `LOC_*` keys must be locked (Oblivion Remastered layer).
    pub locks_loc_keys: bool,
    pub output_modes: Vec<OutputModeId>,
    /// Stem of the translatable-field file under `assets/data/fields/`.
    pub fields_file: String,
}

impl GameSpec {
    /// Built-in sheet for a game — the exact values the code used before the
    /// sheets were externalized. Kept in sync with `assets/data/games/*.json`
    /// by the `specs_match_builtin` test.
    pub fn builtin(game: Game) -> GameSpec {
        use EngineKind::*;
        use OutputModeId::*;
        let esp = || vec!["esp".to_string(), "esm".to_string()];
        let esp_esl = || vec!["esp".to_string(), "esm".to_string(), "esl".to_string()];
        let all_modes = |dsd: bool| {
            let mut v = vec![Strings, Hybrid, OverridePatch, FullPlugin];
            if dsd {
                v.push(Dsd);
            }
            v
        };
        let (name, engine, exts, masters, fv, hl, utf8, loc, arch, lock, modes) = match game {
            Game::Morrowind => (
                "The Elder Scrolls III: Morrowind",
                Tes3,
                esp(),
                vec!["Morrowind.esm"],
                None,
                16,
                false,
                false,
                "bsa",
                false,
                vec![FullPlugin],
            ),
            Game::Oblivion => (
                "The Elder Scrolls IV: Oblivion",
                Tes4,
                esp(),
                vec!["Oblivion.esm"],
                None,
                20,
                false,
                false,
                "bsa",
                true,
                vec![OverridePatch, FullPlugin],
            ),
            Game::OblivionRD => (
                "The Elder Scrolls IV: Oblivion Remastered",
                Tes4,
                esp(),
                vec!["Oblivion.esm"],
                None,
                20,
                false,
                false,
                "bsa",
                true,
                vec![OverridePatch, FullPlugin],
            ),
            Game::Skyrim => (
                "The Elder Scrolls V: Skyrim (Legendary)",
                Tes4,
                esp(),
                vec!["Skyrim.esm"],
                Some((40, 43)),
                24,
                false,
                true,
                "bsa",
                false,
                all_modes(false),
            ),
            Game::SkyrimSE => (
                "The Elder Scrolls V: Skyrim Special Edition",
                Tes4,
                esp_esl(),
                vec!["Skyrim.esm"],
                Some((44, 44)),
                24,
                true,
                true,
                "bsa",
                false,
                all_modes(true),
            ),
            Game::Fallout3 => (
                "Fallout 3",
                Tes4,
                esp(),
                vec!["Fallout3.esm"],
                Some((2, 15)),
                24,
                false,
                // Texts inline in the plugin: no STRINGS files before Skyrim.
                false,
                "bsa",
                false,
                vec![OverridePatch, FullPlugin],
            ),
            Game::FalloutNV => (
                "Fallout: New Vegas",
                Tes4,
                esp(),
                vec!["FalloutNV.esm"],
                Some((2, 15)),
                24,
                false,
                // Texts inline in the plugin: no STRINGS files before Skyrim.
                false,
                "bsa",
                false,
                vec![OverridePatch, FullPlugin],
            ),
            Game::Fallout4 => (
                "Fallout 4",
                Tes4,
                esp_esl(),
                vec!["Fallout4.esm"],
                Some((131, 131)),
                24,
                true,
                true,
                "ba2",
                false,
                all_modes(false),
            ),
            Game::Fallout76 => (
                "Fallout 76",
                Tes4,
                esp_esl(),
                vec!["SeventySix.esm"],
                Some((182, 201)),
                24,
                true,
                true,
                "ba2",
                false,
                all_modes(false),
            ),
            Game::Starfield => (
                "Starfield",
                Tes4,
                esp_esl(),
                vec!["Starfield.esm"],
                Some((552, u16::MAX)),
                24,
                true,
                true,
                "ba2",
                false,
                all_modes(false),
            ),
            Game::Unknown => (
                "Unknown game",
                Tes4,
                esp_esl(),
                vec![],
                None,
                24,
                false,
                true,
                "bsa",
                false,
                all_modes(false),
            ),
        };
        GameSpec {
            format: 1,
            id: game.id().to_string(),
            name: name.to_string(),
            engine,
            plugin_extensions: exts,
            masters: masters.into_iter().map(String::from).collect(),
            form_versions: fv,
            header_len: hl,
            utf8,
            localized_strings: loc,
            strings_dir: "Strings".to_string(),
            // Starfield names its STRINGS with ISO 639-1 codes
            // (`Starfield_fr.strings`); the older games use the English
            // language name (`Skyrim_French.strings`).
            strings_lang_style: if game == Game::Starfield {
                LangStyle::Iso2
            } else {
                LangStyle::Name
            },
            archive_extension: arch.to_string(),
            locks_loc_keys: lock,
            output_modes: modes,
            fields_file: game.id().to_lowercase(),
        }
    }

    /// Parse a sheet from JSON, checking the schema version and the id.
    pub fn from_json(text: &str, expected_id: Option<&str>) -> Result<GameSpec, String> {
        let spec: GameSpec = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if spec.format != 1 {
            return Err(format!(
                "format {} non pris en charge (attendu 1)",
                spec.format
            ));
        }
        if let Some(id) = expected_id
            && spec.id != id
        {
            return Err(format!("id « {} » ≠ nom du fichier « {id} »", spec.id));
        }
        Ok(spec)
    }

    /// Pretty JSON, as written in `assets/data/games/`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("GameSpec serializes")
    }
}

/// A problem met while loading a sheet; the built-in sheet is used instead.
#[derive(Debug, Clone)]
pub struct LoadIssue {
    pub game: Game,
    pub path: std::path::PathBuf,
    pub reason: String,
}

/// All the game sheets and translatable-field tables, keyed by [`Game`].
#[derive(Debug, Clone)]
pub struct GameRegistry {
    specs: BTreeMap<&'static str, GameSpec>,
    fields: BTreeMap<&'static str, FieldTable>,
    /// Where the field files are read from (shipped, user), to reload a game.
    fields_dirs: Option<(PathBuf, Option<PathBuf>)>,
}

impl Default for GameRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl GameRegistry {
    /// Registry made only of the built-in sheets.
    pub fn builtin() -> Self {
        let mut specs = BTreeMap::new();
        let mut fields = BTreeMap::new();
        for g in Game::ALL.iter().chain(std::iter::once(&Game::Unknown)) {
            specs.insert(g.id(), GameSpec::builtin(*g));
        }
        for g in Game::ALL {
            fields.insert(g.id(), FieldTable::builtin(*g));
        }
        GameRegistry {
            specs,
            fields,
            fields_dirs: None,
        }
    }

    /// Load `<games_dir>/<id>.json` for every known game; a missing or invalid
    /// sheet falls back to the built-in one and is reported. Field tables stay
    /// built-in until [`load_fields`](Self::load_fields).
    pub fn load(games_dir: &Path) -> (Self, Vec<LoadIssue>) {
        let mut reg = Self::builtin();
        let mut issues = Vec::new();
        for g in Game::ALL {
            let path = games_dir.join(format!("{}.json", g.id()));
            match std::fs::read_to_string(&path) {
                Ok(text) => match GameSpec::from_json(&text, Some(g.id())) {
                    Ok(spec) => {
                        reg.specs.insert(g.id(), spec);
                    }
                    Err(reason) => issues.push(LoadIssue {
                        game: *g,
                        path,
                        reason,
                    }),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => issues.push(LoadIssue {
                    game: *g,
                    path,
                    reason: e.to_string(),
                }),
            }
        }
        (reg, issues)
    }

    /// Write every built-in sheet to `<games_dir>/<id>.json` (used once to
    /// generate the shipped files, and by tests).
    pub fn write_builtin(games_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(games_dir)?;
        for g in Game::ALL {
            std::fs::write(
                games_dir.join(format!("{}.json", g.id())),
                GameSpec::builtin(*g).to_json() + "\n",
            )?;
        }
        Ok(())
    }

    pub fn spec(&self, game: Game) -> &GameSpec {
        self.specs
            .get(game.id())
            .unwrap_or_else(|| &self.specs["Unknown"])
    }

    /// Load the translatable-field tables of every game from
    /// `<shipped_dir>/<id>.json`, a file of the same name under `user_dir`
    /// taking precedence (A-06, R-A06-01/07). Invalid files are reported and
    /// the next source used.
    pub fn load_fields(
        &mut self,
        shipped_dir: &Path,
        user_dir: Option<&Path>,
    ) -> Vec<FieldLoadIssue> {
        self.fields_dirs = Some((shipped_dir.to_path_buf(), user_dir.map(Path::to_path_buf)));
        let mut issues = Vec::new();
        for g in Game::ALL {
            let (t, mut i) = FieldTable::load(shipped_dir, user_dir, *g);
            self.fields.insert(g.id(), t);
            issues.append(&mut i);
        }
        issues
    }

    /// Reload one game's table from disk (after the editor saved or restored
    /// its user file).
    pub fn reload_fields(&mut self, game: Game) -> Vec<FieldLoadIssue> {
        let Some((shipped, user)) = self.fields_dirs.clone() else {
            return Vec::new();
        };
        let (t, issues) = FieldTable::load(&shipped, user.as_deref(), game);
        self.fields.insert(game.id(), t);
        issues
    }

    /// The field table of a game (`Unknown` → Oblivion's, i.e. the common
    /// TES4 base set, so an unrecognised plugin still shows something).
    pub fn fields(&self, game: Game) -> &FieldTable {
        self.fields
            .get(game.id())
            .unwrap_or_else(|| &self.fields[Game::Oblivion.id()])
    }

    /// `(shipped, user)` field folders given to [`load_fields`](Self::load_fields).
    pub fn fields_dirs(&self) -> Option<(&Path, Option<&Path>)> {
        self.fields_dirs
            .as_ref()
            .map(|(s, u)| (s.as_path(), u.as_deref()))
    }

    /// Every user-selectable game, in [`Game::ALL`] order.
    pub fn games(&self) -> impl Iterator<Item = (Game, &GameSpec)> + '_ {
        Game::ALL.iter().map(move |g| (*g, self.spec(*g)))
    }

    /// Union of the plugin extensions of every game, lowercase, in first-seen
    /// order (for file dialogs that accept any game's plugin).
    pub fn all_plugin_extensions(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, spec) in self.games() {
            for e in &spec.plugin_extensions {
                if !out.iter().any(|x| x == e) {
                    out.push(e.clone());
                }
            }
        }
        out
    }

    /// Whether `ext` (any case, no dot) is a plugin extension of some game.
    pub fn is_plugin_extension(&self, ext: &str) -> bool {
        let low = ext.to_lowercase();
        self.games()
            .any(|(_, s)| s.plugin_extensions.contains(&low))
    }

    /// The game whose master list contains `file_name` (case-insensitive), if any.
    pub fn game_for_master(&self, file_name: &str) -> Option<Game> {
        let low = file_name.to_lowercase();
        Game::ALL.iter().copied().find(|g| {
            self.spec(*g)
                .masters
                .iter()
                .any(|m| m.to_lowercase() == low)
        })
    }
}

static REGISTRY: OnceLock<RwLock<GameRegistry>> = OnceLock::new();

fn cell() -> &'static RwLock<GameRegistry> {
    REGISTRY.get_or_init(|| RwLock::new(GameRegistry::builtin()))
}

/// Install the registry loaded from `games_dir` and the field tables from
/// `fields_dir` / `user_fields_dir` (call once at startup, after the folders
/// are known). Returns the load issues to report.
pub fn init(
    games_dir: &Path,
    fields_dir: &Path,
    user_fields_dir: Option<&Path>,
) -> (Vec<LoadIssue>, Vec<FieldLoadIssue>) {
    let (mut reg, issues) = GameRegistry::load(games_dir);
    let field_issues = reg.load_fields(fields_dir, user_fields_dir);
    *cell().write().expect("registry lock") = reg;
    (issues, field_issues)
}

/// Reload one game's field table from disk into the process-wide registry
/// (editor: after saving or restoring the user file).
pub fn reload_fields(game: Game) -> Vec<FieldLoadIssue> {
    cell().write().expect("registry lock").reload_fields(game)
}

/// Read access to the process-wide registry (built-in sheets until [`init`]).
pub fn registry() -> RegistryGuard {
    RegistryGuard(cell().read().expect("registry lock"))
}

/// Read guard over the registry; derefs to [`GameRegistry`].
pub struct RegistryGuard(std::sync::RwLockReadGuard<'static, GameRegistry>);

impl std::ops::Deref for RegistryGuard {
    type Target = GameRegistry;
    fn deref(&self) -> &GameRegistry {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_matches_previous_code() {
        // Values the methods returned before externalization.
        assert!(Game::SkyrimSE.uses_utf8() && !Game::Skyrim.uses_utf8());
        assert!(
            Game::Starfield.uses_utf8()
                && Game::Fallout76.uses_utf8()
                && Game::Fallout4.uses_utf8()
        );
        assert_eq!(Game::Oblivion.header_len(), 20);
        assert_eq!(Game::Morrowind.header_len(), 16);
        assert_eq!(Game::Starfield.header_len(), 24);
        assert!(Game::SkyrimSE.supports_dsd() && !Game::Starfield.supports_dsd());
        assert!(Game::Oblivion.locks_loc_keys() && !Game::Skyrim.locks_loc_keys());
        assert!(!Game::Oblivion.uses_localized_strings() && Game::Skyrim.uses_localized_strings());
        assert!(!Game::Morrowind.uses_localized_strings());
    }

    #[test]
    fn json_round_trip_and_id_check() {
        let s = GameSpec::builtin(Game::Fallout4);
        let back = GameSpec::from_json(&s.to_json(), Some("Fallout4")).unwrap();
        assert_eq!(s, back);
        assert!(GameSpec::from_json(&s.to_json(), Some("Skyrim")).is_err());
    }

    #[test]
    fn load_falls_back_and_reports() {
        let dir = std::env::temp_dir().join(format!("ximod-games-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        GameRegistry::write_builtin(&dir).unwrap();
        std::fs::write(dir.join("Skyrim.json"), "{ not json").unwrap();
        let (reg, issues) = GameRegistry::load(&dir);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].game, Game::Skyrim);
        assert_eq!(reg.spec(Game::Skyrim), &GameSpec::builtin(Game::Skyrim));
        assert_eq!(reg.spec(Game::Fallout4), &GameSpec::builtin(Game::Fallout4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extensions_union() {
        let reg = GameRegistry::builtin();
        assert_eq!(reg.all_plugin_extensions(), vec!["esp", "esm", "esl"]);
        assert!(reg.is_plugin_extension("ESL") && !reg.is_plugin_extension("eet"));
    }

    #[test]
    fn master_lookup() {
        let reg = GameRegistry::builtin();
        assert_eq!(reg.game_for_master("skyrim.esm"), Some(Game::Skyrim));
        assert_eq!(reg.game_for_master("SeventySix.esm"), Some(Game::Fallout76));
        assert_eq!(reg.game_for_master("nope.esm"), None);
    }
}
