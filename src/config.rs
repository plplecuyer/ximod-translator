//! Application configuration (`Config.ini`, next to the executable in portable
//! mode or in the per-user folder — see [`paths`]).
//!
//! Deliberately kept compatible in spirit with XIMOD Architect's `Config.ini`
//! so the two tools feel like one family, plus a few translation-specific keys
//! (source / target language, default output mode).

pub mod paths;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub use paths::{AppPaths, ConfigMode};

/// One detected/registered game installation (persisted in `[Games]`).
#[derive(Debug, Clone, Default)]
pub struct GameEntry {
    /// Absolute path to the game's install root.
    pub path: String,
    /// How it was found: "Steam", "GogGalaxy", "GogRegistry", "Epic", "Xbox",
    /// "Heroic", "Lutris",
    /// "FolderScan", "Manual".
    pub source: String,
    /// True when set manually and not to be overwritten by auto-detection.
    pub locked: bool,
}

/// UI theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "System",
        }
    }

    /// The i18n key of the theme's name in the settings.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Dark => "theme-dark",
            Self::Light => "theme-light",
            Self::System => "theme-system",
        }
    }

    pub fn from_label(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "light" => Self::Light,
            "system" => Self::System,
            _ => Self::Dark,
        }
    }

    pub fn variants() -> &'static [Self] {
        &[Self::Dark, Self::Light, Self::System]
    }
}

/// How a finished translation is written out.
///
/// Mirrors the output modes described in the XIMOD Translator specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// STRINGS / DLSTRINGS / ILSTRINGS files alongside the original plugin.
    Strings,
    /// Original plugin kept, only the strings files localised (hybrid).
    #[default]
    Hybrid,
    /// A small master-dependent override plugin (no redistribution of originals).
    OverridePatch,
    /// A full translated esp/esm.
    FullPlugin,
    /// Dynamic String Distributor JSON (SKSE runtime, Skyrim SE/AE).
    Dsd,
    /// The text file of a text document (MCM, INI, JSON…, C-05): the only
    /// mode a text document offers.
    Text,
    /// The translated compiled scripts of a « scripts » document (C-06):
    /// the only mode it offers.
    Scripts,
}

impl OutputMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strings => "Strings",
            Self::Hybrid => "Hybrid",
            Self::OverridePatch => "OverridePatch",
            Self::FullPlugin => "FullPlugin",
            Self::Dsd => "Dsd",
            Self::Text => "Text",
            Self::Scripts => "Scripts",
        }
    }

    pub fn from_label(s: &str) -> Self {
        match s {
            "Strings" => Self::Strings,
            "OverridePatch" => Self::OverridePatch,
            "FullPlugin" => Self::FullPlugin,
            "Dsd" => Self::Dsd,
            "Text" => Self::Text,
            "Scripts" => Self::Scripts,
            _ => Self::Hybrid,
        }
    }

    /// The modes of a plugin document (the toolbar's list).
    pub fn variants() -> &'static [Self] {
        &[
            Self::Strings,
            Self::Hybrid,
            Self::OverridePatch,
            Self::FullPlugin,
            Self::Dsd,
        ]
    }

    /// The output of the game sheet this mode is, `None` for the modes of
    /// the text and scripts documents, which no game restricts.
    pub fn sheet_id(self) -> Option<crate::games::OutputModeId> {
        use crate::games::OutputModeId as Id;
        Some(match self {
            Self::Strings => Id::Strings,
            Self::Hybrid => Id::Hybrid,
            Self::OverridePatch => Id::OverridePatch,
            Self::FullPlugin => Id::FullPlugin,
            Self::Dsd => Id::Dsd,
            Self::Text | Self::Scripts => return None,
        })
    }

    /// Whether the game can receive this output (its sheet's
    /// `outputModes`): Oblivion has no STRINGS, DSD is for Skyrim SE alone.
    /// An unknown game restricts nothing.
    pub fn fits(self, game: crate::games::Game) -> bool {
        match self.sheet_id() {
            Some(id) if game != crate::games::Game::Unknown => crate::games::registry()
                .spec(game)
                .output_modes
                .contains(&id),
            _ => true,
        }
    }

    /// The output that usually suits a document (R-T01, D-114): marked
    /// « (usuel) » in the list, never chosen for the user. STRINGS for a
    /// localized plugin of a game with STRINGS; otherwise the override
    /// patch when the game takes it; otherwise the full plugin (Morrowind);
    /// the text file or the scripts for those documents.
    pub fn usual(
        game: crate::games::Game,
        localized: bool,
        is_text: bool,
        is_scripts: bool,
    ) -> Option<Self> {
        if is_text {
            return Some(Self::Text);
        }
        if is_scripts {
            return Some(Self::Scripts);
        }
        if localized && Self::Strings.fits(game) {
            return Some(Self::Strings);
        }
        [Self::OverridePatch, Self::FullPlugin]
            .into_iter()
            .find(|m| m.fits(game))
    }

    /// The message key of the sentence describing this output (R-T01).
    pub fn desc_key(self) -> &'static str {
        match self {
            Self::Strings => "output-desc-strings",
            Self::Hybrid => "output-desc-hybrid",
            Self::OverridePatch => "output-desc-override",
            Self::FullPlugin => "output-desc-full",
            Self::Dsd => "output-desc-dsd",
            Self::Text => "output-desc-text",
            Self::Scripts => "output-desc-scripts",
        }
    }

    /// The modes a document offers: text documents have the text file
    /// alone (R-C05-04).
    pub fn variants_for(is_text: bool, is_scripts: bool) -> &'static [Self] {
        if is_text {
            &[Self::Text]
        } else if is_scripts {
            &[Self::Scripts]
        } else {
            Self::variants()
        }
    }
}

