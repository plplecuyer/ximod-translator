//! Per-document settings (E-12): game, source and target languages, output
//! mode, and the « personal dictionary » choice. Each open file carries its
//! own; the toolbar shows and edits the active document's, or the session
//! defaults when nothing is open (R-E12-01/02).

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, OutputMode, Pref};
use crate::games::Game;
use crate::i18n::I18n;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocSettings {
    /// Game id (`"SkyrimSE"`…); empty when none is chosen.
    pub game: String,
    /// Source language (ISO 639-3).
    pub source: String,
    /// Target language (ISO 639-3).
    pub target: String,
    /// Output mode; `None` while the « Sortie » list is empty (the export
    /// then asks for it).
    #[serde(with = "output_mode_serde", default)]
    pub output: Option<OutputMode>,
    /// « Utiliser mon dictionnaire personnel » (R-E12-06, tier 2).
    #[serde(default)]
    pub use_personal: bool,
}

mod output_mode_serde {
    use super::OutputMode;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(m: &Option<OutputMode>, s: S) -> Result<S::Ok, S::Error> {
        m.map(|m| m.as_str()).unwrap_or("").serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<OutputMode>, D::Error> {
        let s = String::deserialize(d)?;
        Ok(if s.trim().is_empty() {
            None
        } else {
            Some(OutputMode::from_label(&s))
        })
    }
}

impl DocSettings {
    /// How its source and target languages write their numbers (B.6).
    pub fn numbers(&self) -> crate::numstyle::NumPair {
        crate::numstyle::NumPair::of(&self.source, &self.target)
    }

    /// The session defaults (what the toolbar shows without a document).
    pub fn defaults(cfg: &AppConfig) -> Self {
        DocSettings {
            game: cfg.default_game.trim().to_string(),
            source: cfg.source_language.clone(),
            target: cfg.target_language.clone(),
            output: Some(cfg.output_mode),
            use_personal: cfg.personal_dict_default,
        }
    }

    /// Settings of a freshly opened file (R-E12-01): the game detected in the
    /// file, or the game chosen in the toolbar when detection was impossible;
    /// languages and output from the toolbar lists, the session defaults
    /// filling anything still empty.
    pub fn for_new_doc(detected: Game, idle: &IdleSettings, cfg: &AppConfig) -> Self {
        let mut s = idle.resolved(cfg);
        if detected != Game::Unknown {
            s.game = detected.id().to_string();
        }
        // A file of another game than the toolbar's takes that game's own
        // settings (E-04, R-E04-04).
        let to = s.game.clone();
        follow_game(
            (&mut s.source, &mut s.target, &mut s.output),
            &idle.game,
            &to,
            cfg,
        );
        s
    }

    /// Store the settings back as the session defaults.
    pub fn store_defaults(&self, cfg: &mut AppConfig) {
        cfg.default_game = self.game.trim().to_string();
        cfg.source_language = self.source.clone();
        cfg.target_language = self.target.clone();
        if let Some(m) = self.output {
            cfg.output_mode = m;
        }
        cfg.personal_dict_default = self.use_personal;
    }

    /// The game as an enum (`Unknown` when empty or not a known id).
    pub fn game(&self) -> Game {
        Game::from_id(self.game.trim())
    }

    /// The game's display name (the raw id for an unknown one, empty when none).
    pub fn game_label(&self) -> String {
        let g = self.game.trim();
        if g.is_empty() {
            return String::new();
        }
        match Game::from_id(g) {
            Game::Unknown => g.to_string(),
            known => known.display_name().to_string(),
        }
    }

    /// Short game name for the tab badge (« Skyrim SE » rather than the full
    /// « The Elder Scrolls V: Skyrim Special Edition »).
    pub fn game_short(&self, i18n: &I18n) -> String {
        game_short(i18n, &self.game)
    }

