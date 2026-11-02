//! `ximod-games` — everything that *describes* a Bethesda game for XIMOD
//! Translator and its explorers: the [`Game`] identifiers, the per-game
//! [`spec::GameSpec`] sheets (loaded from `assets/data/games/<id>.json`, with
//! built-in defaults), and the translatable-field tables.
//!
//! The crate has no GUI dependency so that the main application and the
//! `esm-explorer` share one source of truth (S-05, lot 1).

pub mod spec;

pub mod fields;
pub use fields::{
    ChainLink, ContextRule, FieldCtx, FieldFile, FieldLoadIssue, FieldMode, FieldRule, FieldSource,
    FieldTable,
};
pub use spec::{EngineKind, GameRegistry, GameSpec, LangStyle, OutputModeId, registry};

/// Human-readable name for a known vanilla/DLC plugin, matched on the file name
/// (case-insensitive, extension ignored). Mainly useful for the opaque DLC names
/// — e.g. Fallout 4's `DLCworkshop03.esm` → "Vault-Tec Workshop". Returns `None`
/// for unrecognized files.
pub fn plugin_display_name(file_name: &str) -> Option<&'static str> {
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name)
        .to_lowercase();
    let name = match stem.as_str() {
        // Fallout 4 add-ons.
        "dlcrobot" => "Automatron",
        "dlcworkshop01" => "Wasteland Workshop",
        "dlccoast" => "Far Harbor",
        "dlcworkshop02" => "Contraptions Workshop",
        "dlcworkshop03" => "Vault-Tec Workshop",
        "dlcnukaworld" => "Nuka-World",
        // Skyrim add-ons.
        "dawnguard" => "Dawnguard",
        "hearthfires" => "Hearthfires",
        "dragonborn" => "Dragonborn",
        _ => return None,
    };
    Some(name)
}

/// A supported Bethesda title (or engine generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Game {
    Morrowind,
    Oblivion,
    /// The Elder Scrolls IV: Oblivion Remastered (2025, UE5 shell over the
    /// original Gamebryo data). A separate Steam app with its own nested data
    /// folder, kept distinct from the original Oblivion.
    OblivionRD,
    Skyrim,
    SkyrimSE,
    Fallout3,
    FalloutNV,
    Fallout4,
    Fallout76,
    Starfield,
    /// Unknown / not yet identified.
    #[default]
    Unknown,
}