/// Application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Interface language (ISO 639-3).
    pub locale: String,
    /// Country (ISO 3166-1 alpha-3) scoping the language list; empty until chosen.
    pub country: String,
    /// Source language of the strings being translated (ISO 639-3).
    pub source_language: String,
    /// Target language of the translation (ISO 639-3).
    pub target_language: String,
    /// Default game (id, e.g. `"SkyrimSE"`) shown in the toolbar when no
    /// document is active and given to the next opened file when its game
    /// cannot be detected (E-12, `[Translation] DefaultGame`). Empty means
    /// "no game chosen".
    pub default_game: String,
    /// Default output mode for new documents.
    pub output_mode: OutputMode,
    /// Initial state of « Utiliser mon dictionnaire personnel » for a new
    /// document (`[Translation] PersonalDictDefault`, E-12).
    pub personal_dict_default: bool,
    /// UI theme.
    pub theme: Theme,
    /// Interface font size.
    pub font_size: f32,
    /// Maximum number of recent files kept.
    pub max_recent_files: usize,
    /// Recently opened files.
    pub recent_files: Vec<PathBuf>,
    /// Window width / height (persisted).
    pub window_width: f32,
    pub window_height: f32,
    /// Main window position (`-1` = let the OS place it).
    pub window_x: f32,
    pub window_y: f32,
    /// Settings window geometry (`-1` position = center on first open).
    pub settings_x: f32,
    pub settings_y: f32,
    pub settings_w: f32,
    pub settings_h: f32,
    /// Flags window geometry.
    pub flags_x: f32,
    pub flags_y: f32,
    pub flags_w: f32,
    pub flags_h: f32,
    /// Fields editor window geometry (A-06).
    pub fields_win_x: f32,
    pub fields_win_y: f32,
    pub fields_win_w: f32,
    pub fields_win_h: f32,
    /// « Traduire un FOMOD » and « Empaqueter une traduction » windows (M-02, M-03).
    pub fomod_win: [f32; 4],
    /// The text formats editor (C-05): `[Window] FormatsWinX/Y/W/H`.
    pub formats_win: [f32; 4],
    /// Online translation (D-01 / D-02): `[Online]`.
    pub online: crate::online::OnlineConfig,
    /// The interface translation editor (E-10): `[Window] TrWinX/Y/W/H`.
    pub tr_win: [f32; 4],
    pub pack_win: [f32; 4],
    /// The edit window (A-02): `[Window] EditX / EditY / EditW / EditH`.
    pub edit_win: [f32; 4],
    /// Last context tab of the edit window (`[Editing] EditTab`).
    pub edit_tab: String,
    /// Special characters shown as glyphs (`[Editing] ShowSpecialChars`, A-11).
    pub show_special_chars: bool,
    /// Propose the propagation after a validation (`[Editing]
    /// ProposePropagation`, A-12).
    pub propose_propagation: bool,
    /// Propagate a translation to the open files of the same source
    /// (`[Editing] PropagateAcrossFiles`, B-25).
    pub propagate_files: bool,
    /// The filter row under the grid headers (`[Editing] ColumnFilters`,
    /// A-10), hidden by default.
    pub column_filters: bool,
    /// The Commentaire column of the grid (`[Columns] ShowComment`, C-03;
    /// read from `[Editing]` before étape 2).
    pub show_comment: bool,
    /// The Provenance column of the grid (`[Columns] ShowProvenance`, B-09).
    pub show_provenance: bool,
    /// Rich tooltips on the Source and Traduction cells (`[Columns]
    /// RichTooltips`, A-16): identity, text, marks, length, provenance.
    pub rich_tooltips: bool,
    /// The Relecture column of the grid (`[Columns] ShowReview`, A-17).
    pub show_review: bool,
    /// The status buttons column of the grid (`[Columns] ShowStatusButtons`,
    /// A-04).
    pub show_status_buttons: bool,
    /// Generated comments written to EET / XML (`[Export] GeneratedComments`,
    /// C-03, decision D-23).
    pub export_generated_comments: bool,
    /// Text documents: untranslated lines keep their source at writing
    /// (`[Export] UntranslatedCopy`, C-05, decision D-25).
    pub export_untranslated_copy: bool,
    /// Localizing a plugin also writes the STRINGS of the source language
    /// (`[Export] LocalizeSourceStrings`, C-09, decision D-61).
    pub localize_source_strings: bool,
    /// « Reporter aussi la traduction dans la source .psc »
    /// (`[Scripts] CopyToSource`, C-07, decision D-67).
    pub scripts_copy_to_source: bool,
    /// Keep the original before an export overwrites a file (E-06):
    /// `[Output] KeepOriginal`, and the copies kept per file
    /// (`OriginalCopies`).
    pub keep_original: bool,
    pub original_copies: usize,
    /// The Rechercher et remplacer window (`[Window] Replace*`).
    pub replace_win: [f32; 4],
    /// The Matrices de remplacement window (B-17): `[Window] MatricesX/Y/W/H`.
    pub matrices_win: [f32; 4],
    /// The exclusion rules window (B-18): x, y, w, h.
    pub rules_win: [f32; 4],
    /// Glossaire, motifs et dérivations (B-15): `[Window] AssistX/Y/W/H`.
    pub assist_win: [f32; 4],
    /// Base pays / langues (E-11): `[Window] PropsX/Y/W/H`.
    pub props_win: [f32; 4],
    /// Analyser un dossier (D-07): `[Window] ScanX/Y/W/H`.
    pub scan_win: [f32; 4],
    /// Comparer deux dictionnaires (D-07): `[Window] CompareX/Y/W/H`.
    pub compare_win: [f32; 4],
    /// `[Scan] LastDir, Subdirs, Scripts` (D-07).
    pub scan_last_dir: String,
    pub scan_subdirs: bool,
    pub scan_scripts: bool,
    /// Propose to generalize an exclusion set by hand (B-18, D-75).
    pub propose_generalization: bool,
    /// The last matrix chosen per target language (B-17): `[Replace]
    /// LastMatrix`, `lang=name` pairs separated by `|`.
    pub last_matrix: String,
    /// The format chosen last per name pattern in Importer avec un format
    /// (D-08, R-D08-05): `[Import] Last.<extension>=<format id>`.
    pub import_last: std::collections::BTreeMap<String, String>,
    /// Vérifications qualité (D-05): `[Quality] Checks` (keys, comma-separated)
    /// and `[Window] Quality*`.
    pub quality_checks: String,
    pub quality_win: [f32; 4],
    /// The Historique window (A-18): `[Window] HistX/Y/W/H`.
    pub hist_win: [f32; 4],
    /// The Recherche avancée window (A-19): `[Window] SearchX/Y/W/H`.
    pub search_win: [f32; 4],
    /// The Scripts du mod window (C-06): `[Window] ScriptsX/Y/W/H`.
    pub scripts_win: [f32; 4],
    /// The Concordance window (A-20): `[Window] ConcordX/Y/W/H`.
    pub concord_win: [f32; 4],
    /// The journal of the session (E-01): `[Window] JournalX/Y/W/H`.
    pub journal_win: [f32; 4],
    /// Permanent answers of the confirmation dialogs (E-03):
    /// `[Confirmations] <question>=<answer>`; see [`crate::remember`].
    pub confirmations: BTreeMap<String, String>,
    /// The history of the translations (A-18): `[History] Enabled`,
    /// `MaxEntries`.
    pub history_enabled: bool,
    pub history_max: usize,
    /// Log the frames slower than this many milliseconds (`[Diagnostics]
    /// FrameLog`, 0 = off; the environment variable `XIMOD_FRAME_LOG` wins).
    pub frame_log_ms: u32,
    /// `[Window] DialogX/Y/W/H` (C-01).
    pub dialog_win: [f32; 4],
    /// `[Dialogues] FollowGrid` / `Split` (C-01).
    pub dialog_follow_grid: bool,
    pub dialog_split: f32,
    /// Dictionary tree window (B-02).
    pub dicts_win: [f32; 4],
    /// Dictionary editor (B-04).
    pub dict_edit_win: [f32; 4],
    /// Mise à jour du mod (B-07).
    pub update_win: [f32; 4],
    /// About window position (fixed size, so only the position is persisted).
    pub about_x: f32,
    pub about_y: f32,
    /// Status-colors window position (fixed size).
    pub status_colors_x: f32,
    pub status_colors_y: f32,
    /// Color-picker window position (fixed size).
    pub picker_x: f32,
    pub picker_y: f32,
    /// Spell-checker window geometry (resizable, position + size persisted).
    pub spell_x: f32,
    pub spell_y: f32,
    pub spell_w: f32,
    pub spell_h: f32,
    /// Archive-navigator window geometry (resizable, position + size persisted).
    pub archive_x: f32,
    pub archive_y: f32,
    pub archive_w: f32,
    pub archive_h: f32,
    /// Book-preview window geometry (resizable, position + size persisted).
    pub book_x: f32,
    pub book_y: f32,
    pub book_w: f32,
    pub book_h: f32,
    /// Dictionary-viewer window geometry (resizable, position + size persisted).
    pub dictview_x: f32,
    pub dictview_y: f32,
    pub dictview_w: f32,
    pub dictview_h: f32,
    /// Explorer tool windows geometry (position + size persisted per tool).
    pub esm_win_x: f32,
    pub esm_win_y: f32,
    pub esm_win_w: f32,
    pub esm_win_h: f32,
    pub bsa_win_x: f32,
    pub bsa_win_y: f32,
    pub bsa_win_w: f32,
    pub bsa_win_h: f32,
    pub swf_win_x: f32,
    pub swf_win_y: f32,
    pub swf_win_w: f32,
    pub swf_win_h: f32,
    /// Dictionary-viewer zebra-stripe color. Used only when
    /// `dictview_stripe_custom` is true; otherwise the theme's faint color is
    /// used, so the default look stays correct in both light and dark themes.
    pub dictview_stripe: (u8, u8, u8),
    pub dictview_stripe_custom: bool,
    /// Folder holding the game translation bases (empty → `<exe>/bases/jeux`).
    pub game_bases_dir: String,
    /// Folder holding the user's mod translation bases (empty → `<exe>/bases/mods`).
    pub mod_bases_dir: String,
    /// Operations kept per document for undo (`[Editing] UndoDepth`, A-01).
    pub undo_depth: usize,
    /// Show the lines flagged « never shown in game » (`[Editing] ShowUnused`).
    pub show_unused: bool,
    /// Transient notifications in the top-right corner (`[General] Toasts`, F-05).
    pub toasts: bool,
    /// Name signed in the FOMOD translations (`[General] Translator`, F-03).
    pub translator: String,
    /// `[Backup]` (B-11): autosave of the work in progress, its interval in
    /// minutes, its folder (empty = `<config>/autosave`), and the timestamped
    /// copies kept per mod dictionary.
    pub autosave_enabled: bool,
    pub autosave_minutes: u32,
    pub autosave_dir: String,
    pub backup_copies: usize,
    /// Write the mod's own dictionary at each save and export (B-03).
    pub mod_dict_auto: bool,
    /// Check at opening that the mod's dictionary was written for this
    /// version of the plugin (B-12): `[Dictionaries] CheckCrc`.
    pub check_crc: bool,
    /// Personal dictionary (B-04): fill it at each save; keep the field
    /// with the pairs added by Ctrl+M.
    pub personal_autofill: bool,
    pub personal_keep_field: bool,
    /// Old data of the mod dictionaries kept this many months (0 = forever).
    pub old_data_months: u32,
    /// Word heuristic threshold (B-01), percent of differing words admitted.
    pub fuzzy_threshold: u8,
    /// The normalized tier of the stack at opening (B-19, D-72).
    pub normalize: bool,
    /// The dictionary tree per game and pair (B-02):
    /// `Dicts_<Game>_<SRC>-<DST>` → order, cases and additions.
    pub dictionaries: std::collections::BTreeMap<String, String>,
    /// Root folder of a portable ESP-ESM Translator install (empty = not set).
    /// These tools are portable and not in the Windows registry, hence manual.
    pub eet_dir: String,
    /// Root folder of a portable xTranslator install (empty = not set).
    pub xtranslator_dir: String,
    /// Splash screen duration in seconds (0 disables it).
    pub splash_screen_seconds: u32,
    /// Whether the initial configuration has been completed at least once.
    pub first_start_done: bool,
    /// Per-status colors `(r, g, b)`, indexed by [`crate::status::Status::index`].
    pub status_colors: [(u8, u8, u8); crate::status::COUNT],
    /// Run game auto-detection at startup when no game is registered yet.
    pub auto_detect_games: bool,
    /// Detected/registered game installations, keyed by the stable game id
    /// ([`crate::games::Game::id`]).
    pub games: BTreeMap<String, GameEntry>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            locale: "eng".to_string(),
            country: String::new(),
            source_language: "eng".to_string(),
            target_language: "fra".to_string(),
            default_game: String::new(),
            output_mode: OutputMode::Hybrid,
            personal_dict_default: false,
            theme: Theme::Dark,
            font_size: 14.0,
            max_recent_files: 10,
            recent_files: Vec::new(),
            window_width: 1280.0,
            window_height: 800.0,
            window_x: -1.0,
            window_y: -1.0,
            settings_x: -1.0,
            settings_y: -1.0,
            settings_w: 770.0,
            settings_h: 400.0,
            flags_x: -1.0,
            flags_y: -1.0,
            flags_w: 720.0,
            flags_h: 520.0,
            fields_win_x: -1.0,
            fields_win_y: -1.0,
            fields_win_w: 1100.0,
            fields_win_h: 720.0,
            fomod_win: [-1.0, -1.0, 1150.0, 720.0],
            formats_win: [-1.0, -1.0, 860.0, 560.0],
            tr_win: [-1.0, -1.0, 1100.0, 720.0],
            online: Default::default(),
            pack_win: [-1.0, -1.0, 720.0, 640.0],
            edit_win: [-1.0, -1.0, 760.0, 520.0],
            edit_tab: "record".to_string(),
            show_special_chars: false,
            propose_propagation: false,
            propagate_files: true,
            column_filters: false,
            show_comment: false,
            show_provenance: false,
            rich_tooltips: true,
            show_review: false,
            show_status_buttons: false,
            export_generated_comments: false,
            export_untranslated_copy: true,
            localize_source_strings: true,
            scripts_copy_to_source: false,
            keep_original: true,
            original_copies: crate::backup::DEFAULT_ORIGINAL_COPIES,
            replace_win: [-1.0, -1.0, 620.0, 400.0],
            matrices_win: [-1.0, -1.0, 1180.0, 540.0],
            rules_win: [-1.0, -1.0, 1100.0, 600.0],
            assist_win: [-1.0, -1.0, 1180.0, 640.0],
            props_win: [-1.0, -1.0, 820.0, 520.0],
            scan_win: [-1.0, -1.0, 1000.0, 600.0],
            compare_win: [-1.0, -1.0, 1000.0, 620.0],
            scan_last_dir: String::new(),
            scan_subdirs: true,
            scan_scripts: false,
            propose_generalization: true,
            last_matrix: String::new(),
            import_last: std::collections::BTreeMap::new(),
            quality_checks: crate::quality::Check::format_set(&crate::quality::Check::default_set()),
            quality_win: [-1.0, -1.0, 700.0, 520.0],
            hist_win: [-1.0, -1.0, 820.0, 480.0],
            search_win: [-1.0, -1.0, 760.0, 470.0],
            scripts_win: [-1.0, -1.0, 900.0, 520.0],
            concord_win: [-1.0, -1.0, 820.0, 480.0],
            journal_win: [-1.0, -1.0, 860.0, 440.0],
            confirmations: BTreeMap::new(),
            history_enabled: true,
            history_max: crate::history::DEFAULT_MAX,
            frame_log_ms: 0,
            dialog_win: [-1.0, -1.0, 900.0, 580.0],
            dialog_follow_grid: true,
            dialog_split: 0.47,
            dicts_win: [-1.0, -1.0, 760.0, 520.0],
            dict_edit_win: [-1.0, -1.0, 760.0, 480.0],
            update_win: [-1.0, -1.0, 760.0, 460.0],
            about_x: -1.0,
            about_y: -1.0,
            status_colors_x: -1.0,
            status_colors_y: -1.0,
            picker_x: -1.0,
            picker_y: -1.0,
            spell_x: -1.0,
            spell_y: -1.0,
            spell_w: 560.0,
            spell_h: 480.0,
            archive_x: -1.0,
            archive_y: -1.0,
            archive_w: 640.0,
            archive_h: 520.0,
            book_x: -1.0,
            book_y: -1.0,
            book_w: 820.0,
            book_h: 620.0,
            dictview_x: -1.0,
            dictview_y: -1.0,
            dictview_w: 720.0,
            dictview_h: 480.0,
            esm_win_x: -1.0,
            esm_win_y: -1.0,
            esm_win_w: 1000.0,
            esm_win_h: 700.0,
            bsa_win_x: -1.0,
            bsa_win_y: -1.0,
            bsa_win_w: 1000.0,
            bsa_win_h: 700.0,
            swf_win_x: -1.0,
            swf_win_y: -1.0,
            swf_win_w: 1000.0,
            swf_win_h: 700.0,
            dictview_stripe: (222, 230, 242),
            dictview_stripe_custom: false,
            game_bases_dir: String::new(),
            mod_bases_dir: String::new(),
            undo_depth: crate::undo::DEFAULT_DEPTH,
            show_unused: false,
            toasts: true,
            translator: String::new(),
            autosave_enabled: true,
            autosave_minutes: crate::backup::DEFAULT_AUTOSAVE_MINUTES,
            autosave_dir: String::new(),
            backup_copies: crate::backup::DEFAULT_COPIES,
            mod_dict_auto: true,
            check_crc: true,
            personal_autofill: false,
            personal_keep_field: false,
            old_data_months: 0,
            fuzzy_threshold: 20,
            normalize: true,
            dictionaries: Default::default(),
            eet_dir: String::new(),
            xtranslator_dir: String::new(),
            splash_screen_seconds: 2,
            first_start_done: false,
            status_colors: default_status_colors(),
            auto_detect_games: true,
            games: BTreeMap::new(),
        }
    }
}