    /// Tab badge « Jeu · SRC→DST · Sortie » (R-E12-08).
    pub fn badge(&self, i18n: &I18n) -> String {
        i18n.t_args(
            "tab-badge",
            &[
                ("game", &self.game_short(i18n)),
                ("src", &self.source.to_uppercase()),
                ("dst", &self.target.to_uppercase()),
                (
                    "mode",
                    &self
                        .output
                        .map(|m| i18n.t(crate::ui::output_mode_key(m)))
                        .unwrap_or_else(|| "—".to_string()),
                ),
            ],
        )
    }

    /// The `_SRC-DST` suffix of a dictionary file for this pair.
    pub fn pair_suffix(&self) -> String {
        format!(
            "{}-{}",
            self.source.to_uppercase(),
            self.target.to_uppercase()
        )
    }
}

/// What the toolbar lists show when no document is open: four choices that
/// start empty at launch, are emptied again when the last tab closes, and
/// must all be filled before a file can be opened (they are the settings of
/// the next file).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdleSettings {
    pub game: String,
    pub source: String,
    pub target: String,
    pub output: Option<OutputMode>,
}

impl IdleSettings {
    /// True when the four lists hold a value.
    pub fn is_complete(&self) -> bool {
        !self.game.trim().is_empty()
            && !self.source.trim().is_empty()
            && !self.target.trim().is_empty()
            && self.output.is_some()
    }

    /// Back to the four empty positions.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The « Jeu » list preset at launch (`--game <id>`): the game, with its
    /// own settings, as if chosen in the toolbar (recette du lot 2 : the
    /// output of Oblivion stayed empty when launched with `--game Oblivion`).
    pub fn preset_game(&mut self, game: &str, cfg: &AppConfig) {
        let game = game.trim();
        follow_game(
            (&mut self.source, &mut self.target, &mut self.output),
            "",
            game,
            cfg,
        );
        self.game = game.to_string();
    }

    /// The lists as document settings; an empty list stays empty, only the
    /// personal-dictionary choice comes from the configuration.
    pub fn resolved(&self, cfg: &AppConfig) -> DocSettings {
        DocSettings {
            game: self.game.trim().to_string(),
            source: self.source.clone(),
            target: self.target.clone(),
            output: self.output,
            use_personal: cfg.personal_dict_default,
        }
    }

    /// Remember the filled lists as the session defaults (`[Translation]`),
    /// except the values the game has of its own (E-04): those stay in its
    /// section, the common ones unchanged.
    pub fn store_defaults(&self, cfg: &mut AppConfig) {
        let game = self.game.trim();
        if !game.is_empty() {
            cfg.default_game = game.to_string();
        }
        if !self.source.is_empty() && !cfg.has_own(game, Pref::Source) {
            cfg.source_language = self.source.clone();
        }
        if !self.target.is_empty() && !cfg.has_own(game, Pref::Target) {
            cfg.target_language = self.target.clone();
        }
        if let Some(m) = self.output
            && !cfg.has_own(game, Pref::Output)
        {
            cfg.output_mode = m;
        }
    }
}

/// The source, target and output when the game goes from `from` to `to`
/// (E-04, R-E04-04): a setting `to` has of its own takes its value; one only
/// `from` had goes back to the common value; the others stay as they are
/// (a configuration without [Game.*] behaves as the 0.4.0, R-E04-06).
/// Returns the settings that changed.
pub fn follow_game(
    (source, target, output): (&mut String, &mut String, &mut Option<OutputMode>),
    from: &str,
    to: &str,
    cfg: &AppConfig,
) -> Vec<Pref> {
    let (from, to) = (from.trim(), to.trim());
    if from == to {
        return Vec::new();
    }
    let own = cfg.prefs_for(to);
    let common = cfg.common_prefs();
    let pick = |p: Pref| -> Option<&crate::config::ResolvedPrefs> {
        if cfg.has_own(to, p) {
            Some(&own)
        } else if cfg.has_own(from, p) {
            Some(&common)
        } else {
            None
        }
    };
    let mut changed = Vec::new();
    if let Some(r) = pick(Pref::Source)
        && *source != r.source
    {
        *source = r.source.clone();
        changed.push(Pref::Source);
    }
    if let Some(r) = pick(Pref::Target)
        && *target != r.target
    {
        *target = r.target.clone();
        changed.push(Pref::Target);
    }
    if let Some(r) = pick(Pref::Output)
        && *output != Some(r.output)
    {
        *output = Some(r.output);
        changed.push(Pref::Output);
    }
    changed
}