impl Game {
    /// Short, stable identifier (used in config and the EET "GAME" field).
    pub fn id(&self) -> &'static str {
        match self {
            Game::Morrowind => "Morrowind",
            Game::Oblivion => "Oblivion",
            Game::OblivionRD => "OblivionRD",
            Game::Skyrim => "Skyrim",
            Game::SkyrimSE => "SkyrimSE",
            Game::Fallout3 => "Fallout3",
            Game::FalloutNV => "FalloutNV",
            Game::Fallout4 => "Fallout4",
            Game::Fallout76 => "Fallout76",
            Game::Starfield => "Starfield",
            Game::Unknown => "Unknown",
        }
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Game::Morrowind => "The Elder Scrolls III: Morrowind",
            Game::Oblivion => "The Elder Scrolls IV: Oblivion",
            Game::OblivionRD => "The Elder Scrolls IV: Oblivion Remastered",
            Game::Skyrim => "The Elder Scrolls V: Skyrim (Legendary)",
            Game::SkyrimSE => "The Elder Scrolls V: Skyrim Special Edition",
            Game::Fallout3 => "Fallout 3",
            Game::FalloutNV => "Fallout: New Vegas",
            Game::Fallout4 => "Fallout 4",
            Game::Fallout76 => "Fallout 76",
            Game::Starfield => "Starfield",
            Game::Unknown => "Unknown game",
        }
    }

    /// Every user-selectable game (excludes [`Game::Unknown`]), in release order.
    /// Used to populate the game drop-down of the "convert to ximdb" dialog.
    pub const ALL: &'static [Game] = &[
        Game::Morrowind,
        Game::Oblivion,
        Game::OblivionRD,
        Game::Skyrim,
        Game::SkyrimSE,
        Game::Fallout3,
        Game::FalloutNV,
        Game::Fallout4,
        Game::Fallout76,
        Game::Starfield,
    ];

    /// Parse from a stable id (as stored in config / EET files).
    pub fn from_id(s: &str) -> Game {
        match s {
            "Morrowind" => Game::Morrowind,
            "Oblivion" => Game::Oblivion,
            "OblivionRD" => Game::OblivionRD,
            "Skyrim" => Game::Skyrim,
            "SkyrimSE" => Game::SkyrimSE,
            "Fallout3" => Game::Fallout3,
            "FalloutNV" => Game::FalloutNV,
            "Fallout4" => Game::Fallout4,
            "Fallout76" => Game::Fallout76,
            "Starfield" => Game::Starfield,
            _ => Game::Unknown,
        }
    }

    /// Whether this game/engine uses external localized STRINGS files
    /// (TESV Skyrim and later). Older engines store text inline in the plugin.
    pub fn uses_localized_strings(&self) -> bool {
        crate::spec::registry().spec(*self).localized_strings
    }

    /// Whether Dynamic String Distributor (SKSE) output is applicable.
    /// DSD targets Skyrim Special Edition / Anniversary Edition only.
    pub fn supports_dsd(&self) -> bool {
        crate::spec::registry()
            .spec(*self)
            .output_modes
            .contains(&crate::spec::OutputModeId::Dsd)
    }

    /// Whether strings are stored as UTF-8 (the 64-bit Creation Engine titles).
    /// The older engines use the Windows-1252 code page instead. Governs how the
    /// write engine encodes translated zstrings.
    pub fn uses_utf8(&self) -> bool {
        crate::spec::registry().spec(*self).utf8
    }

    /// Guess the game from a plugin's master list (case-insensitive).
    pub fn from_masters(masters: &[String]) -> Game {
        let joined = masters.join(" ").to_lowercase();
        // Order matters: check the more specific masters first.
        if joined.contains("starfield") {
            Game::Starfield
        } else if joined.contains("seventysix") || joined.contains("fallout76") {
            Game::Fallout76
        } else if joined.contains("fallout4") {
            Game::Fallout4
        } else if joined.contains("falloutnv") {
            Game::FalloutNV
        } else if joined.contains("fallout3") {
            Game::Fallout3
        } else if joined.contains("skyrim") {
            // Cannot tell LE from SE by master name alone; assume SE (dominant).
            Game::SkyrimSE
        } else if joined.contains("oblivion") {
            Game::Oblivion
        } else if joined.contains("morrowind") {
            Game::Morrowind
        } else {
            Game::Unknown
        }
    }

    /// Map a TES4 "form version" (the `u16` at offset 20 of a record header) to
    /// the game it implies. Ranges follow xTranslator's `getGameByFormVersion`
    /// (`TESVT_Const.pas`). Fallout 3 and New Vegas share the 2–15 Gamebryo range
    /// and are told apart by their masters, so this returns `Fallout3` for it;
    /// callers that know the masters should prefer [`Game::detect`].
    pub fn from_form_version(fv: u16) -> Game {
        match fv {
            40..=43 => Game::Skyrim,  // Legendary / classic 32-bit
            44 => Game::SkyrimSE,     // Special / Anniversary Edition
            2..=15 => Game::Fallout3, // Gamebryo FO3 / FNV family
            131 => Game::Fallout4,
            182..=201 => Game::Fallout76,
            // Starfield (Creation Engine 2). The initial release was 552–576, but
            // game updates keep bumping the form version (e.g. 581), so treat any
            // high value as Starfield — no later engine exists to collide with.
            // Getting this right matters beyond detection: a mis-detected
            // (Unknown) Starfield master would write output as Windows-1252
            // instead of UTF-8 and drop the Starfield-specific field set.
            552..=u16::MAX => Game::Starfield,
            _ => Game::Unknown,
        }
    }

    /// Automatic, transparent game identification from the loaded plugin — the
    /// UI never asks the user to pick a game (unlike xTranslator). Cascade:
    ///
    /// 1. magic `TES3` → Morrowind;
    /// 2. otherwise (`TES4`) the masters give the family, and the **form version**
    ///    refines it — it is the *only* way to split Skyrim LE (40–43) from
    ///    SE (44), whose master names are identical.
    ///
    /// `form_version` is `None` for Oblivion (20-byte header, no such field) and
    /// for TES3; in that case the masters decide alone.
    pub fn detect(magic: &str, masters: &[String], form_version: Option<u16>) -> Game {
        if magic == "TES3" {
            return Game::Morrowind;
        }
        let by_master = Game::from_masters(masters);
        match form_version {
            Some(fv) => match by_master {
                // Skyrim: master names are identical between editions; the form
                // version is the deciding signal.
                Game::Skyrim | Game::SkyrimSE => {
                    if (40..=43).contains(&fv) {
                        Game::Skyrim
                    } else if fv == 44 {
                        Game::SkyrimSE
                    } else {
                        by_master
                    }
                }
                // FO3 vs FNV both fall in 2–15; the masters already disambiguate.
                Game::Fallout3 | Game::FalloutNV => by_master,
                // No usable master clue: trust the form version alone.
                Game::Unknown => Game::from_form_version(fv),
                // Fallout 4/76, Starfield, Oblivion: the master is authoritative.
                other => other,
            },
            None => by_master,
        }
    }

    /// Length in bytes of a record/GRUP header for this game's format regime.
    /// Oblivion's TES4 header is 20 bytes (no form-version field); the modern
    /// TES4 games (Skyrim onward) use 24. Morrowind (TES3) is a different format
    /// altogether — its 16 here is informational; it needs a dedicated parser.
    pub fn header_len(&self) -> usize {
        crate::spec::registry().spec(*self).header_len as usize
    }

    /// Whether this game stores overridden vanilla strings as localization keys
    /// (`LOC_FN_…`) that must not be surfaced as translatable text. True for
    /// Oblivion (the Remastered edition's localization layer); harmless for the
    /// classic edition, which has no such keys.
    pub fn locks_loc_keys(&self) -> bool {
        crate::spec::registry().spec(*self).locks_loc_keys
    }
}