/// The shipped base sub-folder (`"jeux"` / `"mods"`), trying
/// `assets/bases/<sub>` then `bases/<sub>` next to the executable, the macOS
/// bundle, and finally the development working directory. Falls back to
/// `assets/bases/<sub>` when none exists yet (so it can be created there).
/// The shipped `jeux` folder (read-only dictionaries, B-04).
pub fn shipped_game_base_dir() -> PathBuf {
    shipped_base_dir("jeux")
}

fn shipped_base_dir(sub: &str) -> PathBuf {
    let rel_assets = PathBuf::from("assets").join("bases").join(sub);
    let rel_plain = PathBuf::from("bases").join(sub);
    let mut candidates: Vec<PathBuf> = Vec::new();
    let exe_dir = AppPaths::exe_dir();
    candidates.push(exe_dir.join(&rel_assets));
    candidates.push(exe_dir.join(&rel_plain));
    if let Some(up) = exe_dir.parent() {
        candidates.push(up.join("Resources").join(&rel_assets));
        candidates.push(up.join("Resources").join(&rel_plain));
    }
    candidates.push(rel_assets.clone());
    candidates.push(rel_plain);
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or(rel_assets)
}

/// The *writable* default base sub-folder (E-05): in portable mode the shipped
/// one next to the executable (unchanged behaviour); in user mode
/// `<data_dir>/bases/<sub>`, the shipped folder then only being read
/// ([`shipped_base_dir`]) — the executable may live under `Program Files`.
fn default_base_dir(sub: &str) -> PathBuf {
    let p = paths::get();
    match p.mode {
        ConfigMode::Portable => shipped_base_dir(sub),
        ConfigMode::User => p.data_dir.join("bases").join(sub),
    }
}

/// Locate the shipped `assets/data` folder (holding `GamesDLC.json`, the game
/// sheets…), trying next to the executable, the macOS bundle, then the
/// development working directory. Falls back to `assets/data` (so it can be
/// created there) when none exists yet.
pub fn assets_data_dir() -> PathBuf {
    let rel_assets = PathBuf::from("assets").join("data");
    let rel_plain = PathBuf::from("data");
    let mut candidates: Vec<PathBuf> = Vec::new();
    let exe_dir = AppPaths::exe_dir();
    candidates.push(exe_dir.join(&rel_assets));
    candidates.push(exe_dir.join(&rel_plain));
    if let Some(up) = exe_dir.parent() {
        candidates.push(up.join("Resources").join(&rel_assets));
    }
    candidates.push(rel_assets.clone());
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or(rel_assets)
}

/// Parse an `R,G,B` triple.
fn parse_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// Default pastel status colors, indexed by status position.
fn default_status_colors() -> [(u8, u8, u8); crate::status::COUNT] {
    let mut c = [(0u8, 0u8, 0u8); crate::status::COUNT];
    for s in crate::status::Status::ALL {
        c[s.index()] = s.default_rgb();
    }
    c
}