/// Short name of game `game` (an id): « Skyrim SE » (tab badge, status bar).
pub fn game_short(i18n: &I18n, game: &str) -> String {
    let g = game.trim();
    let key = match Game::from_id(g) {
        Game::Unknown => return g.to_string(),
        Game::Morrowind => "game-short-morrowind",
        Game::Oblivion => "game-short-oblivion",
        Game::OblivionRD => "game-short-oblivionrd",
        Game::Skyrim => "game-short-skyrim",
        Game::SkyrimSE => "game-short-skyrimse",
        Game::Fallout3 => "game-short-fallout3",
        Game::FalloutNV => "game-short-falloutnv",
        Game::Fallout4 => "game-short-fallout4",
        Game::Fallout76 => "game-short-fallout76",
        Game::Starfield => "game-short-starfield",
    };
    i18n.t(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oblivion_full() -> AppConfig {
        let mut cfg = AppConfig {
            source_language: "eng".into(),
            target_language: "fra".into(),
            output_mode: OutputMode::OverridePatch,
            ..AppConfig::default()
        };
        let ob = cfg.game_prefs.entry("Oblivion".into()).or_default();
        ob.output = Some(OutputMode::FullPlugin);
        ob.target = Some("deu".into());
        cfg
    }

    #[test]
    fn the_toolbar_follows_the_game() {
        // T-E04-01: Skyrim SE → Oblivion shows Oblivion's output and target;
        // back to Skyrim SE, the common values again.
        let cfg = oblivion_full();
        let mut idle = IdleSettings {
            game: "SkyrimSE".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::OverridePatch),
        };
        fn fields(i: &mut IdleSettings) -> (&mut String, &mut String, &mut Option<OutputMode>) {
            (&mut i.source, &mut i.target, &mut i.output)
        }
        let ch = follow_game(fields(&mut idle), "SkyrimSE", "Oblivion", &cfg);
        assert_eq!(ch, vec![Pref::Target, Pref::Output]);
        assert_eq!(idle.output, Some(OutputMode::FullPlugin));
        assert_eq!(idle.target, "deu");
        follow_game(fields(&mut idle), "Oblivion", "SkyrimSE", &cfg);
        assert_eq!(idle.output, Some(OutputMode::OverridePatch));
        assert_eq!(idle.target, "fra");
        // Neither game has a section: nothing moves (R-E04-06).
        idle.target = "ita".into();
        assert!(follow_game(fields(&mut idle), "SkyrimSE", "Fallout4", &cfg).is_empty());
        assert_eq!(idle.target, "ita");
        // Its own values are not stored as the common ones.
        idle.game = "Oblivion".into();
        idle.output = Some(OutputMode::FullPlugin);
        let mut cfg2 = cfg.clone();
        idle.store_defaults(&mut cfg2);
        assert_eq!(cfg2.output_mode, OutputMode::OverridePatch);
        assert_eq!(cfg2.target_language, "fra");
        assert_eq!(cfg2.default_game, "Oblivion");
    }

    #[test]
    fn a_game_preset_at_launch_takes_its_settings() {
        let cfg = oblivion_full();
        let mut idle = IdleSettings::default();
        idle.preset_game(" Oblivion ", &cfg);
        assert_eq!(idle.game, "Oblivion");
        assert_eq!(idle.output, Some(OutputMode::FullPlugin));
        assert_eq!(idle.target, "deu");
        // A game without settings of its own: the lists stay as they are.
        let mut idle = IdleSettings::default();
        idle.preset_game("SkyrimSE", &cfg);
        assert_eq!(idle.game, "SkyrimSE");
        assert_eq!(idle.output, None);
        assert!(idle.target.is_empty());
    }

    #[test]
    fn a_file_of_another_game_takes_its_settings() {
        // T-E04-02.
        let cfg = oblivion_full();
        let idle = IdleSettings {
            game: "SkyrimSE".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::OverridePatch),
        };
        let s = DocSettings::for_new_doc(Game::Oblivion, &idle, &cfg);
        assert_eq!((s.game.as_str(), s.target.as_str()), ("Oblivion", "deu"));
        assert_eq!(s.output, Some(OutputMode::FullPlugin));
        let s = DocSettings::for_new_doc(Game::SkyrimSE, &idle, &cfg);
        assert_eq!(s.output, Some(OutputMode::OverridePatch));
        assert_eq!(s.target, "fra");
    }

    #[test]
    fn new_doc_takes_detected_game_or_default() {
        let cfg = AppConfig {
            default_game: "Fallout4".into(),
            source_language: "eng".into(),
            target_language: "fra".into(),
            output_mode: OutputMode::Strings,
            personal_dict_default: true,
            ..Default::default()
        };
        let idle = IdleSettings {
            game: "Fallout4".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::Strings),
        };
        assert!(idle.is_complete());
        let s = DocSettings::for_new_doc(Game::SkyrimSE, &idle, &cfg);
        assert_eq!(s.game, "SkyrimSE");
        assert_eq!(
            (
                s.source.as_str(),
                s.target.as_str(),
                s.output,
                s.use_personal
            ),
            ("eng", "fra", Some(OutputMode::Strings), true)
        );
        let s = DocSettings::for_new_doc(Game::Unknown, &idle, &cfg);
        assert_eq!(s.game, "Fallout4");
        assert_eq!(s.game(), Game::Fallout4);
        assert_eq!(s.pair_suffix(), "ENG-FRA");
        // Round trip through the defaults.
        let mut cfg2 = AppConfig::default();
        s.store_defaults(&mut cfg2);
        assert_eq!(DocSettings::defaults(&cfg2), s);
    }

    #[test]
    fn idle_lists_start_empty_and_fill_the_next_file() {
        let cfg = AppConfig {
            output_mode: OutputMode::Hybrid,
            ..Default::default()
        };
        let mut idle = IdleSettings::default();
        assert!(!idle.is_complete());
        // Partial lists: the empty ones stay empty, the output too.
        idle.source = "eng".into();
        let r = idle.resolved(&cfg);
        assert_eq!(
            (
                r.game.as_str(),
                r.source.as_str(),
                r.target.as_str(),
                r.output
            ),
            ("", "eng", "", None)
        );
        idle.game = "SkyrimSE".into();
        idle.target = "deu".into();
        assert!(!idle.is_complete(), "the output is still empty");
        idle.output = Some(OutputMode::Dsd);
        assert!(idle.is_complete());
        // Only the filled lists reach the defaults.
        let mut cfg2 = AppConfig::default();
        IdleSettings {
            target: "ita".into(),
            ..Default::default()
        }
        .store_defaults(&mut cfg2);
        assert_eq!(
            (
                cfg2.default_game.as_str(),
                cfg2.source_language.as_str(),
                cfg2.target_language.as_str()
            ),
            ("", "eng", "ita")
        );
        idle.clear();
        assert_eq!(idle, IdleSettings::default());
    }

    #[test]
    fn json_round_trip() {
        let s = DocSettings {
            game: "Starfield".into(),
            source: "eng".into(),
            target: "deu".into(),
            output: Some(OutputMode::Dsd),
            use_personal: true,
        };
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"output\":\"Dsd\""));
        let back: DocSettings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // An empty output round-trips as an empty string.
        let s = DocSettings { output: None, ..s };
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"output\":\"\""));
        assert_eq!(serde_json::from_str::<DocSettings>(&text).unwrap(), s);
    }
}