// The translatable-field tables live in `assets/data/fields/<id>.json`
// (module [`fields`], A-06); ask `registry().fields(game)`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_display_name() {
        // The opaque Fallout 4 workshop DLCs (case-insensitive, extension free).
        assert_eq!(
            plugin_display_name("DLCworkshop01.esm"),
            Some("Wasteland Workshop")
        );
        assert_eq!(
            plugin_display_name("DLCworkshop02.esm"),
            Some("Contraptions Workshop")
        );
        assert_eq!(
            plugin_display_name("dlcworkshop03"),
            Some("Vault-Tec Workshop")
        );
        assert_eq!(plugin_display_name("DLCNukaWorld.esm"), Some("Nuka-World"));
        assert_eq!(plugin_display_name("Dragonborn.esm"), Some("Dragonborn"));
        assert_eq!(plugin_display_name("MyRandomMod.esp"), None);
    }

    #[test]
    fn test_from_masters() {
        assert_eq!(
            Game::from_masters(&["Starfield.esm".into()]),
            Game::Starfield
        );
        assert_eq!(
            Game::from_masters(&["Skyrim.esm".into(), "Update.esm".into()]),
            Game::SkyrimSE
        );
        assert_eq!(Game::from_masters(&[]), Game::Unknown);
    }

    #[test]
    fn test_dsd_only_skyrim_se() {
        assert!(Game::SkyrimSE.supports_dsd());
        assert!(!Game::Starfield.supports_dsd());
    }

    #[test]
    fn test_per_game_fields() {
        // The shipped tables (embedded) keep the former per-game behaviour.
        let reg = GameRegistry::builtin();
        let is = |g: Game, r: &str, f: &str| reg.fields(g).is_translatable(r, f);
        assert!(is(Game::Skyrim, "MESG", "ITXT"));
        assert!(!is(Game::Skyrim, "MESG", "EDID"));
        // Starfield-specific records are translatable only under Starfield.
        assert!(is(Game::Starfield, "GBFM", "FULL"));
        assert!(is(Game::Starfield, "FLST", "FULL"));
        assert!(!is(Game::Skyrim, "GBFM", "FULL"));
        // MSTT:FULL is intentionally excluded (matches EET/SST).
        assert!(!is(Game::Starfield, "MSTT", "FULL"));

        // Fallout records, shared across the family; absent on Skyrim/Starfield.
        assert!(is(Game::Fallout3, "CREA", "FULL"));
        assert!(is(Game::FalloutNV, "RCPE", "FULL"));
        assert!(is(Game::Fallout4, "STAT", "FULL"));
        assert!(is(Game::Fallout76, "STAT", "FULL"));
        assert!(!is(Game::Skyrim, "RCPE", "FULL"));

        // Skyrim placed-reference name override (LE and SE, by inheritance).
        assert!(is(Game::Skyrim, "REFR", "FULL"));
        assert!(is(Game::SkyrimSE, "REFR", "FULL"));
        assert!(!is(Game::Fallout4, "REFR", "FULL"));

        // The base table stays available to every TES4 game; Morrowind has its own.
        assert!(is(Game::Fallout4, "WEAP", "FULL"));
        assert!(is(Game::Morrowind, "BOOK", "FNAM"));
        assert!(!is(Game::Morrowind, "WEAP", "FULL"));
        assert!(is(Game::OblivionRD, "WEAP", "FULL"));
    }

    #[test]
    fn test_form_version() {
        assert_eq!(Game::from_form_version(40), Game::Skyrim); // LE, low end
        assert_eq!(Game::from_form_version(43), Game::Skyrim); // LE, high end
        assert_eq!(Game::from_form_version(44), Game::SkyrimSE);
        assert_eq!(Game::from_form_version(131), Game::Fallout4);
        assert_eq!(Game::from_form_version(575), Game::Starfield);
        // Post-launch Starfield updates raised the form version past the original
        // 552–576 window (e.g. 581); anything high must still resolve to Starfield.
        assert_eq!(Game::from_form_version(581), Game::Starfield);
        assert_eq!(Game::from_form_version(600), Game::Starfield);
    }

    #[test]
    fn test_detect_cascade() {
        // Morrowind is decided by magic alone.
        assert_eq!(Game::detect("TES3", &[], None), Game::Morrowind);

        // Skyrim LE vs SE: identical masters, split by form version.
        let sky = ["Skyrim.esm".to_string(), "Update.esm".to_string()];
        assert_eq!(Game::detect("TES4", &sky, Some(40)), Game::Skyrim);
        assert_eq!(Game::detect("TES4", &sky, Some(43)), Game::Skyrim);
        assert_eq!(Game::detect("TES4", &sky, Some(44)), Game::SkyrimSE);

        // Oblivion: no form version (20-byte header) — masters decide.
        assert_eq!(
            Game::detect("TES4", &["Oblivion.esm".to_string()], None),
            Game::Oblivion
        );

        // Fallout 3 vs NV share 2–15; masters disambiguate.
        assert_eq!(
            Game::detect("TES4", &["FalloutNV.esm".to_string()], Some(15)),
            Game::FalloutNV
        );

        // Starfield by form version, with masters agreeing.
        assert_eq!(
            Game::detect("TES4", &["Starfield.esm".to_string()], Some(575)),
            Game::Starfield
        );

        // Unknown masters: fall back to the form version.
        assert_eq!(Game::detect("TES4", &[], Some(131)), Game::Fallout4);

        // The base Starfield.esm has *no* masters, so it is identified by its
        // form version alone — which the live game has raised to 581.
        assert_eq!(Game::detect("TES4", &[], Some(581)), Game::Starfield);
    }

    #[test]
    fn test_header_len() {
        assert_eq!(Game::Oblivion.header_len(), 20);
        assert_eq!(Game::Skyrim.header_len(), 24);
        assert_eq!(Game::Starfield.header_len(), 24);
    }
}