impl AppConfig {
    /// The last matrix chosen for a target language (B-17).
    pub fn last_matrix_for(&self, lang: &str) -> Option<String> {
        self.last_matrix.split('|').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == lang && !v.is_empty()).then(|| v.to_string())
        })
    }

    /// Remember the matrix chosen for a target language (B-17).
    pub fn set_last_matrix(&mut self, lang: &str, name: &str) {
        let mut pairs: Vec<String> = self
            .last_matrix
            .split('|')
            .filter(|kv| !kv.is_empty() && kv.split_once('=').map(|(k, _)| k) != Some(lang))
            .map(str::to_string)
            .collect();
        if !name.is_empty() {
            pairs.push(format!("{lang}={}", name.replace(['|', '='], " ")));
        }
        self.last_matrix = pairs.join("|");
    }

    /// Directory holding the configuration file (next to the executable in
    /// portable mode, the per-user folder otherwise — see [`paths`]).
    pub fn config_dir() -> Option<PathBuf> {
        Some(paths::get().config_dir.clone())
    }

    /// Full path to `Config.ini`.
    pub fn config_path() -> Option<PathBuf> {
        Some(paths::get().config_path())
    }

    /// Load the configuration, creating a default file if none exists.
    pub fn load() -> anyhow::Result<Self> {
        let path =
            Self::config_path().ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-config-path")))?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(Self::parse_ini(&content))
        } else {
            let config = Self::default();
            if let Err(e) = config.save() {
                tracing::warn!("failed to create default Config.ini: {e}");
            }
            Ok(config)
        }
    }

    /// The configuration as it is on disk, a default one when there is
    /// none — nothing written (the command line, R-D06-01).
    pub fn load_read_only() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|c| Self::parse_ini(&c))
            .unwrap_or_default()
    }

    fn parse_ini(content: &str) -> Self {
        let mut config = Self::default();
        let mut values: HashMap<String, String> = HashMap::new();

        // The keys are global, except those of [Confirmations] (E-03), read
        // in their section only.
        let mut in_confirmations = false;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_confirmations = line[1..line.len() - 1]
                    .trim()
                    .eq_ignore_ascii_case("Confirmations");
                continue;
            }
            if let Some(pos) = line.find('=') {
                let key = line[..pos].trim().to_string();
                let value = line[pos + 1..].trim().to_string();
                if in_confirmations {
                    if crate::remember::Question::from_key(&key).is_some() && !value.is_empty() {
                        config.confirmations.insert(key, value);
                    }
                    continue;
                }
                values.insert(key, value);
            }
        }

        if let Some(v) = values.get("Locale") {
            config.locale = v.clone();
        }
        if let Some(v) = values.get("Country") {
            config.country = v.clone();
        }
        if let Some(v) = values.get("SourceLanguage") {
            config.source_language = v.clone();
        }
        if let Some(v) = values.get("TargetLanguage") {
            config.target_language = v.clone();
        }
        // The game is only a default since E-12 (each document carries its own);
        // the historical SelectedGame key is deliberately ignored.
        if let Some(v) = values.get("DefaultGame") {
            config.default_game = v.trim().to_string();
        }
        if let Some(v) = values.get("OutputMode") {
            config.output_mode = OutputMode::from_label(v);
        }
        if let Some(v) = values.get("PersonalDictDefault") {
            config.personal_dict_default = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("Theme") {
            config.theme = Theme::from_label(v);
        }
        if let Some(v) = values.get("FontSize") {
            config.font_size = v.parse().unwrap_or(14.0);
        }
        if let Some(v) = values.get("MaxRecentFiles") {
            config.max_recent_files = v.parse().unwrap_or(10);
        }
        if let Some(v) = values.get("SplashScreenSeconds") {
            config.splash_screen_seconds = v.parse().unwrap_or(2);
        }
        if let Some(v) = values.get("WindowWidth") {
            config.window_width = v.parse().unwrap_or(1280.0);
        }
        if let Some(v) = values.get("WindowHeight") {
            config.window_height = v.parse().unwrap_or(800.0);
        }
        let getf = |k: &str| values.get(k).and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = getf("WindowX") {
            config.window_x = v;
        }
        if let Some(v) = getf("WindowY") {
            config.window_y = v;
        }
        if let Some(v) = getf("SettingsX") {
            config.settings_x = v;
        }
        if let Some(v) = getf("SettingsY") {
            config.settings_y = v;
        }
        if let Some(v) = getf("SettingsW") {
            config.settings_w = v;
        }
        if let Some(v) = getf("SettingsH") {
            config.settings_h = v;
        }
        if let Some(v) = getf("FlagsX") {
            config.flags_x = v;
        }
        if let Some(v) = getf("FieldsWinX") {
            config.fields_win_x = v;
        }
        if let Some(v) = getf("FieldsWinY") {
            config.fields_win_y = v;
        }
        if let Some(v) = getf("FieldsWinW") {
            config.fields_win_w = v;
        }
        if let Some(v) = getf("FieldsWinH") {
            config.fields_win_h = v;
        }
        for (i, k) in ["FomodWinX", "FomodWinY", "FomodWinW", "FomodWinH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.fomod_win[i] = v;
            }
        }
        for (i, k) in ["PackWinX", "PackWinY", "PackWinW", "PackWinH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.pack_win[i] = v;
            }
        }
        for (i, k) in ["FormatsWinX", "FormatsWinY", "FormatsWinW", "FormatsWinH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.formats_win[i] = v;
            }
        }
        for (i, k) in ["TrWinX", "TrWinY", "TrWinW", "TrWinH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.tr_win[i] = v;
            }
        }
        for (i, k) in ["EditX", "EditY", "EditW", "EditH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.edit_win[i] = v;
            }
        }
        for (i, k) in ["ReplaceX", "ReplaceY", "ReplaceW", "ReplaceH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.replace_win[i] = v;
            }
        }
        for (i, k) in ["MatricesX", "MatricesY", "MatricesW", "MatricesH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.matrices_win[i] = v;
            }
        }
        for (i, k) in ["RulesX", "RulesY", "RulesW", "RulesH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.rules_win[i] = v;
            }
        }
        for (i, k) in ["AssistX", "AssistY", "AssistW", "AssistH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.assist_win[i] = v;
            }
        }
        for (i, k) in ["PropsX", "PropsY", "PropsW", "PropsH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.props_win[i] = v;
            }
        }
        for (i, k) in ["ScanX", "ScanY", "ScanW", "ScanH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.scan_win[i] = v;
            }
        }
        for (i, k) in ["CompareX", "CompareY", "CompareW", "CompareH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.compare_win[i] = v;
            }
        }
        for (i, k) in ["QualityX", "QualityY", "QualityW", "QualityH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.quality_win[i] = v;
            }
        }
        for (i, k) in ["HistX", "HistY", "HistW", "HistH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.hist_win[i] = v;
            }
        }
        for (i, k) in ["SearchX", "SearchY", "SearchW", "SearchH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.search_win[i] = v;
            }
        }
        for (i, k) in ["ScriptsX", "ScriptsY", "ScriptsW", "ScriptsH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.scripts_win[i] = v;
            }
        }
        for (i, k) in ["ConcordX", "ConcordY", "ConcordW", "ConcordH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.concord_win[i] = v;
            }
        }
        for (i, k) in ["JournalX", "JournalY", "JournalW", "JournalH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.journal_win[i] = v;
            }
        }
        for (i, k) in ["DialogX", "DialogY", "DialogW", "DialogH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.dialog_win[i] = v;
            }
        }
        if let Some(v) = values.get("FollowGrid") {
            config.dialog_follow_grid = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values
            .get("Split")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            config.dialog_split = v.clamp(0.2, 0.8);
        }
        for (i, k) in ["DictsX", "DictsY", "DictsW", "DictsH"].iter().enumerate() {
            if let Some(v) = getf(k) {
                config.dicts_win[i] = v;
            }
        }
        for (i, k) in ["DictEditX", "DictEditY", "DictEditW", "DictEditH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.dict_edit_win[i] = v;
            }
        }
        for (i, k) in ["UpdateX", "UpdateY", "UpdateW", "UpdateH"]
            .iter()
            .enumerate()
        {
            if let Some(v) = getf(k) {
                config.update_win[i] = v;
            }
        }
        if let Some(v) = values.get("Checks") {
            config.quality_checks = v.trim().to_string();
        }
        if let Some(v) = getf("FlagsY") {
            config.flags_y = v;
        }
        if let Some(v) = getf("FlagsW") {
            config.flags_w = v;
        }
        if let Some(v) = getf("FlagsH") {
            config.flags_h = v;
        }
        if let Some(v) = getf("AboutX") {
            config.about_x = v;
        }
        if let Some(v) = getf("AboutY") {
            config.about_y = v;
        }
        if let Some(v) = getf("StatusColorsX") {
            config.status_colors_x = v;
        }
        if let Some(v) = getf("StatusColorsY") {
            config.status_colors_y = v;
        }
        if let Some(v) = getf("PickerX") {
            config.picker_x = v;
        }
        if let Some(v) = getf("PickerY") {
            config.picker_y = v;
        }
        if let Some(v) = getf("SpellX") {
            config.spell_x = v;
        }
        if let Some(v) = getf("SpellY") {
            config.spell_y = v;
        }
        if let Some(v) = getf("SpellW") {
            config.spell_w = v;
        }
        if let Some(v) = getf("SpellH") {
            config.spell_h = v;
        }
        if let Some(v) = getf("ArchiveX") {
            config.archive_x = v;
        }
        if let Some(v) = getf("ArchiveY") {
            config.archive_y = v;
        }
        if let Some(v) = getf("ArchiveW") {
            config.archive_w = v;
        }
        if let Some(v) = getf("ArchiveH") {
            config.archive_h = v;
        }
        if let Some(v) = getf("BookX") {
            config.book_x = v;
        }
        if let Some(v) = getf("BookY") {
            config.book_y = v;
        }
        if let Some(v) = getf("BookW") {
            config.book_w = v;
        }
        if let Some(v) = getf("BookH") {
            config.book_h = v;
        }
        if let Some(v) = getf("DictViewX") {
            config.dictview_x = v;
        }
        if let Some(v) = getf("DictViewY") {
            config.dictview_y = v;
        }
        if let Some(v) = getf("DictViewW") {
            config.dictview_w = v;
        }
        if let Some(v) = getf("DictViewH") {
            config.dictview_h = v;
        }
        if let Some(v) = getf("EsmWinX") {
            config.esm_win_x = v;
        }
        if let Some(v) = getf("EsmWinY") {
            config.esm_win_y = v;
        }
        if let Some(v) = getf("EsmWinW") {
            config.esm_win_w = v;
        }
        if let Some(v) = getf("EsmWinH") {
            config.esm_win_h = v;
        }
        if let Some(v) = getf("BsaWinX") {
            config.bsa_win_x = v;
        }
        if let Some(v) = getf("BsaWinY") {
            config.bsa_win_y = v;
        }
        if let Some(v) = getf("BsaWinW") {
            config.bsa_win_w = v;
        }
        if let Some(v) = getf("BsaWinH") {
            config.bsa_win_h = v;
        }
        if let Some(v) = getf("SwfWinX") {
            config.swf_win_x = v;
        }
        if let Some(v) = getf("SwfWinY") {
            config.swf_win_y = v;
        }
        if let Some(v) = getf("SwfWinW") {
            config.swf_win_w = v;
        }
        if let Some(v) = getf("SwfWinH") {
            config.swf_win_h = v;
        }
        if let Some(v) = values.get("DictViewStripe")
            && let Some(rgb) = parse_rgb(v)
        {
            config.dictview_stripe = rgb;
        }
        if let Some(v) = values.get("DictViewStripeCustom") {
            config.dictview_stripe_custom = v.trim() != "0";
        }
        if let Some(v) = values.get("FirstStart") {
            config.first_start_done = v.trim() != "0";
        }
        if let Some(v) = values.get("GameBasesDir") {
            config.game_bases_dir = v.clone();
        }
        if let Some(v) = values.get("ModBasesDir") {
            config.mod_bases_dir = v.clone();
        }
        if let Some(v) = values.get("EetDir") {
            config.eet_dir = v.clone();
        }
        if let Some(v) = values
            .get("UndoDepth")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            config.undo_depth = v.clamp(
                *crate::undo::DEPTH_RANGE.start(),
                *crate::undo::DEPTH_RANGE.end(),
            );
        }
        // [History] (A-18).
        if let Some(v) = values.get("Enabled") {
            config.history_enabled = v.trim() != "0";
        }
        if let Some(v) = values
            .get("MaxEntries")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            config.history_max = v.clamp(
                *crate::history::MAX_RANGE.start(),
                *crate::history::MAX_RANGE.end(),
            );
        }
        if let Some(v) = values.get("FrameLog").and_then(|v| v.trim().parse().ok()) {
            config.frame_log_ms = v;
        }
        if let Some(v) = values.get("Translator") {
            config.translator = v.trim().to_string();
        }
        if let Some(v) = values.get("Toasts") {
            config.toasts = v.trim() != "0";
        }
        if let Some(v) = values.get("ShowUnused") {
            config.show_unused = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("PropagateAcrossFiles") {
            config.propagate_files = v.trim() != "0";
        }
        if let Some(v) = values.get("ProposePropagation") {
            config.propose_propagation = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("ColumnFilters") {
            config.column_filters = matches!(v.trim(), "1" | "true" | "yes");
        }
        // [Columns] (D-42); ShowComment was under [Editing] before: the
        // keys are read whatever their section, so it is taken over.
        if let Some(v) = values.get("ShowComment") {
            config.show_comment = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("RichTooltips") {
            config.rich_tooltips = !matches!(v.trim(), "0" | "false" | "no");
        }
        if let Some(v) = values.get("ShowProvenance") {
            config.show_provenance = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("ShowReview") {
            config.show_review = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("ShowStatusButtons") {
            config.show_status_buttons = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("GeneratedComments") {
            config.export_generated_comments = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("UntranslatedCopy") {
            config.export_untranslated_copy = v.trim() != "0";
        }
        if let Some(v) = values.get("LocalizeSourceStrings") {
            config.localize_source_strings = v.trim() != "0";
        }
        if let Some(v) = values.get("LastMatrix") {
            config.last_matrix = v.trim().to_string();
        }
        if let Some(v) = values.get("LastDir") {
            config.scan_last_dir = v.trim().to_string();
        }
        if let Some(v) = values.get("Subdirs") {
            config.scan_subdirs = v.trim() != "0";
        }
        if let Some(v) = values.get("Scripts") {
            config.scan_scripts = v.trim() == "1";
        }
        for (k, v) in &values {
            if let Some(ext) = k.strip_prefix("Last.")
                && !v.trim().is_empty()
            {
                config
                    .import_last
                    .insert(ext.to_lowercase(), v.trim().to_string());
            }
        }
        if let Some(v) = values.get("CopyToSource") {
            config.scripts_copy_to_source = v.trim() == "1";
        }
        // [Output] (E-06).
        if let Some(v) = values.get("KeepOriginal") {
            config.keep_original = v.trim() != "0";
        }
        if let Some(n) = values
            .get("OriginalCopies")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            let r = crate::backup::ORIGINAL_COPIES_RANGE;
            config.original_copies = n.clamp(*r.start(), *r.end());
        }
        // [Online] (D-01 / D-02): never a key here.
        {
            let o = &mut config.online;
            let gs = |k: &str| values.get(k).map(|v| v.trim().to_string());
            let gu = |k: &str| values.get(k).and_then(|v| v.trim().parse::<usize>().ok());
            let gb = |k: &str| {
                values
                    .get(k)
                    .map(|v| matches!(v.trim(), "1" | "true" | "yes"))
            };
            if let Some(v) = gs("Service") {
                o.service = crate::online::Service::from_id(&v);
            }
            if let Some(v) = gs("DeepLEndpoint") {
                o.deepl_endpoint = v;
            }
            if let Some(v) = gs("DeepLFormality") {
                o.deepl_formality = v;
            }
            if let Some(v) = gs("AzureRegion") {
                o.azure_region = v;
            }
            if let Some(v) = gs("AzureEndpoint") {
                o.azure_endpoint = v;
            }
            if let Some(v) = gs("OpenAIEndpoint") {
                o.openai_endpoint = v;
            }
            if let Some(v) = gs("OpenAIModel") {
                o.openai_model = v;
            }
            if let Some(v) = gu("MaxCharsPerRequest") {
                o.max_chars = v.clamp(100, 100_000);
            }
            if let Some(v) = gu("MaxLinesPerBatch") {
                o.max_lines = v.clamp(1, 1000);
            }
            if let Some(v) = gu("MaxRequestsPerMinute") {
                o.max_rpm = v.clamp(1, 600) as u32;
            }
            if let Some(v) = gu("SessionWarnChars") {
                o.session_warn = v;
            }
            if let Some(v) = gb("ProtectTags") {
                o.protect_tags = v;
            }
            if let Some(v) = gb("ProtectNumbers") {
                o.protect_numbers = v;
            }
            if let Some(v) = gb("ProtectNewlines") {
                o.protect_newlines = v;
            }
            if let Some(v) = gs("Proxy") {
                o.proxy = v;
            }
            if let Some(v) = gu("TimeoutSeconds") {
                o.timeout_secs = v.clamp(5, 600) as u64;
            }
            if let Some(v) = gs("LastScope") {
                o.last_scope = v;
            }
            if let Some(v) = gs("BrowserSite") {
                o.browser_site = crate::online::browser::Site::from_id(&v);
            }
        }
        if let Some(v) = values.get("ShowSpecialChars") {
            config.show_special_chars = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("EditTab") {
            let v = v.trim();
            if !v.is_empty() {
                config.edit_tab = v.to_string();
            }
        }
        if let Some(v) = values.get("AutosaveEnabled") {
            config.autosave_enabled = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values
            .get("AutosaveMinutes")
            .and_then(|v| v.trim().parse::<u32>().ok())
        {
            let r = crate::backup::AUTOSAVE_MINUTES_RANGE;
            config.autosave_minutes = v.clamp(*r.start(), *r.end());
        }
        if let Some(v) = values.get("AutosaveDir") {
            config.autosave_dir = v.trim().to_string();
        }
        if let Some(v) = values
            .get("Copies")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            let r = crate::backup::COPIES_RANGE;
            config.backup_copies = v.clamp(*r.start(), *r.end());
        }
        if let Some(v) = values.get("CheckCrc") {
            config.check_crc = v.trim() != "0";
        }
        if let Some(v) = values.get("ModDictAuto") {
            config.mod_dict_auto = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values
            .get("OldDataMonths")
            .and_then(|v| v.trim().parse::<u32>().ok())
        {
            config.old_data_months = v.min(240);
        }
        if let Some(v) = values
            .get("Threshold")
            .and_then(|v| v.trim().parse::<u8>().ok())
        {
            config.fuzzy_threshold = v.clamp(5, 50);
        }
        if let Some(v) = values.get("Normalize") {
            config.normalize = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("ProposeGeneralization") {
            config.propose_generalization = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("AutoFill") {
            config.personal_autofill = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("KeepField") {
            config.personal_keep_field = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = values.get("XTranslatorDir") {
            config.xtranslator_dir = v.clone();
        }

        // Per-status colors: `StatusColor_<key>=R,G,B`.
        for s in crate::status::Status::ALL {
            if let Some(v) = values.get(&format!("StatusColor_{}", s.key()))
                && let Some(rgb) = parse_rgb(v)
            {
                config.status_colors[s.index()] = rgb;
            }
        }

        // [Dictionaries] — the trees (B-02).
        for (k, v) in &values {
            if let Some(key) = k.strip_prefix("Dicts_")
                && !key.is_empty()
                && !v.trim().is_empty()
            {
                config
                    .dictionaries
                    .insert(key.to_string(), v.trim().to_string());
            }
        }
        // [Games] — detected/registered installations.
        if let Some(v) = values.get("AutoDetectGames") {
            config.auto_detect_games = v.trim() != "0";
        }
        for (k, v) in &values {
            if let Some(id) = k.strip_prefix("GamePath_") {
                config.games.entry(id.to_string()).or_default().path = v.clone();
            } else if let Some(id) = k.strip_prefix("GameSource_") {
                config.games.entry(id.to_string()).or_default().source = v.clone();
            } else if let Some(id) = k.strip_prefix("GameLocked_") {
                config.games.entry(id.to_string()).or_default().locked = v.trim() != "0";
            }
        }
        // Drop entries that ended up with no path (stray keys).
        config.games.retain(|_, e| !e.path.trim().is_empty());

        config.recent_files.clear();
        for i in 0..config.max_recent_files {
            let key = format!("RecentFile{i}");
            if let Some(v) = values.get(&key)
                && !v.is_empty()
            {
                config.recent_files.push(PathBuf::from(v));
            }
        }

        config
    }

    /// Persist the configuration to `Config.ini` (atomically: a crash never
    /// leaves a truncated file). Refused, with an explicit error, when the
    /// configuration folder is read-only (R-E05-06).
    pub fn save(&self) -> anyhow::Result<()> {
        let p = paths::get();
        if p.read_only {
            anyhow::bail!(crate::l10n!(
                "err-config-readonly",
                path = p.config_dir.display()
            ));
        }
        crate::slow_io("Config.ini", || {
            paths::write_atomic(&p.config_path(), self.to_ini().as_bytes())
        })?;
        Ok(())
    }

    fn to_ini(&self) -> String {
        let mut lines = Vec::new();
        lines.push("; XIMOD Translator configuration".to_string());
        lines.push("; Edit manually at your own risk.".to_string());
        lines.push(String::new());

        lines.push("[General]".to_string());
        // Informative only (R-E05): the mode is decided by the presence of
        // Config.ini next to the executable, never by this key.
        lines.push(format!("ConfigMode={}", paths::get().mode.as_str()));
        lines.push(format!("Locale={}", self.locale));
        lines.push(format!("Country={}", self.country));
        lines.push(format!(
            "FirstStart={}",
            if self.first_start_done { 1 } else { 0 }
        ));
        lines.push(format!("Theme={}", self.theme.as_str()));
        lines.push(format!("FontSize={}", self.font_size));
        lines.push(format!("MaxRecentFiles={}", self.max_recent_files));
        lines.push(format!(
            "SplashScreenSeconds={}",
            self.splash_screen_seconds
        ));
        lines.push(format!("Toasts={}", if self.toasts { 1 } else { 0 }));
        lines.push(format!("Translator={}", self.translator));
        lines.push(String::new());

        lines.push("[Translation]".to_string());
        lines.push(format!("SourceLanguage={}", self.source_language));
        lines.push(format!("TargetLanguage={}", self.target_language));
        lines.push(format!("OutputMode={}", self.output_mode.as_str()));
        lines.push(format!("DefaultGame={}", self.default_game));
        lines.push(format!(
            "PersonalDictDefault={}",
            if self.personal_dict_default { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Window]".to_string());
        lines.push(format!("WindowWidth={}", self.window_width));
        lines.push(format!("WindowHeight={}", self.window_height));
        lines.push(format!("WindowX={}", self.window_x));
        lines.push(format!("WindowY={}", self.window_y));
        lines.push(format!("SettingsX={}", self.settings_x));
        lines.push(format!("SettingsY={}", self.settings_y));
        lines.push(format!("SettingsW={}", self.settings_w));
        lines.push(format!("SettingsH={}", self.settings_h));
        lines.push(format!("FlagsX={}", self.flags_x));
        lines.push(format!("FlagsY={}", self.flags_y));
        lines.push(format!("FlagsW={}", self.flags_w));
        lines.push(format!("FlagsH={}", self.flags_h));
        lines.push(format!("FieldsWinX={}", self.fields_win_x));
        lines.push(format!("FieldsWinY={}", self.fields_win_y));
        lines.push(format!("FieldsWinW={}", self.fields_win_w));
        lines.push(format!("FieldsWinH={}", self.fields_win_h));
        for (i, k) in ["FomodWinX", "FomodWinY", "FomodWinW", "FomodWinH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.fomod_win[i]));
        }
        for (i, k) in ["FormatsWinX", "FormatsWinY", "FormatsWinW", "FormatsWinH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.formats_win[i]));
        }
        for (i, k) in ["TrWinX", "TrWinY", "TrWinW", "TrWinH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.tr_win[i]));
        }
        for (i, k) in ["PackWinX", "PackWinY", "PackWinW", "PackWinH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.pack_win[i]));
        }
        for (i, k) in ["EditX", "EditY", "EditW", "EditH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.edit_win[i]));
        }
        for (i, k) in ["ReplaceX", "ReplaceY", "ReplaceW", "ReplaceH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.replace_win[i]));
        }
        for (i, k) in ["MatricesX", "MatricesY", "MatricesW", "MatricesH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.matrices_win[i]));
        }
        for (i, k) in ["RulesX", "RulesY", "RulesW", "RulesH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.rules_win[i]));
        }
        for (i, k) in ["AssistX", "AssistY", "AssistW", "AssistH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.assist_win[i]));
        }
        for (i, k) in ["PropsX", "PropsY", "PropsW", "PropsH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.props_win[i]));
        }
        for (i, k) in ["ScanX", "ScanY", "ScanW", "ScanH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.scan_win[i]));
        }
        for (i, k) in ["CompareX", "CompareY", "CompareW", "CompareH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.compare_win[i]));
        }
        for (i, k) in ["QualityX", "QualityY", "QualityW", "QualityH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.quality_win[i]));
        }
        for (i, k) in ["HistX", "HistY", "HistW", "HistH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.hist_win[i]));
        }
        for (i, k) in ["SearchX", "SearchY", "SearchW", "SearchH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.search_win[i]));
        }
        for (i, k) in ["ScriptsX", "ScriptsY", "ScriptsW", "ScriptsH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.scripts_win[i]));
        }
        for (i, k) in ["ConcordX", "ConcordY", "ConcordW", "ConcordH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.concord_win[i]));
        }
        for (i, k) in ["JournalX", "JournalY", "JournalW", "JournalH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.journal_win[i]));
        }
        for (i, k) in ["DialogX", "DialogY", "DialogW", "DialogH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.dialog_win[i]));
        }
        for (i, k) in ["DictsX", "DictsY", "DictsW", "DictsH"].iter().enumerate() {
            lines.push(format!("{k}={}", self.dicts_win[i]));
        }
        for (i, k) in ["DictEditX", "DictEditY", "DictEditW", "DictEditH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.dict_edit_win[i]));
        }
        for (i, k) in ["UpdateX", "UpdateY", "UpdateW", "UpdateH"]
            .iter()
            .enumerate()
        {
            lines.push(format!("{k}={}", self.update_win[i]));
        }
        lines.push(String::new());
        lines.push("[History]".to_string());
        lines.push(format!(
            "Enabled={}",
            if self.history_enabled { 1 } else { 0 }
        ));
        lines.push(format!("MaxEntries={}", self.history_max));
        lines.push(String::new());

        lines.push("[Diagnostics]".to_string());
        lines.push(format!("FrameLog={}", self.frame_log_ms));
        lines.push(String::new());

        lines.push("[Quality]".to_string());
        lines.push(format!("Checks={}", self.quality_checks));
        // About / Status-colors / Color-picker: fixed size, so only positions
        // are persisted (never a width/height).
        lines.push(format!("AboutX={}", self.about_x));
        lines.push(format!("AboutY={}", self.about_y));
        lines.push(format!("StatusColorsX={}", self.status_colors_x));
        lines.push(format!("StatusColorsY={}", self.status_colors_y));
        lines.push(format!("PickerX={}", self.picker_x));
        lines.push(format!("PickerY={}", self.picker_y));
        lines.push(format!("SpellX={}", self.spell_x));
        lines.push(format!("SpellY={}", self.spell_y));
        lines.push(format!("SpellW={}", self.spell_w));
        lines.push(format!("SpellH={}", self.spell_h));
        lines.push(format!("ArchiveX={}", self.archive_x));
        lines.push(format!("ArchiveY={}", self.archive_y));
        lines.push(format!("ArchiveW={}", self.archive_w));
        lines.push(format!("ArchiveH={}", self.archive_h));
        lines.push(format!("BookX={}", self.book_x));
        lines.push(format!("BookY={}", self.book_y));
        lines.push(format!("BookW={}", self.book_w));
        lines.push(format!("BookH={}", self.book_h));
        lines.push(format!("DictViewX={}", self.dictview_x));
        lines.push(format!("DictViewY={}", self.dictview_y));
        lines.push(format!("DictViewW={}", self.dictview_w));
        lines.push(format!("DictViewH={}", self.dictview_h));
        lines.push(format!("EsmWinX={}", self.esm_win_x));
        lines.push(format!("EsmWinY={}", self.esm_win_y));
        lines.push(format!("EsmWinW={}", self.esm_win_w));
        lines.push(format!("EsmWinH={}", self.esm_win_h));
        lines.push(format!("BsaWinX={}", self.bsa_win_x));
        lines.push(format!("BsaWinY={}", self.bsa_win_y));
        lines.push(format!("BsaWinW={}", self.bsa_win_w));
        lines.push(format!("BsaWinH={}", self.bsa_win_h));
        lines.push(format!("SwfWinX={}", self.swf_win_x));
        lines.push(format!("SwfWinY={}", self.swf_win_y));
        lines.push(format!("SwfWinW={}", self.swf_win_w));
        lines.push(format!("SwfWinH={}", self.swf_win_h));
        let (sr, sg, sb) = self.dictview_stripe;
        lines.push(format!("DictViewStripe={sr},{sg},{sb}"));
        lines.push(format!(
            "DictViewStripeCustom={}",
            if self.dictview_stripe_custom { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Bases]".to_string());
        lines.push(format!("GameBasesDir={}", self.game_bases_dir));
        lines.push(format!("ModBasesDir={}", self.mod_bases_dir));
        lines.push(String::new());

        lines.push("[Editing]".to_string());
        lines.push(format!("UndoDepth={}", self.undo_depth));
        lines.push(format!(
            "ShowUnused={}",
            if self.show_unused { 1 } else { 0 }
        ));
        lines.push(format!("EditTab={}", self.edit_tab));
        lines.push(format!(
            "ShowSpecialChars={}",
            if self.show_special_chars { 1 } else { 0 }
        ));
        lines.push(format!(
            "ProposePropagation={}",
            if self.propose_propagation { 1 } else { 0 }
        ));
        lines.push(format!(
            "PropagateAcrossFiles={}",
            if self.propagate_files { 1 } else { 0 }
        ));
        lines.push(format!(
            "ColumnFilters={}",
            if self.column_filters { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Columns]".to_string());
        lines.push(format!(
            "ShowStatusButtons={}",
            if self.show_status_buttons { 1 } else { 0 }
        ));
        lines.push(format!(
            "ShowComment={}",
            if self.show_comment { 1 } else { 0 }
        ));
        lines.push(format!("RichTooltips={}", u8::from(self.rich_tooltips)));
        lines.push(format!(
            "ShowReview={}",
            if self.show_review { 1 } else { 0 }
        ));
        lines.push(format!(
            "ShowProvenance={}",
            if self.show_provenance { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Export]".to_string());
        lines.push(format!(
            "GeneratedComments={}",
            if self.export_generated_comments { 1 } else { 0 }
        ));
        lines.push(format!(
            "UntranslatedCopy={}",
            if self.export_untranslated_copy { 1 } else { 0 }
        ));
        lines.push(format!(
            "LocalizeSourceStrings={}",
            if self.localize_source_strings { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Replace]".to_string());
        lines.push(format!("LastMatrix={}", self.last_matrix));
        lines.push(String::new());

        lines.push("[Scan]".to_string());
        lines.push(format!("LastDir={}", self.scan_last_dir));
        lines.push(format!("Subdirs={}", u8::from(self.scan_subdirs)));
        lines.push(format!("Scripts={}", u8::from(self.scan_scripts)));
        lines.push(String::new());

        lines.push("[Import]".to_string());
        for (ext, id) in &self.import_last {
            lines.push(format!("Last.{ext}={id}"));
        }
        lines.push(String::new());

        lines.push("[Scripts]".to_string());
        lines.push(format!(
            "CopyToSource={}",
            if self.scripts_copy_to_source { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Output]".to_string());
        lines.push(format!(
            "KeepOriginal={}",
            if self.keep_original { 1 } else { 0 }
        ));
        lines.push(format!("OriginalCopies={}", self.original_copies));
        lines.push(String::new());

        let o = &self.online;
        lines.push("[Online]".to_string());
        lines.push(format!(
            "Service={}",
            o.service.as_ref().map(|s| s.id()).unwrap_or_default()
        ));
        lines.push(format!("DeepLEndpoint={}", o.deepl_endpoint));
        lines.push(format!("DeepLFormality={}", o.deepl_formality));
        lines.push(format!("AzureRegion={}", o.azure_region));
        lines.push(format!("AzureEndpoint={}", o.azure_endpoint));
        lines.push(format!("OpenAIEndpoint={}", o.openai_endpoint));
        lines.push(format!("OpenAIModel={}", o.openai_model));
        lines.push(format!("MaxCharsPerRequest={}", o.max_chars));
        lines.push(format!("MaxLinesPerBatch={}", o.max_lines));
        lines.push(format!("MaxRequestsPerMinute={}", o.max_rpm));
        lines.push(format!("SessionWarnChars={}", o.session_warn));
        lines.push(format!("ProtectTags={}", u8::from(o.protect_tags)));
        lines.push(format!("ProtectNumbers={}", u8::from(o.protect_numbers)));
        lines.push(format!("ProtectNewlines={}", u8::from(o.protect_newlines)));
        lines.push(format!("Proxy={}", o.proxy));
        lines.push(format!("TimeoutSeconds={}", o.timeout_secs));
        lines.push(format!("LastScope={}", o.last_scope));
        lines.push(format!("BrowserSite={}", o.browser_site.id()));
        lines.push(String::new());

        lines.push("[Backup]".to_string());
        lines.push(format!(
            "AutosaveEnabled={}",
            if self.autosave_enabled { 1 } else { 0 }
        ));
        lines.push(format!("AutosaveMinutes={}", self.autosave_minutes));
        lines.push(format!("AutosaveDir={}", self.autosave_dir));
        lines.push(format!("Copies={}", self.backup_copies));
        lines.push(format!(
            "ModDictAuto={}",
            if self.mod_dict_auto { 1 } else { 0 }
        ));
        lines.push(format!("OldDataMonths={}", self.old_data_months));
        lines.push(String::new());

        lines.push("[Tools]".to_string());
        lines.push(format!("EetDir={}", self.eet_dir));
        lines.push(format!("XTranslatorDir={}", self.xtranslator_dir));
        lines.push(String::new());

        lines.push("[Fuzzy]".to_string());
        lines.push(format!("Threshold={}", self.fuzzy_threshold));
        lines.push(format!("Normalize={}", u8::from(self.normalize)));
        lines.push(String::new());
        lines.push("[Rules]".to_string());
        lines.push(format!(
            "ProposeGeneralization={}",
            u8::from(self.propose_generalization)
        ));
        lines.push(String::new());

        lines.push("[Dialogues]".to_string());
        lines.push(format!(
            "FollowGrid={}",
            if self.dialog_follow_grid { 1 } else { 0 }
        ));
        lines.push(format!("Split={:.3}", self.dialog_split));
        lines.push(String::new());

        lines.push("[Personal]".to_string());
        lines.push(format!(
            "AutoFill={}",
            if self.personal_autofill { 1 } else { 0 }
        ));
        lines.push(format!(
            "KeepField={}",
            if self.personal_keep_field { 1 } else { 0 }
        ));
        lines.push(String::new());

        lines.push("[Dictionaries]".to_string());
        lines.push(format!("CheckCrc={}", if self.check_crc { 1 } else { 0 }));
        for (k, v) in &self.dictionaries {
            lines.push(format!("Dicts_{k}={v}"));
        }
        lines.push(String::new());

        lines.push("[Games]".to_string());
        lines.push(format!(
            "AutoDetectGames={}",
            if self.auto_detect_games { 1 } else { 0 }
        ));
        for (id, e) in &self.games {
            if e.path.trim().is_empty() {
                continue;
            }
            lines.push(format!("GamePath_{id}={}", e.path));
            if !e.source.is_empty() {
                lines.push(format!("GameSource_{id}={}", e.source));
            }
            if e.locked {
                lines.push(format!("GameLocked_{id}=1"));
            }
        }
        lines.push(String::new());

        lines.push("[Statuses]".to_string());
        for s in crate::status::Status::ALL {
            let (r, g, b) = self.status_colors[s.index()];
            lines.push(format!("StatusColor_{}={r},{g},{b}", s.key()));
        }
        lines.push(String::new());

        // E-03: only when something is remembered for good.
        if !self.confirmations.is_empty() {
            lines.push("[Confirmations]".to_string());
            for (k, v) in &self.confirmations {
                lines.push(format!("{k}={v}"));
            }
            lines.push(String::new());
        }

        lines.push("[RecentFiles]".to_string());
        for (i, path) in self.recent_files.iter().enumerate() {
            lines.push(format!("RecentFile{i}={}", path.display()));
        }

        lines.join("\n")
    }

    /// The game-bases and mod-bases folders, resolved to absolute paths. When a
    /// path is not set in the config, it defaults to the writable base folder
    /// ([`default_base_dir`]: `assets/bases/<sub>` next to the executable in
    /// portable mode, `<data>/bases/<sub>` in user mode). Game folder first, so
    /// mod translations override game translations for the same key.
    pub fn resolved_base_dirs(&self) -> Vec<PathBuf> {
        let g = if self.game_bases_dir.trim().is_empty() {
            default_base_dir("jeux")
        } else {
            PathBuf::from(self.game_bases_dir.trim())
        };
        let m = if self.mod_bases_dir.trim().is_empty() {
            default_base_dir("mods")
        } else {
            PathBuf::from(self.mod_bases_dir.trim())
        };
        vec![g, m]
    }

    /// Base folders to index for auto-translation given the selected game: only
    /// that game's sub-folder under `jeux` (plus all mod bases). When no game is
    /// selected, falls back to the whole `jeux` tree (as [`resolved_base_dirs`]).
    ///
    /// In user mode the shipped bases next to the executable (read-only, D-06)
    /// are appended so they are indexed too when they exist and differ from
    /// the writable folders.
    pub fn base_dirs_for_selected(&self) -> Vec<PathBuf> {
        self.base_dirs_for(&self.default_game)
    }

    /// Base folders to index for a given game (E-12: each document has its
    /// own): `jeux/<game>` and `mods/<game>`; the whole trees when `game` is
    /// empty. Game folder first, so mod translations override game translations
    /// for the same key.
    pub fn base_dirs_for(&self, game: &str) -> Vec<PathBuf> {
        let dirs = self.resolved_base_dirs();
        let (jeux, mods) = (dirs[0].clone(), dirs[1].clone());
        let g = game.trim();
        let sub = |d: &PathBuf| if g.is_empty() { d.clone() } else { d.join(g) };
        let mut out = vec![sub(&jeux), sub(&mods)];
        if paths::get().mode == ConfigMode::User {
            for (name, writable) in [("jeux", &jeux), ("mods", &mods)] {
                let shipped = shipped_base_dir(name);
                if shipped.is_dir() && &shipped != writable {
                    let d = sub(&shipped);
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
            }
        }
        out
    }

    /// The rescue folder (B-11): `[Backup] AutosaveDir`, or `<config>/autosave`.
    pub fn autosave_dir(&self) -> PathBuf {
        let d = self.autosave_dir.trim();
        if d.is_empty() {
            paths::get().autosave_dir.clone()
        } else {
            PathBuf::from(d)
        }
    }

    /// The game-bases folder (first of [`resolved_base_dirs`]).
    pub fn game_base_dir(&self) -> PathBuf {
        self.resolved_base_dirs().into_iter().next().unwrap()
    }

    /// The mod-bases folder (second of [`resolved_base_dirs`]).
    pub fn mod_base_dir(&self) -> PathBuf {
        self.resolved_base_dirs().into_iter().nth(1).unwrap()
    }

    /// Path registered for a game id (non-empty), if any.
    pub fn game_path(&self, id: &str) -> Option<&str> {
        self.games
            .get(id)
            .map(|e| e.path.as_str())
            .filter(|p| !p.trim().is_empty())
    }

    /// Register or update a game's install path unless the existing entry is
    /// locked (a manual choice to preserve). Returns `true` if it was written.
    pub fn set_game(&mut self, id: &str, path: &str, source: &str, locked: bool) -> bool {
        if let Some(e) = self.games.get(id)
            && e.locked
        {
            return false;
        }
        self.games.insert(
            id.to_string(),
            GameEntry {
                path: path.to_string(),
                source: source.to_string(),
                locked,
            },
        );
        true
    }

    /// Add a file to the front of the recent list (deduplicated, capped).
    pub fn add_recent_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        while self.recent_files.len() > self.max_recent_files {
            self.recent_files.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usual_output_follows_the_document_and_its_game() {
        use crate::games::Game;
        // RFM.esm: a plugin with its texts inline.
        assert_eq!(
            OutputMode::usual(Game::SkyrimSE, false, false, false),
            Some(OutputMode::OverridePatch)
        );
        assert_eq!(
            OutputMode::usual(Game::SkyrimSE, true, false, false),
            Some(OutputMode::Strings)
        );
        assert_eq!(
            OutputMode::usual(Game::Fallout4, true, false, false),
            Some(OutputMode::Strings)
        );
        // Oblivion has no STRINGS: a localized flag changes nothing.
        assert_eq!(
            OutputMode::usual(Game::Oblivion, true, false, false),
            Some(OutputMode::OverridePatch)
        );
        assert_eq!(
            OutputMode::usual(Game::Morrowind, false, false, false),
            Some(OutputMode::FullPlugin)
        );
        assert_eq!(
            OutputMode::usual(Game::SkyrimSE, false, true, false),
            Some(OutputMode::Text)
        );
        assert_eq!(
            OutputMode::usual(Game::SkyrimSE, false, false, true),
            Some(OutputMode::Scripts)
        );
        // Every output has its sentence, and none the same key.
        let mut keys: Vec<&str> = OutputMode::variants()
            .iter()
            .chain([OutputMode::Text, OutputMode::Scripts].iter())
            .map(|m| m.desc_key())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 7);
        // The reference language has them (the others are checked against
        // it by the messages test), and the mark of the usual output.
        let eng = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales/eng/main.ftl"),
        )
        .unwrap();
        for k in keys.iter().chain(["output-usual"].iter()) {
            assert!(eng.contains(&format!("\n{k} = ")), "{k}");
        }
    }

    #[test]
    fn test_defaults() {
        let c = AppConfig::default();
        assert_eq!(c.source_language, "eng");
        assert_eq!(c.target_language, "fra");
        assert_eq!(c.output_mode, OutputMode::Hybrid);
    }

    #[test]
    fn test_ini_roundtrip() {
        let mut c = AppConfig {
            locale: "fra".into(),
            theme: Theme::Light,
            output_mode: OutputMode::Dsd,
            default_game: "SkyrimSE".into(),
            personal_dict_default: true,
            ..AppConfig::default()
        };
        c.recent_files.push(PathBuf::from("/tmp/x.eet"));
        let ini = c.to_ini();
        let parsed = AppConfig::parse_ini(&ini);
        assert_eq!(parsed.locale, "fra");
        assert_eq!(parsed.theme, Theme::Light);
        assert_eq!(parsed.output_mode, OutputMode::Dsd);
        assert_eq!(parsed.recent_files.len(), 1);
        assert_eq!(parsed.default_game, "SkyrimSE");
        assert!(parsed.personal_dict_default);
        // The historical key is ignored (E-12: the game is only a default).
        let old = AppConfig::parse_ini("[Translation]\nSelectedGame=Fallout4\n");
        assert_eq!(old.default_game, "");
    }

    #[test]
    fn test_games_roundtrip() {
        let mut c = AppConfig::default();
        c.set_game("SkyrimSE", "/games/Skyrim Special Edition", "Steam", false);
        c.set_game("Morrowind", "/games/Morrowind", "Manual", true);
        let ini = c.to_ini();
        let p = AppConfig::parse_ini(&ini);
        assert_eq!(
            p.game_path("SkyrimSE"),
            Some("/games/Skyrim Special Edition")
        );
        assert_eq!(p.games.get("Morrowind").map(|e| e.locked), Some(true));
        assert_eq!(
            p.games.get("SkyrimSE").map(|e| e.source.as_str()),
            Some("Steam")
        );
        // A locked entry is not overwritten by set_game.
        let mut p2 = p;
        assert!(!p2.set_game("Morrowind", "/other", "Steam", false));
        assert_eq!(p2.game_path("Morrowind"), Some("/games/Morrowind"));
    }

    #[test]
    fn test_tools_roundtrip() {
        let c = AppConfig {
            eet_dir: "F:\\Tools\\ESP-ESM Translator".into(),
            xtranslator_dir: "F:\\Tools\\xTranslator".into(),
            ..AppConfig::default()
        };
        let p = AppConfig::parse_ini(&c.to_ini());
        assert_eq!(p.eet_dir, "F:\\Tools\\ESP-ESM Translator");
        assert_eq!(p.xtranslator_dir, "F:\\Tools\\xTranslator");
    }

    #[test]
    fn test_edit_window_roundtrip() {
        // A-02: geometry under [Window] Edit*, last tab under [Editing] EditTab.
        let c = AppConfig {
            edit_win: [10.0, 20.0, 900.0, 600.0],
            edit_tab: "aliases".into(),
            ..AppConfig::default()
        };
        let ini = c.to_ini();
        assert!(ini.contains("EditX=10"));
        assert!(ini.contains("EditH=600"));
        assert!(ini.contains("EditTab=aliases"));
        let p = AppConfig::parse_ini(&ini);
        assert_eq!(p.edit_win, [10.0, 20.0, 900.0, 600.0]);
        assert_eq!(p.edit_tab, "aliases");
        // Defaults: near the main window, 760 × 520, Enregistrement.
        let d = AppConfig::default();
        assert_eq!(d.edit_win, [-1.0, -1.0, 760.0, 520.0]);
        assert_eq!(d.edit_tab, "record");
    }

    #[test]
    fn the_ctrl_g_site_is_saved_and_read_back() {
        use crate::online::browser::Site;
        assert_eq!(AppConfig::default().online.browser_site, Site::Google);
        let mut c = AppConfig::default();
        c.online.browser_site = Site::Claude;
        let ini = c.to_ini();
        assert!(ini.contains("BrowserSite=claude"), "{ini}");
        assert_eq!(AppConfig::parse_ini(&ini).online.browser_site, Site::Claude);
    }

    #[test]
    fn confirmations_round_trip_in_their_section_only() {
        // E-03: [Confirmations] only when something is remembered for good;
        // unknown questions are dropped; a same-named key elsewhere is not
        // an answer.
        let d = AppConfig::default();
        assert!(!d.to_ini().contains("[Confirmations]"));
        let mut c = AppConfig::default();
        c.confirmations.insert("Propagate".into(), "yes".into());
        c.confirmations
            .insert("PersonalConflict".into(), "keep".into());
        let ini = c.to_ini();
        assert!(ini.contains("[Confirmations]\nPersonalConflict=keep\nPropagate=yes"));
        let p = AppConfig::parse_ini(&ini);
        assert_eq!(p.confirmations, c.confirmations);
        let p = AppConfig::parse_ini(
            "[General]\nJournalClear=yes\n[Confirmations]\nCloseUnsaved=yes\nDictGame=yes\n[Window]\nOnlineVolume=yes\n",
        );
        assert_eq!(p.confirmations.len(), 1);
        assert_eq!(p.confirmations["DictGame"], "yes");
    }

    #[test]
    fn test_output_mode_parse() {
        assert_eq!(OutputMode::from_label("Dsd"), OutputMode::Dsd);
        assert_eq!(OutputMode::from_label("garbage"), OutputMode::Hybrid);
    }
}
