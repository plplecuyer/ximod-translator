//! The main application: `XimodApp` implements `eframe::App`.
//!
//! XIMOD Translator's shell: a native splash (handled in `main`), a menu bar, a
//! toolbar, one tab per open file, the translation grid, a status bar, and three
//! *independent* windows (Settings, Flags, About) rendered as immediate egui
//! viewports whose position and size are persisted to `Config.ini`.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use crate::ScreenInfo;
use crate::config::{AppConfig, OutputMode, Theme};
use crate::data::{CountriesData, CountryLanguagesData, CountryNamesData, LanguagesData};
use crate::dict_stack::DictStack;
use crate::doc_settings::{DocSettings, IdleSettings};
use crate::games::Game;
use crate::i18n::I18n;
use crate::tm::apply::{MergeOptions, merge};
use crate::tm::encoding::Codec;
use crate::tm::{self, TranslationMemory, WriteOutcome, eet::EetWriteMode};
use crate::ui::grid;
use crate::undo::{EntryState, OpKind, UndoStack, op_from_before, record};

#[path = "app_allmods.rs"]
mod app_allmods;
#[path = "app_assist.rs"]
mod app_assist;
#[path = "app_common.rs"]
mod app_common;
#[path = "app_concord.rs"]
mod app_concord;
#[path = "app_crc.rs"]
mod app_crc;
#[path = "app_derive.rs"]
mod app_derive;
#[path = "app_dialog.rs"]
mod app_dialog;
#[path = "app_encoding.rs"]
mod app_encoding;
#[path = "app_history.rs"]
mod app_history;
#[path = "app_journal.rs"]
mod app_journal;
#[path = "app_localize.rs"]
mod app_localize;
#[path = "app_matrices.rs"]
mod app_matrices;
#[path = "app_online.rs"]
mod app_online;
#[path = "app_patterns.rs"]
mod app_patterns;
#[path = "app_profiles.rs"]
mod app_profiles;
#[path = "app_props.rs"]
mod app_props;
#[path = "app_review.rs"]
mod app_review;
#[path = "app_rules.rs"]
mod app_rules;
#[path = "app_savepart.rs"]
mod app_savepart;
#[path = "app_scripts.rs"]
mod app_scripts;
#[path = "app_search.rs"]
mod app_search;
#[path = "app_text.rs"]
mod app_text;
#[path = "app_translation.rs"]
mod app_translation;
#[path = "app_vmad.rs"]
mod app_vmad;
#[path = "compare_window.rs"]
mod compare_window;
#[path = "import_dialog.rs"]
mod import_dialog;
#[path = "scan_window.rs"]
mod scan_window;

/// Fixed sizes (width, height) of the non-resizable windows. Their size is never
/// written to Config.ini — only their position is persisted.
const ABOUT_SIZE: [f32; 2] = [420.0, 140.0];
// Initial sizes; the window height is then auto-fitted to its content.
const STATUS_COLORS_SIZE: [f32; 2] = [380.0, 250.0];
const PICKER_SIZE: [f32; 2] = [560.0, 360.0];

/// Flags grid layout (matches XIMOD Architect).
const FLAG_COLUMNS: usize = 6;
const FLAG_CELL_W: f32 = 104.0;
const FLAG_CELL_H: f32 = 70.0;
const FLAG_ROW_H: f32 = FLAG_CELL_H + 26.0;

/// Why the flags window is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlagsPurpose {
    /// Settings ▸ Général: country → interface locale.
    Locale,
    /// Toolbar « Cible » button: country → the « Cible » list.
    Target,
    /// Outils ▸ Traduire l'interface… (E-10): the editor's country.
    Translation,
}

/// One open document (a plugin or a translation database) shown in its own tab.
pub(crate) struct Doc {
    path: PathBuf,
    tm: TranslationMemory,
    /// External STRINGS resolved for a localized plugin (kept for delocalization).
    strings: Option<crate::plugin::StringsBundle>,
    /// Game, languages, output mode and personal-dictionary choice (E-12).
    pub(crate) settings: DocSettings,
    /// The source languages the file provides (ISO 639-3): the STRINGS found
    /// for a localized plugin, the language a base declares, or the languages
    /// detected in the text otherwise. The « Source » list offers only these.
    source_langs: Vec<String>,
    /// Undo / redo stacks of this document (A-01, R-A01-02).
    undo: UndoStack,
    /// Hash of the translatable content when last loaded / saved; differs
    /// from [`Doc::content_hash`] when the document was modified.
    saved_hash: u64,
    /// `(revision, dirty)` memo so the tab marker costs nothing per frame.
    dirty_memo: std::cell::Cell<Option<(u64, bool)>>,
    /// Undo revision of the last autosave written (B-11).
    autosaved_revision: u64,
    /// Unique number of this document in the session (the view cache's key).
    serial: u64,
    /// A text document (C-05): the file's lines, for writing it back, and
    /// the format it was read with.
    text: Option<std::sync::Arc<crate::textfmt::TextDoc>>,
    text_fmt: Option<crate::textfmt::TextFormat>,
    /// The interface files found next to a plugin (R-C05-05).
    interface_files: Vec<PathBuf>,
    /// A « scripts » document (C-06): the compiled scripts its lines come
    /// from, for writing them back and for the Script tab.
    scripts: Option<std::sync::Arc<crate::scripts::ScriptSet>>,
    /// The name shown for a document that is not one file
    /// (« skipper.esp — scripts »).
    display: Option<String>,
    /// The history of the translations (A-18).
    history: crate::history::History,
    /// Name, size and CRC-32 of the plugin or text file as opened (B-12).
    ident: Option<crate::plugin_id::Ident>,
    /// The « Tous les mods » tab (B-25): the synthesis of the open files.
    all: Option<Box<app_allmods::AllMods>>,
    /// Bumped when the lines are re-extracted (the source index of C-04).
    sources_rev: u64,
    /// The deductions of the exclusion rules (B-18), built on first use.
    deduced: std::sync::OnceLock<std::sync::Arc<crate::exclusions::Deduced>>,
    /// The pages of code of the file (D-09).
    enc: app_encoding::EncState,
}

/// The view of a tab left (B-25): its key and what the filter computed.
pub(crate) struct StoredView {
    key: Option<(u64, u64, (usize, usize), crate::filter::Criteria)>,
    visible: Vec<usize>,
    total: usize,
    unique: usize,
    unique_remaining: usize,
    per_status: [usize; crate::status::COUNT],
    shown_per_status: [usize; crate::status::COUNT],
}

static DOC_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Doc {
    pub(crate) fn new(
        path: PathBuf,
        tm: TranslationMemory,
        strings: Option<crate::plugin::StringsBundle>,
        settings: DocSettings,
        depth: usize,
    ) -> Self {
        // The translations read with the file are « importées » (B-09)
        // unless the file itself says where they came from.
        let mut tm = tm;
        // A pivot JSON carries its history (A-18).
        let history = crate::history::History::from_carried(std::mem::take(&mut tm.history));
        let format = tm.meta.source_format.clone();
        crate::provenance::mark_imported(&mut tm.entries, &format);
        let mut d = Doc {
            path,
            tm,
            strings,
            settings,
            source_langs: Vec::new(),
            undo: UndoStack::new(depth),
            saved_hash: 0,
            dirty_memo: std::cell::Cell::new(None),
            autosaved_revision: 0,
            serial: DOC_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            text: None,
            text_fmt: None,
            interface_files: Vec::new(),
            scripts: None,
            display: None,
            history,
            ident: None,
            all: None,
            sources_rev: 0,
            enc: Default::default(),
            deduced: std::sync::OnceLock::new(),
        };
        d.mark_saved();
        d
    }

    /// Whether this is a text document (MCM, INI, JSON…, C-05).
    fn is_text(&self) -> bool {
        self.text.is_some()
    }

    /// The text format of a text document.
    fn text_format(&self) -> Option<crate::textfmt::TextFormat> {
        self.text_fmt.clone()
    }

    /// The current content is the saved one (after a load or a save).
    fn mark_saved(&mut self) {
        self.saved_hash = self.content_hash();
        self.dirty_memo.set(None);
    }

    /// Hash of every line's translation and status (what a save persists).
    fn content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for e in &self.tm.entries {
            e.trans.hash(&mut h);
            e.status_sst.hash(&mut h);
            e.status_eet.hash(&mut h);
            e.prov.hash(&mut h);
            e.review.hash(&mut h);
        }
        h.finish()
    }

    /// Whether the content differs from the saved one (R-A01-05); recomputed
    /// only when the undo revision moved.
    fn is_dirty(&self) -> bool {
        // The synthesis saves nothing: its files carry the changes (B-25).
        if self.all.is_some() {
            return false;
        }
        if let Some((rev, dirty)) = self.dirty_memo.get()
            && rev == self.undo.revision
        {
            return dirty;
        }
        let dirty = self.content_hash() != self.saved_hash;
        self.dirty_memo.set(Some((self.undo.revision, dirty)));
        dirty
    }

    fn file_name(&self) -> String {
        if let Some(d) = &self.display {
            return d.clone();
        }
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }
    fn dir(&self) -> String {
        self.path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    RecentFiles,
    /// Online translation (D-01 / D-02).
    Online,
    /// Configuration location (E-05); backups will join it (B-11).
    Advanced,
}

/// Undo / redo shortcuts (COMMAND = Ctrl on Windows / Linux, ⌘ on macOS).
/// Keyboard shortcuts of the main menu (F-01), defined once so the menu
/// display and the global handler stay in sync. COMMAND is Ctrl on Windows
/// and Linux, ⌘ on macOS.
struct MenuShortcuts {
    open: egui::KeyboardShortcut,
    save_as: egui::KeyboardShortcut,
    export: egui::KeyboardShortcut,
    close_doc: egui::KeyboardShortcut,
    settings: egui::KeyboardShortcut,
    manual: egui::KeyboardShortcut,
    quit: egui::KeyboardShortcut,
    replace: egui::KeyboardShortcut,
    quality: egui::KeyboardShortcut,
    personal: egui::KeyboardShortcut,
    dialogues: egui::KeyboardShortcut,
    /// Ctrl+G: the line online; Ctrl+Maj+G: the batch dialog (D-01).
    online_line: egui::KeyboardShortcut,
    online_batch: egui::KeyboardShortcut,
    /// Ctrl+Maj+R: Traduction = Relecture (A-17).
    review_accept: egui::KeyboardShortcut,
    /// Ctrl+Alt+F: Recherche avancée (A-19).
    search: egui::KeyboardShortcut,
    /// Ctrl+K: Concordance (A-20).
    concord: egui::KeyboardShortcut,
    /// Ctrl+Shift+J: the journal of the session (E-01).
    journal: egui::KeyboardShortcut,
}

impl MenuShortcuts {
    const fn new() -> MenuShortcuts {
        use egui::{Key, KeyboardShortcut, Modifiers};
        MenuShortcuts {
            open: KeyboardShortcut::new(Modifiers::COMMAND, Key::O),
            save_as: KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
            export: KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::E),
            close_doc: KeyboardShortcut::new(Modifiers::COMMAND, Key::W),
            settings: KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma),
            manual: KeyboardShortcut::new(Modifiers::NONE, Key::F1),
            quit: KeyboardShortcut::new(Modifiers::COMMAND, Key::Q),
            replace: KeyboardShortcut::new(Modifiers::COMMAND, Key::H),
            quality: KeyboardShortcut::new(Modifiers::NONE, Key::F12),
            personal: KeyboardShortcut::new(Modifiers::COMMAND, Key::M),
            dialogues: KeyboardShortcut::new(Modifiers::NONE, Key::F10),
            online_line: KeyboardShortcut::new(Modifiers::COMMAND, Key::G),
            online_batch: KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G),
            review_accept: KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::R),
            search: KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::F),
            concord: KeyboardShortcut::new(Modifiers::COMMAND, Key::K),
            journal: KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::J),
        }
    }
}

const MENU_SHORTCUTS: MenuShortcuts = MenuShortcuts::new();

/// The text of a shortcut for a menu entry or a tooltip, platform-aware
/// (« Ctrl+O », « ⌘O »), with the modifier names of the interface language
/// (« Ctrl+Maj+E » in French); egui names the comma key « Comma ».
pub(crate) fn shortcut_text(
    ctx: &egui::Context,
    sc: &egui::KeyboardShortcut,
    i18n: &crate::i18n::I18n,
) -> String {
    let raw = ctx.format_shortcut(sc);
    raw.split('+')
        .map(|part| match part {
            "Ctrl" => i18n.t("key-mod-ctrl"),
            "Shift" => i18n.t("key-mod-shift"),
            "Alt" => i18n.t("key-mod-alt"),
            "Comma" => ",".to_string(),
            "Enter" => i18n.t("key-enter"),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// A shortcut of the interface, as text (see [`shortcut_text`]).
pub(crate) fn key_text(
    ctx: &egui::Context,
    i18n: &crate::i18n::I18n,
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> String {
    shortcut_text(ctx, &egui::KeyboardShortcut::new(modifiers, key), i18n)
}

/// Buttons of a confirmation dialog (F-06): the primary one first and in
/// bold, the cancelling one last; Entrée fires the primary, Échap the
/// cancel. Returns `(primary, cancel)`.
pub(crate) fn confirm_buttons(ui: &mut egui::Ui, primary: &str, cancel: &str) -> (bool, bool) {
    let (enter, esc) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    let mut p = false;
    let mut c = false;
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new(primary).strong()).clicked() {
            p = true;
        }
        if ui.button(cancel).clicked() {
            c = true;
        }
    });
    (p || enter, c || esc)
}

/// A menu entry with its shortcut at the right; closes the menu when
/// clicked. A disabled entry shows `disabled_tip` on hover when not empty.
fn menu_item(
    ui: &mut egui::Ui,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    disabled_tip: &str,
) -> bool {
    let mut btn = egui::Button::new(label).wrap_mode(egui::TextWrapMode::Extend);
    if let Some(sc) = shortcut {
        btn = btn.shortcut_text(sc);
    }
    let resp = ui.add_enabled(enabled, btn);
    let resp = if !enabled && !disabled_tip.is_empty() {
        resp.on_disabled_hover_text(disabled_tip)
    } else {
        resp
    };
    let clicked = resp.clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

/// Ctrl+Alt+Z: « Revenir à la traduction précédente » (A-18), consumed
/// before Ctrl+Z (which it would match).
const SHORTCUT_HISTORY_PREV: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
    egui::Key::Z,
);
/// Ctrl+Alt+G: « Ajouter au glossaire » in the edit window (B-15),
/// consumed before Ctrl+G (En ligne).
const SHORTCUT_GLOSSARY_ADD: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
    egui::Key::G,
);
/// Extra top and bottom margin of the toolbar and of the filter bar, in
/// points, over egui's default panel.
const TOOLBAR_EXTRA: f32 = 4.0;

/// A vertical separator of the height of a widget, for a wrapping row:
/// egui's own takes all the height left in the panel, so one that wrapped
/// to the second row of the filter bar made that row overlap the first
/// (reported 25/10/2026).
/// Start a new row of a wrapping bar when a label and the widget that
/// follows it (`w` wide) no longer fit together: egui's combo boxes take
/// the room left on the row and never wrap by themselves, and a label
/// kept apart from its list reads badly.
fn keep_together(ui: &mut egui::Ui, label: &str, w: f32) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let lw = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    let need = lw + ui.spacing().item_spacing.x + w;
    // In a wrapping row `available_width` is the width of a whole row, and
    // `max_rect` grows with what overflowed: the right edge is the clip's.
    let right = ui.max_rect().right().min(ui.clip_rect().right() - 4.0);
    let at_start = ui.cursor().min.x <= ui.max_rect().min.x + 1.0;
    if !at_start && right - ui.cursor().min.x < need {
        ui.end_row();
    }
}

/// The width of a combo box of `width` (egui's default when `None`)
/// showing `text`: it grows with its text.
fn combo_w(ui: &egui::Ui, width: Option<f32>, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let tw = ui.fonts(|f| {
        f.layout_no_wrap(text.to_string(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    let s = ui.spacing();
    width.unwrap_or(s.combo_width).max(tw)
        + 2.0 * s.button_padding.x
        + s.icon_width
        + s.item_spacing.x
}

fn vsep(ui: &mut egui::Ui) {
    // No separator at the start of a row, nor one that would itself wrap
    // to start the next row.
    let right = ui.max_rect().right().min(ui.clip_rect().right() - 4.0);
    let x = ui.cursor().min.x;
    if x <= ui.max_rect().min.x + 1.0 || right - x < 6.0 + ui.spacing().item_spacing.x {
        return;
    }
    let h = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, h), egui::Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.y_range(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}
const SHORTCUT_UNDO: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const SHORTCUT_REDO: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
const SHORTCUT_REDO_ALT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    egui::Key::Z,
);

/// A cell edit in progress (A-01).
struct EditSession {
    doc: usize,
    /// States captured at focus gain: the edited line first, then the
    /// duplicates a unique-view propagation may touch.
    before: Vec<(usize, EntryState)>,
    /// The review of the line is edited (A-17), not its translation.
    review: bool,
}

/// One misspelling found by the spell checker: which entry it is in, the word,
/// and precomputed suggestions (computed once per scan, not per frame).
struct SpellIssue {
    entry: usize,
    word: String,
    suggestions: Vec<String>,
}

/// An action requested from a row of the spell-check window, applied after the
/// UI closure so `self` isn't mutated mid-render.
enum SpellAction {
    /// Replace the misspelled word of issue `idx` with this suggestion.
    Fix(usize, String),
    /// Ignore the word of issue `idx` for this session.
    Ignore(usize),
    /// Add the word of issue `idx` to the personal dictionary.
    Add(usize),
}

/// A dictionary-format conversion direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvKind {
    EetToXimdb,
    SstToXimdb,
    XimdbToEet,
    XimdbToSst,
}

impl ConvKind {
    /// Source-file extension.
    fn src_ext(self) -> &'static str {
        match self {
            ConvKind::EetToXimdb => "eet",
            ConvKind::SstToXimdb => "sst",
            ConvKind::XimdbToEet | ConvKind::XimdbToSst => "ximdb",
        }
    }
    /// Destination-file extension.
    fn dst_ext(self) -> &'static str {
        match self {
            ConvKind::EetToXimdb | ConvKind::SstToXimdb => "ximdb",
            ConvKind::XimdbToEet => "eet",
            ConvKind::XimdbToSst => "sst",
        }
    }
    /// Whether the destination is a XIMOD base (`.ximdb`).
    fn to_ximdb(self) -> bool {
        matches!(self, ConvKind::EetToXimdb | ConvKind::SstToXimdb)
    }
    /// The non-ximdb ("foreign") extension of the conversion — `"eet"` or
    /// `"sst"` — identifying the external tool it belongs to.
    fn foreign_ext(self) -> &'static str {
        if self.to_ximdb() {
            self.src_ext()
        } else {
            self.dst_ext()
        }
    }
    /// True when the conversion involves ESP-ESM Translator (`.eet`).
    fn is_eet(self) -> bool {
        self.foreign_ext() == "eet"
    }
}

/// A "to-ximdb" conversion (EET/SST → ximdb) waiting on the confirmation
/// dialog before it is written to `assets/bases/mods/<Jeu>` (conversions only
/// ever produce **mod** dictionaries: game bases are built from the installed
/// games or shipped ready-made).
#[derive(Debug, Clone)]
struct PendingConv {
    /// Conversion direction (always a `*_to_ximdb` variant here).
    kind: ConvKind,
    /// Source file(s): one entry for a unitary conversion, many for a batch.
    inputs: Vec<PathBuf>,
    /// True for a folder (batch) conversion, false for a single file.
    batch: bool,
    /// Target game chosen in the drop-down (prefixes the output file name).
    game: crate::games::Game,
}

/// A dictionary save awaiting the "Is this really a dictionary for <game>?"
/// confirmation. Cancelling drops it without writing anything.
enum PendingSave {
    /// Pre-built rows (from STRINGS / from a plugin pair).
    Rows {
        out: PathBuf,
        lang: String,
        rows: Vec<crate::basedb::BaseRow>,
    },
    /// Raw base files to compile (from a game folder / from mod files).
    Compile {
        out: PathBuf,
        lang: String,
        files: Vec<PathBuf>,
    },
}

/// A loaded dictionary shown in the read-only viewer window.
struct DictView {
    /// File name (window title).
    title: String,
    /// All (source, translation) pairs.
    all: Vec<(String, String)>,
    /// Current filter text and the row indices it matches (recomputed on change).
    filter: String,
    filtered: Vec<usize>,
    last_filter: Option<String>,
    /// Cached wrapped-row heights (parallel to `filtered`) and the column width
    /// they were computed for. Recomputed only when the filter or width changes,
    /// so plain scrolling never re-lays-out the (tens of thousands of) rows.
    heights: Vec<f32>,
    heights_w: f32,
}

/// Detect a game from a file stem by matching a known game id as a
/// case-insensitive substring (longest id first, so `SkyrimSE` wins over
/// `Skyrim`). Returns [`crate::games::Game::Unknown`] when nothing matches —
/// e.g. `BDD_Fallout3_EN-FR` → `Fallout3`.
fn detect_game(stem: &str) -> crate::games::Game {
    let low = stem.to_lowercase();
    let mut games: Vec<crate::games::Game> = crate::games::Game::ALL.to_vec();
    games.sort_by_key(|g| std::cmp::Reverse(g.id().len()));
    games
        .into_iter()
        .find(|g| low.contains(&g.id().to_lowercase()))
        .unwrap_or(crate::games::Game::Unknown)
}

/// ESP-ESM Translator names its per-game folders (`Bases Persos/<Jeu>/`) with
/// spaces and full names; XIMOD Translator and xTranslator use the compact ids.
/// This table is the official correspondence (decision of 30/09/2026).
const EET_GAME_FOLDERS: &[(&str, crate::games::Game)] = &[
    ("Fallout 3", crate::games::Game::Fallout3),
    ("Fallout 4", crate::games::Game::Fallout4),
    ("Fallout 76", crate::games::Game::Fallout76),
    ("Fallout New Vegas", crate::games::Game::FalloutNV),
    ("Morrowind", crate::games::Game::Morrowind),
    ("Oblivion", crate::games::Game::Oblivion),
    ("Skyrim", crate::games::Game::Skyrim),
    ("Skyrim SE", crate::games::Game::SkyrimSE),
    ("Starfield", crate::games::Game::Starfield),
];

/// ESP-ESM Translator folder name for a game (`Bases Persos/<this>`), when the
/// game has one.
fn eet_folder_for(game: crate::games::Game) -> Option<&'static str> {
    EET_GAME_FOLDERS
        .iter()
        .find(|(_, g)| *g == game)
        .map(|(n, _)| *n)
}

/// Game from an ESP-ESM Translator `<Jeu>` folder name (exact, case-insensitive).
fn game_from_eet_folder(name: &str) -> crate::games::Game {
    EET_GAME_FOLDERS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        .map(|(_, g)| *g)
        .unwrap_or(crate::games::Game::Unknown)
}

/// Game from a XIMOD / xTranslator `<Jeu>` folder name (the game id, exact,
/// case-insensitive) — the two tools use identical folder names.
fn game_from_id_folder(name: &str) -> crate::games::Game {
    crate::games::Game::ALL
        .iter()
        .copied()
        .find(|g| g.id().eq_ignore_ascii_case(name.trim()))
        .unwrap_or(crate::games::Game::Unknown)
}

/// Game inferred from a dictionary's **parent folder name**, according to the
/// tool whose layout the file follows: ESP-ESM Translator (`Bases Persos/<Jeu>/`,
/// full names) or XIMOD / xTranslator (`mods/<Jeu>/`, `UserDictionaries/<Jeu>/`,
/// game ids). Returns [`crate::games::Game::Unknown`] when the folder is not a
/// game folder.
fn game_from_folder(input: &std::path::Path, eet_layout: bool) -> crate::games::Game {
    let Some(name) = input
        .parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
    else {
        return crate::games::Game::Unknown;
    };
    if eet_layout {
        game_from_eet_folder(name)
    } else {
        game_from_id_folder(name)
    }
}

/// Recursively collect files under `dir` whose extension matches `ext`
/// (case-insensitive), sorted. Used by batch conversion so a whole tree of
/// per-game sub-folders is processed in one pass.
fn collect_files_recursive(dir: &std::path::Path, ext: &str) -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, ext: &str, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, ext, out);
            } else if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(ext))
                    .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, ext, &mut out);
    out.sort();
    out
}

/// Whether a name token looks like the hash ESP-ESM Translator appends to its
/// mod-dictionary file names: a run of at least 8 hexadecimal characters. Such
/// tokens are stripped from the derived ximdb name (they encode the *source*
/// plugin, not our base, so they would be misleading).
fn is_hash_token(t: &str) -> bool {
    t.len() >= 8 && t.chars().all(|c| c.is_ascii_hexdigit())
}

/// Starting folder for a "Parcourir…" picker: the field's current path (or its
/// nearest existing ancestor) when set; otherwise "This PC" on Windows (so all
/// drives show) or the filesystem root elsewhere — never the Documents default.
fn browse_folder_start(field: &str) -> PathBuf {
    let t = field.trim();
    if !t.is_empty() {
        let mut cur = PathBuf::from(t);
        loop {
            if cur.exists() {
                return cur;
            }
            match cur.parent() {
                Some(p) => cur = p.to_path_buf(),
                None => break,
            }
        }
    }
    #[cfg(windows)]
    {
        // The "This PC" shell folder (CLSID), parsed by the Windows file dialog.
        PathBuf::from("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}

/// Starting folder for a "Parcourir…" picker on a **file**-path field: the file's
/// containing folder (or the nearest existing ancestor) when set; otherwise
/// "This PC" on Windows / the filesystem root elsewhere.
fn browse_start_for_file(field: &str) -> PathBuf {
    let t = field.trim();
    if !t.is_empty() {
        let p = PathBuf::from(t);
        let mut cur = if p.is_file() {
            p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
        } else {
            p
        };
        loop {
            if cur.is_dir() {
                return cur;
            }
            match cur.parent() {
                Some(par) => cur = par.to_path_buf(),
                None => break,
            }
        }
    }
    #[cfg(windows)]
    {
        PathBuf::from("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}

/// `(full, stripped)` comparison keys for a plugin file stem: the lowercased
/// tokens (split on `_ - space`) joined, and the same with a trailing 2–3 letter
/// language code dropped (FR, de, eng, fra…). A translated mod usually appends
/// such a code, which must be ignored when matching the two plugins.
fn plugin_match_keys(stem: &str) -> (String, String) {
    let toks: Vec<String> = stem
        .to_lowercase()
        .split(['_', '-', ' '])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let full = toks.join(" ");
    let stripped = if toks.len() >= 2 {
        let last = &toks[toks.len() - 1];
        if (last.len() == 2 || last.len() == 3) && crate::lang::iso3_from_token(last).is_some() {
            toks[..toks.len() - 1].join(" ")
        } else {
            full.clone()
        }
    } else {
        full.clone()
    };
    (full, stripped)
}

/// Whether two plugin files share the same base name, ignoring a trailing
/// language code on either side (so `MonMod.esp` ↔ `MonMod_FR.esp` match, and
/// `Oblivion.esm` ↔ `Oblivion.esm` match, but `A.esp` ↔ `B_FR.esp` do not).
fn plugin_names_match(a: &std::path::Path, b: &std::path::Path) -> bool {
    let sa = a.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let sb = b.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let (fa, ka) = plugin_match_keys(sa);
    let (fb, kb) = plugin_match_keys(sb);
    fa == fb || fa == kb || ka == fb || ka == kb
}

/// The pieces decoded from a dictionary file stem (EET/SST naming).
struct DictNameParts {
    /// Game recognised in the name, else [`crate::games::Game::Unknown`].
    game: crate::games::Game,
    /// Language pair (ISO 639-3, upper-cased) when found, e.g. `("ENG","FRA")`.
    langs: Option<(String, String)>,
    /// The name with the game token, the language pair and known markers
    /// (`BDD`, `English`) removed — used as the mod name for a mod dictionary.
    core: String,
}

/// Decompose an EET/SST dictionary file stem into its game, language pair and
/// core (mod) name, following the modding-tool conventions. Examples:
/// `BDD_Fallout3_EN-FR` → game `Fallout3`, langs `ENG`/`FRA`;
/// `MonMod_EN-FR` → core `MonMod`, langs `ENG`/`FRA`. Language codes may be
/// ISO 639-1 (`EN`) or 639-3 (`ENG`); the pair is two adjacent language tokens.
fn parse_dict_name(stem: &str) -> DictNameParts {
    let game = detect_game(stem);
    let raw: Vec<&str> = stem
        .split(['_', '-', ' ', '.'])
        .filter(|t| !t.is_empty())
        .collect();
    // First adjacent pair of language tokens → (src, dst).
    let mut langs = None;
    let mut lang_idx: Option<(usize, usize)> = None;
    for i in 0..raw.len().saturating_sub(1) {
        if let (Some(a), Some(b)) = (
            crate::i18n::iso639_to_3(raw[i]),
            crate::i18n::iso639_to_3(raw[i + 1]),
        ) {
            langs = Some((a.to_uppercase(), b.to_uppercase()));
            lang_idx = Some((i, i + 1));
            break;
        }
    }
    // Core name: drop the language tokens, a BDD/English marker, and (when
    // present) the game token.
    let game_id_low = game.id().to_lowercase();
    let core = raw
        .iter()
        .enumerate()
        .filter(|(i, t)| {
            if let Some((a, b)) = lang_idx
                && (*i == a || *i == b)
            {
                return false;
            }
            if is_hash_token(t) {
                return false;
            }
            let tl = t.to_lowercase();
            if tl == "bdd" || tl == "english" {
                return false;
            }
            !(game != crate::games::Game::Unknown && tl == game_id_low)
        })
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join("_");
    DictNameParts { game, langs, core }
}

/// Split a STRINGS file name `<base>_<lang>.<KIND>` into `(base, lang, kind)`
/// (case-insensitive on the extension). Returns `None` when the name is not a
/// language-suffixed STRINGS file.
fn split_strings_name(name: &str) -> Option<(String, String, crate::tm::strings::StringsKind)> {
    let dot = name.rfind('.')?;
    let kind = crate::tm::strings::kind_from_extension(&name[dot + 1..])?;
    let base = &name[..dot];
    let us = base.rfind('_')?;
    let (b, lang) = (&base[..us], &base[us + 1..]);
    if b.is_empty() || lang.is_empty() {
        return None;
    }
    Some((b.to_string(), lang.to_string(), kind))
}

/// Messages from the background game-detection thread.
enum DetectMsg {
    /// A source is about to run: `(done, total, source-id)`.
    Step(usize, usize, &'static str),
    /// Detection finished with these results.
    Done(Vec<crate::games::detect::DetectedGame>),
}

/// An action from the "Dictionnaires" menu.
#[derive(Debug, Clone, Copy)]
enum DictAction {
    ConvUnit(ConvKind),
    ConvBatch(ConvKind),
    FromGame,
    FromMod,
    FromStrings,
    FromPlugins,
    /// Build a game dictionary from a pair of UEExtractor locres CSVs.
    FromLocresCsv,
    /// Export a game dictionary (ximdb) as translated STRINGS files.
    ExportGameStrings,
    /// Auto-detect installed games across stores and OSes.
    DetectGames,
    /// Register a game folder chosen manually.
    AddGameManual,
    /// Open a dictionary (ximdb) in the read-only viewer.
    ViewDict,
    /// Write the active document's own dictionary now (B-03).
    SaveModDict,
    /// The dictionary tree of the active document (B-02).
    ManageDicts,
    /// A text file of « source<TAB>traduction » pairs → a text-only
    /// dictionary (B-05).
    FromTextPairs,
    /// The editor on the personal dictionary / on a chosen file (B-04).
    EditPersonal,
    EditDict,
    /// Mise à jour du mod (B-07).
    ModUpdate,
    /// Comparer deux dictionnaires (D-07).
    Compare,
}

/// A word index being built (B-01).
enum FuzzyMsg {
    Progress(usize, usize),
    Done(Option<Arc<crate::fuzzy::FuzzyIndex>>),
}

struct FuzzyJob {
    rx: std::sync::mpsc::Receiver<FuzzyMsg>,
    progress: f32,
}

/// The propagation dialog (A-12): its groups and where it is drawn.
struct PropagateDialog {
    groups: Vec<crate::propagate::Group>,
    /// Opened from the edit window: drawn in its viewport.
    in_edit: bool,
}

/// What the Vocabulaire tab shows (B-01).
#[derive(Default)]
struct VocabState {
    /// Known fragments of the line (B-14).
    fragments: Vec<crate::instant::Found>,
    /// The glossary's terms in the line, with their group (B-15).
    glossary: Vec<(String, String, String)>,
    /// The pattern that translates the line (B-16).
    pattern: Option<(usize, String, String)>,
    /// (document serial, revision, line) the suggestions are for.
    key: Option<(u64, u64, usize)>,
    changed_at: Option<f64>,
    /// `None` while an index builds.
    suggestions: Option<Vec<(String, crate::fuzzy::Suggestion)>>,
    /// The candidate translations of the source in the stack (B-08).
    candidates: Vec<crate::ambiguity::Candidate>,
    /// The entry compared with the line (A-08): `None` = the first
    /// suggestion; the comparison cached for it; ⇄ Inverser.
    diff_pick: Option<DiffPick>,
    diff: Option<(DiffPick, crate::diff::Diff)>,
    diff_swapped: bool,
}

/// What the differences frame compares with the line (A-08): a suggestion
/// (its source) or a candidate (its translation, B-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffPick {
    Suggestion(usize),
    Candidate(usize),
}

/// A pending addition to the personal dictionary (B-04, R-B04-02).
struct PersonalPrompt {
    pairs: Vec<crate::basedb::BaseRow>,
    dict: crate::dict_edit::DictDoc,
    /// Pairs already known with another translation.
    conflicts: usize,
    many_confirmed: bool,
    remember: bool,
}

/// What is pulled out of an archive when opening a plugin from the navigator:
/// the plugin file name, its bytes, and the archive's matching STRINGS files as
/// `(name, bytes)` pairs (co-extracted so the plugin's text resolves).
type ExtractedPlugin = (String, Vec<u8>, Vec<(String, Vec<u8>)>);

/// Keyboard-focusable controls of the Settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    TabGeneral,
    TabRecent,
    TabOnline,
    TabAdvanced,
    Flag,
    Language,
    Theme,
    FontSize,
    MaxRecent,
    WinW,
    WinH,
    Clear,
    /// The configuration-location radios (Avancé).
    ConfigMode,
    Save,
    Cancel,
}

/// Tab-order of the General tab.
const ORDER_GENERAL: &[Focus] = &[
    Focus::TabGeneral,
    Focus::TabRecent,
    Focus::TabOnline,
    Focus::TabAdvanced,
    Focus::Flag,
    Focus::Language,
    Focus::Theme,
    Focus::FontSize,
    Focus::MaxRecent,
    Focus::WinW,
    Focus::WinH,
    Focus::Save,
    Focus::Cancel,
];

pub struct XimodApp {
    pub(crate) config: AppConfig,
    pub(crate) i18n: I18n,
    countries: CountriesData,
    country_languages: CountryLanguagesData,
    /// The names of the countries per language (E-11).
    country_names: CountryNamesData,
    /// Base pays / langues (E-11).
    props_win: app_props::PropsWindow,
    screen: ScreenInfo,

    pub(crate) docs: Vec<Doc>,
    pub(crate) active: usize,
    /// The toolbar lists while no document is open: empty at launch and after
    /// the last tab closes, filled by the user before the next file.
    idle: IdleSettings,
    /// The « Cible » list: the official languages of the country picked in
    /// the flags window (ISO 639-3), empty at launch.
    target_langs: Vec<String>,
    /// What the flags window is open for: the interface locale (Settings) or
    /// the « Cible » list (toolbar button).
    flags_purpose: FlagsPurpose,
    /// Fichier ▸ Exporter… asked while the « Sortie » list holds a mode: the
    /// confirmation « est-ce la bonne sortie ? » is pending.
    pending_output_confirm: bool,

    // Combinable filters (text / status / record / char count).
    filter: String,
    filter_statuses: [bool; crate::status::COUNT],
    filter_record: String,
    filter_field: String,
    filter_min_chars: u32,
    /// Filters set from the row menu (M-01): record / source.
    filter_formid: Option<u32>,
    filter_source_exact: Option<String>,
    /// Search target and regex option of the filter field (A-09), kept
    /// for the session only.
    filter_in: crate::filter::SearchIn,
    filter_regex: bool,
    /// The text as applied (debounced: 250 ms after the last keystroke or
    /// on Entrée), the time of the last keystroke, and the error of an
    /// invalid expression (the previous filter stays in force).
    filter_applied: String,
    filter_applied_in: crate::filter::SearchIn,
    filter_applied_regex: bool,
    filter_typed_at: Option<f64>,
    filter_error: Option<String>,
    /// Ctrl+F: focus the field and select its text.
    filter_focus_request: bool,
    /// The filtered view's key `(document serial, revision, pinned rows,
    /// criteria)`: recomputed only when one of them moved.
    view_key: Option<(u64, u64, (usize, usize), crate::filter::Criteria)>,
    /// The views of the other tabs, kept so going back to a tab is
    /// immediate (B-25: « Tous les mods » and its files, R-B25-05).
    view_store: std::collections::HashMap<u64, StoredView>,
    /// The row heights of the other tabs (same reason).
    heights_store: std::collections::HashMap<u64, grid::HeightCache>,
    /// The serial whose heights are in `grid_heights`.
    heights_serial: u64,
    filter_max_chars: u32,
    filter_unique_only: bool,
    /// Filtered view for the active doc, refreshed each frame.
    visible: Vec<usize>,
    view_total: usize,
    view_unique: usize,
    view_unique_remaining: usize,
    /// Lines per status in the document and in the view (A-15).
    view_per_status: [usize; crate::status::COUNT],
    view_shown_per_status: [usize; crate::status::COUNT],

    status: String,

    applied_font_size: f32,

    // Settings window (temp values).
    show_settings: bool,
    settings_needs_center: bool,
    /// Frames left during which the Settings window is pulled to the front
    /// (first launch, F-02): the main window must not cover it.
    settings_focus_frames: u8,
    /// One-shot request to fit the Settings window to its content (on open and
    /// when the active tab changes).
    settings_fit: bool,
    /// Last tab drawn, to detect a tab change and re-fit.
    settings_last_tab: SettingsTab,
    settings_tab: SettingsTab,
    settings_focus: Focus,
    /// True for one frame after the focus moved via keyboard, so we drive egui's
    /// native focus only then — re-requesting every frame would close a
    /// just-opened combo popup.
    settings_focus_dirty: bool,
    temp_locale: String,
    temp_country: String,
    temp_theme: Theme,
    temp_font_size: f32,
    temp_max_recent_files: usize,
    temp_window_width: f32,
    temp_window_height: f32,
    temp_game_bases_dir: String,
    temp_mod_bases_dir: String,
    temp_eet_dir: String,
    temp_xtranslator_dir: String,
    /// Configuration location chosen in Paramètres ▸ Avancé (E-05); applied by
    /// a migration on save, effective at the next start.
    temp_config_mode: crate::config::ConfigMode,

    // Flags window.
    show_flags: bool,
    flags_needs_center: bool,
    /// Outils ▸ Données ▸ Champs traduisibles par jeu… (A-06).
    fields_editor: Option<crate::ui::fields_editor::FieldsEditor>,
    fields_needs_center: bool,
    /// « Traduire un FOMOD… » (M-02) and « Empaqueter une traduction… » (M-03).
    /// « Traduire un FOMOD » (M-02 on the shared crate, F-03) and its job.
    pub(crate) fomod_trans: crate::ui::fomod_window::FomodTranslationState,
    pub(crate) fomod_job: Option<crate::ui::fomod_window::FomodJob>,
    pub(crate) fomod_needs_center: bool,
    /// The edit window (A-02), one at a time, on the active document.
    edit_win: Option<crate::ui::edit_window::EditWindow>,
    /// Rechercher et remplacer (A-11).
    replace: crate::ui::replace_window::ReplaceWindow,
    /// Vérifications qualité (D-05): the window, the indicator per line
    /// (tooltip), the set the filter uses, and the document they belong to.
    quality: crate::ui::quality_window::QualityWindow,
    /// The dictionary tree window (B-02).
    dicts: crate::ui::dicts_window::DictsWindow,
    /// The dictionary editor (B-04) and the pending « Ajouter à mon
    /// dictionnaire personnel » with its session choice on conflicts.
    dict_editor: crate::ui::dict_editor::DictEditor,
    /// Appliquer les dictionnaires (B-06).
    apply_dlg: crate::ui::apply_dialog::ApplyDialog,
    /// Mise à jour du mod (B-07).
    update: crate::ui::update_window::UpdateWindow,
    /// Word indexes (B-01): one per dictionary file, built in the background.
    fuzzy_cache: std::collections::HashMap<PathBuf, Arc<crate::fuzzy::FuzzyIndex>>,
    fuzzy_jobs: std::collections::HashMap<PathBuf, FuzzyJob>,
    /// Fragment indexes (B-14): one per dictionary file.
    frag_cache: std::collections::HashMap<PathBuf, Arc<crate::instant::FragmentIndex>>,
    /// « Ne plus demander » of the whole-source replacement (session).
    instant_no_confirm: bool,
    /// Propagation aux lignes de même source (A-12), when open.
    propagate_dlg: Option<PropagateDialog>,
    /// The dialogues (C-01, C-02, C-13).
    dialogs: crate::ui::dialog_window::DialogWindow,
    /// Grid filter on a speaker key (C-02).
    filter_speaker: Option<String>,
    /// « Voir les lignes » of a rule (B-18): its id and its lines.
    filter_lock_rule: Option<(String, crate::filter::AnomalySet)>,
    /// The Commentaire column (C-03): the per-line cache of the grid and
    /// the whole-document texts of « Dans : Commentaire ».
    comment_cache: crate::comments::Cache,
    comment_texts: Option<(crate::comments::CacheKey, crate::filter::CommentTexts)>,
    /// « Quel format ? » for a text file (C-05), while asked.
    /// Importer avec un format (D-08).
    import_dlg: Option<import_dialog::ImportDialog>,
    /// Analyser un dossier, Comparer deux dictionnaires (D-07).
    scan_win: scan_window::ScanWin,
    compare_win: compare_window::CompareWin,
    /// Fichier ▸ Charger dans la relecture… (A-17), while it is open.
    review_dlg: Option<app_review::ReviewDialog>,
    /// Log the frames slower than this many milliseconds
    /// (`XIMOD_FRAME_LOG=<ms>`, 100 for an empty value); 0 logs nothing.
    frame_log: u128,
    /// The row heights of the grid, kept between frames.
    grid_heights: grid::HeightCache,
    /// The clipboard, read off the interface thread.
    clipboard: ClipboardWatch,
    /// The window title last sent (sent again only when it changes).
    last_title: Option<String>,
    /// Frames drawn since `frame_since` (the frame log).
    frame_count: u32,
    repaint_askers: Vec<String>,
    frame_asked: u32,
    frame_events: u32,
    frame_since: std::time::Instant,
    /// The record and field signatures of the active document, for the
    /// filter bar's lists (document serial, GRUP list, Champ list).
    filter_sigs: Option<(u64, Vec<String>, Vec<String>)>,
    /// Those of the other tabs (B-25).
    filter_sigs_store: std::collections::HashMap<u64, (u64, Vec<String>, Vec<String>)>,
    /// Outils ▸ Historique… (A-18) and its confirmation.
    history_win: app_history::HistoryWindow,
    /// Outils ▸ Recherche avancée… (A-19) and the search applied to the
    /// grid (the chip of the filter bar).
    search_win: app_search::SearchWindow,
    /// Outils ▸ Scripts ▸ Scripts du mod… (C-06).
    scripts_win: app_scripts::ScriptsWindow,
    /// Matrices de remplacement (B-17): the editor and the generation of
    /// the files (bumped at each save).
    matrices_win: app_matrices::MatricesWindow,
    matrices_gen: u64,
    /// Règles d'exclusion (B-18): the window, the generalization dialog
    /// and the offer of the status bar.
    rules_win: app_rules::RulesWindow,
    /// Assistants (B-15): the window, the dialog Appliquer les assistants,
    /// Ajouter au glossaire (and the group chosen last), the generation of
    /// the glossary files and the assistant of the active pair and game.
    assist_win: app_assist::AssistWindow,
    assist_dlg: app_assist::AssistDialog,
    add_term: Option<app_assist::AddTerm>,
    add_term_last: Option<String>,
    /// The source selection when the context menu of the edit window's
    /// Source area opened: (line, from, to).
    edit_ctx_sel: Option<(usize, usize, usize)>,
    assist_gen: u64,
    assist_cache: Option<((String, String, u64), Arc<crate::assist::Assistant>)>,
    /// Dérivations (B-21): the spells the tomes teach, per document
    /// serial; the undo pushes seen by the following of the parents.
    derive_teaches: std::collections::HashMap<u64, Arc<std::collections::HashMap<u32, u32>>>,
    derive_seen: std::collections::HashMap<u64, u64>,
    generalize: Option<app_rules::GeneralizeDlg>,
    generalize_offer: Option<(u64, crate::exclusions::Proposal)>,
    /// The note of the migration of papyrus_rules.json, shown once.
    rules_migrated: Option<String>,
    /// The script blocks of the active plugin, for the Script tab (C-07).
    sctx_cache: std::cell::RefCell<Option<(PathBuf, std::sync::Arc<app_scripts::SctxMap>)>>,
    concord_win: app_concord::ConcordWindow,
    /// The journal of the session (E-01) and its window.
    journal: crate::journal::Journal,
    journal_win: app_journal::JournalWindow,
    /// « Voir les lignes » of a journal entry: the chip (R-E01-04).
    filter_journal: Option<app_journal::JournalChip>,
    /// The last alert recorded (an alert stays shown for several frames).
    alert_logged: Option<String>,
    /// The encoding refusal of the open export dialog is already recorded.
    export_enc_logged: bool,
    /// When the opening in progress started (duration in the journal).
    open_started: Option<std::time::Instant>,
    /// Fichier ▸ Enregistrer une partie… (B-10) and its choices per file.
    savepart_dlg: Option<app_savepart::SavePartDialog>,
    /// Fichier ▸ Encodage du fichier… (D-09).
    encoding_dlg: Option<app_encoding::EncodingDialog>,
    /// What localizing the active plugin will write (C-09).
    localize_cache: app_localize::LocalizeCache,
    /// Reconnaissance du dictionnaire du mod (B-12).
    crc: app_crc::CrcState,
    savepart_memory: app_savepart::SavePartMemory,
    /// The folder of the last file saved (B-10: the part proposed there).
    last_save_dir: Option<PathBuf>,
    /// The folder of the last export written (E-06: its backups/).
    last_export_dir: Option<PathBuf>,
    search_filter: Option<app_search::AppliedSearch>,
    history_confirm: Option<app_history::HistoryConfirm>,
    /// Online translation (D-01 / D-02).
    online: app_online::OnlineState,
    /// The text formats editor (C-05), in its own window.
    formats_editor: Option<crate::ui::formats_editor::FormatsEditor>,
    formats_needs_center: bool,
    /// Outils ▸ Traduire l'interface… (E-10), in its own window.
    tr: Option<Box<crate::ui::translation_editor::TrEditor>>,
    tr_needs_center: bool,
    /// The font registered as the preview family (E-10).
    tr_preview_font: Option<String>,
    /// True while the edit window's body runs (a menu action from there
    /// opens its dialogs in that viewport).
    edit_win_active_frame: bool,
    /// The text area of the edit window that had the focus last (A-20).
    edit_last_text: Option<egui::Id>,
    /// The Vocabulaire tab of the edit window.
    vocab: VocabState,
    /// The Communes tab (C-04) and the source indexes of the open files.
    common: app_common::CommonState,
    personal_prompt: Option<PersonalPrompt>,
    personal_session_choice: Option<bool>,
    anomalies: std::collections::HashMap<usize, String>,
    anomaly_set: crate::filter::AnomalySet,
    anomaly_serial: Option<u64>,
    filter_anomalies: bool,
    /// « Ambiguës » of the filter bar (B-08).
    filter_ambiguous: bool,
    /// « Provenance : » of the filter bar (B-09).
    filter_prov: crate::provenance::ProvFilter,
    /// ≠ : only the lines whose review differs from the translation (A-17).
    filter_review_diff: bool,
    /// The filter row under the headers (A-10): the texts being typed,
    /// the applied ones, and the error of each applied field.
    filter_columns: crate::filter::ColumnFilters,
    filter_columns_applied: crate::filter::ColumnFilters,
    filter_column_errors: [Option<String>; crate::filter::N_COLS],
    /// The short provenance labels of the document, for the Provenance
    /// column filter (document serial, revision, texts).
    prov_texts: Option<(u64, u64, crate::filter::CommentTexts)>,
    /// « Effacer » of the recent list waits for a second click (F-06).
    clear_recent_armed: bool,
    /// Closing a document with unsaved changes asks first (F-06): its index.
    close_prompt: Option<usize>,
    /// Quitting with unsaved documents asks first.
    exit_prompt: bool,
    /// The exit prompt was answered: let the close through.
    exit_confirmed: bool,
    /// Transient notifications (F-05) and the frame time they are stamped with.
    toasts: crate::ui::toasts::Toasts,
    frame_time: f64,
    /// Rows a Page Up / Page Down jumps over: the grid's height in
    /// single-line rows, measured when it is drawn (F-04).
    grid_page_rows: usize,
    pack_win: Option<crate::ui::package_window::PackageWindow>,
    pack_needs_center: bool,
    flag_filter: String,
    /// The filter field takes the keyboard focus when the window opens.
    flag_filter_focus: bool,
    flag_focus: usize,

    show_about: bool,
    about_needs_center: bool,

    // Status-colors window + its color picker.
    show_status_colors: bool,
    status_colors_needs_center: bool,
    /// One-shot flag: fit the status-colors window height to its content once.
    status_colors_fitted: bool,
    color_picker: super::color_picker::ColorPicker,

    // Spell checker (Hunspell-compatible, CDC §8.3).
    show_spell: bool,
    spell_needs_center: bool,
    /// The loaded checker for `spell_lang` (None when no dictionary is installed).
    spell: Option<crate::spell::SpellChecker>,
    /// The target language the current checker was loaded for (to detect changes).
    spell_lang: String,
    /// Misspellings found in the active document's translations.
    spell_issues: Vec<SpellIssue>,
    /// Human-readable outcome of the last scan.
    spell_status: String,

    // Archive navigator (BSA/BA2, CDC §3).
    show_archive: bool,
    archive_needs_center: bool,
    archive: Option<crate::archive::Archive>,
    archive_filter: String,
    archive_translatable_only: bool,
    archive_selected: Option<usize>,
    archive_status: String,

    // "Dictionary from a plugin pair" dialog: two path fields (manual entry +
    // Browse). The paths persist for the whole session (never auto-cleared).
    show_plugin_pair: bool,
    pp_src_path: String,
    pp_tgt_path: String,
    /// Destination for the plugin-pair dictionary: game bases (`jeux`) vs mods.
    pp_to_game: bool,

    // "Dictionary from a pair of UEExtractor locres CSVs" dialog (source-language
    // CSV + target-language CSV). Same persist-until-close behavior as above.
    show_locres_csv: bool,
    lc_src_path: String,
    lc_tgt_path: String,
    /// Destination for the locres-CSV dictionary: game bases (`jeux`) vs mods.
    lc_to_game: bool,

    /// Auto-translation indexes of the game tier, one per (game, target)
    /// pair (E-12, CDC §6).
    /// One index per dictionary file (B-02).
    file_dbs: crate::basedb::FileDbCache,

    // Book preview (CDC §8 — aperçus).
    show_book: bool,
    book_needs_center: bool,
    book_idx: Option<usize>,
    book_doc: usize,
    /// Cached parsed blocks, rebuilt when the entry or its translation changes.
    book_cache: Option<(usize, usize, String)>, // (doc, idx, trans snapshot)
    book_title: String,
    book_src_blocks: Vec<crate::book::Block>,
    book_trans_blocks: Vec<crate::book::Block>,
    /// User-supplied game fonts mapped to Bethesda faces (never shipped).
    book_fonts: crate::fonts::BookFonts,
    /// Whether the preview renders with the game fonts (when any are installed).
    book_use_game_fonts: bool,

    /// A resize requested from the Settings window, applied to the MAIN window on
    /// the next main-loop frame.
    pending_main_resize: Option<egui::Vec2>,

    // Explorers, opened from the « Outils » menu — each in its OWN independent
    // window, and all three can be open at once. Indexed by `Tool::idx`.
    tool_open: [bool; 3],
    /// True right after opening a tool window, so it is placed at its saved (or
    /// cascaded) geometry on the first frame only. Indexed by `Tool::idx`.
    tool_needs_center: [bool; 3],
    esm_tool: esm_explorer::app::EsmExplorerApp,
    bsa_tool: bsa_explorer::app::BsaExplorerApp,
    swf_tool: swf_explorer::app::SwfTextApp,

    /// A pending "vers ximdb" conversion awaiting the game / jeu-mod dialog.
    /// `Some` while the dialog is open (see [`XimodApp::dict_conv_window`]).
    dict_conv: Option<PendingConv>,
    /// A dictionary save awaiting the game-confirmation dialog.
    pending_save: Option<PendingSave>,
    /// Whether the read-only dictionary viewer window is open. The window can be
    /// open with no file loaded (empty state).
    show_dict_view: bool,
    /// The dictionary currently loaded in the viewer (None = nothing loaded).
    dict_view: Option<DictView>,
    /// True right after opening the viewer, so its viewport is created at the
    /// saved size/position on the first frame only.
    dict_view_needs_center: bool,
    /// A blocking alert message to show as a modal box (None = no alert).
    alert: Option<String>,
    /// Files to open at the first frame (command line).
    startup_files: Vec<PathBuf>,
    /// The cell being typed in (A-01): document, and the states of the line
    /// and of its propagated duplicates when the cell gained focus. Closed
    /// into one `Edit` operation at focus loss.
    edit_session: Option<EditSession>,
    /// Line to bring into view in the grid on the next frame (after undo / redo).
    grid_scroll_to: Option<usize>,
    /// Selected lines and current line of the grid (A-03).
    selection: grid::Selection,
    /// The translation cell to focus at the next frame (« valider puis
    /// suivante » while typing).
    grid_focus_request: Option<usize>,
    /// Lines kept in the view although the filter now excludes them
    /// (R-A03-05): a status set under the user's hand does not hide the
    /// line until the next filter change or selection.
    pinned_rows: std::collections::BTreeSet<usize>,
    last_criteria: Option<crate::filter::Criteria>,
    /// Undo depth being edited in Paramètres ▸ Avancé.
    temp_undo_depth: usize,
    /// Paramètres ▸ Avancé ▸ Historique (A-18).
    temp_history_enabled: bool,
    temp_history_max: usize,
    /// Autosave scheduler and writers (B-11).
    autosave: crate::backup::Autosave,
    /// Rescue files found at start, offered in the resume dialog.
    pending_resume: Option<Vec<(crate::backup::Manifest, PathBuf)>>,
    /// `[Backup]` values being edited in Paramètres ▸ Avancé.
    temp_autosave_enabled: bool,
    temp_toasts: bool,
    temp_autosave_minutes: u32,
    temp_autosave_dir: String,
    temp_backup_copies: usize,
    /// Paramètres ▸ Avancé ▸ Sorties (E-06).
    temp_keep_original: bool,
    /// [Dictionaries] CheckCrc (B-12).
    temp_check_crc: bool,
    temp_original_copies: usize,
    temp_mod_dict_auto: bool,
    temp_personal_autofill: bool,
    temp_personal_keep_field: bool,
    temp_old_data_months: u32,
    temp_fuzzy_threshold: u8,
    temp_normalize: bool,
    temp_propose_generalization: bool,
    /// Paramètres ▸ En ligne (D-01 / D-02).
    temp_online: crate::online::OnlineConfig,
    temp_propose_propagation: bool,
    /// Paramètres ▸ Général: propagate to the open files (B-25).
    temp_propagate_files: bool,
    /// A pending « recharger en tant que <jeu> » awaiting confirmation (E-12):
    /// the document index and the new game id.
    pending_reload: Option<(usize, String)>,
    /// The export dialog (S-04): which optional statuses to include, reset at
    /// each opening (R-S04-03). `Some` while the dialog is shown.
    pending_export: Option<(crate::export::ExportPolicy, bool)>,

    /// Results of the last "detect installed games" run, awaiting confirmation
    /// before being written to `Config.ini` (see [`XimodApp::dict_games_window`]).
    game_detect: Option<Vec<crate::games::detect::DetectedGame>>,
    /// Receiver for the background detection thread (Some while it runs).
    game_detect_rx: Option<std::sync::mpsc::Receiver<DetectMsg>>,
    /// Current detection progress `(done, total, localized label)`.
    detect_progress: Option<(usize, usize, String)>,
}

/// Which embedded explorer is currently on screen (see [`XimodApp::active_tool`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Esm,
    Bsa,
    Swf,
}

impl Tool {
    /// All tools, in menu order.
    const ALL: [Tool; 3] = [Tool::Esm, Tool::Bsa, Tool::Swf];

    /// Stable index into the per-tool open/geometry arrays.
    fn idx(self) -> usize {
        match self {
            Tool::Esm => 0,
            Tool::Bsa => 1,
            Tool::Swf => 2,
        }
    }

    /// i18n key of this tool's label in the « Outils » menu.
    fn key(self) -> &'static str {
        match self {
            Tool::Esm => "tool-esm",
            Tool::Bsa => "tool-bsa",
            Tool::Swf => "tool-swf",
        }
    }
}

impl XimodApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screen: ScreenInfo) -> Self {
        let config = AppConfig::load().unwrap_or_default();
        // The endpoint and model of the first version serve the « openai »
        // profile until it has a user copy (D-03, R-D03-05).
        crate::online::profile::set_legacy(
            &config.online.openai_endpoint,
            &config.online.openai_model,
        );
        let mut i18n = I18n::new();
        // The shared FOMOD crate signs its XML with this application and
        // keeps its translation memory in the configuration folder (F-03).
        ximod_fomod::set_tool(crate::APP_NAME, env!("CARGO_PKG_VERSION"), "");
        ximod_fomod::set_data_dir(Some(crate::config::paths::get().config_dir.clone()));
        i18n.set_locale(&config.locale);
        let countries = CountriesData::load();
        let country_languages = CountryLanguagesData::load();
        let country_names = CountryNamesData::load();

        // SVG/PNG loaders for the flag images.
        egui_extras::install_image_loaders(&cc.egui_ctx);

        match config.theme {
            Theme::Light => cc.egui_ctx.set_visuals(egui::Visuals::light()),
            _ => cc.egui_ctx.set_visuals(egui::Visuals::dark()),
        }

        // Load EVERY font referenced by the language table so the language
        // drop-down renders all scripts instead of tofu (□) boxes.
        let all_fonts = LanguagesData::load().all_fonts();
        let mut font_defs = crate::fonts::build_font_definitions_with_preview(&all_fonts, None);
        // Register any user-supplied game book fonts (never shipped by XIMOD).
        let book_fonts = crate::fonts::load_book_fonts(&mut font_defs);
        cc.egui_ctx.set_fonts(font_defs);
        apply_font_size(&cc.egui_ctx, config.font_size);
        // Menus are content-fit via `fit_menu` (no wrapping). `menu_width` only
        // seeds the first sizing frame: keep it non-zero (≥ the longest item) so a
        // freshly opened menu is laid out correctly at once — a value of 0 left the
        // first open of a wide menu (Dictionnaires) mis-sized until reopened.
        cc.egui_ctx.all_styles_mut(|s| s.spacing.menu_width = 460.0);
        let book_use_game_fonts = !book_fonts.is_empty();

        let temp_locale = config.locale.clone();
        let temp_country = config.country.clone();
        let show_settings = !config.first_start_done;
        let applied_font_size = config.font_size;
        let temp_undo_depth = config.undo_depth;
        let frame_log_ms = config.frame_log_ms as u128;
        {
            let ms = std::env::var("XIMOD_FRAME_LOG")
                .ok()
                .map(|v| v.trim().parse().unwrap_or(100))
                .unwrap_or(config.frame_log_ms as u64);
            // Operations leaving the process are logged from a quarter of
            // the frame threshold.
            crate::SLOW_IO_MS.store(ms / 4, std::sync::atomic::Ordering::Relaxed);
        }
        let (temp_history_enabled, temp_history_max) = (config.history_enabled, config.history_max);
        let temp_autosave_enabled = config.autosave_enabled;
        let temp_toasts = config.toasts;
        let temp_autosave_minutes = config.autosave_minutes;
        let temp_autosave_dir = config.autosave_dir.clone();
        let temp_backup_copies = config.backup_copies;
        let (temp_keep_original, temp_original_copies) =
            (config.keep_original, config.original_copies);
        let temp_check_crc = config.check_crc;
        let temp_mod_dict_auto = config.mod_dict_auto;
        let temp_personal_autofill = config.personal_autofill;
        let temp_personal_keep_field = config.personal_keep_field;
        let temp_old_data_months = config.old_data_months;
        let temp_fuzzy_threshold = config.fuzzy_threshold;
        let temp_normalize = config.normalize;
        let temp_propose_generalization = config.propose_generalization;
        let temp_propose_propagation = config.propose_propagation;
        let temp_propagate_files = config.propagate_files;
        // Rescue folder: what a previous session left behind is offered for
        // resumption before anything else (B-11, R-B11-05).
        let autosave = crate::backup::Autosave::new(config.autosave_dir(), config.autosave_minutes);
        let found = crate::backup::Autosave::pending(autosave.dir());
        let pending_resume = (!found.items.is_empty()).then_some(found.items);
        let quota_alert = found.quota_purged.then(|| {
            i18n.t_arg(
                "backup-quota",
                "size",
                &i18n.size(crate::backup::QUOTA_BYTES),
            )
        });
        // A read-only configuration folder is announced once at start (R-E05-06).
        let read_only_alert = {
            let p = crate::config::paths::get();
            p.read_only.then(|| {
                i18n.t_arg(
                    "config-mode-not-writable",
                    "path",
                    &p.config_dir.display().to_string(),
                )
            })
        };
        // The former papyrus_rules.json of the user, merged once (D-73).
        let rules_migrated = crate::exclusions::Dirs::installed()
            .migrate_papyrus()
            .map(|n| i18n.t_arg("rules-migrated", "n", &n.to_string()));
        // The journal of the session (E-01): its first word.
        let mut journal = crate::journal::Journal::new();
        {
            let p = crate::config::paths::get();
            let mode = if p.mode.as_str() == "portable" {
                i18n.t("journal-mode-portable")
            } else {
                i18n.t("journal-mode-user")
            };
            let start = i18n.t_args(
                "journal-start",
                &[
                    ("version", env!("CARGO_PKG_VERSION")),
                    ("mode", &mode),
                    ("lang", &i18n.display_name(&config.locale)),
                    ("dir", &p.config_dir.display().to_string()),
                ],
            );
            journal.push(
                crate::journal::Level::Info,
                crate::journal::Kind::Start,
                None,
                &start,
            );
        }

        Self {
            temp_locale,
            temp_country,
            temp_theme: config.theme,
            temp_font_size: config.font_size,
            temp_max_recent_files: config.max_recent_files,
            temp_window_width: config.window_width,
            temp_window_height: config.window_height,
            temp_game_bases_dir: config.game_bases_dir.clone(),
            temp_mod_bases_dir: config.mod_bases_dir.clone(),
            temp_eet_dir: config.eet_dir.clone(),
            temp_xtranslator_dir: config.xtranslator_dir.clone(),
            temp_config_mode: crate::config::paths::get().mode,
            config,
            i18n,
            countries,
            country_languages,
            country_names,
            props_win: Default::default(),
            screen,
            docs: Vec::new(),
            idle: IdleSettings::default(),
            target_langs: Vec::new(),
            flags_purpose: FlagsPurpose::Locale,
            pending_output_confirm: false,
            active: 0,
            filter: String::new(),
            filter_statuses: [true; crate::status::COUNT],
            filter_record: String::new(),
            filter_field: String::new(),
            filter_min_chars: 0,
            filter_formid: None,
            filter_source_exact: None,
            filter_in: crate::filter::SearchIn::All,
            filter_regex: false,
            filter_applied: String::new(),
            filter_applied_in: crate::filter::SearchIn::All,
            filter_applied_regex: false,
            filter_typed_at: None,
            filter_error: None,
            filter_focus_request: false,
            view_key: None,
            view_store: Default::default(),
            heights_store: Default::default(),
            heights_serial: 0,
            filter_max_chars: 0,
            filter_unique_only: false,
            visible: Vec::new(),
            view_total: 0,
            view_per_status: [0; crate::status::COUNT],
            view_shown_per_status: [0; crate::status::COUNT],
            view_unique: 0,
            view_unique_remaining: 0,
            status: String::new(),
            applied_font_size,
            show_settings,
            settings_needs_center: show_settings,
            settings_focus_frames: if show_settings { 30 } else { 0 },
            settings_fit: true,
            settings_last_tab: SettingsTab::General,
            settings_tab: SettingsTab::General,
            settings_focus: Focus::TabGeneral,
            settings_focus_dirty: true,
            show_flags: false,
            flags_needs_center: false,
            fields_editor: None,
            fields_needs_center: false,
            fomod_trans: Default::default(),
            fomod_job: None,
            fomod_needs_center: false,
            edit_win: None,
            replace: Default::default(),
            quality: Default::default(),
            dicts: Default::default(),
            dict_editor: Default::default(),
            apply_dlg: Default::default(),
            update: Default::default(),
            fuzzy_cache: Default::default(),
            fuzzy_jobs: Default::default(),
            frag_cache: Default::default(),
            instant_no_confirm: false,
            propagate_dlg: None,
            dialogs: Default::default(),
            filter_speaker: None,
            filter_lock_rule: None,
            comment_cache: Default::default(),
            comment_texts: None,
            import_dlg: None,
            scan_win: Default::default(),
            compare_win: Default::default(),
            review_dlg: None,
            frame_log: std::env::var("XIMOD_FRAME_LOG")
                .ok()
                .map(|v| v.trim().parse().unwrap_or(100))
                .unwrap_or(frame_log_ms),
            grid_heights: Default::default(),
            clipboard: Default::default(),
            last_title: None,
            frame_count: 0,
            repaint_askers: Vec::new(),
            frame_asked: 0,
            frame_events: 0,
            frame_since: std::time::Instant::now(),
            filter_sigs: None,
            filter_sigs_store: Default::default(),
            history_win: Default::default(),
            search_win: Default::default(),
            scripts_win: Default::default(),
            matrices_win: Default::default(),
            matrices_gen: 0,
            rules_win: Default::default(),
            assist_win: Default::default(),
            assist_dlg: Default::default(),
            add_term: None,
            add_term_last: None,
            edit_ctx_sel: None,
            assist_gen: 0,
            assist_cache: None,
            derive_teaches: Default::default(),
            derive_seen: Default::default(),
            generalize: None,
            generalize_offer: None,
            rules_migrated,
            sctx_cache: Default::default(),
            concord_win: Default::default(),
            journal,
            journal_win: Default::default(),
            filter_journal: None,
            alert_logged: None,
            export_enc_logged: false,
            open_started: None,
            savepart_dlg: None,
            encoding_dlg: None,
            localize_cache: Default::default(),
            crc: Default::default(),
            savepart_memory: Default::default(),
            last_save_dir: None,
            last_export_dir: None,
            search_filter: None,
            history_confirm: None,
            online: Default::default(),
            formats_editor: None,
            formats_needs_center: false,
            tr: None,
            tr_needs_center: false,
            tr_preview_font: None,
            edit_win_active_frame: false,
            edit_last_text: None,
            vocab: Default::default(),
            common: Default::default(),
            personal_prompt: None,
            personal_session_choice: None,
            anomalies: Default::default(),
            anomaly_set: Default::default(),
            anomaly_serial: None,
            filter_anomalies: false,
            filter_ambiguous: false,
            filter_prov: crate::provenance::ProvFilter::All,
            filter_review_diff: false,
            filter_columns: Default::default(),
            filter_columns_applied: Default::default(),
            filter_column_errors: Default::default(),
            prov_texts: None,
            grid_page_rows: 20,
            clear_recent_armed: false,
            close_prompt: None,
            exit_prompt: false,
            exit_confirmed: false,
            toasts: crate::ui::toasts::Toasts::default(),
            frame_time: 0.0,
            pack_win: None,
            pack_needs_center: false,
            flag_filter: String::new(),
            flag_filter_focus: false,
            flag_focus: 0,
            show_about: false,
            about_needs_center: false,
            show_status_colors: false,
            status_colors_needs_center: false,
            status_colors_fitted: false,
            color_picker: super::color_picker::ColorPicker::default(),
            show_spell: false,
            spell_needs_center: false,
            spell: None,
            spell_lang: String::new(),
            spell_issues: Vec::new(),
            spell_status: String::new(),
            show_archive: false,
            archive_needs_center: false,
            archive: None,
            archive_filter: String::new(),
            archive_translatable_only: false,
            archive_selected: None,
            archive_status: String::new(),
            show_plugin_pair: false,
            pp_src_path: String::new(),
            pp_tgt_path: String::new(),
            pp_to_game: true,
            show_locres_csv: false,
            lc_src_path: String::new(),
            lc_tgt_path: String::new(),
            lc_to_game: true,
            file_dbs: crate::basedb::FileDbCache::default(),
            show_book: false,
            book_needs_center: false,
            book_idx: None,
            book_doc: 0,
            book_cache: None,
            book_title: String::new(),
            book_src_blocks: Vec::new(),
            book_trans_blocks: Vec::new(),
            book_fonts,
            book_use_game_fonts,
            pending_main_resize: None,
            tool_open: [false; 3],
            tool_needs_center: [false; 3],
            esm_tool: esm_explorer::app::EsmExplorerApp::default(),
            bsa_tool: bsa_explorer::app::BsaExplorerApp::default(),
            swf_tool: swf_explorer::app::SwfTextApp::with_initial(None),
            dict_conv: None,
            pending_save: None,
            show_dict_view: false,
            dict_view: None,
            dict_view_needs_center: false,
            alert: read_only_alert.or(quota_alert),
            pending_reload: None,
            startup_files: Vec::new(),
            edit_session: None,
            grid_scroll_to: None,
            selection: grid::Selection::default(),
            grid_focus_request: None,
            pinned_rows: std::collections::BTreeSet::new(),
            last_criteria: None,
            temp_undo_depth,
            temp_history_enabled,
            temp_history_max,
            autosave,
            pending_resume,
            temp_autosave_enabled,
            temp_toasts,
            temp_autosave_minutes,
            temp_autosave_dir,
            temp_backup_copies,
            temp_keep_original,
            temp_original_copies,
            temp_check_crc,
            temp_mod_dict_auto,
            temp_personal_autofill,
            temp_personal_keep_field,
            temp_old_data_months,
            temp_fuzzy_threshold,
            temp_normalize,
            temp_propose_generalization,
            temp_online: Default::default(),
            temp_propose_propagation,
            temp_propagate_files,
            pending_export: None,
            game_detect: None,
            game_detect_rx: None,
            detect_progress: None,
        }
    }

    /// The « Outils » menu: open the explorers, each in its own independent
    /// window. Several can be open at once; the marker shows which are open and
    /// clicking a tool toggles its window.
    fn tools_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.i18n.t("menu-tools"), |ui| {
            fit_menu(ui);
            // Rechercher et remplacer… (A-11), Ctrl+H.
            let has_doc = !self.docs.is_empty();
            if menu_item(
                ui,
                &self.i18n.t("menu-replace"),
                Some(&shortcut_text(
                    ui.ctx(),
                    &MENU_SHORTCUTS.replace,
                    &self.i18n,
                )),
                has_doc,
                "",
            ) {
                self.open_replace_window(ui.ctx());
            }
            // Recherche avancée… (A-19), Ctrl+Alt+F.
            if menu_item(
                ui,
                &self.i18n.t("menu-search-advanced"),
                Some(&shortcut_text(ui.ctx(), &MENU_SHORTCUTS.search, &self.i18n)),
                has_doc,
                "",
            ) {
                self.open_search_window();
            }
            // Concordance… (A-20), Ctrl+K.
            if menu_item(
                ui,
                &self.i18n.t("menu-concordance"),
                Some(&shortcut_text(
                    ui.ctx(),
                    &MENU_SHORTCUTS.concord,
                    &self.i18n,
                )),
                has_doc,
                "",
            ) {
                self.open_concord_window(ui.ctx(), None);
            }
            if menu_item(
                ui,
                &self.i18n.t("menu-quality"),
                Some(&shortcut_text(
                    ui.ctx(),
                    &MENU_SHORTCUTS.quality,
                    &self.i18n,
                )),
                has_doc,
                "",
            ) {
                self.open_quality_window(false);
            }
            if menu_item(
                ui,
                &self.i18n.t("menu-dialogues"),
                Some(&shortcut_text(
                    ui.ctx(),
                    &MENU_SHORTCUTS.dialogues,
                    &self.i18n,
                )),
                has_doc,
                "",
            ) {
                self.open_dialog_window();
            }
            // Traduire en ligne… (D-01).
            if menu_item(
                ui,
                &self.i18n.t("menu-online-batch"),
                Some(&shortcut_text(
                    ui.ctx(),
                    &MENU_SHORTCUTS.online_batch,
                    &self.i18n,
                )),
                has_doc,
                "",
            ) {
                self.open_online_dialog();
            }
            ui.separator();
            for tool in Tool::ALL {
                // A plain button (not selectable_label): the latter allocates the
                // full available width, which fights the content-fit menu sizing.
                let open = self.tool_open[tool.idx()];
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                let label = format!("{mark}{}", self.i18n.t(tool.key()));
                if ui.button(label).clicked() {
                    if open {
                        self.tool_open[tool.idx()] = false;
                    } else {
                        self.tool_open[tool.idx()] = true;
                        self.tool_needs_center[tool.idx()] = true; // place on open
                    }
                    ui.close_menu();
                }
            }
            ui.separator();
            // Outils ▸ Traduire un FOMOD… (M-02) / Empaqueter une traduction… (M-03).
            {
                let open = self.fomod_trans.open;
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("fomod-menu")))
                    .on_hover_text(self.i18n.t("fomod-menu-tip"))
                    .clicked()
                {
                    // Already open: brought to the front (its own close asks
                    // about unsaved edits).
                    self.open_fomod_translation();
                    self.fomod_needs_center = !open && self.fomod_needs_center;
                    ui.close_menu();
                }
                let open = self.pack_win.is_some();
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("pack-menu")))
                    .on_hover_text(self.i18n.t("pack-menu-tip"))
                    .clicked()
                {
                    if open {
                        self.pack_win = None;
                    } else {
                        self.open_pack_window();
                    }
                    ui.close_menu();
                }
            }
            // Historique… (A-18): a window like the others, marked when open.
            {
                let open = self.history_win.open;
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .add_enabled(
                        !self.docs.is_empty(),
                        egui::Button::new(format!("{mark}{}", self.i18n.t("menu-history"))),
                    )
                    .clicked()
                {
                    if open {
                        self.history_win.open = false;
                    } else {
                        self.open_history_window(None);
                    }
                    ui.close_menu();
                }
            }
            ui.separator();
            // Outils ▸ Données: the editors of the data files (D-03).
            ui.menu_button(self.i18n.t("menu-tools-data"), |ui| {
                fit_menu(ui);
                let open = self.fields_editor.is_some();
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("fields-menu")))
                    .clicked()
                {
                    if open {
                        self.fields_editor = None;
                    } else {
                        self.open_fields_editor();
                    }
                    ui.close_menu();
                }
                // Formats texte… (C-05).
                let open = self.formats_editor.is_some();
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("formats-menu")))
                    .clicked()
                {
                    if open {
                        self.formats_editor = None;
                    } else {
                        self.formats_editor = Some(crate::ui::formats_editor::FormatsEditor::new());
                        self.formats_needs_center = true;
                    }
                    ui.close_menu();
                }
                // Matrices de remplacement… (B-17).
                let mark = if self.matrices_win.open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("matrices-menu")))
                    .clicked()
                {
                    self.open_matrices_window();
                    ui.close_menu();
                }
                // Glossaire, motifs et dérivations… (B-15).
                let mark = if self.assist_win.open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("assist-menu")))
                    .clicked()
                {
                    self.open_assist_window();
                    ui.close_menu();
                }
            });
            // Outils ▸ Règles d'exclusion… (B-18).
            let mark = if self.rules_win.open {
                format!("{} ", crate::icons::DOT)
            } else {
                "    ".to_string()
            };
            if ui
                .button(format!("{mark}{}", self.i18n.t("menu-rules")))
                .clicked()
            {
                self.open_rules_window(app_rules::RulesTab::Active);
                ui.close_menu();
            }
            // Outils ▸ Scripts (C-08): the VMAD switch of the active file.
            ui.menu_button(self.i18n.t("menu-scripts"), |ui| {
                fit_menu(ui);
                // Scripts du mod… (C-06).
                if ui
                    .add_enabled(
                        self.scripts_available(),
                        egui::Button::new(self.i18n.t("menu-scripts-window")),
                    )
                    .clicked()
                {
                    self.open_scripts_window();
                    ui.close_menu();
                }
                ui.separator();
                let state = self.vmad_active_state();
                let mut on = state.unwrap_or(false);
                let resp = ui
                    .add_enabled(
                        state.is_some(),
                        egui::Checkbox::new(&mut on, self.i18n.t("menu-vmad-enable")),
                    )
                    .on_hover_text(self.i18n.t("menu-vmad-enable-tip"))
                    .on_disabled_hover_text(self.i18n.t("menu-vmad-none"));
                if resp.changed() {
                    self.vmad_switch(on);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(self.i18n.t("menu-scripts-rules")).clicked() {
                    self.open_rules_window(app_rules::RulesTab::Editor);
                    ui.close_menu();
                }
            });
            ui.separator();
            // Outils ▸ Traduire l'interface… (E-10).
            {
                let open = self.tr.is_some();
                let mark = if open {
                    format!("{} ", crate::icons::DOT)
                } else {
                    "    ".to_string()
                };
                if ui
                    .button(format!("{mark}{}", self.i18n.t("menu-translate-ui")))
                    .on_hover_text(self.i18n.t("menu-translate-ui-tip"))
                    .clicked()
                {
                    self.open_translation_editor();
                    ui.close_menu();
                }
            }
        });
    }

    fn open_pack_window(&mut self) {
        let s = self.active_settings();
        self.pack_win = Some(crate::ui::package_window::PackageWindow::new(
            s.game(),
            &s.target,
            "",
        ));
        self.pack_needs_center = true;
    }

    /// The packaging window (M-03): an independent viewport with a
    /// remembered geometry, like the fields editor (the FOMOD window has
    /// its own driver since F-03).
    fn tool_windows(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        for which in 1..2 {
            let is_open = if which == 0 {
                false
            } else {
                self.pack_win.is_some()
            };
            if !is_open {
                continue;
            }
            let (id, title_key) = if which == 0 {
                ("ximod-fomod", "fomod-title")
            } else {
                ("ximod-pack", "pack-title")
            };
            let geo = if which == 0 {
                self.config.fomod_win
            } else {
                self.config.pack_win
            };
            let just_opened = if which == 0 {
                self.fomod_needs_center
            } else {
                self.pack_needs_center
            };
            let place = if just_opened {
                let pos = self.open_pos_cascade(geo[0], geo[1]);
                Some((pos, [geo[2].max(400.0), geo[3].max(300.0)]))
            } else {
                None
            };
            let mut builder = egui::ViewportBuilder::default().with_title(format!(
                "{} — {}",
                crate::APP_NAME,
                self.i18n.t(title_key)
            ));
            if let Some((pos, size)) = place {
                builder = builder
                    .with_position(pos)
                    .with_inner_size(size)
                    .with_active(true);
            }
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(id),
                builder,
                |vctx, _class| {
                    if let Some((pos, size)) = place {
                        vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                            pos[0], pos[1],
                        )));
                        vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            size[0], size[1],
                        )));
                    }
                    if which == 0 {
                        self.fomod_needs_center = false;
                    } else {
                        self.pack_needs_center = false;
                    }
                    egui::CentralPanel::default().show(vctx, |ui| {
                        if let Some(w) = self.pack_win.as_mut() {
                            close = w.body(ui, &self.i18n, Some(frame))
                                == crate::ui::package_window::Outcome::Close;
                        }
                    });
                    if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                        if which == 0 {
                            self.config.fomod_win = [x, y, w, h];
                        } else {
                            self.config.pack_win = [x, y, w, h];
                        }
                    }
                    if vctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.pack_win = None;
                let _ = self.config.save();
            }
        }
    }

    /// Open the fields editor on the selected game (Skyrim SE when none).
    fn open_fields_editor(&mut self) {
        let game = self.active_settings().game();
        let game = if game == crate::games::Game::Unknown {
            crate::games::Game::SkyrimSE
        } else {
            game
        };
        self.fields_editor = Some(crate::ui::fields_editor::FieldsEditor::new(game));
        self.fields_needs_center = true;
    }

    /// The fields editor, in its own window (A-06).
    fn fields_window(&mut self, ctx: &egui::Context) {
        if self.fields_editor.is_none() {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-fields-editor");
        let c = &self.config;
        let (sx, sy, sw, sh) = (
            c.fields_win_x,
            c.fields_win_y,
            c.fields_win_w,
            c.fields_win_h,
        );
        let just_opened = self.fields_needs_center;
        let place = if just_opened {
            let pos = self.open_pos_cascade(sx, sy);
            Some((
                pos,
                [
                    if sw > 0.0 { sw } else { 1100.0 },
                    if sh > 0.0 { sh } else { 720.0 },
                ],
            ))
        } else {
            None
        };
        let mut builder = egui::ViewportBuilder::default().with_title(format!(
            "{} — {}",
            crate::APP_NAME,
            self.i18n.t("fields-title")
        ));
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.fields_needs_center = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                if let Some(ed) = self.fields_editor.as_mut() {
                    close = ed.body(ui, &self.i18n);
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                let c = &mut self.config;
                (
                    c.fields_win_x,
                    c.fields_win_y,
                    c.fields_win_w,
                    c.fields_win_h,
                ) = (x, y, w, h);
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.fields_editor = None;
            let _ = self.config.save();
        }
    }

    /// The text formats editor, in its own window (C-05).
    fn formats_window(&mut self, ctx: &egui::Context) {
        if self.formats_editor.is_none() {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-formats-editor");
        let [sx, sy, sw, sh] = self.config.formats_win;
        let just_opened = self.formats_needs_center;
        let place = if just_opened {
            let pos = self.open_pos_cascade(sx, sy);
            Some((
                pos,
                [
                    if sw > 0.0 { sw } else { 860.0 },
                    if sh > 0.0 { sh } else { 560.0 },
                ],
            ))
        } else {
            None
        };
        let mut builder = egui::ViewportBuilder::default().with_title(format!(
            "{} — {}",
            crate::APP_NAME,
            self.i18n.t("formats-title")
        ));
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.formats_needs_center = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                if let Some(ed) = self.formats_editor.as_mut() {
                    close = ed.body(ui, &self.i18n);
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.formats_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.formats_editor = None;
            let _ = self.config.save();
        }
    }

    /// Render every open explorer, each in its **own independent OS window** (an
    /// egui viewport). All three can be open at once. Each shares this `Context`,
    /// so they inherit the content-fit menu styling like the main window.
    fn show_tool_windows(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for tool in Tool::ALL {
            if self.tool_open[tool.idx()] {
                self.show_one_tool(ctx, frame, tool);
            }
        }
    }

    /// Read this tool's saved geometry from the config `(x, y, w, h)`.
    fn tool_geometry(&self, tool: Tool) -> (f32, f32, f32, f32) {
        let c = &self.config;
        match tool {
            Tool::Esm => (c.esm_win_x, c.esm_win_y, c.esm_win_w, c.esm_win_h),
            Tool::Bsa => (c.bsa_win_x, c.bsa_win_y, c.bsa_win_w, c.bsa_win_h),
            Tool::Swf => (c.swf_win_x, c.swf_win_y, c.swf_win_w, c.swf_win_h),
        }
    }

    /// Store this tool's window geometry back into the config.
    fn set_tool_geometry(&mut self, tool: Tool, x: f32, y: f32, w: f32, h: f32) {
        let c = &mut self.config;
        match tool {
            Tool::Esm => {
                (c.esm_win_x, c.esm_win_y, c.esm_win_w, c.esm_win_h) = (x, y, w, h);
            }
            Tool::Bsa => {
                (c.bsa_win_x, c.bsa_win_y, c.bsa_win_w, c.bsa_win_h) = (x, y, w, h);
            }
            Tool::Swf => {
                (c.swf_win_x, c.swf_win_y, c.swf_win_w, c.swf_win_h) = (x, y, w, h);
            }
        }
    }

    /// Show one explorer in its own window.
    fn show_one_tool(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame, tool: Tool) {
        let (vp_id_src, panel_id) = match tool {
            Tool::Esm => ("ximod-tool-esm", "esm_menu_bar"),
            Tool::Bsa => ("ximod-tool-bsa", "bsa_menu_bar"),
            Tool::Swf => ("ximod-tool-swf", "swf_menu_bar"),
        };
        let title = self.i18n.t(tool.key());
        let i = tool.idx();
        let (sx, sy, sw, sh) = self.tool_geometry(tool);
        let just_opened = self.tool_needs_center[i];

        // First-frame placement: saved geometry, or cascaded near the main window.
        let place = if just_opened {
            let pos = self.open_pos_cascade(sx, sy);
            let w = if sw > 0.0 { sw } else { 1000.0 };
            let h = if sh > 0.0 { sh } else { 700.0 };
            Some((pos, [w, h]))
        } else {
            None
        };

        let id = egui::ViewportId::from_hash_of(vp_id_src);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.dash(&self.i18n.t("app-title"), &title));
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }

        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            // Force the placement via explicit commands too — for child viewports
            // this is more reliable than the builder alone.
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.tool_needs_center[i] = false;

            egui::TopBottomPanel::top(panel_id).show(vctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    match tool {
                        Tool::Esm => self.esm_tool.menu_contents(ui, vctx),
                        Tool::Bsa => self.bsa_tool.menu_contents(ui, vctx),
                        Tool::Swf => self.swf_tool.menu_contents(ui, vctx),
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(title.as_str());
                    });
                });
            });
            match tool {
                Tool::Esm => self.esm_tool.body(vctx, frame),
                Tool::Bsa => self.bsa_tool.body(vctx, frame),
                Tool::Swf => self.swf_tool.body(vctx, frame),
            }

            // Persist geometry — but not on the placement frame, where the window
            // rect still reports its pre-placement value.
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.set_tool_geometry(tool, x, y, w, h);
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.tool_open[i] = false;
            let _ = self.config.save(); // persist this window's last geometry now
        }
    }

    /// The main window's current rectangle `(x, y, w, h)`. Falls back to a
    /// screen-centered rectangle when the position is not yet known.
    fn main_rect(&self) -> (f32, f32, f32, f32) {
        let mw = self.config.window_width;
        let mh = self.config.window_height;
        let (mx, my) = if self.config.window_x >= 0.0 && self.config.window_y >= 0.0 {
            (self.config.window_x, self.config.window_y)
        } else {
            (
                ((self.screen.width - mw) / 2.0).max(0.0),
                ((self.screen.height - mh) / 2.0).max(0.0),
            )
        };
        (mx, my, mw, mh)
    }

    /// Centered position for a child window of the given size, **on the main
    /// window** (which is itself centered on the screen).
    fn centered(&self, w: f32, h: f32) -> [f32; 2] {
        let (mx, my, mw, mh) = self.main_rect();
        [
            (mx + (mw - w) / 2.0).max(0.0),
            (my + (mh - h) / 2.0).max(0.0),
        ]
    }

    /// Opening position: the saved one when valid, otherwise centered on the
    /// main window (first launch, when Config.ini had `-1`).
    fn open_pos(&self, saved_x: f32, saved_y: f32, w: f32, h: f32) -> [f32; 2] {
        if saved_x >= 0.0 && saved_y >= 0.0 {
            [saved_x, saved_y]
        } else {
            self.centered(w, h)
        }
    }

    /// Opening position for a tool window: the saved one when valid, otherwise
    /// cascaded from the main window (a slight down-right offset, Windows-style).
    pub(crate) fn open_pos_cascade(&self, saved_x: f32, saved_y: f32) -> [f32; 2] {
        if saved_x >= 0.0 && saved_y >= 0.0 {
            return [saved_x, saved_y];
        }
        const OFFSET: f32 = 40.0;
        let (mx, my, _, _) = self.main_rect();
        [(mx + OFFSET).max(0.0), (my + OFFSET).max(0.0)]
    }

    /// An icon button from `assets/images/icons/<file>`, falling back to a text
    /// glyph if the image is missing. Returns the click response.
    fn icon_button(ui: &mut egui::Ui, file: &str, fallback: &str, size: f32) -> egui::Response {
        if let Some(path) = crate::data::image_icon(file) {
            ui.add_sized(
                egui::vec2(size, size),
                egui::ImageButton::new(
                    egui::Image::from_uri(format!("file://{}", path.display()))
                        .fit_to_exact_size(egui::vec2(size - 4.0, size - 4.0)),
                ),
            )
        } else {
            ui.button(fallback)
        }
    }

    fn active_doc(&self) -> Option<&Doc> {
        self.docs.get(self.active)
    }

    // --------------------------- file actions ------------------------------

    fn open_file(&mut self, parent: &eframe::Frame) {
        let picked = rfd::FileDialog::new()
            .set_parent(parent)
            .add_filter(
                self.i18n.t("dialog-translations-plugins"),
                &{
                    let mut v: Vec<String> = ["eet", "sst", "dsd", "json", "xml"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    v.extend(crate::games::registry().all_plugin_extensions());
                    v
                }
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            )
            .add_filter(
                self.i18n.t("dialog-strings-files"),
                &["strings", "dlstrings", "ilstrings"],
            )
            // Compiled scripts (C-06).
            .add_filter(self.i18n.t("output-scripts"), &["pex"])
            .add_filter(
                self.i18n.t("open-filter-text"),
                &["txt", "ini", "json", "csv", "cfg"],
            )
            .add_filter(self.i18n.t("dialog-all-files"), &["*"])
            .pick_files();
        if let Some(paths) = picked {
            for p in paths {
                self.load_path(p);
            }
        }
    }

    pub(crate) fn load_path(&mut self, path: PathBuf) {
        self.load_path_with(path, None);
    }

    /// [`load_path`](Self::load_path) with the document's settings imposed
    /// (resume, B-11) instead of detected / defaulted. Returns whether the
    /// document is now open.
    pub(crate) fn load_path_with(&mut self, path: PathBuf, settings: Option<DocSettings>) -> bool {
        // Already open? Just focus its tab.
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = i;
            return true;
        }
        self.open_started = Some(std::time::Instant::now());
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let imposed_game = settings
            .as_ref()
            .map(|s| s.game())
            .filter(|g| *g != Game::Unknown);
        // A compiled script, or the « scripts » document of a plugin (C-06).
        if ext == "pex" {
            return self.open_pex_path(path, settings);
        }
        // A file that cannot say its game (a text file) opens with the game
        // of the toolbar; none chosen, it is asked for (D-95).
        let no_game = settings.is_none() && self.idle.game.trim().is_empty();
        let is_plugin_ext = crate::games::registry().is_plugin_extension(&ext);
        // A text file (MCM, INI, JSON…, C-05): the formats decide, or ask.
        // A `.json` is tried as a base first (pivot, DSD).
        let text_candidate = !is_plugin_ext
            && !matches!(
                ext.as_str(),
                "eet" | "sst" | "dsd" | "strings" | "dlstrings" | "ilstrings"
            );
        // A `.json` or `.xml` is tried as a base first (pivot, DSD, the XML
        // of xTranslator), then as a text file (D-08).
        let result = if is_plugin_ext {
            crate::plugin::read_file_as(&path, imposed_game)
        } else if text_candidate && ext != "json" && ext != "xml" {
            if no_game {
                return self.ask_game(&path);
            }
            return self.open_text_file(path, settings);
        } else {
            match tm::load(&path) {
                Ok(tm) if tm.entries.is_empty() && text_candidate => {
                    if no_game {
                        return self.ask_game(&path);
                    }
                    return self.open_text_file(path, settings);
                }
                Ok(tm) => Ok(tm),
                Err(_) if text_candidate => {
                    if no_game {
                        return self.ask_game(&path);
                    }
                    return self.open_text_file(path, settings);
                }
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(mut tm) => {
                let n = tm.entries.len();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                // The new document takes the detected game and the session
                // defaults (R-E12-01).
                let detected = Game::from_id(&tm.meta.game);
                let declared = settings.is_some();
                let mut settings = settings.unwrap_or_else(|| {
                    DocSettings::for_new_doc(detected, &self.idle, &self.config)
                });
                // The game detected and none in the toolbar: the file opens
                // with it, and the toolbar keeps it for the next one (D-95);
                // still unknown, it is asked for.
                if settings.game.trim().is_empty() {
                    return self.ask_game(&path);
                }
                if self.idle.game.trim().is_empty() {
                    self.idle.game = settings.game.clone();
                }
                // The page of code of a plugin of an older game (D-09), before
                // the languages are looked at: the texts may change.
                let enc = self.encoding_resolve(&path, &mut tm, &settings, declared);
                let enc_note = self.encoding_note(&enc);
                // The source languages the file provides decide the « Source »
                // list; the source is picked among them (the requested one when
                // it is there, else English, else the first / best candidate).
                let source_langs = Self::source_languages_of(&path, &tm);
                let wanted = settings.source.trim().to_lowercase();
                let pick = source_langs
                    .iter()
                    .find(|l| **l == wanted)
                    .or_else(|| {
                        source_langs
                            .iter()
                            .find(|l| *l == "eng")
                            .filter(|_| wanted.is_empty())
                    })
                    .or_else(|| source_langs.first())
                    .cloned()
                    .unwrap_or_default();
                let detected_note = (!wanted.is_empty() && !pick.is_empty() && pick != wanted)
                    .then(|| {
                        self.i18n
                            .t_arg("status-source-set", "lang", &self.i18n.display_name(&pick))
                    });
                settings.source = pick;
                // Localized plugin? Resolve its external STRINGS so the strings
                // are readable (and later inlinable / exportable).
                let mut resolved = 0usize;
                let mut strings = None;
                if tm.meta.source_format == "plugin" && tm.entries.iter().any(|e| e.idstexte >= 0) {
                    // The STRINGS of Skyrim LE are in the page of their language (R-D09-06).
                    let codec = app_encoding::page_game(&settings).then(|| Codec {
                        page: self.pages().for_language(&settings.source),
                        strict: false,
                    });
                    let (n, bundle) = crate::tm::encoding::with_input(codec, || {
                        Self::resolve_strings_for(&path, &mut tm, &settings.source)
                    });
                    resolved = n;
                    strings = bundle;
                }
                // The interface files of a plugin (R-C05-05).
                let interface_files = if tm.meta.source_format == "plugin" {
                    crate::textfmt::interface_files_of(&path, &settings.source, settings.game())
                } else {
                    Vec::new()
                };
                let strings_note = (resolved > 0).then(|| {
                    self.i18n
                        .t_arg("status-via-strings", "n", &resolved.to_string())
                });
                let mut doc = Doc::new(path.clone(), tm, strings, settings, self.config.undo_depth);
                doc.source_langs = source_langs;
                doc.interface_files = interface_files;
                doc.enc = enc;
                let mut head = self
                    .i18n
                    .t_args("status-opened", &[("name", &name), ("n", &n.to_string())]);
                if let Some(note) = strings_note {
                    head = self.i18n.paren(&head, &note);
                }
                if let Some(note) = enc_note {
                    head = self.i18n.dash(&head, &note);
                }
                if let Some(note) = detected_note {
                    head = self.i18n.dash(&head, &note);
                }
                self.finish_open(doc, head)
            }
            Err(e) => {
                self.notify_err(self.i18n.t_arg("err-open", "error", &e.to_string()));
                false
            }
        }
    }

    /// The common end of an opening: the dictionary stack pre-fills the
    /// lines (an undoable operation that leaves the file « non modifié »,
    /// R-A01-07), the document joins the tabs, the status bar sums it up.
    pub(crate) fn finish_open(&mut self, mut doc: Doc, head: String) -> bool {
        // The identity of the file, checked against the mod's dictionary
        // (B-12): a question may hold the opening.
        if doc.ident.is_none() {
            doc.ident = Self::ident_of(&doc);
        }
        match self.crc_on_open(doc, head) {
            Some((doc, head)) => self.finish_open_now(doc, head),
            None => true,
        }
    }

    /// [`finish_open`](Self::finish_open) once the mod's dictionary is
    /// settled.
    pub(crate) fn finish_open_now(&mut self, mut doc: Doc, head: String) -> bool {
        let path = doc.path.clone();
        let stack = self.resolve_stack(&path, &doc.settings);
        // The VMAD lines (C-08): locks lifted around the filling, laid
        // again after it as the files remember the switch.
        let vmad_on = self.vmad_remembered(&doc);
        doc.tm.meta.vmad = vmad_on;
        self.vmad_log_unread(&doc);
        // The exclusion rules (B-18): those the mod's dictionary remembers,
        // lifted around the filling, laid after it.
        self.rules_remembered(&mut doc);
        let plan = self.rules_plan(&doc);
        let mut opened_report: Option<crate::dict_stack::StackReport> = None;
        {
            let stack_line = match stack {
                Some(stack) => {
                    let mut origins = Vec::new();
                    let report = record(
                        &mut doc.undo,
                        &mut doc.tm.entries,
                        OpKind::ApplyBase,
                        Some(stack.label(&self.i18n)),
                        |es| {
                            Self::vmad_release(es);
                            plan.release(es);
                            let r = stack.auto_translate_entries(es, &mut origins);
                            plan.lay(es);
                            Self::vmad_lay(es, vmad_on);
                            r
                        },
                    );
                    opened_report = Some(report.clone());
                    stack.status_line(&self.i18n, &report)
                }
                None => {
                    Self::vmad_release(&mut doc.tm.entries);
                    plan.release(&mut doc.tm.entries);
                    plan.lay(&mut doc.tm.entries);
                    Self::vmad_lay(&mut doc.tm.entries, vmad_on);
                    self.i18n.t("dict-stack-no-target")
                }
            };
            doc.mark_saved();
            // The pre-filling of the opening is not a change of the work:
            // no history entry (A-18).
            doc.undo.take_log();
            // Lines whose text changed since the mod's dictionary (B-07).
            let changed = self.changed_since_dict(&doc.path, &doc.settings, &doc.tm.entries);
            let stack_line = if changed > 0 {
                self.i18n.dash(
                    &stack_line,
                    &self
                        .i18n
                        .t_arg("status-changed-lines", "n", &changed.to_string()),
                )
            } else {
                stack_line
            };
            let interface_note = doc.interface_files.first().map(|p| {
                self.i18n.t_arg(
                    "interface-found",
                    "file",
                    &p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            });
            // Before « Tous les mods », which stays last (B-25).
            let at = self.all_index().unwrap_or(self.docs.len());
            self.docs.insert(at, doc);
            self.active = at;
            self.config.add_recent_file(path);
            // The journal: the opening, then the stack level by level (R-E01-02).
            let secs = self
                .open_started
                .take()
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(0.0);
            self.journal_opened(at, secs);
            if let Some(report) = &opened_report {
                self.journal_stack(at, report);
            }
            let mut status = self.i18n.dash(&head, &stack_line);
            if let Some(note) = interface_note {
                status = self.i18n.dash(&status, &note);
            }
            self.status = status;
            true
        }
    }

    /// Try to load the plugin's external STRINGS and resolve its string ids in
    /// place. Looks first for loose STRINGS next to the plugin (or a sibling
    /// `Strings/`), then inside a sibling **BSA/BA2** archive (Creation Club and
    /// many mods ship their STRINGS there). Returns how many were resolved and the
    /// bundle that worked (kept for later delocalization).
    /// Resolve a plugin's external STRINGS for a given source language, filling
    /// each localized entry's `source` (never its `trans`). Free-standing so it
    /// can run per open document on a language change without borrowing `self`.
    fn resolve_strings_for(
        path: &std::path::Path,
        tm: &mut TranslationMemory,
        source_language: &str,
    ) -> (usize, Option<crate::plugin::StringsBundle>) {
        crate::headless::resolve_strings(path, tm, source_language)
    }

    /// The dictionary tree of a document (B-02): discovered from the folders,
    /// merged with what the user saved for the game and pair; the personal
    /// node follows the document's own switch (E-12).
    fn dict_tree(&self, path: &std::path::Path, settings: &DocSettings) -> crate::dicts::Tree {
        crate::headless::dict_tree(&self.config, path, settings)
    }

    /// `Dicts_<Game>_<SRC>-<DST>`'s key part.
    fn dicts_key(settings: &DocSettings) -> String {
        crate::headless::dicts_key(settings)
    }

    /// The dictionary stack of a document (R-E12-06, B-02): the active
    /// nodes of its tree, each indexed through the per-file cache.
    fn resolve_stack(
        &mut self,
        path: &std::path::Path,
        settings: &DocSettings,
    ) -> Option<DictStack> {
        if settings.target.trim().is_empty() {
            return None;
        }
        let tree = self.dict_tree(path, settings);
        let mut stack = DictStack::resolve(&tree, &mut self.file_dbs);
        stack.normalize = self.config.normalize;
        // A mod dictionary set aside at opening (B-12).
        if !self.crc.off.is_empty() {
            stack.levels.retain(|l| !self.crc.off.contains(&l.path));
        }
        Some(stack)
    }

    /// The source languages a file provides (ISO 639-3), best first: the
    /// STRINGS found next to a localized plugin or in its archive; the language
    /// a base declares; otherwise the languages detected in the source text
    /// (Lingua), with their confidence shown in the status bar.
    pub(crate) fn source_languages_of(
        path: &std::path::Path,
        tm: &TranslationMemory,
    ) -> Vec<String> {
        let is_localized_plugin =
            tm.meta.source_format == "plugin" && tm.entries.iter().any(|e| e.idstexte >= 0);
        if is_localized_plugin {
            let mut langs: Vec<String> = crate::plugin::StringsBundle::available_langs(path)
                .iter()
                .map(|tok| {
                    crate::lang::iso3_from_token(tok)
                        .map(str::to_string)
                        .unwrap_or_else(|| tok.to_lowercase())
                })
                .collect();
            langs.sort();
            langs.dedup();
            if !langs.is_empty() {
                return langs;
            }
        }
        // A loose STRINGS file names its language.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if matches!(ext.as_str(), "strings" | "dlstrings" | "ilstrings")
            && let Some(tok) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(crate::lang::strings_lang_token)
        {
            return vec![
                crate::lang::iso3_from_token(&tok)
                    .map(str::to_string)
                    .unwrap_or_else(|| tok.to_lowercase()),
            ];
        }
        // A base that declares its source language (xTranslator XML…).
        let declared = tm.meta.lang_source.trim();
        if !declared.is_empty() {
            return vec![
                crate::lang::iso3_from_token(declared)
                    .map(str::to_string)
                    .unwrap_or_else(|| declared.to_lowercase()),
            ];
        }
        // Otherwise: detect from the text.
        crate::langdetect::detect_texts(tm.entries.iter().map(|e| e.source.as_str()))
            .into_iter()
            .map(|c| c.iso3)
            .collect()
    }

    /// Re-run the dictionary stack of document `idx` on its still-empty lines
    /// and report in the status bar. Returns the number of lines filled.
    /// After a target change: a text document reads its target file first
    /// (R-C05-03), then the dictionary stack completes the empty lines.
    fn target_changed(&mut self, idx: usize) {
        // The journal (R-E01-02): the new target, before what it brings.
        if let Some(doc) = self.docs.get(idx) {
            let code = doc.settings.target.clone();
            let lang = if code.trim().is_empty() {
                "—".to_string()
            } else {
                self.i18n.display_name(&code)
            };
            let name = doc.file_name();
            let text = self.i18n.t_arg("journal-target", "lang", &lang);
            self.journal_for(
                crate::journal::Level::Info,
                crate::journal::Kind::Config,
                Some(&name),
                &text,
            );
        }
        let head = if self.docs.get(idx).is_some_and(Doc::is_text) {
            self.text_target_changed(idx)
        } else {
            None
        };
        // The mod's dictionary is known now: the pages it remembers (D-09),
        // then the identity check (B-12), whose question may hold the
        // dictionaries.
        self.encoding_on_target(idx);
        // The VMAD switch the mod's dictionary remembers (C-08).
        if let Some(doc) = self.docs.get(idx) {
            let on = self.vmad_remembered(doc);
            self.docs[idx].tm.meta.vmad = on;
        }
        let note = match self.crc_on_target(idx) {
            Ok(note) => note,
            Err(()) => return,
        };
        self.restack_doc(idx);
        if let Some(note) = note {
            self.status = self.i18n.dash(&self.status, &note);
        }
        if let Some(head) = head {
            self.status = self.i18n.dash(&head, &self.status);
        }
    }

    fn restack_doc(&mut self, idx: usize) -> usize {
        self.restack_doc_as(idx, OpKind::ApplyBase, None)
    }

    /// [`restack_doc`](Self::restack_doc) recorded as `kind` (the
    /// exclusion rules applied again, B-18), `label` naming it.
    pub(crate) fn restack_doc_as(
        &mut self,
        idx: usize,
        kind: OpKind,
        label: Option<String>,
    ) -> usize {
        let Some(doc) = self.docs.get(idx) else {
            return 0;
        };
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        let plan = self.rules_plan(doc);
        let Some(stack) = self.resolve_stack(&path, &settings) else {
            // No target: the rules alone.
            let doc = &mut self.docs[idx];
            let vmad_on = doc.tm.meta.vmad;
            record(&mut doc.undo, &mut doc.tm.entries, kind, label, |es| {
                Self::vmad_release(es);
                plan.release(es);
                plan.lay(es);
                Self::vmad_lay(es, vmad_on);
            });
            if kind == OpKind::ApplyBase {
                self.status = self.i18n.t("dict-stack-no-target");
            }
            return 0;
        };
        let doc = &mut self.docs[idx];
        let vmad_on = doc.tm.meta.vmad;
        let mut origins = Vec::new();
        let label = label.or_else(|| Some(stack.label(&self.i18n)));
        let report = record(&mut doc.undo, &mut doc.tm.entries, kind, label, |es| {
            Self::vmad_release(es);
            plan.release(es);
            let r = stack.auto_translate_entries(es, &mut origins);
            plan.lay(es);
            Self::vmad_lay(es, vmad_on);
            r
        });
        let mut line = stack.status_line(&self.i18n, &report);
        self.journal_stack(idx, &report);
        // Lines whose text changed since the mod's dictionary (B-07).
        let (path, settings, entries) = {
            let doc = &self.docs[idx];
            (
                doc.path.clone(),
                doc.settings.clone(),
                doc.tm.entries.clone(),
            )
        };
        let changed = self.changed_since_dict(&path, &settings, &entries);
        if changed > 0 {
            line = self.i18n.dash(
                &line,
                &self
                    .i18n
                    .t_arg("status-changed-lines", "n", &changed.to_string()),
            );
        }
        self.status = match self.interface_note(idx) {
            Some(note) => self.i18n.dash(&line, &note),
            None => line,
        };
        report.total()
    }

    /// « Fichier d'interface trouvé : … » for a plugin that has one (R-C05-05).
    fn interface_note(&self, idx: usize) -> Option<String> {
        let p = self.docs.get(idx)?.interface_files.first()?;
        Some(
            self.i18n.t_arg(
                "interface-found",
                "file",
                &p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ),
        )
    }

    /// The settings the toolbar and the Dictionnaires menu work with: the
    /// active document's, or the toolbar lists when no document is open
    /// (R-E12-02); empty lists give empty strings.
    fn active_settings(&self) -> DocSettings {
        match self.active_doc() {
            Some(d) => d.settings.clone(),
            None => self.idle.resolved(&self.config),
        }
    }

    /// Change the game of document `idx` (R-E12-03): re-read the file with the
    /// new game's rules, carry the translations over by layered matching, then
    /// apply the document's dictionary stack. The other documents are untouched.
    pub(crate) fn reload_doc_as(&mut self, idx: usize, game: String) {
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let path = doc.path.clone();
        let is_plugin = doc.tm.meta.source_format == "plugin";
        let target = Game::from_id(game.trim());
        let mut settings = doc.settings.clone();
        settings.game = game.trim().to_string();
        if !is_plugin || target == Game::Unknown {
            // A base or an unknown game id: nothing to re-extract, the setting
            // alone changes (bases and exports follow it).
            self.docs[idx].settings = settings;
            self.restack_doc(idx);
            return;
        }
        let name = doc.file_name();
        let codec = doc.enc.codec();
        match crate::plugin::read_file_with(&path, Some(target), codec) {
            Ok(mut tm) => {
                let mut strings = None;
                if tm.entries.iter().any(|e| e.idstexte >= 0) {
                    let (_, bundle) = Self::resolve_strings_for(&path, &mut tm, &settings.source);
                    strings = bundle;
                }
                // Layered matching (FormID → EDID → context → text) from the
                // document's current lines, then the stack for what is left.
                let old = &self.docs[idx].tm;
                let report = merge(&mut tm, old, MergeOptions::default());
                let was_dirty = self.docs[idx].is_dirty();
                let doc = &mut self.docs[idx];
                doc.tm = tm;
                doc.sources_rev += 1;
                doc.strings = strings;
                doc.settings = settings;
                // The lines were re-extracted: the history no longer applies
                // to them (R-A01-06).
                doc.undo.clear();
                if !was_dirty {
                    doc.mark_saved();
                }
                let filled = self.restack_doc(idx);
                self.status = self.i18n.t_args(
                    "status-reloaded-as",
                    &[
                        ("name", &name),
                        ("game", target.display_name()),
                        ("lines", &self.docs[idx].tm.entries.len().to_string()),
                        ("kept", &report.effective.to_string()),
                        ("filled", &filled.to_string()),
                    ],
                );
            }
            Err(e) => {
                self.status = self
                    .i18n
                    .colon(&self.i18n.t("status-export-read-error"), &e.to_string())
            }
        }
    }

    /// Confirmation before « recharger en tant que » when the document was
    /// modified (R-E12-03).
    fn reload_window(&mut self, ctx: &egui::Context) {
        let Some((idx, game)) = self.pending_reload.clone() else {
            return;
        };
        let Some(doc) = self.docs.get(idx) else {
            self.pending_reload = None;
            return;
        };
        let label = DocSettings {
            game: game.clone(),
            ..doc.settings.clone()
        }
        .game_label();
        let msg = self.i18n.t_args(
            "doc-reload-as-game",
            &[("file", &doc.file_name()), ("game", &label)],
        );
        let title = self.i18n.t("doc-reload-title");
        let oui = self.i18n.t("btn-yes");
        let annuler = self.i18n.t("btn-cancel");
        let mut choice: Option<bool> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(4.0);
                ui.add(egui::Label::new(msg).wrap_mode(egui::TextWrapMode::Wrap));
                ui.add_space(12.0);
                let (p, c) = confirm_buttons(ui, &oui, &annuler);
                if p {
                    choice = Some(true);
                } else if c {
                    choice = Some(false);
                }
            });
        if let Some(c) = choice {
            self.pending_reload = None;
            if c {
                self.reload_doc_as(idx, game);
            }
        }
    }

    /// Non-interactive export of one document to its own folder, mirroring the
    /// per-mode logic of [`Self::export_translation`] but with an auto-derived
    /// file name and no dialogs. Falls back to a lossless pivot (`.json`) when the
    /// selected mode does not apply to the document (e.g. a STRINGS/plugin mode on
    /// a loose translation, or DSD on a non-Skyrim game).
    #[allow(dead_code)] // Kept for the automatic export at resume (B-11) / close (E-06).
    fn export_doc_auto(
        doc: &Doc,
        tm: &TranslationMemory,
        mode: OutputMode,
        target_lang: &str,
    ) -> Result<(), String> {
        let dir = doc
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let stem = doc
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let ext = doc
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("esp")
            .to_string();
        let game = Game::from_id(&tm.meta.game);
        let is_plugin = tm.meta.source_format == "plugin";
        let utf8 = if is_plugin { game.uses_utf8() } else { true };
        let lang = crate::lang::strings_suffix_for(game, target_lang);
        let plugin_name = doc.file_name();

        match mode {
            OutputMode::FullPlugin | OutputMode::OverridePatch if is_plugin => {
                let srcbytes = std::fs::read(&doc.path).map_err(|e| e.to_string())?;
                let bytes = if mode == OutputMode::FullPlugin {
                    crate::plugin::write_delocalized_esp(&srcbytes, tm, doc.strings.as_ref())
                        .map(|(b, _)| b)
                        .map_err(|e| e.to_string())?
                } else {
                    let opts = crate::plugin::OverrideOptions {
                        bundle: doc.strings.as_ref(),
                        esl_if_eligible: false,
                    };
                    crate::plugin::write_override_esp_with(&srcbytes, tm, &plugin_name, &opts)
                        .map(|(b, _)| b)
                        .map_err(|e| e.to_string())?
                };
                // Never overwrite the source plugin: a full esp is suffixed with
                // the language, an override with `_patch`.
                let name = if mode == OutputMode::OverridePatch {
                    format!("{stem}_patch.{ext}")
                } else {
                    format!("{stem}_{lang}.{ext}")
                };
                std::fs::write(dir.join(name), &bytes).map_err(|e| e.to_string())
            }
            OutputMode::Dsd if game.supports_dsd() => {
                let (bytes, n) = crate::tm::dsd::to_json_bytes(tm);
                if n == 0 {
                    return Err(crate::l10n!("err-nothing-to-export"));
                }
                std::fs::write(dir.join(format!("{stem}_DSD.json")), &bytes)
                    .map_err(|e| e.to_string())
            }
            OutputMode::Strings | OutputMode::Hybrid => {
                let files = crate::plugin::build_strings_files(tm, utf8);
                if files.is_empty() {
                    return Self::save_pivot(doc, &dir, &stem, &lang);
                }
                let mut ok = 0usize;
                for (kind, bytes) in &files {
                    let name = format!("{stem}_{lang}.{}", kind.extension());
                    if std::fs::write(dir.join(name), bytes).is_ok() {
                        ok += 1;
                    }
                }
                if ok > 0 {
                    Ok(())
                } else {
                    Err(crate::l10n!("err-strings-write"))
                }
            }
            // A plugin-only mode on a non-plugin document, or DSD on a game that
            // does not support it: keep the work via a lossless pivot file.
            _ => Self::save_pivot(doc, &dir, &stem, &lang),
        }
    }

    /// Save a document as a lossless pivot JSON next to its source (fallback for
    /// [`Self::export_doc_auto`] when the chosen mode does not apply).
    fn save_pivot(doc: &Doc, dir: &std::path::Path, stem: &str, lang: &str) -> Result<(), String> {
        let out = dir.join(format!("{stem}_{lang}.json"));
        tm::save(&doc.tm, &out)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Autosave cycle (B-11, R-B11-02/03): every interval, each document whose
    /// content moved since its last autosave and that is modified is written
    /// in the background; finished writes are reported discreetly.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        for ev in self.autosave.poll() {
            match ev {
                crate::backup::AutosaveEvent::Done {
                    source,
                    at,
                    revision,
                } => {
                    if let Some(d) = self.docs.iter_mut().find(|d| d.path == source) {
                        d.autosaved_revision = revision;
                    }
                    self.status = self.i18n.t_arg(
                        "backup-autosaved",
                        "time",
                        &crate::backup::display_time(&self.i18n, at),
                    );
                }
                crate::backup::AutosaveEvent::Failed { source, error } => {
                    self.notify_warn(self.i18n.colon(
                        &self.i18n.t_arg(
                            "backup-autosave-failed",
                            "file",
                            &source.display().to_string(),
                        ),
                        &error.to_string(),
                    ));
                }
            }
        }
        if !self.config.autosave_enabled {
            return;
        }
        // A pass even when the user is idle (no input → no repaint otherwise).
        ctx.request_repaint_after(std::time::Duration::from_secs(30));
        if !self.autosave.due() {
            return;
        }
        for d in &self.docs {
            if !d.is_dirty()
                || d.undo.revision == d.autosaved_revision
                || self.autosave.is_inflight(&d.path)
            {
                continue;
            }
            let modified = d.tm.entries.iter().filter(|e| e.is_translated()).count();
            let manifest = crate::backup::Manifest::new(&d.path, &d.settings, modified);
            let mut snapshot = d.tm.clone();
            // The entries not yet written come back with a resumption.
            snapshot.history = d.history.pending().to_vec();
            self.autosave.spawn(snapshot, manifest, d.undo.revision);
        }
    }

    /// « Reprendre le travail ? » (B-11, R-B11-05, figure B.2), shown at start
    /// when rescue files exist. Reprendre reopens each source with its saved
    /// state; Ignorer leaves the files for 30 days without offering them again.
    fn resume_window(&mut self, ctx: &egui::Context) {
        let Some(items) = self.pending_resume.as_ref() else {
            return;
        };
        let title = self.i18n.t("backup-resume-title");
        let intro = self.i18n.t("backup-resume-intro");
        let hint = self.i18n.t("backup-resume-hint");
        let (col_file, col_saved, col_lines) = (
            self.i18n.t("backup-col-file"),
            self.i18n.t("backup-col-saved"),
            self.i18n.t("backup-col-modified"),
        );
        let rows: Vec<(String, String, String, bool)> = items
            .iter()
            .map(|(m, _)| {
                (
                    m.file_name(),
                    crate::backup::display_datetime(&self.i18n, m.saved),
                    m.modified.to_string(),
                    m.source.is_file(),
                )
            })
            .collect();
        let missing = self.i18n.t("backup-source-missing-short");
        let ignore = self.i18n.t("btn-ignore");
        let resume = self.i18n.t("btn-resume");
        let mut choice: Option<bool> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.set_width(540.0);
                ui.add_space(4.0);
                ui.add(egui::Label::new(&intro).wrap_mode(egui::TextWrapMode::Wrap));
                ui.add_space(6.0);
                egui::Grid::new("resume_rows")
                    .striped(true)
                    .num_columns(3)
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.strong(&col_file);
                        ui.strong(&col_saved);
                        ui.strong(&col_lines);
                        ui.end_row();
                        for (file, saved, lines, exists) in &rows {
                            if *exists {
                                ui.label(file);
                            } else {
                                ui.label(
                                    egui::RichText::new(self.i18n.dash(file, &missing)).weak(),
                                );
                            }
                            ui.label(saved);
                            ui.label(lines);
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(&hint).weak())
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(egui::RichText::new(&resume).strong()).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(&ignore).clicked() {
                            choice = Some(false);
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(false);
        }
        if let Some(c) = choice {
            let items = self.pending_resume.take().unwrap_or_default();
            if c {
                self.do_resume(items);
            } else {
                for (m, _) in &items {
                    let _ = self.autosave.ignore(m);
                }
            }
        }
    }

    /// Reopen each source and apply its saved state as an undoable operation
    /// that leaves the document modified (R-B11-05).
    fn do_resume(&mut self, items: Vec<(crate::backup::Manifest, PathBuf)>) {
        let mut notes: Vec<String> = Vec::new();
        let mut missing = false;
        for (m, state) in items {
            if !m.source.is_file() {
                missing = true;
                notes.push(self.i18n.t_arg(
                    "backup-resume-source-missing",
                    "path",
                    &m.source.display().to_string(),
                ));
                continue;
            }
            let saved = match crate::backup::Autosave::load_state(&state) {
                Ok(tm) => tm,
                Err(e) => {
                    notes.push(self.i18n.colon(&m.file_name(), &e.to_string()));
                    continue;
                }
            };
            if !self.load_path_with(m.source.clone(), Some(m.settings.clone())) {
                continue;
            }
            let idx = self.active;
            let doc = &mut self.docs[idx];
            let report = record(
                &mut doc.undo,
                &mut doc.tm.entries,
                OpKind::Resume,
                None,
                |es| crate::backup::resume_into(es, &saved),
            );
            // The history of the lost session (A-18, B-11).
            doc.history.merge_unsaved(saved.history.clone());
            // Resumed lines are modifications until saved; the rescue files
            // stay until the document is closed normally.
            doc.dirty_memo.set(None);
            let mut line = self.i18n.t_args(
                "backup-resumed",
                &[
                    ("file", &m.file_name()),
                    ("count", &report.matched.to_string()),
                ],
            );
            if m.source_changed() || report.unmatched > 0 {
                line = self.i18n.dash(
                    &line,
                    &self.i18n.t_arg(
                        "backup-resume-unmatched",
                        "count",
                        &report.unmatched.to_string(),
                    ),
                );
            }
            notes.push(line);
        }
        if !notes.is_empty() {
            self.status = notes.last().cloned().unwrap_or_default();
            if notes.len() > 1 || missing {
                self.alert = Some(notes.join("\n"));
            }
        }
    }

    fn close_doc(&mut self, index: usize) {
        if self.edit_session.as_ref().is_some_and(|s| s.doc == index) {
            self.edit_session = None;
        }
        // The edit window belongs to the active document: it closes with it
        // (R-A02-01) and is not reopened by itself.
        if index == self.active && self.edit_win.take().is_some() {
            let _ = self.config.save();
        }
        if index < self.docs.len() {
            let name = self.docs[index].file_name();
            self.journal_for(
                crate::journal::Level::Info,
                crate::journal::Kind::Close,
                Some(&name),
                &self.i18n.t("journal-closed"),
            );
            // A normal close drops the rescue files (R-B11-04).
            let _ = crate::slow_io("autosave discard", || {
                self.autosave.discard(&self.docs[index].path)
            });
            self.docs.remove(index);
            if self.active >= self.docs.len() && self.active > 0 {
                self.active -= 1;
            }
            if self.docs.is_empty() {
                // Last tab closed: the four lists go back to their empty position.
                self.idle.clear();
            }
        }
    }

    fn save_as(&mut self, parent: &eframe::Frame) {
        let Some(doc) = self.active_doc() else {
            return;
        };
        // A text document proposes its target file first (R-C05-04); the
        // base formats stay available for a memory of the work.
        let text_target = doc
            .text_format()
            .and_then(|fmt| self.text_target_of(doc, &fmt));
        let mut dialog = rfd::FileDialog::new().set_parent(parent);
        let mut text_ext = String::new();
        if let Some(tp) = &text_target {
            text_ext = tp
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("txt")
                .to_lowercase();
            if let Some(dir) = tp.parent() {
                dialog = dialog.set_directory(dir);
            }
            dialog = dialog
                .set_file_name(
                    tp.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .add_filter(self.i18n.t("output-text"), &[text_ext.as_str()]);
        }
        let picked = dialog
            .add_filter(self.i18n.t("dialog-sst"), &["sst"])
            .add_filter(self.i18n.t("dialog-eet"), &["eet"])
            .add_filter(self.i18n.t("dialog-dsd-json"), &["dsd"])
            .add_filter(self.i18n.t("dialog-pivot-json"), &["json"])
            .add_filter(self.i18n.t("dialog-csv"), &["csv"])
            .add_filter(self.i18n.t("dialog-xml"), &["xml"])
            .add_filter(
                self.i18n.t("dialog-strings-format"),
                &["strings", "dlstrings", "ilstrings"],
            )
            .save_file();
        if let Some(path) = picked {
            let picked_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if text_target.is_some() && picked_ext == text_ext {
                let idx = self.active;
                let tm = self.docs[idx].tm.clone();
                match self.write_text_doc(idx, &tm, &path) {
                    Ok(n) => {
                        if let Some(d) = self.docs.get_mut(idx) {
                            d.mark_saved();
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        self.notify_ok(self.i18n.t_args(
                            "status-text-saved",
                            &[("file", &name), ("n", &n.to_string())],
                        ));
                        self.write_mod_dict(idx, false);
                        self.personal_autofill(idx);
                    }
                    Err(e) => self.notify_err(self.i18n.t_arg("err-save", "error", &e.to_string())),
                }
                return;
            }
            // The formats that carry a comment receive the generated text
            // when asked (R-C03-06, decision D-23): the author's comment is
            // written as is otherwise.
            let with_generated = self.config.export_generated_comments
                && crate::comments::format_carries_comment(&path);
            let generated: Option<TranslationMemory> = if with_generated {
                let map = self.dialog_map_blocking();
                let g = crate::comments::Generator {
                    map: map.as_deref(),
                    i18n: &self.i18n,
                    voices: self.dialogs.voices.clone(),
                };
                self.active_doc()
                    .map(|doc| crate::comments::with_generated(&doc.tm, &g))
            } else {
                None
            };
            let Some(doc) = self.active_doc() else {
                return;
            };
            // The pivot JSON carries the history (A-18).
            let with_history: Option<TranslationMemory> =
                (picked_ext == "json" && !doc.history.entries.is_empty()).then(|| {
                    let mut tm = generated.clone().unwrap_or_else(|| doc.tm.clone());
                    tm.history = doc.history.entries.clone();
                    tm
                });
            let to_write: &TranslationMemory = with_history
                .as_ref()
                .or(generated.as_ref())
                .unwrap_or(&doc.tm);
            match crate::slow_io("file write", || tm::save(to_write, &path)) {
                Ok(outcome) => {
                    self.last_save_dir = path.parent().map(|d| d.to_path_buf());
                    // The document is now the saved one (R-A01-05).
                    if let Some(d) = self.docs.get_mut(self.active) {
                        d.mark_saved();
                    }
                    self.notify_ok(self.i18n.t_arg(
                        "status-saved",
                        "what",
                        &describe_outcome(&self.i18n, &outcome, &path),
                    ));
                    // A format without the review says so (R-A17-07).
                    let reviewed = self
                        .active_doc()
                        .map(|d| crate::review::count(&d.tm.entries))
                        .unwrap_or(0);
                    if reviewed > 0
                        && !matches!(outcome, WriteOutcome::Json | WriteOutcome::Xml("ximod"))
                    {
                        self.notify_warn(self.i18n.t_arg(
                            "review-not-saved",
                            "n",
                            &crate::ui::export_dialog::fmt_count(&self.i18n, reviewed),
                        ));
                    }
                    let idx = self.active;
                    self.write_mod_dict(idx, false);
                    self.personal_autofill(idx);
                }
                Err(e) => self.notify_err(self.i18n.t_arg("err-save", "error", &e.to_string())),
            }
        }
    }

    /// Export the active document's translation as a game-ready output, chosen by
    /// the toolbar's `OutputMode`: full esp, override patch, STRINGS files, or DSD.
    /// Fichier ▸ Exporter la traduction… — first step: the « Lignes exportées »
    /// dialog (S-04). The export itself follows in [`Self::export_translation`].
    fn open_export_dialog(&mut self) {
        if self.active_doc().is_none() {
            self.status = self.i18n.t("status-no-doc");
            return;
        }
        // « Sortie » rules: an empty list is an alert; a filled one is
        // confirmed (« est-ce la bonne sortie ? ») before anything else.
        let Some(mode) = self.active_settings().output else {
            let msg = self.i18n.t("alert-output-missing");
            self.status = msg.clone();
            self.alert = Some(msg);
            return;
        };
        if !self.pending_output_confirm {
            self.pending_output_confirm = true;
            return;
        }
        self.pending_output_confirm = false;
        // Checks that do not depend on the lines, so the user is told before
        // the dialog rather than after.
        let (is_plugin, game, is_text, is_scripts) = {
            let doc = self.active_doc().unwrap();
            (
                doc.tm.meta.source_format == "plugin",
                Game::from_id(&doc.tm.meta.game),
                doc.is_text(),
                doc.scripts.is_some(),
            )
        };
        match mode {
            OutputMode::FullPlugin | OutputMode::OverridePatch if !is_plugin => {
                self.status = self.i18n.t("status-export-needs-plugin");
                return;
            }
            OutputMode::Hybrid if !is_plugin => {
                self.status = self.i18n.t("status-hybrid-needs-plugin");
                return;
            }
            // An output the game cannot receive (a sheet changed since).
            m if !m.fits(game) => {
                self.status = self.output_unfit_message(m, game);
                return;
            }
            OutputMode::Text if !is_text => {
                self.status = self.i18n.t("status-export-needs-text");
                return;
            }
            m if is_text && m != OutputMode::Text => {
                self.status = self.i18n.t("status-text-only-text");
                return;
            }
            // A « scripts » document has its scripts alone (C-06).
            OutputMode::Scripts if !is_scripts => {
                self.status = self.i18n.t("status-export-needs-scripts");
                return;
            }
            m if is_scripts && m != OutputMode::Scripts => {
                self.status = self.i18n.t("status-scripts-only-scripts");
                return;
            }
            _ => {}
        }
        self.pending_export = Some((crate::export::ExportPolicy::validated_only(), true));
        self.export_enc_logged = false;
    }

    /// The « Lignes exportées » dialog (S-04, figure B.1): the rule, the
    /// optional statuses to include (unchecked at each opening), the warning
    /// once one is checked, and the live count. Exporter runs the export with
    /// that policy; Annuler / Escape close it.
    /// « Avez-vous choisi la bonne sortie ? » — asked by Fichier ▸ Exporter…
    /// while the « Sortie » list holds a mode. Oui continues to the export
    /// dialog, Non / Escape stop there.
    fn output_confirm_window(&mut self, ctx: &egui::Context) {
        if !self.pending_output_confirm {
            return;
        }
        let Some(mode) = self.active_settings().output else {
            self.pending_output_confirm = false;
            return;
        };
        let mut msg = self
            .i18n
            .t_arg("confirm-output-body", "mode", &self.output_label(mode));
        if mode == OutputMode::FullPlugin {
            // A rewritten original is the wrong way to publish someone else's
            // mod: mod authors expect an override plugin.
            msg.push_str("\n\n");
            msg.push_str(&self.i18n.t("confirm-output-full-plugin-hint"));
        }
        let title = self.i18n.t("confirm-output-title");
        let oui = self.i18n.t("btn-yes");
        let non = self.i18n.t("btn-no");
        let mut choice: Option<bool> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(4.0);
                ui.add(egui::Label::new(msg).wrap_mode(egui::TextWrapMode::Wrap));
                ui.add_space(12.0);
                let (p, c) = confirm_buttons(ui, &oui, &non);
                if p {
                    choice = Some(true);
                } else if c {
                    choice = Some(false);
                }
            });
        match choice {
            Some(true) => self.open_export_dialog(), // the pending flag lets it through
            Some(false) => self.pending_output_confirm = false,
            None => {}
        }
    }

    fn export_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some((mut policy, mut esl)) = self.pending_export else {
            return;
        };
        let Some(doc) = self.active_doc() else {
            self.pending_export = None;
            return;
        };
        let counts = policy.counts(&doc.tm);
        let Some(mode) = self.active_settings().output else {
            self.pending_export = None;
            return;
        };
        let mode_name = self.output_label(mode);
        let title = self.i18n.dash(&self.i18n.t("export-title"), &mode_name);
        // Override patch of a localized plugin (S-01): what will be written,
        // computed on the filtered memory (unresolved ids, records, ESL).
        let override_info =
            if mode == OutputMode::OverridePatch && doc.tm.meta.source_format == "plugin" {
                let localized = doc.tm.entries.iter().any(|e| e.idstexte >= 0);
                let plan = std::fs::read(&doc.path).ok().and_then(|src| {
                    let filtered = policy.filtered(&doc.tm);
                    let opts = crate::plugin::OverrideOptions {
                        bundle: doc.strings.as_ref(),
                        esl_if_eligible: esl,
                    };
                    crate::plugin::plan_override(&src, &filtered, &opts).ok()
                });
                Some(crate::ui::export_dialog::OverrideInfo {
                    localized,
                    game: doc.settings.game_label(),
                    records: plan.as_ref().map(|p| p.records).unwrap_or(0),
                    unresolved: plan.as_ref().map(|p| p.unresolved.len()).unwrap_or(0),
                    esl_eligible: plan.as_ref().map(|p| p.esl_eligible).unwrap_or(false),
                })
            } else {
                None
            };
        let is_text = doc.is_text();
        let overwrite_note = (mode == OutputMode::FullPlugin
            && doc.tm.meta.source_format == "plugin")
            .then(|| self.i18n.t("export-overwrite-self"));
        let mut copy = self.config.export_untranslated_copy;
        // Lines the page of writing cannot hold (D-09).
        let out_page = self.out_page(doc);
        let bad = Self::unencodable_lines(&policy.filtered(&doc.tm), out_page);
        let encoding_bad = (bad > 0).then(|| {
            self.i18n.t_args(
                "export-encoding-refused",
                &[
                    ("page", &self.i18n.t(out_page.name_key())),
                    ("n", &bad.to_string()),
                ],
            )
        });
        // Recorded once per dialog, with its lines (R-E01-04).
        if let Some(msg) = &encoding_bad
            && !self.export_enc_logged
        {
            let rows = Self::unencodable_rows(&policy.filtered(&doc.tm), out_page);
            let (msg, idx) = (msg.clone(), self.active);
            self.export_enc_logged = true;
            self.journal_rows(
                crate::journal::Level::Error,
                crate::journal::Kind::Export,
                idx,
                &msg,
                rows,
            );
        }
        // A plugin to localize (C-09): the files it will give.
        let localize_lines = if mode == OutputMode::Strings {
            self.localize_lines(&policy)
        } else {
            None
        };
        let localize_texts = (self.i18n.t("localize-intro"), self.i18n.t("localize-files"));
        // Exported VMAD lines that STRINGS or DSD do not carry (C-08).
        let vmad_note = {
            let doc = self.active_doc();
            let unwritten = matches!(
                mode,
                OutputMode::Strings | OutputMode::Hybrid | OutputMode::Dsd
            ) && localize_lines.is_none();
            let n = doc
                .filter(|_| unwritten)
                .map(|d| {
                    policy
                        .filtered(&d.tm)
                        .entries
                        .iter()
                        .filter(|e| e.field_sig == crate::vmad_lock::FIELD)
                        .count()
                })
                .unwrap_or(0);
            (n > 0).then(|| {
                self.i18n
                    .t_arg("export-vmad-not-written", "n", &n.to_string())
            })
        };
        let mut source_too = self.config.localize_source_strings;
        let mut outcome = crate::ui::export_dialog::Outcome::Open;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(crate::ui::export_dialog::WIDTH)
            .show(ctx, |ui| {
                outcome = crate::ui::export_dialog::body(
                    ui,
                    &self.i18n,
                    &mut policy,
                    &counts,
                    override_info.as_ref(),
                    &mut esl,
                    crate::ui::export_dialog::Extra {
                        untranslated_copy: if is_text { Some(&mut copy) } else { None },
                        overwrite_note: overwrite_note.as_deref(),
                        encoding_bad: encoding_bad.as_deref(),
                        vmad_note: vmad_note.as_deref(),
                        localize: localize_lines.as_ref().map(|l| {
                            crate::ui::export_dialog::LocalizeInfo {
                                intro: &localize_texts.0,
                                files_title: &localize_texts.1,
                                files: &l.files,
                                source_label: &l.source_label,
                                source_too: &mut source_too,
                                table: &l.table,
                                unknown: l.unknown.as_deref(),
                            }
                        }),
                    },
                );
            });
        if copy != self.config.export_untranslated_copy {
            self.config.export_untranslated_copy = copy;
            let _ = self.config.save();
        }
        if source_too != self.config.localize_source_strings {
            self.config.localize_source_strings = source_too;
            let _ = self.config.save();
            // The list of files changes.
            self.localize_cache = Default::default();
        }
        use crate::ui::export_dialog::Outcome;
        if outcome == Outcome::Cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_export = None;
        } else if outcome == Outcome::Export {
            self.pending_export = None;
            // Everything written goes in the document's page (R-D09-03).
            crate::tm::encoding::with_output(Some(out_page), || {
                self.export_translation(frame, policy, counts, esl)
            });
        } else {
            self.pending_export = Some((policy, esl));
        }
    }

    /// Second step of Fichier ▸ Exporter la traduction… : write the output of
    /// the current mode from the memory filtered by `policy` (S-04), through
    /// the system save dialog as before.
    /// The backups/ folder of the last export, or of the active document's
    /// folder, when it exists (E-06).
    fn export_backups_dir(&self) -> Option<PathBuf> {
        let dir = self.last_export_dir.clone().or_else(|| {
            self.active_doc()
                .and_then(|d| d.path.parent().map(PathBuf::from))
        })?;
        Some(dir.join("backups")).filter(|d| d.is_dir())
    }

    /// Whether an export to `path` overwrites the open document's own file
    /// (E-06: the original is then always kept).
    fn export_overwrites_open(&self, path: &std::path::Path) -> bool {
        self.active_doc()
            .is_some_and(|d| crate::backup::same_file(path, &d.path))
    }

    /// An export output written with its original kept (E-06); the error
    /// text when nothing was written.
    fn export_write(
        &mut self,
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Result<Option<PathBuf>, String> {
        let forced = self.export_overwrites_open(path);
        // The script blocks read for the Script tab are those of the file
        // as it was (C-07).
        self.sctx_cache.replace(None);
        let res = crate::backup::write_export(
            path,
            bytes,
            self.config.keep_original,
            forced,
            self.config.original_copies,
        );
        if let Some(dir) = path.parent() {
            self.last_export_dir = Some(dir.to_path_buf());
        }
        match res {
            Ok(kept) => {
                if let Some(k) = &kept {
                    tracing::info!("export: original kept as {}", k.display());
                }
                let text = self.i18n.t_args(
                    "journal-export-file",
                    &[
                        ("path", &path.display().to_string()),
                        ("size", &self.i18n.size(bytes.len() as u64)),
                    ],
                );
                self.journal_add(
                    crate::journal::Level::Info,
                    crate::journal::Kind::Export,
                    &text,
                );
                Ok(kept)
            }
            Err(crate::backup::ExportWriteError::Keep(e)) => {
                tracing::warn!("export: original not kept ({e}), nothing written");
                Err(self
                    .i18n
                    .t_arg("export-keep-failed", "error", &e.to_string()))
            }
            Err(crate::backup::ExportWriteError::Write(e)) => {
                Err(self.i18n.t_arg("err-generic", "error", &e.to_string()))
            }
        }
    }

    /// The copy before a writer that writes the file itself (a text file).
    fn export_keep_before(&mut self, path: &std::path::Path) -> Result<Option<PathBuf>, String> {
        let forced = self.export_overwrites_open(path);
        if let Some(dir) = path.parent() {
            self.last_export_dir = Some(dir.to_path_buf());
        }
        crate::backup::keep_before_export(path, self.config.keep_original, forced).map_err(|e| {
            self.i18n
                .t_arg("export-keep-failed", "error", &e.to_string())
        })
    }

    /// « Original conservé : backups/MonMod_20261025-161203.esp ».
    fn kept_note(&self, kept: &Option<PathBuf>) -> Option<String> {
        self.kept_list_note(kept.as_slice())
    }

    fn kept_list_note(&self, kept: &[PathBuf]) -> Option<String> {
        if kept.is_empty() {
            return None;
        }
        let names: Vec<String> = kept
            .iter()
            .map(|k| {
                format!(
                    "backups/{}",
                    k.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                )
            })
            .collect();
        Some(
            self.i18n
                .t_arg("export-kept-original", "file", &names.join(", ")),
        )
    }

    fn export_translation(
        &mut self,
        parent: &eframe::Frame,
        policy: crate::export::ExportPolicy,
        counts: crate::export::ExportCounts,
        esl_if_eligible: bool,
    ) {
        let Some(doc) = self.active_doc() else {
            self.status = self.i18n.t("status-no-doc");
            return;
        };
        let Some(mode) = self.active_settings().output else {
            self.status = self.i18n.t("alert-output-missing");
            return;
        };
        let game = Game::from_id(&doc.tm.meta.game);
        let is_plugin = doc.tm.meta.source_format == "plugin";
        // What the writers see: excluded lines carry no translation (R-S04-02).
        let filtered = policy.filtered(&doc.tm);
        let tm: &TranslationMemory = &filtered;
        let exported_line = self.i18n.t_arg(
            "status-export-lines",
            "count",
            &crate::ui::export_dialog::fmt_count(&self.i18n, counts.total),
        );
        // Plugins carry a game (→ its code page); a loose STRINGS document has no
        // game, and those files are UTF-8 by convention (SSE era).
        let utf8 = if is_plugin { game.uses_utf8() } else { true };
        let stem = doc
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let plugin_name = doc.file_name();

        match mode {
            OutputMode::FullPlugin | OutputMode::OverridePatch => {
                if !is_plugin {
                    self.status = self.i18n.t("status-export-needs-plugin");
                    return;
                }
                let src = match std::fs::read(&doc.path) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = self
                            .i18n
                            .colon(&self.i18n.t("status-export-read-error"), &e.to_string());
                        return;
                    }
                };
                // FullPlugin routes through the delocalizer: for a localized
                // plugin it inlines the STRINGS into a standalone esp; for a
                // non-localized one it is a plain full translated esp.
                let mut deloc_note: Option<String> = None;
                let result = if mode == OutputMode::FullPlugin {
                    crate::plugin::write_delocalized_esp(&src, tm, doc.strings.as_ref()).map(
                        |(b, rep)| {
                            if !rep.is_complete() {
                                deloc_note = Some(self.i18n.t_args(
                                    "status-deloc-incomplete",
                                    &[
                                        ("inlined", &rep.extra_inlined.to_string()),
                                        ("unresolved", &rep.unresolved.len().to_string()),
                                    ],
                                ));
                            }
                            b
                        },
                    )
                } else {
                    // S-01: a localized original yields a delocalized patch,
                    // the light flag is set when asked and eligible.
                    let opts = crate::plugin::OverrideOptions {
                        bundle: doc.strings.as_ref(),
                        esl_if_eligible,
                    };
                    crate::plugin::write_override_esp_with(&src, tm, &plugin_name, &opts).map(
                        |(b, rep)| {
                            let mut notes: Vec<String> = Vec::new();
                            if rep.localized {
                                notes.push(self.i18n.t_arg(
                                    "export-override-delocalized",
                                    "count",
                                    &rep.records.to_string(),
                                ));
                            }
                            if !rep.unresolved.is_empty() {
                                notes.push(self.i18n.t_arg(
                                    "export-override-unresolved",
                                    "count",
                                    &rep.unresolved.len().to_string(),
                                ));
                            }
                            if rep.esl {
                                notes.push(self.i18n.t("export-override-esl"));
                            }
                            if !notes.is_empty() {
                                deloc_note = Some(self.i18n.list(&notes));
                            }
                            b
                        },
                    )
                };
                let bytes = match result {
                    Ok(b) => b,
                    Err(e) => {
                        self.notify_err(
                            self.i18n
                                .colon(&self.i18n.t("status-export-error"), &e.to_string()),
                        );
                        return;
                    }
                };
                let default = if mode == OutputMode::OverridePatch {
                    format!("{stem}_patch.esp")
                } else {
                    plugin_name.clone()
                };
                let plugin_exts = crate::games::registry().all_plugin_extensions();
                // A full plugin is proposed in place of the open one (E-06:
                // its original is kept in backups/).
                let mut fd = rfd::FileDialog::new().set_parent(parent);
                if mode == OutputMode::FullPlugin
                    && let Some(dir) = doc.path.parent()
                {
                    fd = fd.set_directory(dir);
                }
                if let Some(path) = fd
                    .set_file_name(&default)
                    .add_filter(
                        self.i18n.t("dialog-plugin"),
                        &plugin_exts.iter().map(String::as_str).collect::<Vec<_>>(),
                    )
                    .save_file()
                {
                    match self.export_write(&path, &bytes) {
                        Ok(kept) => {
                            let mut msg = self.i18n.dash(
                                &self.i18n.t("status-export-ok"),
                                &self
                                    .i18n
                                    .paren(&exported_line, &self.i18n.size(bytes.len() as u64)),
                            );
                            if let Some(note) = &deloc_note {
                                msg = self.i18n.dash(&msg, note);
                            }
                            if let Some(note) = self.kept_note(&kept) {
                                msg = self.i18n.dash(&msg, &note);
                            }
                            self.notify_ok(msg);
                            let idx = self.active;
                            self.write_mod_dict(idx, false);
                            self.personal_autofill(idx);
                        }
                        Err(e) => self.notify_err(e),
                    }
                }
            }
            // The translated scripts of a « scripts » document (C-06).
            OutputMode::Scripts => {
                let owned: TranslationMemory = tm.clone();
                self.export_scripts(parent, &owned, &exported_line);
            }
            OutputMode::Strings if self.localize_table(doc).is_some() => {
                // A plugin whose texts are inline: localized (C-09).
                let owned: TranslationMemory = tm.clone();
                self.export_localized(parent, &owned, &exported_line);
            }
            OutputMode::Strings | OutputMode::Hybrid => {
                // Hybrid is the plugin-loaded workflow (records/fields for
                // context); Strings works from a plain STRINGS document too. Both
                // produce the same STRINGS output (CDC §4).
                if mode == OutputMode::Hybrid && !is_plugin {
                    self.status = self.i18n.t("status-hybrid-needs-plugin");
                    return;
                }
                let files = crate::plugin::build_strings_files(tm, utf8);
                if files.is_empty() {
                    self.status = self.i18n.t("status-export-no-strings");
                    return;
                }
                // File names follow the game's convention (R-S01-04):
                // `_French` for Skyrim / Fallout 4, `_fr` for Starfield.
                let lang = crate::lang::strings_suffix_for(game, &self.active_settings().target);
                if let Some(dir) = rfd::FileDialog::new().set_parent(parent).pick_folder() {
                    let mut written = 0usize;
                    let mut kept_all: Vec<PathBuf> = Vec::new();
                    let mut failed: Option<String> = None;
                    for (kind, bytes) in &files {
                        let name = format!("{stem}_{lang}.{}", kind.extension());
                        match self.export_write(&dir.join(&name), bytes) {
                            Ok(kept) => {
                                written += 1;
                                kept_all.extend(kept);
                            }
                            Err(e) => {
                                // A copy refused stops the export (R-E06-01).
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    let key = if mode == OutputMode::Hybrid {
                        "status-export-strings-hybrid"
                    } else {
                        "status-export-strings-ok"
                    };
                    let mut msg = self.i18n.dash(
                        &self.i18n.t_arg(key, "n", &written.to_string()),
                        &exported_line,
                    );
                    if let Some(note) = self.kept_list_note(&kept_all) {
                        msg = self.i18n.dash(&msg, &note);
                    }
                    match failed {
                        Some(e) => self.notify_err(e),
                        None if !kept_all.is_empty() => self.notify_ok(msg),
                        None => self.status = msg,
                    }
                    if written > 0 {
                        let idx = self.active;
                        self.write_mod_dict(idx, false);
                        self.personal_autofill(idx);
                    }
                }
            }
            OutputMode::Dsd => {
                if !game.supports_dsd() {
                    self.status = self.i18n.t("warn-dsd-skyrim-only");
                    return;
                }
                let (bytes, n) = crate::tm::dsd::to_json_bytes(tm);
                if n == 0 {
                    self.status = self.i18n.t("status-export-no-strings");
                    return;
                }
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(parent)
                    .set_file_name(format!("{stem}.json"))
                    .add_filter(self.i18n.t("dialog-dsd-json"), &["json"])
                    .save_file()
                {
                    match self.export_write(&path, &bytes) {
                        Ok(kept) => {
                            let mut msg = self.i18n.dash(
                                &self.i18n.t_arg("status-export-dsd-ok", "n", &n.to_string()),
                                &exported_line,
                            );
                            match self.kept_note(&kept) {
                                Some(note) => {
                                    msg = self.i18n.dash(&msg, &note);
                                    self.notify_ok(msg);
                                }
                                None => self.status = msg,
                            }
                            let idx = self.active;
                            self.write_mod_dict(idx, false);
                            self.personal_autofill(idx);
                        }
                        Err(e) => self.notify_err(e),
                    }
                }
            }
            // The text file of a text document (R-C05-04): `<base>_<langue
            // cible>.<ext>` next to the file, proposed in the save box.
            OutputMode::Text => {
                let Some(fmt) = doc.text_format() else {
                    self.status = self.i18n.t("status-export-needs-text");
                    return;
                };
                let proposed = self
                    .text_target_of(doc, &fmt)
                    .unwrap_or_else(|| doc.path.clone());
                let dir = proposed
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                let name = proposed
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let ext = proposed
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("txt")
                    .to_string();
                let filtered: TranslationMemory = tm.clone();
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(parent)
                    .set_directory(&dir)
                    .set_file_name(&name)
                    .add_filter(self.i18n.t("output-text"), &[ext.as_str()])
                    .save_file()
                {
                    let idx = self.active;
                    // The original kept first (R-E06-01): no copy, no write.
                    let kept = match self.export_keep_before(&path) {
                        Ok(k) => k,
                        Err(e) => {
                            self.notify_err(e);
                            return;
                        }
                    };
                    match self.write_text_doc(idx, &filtered, &path) {
                        Ok(n) => {
                            if kept.is_some() {
                                crate::backup::purge_originals(&path, self.config.original_copies);
                            }
                            let file = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let mut msg = self.i18n.dash(
                                &self.i18n.t_args(
                                    "status-export-text-ok",
                                    &[("file", &file), ("n", &n.to_string())],
                                ),
                                &exported_line,
                            );
                            match self.kept_note(&kept) {
                                Some(note) => {
                                    msg = self.i18n.dash(&msg, &note);
                                    self.notify_ok(msg);
                                }
                                None => self.status = msg,
                            }
                            self.write_mod_dict(idx, false);
                            self.personal_autofill(idx);
                        }
                        Err(e) => {
                            self.status = self.i18n.t_arg("err-generic", "error", &e.to_string())
                        }
                    }
                }
            }
        }
    }

    // ------------------------------ panels --------------------------------

    fn menu_bar(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let mut do_open = false;
        let mut do_save = false;
        let mut do_save_part = false;
        let mut do_encoding = false;
        let mut do_load_review = false;
        let mut do_export = false;
        let mut do_quit = false;
        let mut do_reload_bases = false;
        let mut do_compile_bases = false;
        let mut do_apply_dicts = false;
        let mut do_apply_assist = false;
        let mut recent_to_open: Option<PathBuf> = None;
        let mut dict_action: Option<DictAction> = None;
        let mut do_close_doc = false;
        let mut do_manual = false;
        let mut do_open_interface = false;
        let mut do_import_format = false;
        let mut do_scan = false;
        let has_doc = !self.docs.is_empty();

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button(self.i18n.t("menu-file"), |ui| {
                    fit_menu(ui);
                    // Opening, saving and the recent list need a game in the
                    // toolbar "Jeu" selector (it drives the dictionaries and the
                    // destination folders); until then they are greyed with a
                    // hint, like the Dictionaries menu.
                    let game_ok = self.game_selected();
                    let needs_game = self.i18n.t("menu-needs-game");
                    // « Tous les mods » saves and exports nothing: file by
                    // file (R-B25-04).
                    let all = self.all_active();
                    let ro_tip = self.i18n.t("all-mods-readonly");
                    let file_tip = if all { &ro_tip } else { &needs_game };
                    let sc = &MENU_SHORTCUTS;
                    // Opening needs no game chosen beforehand (D-95): the
                    // header of a plugin says its game; only a file whose
                    // game stays unknown asks for it.
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-open"),
                        Some(&shortcut_text(ctx, &sc.open, &self.i18n)),
                        true,
                        "",
                    ) {
                        do_open = true;
                    }
                    // The interface files found next to the plugin (R-C05-05).
                    let has_interface = self
                        .active_doc()
                        .is_some_and(|d| !d.interface_files.is_empty());
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-open-interface"),
                        None,
                        has_interface,
                        &self.i18n.t("menu-open-interface-none"),
                    ) {
                        do_open_interface = true;
                    }
                    // Importer avec un format… (D-08).
                    if menu_item(ui, &self.i18n.t("menu-import-format"), None, !all, &ro_tip) {
                        do_import_format = true;
                    }
                    // Analyser un dossier… (D-07).
                    if menu_item(ui, &self.i18n.t("menu-scan"), None, true, "") {
                        do_scan = true;
                    }
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-save"),
                        Some(&shortcut_text(ctx, &sc.save_as, &self.i18n)),
                        game_ok && has_doc && !all,
                        file_tip,
                    ) {
                        do_save = true;
                    }
                    // Enregistrer une partie… (B-10).
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-save-part"),
                        None,
                        game_ok && has_doc && !all,
                        file_tip,
                    ) {
                        do_save_part = true;
                    }
                    // Encodage du fichier… (D-09): the games in a page of code.
                    let page_doc = !all
                        && self
                            .active_doc()
                            .is_some_and(|d| app_encoding::page_game(&d.settings));
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-encoding"),
                        None,
                        page_doc,
                        &self.i18n.t("encoding-utf8-game"),
                    ) {
                        do_encoding = true;
                    }
                    // Charger dans la relecture… (A-17).
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-load-review"),
                        None,
                        has_doc && !all,
                        &ro_tip,
                    ) {
                        do_load_review = true;
                    }
                    ui.separator();
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-export"),
                        Some(&shortcut_text(ctx, &sc.export, &self.i18n)),
                        has_doc && !all,
                        &ro_tip,
                    ) {
                        do_export = true;
                    }
                    // Ouvrir le dossier des sauvegardes de l'export (E-06).
                    let export_backups = self.export_backups_dir();
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-open-export-backups"),
                        None,
                        export_backups.is_some(),
                        &self.i18n.t("menu-open-export-backups-none"),
                    ) && let Some(dir) = export_backups
                    {
                        crate::fonts::open_path(&dir);
                    }
                    ui.separator();
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-close-doc"),
                        Some(&shortcut_text(ctx, &sc.close_doc, &self.i18n)),
                        has_doc && !all,
                        &ro_tip,
                    ) {
                        do_close_doc = true;
                    }
                    ui.separator();
                    let mut child_rect = None;
                    if !self.config.recent_files.is_empty() {
                        let r = ui.menu_button(self.i18n.t("menu-recent"), |ui| {
                            fit_menu(ui);
                            for path in &self.config.recent_files {
                                let label = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("")
                                    .to_string();
                                if ui.button(label).clicked() {
                                    recent_to_open = Some(path.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.min_rect()
                        });
                        child_rect = r.inner;
                        ui.separator();
                    }
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-exit"),
                        Some(&shortcut_text(ctx, &sc.quit, &self.i18n)),
                        true,
                        "",
                    ) {
                        do_quit = true;
                    }
                    close_menu_if_pointer_left(ui, child_rect);
                });
                ui.menu_button(self.i18n.t("menu-options"), |ui| {
                    fit_menu(ui);
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-settings"),
                        Some(&shortcut_text(ctx, &MENU_SHORTCUTS.settings, &self.i18n)),
                        true,
                        "",
                    ) {
                        self.open_settings();
                        ui.close_menu();
                    }
                    // Base pays / langues (E-11, D-84).
                    if ui.button(self.i18n.t("menu-properties")).clicked() {
                        self.open_props();
                        ui.close_menu();
                    }
                    if ui.button(self.i18n.t("menu-status-colors")).clicked() {
                        self.show_status_colors = true;
                        self.status_colors_needs_center = true;
                        ui.close_menu();
                    }
                    if ui.button(self.i18n.t("menu-dictview-stripe")).clicked() {
                        // Seed the picker with the current effective stripe color:
                        // the custom one, or the theme's faint color when none.
                        let seed = if self.config.dictview_stripe_custom {
                            self.config.dictview_stripe
                        } else {
                            let c = ui.visuals().faint_bg_color;
                            (c.r(), c.g(), c.b())
                        };
                        self.color_picker
                            .open_for(super::color_picker::PickerTarget::DictStripe, seed);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .checkbox(
                            &mut self.config.show_special_chars,
                            self.i18n.t("opt-special-chars"),
                        )
                        .changed()
                    {
                        let _ = self.config.save();
                    }
                    // The optional columns (D-42): Commentaire (C-03,
                    // R-C03-01), Provenance (B-09)…
                    // The sub-menu's rect keeps the Options menu open while
                    // the pointer is on it (as the Dictionnaires sub-menus).
                    let columns_rect = ui
                        .menu_button(self.i18n.t("menu-columns"), |ui| {
                            fit_menu(ui);
                            for (i, (label, on)) in self.column_menu_items().into_iter().enumerate()
                            {
                                let mut v = on;
                                if ui.checkbox(&mut v, label).changed() {
                                    self.toggle_column(i);
                                }
                            }
                            ui.min_rect()
                        })
                        .inner;
                    // What the EET / XML writers receive (R-C03-06, D-23).
                    if ui
                        .checkbox(
                            &mut self.config.export_generated_comments,
                            self.i18n.t("menu-export-generated-comments"),
                        )
                        .changed()
                    {
                        let _ = self.config.save();
                    }
                    ui.separator();
                    if ui.button(self.i18n.t("menu-spell")).clicked() {
                        self.open_spell();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(self.i18n.t("menu-reload-bases")).clicked() {
                        do_reload_bases = true;
                        ui.close_menu();
                    }
                    if ui.button(self.i18n.t("menu-compile-bases")).clicked() {
                        do_compile_bases = true;
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_doc, egui::Button::new(self.i18n.t("menu-apply-dicts")))
                        .clicked()
                    {
                        do_apply_dicts = true;
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, columns_rect);
                });
                ui.menu_button(self.i18n.t("menu-dict"), |ui| {
                    fit_menu(ui);
                    // Every entry is disabled until a game is chosen in the toolbar
                    // "Jeu" selector (it decides the target sub-folder).
                    let game_ok = self.game_selected();
                    let child_rect = ui
                        .add_enabled_ui(game_ok, |ui| {
                            let conv = [
                                ("dict-eet-ximdb", ConvKind::EetToXimdb),
                                ("dict-sst-ximdb", ConvKind::SstToXimdb),
                                ("dict-ximdb-eet", ConvKind::XimdbToEet),
                                ("dict-ximdb-sst", ConvKind::XimdbToSst),
                            ];
                            // Keep the parent menu open while a nested
                            // "Unitaire/Par lot" sub-menu is open.
                            let eet_set = !self.config.eet_dir.trim().is_empty();
                            let xt_set = !self.config.xtranslator_dir.trim().is_empty();
                            let mut child_rect: Option<egui::Rect> = None;
                            for (key, kind) in conv {
                                // Disable a conversion when its tool folder is unset.
                                let enabled = if kind.is_eet() { eet_set } else { xt_set };
                                // Render the sub-menu button directly on the menu `ui`
                                // (no per-item `add_enabled_ui`): that keeps the menu's
                                // response covering every row, so egui closes the open
                                // sub-menu as soon as the pointer moves to another item.
                                if enabled {
                                    let inner = ui
                                        .menu_button(self.i18n.t(key), |ui| {
                                            fit_menu(ui);
                                            if ui.button(self.i18n.t("dict-unit")).clicked() {
                                                dict_action = Some(DictAction::ConvUnit(kind));
                                                ui.close_menu();
                                            }
                                            if ui.button(self.i18n.t("dict-batch")).clicked() {
                                                dict_action = Some(DictAction::ConvBatch(kind));
                                                ui.close_menu();
                                            }
                                            ui.min_rect()
                                        })
                                        .inner;
                                    if let Some(r) = inner {
                                        child_rect = Some(r);
                                    }
                                } else {
                                    ui.add_enabled(false, egui::Button::new(self.i18n.t(key)));
                                }
                            }
                            ui.separator();
                            if ui.button(self.i18n.t("dict-from-game")).clicked() {
                                dict_action = Some(DictAction::FromGame);
                                ui.close_menu();
                            }
                            if ui.button(self.i18n.t("dict-from-mod")).clicked() {
                                dict_action = Some(DictAction::FromMod);
                                ui.close_menu();
                            }
                            if ui.button(self.i18n.t("dict-from-strings")).clicked() {
                                dict_action = Some(DictAction::FromStrings);
                                ui.close_menu();
                            }
                            if ui.button(self.i18n.t("dict-from-plugins")).clicked() {
                                dict_action = Some(DictAction::FromPlugins);
                                ui.close_menu();
                            }
                            if ui.button(self.i18n.t("dict-from-locres")).clicked() {
                                dict_action = Some(DictAction::FromLocresCsv);
                                ui.close_menu();
                            }
                            if ui
                                .button(self.i18n.t("dict-from-text-pairs"))
                                .on_hover_text(self.i18n.t("dict-from-text-pairs-tip"))
                                .clicked()
                            {
                                dict_action = Some(DictAction::FromTextPairs);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(self.i18n.t("dict-export-game-strings")).clicked() {
                                dict_action = Some(DictAction::ExportGameStrings);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(self.i18n.t("games-detect")).clicked() {
                                dict_action = Some(DictAction::DetectGames);
                                ui.close_menu();
                            }
                            if ui.button(self.i18n.t("games-add-manual")).clicked() {
                                dict_action = Some(DictAction::AddGameManual);
                                ui.close_menu();
                            }
                            child_rect
                        })
                        .inner;
                    // The read-only viewer does not depend on a selected game, so
                    // it stays enabled even when the rest of the menu is greyed.
                    ui.separator();
                    if ui.button(self.i18n.t("dict-view")).clicked() {
                        dict_action = Some(DictAction::ViewDict);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_doc, egui::Button::new(self.i18n.t("dicts-menu")))
                        .clicked()
                    {
                        dict_action = Some(DictAction::ManageDicts);
                        ui.close_menu();
                    }
                    if ui
                        .button(self.i18n.t("dict-editor-menu-personal"))
                        .clicked()
                    {
                        dict_action = Some(DictAction::EditPersonal);
                        ui.close_menu();
                    }
                    if ui.button(self.i18n.t("dict-editor-menu-file")).clicked() {
                        dict_action = Some(DictAction::EditDict);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_doc, egui::Button::new(self.i18n.t("update-menu")))
                        .clicked()
                    {
                        dict_action = Some(DictAction::ModUpdate);
                        ui.close_menu();
                    }
                    if ui.button(self.i18n.t("menu-compare")).clicked() {
                        dict_action = Some(DictAction::Compare);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_doc, egui::Button::new(self.i18n.t("dict-save-mod")))
                        .on_hover_text(self.i18n.t("dict-save-mod-tip"))
                        .clicked()
                    {
                        dict_action = Some(DictAction::SaveModDict);
                        ui.close_menu();
                    }
                    // Appliquer les assistants… (B-15, D-80).
                    ui.separator();
                    if ui
                        .add_enabled(has_doc, egui::Button::new(self.i18n.t("assist-apply-menu")))
                        .clicked()
                    {
                        do_apply_assist = true;
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, child_rect);
                });
                self.tools_menu(ui);
                ui.menu_button(self.i18n.t("menu-help"), |ui| {
                    fit_menu(ui);
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-manual"),
                        Some(&shortcut_text(ctx, &MENU_SHORTCUTS.manual, &self.i18n)),
                        true,
                        "",
                    ) {
                        do_manual = true;
                    }
                    if menu_item(
                        ui,
                        &self.i18n.t("menu-journal"),
                        Some(&shortcut_text(ctx, &MENU_SHORTCUTS.journal, &self.i18n)),
                        true,
                        "",
                    ) {
                        self.open_journal_window();
                    }
                    ui.separator();
                    if ui.button(self.i18n.t("menu-about")).clicked() {
                        self.show_about = true;
                        self.about_needs_center = true;
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, None);
                });
            });
        });

        if do_open {
            crate::slow_io("open file", || self.open_file(frame));
        }
        if do_import_format {
            self.import_with_format();
        }
        if do_scan {
            self.open_scan_window();
        }
        if do_open_interface {
            self.open_interface_files();
        }
        if do_save {
            crate::slow_io("save as", || self.save_as(frame));
        }
        if do_save_part {
            self.open_savepart_dialog();
        }
        if do_encoding {
            self.open_encoding_dialog();
        }
        if do_load_review {
            self.open_review_dialog();
        }
        if do_export {
            self.open_export_dialog();
        }
        if let Some(path) = recent_to_open {
            self.load_path(path);
        }
        if do_reload_bases {
            self.reload_bases();
        }
        if do_compile_bases {
            self.compile_game_bases();
        }
        if let Some(action) = dict_action {
            self.run_dict_action(action);
        }
        if do_apply_dicts {
            self.open_apply_dialog(crate::replace::Scope::Document, false);
        }
        if do_apply_assist {
            self.open_assist_dialog(crate::replace::Scope::Document);
        }
        if do_close_doc {
            self.request_close_doc(self.active);
        }
        if do_manual {
            self.open_manual();
        }
        if do_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The global menu shortcuts (F-01), fired from the main window only when
    /// no modal dialog is open and no window of its own handles keys.
    fn handle_menu_shortcuts(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.modal_open() || self.show_settings || self.pending_save.is_some() {
            return;
        }
        let sc = &MENU_SHORTCUTS;
        let game_ok = self.game_selected();
        let has_doc = !self.docs.is_empty();
        // Ctrl+Tab / Ctrl+Maj+Tab: the tabs, « Tous les mods » included
        // (B-25); the more specific one first.
        let tab_back = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::Tab,
        );
        let tab_next = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Tab);
        if ctx.input_mut(|i| i.consume_shortcut(&tab_back)) {
            self.cycle_tab(true);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&tab_next)) {
            self.cycle_tab(false);
            return;
        }
        // The synthesis saves, exports and closes nothing (R-B25-04).
        let file_doc = has_doc && !self.all_active();
        // The more specific combination first (Ctrl+Maj+E before Ctrl+…).
        if ctx.input_mut(|i| i.consume_shortcut(&sc.export)) {
            if file_doc {
                self.open_export_dialog();
            }
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.open)) {
            self.open_file(frame);
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.save_as)) {
            if game_ok && file_doc {
                self.save_as(frame);
            }
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.close_doc)) {
            if file_doc {
                self.request_close_doc(self.active);
            }
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.settings)) {
            self.open_settings();
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.manual)) {
            self.open_manual();
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.quit)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.replace)) {
            self.open_replace_window(ctx);
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.quality)) {
            self.open_quality_window(true);
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.dialogues)) {
            self.open_dialog_window();
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.online_batch)) {
            self.open_online_dialog();
        } else if has_doc && ctx.input_mut(|i| i.consume_shortcut(&sc.review_accept)) {
            let targets = self.selection.targets();
            self.review_command(app_review::ReviewCmd::Accept, &targets);
        } else if has_doc && ctx.input_mut(|i| i.consume_shortcut(&sc.online_line)) {
            let targets = self.selection.targets();
            self.online_line(targets);
        } else if ctx.input_mut(|i| i.consume_shortcut(&sc.personal)) {
            let targets = self.selection.targets();
            if !targets.is_empty() {
                self.add_to_personal(&targets);
            } else {
                self.status = self.i18n.t("status-keys-no-selection");
            }
        } else if self.replace.open {
            // F3 / Maj+F3 from the main window too (R-A11-04).
            let (f3, sf3) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F3),
                    i.consume_key(egui::Modifiers::SHIFT, egui::Key::F3),
                )
            });
            if sf3 {
                self.replace_step(-1);
            } else if f3 {
                self.replace_step(1);
            }
        }
    }

    /// Aide ▸ Manuel (F1): the manual of the interface language shipped in
    /// `Manuals/` (next to the executable, or at the repository root), or
    /// the English one; a missing manual is said in an alert.
    fn open_manual(&mut self) {
        match crate::manual::find_manual(&self.config.locale) {
            Some(path) => {
                crate::fonts::open_url(&path.to_string_lossy());
                self.status = self.i18n.t_arg(
                    "manual-opened",
                    "file",
                    &path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            None => self.alert_msg("manual-missing"),
        }
    }

    /// Close a document from the menu or Ctrl+W (the tab's cross does the
    /// same): asks first when it has unsaved changes (F-06).
    fn request_close_doc(&mut self, index: usize) {
        // « Tous les mods » goes with the last but one file (R-B25-07).
        if self.docs.get(index).is_some_and(|d| d.all.is_some()) {
            return;
        }
        if self.docs.get(index).is_some_and(|d| d.is_dirty()) {
            self.close_prompt = Some(index);
        } else {
            self.close_doc(index);
        }
    }

    /// « Le document a des modifications non enregistrées » (F-06):
    /// Enregistrer sous… (primary, Entrée), Fermer sans enregistrer, Annuler
    /// (Échap). Also the exit prompt, which lists every modified document and
    /// saves them one by one.
    fn close_prompt_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let exit = self.exit_prompt;
        let Some(index) = (if exit {
            self.docs.iter().position(|d| d.is_dirty())
        } else {
            self.close_prompt
        }) else {
            if exit {
                // Nothing dirty any more: leave.
                self.exit_prompt = false;
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        };
        let Some(doc) = self.docs.get(index) else {
            self.close_prompt = None;
            self.exit_prompt = false;
            return;
        };
        if !doc.is_dirty() && !exit {
            self.close_prompt = None;
            self.close_doc(index);
            return;
        }
        let title = self.i18n.t(if exit {
            "exit-title"
        } else {
            "close-doc-title"
        });
        let names: Vec<String> = self
            .docs
            .iter()
            .filter(|d| d.is_dirty())
            .map(|d| d.file_name())
            .collect();
        let msg = if exit {
            self.i18n.t_arg("exit-unsaved", "names", &names.join(", "))
        } else {
            self.i18n
                .t_arg("close-doc-unsaved", "file", &doc.file_name())
        };
        let save_l = self.i18n.t("menu-save");
        let discard_l = self.i18n.t(if exit {
            "btn-quit-discard"
        } else {
            "btn-close-discard"
        });
        let cancel_l = self.i18n.t("btn-cancel");
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.add_space(4.0);
                ui.add(egui::Label::new(msg).wrap_mode(egui::TextWrapMode::Wrap));
                ui.add_space(12.0);
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new(&save_l).strong()).clicked() || enter {
                        save = true;
                    }
                    if ui.button(&discard_l).clicked() {
                        discard = true;
                    }
                    if ui.button(&cancel_l).clicked() || esc {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.close_prompt = None;
            self.exit_prompt = false;
        } else if save {
            // Enregistrer sous… on that document; when it succeeded the
            // document is clean and the prompt moves on (or closes it).
            self.active = index;
            self.save_as(frame);
            if self.docs.get(index).is_some_and(|d| !d.is_dirty()) && !exit {
                self.close_prompt = None;
                self.close_doc(index);
            }
        } else if discard {
            if exit {
                self.exit_prompt = false;
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.close_prompt = None;
                self.close_doc(index);
            }
        }
    }

    fn open_settings(&mut self) {
        self.temp_locale = self.config.locale.clone();
        self.temp_country = self.config.country.clone();
        self.temp_theme = self.config.theme;
        self.temp_font_size = self.config.font_size;
        self.temp_max_recent_files = self.config.max_recent_files;
        self.temp_window_width = self.config.window_width;
        self.temp_window_height = self.config.window_height;
        self.temp_game_bases_dir = self.config.game_bases_dir.clone();
        self.temp_mod_bases_dir = self.config.mod_bases_dir.clone();
        self.temp_eet_dir = self.config.eet_dir.clone();
        self.temp_xtranslator_dir = self.config.xtranslator_dir.clone();
        self.temp_config_mode = crate::config::paths::get().mode;
        self.temp_undo_depth = self.config.undo_depth;
        self.temp_history_enabled = self.config.history_enabled;
        self.temp_history_max = self.config.history_max;
        self.temp_autosave_enabled = self.config.autosave_enabled;
        self.temp_toasts = self.config.toasts;
        self.temp_autosave_minutes = self.config.autosave_minutes;
        self.temp_autosave_dir = self.config.autosave_dir.clone();
        self.temp_backup_copies = self.config.backup_copies;
        self.temp_keep_original = self.config.keep_original;
        self.temp_check_crc = self.config.check_crc;
        self.temp_original_copies = self.config.original_copies;
        self.temp_mod_dict_auto = self.config.mod_dict_auto;
        self.temp_personal_autofill = self.config.personal_autofill;
        self.temp_personal_keep_field = self.config.personal_keep_field;
        self.temp_old_data_months = self.config.old_data_months;
        self.temp_fuzzy_threshold = self.config.fuzzy_threshold;
        self.temp_normalize = self.config.normalize;
        self.temp_propose_generalization = self.config.propose_generalization;
        self.temp_propose_propagation = self.config.propose_propagation;
        self.temp_propagate_files = self.config.propagate_files;
        self.temp_online = self.config.online.clone();
        self.online_load_keys();
        self.online.show_key = false;
        self.online.test_result.clear();
        self.settings_tab = SettingsTab::General;
        self.settings_focus = Focus::TabGeneral;
        self.settings_focus_dirty = true;
        self.show_settings = true;
        self.settings_needs_center = true;
    }

    /// The toolbar's four lists (E-12): they show and edit the active
    /// document's settings, or the session defaults when nothing is open; the
    /// right-hand label says which (R-E12-02). Each change is applied to that
    /// document only.
    fn toolbar(&mut self, ctx: &egui::Context) {
        self.output_fit_game();
        let has_doc = self.active_doc().is_some();
        let old = self.active_settings();
        // What the lists edit: the document's settings, or the idle lists.
        let old_idle = match self.active_doc() {
            Some(d) => IdleSettings {
                game: d.settings.game.clone(),
                source: d.settings.source.clone(),
                target: d.settings.target.clone(),
                output: d.settings.output,
            },
            None => self.idle.clone(),
        };
        let mut cur = old_idle.clone();

        // Game selector: detected games + games that already have a dictionary
        // folder under assets/bases/jeux. A leading empty option means "no game
        // chosen" (which greys the File and Dictionnaires menus).
        let game_ids = self.selectable_games();
        let game_opts: Vec<(String, String)> = game_ids
            .iter()
            .map(|id| {
                let g = Game::from_id(id);
                let label = if g == Game::Unknown {
                    id.clone()
                } else {
                    g.display_name().to_string()
                };
                (id.clone(), label)
            })
            .collect();
        let lbl_game = self.i18n.t("toolbar-game");
        let sel_game_label = DocSettings {
            game: cur.game.clone(),
            ..old.clone()
        }
        .game_label();
        let sel_output_label = cur
            .output
            .map(|m| self.output_label(m))
            .unwrap_or_else(|| " ".to_string());
        // « Source »: only the languages the active file provides (nothing
        // without a document). « Cible »: the official languages of the country
        // picked with the Cible button, plus the document's current target.
        let source_opts: Vec<String> = self
            .active_doc()
            .map(|d| d.source_langs.clone())
            .unwrap_or_default();
        let mut target_opts: Vec<String> = self.target_langs.clone();
        if !cur.target.is_empty() && !target_opts.contains(&cur.target) {
            target_opts.insert(0, cur.target.clone());
        }
        let scope = match self.active_doc() {
            // « Tous les mods »: the file of the current line (B-25).
            Some(d) if d.all.is_some() => {
                let file = self
                    .selection
                    .cursor
                    .and_then(|l| self.all_origin(l))
                    .and_then(|(i, _)| self.docs.get(i))
                    .or_else(|| self.docs.first())
                    .map(|d| d.file_name())
                    .unwrap_or_default();
                self.i18n.t_arg("toolbar-scope-doc", "file", &file)
            }
            Some(d) => self.i18n.t_arg("toolbar-scope-doc", "file", &d.file_name()),
            None => self.i18n.t("toolbar-scope-default"),
        };

        let mut undo_clicked = false;
        let mut redo_clicked = false;
        let mut target_clicked = false;
        // « Tous les mods »: the settings of the current line's file, read
        // only (R-B25-04).
        let readonly = self.all_active();
        let ro_tip = self.i18n.t("all-mods-readonly");
        // The row is 8 px taller than egui's default panel (the undo / redo
        // buttons were clipped by the bottom separator; 2 px less since
        // 25/10/2026, the filter bar's rows being brought to the same height).
        let mut frame = egui::Frame::side_top_panel(&ctx.style());
        frame.inner_margin.top += TOOLBAR_EXTRA;
        frame.inner_margin.bottom += TOOLBAR_EXTRA;
        // A wrapped row is as tall as the first: the gap is twice the margin.
        let row_gap = frame.inner_margin.top + frame.inner_margin.bottom;
        egui::TopBottomPanel::top("toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = row_gap;
                ui.horizontal_wrapped(|ui| {
                    // Labels never flow: a label at the end of a row would start its
                    // text there and continue on the next one, and that row would be
                    // placed without the row gap, over the first (reported 25/10/2026).
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    keep_together(ui, lbl_game.as_str(), combo_w(ui, None, &sel_game_label));
                    ui.label(lbl_game.as_str());
                    ui.add_enabled_ui(!readonly, |ui| {
                        egui::ComboBox::from_id_salt("game_sel")
                            .selected_text(sel_game_label.as_str())
                            .show_ui(ui, |ui| {
                                // Empty position (no game selected).
                                ui.selectable_value(&mut cur.game, String::new(), " ");
                                for (id, label) in &game_opts {
                                    ui.selectable_value(&mut cur.game, id.clone(), label.as_str());
                                }
                            });
                    })
                    .response
                    .on_disabled_hover_text(&ro_tip);
                    vsep(ui);
                    let src_l = self.i18n.t("toolbar-source");
                    keep_together(
                        ui,
                        &src_l,
                        combo_w(ui, None, &self.i18n.display_name(&cur.source)),
                    );
                    ui.label(src_l);
                    ui.add_enabled_ui(has_doc && !readonly && !source_opts.is_empty(), |ui| {
                        lang_combo(
                            ui,
                            &self.i18n,
                            "src_lang",
                            &mut cur.source,
                            &source_opts,
                            false,
                        );
                    });
                    vsep(ui);
                    // « Cible » is a button: it opens the flags window, whose
                    // country fills the list with its official languages.
                    let cible_l = self.i18n.t("toolbar-target-btn");
                    keep_together(
                        ui,
                        &cible_l,
                        combo_w(ui, None, &self.i18n.display_name(&cur.target)) + 16.0,
                    );
                    if ui
                        .add_enabled(!readonly, egui::Button::new(cible_l))
                        .on_hover_text(self.i18n.t("toolbar-target-tip"))
                        .on_disabled_hover_text(&ro_tip)
                        .clicked()
                    {
                        target_clicked = true;
                    }
                    ui.add_enabled_ui(!readonly, |ui| {
                        lang_combo(
                            ui,
                            &self.i18n,
                            "dst_lang",
                            &mut cur.target,
                            &target_opts,
                            true,
                        );
                    });
                    vsep(ui);
                    let out_l = self.i18n.t("toolbar-output");
                    keep_together(ui, &out_l, combo_w(ui, None, &sel_output_label));
                    ui.label(out_l);
                    ui.add_enabled_ui(!readonly, |ui| {
                        egui::ComboBox::from_id_salt("output_mode")
                            .selected_text(sel_output_label)
                            .show_ui(ui, |ui| {
                                // Empty position (no output chosen), like the game list.
                                ui.selectable_value(&mut cur.output, None, " ");
                                let is_text = self.active_doc().is_some_and(Doc::is_text);
                                let is_scripts =
                                    self.active_doc().is_some_and(|d| d.scripts.is_some());
                                // The outputs the game cannot receive are
                                // shown greyed, not selectable.
                                let game = Game::from_id(cur.game.trim());
                                for m in OutputMode::variants_for(is_text, is_scripts) {
                                    if m.fits(game) {
                                        ui.selectable_value(
                                            &mut cur.output,
                                            Some(*m),
                                            self.output_label(*m),
                                        );
                                    } else {
                                        ui.add_enabled(
                                            false,
                                            egui::SelectableLabel::new(
                                                false,
                                                self.output_label(*m),
                                            ),
                                        )
                                        .on_disabled_hover_text(self.i18n.t_arg(
                                            "output-not-for-game",
                                            "game",
                                            game.display_name(),
                                        ));
                                    }
                                }
                            });
                    });
                    // ↶ Annuler / ↷ Rétablir (A-01), greyed when their stack is empty.
                    vsep(ui);
                    let (can_undo, can_redo, undo_tip, redo_tip) = match self.active_doc() {
                        // The synthesis undoes through its files (R-B25-05).
                        Some(d) if d.all.is_some() => (
                            self.all_mods_peek(false).is_some(),
                            self.all_mods_peek(true).is_some(),
                            self.undo_tip("undo-tip", self.all_mods_peek(false)),
                            self.undo_tip("redo-tip", self.all_mods_peek(true)),
                        ),
                        Some(d) => (
                            d.undo.can_undo(),
                            d.undo.can_redo(),
                            self.undo_tip("undo-tip", d.undo.peek_undo()),
                            self.undo_tip("redo-tip", d.undo.peek_redo()),
                        ),
                        None => (
                            false,
                            false,
                            self.i18n.t("undo-none"),
                            self.i18n.t("redo-none"),
                        ),
                    };
                    if ui
                        .add_enabled(can_undo, egui::Button::new(crate::icons::UNDO))
                        .on_hover_text(undo_tip)
                        .clicked()
                    {
                        undo_clicked = true;
                    }
                    if ui
                        .add_enabled(can_redo, egui::Button::new(crate::icons::REDO))
                        .on_hover_text(redo_tip)
                        .clicked()
                    {
                        redo_clicked = true;
                    }
                    vsep(ui);
                    ui.add(egui::Label::new(egui::RichText::new(&scope).weak()).truncate());
                });
            });

        if undo_clicked {
            self.do_undo();
        }
        if redo_clicked {
            self.do_redo();
        }
        if target_clicked {
            self.flags_purpose = FlagsPurpose::Target;
            self.open_flags();
        }
        if cur == old_idle {
            return;
        }
        if !has_doc {
            // No document: the lists are the settings of the next file; the
            // filled ones are also remembered as the session defaults.
            cur.store_defaults(&mut self.config);
            self.idle = cur;
            return;
        }
        let cur = DocSettings {
            game: cur.game,
            source: cur.source,
            target: cur.target,
            output: cur.output,
            use_personal: old.use_personal,
        };
        let idx = self.active;
        // Source: one of the languages the file provides (R-E12-04); the STRINGS
        // of a localized plugin are re-resolved in that language.
        if cur.source != old.source {
            // A text document: the sibling file of that language becomes
            // the document (R-C05-03), when there is one and nothing is lost.
            if self.docs[idx].is_text() && self.switch_text_source(idx, &cur.source) {
                return;
            }
            let doc = &mut self.docs[idx];
            doc.settings.source = cur.source.clone();
            if doc.tm.entries.iter().any(|e| e.idstexte >= 0) {
                let path = doc.path.clone();
                let (n, bundle) = Self::resolve_strings_for(&path, &mut doc.tm, &cur.source);
                if n > 0 {
                    doc.strings = bundle;
                }
            }
            self.restack_doc(idx);
        }
        // Target (R-E12-05): the pair's index (cache) completes the empty lines.
        if cur.target != old.target {
            self.docs[idx].settings.target = cur.target.clone();
            self.target_changed(idx);
        }
        // Output: the document's export mode only.
        if cur.output != old.output {
            self.docs[idx].settings.output = cur.output;
            let mode = cur
                .output
                .map(|m| self.i18n.t(output_mode_key(m)))
                .unwrap_or_else(|| "—".to_string());
            let text = self.i18n.t_arg("journal-output", "mode", &mode);
            self.journal_add(
                crate::journal::Level::Info,
                crate::journal::Kind::Config,
                &text,
            );
        }
        // Game (R-E12-03): reload as the new game, after confirmation when the
        // document was modified.
        if cur.game != old.game {
            if self.docs[idx].is_dirty() {
                self.pending_reload = Some((idx, cur.game.clone()));
            } else {
                self.reload_doc_as(idx, cur.game.clone());
            }
        }
    }

    /// Confirmation shown when the user picks a different game while documents are
    /// open: offers to save the files and their dictionaries before switching.
    /// **Oui** exports each file (per the output mode) + writes a mod dictionary,
    /// then closes all files and switches; **Non** switches without saving;
    /// **Annuler** keeps the current game and files. In-app modal (portable).
    /// The combinable filter bar (text / status / record / char count) plus a
    /// reset button. Shown whenever a document is open.
    fn filter_bar(&mut self, ctx: &egui::Context) {
        if self.docs.is_empty() {
            return;
        }
        // Precompute (borrow self.i18n / docs) before the mutating closure.
        // The signatures of the document are gathered once per document
        // (50 000 lines must not be scanned every frame).
        let serial = self.active_doc().map(|d| d.serial).unwrap_or(0);
        if self.filter_sigs.as_ref().map(|c| c.0) != Some(serial) {
            // The signatures of the tab left are kept (B-25).
            if let Some(old) = self.filter_sigs.take() {
                self.filter_sigs_store.insert(old.0, old);
            }
            let live: std::collections::HashSet<u64> = self.docs.iter().map(|d| d.serial).collect();
            self.filter_sigs_store.retain(|s, _| live.contains(s));
            let cached = self.filter_sigs_store.remove(&serial);
            self.filter_sigs = Some(cached.unwrap_or_else(|| {
                let (records, fields) = self
                    .active_doc()
                    .map(|d| {
                        (
                            crate::filter::record_sigs(&d.tm.entries),
                            crate::filter::field_sigs(&d.tm.entries),
                        )
                    })
                    .unwrap_or_default();
                (serial, records, fields)
            }));
        }
        let (records, fields) = self
            .filter_sigs
            .as_ref()
            .map(|c| (c.1.clone(), c.2.clone()))
            .unwrap_or_default();
        let colors = self.config.status_colors;
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        let filter_clear_tip = self.i18n.t("filter-clear-tip");
        let lbl_filter = self.i18n.t("toolbar-filter");
        let lbl_status = self.i18n.t("filter-status");
        let lbl_record = self.i18n.t("filter-record");
        let lbl_field = self.i18n.t("filter-field");
        let lbl_chars = self.i18n.t("filter-chars");
        let lbl_all = self.i18n.t("filter-all");
        let lbl_reset = self.i18n.t("filter-reset");
        let lbl_unique = self.i18n.t("filter-unique");
        let lbl_unique_hint = self.i18n.t("filter-unique-hint");
        let lbl_unused = self.i18n.t("filter-unused");
        let lbl_chip_record = self.i18n.t("filter-chip-record");
        let lbl_chip_speaker = self.i18n.t("filter-chip-speaker");
        let lbl_chip_source = self.i18n.t("filter-chip-source");
        let lbl_chip_tip = self.i18n.t("filter-chip-tip");
        let lbl_unused_hint = self.i18n.t("filter-unused-hint");

        let lbl_in = self.i18n.t("filter-in");
        let lbl_anomalies = self.i18n.t("filter-anomalies");
        let lbl_anomalies_tip = self.i18n.t("filter-anomalies-tip");
        let lbl_prov = self.i18n.t("filter-provenance");
        let lbl_ambiguous = self.i18n.t("filter-ambiguous");
        let lbl_ambiguous_tip = self.i18n.t("filter-ambiguous-tip");
        let lbl_columns = self.i18n.t("filter-columns");
        let lbl_columns_tip = self.i18n.t("filter-columns-tip");
        let prov_labels: Vec<String> = crate::provenance::ProvFilter::ALL
            .iter()
            .map(|p| self.i18n.t(p.i18n_key()))
            .collect();
        let in_labels: Vec<String> = crate::filter::SearchIn::ALL
            .iter()
            .map(|t| self.i18n.t(t.i18n_key()))
            .collect();
        let lbl_regex_tip = self.i18n.t("filter-regex-tip");
        let filter_hint = if self.filter_regex {
            self.i18n.t("filter-hint-regex")
        } else if self.filter_in == crate::filter::SearchIn::All {
            self.i18n.t_args(
                "filter-hint-all",
                &[
                    ("source", &self.i18n.t("filter-in-source")),
                    ("trans", &self.i18n.t("filter-in-trans")),
                ],
            )
        } else {
            self.i18n.t_arg(
                "filter-hint-in",
                "target",
                &self.i18n.t(self.filter_in.i18n_key()),
            )
        };
        // Ctrl+Alt+F: Recherche avancée (A-19), before Ctrl+Maj+F and Ctrl+F
        // (which would match it).
        if !self.modal_open() && ctx.input_mut(|i| i.consume_shortcut(&MENU_SHORTCUTS.search)) {
            self.open_search_window();
        }
        // Ctrl+K: Concordance (A-20).
        if !self.modal_open() && ctx.input_mut(|i| i.consume_shortcut(&MENU_SHORTCUTS.concord)) {
            self.open_concord_window(ctx, None);
        }
        // Ctrl+Shift+J: the journal of the session (E-01).
        if ctx.input_mut(|i| i.consume_shortcut(&MENU_SHORTCUTS.journal)) {
            self.open_journal_window();
        }
        // Ctrl+Maj+F: the filter row (A-10), before Ctrl+F.
        if !self.modal_open()
            && ctx.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                    egui::Key::F,
                ))
            })
        {
            self.toggle_column_filters();
        }
        // Ctrl+F: the field (R-A09-04); consumed here so the grid never sees it.
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::F,
            ))
        }) {
            self.filter_focus_request = true;
        }
        let filter_id = egui::Id::new("ximod-filter-field");
        let field_focused = ctx.memory(|m| m.has_focus(filter_id));
        // Entrée applies at once (while the field has the focus); Échap
        // empties it — egui drops the focus on Échap before the frame, so
        // it is seen on the field as a focus lost with the key down.
        let enter = field_focused
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let esc_down = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if enter {
            self.apply_filter_text();
        }

        // Each row of the bar as tall as the toolbar's (25/10/2026): its
        // widgets are 2 px taller (the status buttons), so 1 px less of
        // margin on each side, and the gap between the two rows twice the
        // margin.
        let mut frame = egui::Frame::side_top_panel(&ctx.style());
        frame.inner_margin.top += TOOLBAR_EXTRA - 1.0;
        frame.inner_margin.bottom += TOOLBAR_EXTRA - 1.0;
        let row_gap = frame.inner_margin.top + frame.inner_margin.bottom;
        egui::TopBottomPanel::top("filter_bar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = row_gap;
                ui.horizontal_wrapped(|ui| {
                    // Labels never flow: a label at the end of a row would start its
                    // text there and continue on the next one, and that row would be
                    // placed without the row gap, over the first (reported 25/10/2026).
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(&lbl_filter);
                    // « Dans : » the target of the text (A-09).
                    keep_together(ui, &lbl_in, combo_w(ui, Some(120.0), ""));
                    ui.label(&lbl_in);
                    let cur = crate::filter::SearchIn::ALL
                        .iter()
                        .position(|t| *t == self.filter_in)
                        .unwrap_or(0);
                    let mut target_changed = false;
                    egui::ComboBox::from_id_salt("ximod-filter-in")
                        .selected_text(&in_labels[cur])
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for (i, t) in crate::filter::SearchIn::ALL.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut self.filter_in, *t, &in_labels[i])
                                    .changed()
                                {
                                    target_changed = true;
                                }
                            }
                        });
                    let mut edit = egui::TextEdit::singleline(&mut self.filter)
                        .id(filter_id)
                        .desired_width(170.0)
                        .hint_text(&filter_hint);
                    if self.filter_error.is_some() {
                        edit = edit.text_color(ui.visuals().error_fg_color);
                    }
                    let resp = ui.add(edit);
                    let resp = match &self.filter_error {
                        Some(err) => resp.on_hover_text(err),
                        None => resp,
                    };
                    if self.filter_focus_request {
                        // Focus with the whole text selected.
                        self.filter_focus_request = false;
                        resp.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), filter_id) {
                            let end = self.filter.chars().count();
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(end),
                                )));
                            state.store(ui.ctx(), filter_id);
                        }
                    }
                    if resp.changed() {
                        self.filter_typed_at = Some(ui.input(|i| i.time));
                    }
                    if esc_down && resp.lost_focus() && !self.filter.is_empty() {
                        self.filter.clear();
                        self.apply_filter_text();
                    }
                    // The red cross clears the field (F-06, as in Architect).
                    if !self.filter.is_empty()
                        && Self::icon_button(ui, "delete.png", crate::icons::X, 16.0)
                            .on_hover_text(&filter_clear_tip)
                            .clicked()
                    {
                        self.filter.clear();
                        self.apply_filter_text();
                    }
                    // « .* » : regular expression (R-A09-01).
                    let mut regex_changed = false;
                    if ui
                        .toggle_value(&mut self.filter_regex, ".*")
                        .on_hover_text(&lbl_regex_tip)
                        .changed()
                    {
                        regex_changed = true;
                    }
                    if target_changed || regex_changed {
                        self.apply_filter_text();
                    }
                    vsep(ui);
                    ui.label(&lbl_status);
                    // One button per status, the glyph dark on the status pastel
                    // (as the status column), framed while the status shows;
                    // a hidden status is grey (a pastel glyph alone was unreadable
                    // on the bar, correction of 25/10/2026).
                    let spacing = ui.spacing().item_spacing.x;
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for s in crate::status::Status::ALL {
                        let (r, g, b) = colors[s.index()];
                        let en = self.filter_statuses[s.index()];
                        let (fill, fg) = if en {
                            (
                                egui::Color32::from_rgb(r, g, b),
                                egui::Color32::from_rgb(35, 35, 35),
                            )
                        } else {
                            (
                                ui.visuals().widgets.inactive.weak_bg_fill,
                                ui.visuals().weak_text_color(),
                            )
                        };
                        let resp = ui.add(
                            egui::Button::new(egui::RichText::new(s.glyph()).color(fg).size(13.0))
                                .fill(fill)
                                .selected(en)
                                .min_size(egui::vec2(22.0, 20.0)),
                        );
                        if resp.on_hover_text(&legends[s.index()]).clicked() {
                            self.filter_statuses[s.index()] = !en;
                        }
                    }
                    ui.spacing_mut().item_spacing.x = spacing;
                    // « Anomalies » (D-05): only the lines flagged by the last check.
                    let anomaly_label = egui::RichText::new(format!("[!] {}", lbl_anomalies))
                        .color(ui.visuals().error_fg_color);
                    if ui
                        .selectable_label(self.filter_anomalies, anomaly_label)
                        .on_hover_text(&lbl_anomalies_tip)
                        .clicked()
                    {
                        self.filter_anomalies = !self.filter_anomalies;
                    }
                    // « Ambiguës » (B-08): the lines the dictionaries left marked.
                    if ui
                        .selectable_label(self.filter_ambiguous, &lbl_ambiguous)
                        .on_hover_text(&lbl_ambiguous_tip)
                        .clicked()
                    {
                        self.filter_ambiguous = !self.filter_ambiguous;
                    }
                    // « Filtres par colonne » (A-10): the row under the headers.
                    let funnel = format!("{} {}", crate::icons::FUNNEL, lbl_columns);
                    if ui
                        .selectable_label(self.config.column_filters, funnel)
                        .on_hover_text(&lbl_columns_tip)
                        .clicked()
                    {
                        self.toggle_column_filters();
                    }
                    // « Provenance : » (B-09, R-B09-06).
                    vsep(ui);
                    let prov_cur = crate::provenance::ProvFilter::ALL
                        .iter()
                        .position(|p| *p == self.filter_prov)
                        .unwrap_or(0);
                    keep_together(ui, &lbl_prov, combo_w(ui, None, &prov_labels[prov_cur]));
                    ui.label(&lbl_prov);
                    let cur = crate::provenance::ProvFilter::ALL
                        .iter()
                        .position(|p| *p == self.filter_prov)
                        .unwrap_or(0);
                    egui::ComboBox::from_id_salt("ximod-filter-prov")
                        .selected_text(&prov_labels[cur])
                        .height(480.0)
                        .show_ui(ui, |ui| {
                            for (i, p) in crate::provenance::ProvFilter::ALL.iter().enumerate() {
                                ui.selectable_value(&mut self.filter_prov, *p, &prov_labels[i]);
                            }
                        });
                    vsep(ui);
                    keep_together(ui, &lbl_record, combo_w(ui, None, &self.filter_record));
                    ui.label(&lbl_record);
                    let sel = if self.filter_record.is_empty() {
                        lbl_all.clone()
                    } else {
                        self.filter_record.clone()
                    };
                    egui::ComboBox::from_id_salt("filter_record")
                        .selected_text(sel)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.filter_record, String::new(), &lbl_all);
                            for r in &records {
                                ui.selectable_value(&mut self.filter_record, r.clone(), r);
                            }
                        });
                    vsep(ui);
                    keep_together(ui, &lbl_field, combo_w(ui, None, &self.filter_field));
                    ui.label(&lbl_field);
                    let sel_f = if self.filter_field.is_empty() {
                        lbl_all.clone()
                    } else {
                        self.filter_field.clone()
                    };
                    egui::ComboBox::from_id_salt("filter_field")
                        .selected_text(sel_f)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.filter_field, String::new(), &lbl_all);
                            for f in &fields {
                                ui.selectable_value(&mut self.filter_field, f.clone(), f);
                            }
                        });
                    vsep(ui);
                    keep_together(ui, &lbl_chars, 2.0 * ui.spacing().interact_size.x + 30.0);
                    ui.label(&lbl_chars);
                    ui.add(
                        egui::DragValue::new(&mut self.filter_min_chars)
                            .range(0..=100_000)
                            .speed(1.0),
                    );
                    ui.label("–");
                    ui.add(
                        egui::DragValue::new(&mut self.filter_max_chars)
                            .range(0..=100_000)
                            .speed(1.0),
                    );
                    vsep(ui);
                    if ui
                        .selectable_label(self.filter_unique_only, &lbl_unique)
                        .on_hover_text(&lbl_unique_hint)
                        .clicked()
                    {
                        self.filter_unique_only = !self.filter_unique_only;
                    }
                    vsep(ui);
                    // « Jamais affichées en jeu » (field mode unused): hidden unless
                    // asked; the choice is kept in Config.ini.
                    ui.checkbox(&mut self.config.show_unused, &lbl_unused)
                        .on_hover_text(&lbl_unused_hint);
                    // Filters set from the row menu (M-01): removable chips.
                    if let Some(f) = self.filter_formid {
                        vsep(ui);
                        if ui
                            .selectable_label(
                                true,
                                format!("{} {f:08X} {}", lbl_chip_record, crate::icons::X),
                            )
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.filter_formid = None;
                        }
                    }
                    if let Some(key) = self.filter_speaker.clone() {
                        vsep(ui);
                        let name = self.speaker_name_of_key(&key);
                        let label = format!(
                            "{} {}",
                            lbl_chip_speaker.replace("{name}", &name),
                            crate::icons::X
                        );
                        if ui
                            .selectable_label(true, label)
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.filter_speaker = None;
                        }
                    }
                    // « Voir les lignes » of an exclusion rule (B-18).
                    if let Some((id, _)) = self.filter_lock_rule.clone() {
                        vsep(ui);
                        let label = format!(
                            "{} {}",
                            self.i18n.t_arg("filter-chip-rule", "rule", &id),
                            crate::icons::X
                        );
                        if ui
                            .selectable_label(true, label)
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.filter_lock_rule = None;
                        }
                    }
                    // « Voir les lignes » of a journal entry (E-01).
                    if let Some((label, _, _)) = self.filter_journal.clone() {
                        vsep(ui);
                        let text = format!(
                            "{} {}",
                            self.i18n.t_arg("filter-chip-journal", "text", &label),
                            crate::icons::X
                        );
                        if ui
                            .selectable_label(true, text)
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.filter_journal = None;
                        }
                    }
                    if let Some(src) = self.filter_source_exact.clone() {
                        vsep(ui);
                        let short: String = src.chars().take(24).collect();
                        let short = if short.len() < src.len() {
                            self.i18n.t_arg("text-cut", "text", &short)
                        } else {
                            short
                        };
                        let label = format!(
                            "{} {}",
                            self.i18n.t_args(
                                "filter-chip-text",
                                &[("label", &lbl_chip_source), ("text", &short)]
                            ),
                            crate::icons::X
                        );
                        if ui
                            .selectable_label(true, label)
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.filter_source_exact = None;
                        }
                    }
                    // « Recherche avancée (n) » (A-19): a click removes it.
                    if let Some(n) = self.active_search().map(|(_, _, n)| *n) {
                        vsep(ui);
                        let label = format!(
                            "{} {}",
                            self.i18n.t_arg(
                                "search-chip",
                                "n",
                                &crate::ui::export_dialog::fmt_count(&self.i18n, n)
                            ),
                            crate::icons::X
                        );
                        if ui
                            .selectable_label(true, label)
                            .on_hover_text(&lbl_chip_tip)
                            .clicked()
                        {
                            self.search_filter = None;
                        }
                    }
                    vsep(ui);
                    if ui.button(&lbl_reset).clicked() {
                        self.search_filter = None;
                        self.filter_formid = None;
                        self.filter_source_exact = None;
                        self.filter_speaker = None;
                        self.filter_lock_rule = None;
                        self.filter_journal = None;
                        self.filter.clear();
                        self.filter_in = crate::filter::SearchIn::All;
                        self.filter_regex = false;
                        self.filter_anomalies = false;
                        self.filter_ambiguous = false;
                        self.filter_columns.clear();
                        self.filter_review_diff = false;
                        self.apply_filter_text();
                        self.filter_statuses = [true; crate::status::COUNT];
                        self.filter_prov = crate::provenance::ProvFilter::All;
                        self.filter_review_diff = false;
                        self.filter_record.clear();
                        self.filter_field.clear();
                        self.filter_min_chars = 0;
                        self.filter_max_chars = 0;
                        self.filter_unique_only = false;
                    }
                });
            });
    }

    /// Build the current filter criteria from the UI state.
    /// Take the typed text as the applied filter when it compiles; otherwise
    /// keep the previous one and show the error (R-A09-03).
    fn apply_filter_text(&mut self) {
        self.filter_typed_at = None;
        // The filter row (A-10): every field, an invalid one reported and
        // ignored (R-A10-02).
        self.filter_columns_applied = self.filter_columns.clone();
        let compiled = self.filter_columns_applied.compile(self.filter_regex);
        self.filter_column_errors = std::array::from_fn(|i| {
            compiled.errors[i]
                .as_ref()
                .map(|e| self.i18n.t_arg("filter-regex-error", "error", e))
        });
        match crate::filter::TextMatch::compile(&self.filter, self.filter_in, self.filter_regex) {
            Ok(_) => {
                self.filter_applied = self.filter.clone();
                self.filter_applied_in = self.filter_in;
                self.filter_applied_regex = self.filter_regex;
                self.filter_error = None;
            }
            Err(e) => self.filter_error = Some(self.i18n.t_arg("filter-regex-error", "error", &e)),
        }
    }

    fn criteria(&self) -> crate::filter::Criteria {
        crate::filter::Criteria {
            text: self.filter_applied.trim().to_string(),
            target: self.filter_applied_in,
            use_regex: self.filter_applied_regex,
            statuses: self.filter_statuses,
            prov: self.filter_prov,
            review_diff: self.filter_review_diff
                && self.config.show_review
                && self.config.column_filters,
            record: self.filter_record.clone(),
            field: self.filter_field.clone(),
            min_chars: self.filter_min_chars as usize,
            max_chars: self.filter_max_chars as usize,
            unique_only: self.filter_unique_only,
            show_unused: self.config.show_unused,
            formid: self.filter_formid,
            source_exact: self.filter_source_exact.clone(),
            only_lines: self
                .speaker_filter_lines()
                .or_else(|| self.filter_lock_rule.as_ref().map(|(_, s)| s.clone()))
                .or_else(|| self.journal_filter_lines()),
            anomalies_only: self.filter_anomalies,
            anomalies: self.anomaly_set.clone(),
            ambiguous_only: self.filter_ambiguous,
            comments: if self.filter_applied_in == crate::filter::SearchIn::Comment
                || self.column_filter_needs(crate::filter::FilterCol::Comment)
                || self.search_needs_comments()
            {
                self.comment_texts.as_ref().map(|(_, t)| t.clone())
            } else {
                None
            },
            columns: if self.config.column_filters {
                let mut c = self.filter_columns_applied.clone();
                // The Fichier column exists in « Tous les mods » only (B-25).
                if !self.all_active() {
                    c.get_mut(crate::filter::FilterCol::File).clear();
                }
                c
            } else {
                Default::default()
            },
            prov_texts: if self.column_filter_needs(crate::filter::FilterCol::Prov) {
                self.prov_texts.as_ref().map(|(_, _, t)| t.clone())
            } else {
                None
            },
            search: self.active_search().map(|(_, f, _)| f.clone()),
        }
    }

    /// Whether the applied advanced search looks into the comments.
    fn search_needs_comments(&self) -> bool {
        self.active_search()
            .is_some_and(|(_, f, _)| f.0.needs_comments())
    }

    /// Whether the applied filter row has a text under column `c` (A-10).
    fn column_filter_needs(&self, c: crate::filter::FilterCol) -> bool {
        self.config.column_filters && !self.filter_columns_applied.get(c).trim().is_empty()
    }

    /// The short provenance labels of the active document, for the
    /// Provenance column filter; rebuilt when the document moves.
    fn ensure_prov_texts(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.prov_texts = None;
            return;
        };
        let key = (doc.serial, doc.undo.revision);
        if self
            .prov_texts
            .as_ref()
            .is_some_and(|(s, r, _)| (*s, *r) == key)
        {
            return;
        }
        let texts: Vec<String> = doc
            .tm
            .entries
            .iter()
            .map(|e| crate::provenance::short_label(&self.i18n, e))
            .collect();
        self.prov_texts = Some((
            key.0,
            key.1,
            crate::filter::CommentTexts(std::sync::Arc::new(texts)),
        ));
    }

    /// Show or hide the filter row (A-10, Ctrl+Maj+F); hiding it empties
    /// its fields: no filter acts unseen (D-47).
    fn toggle_column_filters(&mut self) {
        self.config.column_filters = !self.config.column_filters;
        if !self.config.column_filters {
            self.filter_columns.clear();
            self.filter_review_diff = false;
            self.apply_filter_text();
        }
        let _ = self.config.save();
    }

    /// The column titles of the grid, following the engine of the active
    /// document (S-05).
    fn grid_headers(&self) -> [String; 10] {
        let cols = self
            .docs
            .get(self.active)
            .map(|d| crate::engine::for_tm(&d.tm).columns())
            .unwrap_or_else(|| crate::engine::for_game(crate::games::Game::Unknown).columns());
        [
            self.i18n.t("col-st"),
            self.i18n.t(cols.group),
            self.i18n.t(cols.field),
            self.i18n.t(cols.id),
            self.i18n.t("col-source"),
            self.i18n.t("col-trans"),
            self.i18n.t("col-review"),
            self.i18n.t("col-provenance"),
            self.i18n.t("col-comment"),
            self.i18n.t("col-file"),
        ]
    }

    /// « filtres : GRUP « ARMO », Champ « FULL » » (R-A10-06), or `None`
    /// without a column filter.
    fn column_filters_summary(&self) -> Option<String> {
        if !self.config.column_filters {
            return None;
        }
        let headers = self.grid_headers();
        let mut parts: Vec<String> = self
            .filter_columns_applied
            .active()
            .map(|(c, t)| {
                self.i18n.t_args(
                    "filter-chip-text",
                    &[("label", &headers[c.header()]), ("text", t)],
                )
            })
            .collect();
        if self.filter_review_diff && self.config.show_review {
            parts.push(format!("{} ≠", headers[grid::H_REVIEW]));
        }
        if parts.is_empty() {
            return None;
        }
        Some(
            self.i18n
                .t_arg("filter-columns-active", "list", &parts.join(", ")),
        )
    }

    /// The key of the comment caches (C-03): the document, its dialogue
    /// map, and the voices generation.
    fn comment_key(&self) -> Option<crate::comments::CacheKey> {
        let doc = self.docs.get(self.active)?;
        // The revision of the map's key is dropped: a translation edit
        // changes no comment, and 50 000 texts must not be regenerated
        // per keystroke.
        let map_key = self.dialogs.map_key.map(|(s, _, m, v)| (s, 0, m, v));
        Some((doc.serial, map_key, self.dialogs.voices_gen))
    }

    /// « Dans : Commentaire » needs every line's text (R-C03-04): generate
    /// them once per document / map, starting the context pass if needed.
    fn ensure_comment_texts(&mut self, ctx: &egui::Context) {
        let map = self.dialog_map(ctx);
        let Some(key) = self.comment_key() else {
            self.comment_texts = None;
            return;
        };
        if self.comment_texts.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let g = crate::comments::Generator {
            map: map.as_deref(),
            i18n: &self.i18n,
            voices: self.dialogs.voices.clone(),
        };
        let texts: Vec<String> = doc.tm.entries.iter().map(|e| g.full(e)).collect();
        self.comment_texts = Some((key, crate::filter::CommentTexts(std::sync::Arc::new(texts))));
    }

    /// The row menu's texts (M-01), from the i18n bundle; the clipboard is
    /// read only when a menu is open.
    fn menu_labels(&mut self, ctx: &egui::Context) -> grid::MenuLabels {
        let popup_open = ctx.memory(|m| m.any_popup_open());
        let clipboard = self.clipboard.current(popup_open);
        let t = |k: &str| self.i18n.t(k);
        grid::MenuLabels {
            edit: t("ctx-edit"),
            copy_source: t("ctx-copy-source"),
            copy_trans: t("ctx-copy-trans"),
            paste_one: t("ctx-paste-one"),
            paste_many: t("ctx-paste-many"),
            paste_disabled: t("ctx-paste-disabled"),
            trans_eq_source: t("ctx-trans-eq-source"),
            status_one: t("ctx-status-one"),
            status_many: t("ctx-status-many"),
            validate_next: t("ctx-validate-next"),
            filter_record: t("ctx-filter-record"),
            filter_source: t("ctx-filter-source"),
            filter_speaker: t("ctx-filter-speaker"),
            book: t("ctx-book"),
            book_disabled: t("ctx-book-disabled"),
            add_personal: t("ctx-add-personal"),
            apply_dicts: t("ctx-apply-dicts"),
            propagate: t("ctx-propagate"),
            key_propagate: t("key-ctrl-f6"),
            online: t("menu-online-line"),
            key_online: t("key-ctrl-g"),
            review: t("ctx-review"),
            review_accept: t("review-accept"),
            review_from_trans: t("review-from-trans"),
            review_clear: t("review-clear"),
            key_review: shortcut_text(ctx, &MENU_SHORTCUTS.review_accept, &self.i18n),
            history: t("ctx-history"),
            history_previous: t("history-restore-previous"),
            history_show: t("history-show-line"),
            key_history: shortcut_text(ctx, &SHORTCUT_HISTORY_PREV, &self.i18n),
            propagate_files: (self.file_count() >= 2).then(|| t("ctx-propagate-files")),
            concord: t("ctx-concordance"),
            key_concord: shortcut_text(ctx, &MENU_SHORTCUTS.concord, &self.i18n),
            key_personal: t("key-ctrl-m"),
            key_edit: t("key-enter"),
            key_copy: key_text(ctx, &self.i18n, egui::Modifiers::COMMAND, egui::Key::C),
            key_paste: key_text(ctx, &self.i18n, egui::Modifiers::COMMAND, egui::Key::V),
            key_eq: key_text(ctx, &self.i18n, egui::Modifiers::COMMAND, egui::Key::F8),
            key_status: ["F5".into(), "F6".into(), "F7".into(), "F8".into()],
            key_validate_next: t("key-ctrl-down"),
            clipboard,
        }
    }

    /// Run a row-menu action (M-01) on the selection.
    fn run_menu_action(&mut self, ctx: &egui::Context, idx: usize, action: grid::MenuAction) {
        use grid::MenuAction as A;
        let targets = {
            let mut t = self.selection.targets();
            if !t.contains(&idx) {
                t = vec![idx];
            }
            t
        };
        match action {
            A::Edit => self.open_edit_window(idx),
            A::CopySource => {
                if let Some(e) = self.active_doc().and_then(|d| d.tm.entries.get(idx)) {
                    ctx.copy_text(e.source.clone());
                }
            }
            A::CopyTrans => {
                if let Some(e) = self.active_doc().and_then(|d| d.tm.entries.get(idx)) {
                    ctx.copy_text(e.trans.clone());
                }
            }
            A::PasteOne | A::PasteMany => {
                let Some(text) = read_clipboard().filter(|t| !t.is_empty()) else {
                    return;
                };
                let lines = if action == A::PasteOne {
                    vec![idx]
                } else {
                    targets
                };
                self.set_translations(&lines, |_| text.clone(), "ctx-paste-done");
            }
            A::TransEqSource => {
                self.set_translations(&targets, |e| e.source.clone(), "ctx-eq-done");
            }
            A::Status(st) => {
                self.set_status_selection(st);
            }
            A::ValidateNext => self.validate_and_move(1),
            A::FilterRecord => {
                if let Some(e) = self.active_doc().and_then(|d| d.tm.entries.get(idx)) {
                    self.filter_formid = Some(e.formid);
                    self.filter_source_exact = None;
                }
            }
            A::FilterSource => {
                if let Some(e) = self.active_doc().and_then(|d| d.tm.entries.get(idx)) {
                    self.filter_source_exact = Some(e.source.clone());
                    self.filter_formid = None;
                }
            }
            A::Book => self.open_book_preview(idx),
            A::AddPersonal => self.add_to_personal(&targets),
            A::ApplyDicts => {
                if !self.selection.rows.contains(&idx) {
                    self.selection.set_single(idx);
                }
                self.open_apply_dialog(crate::replace::Scope::Selection, true);
            }
            A::Propagate => {
                let in_edit = self.edit_win_active_frame;
                self.open_propagate(&targets, in_edit);
            }
            A::FilterSpeaker => self.filter_on_speaker_of(ctx, idx),
            A::Online => self.online_line(targets),
            A::ReviewAccept => self.review_command(app_review::ReviewCmd::Accept, &targets),
            A::ReviewFromTrans => self.review_command(app_review::ReviewCmd::FromTrans, &targets),
            A::ReviewClear => self.review_command(app_review::ReviewCmd::Clear, &targets),
            A::HistoryPrevious => self.history_revert_previous(&targets),
            A::HistoryShow => self.open_history_window(Some(idx)),
            A::Concordance => self.open_concord_window(ctx, Some(idx)),
            A::PropagateFiles => self.propagate_selection_to_files(&targets),
        }
    }

    /// Set the translation of several lines in one undoable operation
    /// (paste, « Traduction = source »); Ignoré lines and lines never shown
    /// in game are skipped (R-M01-08).
    fn set_translations(
        &mut self,
        lines: &[usize],
        text_of: impl Fn(&crate::tm::TmEntry) -> String,
        done_key: &str,
    ) {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let mut before = Vec::new();
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            if e.unused || crate::status::derive(e) == crate::status::Status::Ignored {
                continue;
            }
            before.push((i, EntryState::capture(e)));
        }
        for (i, _) in &before {
            let t = text_of(&doc.tm.entries[*i]);
            let e = &mut doc.tm.entries[*i];
            if e.trans != t {
                e.trans = t;
                e.set_translated_flags(!e.trans.is_empty());
            }
        }
        let op = op_from_before(OpKind::Edit, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        self.status = self.i18n.t_arg(done_key, "n", &n.to_string());
    }

    /// Recompute the filtered view (and counters) for the active document.
    fn refresh_view(&mut self, ctx: &egui::Context) {
        // Debounce of the filter field (R-A09-03): applied 250 ms after the
        // last keystroke; an invalid expression keeps the previous filter.
        if let Some(at) = self.filter_typed_at {
            let now = ctx.input(|i| i.time);
            if now - at >= 0.25 {
                self.apply_filter_text();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
        }
        if (self.filter_applied_in == crate::filter::SearchIn::Comment
            && !self.filter_applied.trim().is_empty())
            || self.column_filter_needs(crate::filter::FilterCol::Comment)
            || self.search_needs_comments()
        {
            self.ensure_comment_texts(ctx);
        }
        if self.column_filter_needs(crate::filter::FilterCol::Prov) {
            self.ensure_prov_texts();
        }
        let crit = self.criteria();
        if self.last_criteria.as_ref() != Some(&crit) {
            // A filter change drops the pinned lines (R-A03-05).
            self.pinned_rows.clear();
            self.last_criteria = Some(crit.clone());
        }
        // The view is recomputed only when the document, its revision or
        // the criteria moved (50 000 lines must not be filtered every frame).
        let (serial, revision) = self
            .docs
            .get(self.active)
            .map(|d| (d.serial, d.undo.revision))
            .unwrap_or((0, 0));
        let pinned = (
            self.pinned_rows.len(),
            self.pinned_rows.iter().sum::<usize>(),
        );
        let key = (serial, revision, pinned, crit.clone());
        if self.view_key.as_ref() == Some(&key) {
            return;
        }
        // The view of the tab left is kept; the tab come back to finds its
        // own when nothing moved meanwhile (B-25).
        if let Some((old, ..)) = self.view_key.as_ref()
            && *old != serial
        {
            let old = *old;
            let stored = StoredView {
                key: self.view_key.clone(),
                visible: std::mem::take(&mut self.visible),
                total: self.view_total,
                unique: self.view_unique,
                unique_remaining: self.view_unique_remaining,
                per_status: self.view_per_status,
                shown_per_status: self.view_shown_per_status,
            };
            self.view_store.insert(old, stored);
        }
        let live: std::collections::HashSet<u64> = self.docs.iter().map(|d| d.serial).collect();
        self.view_store.retain(|s, _| live.contains(s));
        if let Some(v) = self.view_store.remove(&serial)
            && v.key.as_ref() == Some(&key)
        {
            self.view_key = v.key;
            self.visible = v.visible;
            self.view_total = v.total;
            self.view_unique = v.unique;
            self.view_unique_remaining = v.unique_remaining;
            self.view_per_status = v.per_status;
            self.view_shown_per_status = v.shown_per_status;
            return;
        }
        self.view_key = Some(key);
        let (stats, mut visible) = if let Some(doc) = self.docs.get(self.active) {
            (
                crate::filter::stats(&doc.tm.entries, self.config.show_unused),
                crit.apply(&doc.tm.entries),
            )
        } else {
            (crate::filter::Stats::default(), Vec::new())
        };
        if !self.pinned_rows.is_empty() {
            let n = self
                .docs
                .get(self.active)
                .map(|d| d.tm.entries.len())
                .unwrap_or(0);
            let mut merged: Vec<usize> = visible
                .iter()
                .copied()
                .chain(self.pinned_rows.iter().copied().filter(|i| *i < n))
                .collect();
            merged.sort_unstable();
            merged.dedup();
            visible = merged;
        }
        self.view_total = stats.total;
        self.view_unique = stats.unique;
        self.view_unique_remaining = stats.unique_remaining();
        self.view_per_status = stats.per_status;
        self.view_shown_per_status = self
            .docs
            .get(self.active)
            .map(|d| crate::filter::shown_per_status(&d.tm.entries, &visible))
            .unwrap_or_default();
        self.visible = visible;
    }

    fn tabs_bar(&mut self, ctx: &egui::Context) {
        if self.docs.is_empty() {
            return;
        }
        let mut to_close: Option<usize> = None;
        let mut restack: Option<usize> = None;
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for i in 0..self.docs.len() {
                        let selected = i == self.active;
                        // « Tous les mods (n) » (B-25): no badge, no close.
                        if self.docs[i].all.is_some() {
                            let text = egui::RichText::new(self.tab_name(i)).italics();
                            let text = if selected { text.strong() } else { text };
                            if ui
                                .selectable_label(selected, text)
                                .on_hover_text(self.i18n.t("tab-all-mods-tip"))
                                .clicked()
                                && !selected
                            {
                                self.close_edit_session();
                                self.active = i;
                                self.selection.clear();
                            }
                            continue;
                        }
                        let name = self.docs[i].file_name();
                        // Known DLC → show its clear name; the file name stays on
                        // hover (e.g. "Vault-Tec Workshop" ⟵ DLCworkshop03.esm).
                        let clear = crate::games::plugin_display_name(&name);
                        let label = clear.unwrap_or(name.as_str());
                        // Badge « Jeu · SRC→DST · Sortie » after the name (E-12):
                        // weak and small, truncated before the name ever is.
                        let badge = match (&self.docs[i].text, self.docs[i].text_format()) {
                            (Some(t), Some(fmt)) => self.i18n.t_args(
                                "tab-badge-text",
                                &[
                                    ("format", &fmt.sig()),
                                    ("enc", t.encoding.label()),
                                    ("src", &self.docs[i].settings.source.to_uppercase()),
                                    ("dst", &self.docs[i].settings.target.to_uppercase()),
                                ],
                            ),
                            _ => self.docs[i].settings.badge(&self.i18n),
                        };
                        // The page of code when it is not the game's (D-09).
                        let badge = match self.encoding_chip(&self.docs[i]) {
                            Some(chip) => self.i18n.dot(&badge, &chip),
                            None => badge,
                        };
                        let mut job = egui::text::LayoutJob::default();
                        let body = egui::TextStyle::Body.resolve(ui.style());
                        let small = egui::TextStyle::Small.resolve(ui.style());
                        let strong = if selected {
                            ui.visuals().strong_text_color()
                        } else {
                            ui.visuals().text_color()
                        };
                        // Marque « modifié » (R-A01-05), recomputed only when the content moved.
                        let marked = if self.docs[i].is_dirty() {
                            format!("{} {label}", crate::icons::DOT)
                        } else {
                            label.to_string()
                        };
                        job.append(
                            &marked,
                            0.0,
                            egui::TextFormat {
                                font_id: body,
                                color: strong,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &badge,
                            8.0,
                            egui::TextFormat {
                                font_id: small,
                                color: ui.visuals().weak_text_color(),
                                ..Default::default()
                            },
                        );
                        let resp = ui.selectable_label(selected, job);
                        let resp = if clear.is_some() {
                            resp.on_hover_text(&name)
                        } else {
                            resp
                        };
                        if resp.clicked() {
                            self.active = i;
                        }
                        // Tab menu: the personal-dictionary choice, until the
                        // dictionary tree (B-02) takes it over.
                        let mut toggled = false;
                        resp.context_menu(|ui| {
                            fit_menu(ui);
                            let mut on = self.docs[i].settings.use_personal;
                            if ui
                                .checkbox(&mut on, self.i18n.t("dict-use-personal"))
                                .changed()
                            {
                                self.docs[i].settings.use_personal = on;
                                toggled = true;
                                ui.close_menu();
                            }
                        });
                        if toggled {
                            restack = Some(i);
                        }
                        if Self::icon_button(ui, "delete.png", crate::icons::X, 18.0).clicked() {
                            to_close = Some(i);
                        }
                        ui.separator();
                    }
                });
            });
        });
        if let Some(i) = restack {
            // The stack changed: fill the still-empty lines from it.
            self.restack_doc(i);
        }
        if let Some(i) = to_close {
            self.request_close_doc(i);
        }
    }

    /// Bottom bar: the last message on the left, the counters on the right.
    /// The counters are laid out first, so the message never pushes them out
    /// of the window: it is truncated instead (full text on hover). The
    /// file's name and folder are not repeated here — the tab and the window
    /// title already carry them.
    fn status_bar(&mut self, ctx: &egui::Context) {
        let has_doc = !self.docs.is_empty();
        let modified = self.active_doc().is_some_and(|d| d.is_dirty());
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        // Cached counters from refresh_view: « Total N · M affichées » and
        // « uniques (à traduire) »; the per-status pills replace the former
        // « traduites » (A-15, decision D-50).
        let counts = format!(
            "{} ({}, {})",
            self.i18n.t_args(
                "counter-total",
                &[
                    ("total", &fmt(self.view_total)),
                    ("shown", &fmt(self.visible.len())),
                ],
            ),
            self.i18n
                .t_arg("counter-unique-n", "n", &fmt(self.view_unique)),
            self.i18n
                .t_arg("counter-remaining-n", "n", &fmt(self.view_unique_remaining)),
        );
        let colors = self.config.status_colors;
        let per_status = self.view_per_status;
        let shown = self.view_shown_per_status;
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        let tips: [String; crate::status::COUNT] = std::array::from_fn(|i| {
            self.i18n.t_args(
                "counter-status-tip",
                &[
                    ("status", &legends[i]),
                    ("n", &fmt(per_status[i])),
                    ("shown", &fmt(shown[i])),
                ],
            )
        });
        let statuses_l = self.i18n.t("counter-statuses");
        // The VMAD lines and the blocks not read (C-08).
        let vmad = self.vmad_counters();
        let vmad_tip = self.i18n.t("menu-vmad-enable-tip");
        // The active column filters, on the « affichées » count (R-A10-06).
        let counts_tip = self.column_filters_summary();
        let mut clicked: Option<(crate::status::Status, bool)> = None;
        let offer = self.rules_offer_label();
        let mut open_offer = false;
        let mut open_journal = false;
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if has_doc {
                        let resp = ui.label(counts);
                        if let Some(tip) = &counts_tip {
                            resp.on_hover_text(tip);
                        }
                        ui.separator();
                        // The pills, in the order of the statuses: glyph and
                        // count in the status colour; a status without a
                        // line is dimmed; a click filters on it.
                        let full_w: f32 = ui.available_width();
                        let compact = full_w < 760.0;
                        let folded = full_w < 520.0;
                        if folded {
                            let resp = ui.selectable_label(
                                self.filter_statuses.iter().any(|b| !b),
                                &statuses_l,
                            );
                            resp.on_hover_ui(|ui| {
                                for (i, st) in crate::status::Status::ALL.iter().enumerate() {
                                    let (r, g, b) = colors[st.index()];
                                    ui.colored_label(
                                        egui::Color32::from_rgb(r, g, b),
                                        format!(
                                            "{} {}  {}",
                                            st.glyph(),
                                            fmt(per_status[i]),
                                            legends[i]
                                        ),
                                    );
                                }
                            });
                        } else {
                            for (i, st) in crate::status::Status::ALL.iter().enumerate().rev() {
                                let (r, g, b) = colors[st.index()];
                                let mut fill = egui::Color32::from_rgb(r, g, b);
                                let mut ink = egui::Color32::from_rgb(35, 35, 35);
                                if per_status[i] == 0 {
                                    fill = fill.gamma_multiply(0.35);
                                    ink = ink.gamma_multiply(0.45);
                                }
                                let text = if compact {
                                    fmt(per_status[i])
                                } else {
                                    format!("{} {}", st.glyph(), fmt(per_status[i]))
                                };
                                // Pressed look when the filter keeps this status
                                // (and not every status).
                                let on = self.filter_statuses[i]
                                    && self.filter_statuses.iter().any(|b| !b);
                                let stroke = if on {
                                    egui::Stroke::new(1.5_f32, ui.visuals().strong_text_color())
                                } else {
                                    egui::Stroke::NONE
                                };
                                let inner = egui::Frame::default()
                                    .fill(fill)
                                    .stroke(stroke)
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::symmetric(6.0, 1.0))
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(text).color(ink).strong(),
                                            )
                                            .selectable(false),
                                        );
                                    });
                                let resp = ui
                                    .interact(
                                        inner.response.rect,
                                        ui.id().with(("ximod-counter", i)),
                                        egui::Sense::click(),
                                    )
                                    .on_hover_text(&tips[i]);
                                if resp.clicked() {
                                    let add = ui.input(|inp| inp.modifiers.command);
                                    clicked = Some((*st, add));
                                }
                            }
                        }
                        ui.separator();
                        if let Some((count, unread)) = &vmad {
                            if let Some(u) = unread {
                                ui.colored_label(ui.visuals().warn_fg_color, u);
                            }
                            ui.label(count).on_hover_text(&vmad_tip);
                            ui.separator();
                        }
                    }
                    // The journal (R-E01-06): before the counters, after the message.
                    if self.journal_status_button(ui, ui.ctx().screen_rect().width() < 760.0) {
                        open_journal = true;
                    }
                    if modified {
                        // « ● Modifié », warning colour readable on both themes (F-05).
                        ui.label(self.i18n.t("statusbar-modified"));
                        ui.label(
                            egui::RichText::new(crate::icons::DOT)
                                .color(ui.visuals().warn_fg_color),
                        );
                        ui.separator();
                    }
                    // Whatever is left, from the left edge: the message, cut
                    // with an ellipsis rather than overflowing (a truncated
                    // label shows its full text on hover by itself).
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // « Généraliser… » after an Ignoré by hand (B-18).
                        if let Some(l) = &offer
                            && ui.link(l).clicked()
                        {
                            open_offer = true;
                        }
                        ui.add(egui::Label::new(self.i18n.localize(&self.status)).truncate());
                    });
                });
            });
        });
        if let Some((st, add)) = clicked {
            self.counter_click(st, add);
        }
        if open_offer {
            self.rules_open_offer();
        }
        if open_journal {
            self.open_journal_window();
        }
    }

    /// A click on a status pill (R-A15-04): the same filter as the buttons
    /// of the filter bar.
    fn counter_click(&mut self, st: crate::status::Status, add: bool) {
        crate::filter::click_status(&mut self.filter_statuses, st, add);
    }

    fn central(&mut self, ctx: &egui::Context) {
        // Status palette and localized legends, computed before borrowing docs.
        let colors = self.config.status_colors;
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        // The filtered view was computed this frame in `refresh_view`; move it out
        // so the panel closure only borrows `self.docs`.
        let visible = std::mem::take(&mut self.visible);
        let unique_only = self.filter_unique_only;
        let book_label = self.i18n.t("book-preview");
        let unused_tip = self.i18n.t("grid-unused-tip");
        // C-08: the VMAD lines held by the switch.
        let vmad_off_tip = self.i18n.t("vmad-off-tip");
        // Column labels follow the engine of the active document (S-05): the
        // grid itself knows nothing about GRUP / EDID.
        let headers = self.grid_headers();
        let col_menu = self.column_menu_items();
        let show_prov = self.config.show_provenance;
        // The status buttons column (A-04): its tooltips.
        let status_buttons = self.config.show_status_buttons.then(|| {
            let keys = [
                "status-btn-validate",
                "status-btn-pending",
                "status-btn-custom",
                "status-btn-ignore",
            ];
            grid::StatusButtonLabels {
                tips: std::array::from_fn(|k| self.i18n.t(keys[k])),
                needs_trans: std::array::from_fn(|k| {
                    let st = grid::STATUS_BUTTONS[k];
                    self.i18n.t_arg(
                        "status-btn-needs-trans",
                        "status",
                        &self.i18n.t(st.legend_key()),
                    )
                }),
            }
        });
        // The Commentaire column (C-03): its map, when the column shows,
        // and the per-line cache aligned on the document.
        let show_comment = self.config.show_comment;
        let comment_map = if show_comment {
            crate::slow_io("dialog map", || self.dialog_map(ctx))
        } else {
            None
        };
        if show_comment {
            let lines = self
                .docs
                .get(self.active)
                .map(|d| d.tm.entries.len())
                .unwrap_or(0);
            match self.comment_key() {
                Some(key) => self.comment_cache.align(key, lines),
                None => self.comment_cache.clear(),
            }
        }
        let comment_cache = std::cell::RefCell::new(std::mem::take(&mut self.comment_cache));
        let menu_labels = crate::slow_io("menu labels", || self.menu_labels(ctx));
        let len_over_tip = self.i18n.t_args(
            "len-over-tip",
            &[
                ("chars", "{chars}"),
                ("max", "{max}"),
                ("bytes", "{bytes}"),
                ("field", "{field}"),
            ],
        );
        let len_newline_tip = self.i18n.t_arg("len-newline-tip", "field", "{field}");
        let bytes_unit = self.i18n.t("len-bytes-unit");
        let alias_tips: [String; 5] = [
            self.i18n.t_arg("alias-missing", "tag", "{tag}"),
            self.i18n.t_arg("alias-unknown", "tag", "{tag}"),
            self.i18n.t_arg("alias-number-missing", "n", "{n}"),
            self.i18n.t_arg("alias-number-unknown", "n", "{n}"),
            self.i18n.t("alias-order"),
        ];
        let undo_label = self.i18n.t("btn-undo");
        let redo_label = self.i18n.t("btn-redo");
        let anomalies = std::mem::take(&mut self.anomalies);
        let i18n = &self.i18n;
        let prov_tip = |e: &crate::tm::TmEntry| crate::provenance::tip(i18n, e);
        let prov_short = |e: &crate::tm::TmEntry| crate::provenance::short_label(i18n, e);
        // The script and the property of a VMAD line (C-08).
        let field_tip = |e: &crate::tm::TmEntry| -> Option<String> {
            (e.field_sig == crate::vmad_lock::FIELD && crate::l10n::has_msg(&e.comment))
                .then(|| i18n.localize(&e.comment))
        };
        let comment_gen = crate::comments::Generator {
            map: comment_map.as_deref(),
            i18n,
            voices: self.dialogs.voices.clone(),
        };
        let comment_of = |idx: usize, e: &crate::tm::TmEntry| -> std::sync::Arc<str> {
            comment_cache.borrow_mut().get(idx, e, &comment_gen)
        };
        let scroll_to = self.grid_scroll_to.take();
        let selection = self.selection.clone();
        let focus_request = self.grid_focus_request;
        // The row heights of the active tab: those kept for it when it was
        // left (B-25).
        let active_serial = self.docs.get(self.active).map(|d| d.serial).unwrap_or(0);
        if active_serial != self.heights_serial {
            let old = std::mem::take(&mut self.grid_heights);
            self.heights_store.insert(self.heights_serial, old);
            let live: std::collections::HashSet<u64> = self.docs.iter().map(|d| d.serial).collect();
            self.heights_store.retain(|s, _| live.contains(s));
            self.grid_heights = self
                .heights_store
                .remove(&active_serial)
                .unwrap_or_default();
            self.heights_serial = active_serial;
        }
        let mut grid_heights = std::mem::take(&mut self.grid_heights);
        let mut grid_out: Option<grid::GridOutput> = None;
        // The filter row (A-10), while it shows.
        let empty_msg = self.column_filters_summary();
        let mut filter_columns = std::mem::take(&mut self.filter_columns);
        let mut review_diff = self.filter_review_diff;
        let filter_errors = self.filter_column_errors.clone();
        let review_diff_tip = self.i18n.t("filter-review-diff-tip");
        let clear_tip = self.i18n.t("filter-columns-clear-tip");
        let mut filter_row = self.config.column_filters.then(|| grid::FilterRow {
            filters: &mut filter_columns,
            errors: &filter_errors,
            review_diff: &mut review_diff,
            review_diff_tip: &review_diff_tip,
            clear_tip: &clear_tip,
        });
        let (all_can_undo, all_can_redo) = (
            self.all_mods_peek(false).is_some(),
            self.all_mods_peek(true).is_some(),
        );
        let out_page = self
            .docs
            .get(self.active)
            .map(|d| self.out_page(d))
            .unwrap_or(crate::codepage::Page::Utf8);
        egui::CentralPanel::default().show(ctx, |ui| {
            let active = self.active;
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
            self.grid_page_rows = ((ui.available_height() - row_h * 2.0) / row_h.max(1.0))
                .floor()
                .max(1.0) as usize;
            if let Some(doc) = self.docs.get_mut(active) {
                let game = doc.settings.game();
                let labels = grid::GridLabels {
                    game,
                    page: out_page,
                    len_over_tip: &len_over_tip,
                    len_newline_tip: &len_newline_tip,
                    bytes_unit: &bytes_unit,
                    alias_tips: &alias_tips,
                    show_special: self.config.show_special_chars,
                    anomalies: &anomalies,
                    prov_tip: &prov_tip,
                    prov_short: if show_prov { Some(&prov_short) } else { None },
                    menu: &menu_labels,
                    legends: &legends,
                    book_label: &book_label,
                    headers: &headers,
                    undo_label: &undo_label,
                    redo_label: &redo_label,
                    can_undo: if doc.all.is_some() {
                        all_can_undo
                    } else {
                        doc.undo.can_undo()
                    },
                    can_redo: if doc.all.is_some() {
                        all_can_redo
                    } else {
                        doc.undo.can_redo()
                    },
                    unused_tip: &unused_tip,
                    vmad_off_tip: &vmad_off_tip,
                    field_tip: &field_tip,
                    comment: if show_comment {
                        Some(&comment_of)
                    } else {
                        None
                    },
                    col_menu: &col_menu,
                    review: self.config.show_review,
                    status_buttons: status_buttons.as_ref(),
                    empty_msg: empty_msg.as_deref(),
                    file_col: doc.all.is_some(),
                };
                let out = crate::slow_io("grid draw", || {
                    grid::draw(
                        ui,
                        &mut doc.tm.entries,
                        &visible,
                        &colors,
                        &labels,
                        scroll_to,
                        &selection,
                        focus_request,
                        &mut grid_heights,
                        (doc.serial, doc.undo.revision),
                        filter_row.as_mut(),
                    )
                });
                // In the unique-lines view, a translation edited on the single
                // visible row propagates to all its hidden duplicates.
                if unique_only && let Some(idx) = out.edited {
                    crate::filter::propagate_translation(&mut doc.tm.entries, idx);
                }
                if out.edited.is_some() || out.review_edited.is_some() {
                    doc.undo.touch(); // live typing: the tab marker follows
                }
                grid_out = Some(out);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(self.i18n.t("hint-open-file"));
                });
            }
        });
        // The view stays available to what runs after the grid (the edit
        // window's Suivant / Précédent, A-02).
        self.visible = visible;
        self.anomalies = anomalies;
        self.grid_heights = grid_heights;
        self.comment_cache = comment_cache.into_inner();
        let _ = filter_row;
        self.filter_columns = filter_columns;
        self.filter_review_diff = review_diff;
        if let Some(out) = grid_out {
            if out.filter_typed {
                self.filter_typed_at = Some(ctx.input(|i| i.time));
            }
            if out.filter_enter || out.filter_now {
                self.apply_filter_text();
            }
            if let Some(i) = out.column_toggled {
                self.toggle_column(i);
            }
            let action = out.menu_action;
            self.handle_grid_output(out, unique_only);
            if let Some((idx, a)) = action {
                self.run_menu_action(ctx, idx, a);
            }
        }
    }

    /// Turn the grid's events into undoable operations (A-01, R-A01-01).
    fn handle_grid_output(&mut self, out: grid::GridOutput, unique_only: bool) {
        let active = self.active;
        if out.focus_applied {
            self.grid_focus_request = None;
        }
        // Selection (A-03): a click on the row, or the focus landing in a cell.
        if let Some((idx, mods)) = out.row_clicked {
            self.selection.click(idx, mods, &self.visible);
            self.pinned_rows.clear();
        }
        if let Some(idx) = out.row_right_clicked {
            if !self.selection.rows.contains(&idx) {
                self.selection.set_single(idx);
                self.pinned_rows.clear();
            } else {
                self.selection.cursor = Some(idx);
            }
        }
        if let Some(idx) = out.focus_gained {
            if !self.selection.rows.contains(&idx) {
                self.selection.set_single(idx);
                self.pinned_rows.clear();
            } else {
                self.selection.cursor = Some(idx);
            }
        }
        if let Some(idx) = out.row_double_clicked {
            self.open_edit_window(idx);
        }
        // A status button of the column (A-04, R-A04-02/04): the line alone
        // when it is not selected, then the same path as F5–F8.
        if let Some((idx, st)) = out.status_button {
            if !self.selection.rows.contains(&idx) {
                self.selection.set_single(idx);
                self.pinned_rows.clear();
            }
            let (changed, _) = self.set_status_selection(st);
            if changed > 0 && st == crate::status::Status::Validated {
                self.propose_propagation(false);
            }
        }
        // A status chosen from the context menu: one operation.
        if let Some((idx, before, st)) = out.status_set {
            self.close_edit_session();
            let name = self.i18n.t(st.legend_key());
            if let Some(doc) = self.docs.get_mut(active) {
                let mut op = op_from_before(OpKind::Status, &[(idx, before)], &mut doc.tm.entries);
                op.label_arg = Some(name);
                doc.undo.push(op);
            }
        }
        // Focus moves: close the previous cell's session, open the new one.
        if let Some(idx) = out.focus_lost
            && self
                .edit_session
                .as_ref()
                .is_some_and(|s| s.doc == active && s.before.first().map(|b| b.0) == Some(idx))
        {
            self.close_edit_session();
        }
        if let Some(idx) = out.focus_gained {
            self.open_edit_session(idx, unique_only);
        }
        // The review cell (A-17): its own session.
        if let Some(idx) = out.review_focus_lost
            && self.edit_session.as_ref().is_some_and(|s| {
                s.doc == active && s.review && s.before.first().map(|b| b.0) == Some(idx)
            })
        {
            self.close_edit_session();
        }
        if let Some(idx) = out.review_focus_gained {
            if !self.selection.rows.contains(&idx) {
                self.selection.set_single(idx);
                self.pinned_rows.clear();
            } else {
                self.selection.cursor = Some(idx);
            }
            self.open_review_session(idx);
        }
        if out.undo_requested {
            self.do_undo();
        }
        if out.redo_requested {
            self.do_redo();
        }
    }

    /// Start a cell edit session on line `idx` of the active document (A-01):
    /// the states captured now become the « before » of one Edit operation
    /// when the session closes. In the unique-lines view the hidden
    /// duplicates are captured too (their translation follows).
    fn open_edit_session(&mut self, idx: usize, unique_only: bool) {
        self.close_edit_session();
        let active = self.active;
        if let Some(doc) = self.docs.get(active) {
            let Some(e) = doc.tm.entries.get(idx) else {
                return;
            };
            let mut before = vec![(idx, EntryState::capture(e))];
            if unique_only {
                let src = &e.source;
                before.extend(
                    doc.tm
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(i, e)| *i != idx && &e.source == src)
                        .map(|(i, e)| (i, EntryState::capture(e))),
                );
            }
            self.edit_session = Some(EditSession {
                doc: active,
                before,
                review: false,
            });
        }
    }

    /// Whether a modal dialog is open (keys of the grid are then inert).
    fn modal_open(&self) -> bool {
        self.alert.is_some()
            || self.pending_reload.is_some()
            || self.pending_export.is_some()
            || self.pending_output_confirm
            || self.pending_resume.is_some()
            || self.pending_save.is_some()
            || self.close_prompt.is_some()
            || self.exit_prompt
            || self.personal_prompt.is_some()
            || self.apply_dlg.open
            || self.propagate_dlg.is_some()
            || self.online.dialog_open
            || self.online.confirm_overwrite.is_some()
            || self.online.confirm_session.is_some()
            || self.import_dlg.is_some()
            || self.review_dlg.is_some()
            || self.history_confirm.is_some()
            || self.savepart_dlg.is_some()
            || self.encoding_dlg.is_some()
            || self.crc.is_open()
    }

    /// F5–F8 and Ctrl+↓ / Ctrl+↑ (A-03, D-05 of lot 1): statuses of the
    /// selection, « valider puis suivante / précédente ».
    fn status_keys(&mut self, ctx: &egui::Context) {
        if self.active_doc().is_none() || self.modal_open() {
            return;
        }
        let ctrl_f6 = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F6));
        if ctrl_f6 {
            let targets = self.selection.targets();
            self.open_propagate(&targets, false);
            return;
        }
        let (f5, f6, f7, f8, down, up) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::F5),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F6),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F7),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F8),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowUp),
            )
        });
        if f5 {
            self.set_status_selection(crate::status::Status::Validated);
            self.propose_propagation(false);
        } else if f6 {
            self.set_status_selection(crate::status::Status::Pending);
        } else if f7 {
            self.set_status_selection(crate::status::Status::Custom);
        } else if f8 {
            self.set_status_selection(crate::status::Status::Ignored);
        } else if down {
            self.validate_and_move(1);
        } else if up {
            self.validate_and_move(-1);
        }
        // Row shortcuts of the menu (M-01), only while no text field has the
        // focus (a focused cell keeps its own copy / paste / Enter).
        let text_focus = ctx.memory(|m| m.focused().is_some());
        if text_focus {
            return;
        }
        let (copy, paste, eq, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::C),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::V),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::F8),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        // ↑ ↓ PgUp PgDn Début Fin move the selection in the view (F-04).
        if let Some(key) = ctx.input_mut(grid::NavKey::consume) {
            let page = self.grid_page_rows.max(1);
            if let Some(n) = grid::nav_target(&self.visible, self.selection.cursor, key, page) {
                self.selection.set_single(n);
                self.grid_scroll_to = Some(n);
            }
            return;
        }
        let Some(cur) = self.selection.cursor else {
            return;
        };
        if copy {
            self.run_menu_action(ctx, cur, grid::MenuAction::CopyTrans);
        } else if paste {
            let many = self.selection.rows.len() > 1;
            self.run_menu_action(
                ctx,
                cur,
                if many {
                    grid::MenuAction::PasteMany
                } else {
                    grid::MenuAction::PasteOne
                },
            );
        } else if eq {
            self.run_menu_action(ctx, cur, grid::MenuAction::TransEqSource);
        } else if enter {
            self.run_menu_action(ctx, cur, grid::MenuAction::Edit);
        }
    }

    /// Give a status to the selected lines (R-A03-02): one undoable operation;
    /// lines without translation cannot become Validé / Personnalisé, lines
    /// never shown in game are never touched. Returns `(changed, skipped)`.
    fn set_status_selection(&mut self, st: crate::status::Status) -> (usize, usize) {
        let targets = self.selection.targets();
        if targets.is_empty() {
            self.status = self.i18n.t("status-keys-no-selection");
            return (0, 0);
        }
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return (0, 0);
        };
        let mut before = Vec::new();
        let mut skipped = 0usize;
        for &idx in &targets {
            let Some(e) = doc.tm.entries.get(idx) else {
                continue;
            };
            if !st.applicable_to(e) {
                if !e.unused {
                    skipped += 1;
                }
                continue;
            }
            before.push((idx, EntryState::capture(e)));
        }
        for (idx, _) in &before {
            let e = &mut doc.tm.entries[*idx];
            crate::status::set(e, st);
            // A status other than Ignoré lifts a rule's lock: « saisie » (C-08).
            crate::vmad_lock::status_by_hand(e);
        }
        let op = op_from_before(OpKind::Status, &before, &mut doc.tm.entries);
        let changed = op.changes.len();
        // One line set Ignoré by hand: the generalization (B-18).
        let ignored_one = (st == crate::status::Status::Ignored && changed == 1)
            .then(|| op.changes.first().map(|c| c.idx))
            .flatten();
        if changed > 0 {
            let mut op = op;
            op.label_arg = Some(self.i18n.t(st.legend_key()));
            doc.undo.push(op);
            // Keep the lines in view even if the status filter now excludes them.
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        let mut msg = self.i18n.t_args(
            "status-keys-done",
            &[
                ("n", &changed.to_string()),
                ("status", &self.i18n.t(st.legend_key())),
            ],
        );
        if skipped > 0 {
            msg.push_str(
                &self
                    .i18n
                    .t_arg("status-keys-skipped", "k", &skipped.to_string()),
            );
        }
        self.status = msg;
        if let Some(i) = ignored_one {
            self.rules_after_ignore(i);
        }
        (changed, skipped)
    }

    /// Ctrl+↓ / Ctrl+↑ (R-A03-03): validate the selection, then select the
    /// line after the last (or before the first) in the view; the typing
    /// focus follows when a cell was being edited.
    fn validate_and_move(&mut self, dir: i32) {
        let was_editing = self.edit_session.is_some();
        let targets = self.selection.targets();
        if targets.is_empty() {
            self.status = self.i18n.t("status-keys-no-selection");
            return;
        }
        self.set_status_selection(crate::status::Status::Validated);
        self.propose_propagation(false);
        match grid::next_in_view(&self.visible, &targets, dir) {
            Some(n) => {
                self.selection.set_single(n);
                self.grid_scroll_to = Some(n);
                if was_editing {
                    self.grid_focus_request = Some(n);
                }
            }
            None => {
                self.status = self.i18n.t(if dir > 0 {
                    "status-keys-last-line"
                } else {
                    "status-keys-first-line"
                });
            }
        }
    }

    /// Close the cell edit in progress into one `Edit` operation (with its
    /// propagated duplicates) when the text or status changed.
    pub(crate) fn close_edit_session(&mut self) {
        let Some(session) = self.edit_session.take() else {
            return;
        };
        let Some(doc) = self.docs.get_mut(session.doc) else {
            return;
        };
        // A review typed (A-17): its own operation, the translation untouched.
        if session.review {
            let op = op_from_before(OpKind::Review, &session.before, &mut doc.tm.entries);
            doc.undo.push(op);
            return;
        }
        let kind = if session.before.len() > 1 {
            OpKind::Propagate
        } else {
            OpKind::Edit
        };
        // The edited line itself is « saisie », its duplicates « propagation ».
        if let Some((first, b)) = session.before.first()
            && let Some(e) = doc.tm.entries.get_mut(*first)
        {
            // A translation typed on a line locked by a rule unlocks it
            // (C-08): the status of a typed line.
            crate::vmad_lock::unlock_by_hand(e, &b.trans, &b.prov);
            crate::provenance::settle(e, &b.trans, &b.prov, Some(crate::provenance::TYPED));
        }
        // The status of a typed line (D-94): Validé on a line without
        // translation, Proposition modifiée on an automatic one retouched;
        // the duplicates that follow the edit, the same.
        for (i, b) in &session.before {
            if let Some(e) = doc.tm.entries.get_mut(*i) {
                let was = crate::status::derive(&crate::tm::TmEntry {
                    trans: b.trans.clone(),
                    status_sst: b.status_sst.clone(),
                    status_eet: b.status_eet,
                    ..Default::default()
                });
                crate::status::after_typing(e, &b.trans, was);
            }
        }
        let op = op_from_before(kind, &session.before, &mut doc.tm.entries);
        // A propagation that only touched the edited cell is a plain edit.
        let op = if op.changes.len() <= 1 {
            crate::undo::Op {
                kind: OpKind::Edit,
                ..op
            }
        } else {
            op
        };
        let touched = op.touched();
        doc.undo.push(op);
        // The anomaly indicator follows the correction (D-07).
        self.refresh_anomaly_lines(&touched);
    }

    fn do_undo(&mut self) {
        self.close_edit_session();
        if self.all_mods_undo(false) {
            return;
        }
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        match doc.undo.undo(&mut doc.tm.entries) {
            Some(r) => self.after_undo_redo(r),
            None => self.status = self.i18n.t("undo-none"),
        }
    }

    fn do_redo(&mut self) {
        self.close_edit_session();
        if self.all_mods_undo(true) {
            return;
        }
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        match doc.undo.redo(&mut doc.tm.entries) {
            Some(r) => self.after_undo_redo(r),
            None => self.status = self.i18n.t("redo-none"),
        }
    }

    /// Scroll to the first touched line and report, with the number of touched
    /// lines the current filter hides (R-A01-04).
    fn after_undo_redo(&mut self, r: crate::undo::UndoResult) {
        self.vmad_sync();
        self.grid_scroll_to = r.first_idx();
        let hidden = r
            .touched
            .iter()
            .filter(|i| !self.visible.contains(i))
            .count();
        self.status = self.i18n.t_args(
            "undo-done",
            &[
                ("count", &r.count().to_string()),
                ("hidden", &hidden.to_string()),
                ("more", if hidden > 0 { "yes" } else { "no" }),
            ],
        );
    }

    /// Tooltip of an undo / redo button: « Annuler : <op> (<n> lignes) — Ctrl+Z ».
    fn undo_tip(&self, key: &str, peek: Option<(OpKind, Option<&str>, usize)>) -> String {
        match peek {
            Some((kind, arg, count)) => {
                let op = self.i18n.t_arg(kind.i18n_key(), "name", arg.unwrap_or(""));
                self.i18n
                    .t_args(key, &[("op", &op), ("count", &count.to_string())])
            }
            None => self.i18n.t(if key == "undo-tip" {
                "undo-none"
            } else {
                "redo-none"
            }),
        }
    }

    /// Open the book-preview window for entry `idx` of the active document.
    fn open_book_preview(&mut self, idx: usize) {
        self.book_idx = Some(idx);
        self.book_doc = self.active;
        self.book_cache = None; // force a reparse
        self.show_book = true;
        self.book_needs_center = true;
    }

    // ------------------------------------------------------------------
    // The edit window (A-02)
    // ------------------------------------------------------------------

    /// Open the edit window on line `idx` of the active document, or bring
    /// it to the front on that line (R-A02-01). The grid selects the line.
    fn open_edit_window(&mut self, idx: usize) {
        use crate::ui::edit_window::{EditTab, EditWindow};
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if idx >= doc.tm.entries.len() {
            return;
        }
        self.selection.set_single(idx);
        self.grid_scroll_to = Some(idx);
        match self.edit_win.as_mut() {
            Some(w) => w.go_to(idx),
            None => {
                self.edit_win = Some(EditWindow::new(
                    idx,
                    EditTab::from_key(&self.config.edit_tab),
                ))
            }
        }
    }

    /// Move the edit window to another line: the pending cell edit is
    /// committed, the grid selects the line and scrolls to it (R-A02-02).
    fn edit_go_to(&mut self, win: &mut crate::ui::edit_window::EditWindow, line: usize) {
        self.close_edit_session();
        self.selection.set_single(line);
        self.grid_scroll_to = Some(line);
        win.go_to(line);
    }

    /// Suivant / Précédent of the window: the view order (R-A02-02); at
    /// either end the window stays and says so.
    fn edit_step(&mut self, win: &mut crate::ui::edit_window::EditWindow, dir: i32) {
        match grid::next_in_view(&self.visible, &[win.line], dir) {
            Some(n) => self.edit_go_to(win, n),
            None => {
                let msg = self.i18n.t(if dir > 0 {
                    "status-keys-last-line"
                } else {
                    "status-keys-first-line"
                });
                self.status = msg.clone();
                win.message = Some(msg);
                win.focus_request = true;
            }
        }
    }

    /// A status from the window's buttons or F5–F8: the displayed line only
    /// (R-A02-04).
    fn edit_set_status(
        &mut self,
        win: &mut crate::ui::edit_window::EditWindow,
        st: crate::status::Status,
    ) {
        self.close_edit_session();
        self.selection.set_single(win.line);
        let (_, skipped) = self.set_status_selection(st);
        win.message = if skipped > 0 {
            Some(self.status.clone())
        } else {
            None
        };
        win.focus_request = true;
    }

    /// Ctrl+Entrée: validate the displayed line, then the next one.
    fn edit_validate_next(&mut self, win: &mut crate::ui::edit_window::EditWindow) {
        self.close_edit_session();
        self.selection.set_single(win.line);
        self.set_status_selection(crate::status::Status::Validated);
        self.propose_propagation(true);
        self.edit_step(win, 1);
    }

    /// The edit window: an independent viewport on the active document's
    /// current line, with its own keyboard handling (its input is separate
    /// from the main window's).
    fn edit_window(&mut self, ctx: &egui::Context) {
        let Some(mut win) = self.edit_win.take() else {
            return;
        };
        // The window lives with a document; none left → it closes.
        let Some(doc) = self.docs.get(self.active) else {
            let _ = self.config.save();
            return;
        };
        if doc.tm.entries.is_empty() {
            let _ = self.config.save();
            return;
        }
        // Follow the grid's current line (another document, a click…).
        if let Some(cur) = self.selection.cursor
            && cur != win.line
            && cur < doc.tm.entries.len()
        {
            win.line = cur;
            win.message = None;
        }
        if win.line >= doc.tm.entries.len() {
            win.line = self.visible.first().copied().unwrap_or(0);
        }

        let id = egui::ViewportId::from_hash_of("ximod-edit");
        let geo = self.config.edit_win;
        let just_opened = win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(560.0), geo[3].max(400.0)]))
        } else {
            None
        };
        let title = {
            let pos = crate::ui::edit_window::position_in_view(&self.visible, win.line)
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".to_string());
            // In « Tous les mods », the line's own file (R-B25-06).
            let file = if doc.all.is_some() {
                self.all_origin(win.line)
                    .map(|(d, _)| self.docs[d].file_name())
                    .unwrap_or_default()
            } else {
                doc.file_name()
            };
            self.i18n.t_args(
                "edit-title",
                &[
                    ("file", &file),
                    ("n", &pos),
                    ("total", &self.visible.len().to_string()),
                ],
            )
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([560.0, 400.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            } else if win.focus_request {
                // Entrée / double-clic while already open: to the front.
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            win.needs_place = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                self.edit_win_active_frame = true;
                close = self.edit_body(ui, vctx, &mut win);
                self.edit_win_active_frame = false;
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.edit_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            // Échap / the cross: the typed text stays (R-A02-04), committed
            // as one operation.
            self.close_edit_session();
            self.config.edit_tab = win.tab.key().to_string();
            let _ = self.config.save();
        } else {
            self.edit_win = Some(win);
        }
    }

    /// The counter line of the edit window (R-A05-02).
    fn counter_text(&self, r: &crate::length::Report) -> String {
        let chars = r.chars.to_string();
        let bytes = r.bytes.to_string();
        let nl = r.limits.newline.map(|b| {
            self.i18n.t(if b {
                "len-newline-yes"
            } else {
                "len-newline-no"
            })
        });
        match (r.limits.max_chars.or(r.limits.max_bytes), nl) {
            (Some(max), Some(nl)) => self.i18n.t_args(
                "len-counter",
                &[
                    ("chars", &chars),
                    ("max", &max.to_string()),
                    ("bytes", &bytes),
                    ("newline", &nl),
                ],
            ),
            (Some(max), None) => {
                let s = self.i18n.t_args(
                    "len-counter",
                    &[
                        ("chars", &chars),
                        ("max", &max.to_string()),
                        ("bytes", &bytes),
                        ("newline", ""),
                    ],
                );
                s.split(" · ")
                    .filter(|p| !p.trim_end().ends_with(':'))
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
            (None, Some(nl)) => self.i18n.t_args(
                "len-counter-newline",
                &[("chars", &chars), ("bytes", &bytes), ("newline", &nl)],
            ),
            (None, None) => self.i18n.t_args(
                "len-counter-nolimit",
                &[("chars", &chars), ("bytes", &bytes)],
            ),
        }
    }

    /// Body of the edit window; returns `true` to close it.
    fn edit_body(
        &mut self,
        ui: &mut egui::Ui,
        vctx: &egui::Context,
        win: &mut crate::ui::edit_window::EditWindow,
    ) -> bool {
        use crate::ui::edit_window::{self as ew, EditTab};
        let active = self.active;
        let unique_only = self.filter_unique_only;
        let show_special = self.config.show_special_chars;
        let colors = self.config.status_colors;
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        let trans_focused =
            vctx.memory(|m| m.has_focus(egui::Id::new(("ximod-edit-trans", win.line))));
        // The text area typed or selected in last (A-20: the word under its
        // cursor), kept when a menu takes the focus.
        for id in [
            egui::Id::new(("ximod-edit-src-te", win.line)),
            egui::Id::new(("ximod-edit-trans", win.line)),
        ] {
            if vctx.memory(|m| m.has_focus(id)) {
                self.edit_last_text = Some(id);
            }
        }

        // Facts about the document, read before any mutable borrow.
        let (game, src_lang, dst_lang) = {
            let doc = &self.docs[active];
            (
                doc.settings.game(),
                doc.settings.source.clone(),
                doc.settings.target.clone(),
            )
        };
        // Whether Enter may insert a line break in the line being typed.
        let newline_ok = {
            let e = &self.docs[active].tm.entries[win.line];
            ew::newline_allowed(game, &e.rec_sig, &e.field_sig)
        };
        // A line never shown in game, or held by its file's VMAD switch
        // (C-08): read-only here too.
        let unused_now = {
            let e = &self.docs[active].tm.entries[win.line];
            e.unused || crate::vmad_lock::is_file_locked(e)
        };

        // ---- Keyboard (R-A02-04): consumed before the widgets see them. ----
        // Ctrl+Alt+G: Ajouter au glossaire (B-15), before Ctrl+G.
        if vctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_GLOSSARY_ADD)) && self.add_term.is_none()
        {
            let line = win.line;
            self.open_add_term(vctx, line);
        }
        let (f9, ctrl_f9, ctrl_f6, ctrl_g) = vctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::F9),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::F9),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::F6),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::G),
            )
        });
        if ctrl_f6 && !unused_now && self.propagate_dlg.is_none() {
            let line = win.line;
            if !self.open_propagate(&[line], true) {
                win.message = Some(self.i18n.t("propagate-none"));
            }
        }
        // En ligne (D-01, R-D01-02): the line's source to the service.
        if ctrl_g && !unused_now {
            let line = win.line;
            self.online_line(vec![line]);
        }
        self.propagate_dialog(vctx);
        if let Some(msg) = self.add_term_dialog(vctx) {
            win.message = Some(msg);
        }
        // Ctrl+Alt+Z on the line of the window (A-18), before Ctrl+Z.
        if vctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_HISTORY_PREV)) {
            let line = win.line;
            self.history_revert_previous(&[line]);
            win.message = Some(self.status.clone());
        }
        let (esc, ctrl_enter, alt_right, alt_left, f5, f6, f7, f8, undo, redo, enter_blocked) =
            vctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                    i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight),
                    i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F5),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F6),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F7),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F8),
                    i.consume_shortcut(&SHORTCUT_UNDO),
                    i.consume_shortcut(&SHORTCUT_REDO) || i.consume_shortcut(&SHORTCUT_REDO_ALT),
                    !newline_ok
                        && trans_focused
                        && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
        if esc {
            if win.confirm.is_some() {
                win.confirm = None;
            } else {
                return true;
            }
        }
        // F9 / Ctrl+F9: instant translation (B-14).
        if (f9 || ctrl_f9) && !unused_now && win.confirm.is_none() {
            self.edit_instant(vctx, win, ctrl_f9);
        }
        // Ctrl+K: Concordance (A-20), with the selection or the word of the
        // area that has the focus.
        if vctx.input_mut(|i| i.consume_shortcut(&MENU_SHORTCUTS.concord)) {
            self.open_concord_window(vctx, None);
        }
        // Ctrl+M: the line's pair to the personal dictionary (B-04).
        if vctx.input_mut(|i| i.consume_shortcut(&MENU_SHORTCUTS.personal)) {
            self.close_edit_session();
            let line = win.line;
            self.add_to_personal(&[line]);
        }
        if undo {
            self.do_undo();
        } else if redo {
            self.do_redo();
        }
        if !unused_now {
            if ctrl_enter {
                self.edit_validate_next(win);
            } else if f5 {
                self.edit_set_status(win, crate::status::Status::Validated);
                self.selection.set_single(win.line);
                self.propose_propagation(true);
            } else if f6 {
                self.edit_set_status(win, crate::status::Status::Pending);
            } else if f7 {
                self.edit_set_status(win, crate::status::Status::Custom);
            } else if f8 {
                self.edit_set_status(win, crate::status::Status::Ignored);
            }
        }
        if alt_right {
            self.edit_step(win, 1);
        } else if alt_left {
            self.edit_step(win, -1);
        }
        if enter_blocked {
            win.message = Some(self.i18n.t("edit-no-newline"));
        }

        // Facts about the (possibly new) line.
        let line = win.line;
        let trans_id = egui::Id::new(("ximod-edit-trans", line));
        let src_id = egui::Id::new(("ximod-edit-src-te", line));
        self.edit_instant_confirm(vctx, win);
        let (
            rec,
            fld,
            edid,
            formid,
            index,
            plugin,
            unused,
            cur_status,
            has_trans,
            is_book,
            source_text,
        ) = {
            let e = &self.docs[active].tm.entries[line];
            (
                e.rec_sig.clone(),
                e.field_sig.clone(),
                e.edid.clone(),
                e.formid,
                e.index,
                e.plugin.clone(),
                e.unused || crate::vmad_lock::is_file_locked(e),
                crate::status::derive(e),
                !e.trans.is_empty(),
                e.rec_sig == "BOOK"
                    || crate::book::has_markup(&e.source)
                    || crate::book::has_markup(&e.trans),
                e.source.clone(),
            )
        };

        // ---- Bottom: navigation, statuses, hint (4). ----
        let mut step: i32 = 0;
        let mut status_pick: Option<crate::status::Status> = None;
        let hint = win
            .message
            .clone()
            .unwrap_or_else(|| self.i18n.t("edit-hint"));
        let is_msg = win.message.is_some();
        let prev_l = format!("{} {}", crate::icons::CARET_LEFT, self.i18n.t("edit-prev"));
        let next_l = format!("{} {}", self.i18n.t("edit-next"), crate::icons::CARET_RIGHT);
        let online_l = format!(
            "{}  {}",
            self.i18n.t("edit-online"),
            shortcut_text(ui.ctx(), &MENU_SHORTCUTS.online_line, &self.i18n)
        );
        let mut online_click = false;
        egui::TopBottomPanel::bottom("ximod-edit-bottom").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(&prev_l).clicked() {
                    step = -1;
                }
                if ui.button(&next_l).clicked() {
                    step = 1;
                }
                if ui
                    .add_enabled(!unused, egui::Button::new(&online_l))
                    .clicked()
                {
                    online_click = true;
                }
                ui.label(
                    egui::RichText::new(self.i18n.t("edit-nav-keys"))
                        .weak()
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (k, st) in [
                        crate::status::Status::Ignored,
                        crate::status::Status::Custom,
                        crate::status::Status::Pending,
                        crate::status::Status::Validated,
                    ]
                    .iter()
                    .enumerate()
                    {
                        let key = ["F8", "F7", "F6", "F5"][k];
                        let (r, g, b) = colors[st.index()];
                        let col = egui::Color32::from_rgb(r, g, b);
                        let text = format!("{}  {key}", legends[st.index()]);
                        let rich = if *st == cur_status {
                            egui::RichText::new(text).strong()
                        } else {
                            egui::RichText::new(text)
                        };
                        let mut btn = egui::Button::new(rich);
                        if *st == cur_status {
                            btn = btn
                                .fill(col)
                                .stroke(egui::Stroke::new(1.5_f32, col.gamma_multiply(0.6)));
                        } else {
                            btn = btn.stroke(egui::Stroke::new(1.0_f32, col));
                        }
                        if ui.add_enabled(!unused, btn).clicked() {
                            status_pick = Some(*st);
                        }
                    }
                });
            });
            let hint_text = if is_msg {
                egui::RichText::new(&hint).color(ui.visuals().warn_fg_color)
            } else {
                egui::RichText::new(&hint).weak().small()
            };
            ui.label(hint_text);
            ui.add_space(2.0);
        });
        // The bottom panel moves the cursor, not the maximum rectangle the
        // available height is measured from: bound it, so the areas below
        // stop above the button bar.
        let mut bounded = {
            let mut max = ui.max_rect();
            max.max.y = max.max.y.min(ui.cursor().max.y);
            ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(max)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            )
        };
        let ui = &mut bounded;
        if let Some(st) = status_pick {
            self.edit_set_status(win, st);
        }
        if online_click {
            let line = win.line;
            self.online_line(vec![line]);
        }
        if step != 0 {
            self.edit_step(win, step);
        }
        let line = win.line;

        // ---- Identity (1). ----
        let mut ident = format!("{rec}:{fld}");
        if !edid.is_empty() {
            ident.push_str(&format!("    {edid}"));
        }
        if formid != 0 {
            ident.push_str(&format!("    FormID {formid:08X}"));
        }
        if index > 1 {
            ident.push_str(&format!("    #{index}"));
        }
        if !plugin.is_empty() {
            ident.push_str(&format!("    {plugin}"));
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(&ident).monospace()).truncate());
        });
        if unused {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.t("edit-unused"));
        }

        // ---- Source (read-only, selectable) and Translation. ----
        // Labels, counter, tab row and frame margins take about 115 px; the
        // context tabs keep at least 140 px (three vocabulary rows, the
        // fragments and the shortcuts); the rest is shared between the
        // Source and the Translation.
        // In a taller window the tabs take half of the extra height (the
        // candidates of B-08, the differences of A-08 and the common lines
        // of C-04 need it).
        let avail = ui.available_height();
        let tabs_h = ((avail - 115.0) * 0.5).max(140.0);
        let rest = (avail - 115.0 - tabs_h).max(140.0);
        let src_h = (rest * 0.42).max(60.0);
        let tr_h = (rest * 0.58).max(80.0);
        // Rows that fill each area, so a click anywhere in it reaches the text.
        let row_h = ui.text_style_height(&egui::TextStyle::Body).max(1.0);
        let src_rows = ((src_h - 12.0) / row_h).floor().max(1.0) as usize;
        let tr_rows = ((tr_h - 12.0) / row_h).floor().max(1.0) as usize;
        let lang_label = |key: &str, code: &str| -> String {
            if code.trim().is_empty() {
                self.i18n.t_arg(key, "lang", "").replace(" ()", "")
            } else {
                self.i18n.t_arg(key, "lang", &self.i18n.display_name(code))
            }
        };
        ui.strong(lang_label("edit-source", &src_lang));
        let concord_label = self.i18n.t("ctx-concordance");
        let concord_key = shortcut_text(vctx, &MENU_SHORTCUTS.concord, &self.i18n);
        let mut src_concord = false;
        let glossary_label = self.i18n.t("glossary-add-menu");
        let glossary_key = shortcut_text(vctx, &SHORTCUT_GLOSSARY_ADD, &self.i18n);
        let mut src_glossary = false;
        let mut src_ctx_sel: Option<(usize, usize)> = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(src_h);
            egui::ScrollArea::vertical()
                .id_salt(("ximod-edit-src", line))
                .show(ui, |ui| {
                    let mut src: &str = &source_text;
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let (color, muted) = (
                        ui.visuals().text_color(),
                        crate::ui::special::muted(ui.visuals()),
                    );
                    let mut layouter = crate::ui::special::layouter(font.clone(), color, muted);
                    let mut edit = egui::TextEdit::multiline(&mut src)
                        .id(src_id)
                        .frame(false)
                        .desired_width(f32::INFINITY)
                        .desired_rows(src_rows);
                    if show_special {
                        edit = edit.layouter(&mut layouter);
                    }
                    // The selection before a right click (which moves the
                    // cursor), for Ajouter au glossaire (B-15).
                    let sel_before = egui::TextEdit::load_state(ui.ctx(), src_id)
                        .and_then(|st| st.cursor.char_range())
                        .map(|r| (r.primary.index, r.secondary.index))
                        .filter(|(a, b)| a != b);
                    let output = edit.show(ui);
                    if output.response.secondary_clicked() {
                        src_ctx_sel = sel_before;
                    }
                    if show_special {
                        let right = ui.max_rect().right();
                        crate::ui::special::paint_pilcrows(
                            ui,
                            &output.galley,
                            output.galley_pos,
                            &font,
                            muted,
                            right,
                        );
                    }
                    // Concordance… (A-20) and Ajouter au glossaire (B-15) on
                    // the Source area.
                    output.response.context_menu(|ui| {
                        if ui
                            .add(egui::Button::new(&concord_label).shortcut_text(&concord_key))
                            .clicked()
                        {
                            src_concord = true;
                            ui.close_menu();
                        }
                        if ui
                            .add(egui::Button::new(&glossary_label).shortcut_text(&glossary_key))
                            .clicked()
                        {
                            src_glossary = true;
                            ui.close_menu();
                        }
                    });
                });
        });
        ui.add_space(4.0);
        ui.strong(lang_label("edit-trans", &dst_lang));
        if src_concord {
            self.open_concord_window(vctx, None);
        }
        if src_ctx_sel.is_some() {
            self.edit_ctx_sel = src_ctx_sel.map(|(a, b)| (line, a, b));
        }
        if src_glossary && self.add_term.is_none() {
            self.open_add_term(vctx, line);
        }

        let menu_labels = self.menu_labels(vctx);
        let menu_row = grid::MenuRow {
            idx: line,
            cur: cur_status,
            unused,
            formid,
            has_trans,
            has_review: !self.docs[active].tm.entries[line].review.is_empty(),
            is_book,
            sel_count: 1,
            is_info: rec == "INFO",
        };
        let mut out = grid::GridOutput::default();
        let focus_now = std::mem::take(&mut win.focus_request);
        let cursor_at = win.cursor_at.take();
        {
            let doc = &mut self.docs[active];
            let e = &mut doc.tm.entries[line];
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(tr_h);
                egui::ScrollArea::vertical()
                    .id_salt(("ximod-edit-tr", line))
                    .show(ui, |ui| {
                        if unused {
                            ui.add(egui::Label::new(egui::RichText::new(&e.source).weak()).wrap());
                            return;
                        }
                        let font = egui::TextStyle::Body.resolve(ui.style());
                        let (color, muted) = (
                            ui.visuals().text_color(),
                            crate::ui::special::muted(ui.visuals()),
                        );
                        let mut layouter = crate::ui::special::layouter(font.clone(), color, muted);
                        let mut edit = egui::TextEdit::multiline(&mut e.trans)
                            .id(trans_id)
                            .frame(false)
                            .desired_width(f32::INFINITY)
                            .desired_rows(tr_rows)
                            .lock_focus(false);
                        if show_special {
                            edit = edit.layouter(&mut layouter);
                        }
                        let output = edit.show(ui);
                        if show_special {
                            let right = ui.max_rect().right();
                            crate::ui::special::paint_pilcrows(
                                ui,
                                &output.galley,
                                output.galley_pos,
                                &font,
                                muted,
                                right,
                            );
                        }
                        let resp = output.response;
                        if focus_now {
                            // Focus, cursor at the end of the text (R-A02-01)
                            // — or where an instant translation left it.
                            let mut state = output.state;
                            let end = cursor_at.unwrap_or_else(|| e.trans.chars().count());
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(end),
                                )));
                            state.store(ui.ctx(), trans_id);
                            resp.request_focus();
                        }
                        if resp.gained_focus() {
                            out.focus_gained = Some(line);
                        }
                        if resp.lost_focus() {
                            out.focus_lost = Some(line);
                        }
                        if resp.changed() {
                            out.edited = Some(line);
                        }
                        resp.context_menu(|ui| {
                            grid::row_menu(
                                ui,
                                &mut out,
                                &menu_row,
                                &menu_labels,
                                &legends,
                                &colors,
                                None,
                            )
                        });
                    });
            });
            if unique_only && let Some(idx) = out.edited {
                crate::filter::propagate_translation(&mut doc.tm.entries, idx);
            }
            if out.edited.is_some() {
                doc.undo.touch();
                win.message = None;
            }
        }
        // Counter (2) — the full A-05 counter will take this place.
        // Counter (2), A-05: chars / limit · bytes · line break; orange from
        // 90 % of the limit, red beyond; [!] on a forbidden line break.
        let (counter, level, bad_nl) = {
            let e = &self.docs[active].tm.entries[line];
            let r = crate::length::Report::of_page(e, game, self.out_page(&self.docs[active]));
            (self.counter_text(&r), r.level(), r.bad_newline)
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let text = egui::RichText::new(counter).small();
            let text = match level {
                crate::length::Level::Ok => text.weak(),
                crate::length::Level::Near => text.color(ui.visuals().warn_fg_color),
                crate::length::Level::Over => text.color(ui.visuals().error_fg_color).strong(),
            };
            ui.label(text);
            if bad_nl {
                ui.label(
                    egui::RichText::new("[!]")
                        .small()
                        .color(ui.visuals().warn_fg_color)
                        .strong(),
                )
                .on_hover_text(self.i18n.t_arg(
                    "len-newline-tip",
                    "field",
                    &format!("{rec}:{fld}"),
                ));
            }
        });

        // The Relecture field (A-17), when the column shows.
        if self.config.show_review && !unused {
            ui.label(egui::RichText::new(self.i18n.t("col-review")).strong());
            let review_id = egui::Id::new(("ximod-edit-review", line));
            let doc = &mut self.docs[active];
            let e = &mut doc.tm.entries[line];
            let differs = crate::review::differs(e);
            let resp = ui.add(
                egui::TextEdit::multiline(&mut e.review)
                    .id(review_id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .text_color_opt(differs.then(|| ui.visuals().hyperlink_color)),
            );
            if resp.gained_focus() {
                out.review_focus_gained = Some(line);
            }
            if resp.lost_focus() {
                out.review_focus_lost = Some(line);
            }
            if resp.changed() {
                doc.undo.touch();
            }
        }
        // Edit session of the Translation area (A-01), like a grid cell.
        if let Some(idx) = out.focus_lost
            && self
                .edit_session
                .as_ref()
                .is_some_and(|s| s.doc == active && s.before.first().map(|b| b.0) == Some(idx))
        {
            self.close_edit_session();
        }
        if let Some(idx) = out.focus_gained {
            self.open_edit_session(idx, unique_only);
        }
        if let Some(idx) = out.review_focus_lost
            && self.edit_session.as_ref().is_some_and(|s| {
                s.doc == active && s.review && s.before.first().map(|b| b.0) == Some(idx)
            })
        {
            self.close_edit_session();
        }
        if let Some(idx) = out.review_focus_gained {
            self.open_review_session(idx);
        }
        if out.undo_requested {
            self.do_undo();
        }
        if out.redo_requested {
            self.do_redo();
        }
        if let Some((idx, action)) = out.menu_action {
            use grid::MenuAction as A;
            match action {
                A::Edit => win.focus_request = true,
                A::ValidateNext => self.edit_validate_next(win),
                A::Status(st) => self.edit_set_status(win, st),
                // The concordance keeps the focus it takes.
                A::Concordance => self.open_concord_window(vctx, None),
                other => {
                    self.selection.set_single(idx);
                    self.run_menu_action(vctx, idx, other);
                    win.focus_request = true;
                }
            }
        }

        // ---- Context tabs (3). ----
        ui.add_space(4.0);
        // The lines of the same source (C-04): their number is in the tab.
        self.common_update(line);
        let common_n = self.common_count().to_string();
        let common_tip = self.i18n.t("common-title");
        ui.horizontal(|ui| {
            for tab in EditTab::AVAILABLE {
                let label = match tab {
                    EditTab::Record => self.i18n.t("edit-tab-record"),
                    EditTab::Aliases => self.i18n.t("edit-tab-aliases"),
                    EditTab::Vocab => self.i18n.t("edit-tab-vocab"),
                    EditTab::Common => self.i18n.t_arg("edit-tab-common", "n", &common_n),
                    EditTab::Script => self.i18n.t("edit-tab-script"),
                    EditTab::Dialogue => self.i18n.t("edit-tab-dialogue"),
                    EditTab::History => self.i18n.t("edit-tab-history"),
                };
                let mut resp = ui.selectable_label(win.tab == tab, label);
                if tab == EditTab::Common {
                    resp = resp.on_hover_text(&common_tip);
                }
                if resp.clicked() {
                    win.tab = tab;
                }
            }
        });
        let mut go: Option<usize> = None;
        let mut fix: Option<bool> = None;
        let mut use_text: Option<(String, String)> = None;
        let mut dialog_action: Option<crate::ui::dialog_window::DialogAction> = None;
        let mut common_action: Option<app_common::CommonAction> = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height().max(40.0));
            match win.tab {
                EditTab::Vocab => {
                    use_text = self.vocab_tab(ui, line);
                }
                EditTab::Common => {
                    common_action = self.common_tab(ui, line);
                }
                EditTab::Script => {
                    let copy = egui::ScrollArea::vertical()
                        .id_salt("ximod-script-tab")
                        .show(ui, |ui| self.script_tab(ui, line))
                        .inner;
                    if let Some(v) = copy {
                        self.config.scripts_copy_to_source = v;
                        let _ = self.config.save();
                    }
                }
                EditTab::Dialogue => {
                    dialog_action = self.dialog_tab(ui, line);
                }
                EditTab::History => self.history_tab(ui, line),
                EditTab::Aliases => {
                    // A-07: the comparison and its repairs.
                    let e = &self.docs[active].tm.entries[line];
                    let report = crate::tags::Report::of(e, game);
                    let (ok_c, warn_c, err_c) = (
                        crate::ui::theme::Palette::of(ui.visuals()).success,
                        ui.visuals().warn_fg_color,
                        ui.visuals().error_fg_color,
                    );
                    if report.nothing_to_compare {
                        ui.label(egui::RichText::new(self.i18n.t("alias-none")).weak());
                    } else if report.is_clean() {
                        ui.colored_label(ok_c, self.i18n.t("alias-ok"));
                    } else {
                        for t in &report.missing_tags {
                            ui.colored_label(err_c, self.i18n.t_arg("alias-missing", "tag", t));
                        }
                        for t in &report.unknown_tags {
                            ui.colored_label(err_c, self.i18n.t_arg("alias-unknown", "tag", t));
                        }
                        for n in &report.missing_numbers {
                            ui.colored_label(
                                err_c,
                                self.i18n.t_arg("alias-number-missing", "n", n),
                            );
                        }
                        for n in &report.unknown_numbers {
                            ui.colored_label(
                                err_c,
                                self.i18n.t_arg("alias-number-unknown", "n", n),
                            );
                        }
                        if report.order_differs {
                            ui.colored_label(warn_c, self.i18n.t("alias-order"));
                        }
                    }
                    let tags_bad = !report.missing_tags.is_empty()
                        || !report.unknown_tags.is_empty()
                        || report.order_differs;
                    let nums_bad =
                        !report.missing_numbers.is_empty() || !report.unknown_numbers.is_empty();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                tags_bad && !unused,
                                egui::Button::new(self.i18n.t("alias-fix")),
                            )
                            .clicked()
                        {
                            fix = Some(true);
                        }
                        if ui
                            .add_enabled(
                                nums_bad && !unused,
                                egui::Button::new(self.i18n.t("alias-fix-numbers")),
                            )
                            .clicked()
                        {
                            fix = Some(false);
                        }
                    });
                }
                EditTab::Record => {
                    let doc = &self.docs[active];
                    // Provenance of the translation (B-02).
                    if let Some(tip) = self.origin_tip(doc, line) {
                        ui.label(egui::RichText::new(tip).weak().small());
                    }
                    // The comment (C-03, R-C03-05): the author's, then the
                    // generated text when the dialogue map is built.
                    if let Some(e) = doc.tm.entries.get(line) {
                        let g = crate::comments::Generator {
                            map: self.dialogs.map.as_deref(),
                            i18n: &self.i18n,
                            voices: self.dialogs.voices.clone(),
                        };
                        let text = g.full(e);
                        if !text.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(self.i18n.t_arg(
                                        "edit-comment-label",
                                        "text",
                                        &text,
                                    ))
                                    .small(),
                                )
                                .wrap(),
                            );
                        }
                    }
                    if formid == 0 {
                        ui.label(egui::RichText::new(self.i18n.t("edit-record-no-id")).weak());
                        return;
                    }
                    let sibs = ew::record_siblings(&doc.tm.entries, line);
                    if sibs.is_empty() {
                        ui.label(egui::RichText::new(self.i18n.t("edit-record-empty")).weak());
                        return;
                    }
                    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
                    let avail_w = ui.available_width();
                    let text_w = ((avail_w - 120.0 - 90.0 - 24.0) / 2.0).max(80.0);
                    egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(egui_extras::Column::exact(120.0))
                        .column(egui_extras::Column::exact(90.0))
                        .column(egui_extras::Column::exact(text_w))
                        .column(egui_extras::Column::remainder())
                        .body(|body| {
                            body.rows(row_h, sibs.len(), |mut row| {
                                let i = sibs[row.index()];
                                let o = &doc.tm.entries[i];
                                let st = crate::status::derive(o);
                                let (r, g, b) = colors[st.index()];
                                let col = egui::Color32::from_rgb(r, g, b);
                                row.col(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(12.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, 2.0, col);
                                    ui.add(
                                        egui::Label::new(&legends[st.index()])
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    let fld_l = if o.index > 1 {
                                        format!("{} #{}", o.field_sig, o.index)
                                    } else {
                                        o.field_sig.clone()
                                    };
                                    if ui.link(egui::RichText::new(fld_l).monospace()).clicked() {
                                        go = Some(i);
                                    }
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(ew::one_line(&o.source, 200))
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(ew::one_line(&o.trans, 200))
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                            });
                        });
                }
            }
        });
        if let Some(i) = go {
            self.edit_go_to(win, i);
        }
        if let Some(tags) = fix {
            let msg = self.fix_lines(&[line], tags);
            win.message = Some(msg);
            win.focus_request = true;
        }
        if let Some((text, prov)) = use_text {
            self.vocab_use(line, text, prov);
            win.message = Some(self.i18n.t("vocab-used"));
            win.focus_request = true;
        }
        match common_action {
            Some(app_common::CommonAction::Apply) => {
                if self.propagate_dlg.is_none() && !self.open_propagate(&[line], true) {
                    win.message = Some(self.i18n.t("propagate-none"));
                }
            }
            Some(app_common::CommonAction::GoTo(serial, idx)) => {
                self.common_go_to(win, serial, idx);
            }
            None => {}
        }
        if let Some(a) = dialog_action {
            match a {
                crate::ui::dialog_window::DialogAction::OpenLine(l) => {
                    self.edit_go_to(win, l);
                }
                crate::ui::dialog_window::DialogAction::OpenWindow => {
                    self.open_dialog_window();
                }
                other => self.dialog_apply(vctx, other),
            }
        }
        false
    }

    /// « Rétablir les alias » (`tags`) or « Rétablir les nombres » on some
    /// lines of the active document (R-A07-04): one undoable operation;
    /// returns the message to show.
    fn fix_lines(&mut self, lines: &[usize], tags: bool) -> String {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return String::new();
        };
        let game = doc.settings.game();
        let mut before = Vec::new();
        let (mut replaced, mut appended) = (0usize, 0usize);
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            if e.unused || e.trans.trim().is_empty() {
                continue;
            }
            let set = crate::tags::TagSet::for_entry(e, game);
            let fixed = if tags {
                crate::tags::fix_tags(&e.source, &e.trans, &set)
            } else {
                crate::tags::fix_numbers(&e.source, &e.trans, &set)
            };
            if fixed.text == e.trans {
                continue;
            }
            before.push((i, EntryState::capture(e)));
            replaced += fixed.replaced;
            appended += fixed.appended;
            let e = &mut doc.tm.entries[i];
            e.trans = fixed.text;
            e.set_translated_flags(true);
        }
        let mut op = op_from_before(OpKind::Fix, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            op.label_arg = Some(self.i18n.t(if tags {
                "alias-fix"
            } else {
                "alias-fix-numbers"
            }));
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        let msg = self.i18n.t_args(
            if tags {
                "alias-fixed"
            } else {
                "alias-fixed-numbers"
            },
            &[
                ("n", &replaced.to_string()),
                ("appended", if appended > 0 { "yes" } else { "no" }),
                ("m", &appended.to_string()),
            ],
        );
        self.status = msg.clone();
        msg
    }

    // ------------------------------------------------------------------
    // Rechercher et remplacer (A-11)
    // ------------------------------------------------------------------

    /// Ctrl+H / Outils ▸ Rechercher et remplacer…: open the window (or bring
    /// it back), prefilled with the selection of the field being typed in,
    /// else the filter text (R-A11-01).
    fn open_replace_window(&mut self, ctx: &egui::Context) {
        if self.docs.is_empty() {
            return;
        }
        let mut seed = String::new();
        if let Some(fid) = ctx.memory(|m| m.focused())
            && let Some(state) = egui::TextEdit::load_state(ctx, fid)
            && let Some(range) = state.cursor.char_range()
        {
            let (a, b) = (
                range.primary.index.min(range.secondary.index),
                range.primary.index.max(range.secondary.index),
            );
            if a < b
                && let Some(e) = self
                    .selection
                    .cursor
                    .and_then(|c| self.docs.get(self.active).and_then(|d| d.tm.entries.get(c)))
            {
                seed = e.trans.chars().skip(a).take(b - a).collect();
            }
        }
        if seed.is_empty() {
            seed = self.filter.trim().to_string();
        }
        if !seed.is_empty() {
            self.replace.find = seed;
            self.replace.mode = crate::replace::Mode::Normal;
            self.replace.compile();
        }
        if !self.replace.open {
            self.replace.needs_place = true;
        }
        self.replace.open = true;
        self.replace.focus_find = true;
    }

    /// The lines of the window's scope, in view order for the view and
    /// the selection, document order otherwise (R-A11-02).
    fn replace_scope_lines(&self) -> Vec<usize> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        match self.replace.scope {
            crate::replace::Scope::Document => (0..doc.tm.entries.len()).collect(),
            crate::replace::Scope::View => self.visible.clone(),
            crate::replace::Scope::Selection => {
                let sel = self.selection.targets();
                let mut in_view: Vec<usize> = self
                    .visible
                    .iter()
                    .copied()
                    .filter(|i| sel.contains(i))
                    .collect();
                for i in sel {
                    if !in_view.contains(&i) {
                        in_view.push(i);
                    }
                }
                in_view
            }
        }
    }

    /// The matching lines of the scope, in its order.
    fn replace_matches(&self) -> Vec<usize> {
        let (Some(doc), Some(m)) = (self.docs.get(self.active), &self.replace.matcher) else {
            return Vec::new();
        };
        crate::replace::matching_lines(
            &doc.tm.entries,
            &self.replace_scope_lines(),
            self.replace.target,
            m,
        )
    }

    /// F3 / Maj+F3: select the next / previous matching line in the grid,
    /// which scrolls; the edit window follows when open (R-A11-04).
    fn replace_step(&mut self, dir: i32) {
        let matches = self.replace_matches();
        match crate::ui::replace_window::next_match(&matches, self.selection.cursor, dir) {
            Some(n) => {
                self.close_edit_session();
                self.selection.set_single(n);
                self.grid_scroll_to = Some(n);
                if let Some(w) = self.edit_win.as_mut() {
                    w.go_to(n);
                }
                self.replace.message = None;
            }
            None => {
                let msg = self.i18n.t("replace-none");
                self.replace.message = Some(msg.clone());
                self.status = msg;
            }
        }
    }

    /// Replace in the given lines with the window's pattern: one undoable
    /// operation; Ignoré lines are never touched, Validé lines go En
    /// attente (R-A11-05). Returns `(lines replaced, lines set pending)`.
    fn replace_apply(
        &mut self,
        lines: &[usize],
        rules: &[crate::replace::Matcher],
    ) -> (usize, usize, Vec<usize>) {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return (0, 0, Vec::new());
        };
        let mut before = Vec::new();
        let mut per_rule = vec![0usize; rules.len()];
        let mut pending = 0usize;
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            if e.unused || crate::status::derive(e) == crate::status::Status::Ignored {
                continue;
            }
            let mut text = e.trans.clone();
            let mut touched = false;
            for (k, m) in rules.iter().enumerate() {
                let (t, n) = m.replace_all(&text);
                if n > 0 {
                    per_rule[k] += n;
                    text = t;
                    touched = true;
                }
            }
            if !touched || text == e.trans {
                continue;
            }
            let was_validated = crate::status::derive(e) == crate::status::Status::Validated;
            before.push((i, EntryState::capture(e)));
            let e = &mut doc.tm.entries[i];
            e.trans = text;
            e.set_translated_flags(!e.trans.is_empty());
            if was_validated {
                crate::status::set(e, crate::status::Status::Pending);
                pending += 1;
            }
        }
        let mut op = op_from_before(OpKind::Replace, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            op.label_arg = Some(self.i18n.t("op-replace-arg"));
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        (n, pending, per_rule)
    }

    /// « Remplacer »: the current line when it matches, then the next one.
    fn replace_current(&mut self) {
        let Some(m) = self.replace.matcher.clone() else {
            return;
        };
        let matches = self.replace_matches();
        let cur = self.selection.cursor.filter(|c| matches.contains(c));
        if let Some(c) = cur {
            let (n, pending, _) = self.replace_apply(&[c], &[m]);
            self.replace.remember();
            self.notify_ok(self.i18n.t_args(
                "replace-done",
                &[("n", &n.to_string()), ("pending", &pending.to_string())],
            ));
        }
        self.replace_step(1);
    }

    /// « Tout remplacer »: every matching line of the scope, one operation.
    fn replace_all(&mut self) {
        let Some(m) = self.replace.matcher.clone() else {
            return;
        };
        let matches = self.replace_matches();
        if matches.is_empty() {
            let msg = self.i18n.t("replace-none");
            self.replace.message = Some(msg.clone());
            self.status = msg;
            return;
        }
        let (n, pending, _) = self.replace_apply(&matches, &[m]);
        self.replace.remember();
        let msg = self.i18n.t_args(
            "replace-done",
            &[("n", &n.to_string()), ("pending", &pending.to_string())],
        );
        self.replace.message = Some(msg.clone());
        self.notify_ok(msg);
    }

    /// The window itself.
    fn replace_window(&mut self, ctx: &egui::Context) {
        if !self.replace.open {
            return;
        }
        if self.docs.is_empty() {
            self.replace.open = false;
            let _ = self.config.save();
            return;
        }
        // Debounced preview (R-A11-03).
        if let Some(at) = self.replace.typed_at {
            if ctx.input(|i| i.time) - at >= 0.25 {
                self.replace.compile();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
        }
        // The matrices of the target language (B-17).
        self.replace_matrices_refresh();
        // Recompute the preview when the document, the scope or the pattern moved.
        let scope_lines = self.replace_scope_lines();
        let revision = self
            .docs
            .get(self.active)
            .map(|d| d.undo.revision)
            .unwrap_or(0);
        let key = (
            revision,
            scope_lines.len(),
            scope_lines.iter().sum::<usize>(),
        );
        if self.replace.preview_key != Some(key) {
            self.replace.preview_key = Some(key);
            self.replace.preview = match (&self.replace.matcher, self.docs.get(self.active)) {
                (Some(m), Some(doc)) => {
                    crate::replace::preview(&doc.tm.entries, &scope_lines, self.replace.target, m)
                }
                _ => crate::replace::Preview::default(),
            };
        }

        let id = egui::ViewportId::from_hash_of("ximod-replace");
        let geo = self.config.replace_win;
        let just_opened = self.replace.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(520.0), geo[3].max(360.0)]))
        } else {
            None
        };
        let file = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let title = self.i18n.t_arg("replace-title", "file", &file);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([520.0, 360.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            } else if self.replace.focus_find {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.replace.needs_place = false;
            let (esc, f3, shift_f3, enter) = vctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F3),
                    i.consume_key(egui::Modifiers::SHIFT, egui::Key::F3),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
            if esc {
                close = true;
            }
            if enter {
                self.replace.compile();
                self.replace_step(1);
            }
            if shift_f3 {
                self.replace_step(-1);
            } else if f3 {
                self.replace_step(1);
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.replace_body(ui) {
                    close = true;
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.replace_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.replace.open = false;
            let _ = self.config.save();
        }
    }

    /// Body of the window; returns `true` to close it.
    fn replace_body(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::replace::{Mode, Scope, Target};
        // 0 — a replacement matrix on the scope (B-17, figure B.2).
        let scope = self.replace_scope_lines();
        self.replace_matrix_ui(ui, &scope);
        ui.add_space(4.0);
        let t = |k: &str| self.i18n.t(k);
        let mut close = false;
        let lw = 110.0;
        let field_w = (ui.available_width() - lw - 40.0).max(200.0);
        ui.label(egui::RichText::new(t("replace-or-rule")).strong());

        // 1 — pattern and replacement, each with its history.
        for which in 0..2 {
            ui.horizontal(|ui| {
                let (label, id) = if which == 0 {
                    (t("replace-find"), egui::Id::new("ximod-replace-find"))
                } else {
                    (t("replace-with"), egui::Id::new("ximod-replace-with"))
                };
                ui.add_sized(
                    [lw, ui.spacing().interact_size.y],
                    egui::Label::new(egui::RichText::new(label).strong()),
                );
                let (text, history) = if which == 0 {
                    (&mut self.replace.find, &self.replace.find_history)
                } else {
                    (&mut self.replace.with, &self.replace.with_history)
                };
                let mut edit = egui::TextEdit::singleline(text)
                    .id(id)
                    .desired_width(field_w)
                    .font(egui::TextStyle::Monospace);
                if which == 0 && self.replace.error.is_some() {
                    edit = edit.text_color(ui.visuals().error_fg_color);
                }
                let resp = ui.add(edit);
                if which == 0 && self.replace.focus_find {
                    self.replace.focus_find = false;
                    resp.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                        let end = text.chars().count();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(end),
                            )));
                        state.store(ui.ctx(), id);
                    }
                }
                if resp.changed() {
                    self.replace.typed_at = Some(ui.input(|i| i.time));
                }
                // ▾ : the last ten patterns of the session.
                let mut pick: Option<String> = None;
                ui.add_enabled_ui(!history.is_empty(), |ui| {
                    ui.menu_button(crate::icons::CARET_DOWN, |ui| {
                        for h in history.iter() {
                            if ui.button(h).clicked() {
                                pick = Some(h.clone());
                                ui.close_menu();
                            }
                        }
                    });
                });
                if let Some(p) = pick {
                    *text = p;
                    self.replace.typed_at = Some(ui.input(|i| i.time));
                }
            });
        }
        ui.add_space(6.0);

        // 2 — target, scope, options, mode.
        let mut recompile = false;
        ui.columns(2, |cols| {
            cols[0].group(|ui| {
                ui.strong(t("filter-in"));
                ui.horizontal_wrapped(|ui| {
                    for tg in Target::ALL {
                        if ui
                            .radio_value(&mut self.replace.target, tg, t(tg.i18n_key()))
                            .changed()
                        {
                            recompile = true;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(t("replace-scope"));
                    let n_view = self.visible.len();
                    let n_sel = self.selection.targets().len();
                    let label = |s: Scope| match s {
                        Scope::Document => t(s.i18n_key()),
                        Scope::View => self.i18n.paren(&t(s.i18n_key()), &n_view.to_string()),
                        Scope::Selection => self.i18n.paren(&t(s.i18n_key()), &n_sel.to_string()),
                    };
                    egui::ComboBox::from_id_salt("ximod-replace-scope")
                        .selected_text(label(self.replace.scope))
                        .show_ui(ui, |ui| {
                            for s in Scope::ALL {
                                if ui
                                    .selectable_value(&mut self.replace.scope, s, label(s))
                                    .changed()
                                {
                                    self.replace.preview_key = None;
                                }
                            }
                        });
                });
            });
            cols[1].group(|ui| {
                ui.strong(t("replace-options"));
                if ui
                    .checkbox(&mut self.replace.case_sensitive, t("replace-case"))
                    .changed()
                {
                    recompile = true;
                }
                if ui
                    .checkbox(&mut self.replace.whole, t("replace-whole"))
                    .changed()
                {
                    recompile = true;
                }
                ui.horizontal(|ui| {
                    ui.label(t("replace-mode"));
                    egui::ComboBox::from_id_salt("ximod-replace-mode")
                        .selected_text(t(self.replace.mode.i18n_key()))
                        .show_ui(ui, |ui| {
                            for m in Mode::ALL {
                                if ui
                                    .selectable_value(&mut self.replace.mode, m, t(m.i18n_key()))
                                    .changed()
                                {
                                    recompile = true;
                                }
                            }
                        });
                });
                ui.label(egui::RichText::new(t("replace-mode-hint")).weak().small());
            });
        });
        if recompile {
            self.replace.compile();
        }
        ui.add_space(6.0);

        // 3 — preview.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(err) = &self.replace.error {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    self.i18n.t_arg("filter-regex-error", "error", err),
                );
            } else if self.replace.matcher.is_none() {
                ui.label(egui::RichText::new(t("replace-type-hint")).weak());
            } else if self.replace.preview.lines == 0 {
                ui.label(egui::RichText::new(t("replace-none")).weak());
            } else {
                let p = &self.replace.preview;
                let (before, after) = p
                    .first
                    .as_ref()
                    .map(|(_, b, a)| {
                        (
                            crate::ui::edit_window::one_line(b, 60),
                            crate::ui::edit_window::one_line(a, 60),
                        )
                    })
                    .unwrap_or_default();
                ui.add(
                    egui::Label::new(self.i18n.t_args(
                        "replace-preview",
                        &[
                            ("n", &p.occurrences.to_string()),
                            ("m", &p.lines.to_string()),
                            ("before", &before),
                            ("after", &after),
                        ],
                    ))
                    .wrap(),
                );
            }
            if !self.replace.target.replaceable() {
                ui.label(egui::RichText::new(t("replace-readonly")).weak().small());
            }
        });
        ui.add_space(4.0);

        // 4 — special characters.
        if ui
            .checkbox(&mut self.config.show_special_chars, t("opt-special-chars"))
            .changed()
        {
            let _ = self.config.save();
        }
        ui.add_space(6.0);

        // Buttons.
        let can = self.replace.can_replace();
        let readonly = t("replace-readonly");
        let mut action: Option<u8> = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.replace.matcher.is_some(),
                    egui::Button::new(t("replace-next")).shortcut_text(key_text(
                        ui.ctx(),
                        &self.i18n,
                        egui::Modifiers::NONE,
                        egui::Key::F3,
                    )),
                )
                .clicked()
            {
                action = Some(1);
            }
            if ui
                .add_enabled(
                    self.replace.matcher.is_some(),
                    egui::Button::new(t("replace-prev")).shortcut_text(key_text(
                        ui.ctx(),
                        &self.i18n,
                        egui::Modifiers::SHIFT,
                        egui::Key::F3,
                    )),
                )
                .clicked()
            {
                action = Some(2);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t("btn-close")).clicked() {
                    close = true;
                }
                let r = ui.add_enabled(
                    can,
                    egui::Button::new(egui::RichText::new(t("replace-all")).strong()),
                );
                if (if can {
                    r
                } else {
                    r.on_disabled_hover_text(&readonly)
                })
                .clicked()
                {
                    action = Some(4);
                }
                let r = ui.add_enabled(can, egui::Button::new(t("replace-one")));
                if (if can {
                    r
                } else {
                    r.on_disabled_hover_text(&readonly)
                })
                .clicked()
                {
                    action = Some(3);
                }
            });
        });
        if let Some(msg) = &self.replace.message {
            ui.label(egui::RichText::new(msg).small());
        }
        match action {
            Some(1) => self.replace_step(1),
            Some(2) => self.replace_step(-1),
            Some(3) => self.replace_current(),
            Some(4) => self.replace_all(),
            _ => {}
        }
        // « Ajouter à une matrice… » (B-17) replaces the rules files.
        ui.horizontal(|ui| self.replace_add_to_matrix_menu(ui));
        close
    }

    // ------------------------------------------------------------------
    // Vérifications qualité (D-05)
    // ------------------------------------------------------------------

    /// The checks ticked, from `[Quality] Checks`.
    fn quality_checks(&self) -> Vec<crate::quality::Check> {
        crate::quality::Check::parse_set(&self.config.quality_checks)
    }

    /// F12 / Outils ▸ Vérifications qualité…: open the window and run the
    /// checks with the current boxes (a second F12 runs them again).
    fn open_quality_window(&mut self, run: bool) {
        if self.docs.is_empty() {
            return;
        }
        if !self.quality.open {
            self.quality.needs_place = true;
        }
        self.quality.open = true;
        if run {
            self.quality_run();
        }
    }

    /// The lines of the window's scope.
    fn quality_scope_lines(&self) -> Vec<usize> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        match self.quality.scope {
            crate::replace::Scope::Document => (0..doc.tm.entries.len()).collect(),
            crate::replace::Scope::View => self.visible.clone(),
            crate::replace::Scope::Selection => self.selection.targets(),
        }
    }

    /// Start the checks on a worker thread; the window shows the progress
    /// and takes the report when it arrives (R-D05-06).
    fn quality_run(&mut self) {
        if self.quality.job.is_some() || self.docs.get(self.active).is_none() {
            return;
        }
        self.close_edit_session();
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let entries = doc.tm.entries.clone();
        let lines = self.quality_scope_lines();
        let checks = self.quality_checks();
        let game = doc.settings.game();
        let page = self.out_page(doc);
        let serial = doc.serial;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let report = crate::quality::run_with_page(
                &entries,
                &lines,
                &checks,
                game,
                page,
                |done, total| {
                    tx2.send(crate::ui::quality_window::Msg::Progress(done, total))
                        .is_ok()
                },
            );
            let _ = tx.send(crate::ui::quality_window::Msg::Done(report));
        });
        self.quality.job = Some(crate::ui::quality_window::Job {
            rx,
            progress: 0.0,
            serial,
        });
        self.quality.message = None;
    }

    /// Collect the worker's messages.
    fn quality_poll(&mut self, ctx: &egui::Context) {
        let Some(job) = self.quality.job.as_mut() else {
            return;
        };
        let mut done: Option<crate::quality::Report> = None;
        loop {
            match job.rx.try_recv() {
                Ok(crate::ui::quality_window::Msg::Progress(d, t)) => {
                    job.progress = if t == 0 { 1.0 } else { d as f32 / t as f32 }
                }
                Ok(crate::ui::quality_window::Msg::Done(r)) => {
                    done = Some(r);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.quality.job = None;
                    return;
                }
            }
        }
        match done {
            Some(report) => {
                let serial = job.serial;
                self.quality.job = None;
                self.quality.selected.clear();
                let n_lines = report.lines().len();
                let msg = if report.findings.is_empty() {
                    self.i18n
                        .t_arg("quality-none", "lines", &report.lines_checked.to_string())
                } else {
                    self.i18n.t_args(
                        "quality-result",
                        &[
                            ("anomalies", &report.findings.len().to_string()),
                            ("lines", &n_lines.to_string()),
                        ],
                    )
                };
                self.quality.message = Some(msg.clone());
                self.status = msg.clone();
                self.anomaly_serial = Some(serial);
                self.set_anomalies_from(&report);
                let level = if report.findings.is_empty() {
                    crate::journal::Level::Info
                } else {
                    crate::journal::Level::Warn
                };
                let rows: Vec<usize> = report.lines().into_iter().collect();
                if let Some(idx) = self.docs.iter().position(|d| d.serial == serial) {
                    self.journal_rows(level, crate::journal::Kind::Check, idx, &msg, rows);
                }
                self.quality.report = Some(report);
                self.quality.sort_rows();
            }
            None => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        }
    }

    /// Rebuild the grid's indicator map and the filter's set from a report.
    fn set_anomalies_from(&mut self, report: &crate::quality::Report) {
        let mut map: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        for (line, findings) in report.by_line() {
            let text: Vec<String> = findings.iter().map(|f| self.finding_text(f)).collect();
            map.insert(line, text.join("\n"));
        }
        self.anomaly_set =
            crate::filter::AnomalySet(std::sync::Arc::new(map.keys().copied().collect()));
        self.anomalies = map;
    }

    /// Recompute the per-line checks of a few lines after an edit (D-07):
    /// the indicator follows the correction without a full run. The
    /// grouped check (identical sources) waits for the next run.
    fn refresh_anomaly_lines(&mut self, lines: &[usize]) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if self.anomaly_serial != Some(doc.serial) || lines.is_empty() {
            return;
        }
        let checks: Vec<crate::quality::Check> = self
            .quality_checks()
            .into_iter()
            .filter(|c| *c != crate::quality::Check::Inconsistent)
            .collect();
        let report = crate::quality::run_with_page(
            &doc.tm.entries,
            lines,
            &checks,
            doc.settings.game(),
            self.out_page(doc),
            |_, _| true,
        );
        let by_line = report.by_line();
        let mut changed = false;
        for &l in lines {
            // Keep a group finding of the last run on that line.
            let group: Option<String> = self.quality.report.as_ref().and_then(|r| {
                let texts: Vec<String> = r
                    .findings
                    .iter()
                    .filter(|f| matches!(&f.detail, crate::quality::Detail::Inconsistent { lines, .. } if lines.contains(&l)))
                    .map(|f| self.finding_text(f))
                    .collect();
                if texts.is_empty() { None } else { Some(texts.join("\n")) }
            });
            let mut parts: Vec<String> = by_line
                .get(&l)
                .map(|v| v.iter().map(|f| self.finding_text(f)).collect())
                .unwrap_or_default();
            if let Some(g) = group {
                parts.push(g);
            }
            let before = self.anomalies.get(&l).cloned();
            if parts.is_empty() {
                self.anomalies.remove(&l);
            } else {
                self.anomalies.insert(l, parts.join("\n"));
            }
            if before != self.anomalies.get(&l).cloned() {
                changed = true;
            }
        }
        if changed {
            self.anomaly_set = crate::filter::AnomalySet(std::sync::Arc::new(
                self.anomalies.keys().copied().collect(),
            ));
        }
        // The results table follows too: the per-line findings of those
        // lines are replaced by the fresh ones (the groups stay).
        if let Some(r) = self.quality.report.as_mut() {
            let before = r.findings.len();
            r.findings.retain(|f| {
                !lines.contains(&f.line)
                    || matches!(f.detail, crate::quality::Detail::Inconsistent { .. })
            });
            let removed = before - r.findings.len();
            let added = report.findings.len();
            r.findings.extend(report.findings);
            if removed > 0 || added > 0 {
                self.quality.selected.clear();
                self.quality.sort_rows();
            }
        }
    }

    /// The text of a finding (table cell, tooltip).
    fn finding_text(&self, f: &crate::quality::Finding) -> String {
        crate::quality::finding_text(&self.i18n, f, |t| crate::ui::edit_window::one_line(t, 40))
    }

    /// « Unifier sur la plus fréquente » (R-D05-05): every line of the
    /// group takes the majority variant, Ignoré lines aside, one operation.
    fn quality_unify(&mut self, finding: usize) {
        let Some(report) = &self.quality.report else {
            return;
        };
        let Some(f) = report.findings.get(finding) else {
            return;
        };
        let crate::quality::Detail::Inconsistent {
            variants, lines, ..
        } = &f.detail
        else {
            return;
        };
        let (Some((best, _)), lines) = (variants.first().cloned(), lines.clone()) else {
            return;
        };
        self.close_edit_session();
        let label = self.i18n.t("quality-unify");
        let n = {
            let Some(doc) = self.docs.get_mut(self.active) else {
                return;
            };
            let mut before = Vec::new();
            for &i in &lines {
                let Some(e) = doc.tm.entries.get(i) else {
                    continue;
                };
                if e.unused
                    || e.trans == best
                    || crate::status::derive(e) == crate::status::Status::Ignored
                {
                    continue;
                }
                let was_validated = crate::status::derive(e) == crate::status::Status::Validated;
                before.push((i, EntryState::capture(e)));
                let e = &mut doc.tm.entries[i];
                e.trans = best.clone();
                e.set_translated_flags(!e.trans.is_empty());
                if was_validated {
                    crate::status::set(e, crate::status::Status::Pending);
                }
            }
            let mut op = op_from_before(OpKind::Replace, &before, &mut doc.tm.entries);
            let n = op.changes.len();
            if n > 0 {
                op.label_arg = Some(label);
                doc.undo.push(op);
                self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
            }
            n
        };
        self.notify_ok(self.i18n.t_arg("quality-unified", "n", &n.to_string()));
        // The group is resolved: drop it from the report first, then the
        // lines' indicators are rebuilt without it.
        if let Some(r) = self.quality.report.as_mut() {
            r.findings.remove(finding);
            self.quality.selected.clear();
            self.quality.sort_rows();
        }
        self.refresh_anomaly_lines(&lines);
    }

    /// The window.
    fn quality_window(&mut self, ctx: &egui::Context) {
        if !self.quality.open {
            return;
        }
        if self.docs.is_empty() {
            self.quality.open = false;
            let _ = self.config.save();
            return;
        }
        self.quality_poll(ctx);
        let id = egui::ViewportId::from_hash_of("ximod-quality");
        let geo = self.config.quality_win;
        let just_opened = self.quality.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(560.0), geo[3].max(400.0)]))
        } else {
            None
        };
        let file = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let title = self.i18n.t_arg("quality-title", "file", &file);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([560.0, 400.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.quality.needs_place = false;
            let (esc, f12, enter) = vctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::F12),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
            if esc {
                close = true;
            }
            if f12 {
                self.quality_run();
            }
            if enter && let Some(&row) = self.quality.selected.iter().next() {
                self.quality_open_line(row);
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.quality_body(ui) {
                    close = true;
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.quality_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.quality.open = false;
            let _ = self.config.save();
        }
    }

    /// Open the line of a result row in the edit window.
    fn quality_open_line(&mut self, row: usize) {
        let Some(report) = &self.quality.report else {
            return;
        };
        let Some(&fi) = self.quality.rows.get(row) else {
            return;
        };
        let Some(f) = report.findings.get(fi) else {
            return;
        };
        let line = f.line;
        self.open_edit_window(line);
    }

    /// Body of the window; returns `true` to close it.
    fn quality_body(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::quality::Check;
        use crate::replace::Scope;
        // The labels first: the closure must not hold `self` while the
        // operations below take it mutably.
        let mut keys: Vec<&'static str> = vec![
            "quality-checks",
            "replace-scope",
            "quality-hint",
            "quality-run",
            "quality-not-run",
            "quality-col-line",
            "quality-col-edid",
            "quality-col-check",
            "quality-col-detail",
            "quality-unify",
            "quality-table-hint",
            "quality-fix-aliases",
            "quality-fix-numbers",
            "btn-close",
        ];
        keys.extend(Check::ALL.iter().map(|c| c.i18n_key()));
        keys.extend(Check::ALL.iter().map(|c| c.short_key()));
        keys.extend(Scope::ALL.iter().map(|s| s.i18n_key()));
        let labels: std::collections::HashMap<&'static str, String> =
            keys.into_iter().map(|k| (k, self.i18n.t(k))).collect();
        let t = |k: &str| labels.get(k).cloned().unwrap_or_else(|| k.to_string());
        let mut close = false;
        let mut run = false;

        // 1 — checks, 2 — scope.
        ui.columns(2, |cols| {
            cols[0].group(|ui| {
                ui.strong(t("quality-checks"));
                let mut set = self.quality_checks();
                let mut changed = false;
                for c in Check::ALL {
                    let mut on = set.contains(&c);
                    if ui.checkbox(&mut on, t(c.i18n_key())).changed() {
                        changed = true;
                        if on {
                            set.push(c);
                        } else {
                            set.retain(|x| *x != c);
                        }
                    }
                }
                if changed {
                    set.sort();
                    self.config.quality_checks = Check::format_set(&set);
                    let _ = self.config.save();
                }
            });
            cols[1].group(|ui| {
                ui.strong(t("replace-scope"));
                for s in Scope::ALL {
                    ui.radio_value(&mut self.quality.scope, s, t(s.i18n_key()));
                }
                ui.label(egui::RichText::new(t("quality-hint")).weak().small());
                ui.add_space(6.0);
                let running = self.quality.job.is_some();
                if ui
                    .add_enabled(
                        !running,
                        egui::Button::new(egui::RichText::new(t("quality-run")).strong())
                            .shortcut_text(key_text(
                                ui.ctx(),
                                &self.i18n,
                                egui::Modifiers::NONE,
                                egui::Key::F12,
                            )),
                    )
                    .clicked()
                {
                    run = true;
                }
                if let Some(job) = &self.quality.job {
                    ui.add(
                        egui::ProgressBar::new(job.progress)
                            .show_percentage()
                            .animate(true),
                    );
                }
            });
        });
        if run {
            self.quality_run();
        }
        ui.add_space(6.0);

        // Buttons at the bottom (drawn first: the table takes the rest).
        let sel_lines: Vec<usize> = {
            let report = self.quality.report.as_ref();
            let mut v: Vec<usize> = self
                .quality
                .selected
                .iter()
                .filter_map(|&r| self.quality.rows.get(r))
                .filter_map(|&fi| report?.findings.get(fi))
                .map(|f| f.line)
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let mut fix: Option<bool> = None;
        egui::TopBottomPanel::bottom("ximod-quality-buttons")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t("quality-table-hint")).weak().small());
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !sel_lines.is_empty(),
                            egui::Button::new(t("quality-fix-aliases")),
                        )
                        .clicked()
                    {
                        fix = Some(true);
                    }
                    if ui
                        .add_enabled(
                            !sel_lines.is_empty(),
                            egui::Button::new(t("quality-fix-numbers")),
                        )
                        .clicked()
                    {
                        fix = Some(false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("btn-close")).clicked() {
                            close = true;
                        }
                    });
                });
            });

        // 3 — results.
        let msg = self
            .quality
            .message
            .clone()
            .unwrap_or_else(|| t("quality-not-run"));
        ui.strong(msg);
        let headers = [
            t("quality-col-line"),
            t("quality-col-edid"),
            t("quality-col-check"),
            t("quality-col-detail"),
        ];
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let table_h = ui.available_height().max(80.0);
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut double: Option<usize> = None;
        let mut unify: Option<usize> = None;
        let mut sort_click: Option<usize> = None;
        let rows = self.quality.rows.clone();
        let selected = self.quality.selected.clone();
        let (sort_col, sort_asc) = self.quality.sort;
        let unify_l = t("quality-unify");
        let texts: Vec<(String, String, String, String, bool)> = {
            let report = self.quality.report.as_ref();
            let doc = self.docs.get(self.active);
            rows.iter()
                .filter_map(|&fi| {
                    let f = report?.findings.get(fi)?;
                    let e = doc?.tm.entries.get(f.line)?;
                    let is_group = matches!(f.detail, crate::quality::Detail::Inconsistent { .. });
                    let edid = if is_group {
                        crate::ui::edit_window::one_line(&e.source, 30)
                    } else {
                        e.edid.clone()
                    };
                    Some((
                        (f.line + 1).to_string(),
                        edid,
                        t(f.check.short_key()),
                        self.finding_text(f),
                        is_group,
                    ))
                })
                .collect()
        };
        egui::ScrollArea::vertical()
            .id_salt("ximod-quality-table")
            .max_height(table_h)
            .show(ui, |ui| {
                egui_extras::TableBuilder::new(ui)
                    .striped(true)
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(egui_extras::Column::exact(60.0))
                    .column(
                        egui_extras::Column::initial(170.0)
                            .at_least(80.0)
                            .clip(true),
                    )
                    .column(
                        egui_extras::Column::initial(130.0)
                            .at_least(80.0)
                            .clip(true),
                    )
                    .column(egui_extras::Column::remainder().clip(true))
                    .header(row_h, |mut h| {
                        for (i, title) in headers.iter().enumerate() {
                            h.col(|ui| {
                                let mark = if sort_col == i {
                                    if sort_asc {
                                        crate::icons::CARET_UP
                                    } else {
                                        crate::icons::CARET_DOWN
                                    }
                                } else {
                                    ""
                                };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(format!("{title}{mark}")).strong(),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    sort_click = Some(i);
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_h, texts.len(), |mut row| {
                            let r = row.index();
                            row.set_selected(selected.contains(&r));
                            let (line, edid, check, detail, is_group) = &texts[r];
                            row.col(|ui| {
                                ui.label(line);
                            });
                            row.col(|ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(edid).monospace())
                                        .truncate()
                                        .selectable(false),
                                );
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(check).truncate().selectable(false));
                            });
                            row.col(|ui| {
                                if *is_group && ui.small_button(&unify_l).clicked() {
                                    unify = Some(r);
                                }
                                ui.add(egui::Label::new(detail).truncate().selectable(false))
                                    .on_hover_text(detail);
                            });
                            let resp = row.response();
                            if resp.clicked() {
                                clicked = Some((r, resp.ctx.input(|i| i.modifiers)));
                            }
                            if resp.double_clicked() {
                                double = Some(r);
                            }
                        });
                    });
            });
        if let Some(col) = sort_click {
            let asc = if self.quality.sort.0 == col {
                !self.quality.sort.1
            } else {
                true
            };
            self.quality.sort = (col, asc);
            self.quality.sort_rows();
        }
        if let Some((r, mods)) = clicked {
            if mods.ctrl || mods.command {
                if !self.quality.selected.remove(&r) {
                    self.quality.selected.insert(r);
                }
            } else if mods.shift {
                let anchor = self.quality.selected.iter().next().copied().unwrap_or(r);
                let (a, b) = (anchor.min(r), anchor.max(r));
                self.quality.selected = (a..=b).collect();
            } else {
                self.quality.selected.clear();
                self.quality.selected.insert(r);
            }
        }
        if let Some(r) = double {
            self.quality_open_line(r);
        }
        if let Some(r) = unify
            && let Some(&fi) = self.quality.rows.get(r)
        {
            self.quality_unify(fi);
        }
        if let Some(tags) = fix {
            let msg = self.fix_lines(&sel_lines, tags);
            self.refresh_anomaly_lines(&sel_lines);
            self.quality.message = Some(msg);
        }
        close
    }

    // ------------------------------------------------------------------
    // The mod's own dictionary (B-03, decision D-02)
    // ------------------------------------------------------------------

    /// The rows of the working dictionary: every line that is not « À
    /// traduire » — translated, whatever its status, or Ignoré — with its
    /// status; the lines never shown in game are left out.
    pub(crate) fn mod_dict_rows(entries: &[crate::tm::TmEntry]) -> Vec<crate::basedb::BaseRow> {
        crate::mod_dict::rows(entries)
    }

    /// Write the dictionary of document `idx` —
    /// `<mods>/<jeu>/<fichier>_<SRC>-<DST>.ximdb`, timestamped copies of
    /// B-11 — after a save or an export (`forced == false`, subject to the
    /// option) or on request from the Dictionnaires menu (`forced`). The
    /// result goes to the status bar, after the message already there.
    pub(crate) fn write_mod_dict(&mut self, idx: usize, forced: bool) -> bool {
        if !forced && !self.config.mod_dict_auto {
            return false;
        }
        if self.docs.get(idx).is_some_and(|d| d.all.is_some()) {
            return false;
        }
        let Some(doc) = self.docs.get(idx) else {
            return false;
        };
        if doc.settings.target.trim().is_empty() {
            if forced {
                self.notify_warn(self.i18n.t("dict-stack-no-target"));
            }
            return false;
        }
        let rows = Self::mod_dict_rows(&doc.tm.entries);
        let out = DictStack::mod_dict_path(&doc.path, &doc.settings, &self.config.mod_base_dir());
        if rows.is_empty() && !out.is_file() {
            if forced {
                self.notify_warn(self.i18n.t("dict-mod-nothing"));
            }
            return false;
        }
        // A dictionary set aside at opening is rewritten after a question
        // (R-B12-03).
        let ident = doc.ident.clone();
        if !self.crc_may_write(idx, &out) {
            return false;
        }
        let name = out
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        self.invalidate_dict(&out);
        let Some(doc) = self.docs.get(idx) else {
            return false;
        };
        // The settings of the file the header keeps (D-09, C-08).
        match crate::mod_dict::write(&crate::mod_dict::Write {
            entries: &doc.tm.entries,
            rules: doc.tm.meta.rules.clone(),
            rules_off: doc.tm.meta.rules_off.clone(),
            enc_in: doc.enc.forced_in.map(|p| p.id().to_string()),
            enc_out: doc.enc.forced_out.map(|p| p.id().to_string()),
            lang: &doc.settings.target.clone(),
            out: &out,
            ident: ident.as_ref(),
            backup_copies: self.config.backup_copies,
            old_months: self.config.old_data_months,
        }) {
            Ok(n) => {
                // The history goes with the mod's dictionary (A-18, D-46).
                self.save_history(idx, &out);
                let msg = self.i18n.t_args(
                    "dict-mod-written",
                    &[("n", &n.to_string()), ("file", &name)],
                );
                if forced {
                    self.notify_ok(msg);
                } else {
                    self.status = self.i18n.dash(&self.status, &msg);
                }
                true
            }
            Err(e) => {
                self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()));
                false
            }
        }
    }

    // ------------------------------------------------------------------
    // The dictionary tree (B-02)
    // ------------------------------------------------------------------

    /// Dictionnaires ▸ Gérer les dictionnaires…: the tree of the active
    /// document, (re)built from the folders and the saved configuration.
    fn open_dicts_window(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if doc.settings.target.trim().is_empty() {
            self.notify_warn(self.i18n.t("dict-stack-no-target"));
            return;
        }
        let tree = self.dict_tree(&doc.path, &doc.settings);
        let w = &mut self.dicts;
        if !w.open {
            w.needs_place = true;
        }
        w.open = true;
        w.doc_serial = Some(doc.serial);
        w.loaded = tree.clone();
        w.tree = tree;
        w.selected = None;
        w.info.clear();
        w.message = None;
        w.confirm_close = false;
    }

    /// Save the tree for the game and pair; the personal node's case is
    /// the document's own switch (E-12).
    fn dicts_save(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        if let Some(g) = self
            .dicts
            .tree
            .groups
            .iter()
            .find(|g| g.role == crate::dicts::Role::Personal)
            && let Some(n) = g.nodes.first()
        {
            doc.settings.use_personal = n.enabled && g.enabled;
        }
        let key = Self::dicts_key(&doc.settings);
        self.config
            .dictionaries
            .insert(key, self.dicts.tree.to_config());
        let _ = self.config.save();
        self.dicts.loaded = self.dicts.tree.clone();
        self.dicts.message = Some(self.i18n.t("dicts-saved"));
    }

    /// The detail of a file: lines, format, date — read once per opening.
    fn dicts_info(&mut self, path: &std::path::Path) -> crate::ui::dicts_window::NodeInfo {
        if let Some(i) = self.dicts.info.get(path) {
            return i.clone();
        }
        let db = self.file_dbs.peek(path).or_else(|| self.file_dbs.get(path));
        let lines = db.as_ref().map(|db| db.rows()).unwrap_or(0);
        let old = db.as_ref().map(|db| db.old_rows()).unwrap_or(0);
        let version = crate::basedb::peek_version(path);
        let modified = crate::basedb::mtime(path)
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let info = crate::ui::dicts_window::NodeInfo {
            lines,
            old,
            version,
            modified,
        };
        self.dicts.info.insert(path.to_path_buf(), info.clone());
        info
    }

    /// How many lines of the active document owe their translation to a
    /// dictionary (R-B02-04).
    fn dicts_used_by(&self, name: &str) -> usize {
        let Some(doc) = self.docs.get(self.active) else {
            return 0;
        };
        doc.tm
            .entries
            .iter()
            .filter(|e| crate::provenance::from_file(e, name))
            .count()
    }

    /// The entries of Options ▸ Colonnes (D-42), in the columns' order:
    /// title and whether the column shows.
    fn column_menu_items(&self) -> Vec<(String, bool)> {
        vec![
            (
                self.i18n.t("menu-columns-status-buttons"),
                self.config.show_status_buttons,
            ),
            (
                self.i18n.t("menu-columns-comment"),
                self.config.show_comment,
            ),
            (self.i18n.t("menu-columns-review"), self.config.show_review),
            (
                self.i18n.t("menu-columns-provenance"),
                self.config.show_provenance,
            ),
        ]
    }

    /// Show or hide the column of entry `i` of [`Self::column_menu_items`].
    fn toggle_column(&mut self, i: usize) {
        match i {
            0 => self.config.show_status_buttons = !self.config.show_status_buttons,
            1 => self.config.show_comment = !self.config.show_comment,
            2 => {
                // A review being typed is closed with its cell.
                self.close_edit_session();
                self.config.show_review = !self.config.show_review;
            }
            3 => self.config.show_provenance = !self.config.show_provenance,
            _ => return,
        }
        let _ = self.config.save();
    }

    /// The provenance of a line, « Statut — provenance » (R-B09-04).
    fn origin_tip(&self, doc: &Doc, idx: usize) -> Option<String> {
        let e = doc.tm.entries.get(idx)?;
        crate::provenance::tip(&self.i18n, e)
    }

    fn dicts_window(&mut self, ctx: &egui::Context) {
        if !self.dicts.open {
            return;
        }
        let serial = self.docs.get(self.active).map(|d| d.serial);
        if serial.is_none() || serial != self.dicts.doc_serial {
            // Another document: the tree belongs to the one it was built for.
            if serial.is_some() {
                self.open_dicts_window();
            } else {
                self.dicts.open = false;
                return;
            }
        }
        let id = egui::ViewportId::from_hash_of("ximod-dicts");
        let geo = self.config.dicts_win;
        let just_opened = self.dicts.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(640.0), geo[3].max(420.0)]))
        } else {
            None
        };
        let (game, src, tgt) = {
            let s = self.active_settings();
            (
                crate::games::Game::from_id(&s.game)
                    .display_name()
                    .to_string(),
                self.i18n.display_name(&s.source),
                self.i18n.display_name(&s.target),
            )
        };
        let title = self.i18n.t_args(
            "dicts-title",
            &[("game", &game), ("source", &src), ("target", &tgt)],
        );
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([640.0, 420.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.dicts.needs_place = false;
            let esc = vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.dicts_body(ui) {
                    close = true;
                }
            });
            if esc || vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.dicts_win = [x, y, w, h];
            }
        });
        if close {
            if self.dicts.dirty() {
                // F-04: nothing is lost silently.
                self.dicts.confirm_close = true;
            } else {
                self.dicts.open = false;
                let _ = self.config.save();
            }
        }
    }

    /// Body of the tree window; returns true to close it.
    fn dicts_body(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::dicts::Role;
        use crate::ui::dicts_window::Selection;
        let i18n_t = |s: &Self, k: &str| s.i18n.t(k);
        let mut close = false;
        let file_name = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let target = self.active_settings().target.trim().to_string();

        // The confirmation on Fermer with changes (F-04).
        if self.dicts.confirm_close {
            let mut choice: Option<u8> = None;
            egui::Window::new(self.i18n.t("dicts-unsaved-title"))
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label(self.i18n.t("dicts-unsaved"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(self.i18n.t("btn-save")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(self.i18n.t("dicts-discard")).clicked() {
                            choice = Some(2);
                        }
                        if ui.button(self.i18n.t("btn-cancel")).clicked() {
                            choice = Some(3);
                        }
                    });
                });
            match choice {
                Some(1) => {
                    self.dicts_save();
                    self.dicts.open = false;
                }
                Some(2) => {
                    self.dicts.tree = self.dicts.loaded.clone();
                    self.dicts.open = false;
                    let _ = self.config.save();
                }
                Some(3) => self.dicts.confirm_close = false,
                _ => {}
            }
            if !self.dicts.open {
                self.dicts.confirm_close = false;
            }
            return false;
        }

        ui.horizontal(|ui| {
            ui.strong(self.i18n.t_arg("dicts-document", "file", &file_name));
            ui.label(
                egui::RichText::new(self.i18n.t("dicts-hint"))
                    .weak()
                    .small(),
            );
        });
        ui.add_space(4.0);

        // Actions collected from the drawing.
        let mut toggle: Option<(Role, Option<std::path::PathBuf>, bool)> = None;
        let mut select: Option<Selection> = None;
        let mut view: Option<std::path::PathBuf> = None;
        let mut action: Option<&'static str> = None;

        let bottom_h = 70.0;
        let avail_h = (ui.available_height() - bottom_h).max(120.0);
        ui.horizontal_top(|ui| {
            let right_w = 190.0;
            let left_w = (ui.available_width() - right_w - 12.0).max(300.0);
            // ---- The tree ----
            ui.vertical(|ui| {
                ui.set_width(left_w);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(left_w - 16.0);
                    ui.set_height(avail_h);
                    egui::ScrollArea::vertical()
                        .id_salt("ximod-dicts-tree")
                        .show(ui, |ui| {
                            ui.set_width(left_w - 28.0);
                            ui.vertical(|ui| {
                                let role_labels: std::collections::HashMap<Role, String> =
                                    Role::ALL
                                        .iter()
                                        .map(|r| (*r, self.i18n.t(r.i18n_key())))
                                        .collect();
                                let groups = self.dicts.tree.groups.clone();
                                for g in &groups {
                                    let g_sel =
                                        self.dicts.selected == Some(Selection::Group(g.role));
                                    ui.horizontal(|ui| {
                                        let mut on = g.enabled;
                                        if ui.checkbox(&mut on, "").changed() {
                                            toggle = Some((g.role, None, on));
                                        }
                                        let folder = self.dicts_group_hint(g.role);
                                        let text = if folder.is_empty() {
                                            role_labels[&g.role].clone()
                                        } else {
                                            format!("{} — {}", role_labels[&g.role], folder)
                                        };
                                        let rich = egui::RichText::new(text).strong();
                                        let rich = if g.enabled { rich } else { rich.weak() };
                                        if ui.selectable_label(g_sel, rich).clicked() {
                                            select = Some(Selection::Group(g.role));
                                        }
                                    });
                                    if g.nodes.is_empty() {
                                        ui.horizontal(|ui| {
                                            ui.add_space(26.0);
                                            ui.label(
                                                egui::RichText::new(
                                                    self.i18n.t("dicts-empty-group"),
                                                )
                                                .weak()
                                                .small(),
                                            );
                                        });
                                    }
                                    for n in &g.nodes {
                                        let n_sel = self.dicts.selected
                                            == Some(Selection::Node(g.role, n.path.clone()));
                                        let info = self.dicts_info(&n.path);
                                        ui.horizontal(|ui| {
                                            ui.add_space(22.0);
                                            let mut on = n.enabled;
                                            if ui
                                                .add_enabled(
                                                    g.enabled,
                                                    egui::Checkbox::new(&mut on, ""),
                                                )
                                                .changed()
                                            {
                                                toggle = Some((g.role, Some(n.path.clone()), on));
                                            }
                                            let label = if n.added
                                                && !g
                                                    .added_dirs
                                                    .iter()
                                                    .any(|d| n.path.starts_with(d))
                                            {
                                                n.path.display().to_string()
                                            } else {
                                                n.name()
                                            };
                                            let rich = egui::RichText::new(label);
                                            let rich = if n.enabled && g.enabled {
                                                rich
                                            } else {
                                                rich.weak()
                                            };
                                            let resp = ui.selectable_label(n_sel, rich);
                                            if resp.clicked() {
                                                select =
                                                    Some(Selection::Node(g.role, n.path.clone()));
                                            }
                                            if resp.double_clicked() {
                                                view = Some(n.path.clone());
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let mut s = self.i18n.t_arg(
                                                        "dicts-lines",
                                                        "n",
                                                        &crate::ui::export_dialog::fmt_count(
                                                            &self.i18n, info.lines,
                                                        ),
                                                    );
                                                    if info.old > 0 {
                                                        s = self.i18n.dot(
                                                            &s,
                                                            &self.i18n.t_arg(
                                                                "dicts-old",
                                                                "n",
                                                                &info.old.to_string(),
                                                            ),
                                                        );
                                                    }
                                                    if n.added {
                                                        s = self
                                                            .i18n
                                                            .dot(&s, &self.i18n.t("dicts-added"));
                                                    }
                                                    ui.label(egui::RichText::new(s).weak().small());
                                                },
                                            );
                                        });
                                    }
                                    for d in &g.added_dirs {
                                        let d_sel = self.dicts.selected
                                            == Some(Selection::Node(g.role, d.clone()));
                                        ui.horizontal(|ui| {
                                            ui.add_space(26.0);
                                            let text = self.i18n.t_arg(
                                                "dicts-folder",
                                                "path",
                                                &d.display().to_string(),
                                            );
                                            if ui
                                                .selectable_label(
                                                    d_sel,
                                                    egui::RichText::new(text).weak().small(),
                                                )
                                                .clicked()
                                            {
                                                select = Some(Selection::Node(g.role, d.clone()));
                                            }
                                        });
                                    }
                                    ui.add_space(2.0);
                                }
                            });
                        });
                });
            });
            // ---- Buttons ----
            ui.vertical(|ui| {
                ui.set_width(right_w);
                let has_sel = self.dicts.selected.is_some();
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new(format!(
                            "{} {}",
                            crate::icons::ARROW_UP,
                            i18n_t(self, "dicts-up")
                        ))
                        .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("up");
                }
                if ui
                    .add_enabled(
                        has_sel,
                        egui::Button::new(format!(
                            "{} {}",
                            crate::icons::ARROW_DOWN,
                            i18n_t(self, "dicts-down")
                        ))
                        .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("down");
                }
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Button::new(i18n_t(self, "dicts-add-file"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("add-file");
                }
                if ui
                    .add(
                        egui::Button::new(i18n_t(self, "dicts-add-dir"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("add-dir");
                }
                let removable = self.dicts.selected_removable();
                if ui
                    .add_enabled(
                        removable,
                        egui::Button::new(i18n_t(self, "dicts-remove"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .on_disabled_hover_text(i18n_t(self, "dicts-remove-tip"))
                    .clicked()
                {
                    action = Some("remove");
                }
                ui.add_space(8.0);
                let is_file =
                    matches!(&self.dicts.selected, Some(Selection::Node(_, p)) if p.is_file());
                if ui
                    .add_enabled(
                        is_file,
                        egui::Button::new(i18n_t(self, "dicts-view"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("view");
                }
                if ui
                    .add_enabled(
                        is_file,
                        egui::Button::new(i18n_t(self, "dicts-edit"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .clicked()
                {
                    action = Some("edit");
                }
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Button::new(i18n_t(self, "dicts-reset"))
                            .min_size([right_w, 0.0].into()),
                    )
                    .on_hover_text(i18n_t(self, "dicts-reset-tip"))
                    .clicked()
                {
                    action = Some("reset");
                }
            });
        });

        // ---- Detail line ----
        let detail = match self.dicts.selected.clone() {
            Some(Selection::Node(role, p)) if p.is_file() => {
                let info = self.dicts_info(&p);
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                let date = info
                    .modified
                    .map(|t| crate::backup::display_datetime(&self.i18n, t))
                    .unwrap_or_else(|| "—".into());
                let v = info
                    .version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into());
                let used = self.dicts_used_by(&name);
                self.i18n.t_args(
                    "dicts-detail",
                    &[
                        ("file", &name),
                        ("role", &self.i18n.t(role.i18n_key())),
                        (
                            "lines",
                            &crate::ui::export_dialog::fmt_count(&self.i18n, info.lines),
                        ),
                        ("v", &v),
                        ("date", &date),
                        (
                            "used",
                            &crate::ui::export_dialog::fmt_count(&self.i18n, used),
                        ),
                        ("doc", &file_name),
                    ],
                )
            }
            _ => self.dicts.message.clone().unwrap_or_default(),
        };
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(detail).small()).truncate());
        });
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(self.i18n.t("dicts-apply")).strong(),
                ))
                .clicked()
            {
                action = Some("apply");
            }
            ui.label(
                egui::RichText::new(self.i18n.t("dicts-apply-hint"))
                    .weak()
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.t("btn-close")).clicked() {
                    close = true;
                }
                if ui
                    .add_enabled(
                        self.dicts.dirty(),
                        egui::Button::new(self.i18n.t("btn-save")),
                    )
                    .clicked()
                {
                    action = Some("save");
                }
            });
        });

        // ---- Apply the collected actions ----
        if let Some(sel) = select {
            self.dicts.selected = Some(sel);
        }
        if let Some((role, path, on)) = toggle
            && let Some(g) = self.dicts.tree.group_mut(role)
        {
            match path {
                None => g.enabled = on,
                Some(p) => {
                    if let Some(n) = g.nodes.iter_mut().find(|n| n.path == p) {
                        n.enabled = on;
                    }
                }
            }
        }
        if let Some(p) = view {
            self.open_dict_view_window();
            self.load_dict_view_path(&p);
        }
        match action {
            Some("up") => {
                self.dicts.move_selected(-1);
            }
            Some("down") => {
                self.dicts.move_selected(1);
            }
            Some("add-file") => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter(
                        self.i18n.t("dialog-databases"),
                        &["ximdb", "sst", "eet", "dsd", "json", "xml"],
                    )
                    .set_directory(self.bases_root())
                    .pick_file()
                {
                    if crate::basedb::is_ximdb_path(&p)
                        && let Some(l) = crate::basedb::peek_lang(&p)
                        && !l.is_empty()
                        && !l.eq_ignore_ascii_case(&target)
                    {
                        let name = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        self.dicts.message = Some(self.i18n.t_args(
                            "dicts-not-target",
                            &[("file", &name), ("lang", &l), ("target", &target)],
                        ));
                        return close;
                    }
                    let added = self.dicts.tree.add_file(p.clone());
                    self.dicts.message = Some(self.i18n.t_args(
                        "dicts-added-file",
                        &[
                            ("n", &usize::from(added).to_string()),
                            ("path", &p.display().to_string()),
                        ],
                    ));
                    self.dicts.selected = Some(Selection::Node(
                        if crate::basedb::is_text_only_hint(&p) {
                            Role::TextOnly
                        } else {
                            Role::External
                        },
                        p,
                    ));
                }
            }
            Some("add-dir") => {
                if let Some(d) = rfd::FileDialog::new()
                    .set_directory(self.bases_root())
                    .pick_folder()
                {
                    let n = self.dicts.tree.add_dir(d.clone(), &target);
                    self.dicts.message = Some(self.i18n.t_args(
                        "dicts-added-dir",
                        &[("n", &n.to_string()), ("path", &d.display().to_string())],
                    ));
                    self.dicts.selected = Some(Selection::Node(Role::External, d));
                }
            }
            Some("remove") => {
                if let Some(Selection::Node(role, p)) = self.dicts.selected.clone()
                    && self.dicts.tree.remove(role, &p)
                {
                    self.dicts.selected = None;
                }
            }
            Some("view") => {
                if let Some(Selection::Node(_, p)) = self.dicts.selected.clone() {
                    self.open_dict_view_window();
                    self.load_dict_view_path(&p);
                }
            }
            Some("edit") => {
                if let Some(Selection::Node(_, p)) = self.dicts.selected.clone() {
                    self.open_dict_editor(Some(p));
                }
            }
            Some("reset") => {
                if let Some(doc) = self.docs.get(self.active) {
                    let key = Self::dicts_key(&doc.settings);
                    let (path, settings) = (doc.path.clone(), doc.settings.clone());
                    self.config.dictionaries.remove(&key);
                    self.dicts.tree = self.dict_tree(&path, &settings);
                    self.dicts.message = Some(self.i18n.t("dicts-reset-done"));
                }
            }
            Some("save") => self.dicts_save(),
            Some("apply") => {
                self.dicts_save();
                let idx = self.active;
                let n = self.restack_doc(idx);
                self.dicts.message = Some(self.i18n.t_arg("dicts-applied", "n", &n.to_string()));
            }
            _ => {}
        }
        close
    }

    /// The folder a role's files come from, for the group label.
    fn dicts_group_hint(&self, role: crate::dicts::Role) -> String {
        use crate::dicts::Role;
        let s = self.active_settings();
        let game: String = s
            .game
            .trim()
            .chars()
            .map(|c| if std::path::is_separator(c) { '_' } else { c })
            .collect();
        match role {
            Role::Game => format!("jeux/{game}"),
            Role::External => format!("mods/{game}"),
            Role::Backups => format!("mods/{game}/backups"),
            _ => String::new(),
        }
    }

    /// Dictionnaires ▸ Créer un dictionnaire texte seul à partir d'un fichier
    /// de paires… (B-05): « source<TAB>traduction », one pair per line, to
    /// `mods/<jeu>/<fichier>_<SRC>-<DST>.ximdb`, after the game confirmation
    /// of the other builders.
    fn dict_from_text_pairs(&mut self) {
        let Some((src, dst)) = self.require_pair() else {
            return;
        };
        let Some(input) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-text-pairs"), &["txt", "tsv", "csv"])
            .pick_file()
        else {
            return;
        };
        let rows = match crate::basedb::read_text_pairs(&input) {
            Ok(r) => r,
            Err(e) => {
                self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()));
                return;
            }
        };
        if rows.is_empty() {
            self.alert_msg("dict-text-pairs-empty");
            return;
        }
        let sanitize = |s: &str| -> String {
            s.chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect()
        };
        let game_sub = sanitize(self.active_settings().game.trim());
        let stem = sanitize(
            input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("texte"),
        );
        let out = self.dict_base_dir(false).join(&game_sub).join(format!(
            "{stem}_{}-{}.ximdb",
            src.to_uppercase(),
            dst.to_uppercase()
        ));
        self.status = self
            .i18n
            .t_arg("dict-text-pairs-read", "n", &rows.len().to_string());
        self.pending_save = Some(PendingSave::Rows {
            out,
            lang: dst,
            rows,
        });
    }

    // ------------------------------------------------------------------
    // The personal dictionary and the dictionary editor (B-04)
    // ------------------------------------------------------------------

    /// Path of the active document's personal dictionary.
    fn personal_path_active(&self) -> Option<PathBuf> {
        let s = self.active_settings();
        if s.target.trim().is_empty() || s.source.trim().is_empty() {
            return None;
        }
        Some(DictStack::personal_path(
            &s,
            &crate::config::paths::get().dict_dir,
        ))
    }

    /// Ctrl+M / menu: the pairs of `lines` (translated ones) go to the
    /// personal dictionary — text-only unless « Conserver le champ » —,
    /// after the confirmations of R-B04-02.
    fn add_to_personal(&mut self, lines: &[usize]) {
        let Some(path) = self.personal_path_active() else {
            self.notify_warn(self.i18n.t("alert-needs-languages"));
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let keep = self.config.personal_keep_field;
        let mut pairs: Vec<crate::basedb::BaseRow> = Vec::new();
        for &i in lines {
            let Some(e) = doc.tm.entries.get(i) else {
                continue;
            };
            if e.unused || e.trans.trim().is_empty() || e.source.trim().is_empty() {
                continue;
            }
            let row = if keep {
                crate::basedb::BaseRow {
                    rec: e.rec_sig.clone(),
                    field: e.field_sig.clone(),
                    formid: e.formid,
                    edid: e.edid.clone(),
                    source: e.source.trim().to_string(),
                    trans: e.trans.trim().to_string(),
                    status: None,
                    ..Default::default()
                }
            } else {
                crate::dict_edit::text_pair(&e.source, &e.trans)
            };
            if !pairs
                .iter()
                .any(|p| p.source == row.source && p.rec == row.rec && p.field == row.field)
            {
                pairs.push(row);
            }
        }
        if pairs.is_empty() {
            self.notify_warn(self.i18n.t("personal-nothing"));
            return;
        }
        let lang = self.active_settings().target.clone();
        let dict = match crate::dict_edit::DictDoc::load(&path, &lang) {
            Ok(d) => d,
            Err(e) => {
                self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()));
                return;
            }
        };
        let conflicts = pairs
            .iter()
            .filter(|p| dict.find(p).is_some_and(|i| dict.rows[i].trans != p.trans))
            .count();
        self.personal_prompt = Some(PersonalPrompt {
            pairs,
            dict,
            conflicts,
            many_confirmed: false,
            remember: false,
        });
        self.personal_prompt_step();
    }

    /// Advance the pending addition: ask what must be asked, else commit.
    fn personal_prompt_step(&mut self) {
        let Some(p) = &self.personal_prompt else {
            return;
        };
        if p.pairs.len() > 20 && !p.many_confirmed {
            return; // the « many » dialog is shown by personal_prompt_window
        }
        if p.conflicts > 0 && self.personal_session_choice.is_none() {
            return; // the conflict dialog
        }
        let replace = self.personal_session_choice.unwrap_or(true);
        self.personal_commit(replace);
    }

    /// Write the pairs; `replace`: pairs already known with another
    /// translation are replaced, else kept.
    fn personal_commit(&mut self, replace: bool) {
        let Some(mut p) = self.personal_prompt.take() else {
            return;
        };
        let (mut added, mut replaced, mut kept) = (0usize, 0usize, 0usize);
        for row in p.pairs.drain(..) {
            match p.dict.find(&row) {
                Some(i) if p.dict.rows[i].trans != row.trans && !replace => kept += 1,
                _ => match p.dict.put(row) {
                    crate::dict_edit::Put::Added => added += 1,
                    crate::dict_edit::Put::Replaced => replaced += 1,
                    crate::dict_edit::Put::Same => {}
                },
            }
        }
        if p.dict.dirty {
            match p.dict.save(self.config.backup_copies) {
                Ok(_) => {
                    self.invalidate_dict(&p.dict.path);
                    if self.dict_editor.open
                        && self
                            .dict_editor
                            .doc
                            .as_ref()
                            .is_some_and(|d| d.path == p.dict.path)
                    {
                        let path = p.dict.path.clone();
                        let lang = p.dict.lang.clone();
                        if let Ok(d) = crate::dict_edit::DictDoc::load(&path, &lang) {
                            self.dict_editor.doc = Some(d);
                            self.dict_editor.touch();
                        }
                    }
                }
                Err(e) => {
                    self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()));
                    return;
                }
            }
        }
        let mut msg = self.i18n.t_args(
            "personal-added",
            &[
                ("n", &added.to_string()),
                ("replaced", &replaced.to_string()),
            ],
        );
        if kept > 0 {
            msg = self.i18n.dash(
                &msg,
                &self.i18n.t_arg("personal-kept", "n", &kept.to_string()),
            );
        }
        self.notify_ok(msg);
    }

    /// The confirmations of Ctrl+M (R-B04-02), as modal boxes.
    fn personal_prompt_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.personal_prompt else {
            return;
        };
        let (many, n, conflicts, remember) = (
            p.pairs.len() > 20 && !p.many_confirmed,
            p.pairs.len(),
            p.conflicts,
            p.remember,
        );
        let valider = self.i18n.t("btn-validate");
        let annuler = self.i18n.t("btn-cancel");
        let mut choice: Option<u8> = None;
        let mut remember_now = remember;
        if many {
            let msg = self.i18n.t_arg("personal-many", "n", &n.to_string());
            egui::Window::new(self.i18n.t("personal-title"))
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(msg);
                    ui.add_space(12.0);
                    let (ok, cancel) = confirm_buttons(ui, &valider, &annuler);
                    if ok {
                        choice = Some(1);
                    }
                    if cancel {
                        choice = Some(0);
                    }
                });
        } else {
            let msg = self
                .i18n
                .t_arg("personal-exists", "n", &conflicts.to_string());
            egui::Window::new(self.i18n.t("personal-exists-title"))
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(msg);
                    ui.add_space(6.0);
                    ui.checkbox(&mut remember_now, self.i18n.t("personal-remember"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(self.i18n.t("personal-replace")).clicked() {
                            choice = Some(2);
                        }
                        if ui.button(self.i18n.t("personal-keep")).clicked() {
                            choice = Some(3);
                        }
                        if ui.button(&annuler).clicked()
                            || ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            choice = Some(0);
                        }
                    });
                });
        }
        if let Some(p) = self.personal_prompt.as_mut() {
            p.remember = remember_now;
        }
        match choice {
            Some(0) => self.personal_prompt = None,
            Some(1) => {
                if let Some(p) = self.personal_prompt.as_mut() {
                    p.many_confirmed = true;
                }
                self.personal_prompt_step();
            }
            Some(c @ (2 | 3)) => {
                let replace = c == 2;
                if remember_now {
                    self.personal_session_choice = Some(replace);
                }
                self.personal_commit(replace);
            }
            _ => {}
        }
    }

    /// Option « Alimenter mon dictionnaire personnel à chaque
    /// enregistrement » (R-B04-06): the Validé and Personnalisé lines not
    /// yet known, as text-only pairs.
    fn personal_autofill(&mut self, idx: usize) {
        if !self.config.personal_autofill {
            return;
        }
        let Some(path) = self.personal_path_active() else {
            return;
        };
        let Some(doc) = self.docs.get(idx) else {
            return;
        };
        let lang = doc.settings.target.clone();
        let Ok(mut dict) = crate::dict_edit::DictDoc::load(&path, &lang) else {
            return;
        };
        let mut added = 0usize;
        for e in &doc.tm.entries {
            if e.unused || e.trans.trim().is_empty() {
                continue;
            }
            if !matches!(
                crate::status::derive(e),
                crate::status::Status::Validated | crate::status::Status::Custom
            ) {
                continue;
            }
            let row = crate::dict_edit::text_pair(&e.source, &e.trans);
            if dict.find(&row).is_none() {
                dict.put(row);
                added += 1;
            }
        }
        if added > 0 {
            match dict.save(self.config.backup_copies) {
                Ok(_) => {
                    self.invalidate_dict(&path);
                    let msg = self
                        .i18n
                        .t_arg("personal-autofilled", "n", &added.to_string());
                    self.status = self.i18n.dash(&self.status, &msg);
                }
                Err(e) => {
                    self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()))
                }
            }
        }
    }

    /// Dictionnaires ▸ Mon dictionnaire personnel… / Modifier un
    /// dictionnaire… / the tree's Modifier…: open the editor on a file.
    fn open_dict_editor(&mut self, path: Option<PathBuf>) {
        let path = match path {
            Some(p) => p,
            None => match self.personal_path_active() {
                Some(p) => p,
                None => {
                    self.notify_warn(self.i18n.t("alert-needs-languages"));
                    return;
                }
            },
        };
        let lang = self.active_settings().target.clone();
        match crate::dict_edit::DictDoc::load(&path, &lang) {
            Ok(mut d) => {
                d.read_only = self.is_shipped_dictionary(&path);
                self.dict_editor.open_on(d);
            }
            Err(e) => self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string())),
        }
    }

    /// A dictionary shipped with the program (the read-only ones): under
    /// the shipped `jeux` folder when it is not the writable one.
    fn is_shipped_dictionary(&self, path: &std::path::Path) -> bool {
        let shipped = crate::config::shipped_game_base_dir();
        let writable = self.config.game_base_dir();
        shipped.is_dir() && shipped != writable && path.starts_with(&shipped)
    }

    /// Dictionnaires ▸ Modifier un dictionnaire…: ask for the file.
    fn pick_dict_to_edit(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-ximdb"), &["ximdb"])
            .set_directory(self.bases_root())
            .pick_file()
        {
            self.open_dict_editor(Some(p));
        }
    }

    fn dict_editor_window(&mut self, ctx: &egui::Context) {
        if !self.dict_editor.open {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-dict-editor");
        let geo = self.config.dict_edit_win;
        let just_opened = self.dict_editor.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(620.0), geo[3].max(420.0)]))
        } else {
            None
        };
        let name = self
            .dict_editor
            .doc
            .as_ref()
            .map(|d| d.name())
            .unwrap_or_default();
        let title = self.i18n.t_arg("dict-editor-title", "file", &name);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([620.0, 420.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.dict_editor.needs_place = false;
            let esc = vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.dict_editor_body(ui) {
                    close = true;
                }
            });
            if esc || vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.dict_edit_win = [x, y, w, h];
            }
        });
        if close {
            if self.dict_editor.doc.as_ref().is_some_and(|d| d.dirty) {
                self.dict_editor.confirm_close = true;
            } else {
                self.dict_editor.open = false;
                let _ = self.config.save();
            }
        }
    }

    /// Save the editor's file (or Enregistrer sous… when read-only).
    fn dict_editor_save(&mut self, save_as: bool) {
        let start = self
            .config
            .mod_base_dir()
            .join(self.active_settings().game.trim());
        let copies = self.config.backup_copies;
        let Some(doc) = self.dict_editor.doc.as_mut() else {
            return;
        };
        let target = if save_as || doc.read_only {
            let _ = std::fs::create_dir_all(&start);
            let Some(p) = rfd::FileDialog::new()
                .add_filter(self.i18n.t("dialog-ximdb"), &["ximdb"])
                .set_directory(start)
                .set_file_name(doc.name())
                .save_file()
            else {
                return;
            };
            Some(p)
        } else {
            None
        };
        let result = match &target {
            Some(p) => doc.save_as(p, copies),
            None => doc.save(copies),
        };
        match result {
            Ok(n) => {
                let path = doc.path.clone();
                let name = doc.name();
                self.invalidate_dict(&path);
                self.dict_editor.message = Some(self.i18n.t_args(
                    "dict-editor-saved",
                    &[("file", &name), ("n", &n.to_string())],
                ));
            }
            Err(e) => self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string())),
        }
    }

    /// Nouveau…: an empty text-only dictionary for the pair (R-B05-05).
    fn dict_editor_new(&mut self) {
        let s = self.active_settings();
        if s.target.trim().is_empty() || s.source.trim().is_empty() {
            self.notify_warn(self.i18n.t("alert-needs-languages"));
            return;
        }
        let start = self.config.mod_base_dir().join(s.game.trim());
        let _ = std::fs::create_dir_all(&start);
        let Some(p) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-ximdb"), &["ximdb"])
            .set_directory(start)
            .set_file_name(format!("texte_{}.ximdb", s.pair_suffix()))
            .save_file()
        else {
            return;
        };
        let mut d = crate::dict_edit::DictDoc {
            path: p,
            lang: s.target.clone(),
            rows: Vec::new(),
            old: Vec::new(),
            dirty: false,
            read_only: false,
        };
        match d.save(0) {
            Ok(_) => self.dict_editor.open_on(d),
            Err(e) => self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string())),
        }
    }

    /// Body of the editor; returns true to close it.
    fn dict_editor_body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        let t = |s: &Self, k: &str| s.i18n.t(k);

        if self.dict_editor.confirm_close {
            let mut choice: Option<u8> = None;
            egui::Window::new(t(self, "dict-editor-unsaved-title"))
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label(t(self, "dict-editor-unsaved"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t(self, "btn-save")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(t(self, "dicts-discard")).clicked() {
                            choice = Some(2);
                        }
                        if ui.button(t(self, "btn-cancel")).clicked() {
                            choice = Some(3);
                        }
                    });
                });
            match choice {
                Some(1) => {
                    self.dict_editor_save(false);
                    if self.dict_editor.doc.as_ref().is_some_and(|d| !d.dirty) {
                        self.dict_editor.open = false;
                    }
                    self.dict_editor.confirm_close = false;
                }
                Some(2) => {
                    self.dict_editor.open = false;
                    self.dict_editor.confirm_close = false;
                    let _ = self.config.save();
                }
                Some(3) => self.dict_editor.confirm_close = false,
                _ => {}
            }
            return false;
        }
        let Some(doc_ro) = self.dict_editor.doc.as_ref().map(|d| d.read_only) else {
            return true;
        };

        // Filter line.
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            ui.strong(t(self, "dict-view-filter"));
            let resp = ui
                .add(egui::TextEdit::singleline(&mut self.dict_editor.filter).desired_width(260.0));
            if resp.changed() {
                self.dict_editor.typed_at = Some(now);
            }
            let (shown, total) = (
                self.dict_editor.rows.len(),
                self.dict_editor
                    .doc
                    .as_ref()
                    .map(|d| d.rows.len())
                    .unwrap_or(0),
            );
            ui.label(
                egui::RichText::new(self.i18n.t_args(
                    "dict-editor-count",
                    &[("shown", &shown.to_string()), ("total", &total.to_string())],
                ))
                .weak(),
            );
            // The old data (B-07), when the file has some.
            let n_old = self
                .dict_editor
                .doc
                .as_ref()
                .map(|d| d.old.len())
                .unwrap_or(0);
            if (n_old > 0 || self.dict_editor.show_old)
                && ui
                    .checkbox(
                        &mut self.dict_editor.show_old,
                        self.i18n.t_arg("dict-editor-old", "n", &n_old.to_string()),
                    )
                    .changed()
            {
                self.dict_editor.selected = None;
                self.dict_editor.clear_edit();
                self.dict_editor.touch();
            }
            if doc_ro {
                ui.label(
                    egui::RichText::new(t(self, "dict-editor-readonly"))
                        .color(ui.visuals().warn_fg_color)
                        .small(),
                );
            }
        });
        // Debounce (R-B04-04): the filter applies a quarter of a second after the keystroke.
        if let Some(at) = self.dict_editor.typed_at {
            if now - at >= 0.25 {
                self.dict_editor.typed_at = None;
                self.dict_editor.touch();
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(60));
            }
        }
        if self.dict_editor.typed_at.is_none() {
            self.dict_editor.refresh_rows();
        }

        // Edit zone and buttons at the bottom (drawn first: the table takes the rest).
        let mut enter = false;
        let mut action: Option<&'static str> = None;
        egui::TopBottomPanel::bottom("ximod-dict-editor-bottom")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(t(self, "dict-editor-source"));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.dict_editor.e_source)
                                .desired_width(300.0),
                        );
                        if self.dict_editor.focus_source {
                            resp.request_focus();
                            self.dict_editor.focus_source = false;
                        }
                        enter |= resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.label(t(self, "dict-editor-field"));
                        let r2 = ui.add(
                            egui::TextEdit::singleline(&mut self.dict_editor.e_field)
                                .desired_width(110.0)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("REC:FLD"),
                        );
                        enter |= r2.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.label(
                            egui::RichText::new(t(self, "dict-editor-field-hint"))
                                .weak()
                                .small(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(t(self, "dict-editor-trans"));
                        let r3 = ui.add(
                            egui::TextEdit::singleline(&mut self.dict_editor.e_trans)
                                .desired_width(ui.available_width() - 20.0),
                        );
                        enter |= r3.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                });

                // Buttons.
                ui.horizontal(|ui| {
                    let sel = self.dict_editor.selected.is_some();
                    let show_old = self.dict_editor.show_old;
                    if ui
                        .add_enabled(
                            !doc_ro && !show_old,
                            egui::Button::new(t(self, "dict-editor-add")),
                        )
                        .clicked()
                    {
                        action = Some("add");
                    }
                    if ui
                        .add_enabled(
                            !doc_ro && sel,
                            egui::Button::new(t(
                                self,
                                if show_old {
                                    "dict-editor-restore"
                                } else {
                                    "dict-editor-replace"
                                },
                            )),
                        )
                        .clicked()
                    {
                        action = Some(if show_old { "restore" } else { "replace" });
                    }
                    if ui
                        .add_enabled(
                            !doc_ro && sel,
                            egui::Button::new(t(self, "dict-editor-delete")),
                        )
                        .clicked()
                    {
                        action = Some("delete");
                    }
                    ui.separator();
                    if ui.button(t(self, "dict-editor-new")).clicked() {
                        action = Some("new");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t(self, "btn-close")).clicked() {
                            close = true;
                        }
                        let dirty = self.dict_editor.doc.as_ref().is_some_and(|d| d.dirty);
                        if doc_ro {
                            if ui.button(t(self, "dict-editor-save-as")).clicked() {
                                action = Some("save-as");
                            }
                        } else if ui
                            .add_enabled(
                                dirty,
                                egui::Button::new(
                                    egui::RichText::new(t(self, "btn-save")).strong(),
                                ),
                            )
                            .clicked()
                        {
                            action = Some("save");
                        }
                    });
                });
                if let Some(m) = &self.dict_editor.message {
                    ui.label(egui::RichText::new(m).weak().small());
                } else {
                    ui.label(
                        egui::RichText::new(t(self, "dict-editor-hint"))
                            .weak()
                            .small(),
                    );
                }
            });

        // Table.
        let headers = [
            t(self, "dict-editor-col-source"),
            t(self, "dict-editor-col-trans"),
            t(self, "dict-editor-col-field"),
            t(self, "dict-editor-col-status"),
        ];
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let table_h = ui.available_height().max(80.0);
        let mut clicked: Option<usize> = None;
        let mut sort_click: Option<usize> = None;
        let (sort_col, sort_asc) = self.dict_editor.sort;
        let rows = self.dict_editor.rows.clone();
        let selected = self.dict_editor.selected;
        let show_old = self.dict_editor.show_old;
        let del = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            && selected.is_some()
            && !ui.memory(|m| m.focused().is_some());
        {
            let doc = self.dict_editor.doc.as_ref().unwrap();
            egui::ScrollArea::vertical()
                .id_salt("ximod-dict-editor-table")
                .max_height(table_h)
                .show(ui, |ui| {
                    egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .sense(egui::Sense::click())
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(
                            egui_extras::Column::initial(220.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::remainder().at_least(80.0).clip(true))
                        .column(egui_extras::Column::exact(100.0))
                        .column(egui_extras::Column::exact(80.0))
                        .header(row_h, |mut h| {
                            for (i, title) in headers.iter().enumerate() {
                                h.col(|ui| {
                                    let mark = if sort_col == i {
                                        if sort_asc {
                                            crate::icons::CARET_UP
                                        } else {
                                            crate::icons::CARET_DOWN
                                        }
                                    } else {
                                        ""
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new(format!("{title}{mark}"))
                                                    .strong(),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        sort_click = Some(i);
                                    }
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, rows.len(), |mut row| {
                                let r = row.index();
                                let idx = rows[r];
                                let (b, date) = if show_old {
                                    (&doc.old[idx].row, Some(doc.old[idx].date.as_str()))
                                } else {
                                    (&doc.rows[idx], None)
                                };
                                row.set_selected(selected == Some(idx));
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(&b.source).truncate().selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(&b.trans).truncate().selectable(false));
                                });
                                row.col(|ui| {
                                    let sig = if b.rec.is_empty() && b.field.is_empty() {
                                        "—".to_string()
                                    } else {
                                        format!("{}:{}", b.rec, b.field)
                                    };
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(sig).monospace())
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(match date {
                                                Some(d) => self
                                                    .i18n
                                                    .dot(&b.status.clone().unwrap_or_default(), d),
                                                None => b.status.clone().unwrap_or_default(),
                                            })
                                            .weak()
                                            .small(),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                if row.response().clicked() {
                                    clicked = Some(idx);
                                }
                            });
                        });
                });
        }
        if let Some(col) = sort_click {
            let asc = if self.dict_editor.sort.0 == col {
                !self.dict_editor.sort.1
            } else {
                true
            };
            self.dict_editor.sort = (col, asc);
            self.dict_editor.touch();
        }
        if let Some(idx) = clicked {
            self.dict_editor.selected = Some(idx);
            let doc = self.dict_editor.doc.as_ref().unwrap();
            let row = if show_old {
                doc.old[idx].row.clone()
            } else {
                doc.rows[idx].clone()
            };
            self.dict_editor.load_edit(&row);
        }

        if enter && !doc_ro && !self.dict_editor.show_old {
            action = Some(if self.dict_editor.selected.is_some() {
                "replace"
            } else {
                "add"
            });
        }
        if del && !doc_ro {
            action = Some("delete");
        }

        match action {
            Some("add") => {
                let row = self.dict_editor.edited_row(None);
                match row {
                    Some(row) => {
                        let doc = self.dict_editor.doc.as_mut().unwrap();
                        let put = doc.put(row);
                        self.dict_editor.touch();
                        self.dict_editor.message = Some(match put {
                            crate::dict_edit::Put::Added => t(self, "dict-editor-added"),
                            crate::dict_edit::Put::Replaced => t(self, "dict-editor-replaced"),
                            crate::dict_edit::Put::Same => t(self, "dict-editor-same"),
                        });
                        self.dict_editor.selected = None;
                        self.dict_editor.clear_edit();
                        self.dict_editor.focus_source = true;
                    }
                    None => self.dict_editor.message = Some(t(self, "dict-editor-incomplete")),
                }
            }
            Some("replace") => {
                if let Some(i) = self.dict_editor.selected {
                    let base = self.dict_editor.doc.as_ref().unwrap().rows.get(i).cloned();
                    match self.dict_editor.edited_row(base.as_ref()) {
                        Some(row) => {
                            let doc = self.dict_editor.doc.as_mut().unwrap();
                            if i < doc.rows.len() {
                                // Another row with the new key? Drop it.
                                if let Some(j) = doc.find(&row).filter(|&j| j != i) {
                                    doc.rows.remove(j);
                                    let i2 = if j < i { i - 1 } else { i };
                                    doc.rows[i2] = row;
                                    self.dict_editor.selected = Some(i2);
                                } else {
                                    doc.rows[i] = row;
                                }
                                doc.dirty = true;
                                self.dict_editor.touch();
                                self.dict_editor.message = Some(t(self, "dict-editor-replaced"));
                            }
                        }
                        None => self.dict_editor.message = Some(t(self, "dict-editor-incomplete")),
                    }
                }
            }
            Some("delete") => {
                if let Some(i) = self.dict_editor.selected.take() {
                    let doc = self.dict_editor.doc.as_mut().unwrap();
                    if self.dict_editor.show_old {
                        doc.remove_old(i);
                    } else {
                        doc.remove(i);
                    }
                    self.dict_editor.touch();
                    self.dict_editor.clear_edit();
                    self.dict_editor.message = Some(t(self, "dict-editor-deleted"));
                }
            }
            Some("restore") => {
                if let Some(i) = self.dict_editor.selected.take() {
                    self.dict_editor.doc.as_mut().unwrap().restore_old(i);
                    self.dict_editor.touch();
                    self.dict_editor.clear_edit();
                    self.dict_editor.message = Some(t(self, "dict-editor-restored"));
                }
            }
            Some("new") => self.dict_editor_new(),
            Some("save") => self.dict_editor_save(false),
            Some("save-as") => self.dict_editor_save(true),
            _ => {}
        }
        close
    }

    // ------------------------------------------------------------------
    // Appliquer les dictionnaires (B-06)
    // ------------------------------------------------------------------

    /// Options ▸ Appliquer les dictionnaires… (scope Document) or the row
    /// menu (scope Selection, preset: tout sauf Validé, strict).
    fn open_apply_dialog(&mut self, scope: crate::replace::Scope, from_menu: bool) {
        if self.docs.is_empty() {
            return;
        }
        if self.all_active() {
            self.status = self.i18n.t("all-mods-file-only");
            return;
        }
        if self.active_settings().target.trim().is_empty() {
            self.notify_warn(self.i18n.t("dict-stack-no-target"));
            return;
        }
        let preset = from_menu.then_some(crate::apply::Options {
            overwrite: crate::apply::Overwrite::KeepValidated,
            mode: crate::apply::Mode::Strict,
            reset: false,
            threshold: self.config.fuzzy_threshold,
            normalize: self.config.normalize,
        });
        self.apply_dlg.open_with(scope, preset);
        self.apply_dlg.opts.threshold = self.config.fuzzy_threshold;
        self.apply_dlg.opts.normalize = self.config.normalize;
    }

    /// The lines of the dialog's scope (Ignoré and unused lines are left
    /// to the engine).
    fn apply_scope_lines(&self) -> Vec<usize> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        match self.apply_dlg.scope {
            crate::replace::Scope::Document => (0..doc.tm.entries.len()).collect(),
            crate::replace::Scope::View => self.visible.clone(),
            crate::replace::Scope::Selection => self.selection.targets(),
        }
    }

    /// The levels of the dialog's source: the active stack, or one file.
    /// The levels of the dialog's source. In the Approchant mode each one
    /// carries its word index (B-01); `pending` says one is still building.
    fn apply_levels(&mut self) -> (Vec<crate::apply::Level>, bool) {
        use crate::ui::apply_dialog::Source;
        let fuzzy = self.apply_dlg.opts.mode == crate::apply::Mode::Fuzzy;
        let mut pending = false;
        let raw: Vec<(
            PathBuf,
            String,
            crate::dicts::Role,
            Arc<crate::basedb::BaseDb>,
        )> = match self.apply_dlg.source.clone() {
            Source::Stack => {
                let Some(doc) = self.docs.get(self.active) else {
                    return (Vec::new(), false);
                };
                let (path, settings) = (doc.path.clone(), doc.settings.clone());
                match self.resolve_stack(&path, &settings) {
                    Some(stack) => stack
                        .levels
                        .into_iter()
                        .map(|l| (l.path.clone(), l.name(), l.role, l.db))
                        .collect(),
                    None => Vec::new(),
                }
            }
            Source::File(p) => match self.file_dbs.get(&p) {
                Some(db) => {
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string();
                    vec![(p, name, crate::dicts::Role::External, db)]
                }
                None => Vec::new(),
            },
        };
        let levels = raw
            .into_iter()
            .map(|(path, name, role, db)| {
                let fuzzy = if fuzzy {
                    let ix = self.fuzzy_index_for(&path, &db);
                    if ix.is_none() && self.fuzzy_jobs.contains_key(&path) {
                        pending = true;
                    }
                    ix
                } else {
                    None
                };
                crate::apply::Level {
                    name,
                    role,
                    db,
                    fuzzy,
                }
            })
            .collect();
        (levels, pending)
    }

    /// Recompute the preview when something changed (R-B06-06): a quarter
    /// of a second after the change, in the background beyond 10 000 lines.
    fn apply_refresh_plan(&mut self, ctx: &egui::Context) {
        use crate::ui::apply_dialog::{Job, Msg, PlanKey};
        // Collect a running job.
        if let Some(job) = self.apply_dlg.job.as_mut() {
            loop {
                match job.rx.try_recv() {
                    Ok(Msg::Progress(d, t)) => {
                        job.progress = if t == 0 { 1.0 } else { d as f32 / t as f32 }
                    }
                    Ok(Msg::Done(plan)) => {
                        self.apply_dlg.plan = Some(plan);
                        self.apply_dlg.job = None;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.apply_dlg.job = None;
                        break;
                    }
                }
            }
            return;
        }
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let lines = self.apply_scope_lines();
        let key = PlanKey {
            serial: doc.serial,
            revision: doc.undo.revision,
            scope: self.apply_dlg.scope,
            lines: lines.len(),
            opts: self.apply_dlg.opts.clone(),
            source: self.apply_dlg.source.clone(),
        };
        if self.apply_dlg.plan_key.as_ref() == Some(&key) && self.apply_dlg.plan.is_some() {
            return;
        }
        let now = ctx.input(|i| i.time);
        match self.apply_dlg.changed_at {
            None => {
                self.apply_dlg.changed_at = Some(now);
                self.apply_dlg.plan = None;
                ctx.request_repaint_after(std::time::Duration::from_millis(260));
                return;
            }
            Some(at) if now - at < 0.25 => {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
                return;
            }
            _ => {}
        }
        self.apply_dlg.changed_at = None;
        let (levels, pending) = self.apply_levels();
        if pending {
            // A word index is building (B-01): the preview says so and the
            // plan is tried again when it is done.
            self.fuzzy_poll(ctx);
            self.apply_dlg.plan = None;
            self.apply_dlg.plan_key = None;
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        let opts = self.apply_dlg.opts.clone();
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if lines.len() <= 10_000 {
            let plan = crate::apply::plan(&doc.tm.entries, &lines, &levels, &opts, |_, _| true);
            self.apply_dlg.plan = Some(plan);
        } else {
            let entries = doc.tm.entries.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let tx2 = tx.clone();
                let plan = crate::apply::plan(&entries, &lines, &levels, &opts, |d, t| {
                    tx2.send(Msg::Progress(d, t)).is_ok()
                });
                let _ = tx.send(Msg::Done(plan));
            });
            self.apply_dlg.job = Some(Job { rx, progress: 0.0 });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.apply_dlg.plan_key = Some(key);
    }

    /// Appliquer: the plan becomes one undoable operation (R-B06-07).
    fn apply_commit(&mut self) {
        let Some(plan) = self.apply_dlg.plan.clone() else {
            return;
        };
        if plan.changes.is_empty() {
            return;
        }
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = plan
            .changes
            .iter()
            .filter_map(|c| {
                doc.tm
                    .entries
                    .get(c.idx)
                    .map(|e| (c.idx, EntryState::capture(e)))
            })
            .collect();
        let touched = crate::apply::commit(&mut doc.tm.entries, &plan);
        let mut op = op_from_before(OpKind::ApplyBase, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        op.label_arg = Some(self.i18n.t_arg("op-apply-dicts", "n", &n.to_string()));
        doc.undo.push(op);
        self.pinned_rows.extend(touched.iter().copied());
        let detail = self.apply_detail(&plan);
        let mut msg = self.i18n.t_args(
            "apply-done",
            &[
                ("n", &plan.filled().to_string()),
                ("detail", &detail),
                ("over", &plan.overwritten.to_string()),
            ],
        );
        if plan.ambiguous > 0 {
            msg = self.i18n.dash(
                &msg,
                &self
                    .i18n
                    .t_arg("apply-ambiguous", "n", &plan.ambiguous.to_string()),
            );
        }
        self.notify_ok(msg);
        self.refresh_anomaly_lines(&touched);
        self.apply_dlg.plan = None;
        self.apply_dlg.plan_key = None;
        self.apply_dlg.open = false;
    }

    /// « 812 strict, 215 relax, 3 texte seul, 58 FormID ».
    fn apply_detail(&self, plan: &crate::apply::Plan) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (n, key) in [
            (plan.exact, "apply-detail-exact"),
            (plan.text, "apply-detail-text"),
            (plan.text_only, "apply-detail-textonly"),
            (plan.formid, "apply-detail-formid"),
            (plan.old, "apply-detail-old"),
            (plan.normalized, "apply-detail-normalized"),
        ] {
            if n > 0 {
                parts.push(self.i18n.t_arg(key, "n", &n.to_string()));
            }
        }
        if plan.fuzzy > 0 {
            parts.push(self.i18n.t_args(
                "apply-fuzzy-detail",
                &[
                    ("hi", &plan.fuzzy_hi.to_string()),
                    ("mid", &plan.fuzzy_mid.to_string()),
                    ("lo", &plan.fuzzy_lo.to_string()),
                ],
            ));
        }
        if plan.emptied > 0 {
            parts.push(
                self.i18n
                    .t_arg("apply-detail-emptied", "n", &plan.emptied.to_string()),
            );
        }
        if parts.is_empty() {
            "—".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// The dialog (modal, in the main window).
    fn apply_dialog_window(&mut self, ctx: &egui::Context) {
        use crate::apply::{Mode, Overwrite};
        use crate::replace::Scope;
        use crate::ui::apply_dialog::Source;
        if !self.apply_dlg.open {
            return;
        }
        if self.docs.is_empty() {
            self.apply_dlg.open = false;
            return;
        }
        self.apply_refresh_plan(ctx);
        let file_name = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let title = self.i18n.t_arg("apply-title", "file", &file_name);
        let (n_doc, n_view, n_sel) = (
            self.docs
                .get(self.active)
                .map(|d| d.tm.entries.len())
                .unwrap_or(0),
            self.visible.len(),
            self.selection.targets().len(),
        );
        let stack_names: String = {
            let Some(doc) = self.docs.get(self.active) else {
                return;
            };
            let tree = self.dict_tree(&doc.path, &doc.settings);
            let names: Vec<String> = tree.active().iter().map(|n| n.name()).collect();
            if names.is_empty() {
                self.i18n.t("dict-stack-none")
            } else {
                names.join(", ")
            }
        };
        let mut do_apply = false;
        let mut cancel = false;
        let mut pick_file = false;
        let mut changed = false;
        let labels_over: Vec<(Overwrite, String)> = Overwrite::ALL
            .iter()
            .map(|o| (*o, self.i18n.t(o.i18n_key())))
            .collect();
        let labels_mode: Vec<(Mode, String)> = Mode::ALL
            .iter()
            .map(|m| {
                let label = if *m == Mode::Fuzzy {
                    self.i18n.t_arg(
                        m.i18n_key(),
                        "pct",
                        &self.apply_dlg.opts.threshold.to_string(),
                    )
                } else {
                    self.i18n.t(m.i18n_key())
                };
                (*m, label)
            })
            .collect();
        let l_scope = [
            self.i18n.t_arg(
                "apply-scope-doc",
                "n",
                &crate::ui::export_dialog::fmt_count(&self.i18n, n_doc),
            ),
            self.i18n.t_arg(
                "apply-scope-view",
                "n",
                &crate::ui::export_dialog::fmt_count(&self.i18n, n_view),
            ),
            self.i18n.t_arg(
                "apply-scope-selection",
                "n",
                &crate::ui::export_dialog::fmt_count(&self.i18n, n_sel),
            ),
        ];
        let l_stack = self.i18n.t_arg("apply-source-stack", "list", &stack_names);
        let l_file = self.i18n.t("apply-source-file");
        let l_browse = self.i18n.t("settings-browse");
        let l_scope_h = self.i18n.t("replace-scope");
        let l_over_h = self.i18n.t("apply-overwrite");
        let l_mode_h = self.i18n.t("apply-matching");
        let l_reset = self.i18n.t("apply-reset");
        let l_apply = self.i18n.t("apply-button");
        let l_cancel = self.i18n.t("btn-cancel");
        let preview = match (&self.apply_dlg.plan, &self.apply_dlg.job) {
            (Some(p), _) => self.i18n.t_args(
                "apply-preview",
                &[
                    (
                        "n",
                        &crate::ui::export_dialog::fmt_count(&self.i18n, p.filled()),
                    ),
                    ("detail", &self.apply_detail(p)),
                    ("over", &p.overwritten.to_string()),
                ],
            ),
            _ if self.apply_dlg.opts.mode == Mode::Fuzzy && !self.fuzzy_jobs.is_empty() => {
                let p = self
                    .fuzzy_jobs
                    .values()
                    .map(|j| j.progress)
                    .fold(1.0f32, f32::min);
                self.i18n
                    .t_arg("vocab-indexing", "pct", &((p * 100.0) as u32).to_string())
            }
            _ => self.i18n.t("apply-preview-computing"),
        };
        let can_apply = self
            .apply_dlg
            .plan
            .as_ref()
            .is_some_and(|p| !p.changes.is_empty());
        let progress = self.apply_dlg.job.as_ref().map(|j| j.progress);
        let file_text = match &self.apply_dlg.source {
            Source::File(p) => p.display().to_string(),
            Source::Stack => self
                .apply_dlg
                .last_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        };
        let dlg = &mut self.apply_dlg;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(600.0);
                // 1 — source.
                let is_stack = dlg.source == Source::Stack;
                if ui.radio(is_stack, &l_stack).clicked() && !is_stack {
                    dlg.source = Source::Stack;
                    changed = true;
                }
                ui.horizontal(|ui| {
                    let is_file = !is_stack;
                    if ui.radio(is_file, &l_file).clicked() && !is_file {
                        match &dlg.last_file {
                            Some(p) => {
                                dlg.source = Source::File(p.clone());
                                changed = true;
                            }
                            None => pick_file = true,
                        }
                    }
                    ui.add(
                        egui::Label::new(egui::RichText::new(&file_text).monospace().small())
                            .truncate(),
                    );
                    if ui.button(&l_browse).clicked() {
                        pick_file = true;
                    }
                });
                ui.add_space(6.0);
                ui.columns(2, |cols| {
                    cols[0].group(|ui| {
                        ui.strong(&l_scope_h);
                        for (s, l) in [
                            (Scope::Document, &l_scope[0]),
                            (Scope::View, &l_scope[1]),
                            (Scope::Selection, &l_scope[2]),
                        ] {
                            if ui.radio_value(&mut dlg.scope, s, l).changed() {
                                changed = true;
                            }
                        }
                        ui.add_space(6.0);
                        if ui.checkbox(&mut dlg.opts.reset, &l_reset).changed() {
                            changed = true;
                        }
                    });
                    cols[1].group(|ui| {
                        ui.strong(&l_over_h);
                        for (o, l) in &labels_over {
                            if ui.radio_value(&mut dlg.opts.overwrite, *o, l).changed() {
                                changed = true;
                            }
                        }
                    });
                });
                ui.group(|ui| {
                    ui.strong(&l_mode_h);
                    for (m, l) in &labels_mode {
                        if ui.radio_value(&mut dlg.opts.mode, *m, l).changed() {
                            changed = true;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&preview).weak());
                if let Some(p) = progress {
                    ui.add(egui::ProgressBar::new(p).show_percentage());
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&l_cancel).clicked()
                            || ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            cancel = true;
                        }
                        if ui
                            .add_enabled(
                                can_apply,
                                egui::Button::new(egui::RichText::new(&l_apply).strong()),
                            )
                            .clicked()
                        {
                            do_apply = true;
                        }
                    });
                });
            });
        if changed {
            self.apply_dlg.changed_at = None;
            self.apply_dlg.plan = None;
        }
        if pick_file
            && let Some(p) = rfd::FileDialog::new()
                .add_filter(
                    self.i18n.t("dialog-databases"),
                    &["ximdb", "sst", "eet", "dsd", "json", "xml"],
                )
                .set_directory(self.bases_root())
                .pick_file()
        {
            self.apply_dlg.last_file = Some(p.clone());
            self.apply_dlg.source = Source::File(p);
            self.apply_dlg.plan = None;
        }
        if cancel {
            self.apply_dlg.open = false;
            self.apply_dlg.job = None;
        }
        if do_apply {
            self.apply_commit();
        }
    }

    // ------------------------------------------------------------------
    // Mise à jour du mod (B-07)
    // ------------------------------------------------------------------

    /// How many lines of a document changed since its dictionary (same
    /// record, text changed), for the status line at opening (R-B07-05).
    fn changed_since_dict(
        &mut self,
        path: &std::path::Path,
        settings: &DocSettings,
        entries: &[crate::tm::TmEntry],
    ) -> usize {
        let mod_path = DictStack::mod_dict_path(path, settings, &self.config.mod_base_dir());
        // A dictionary set aside at opening counts for nothing (B-12).
        if !mod_path.is_file() || self.crc.off.contains(&mod_path) {
            return 0;
        }
        let Some(db) = self.file_dbs.get(&mod_path) else {
            return 0;
        };
        entries
            .iter()
            .filter(|e| !e.unused && db.lookup_by_id(e).is_some())
            .count()
    }

    /// Dictionnaires ▸ Mise à jour du mod…: compare the active document
    /// with its dictionary.
    fn open_update_window(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if doc.all.is_some() {
            self.status = self.i18n.t("all-mods-file-only");
            return;
        }
        if doc.settings.target.trim().is_empty() {
            self.notify_warn(self.i18n.t("dict-stack-no-target"));
            return;
        }
        let mod_path =
            DictStack::mod_dict_path(&doc.path, &doc.settings, &self.config.mod_base_dir());
        let serial = doc.serial;
        let w = &mut self.update;
        if !w.open {
            w.needs_place = true;
        }
        w.open = true;
        w.doc_serial = Some(serial);
        w.message = None;
        w.confirm_forget = false;
        w.tab = crate::ui::update_window::Tab::Changed;
        w.dict_path = Some(mod_path.clone());
        w.comparison = None;
        w.dict_modified = None;
        if mod_path.is_file() {
            match crate::basedb::read_rows_and_old(&mod_path) {
                Ok((_, rows, old)) => {
                    let doc = &self.docs[self.active];
                    let c = crate::update::compare(&doc.tm.entries, &rows, &old);
                    self.update.comparison = Some(c);
                    self.update.diffs.clear();
                    self.update.dict_modified = crate::basedb::mtime(&mod_path)
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                }
                Err(e) => {
                    self.update.message =
                        Some(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()))
                }
            }
        }
    }

    /// « Reprendre les N anciennes traductions » (R-B07-04): the changed
    /// lines whose translation is empty or by similarity take the old one,
    /// En attente, in one operation.
    fn update_take(&mut self) {
        let Some(c) = self.update.comparison.clone() else {
            return;
        };
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let mut before = Vec::new();
        let mut kept = 0usize;
        for ch in &c.changed {
            let Some(e) = doc.tm.entries.get(ch.idx) else {
                continue;
            };
            if ch.old_trans.is_empty() {
                continue;
            }
            let st = crate::status::derive(e);
            if !(e.trans.is_empty() || st == crate::status::Status::BySimilarity) {
                kept += 1;
                continue;
            }
            before.push((ch.idx, EntryState::capture(e)));
            let e = &mut doc.tm.entries[ch.idx];
            e.trans = ch.old_trans.clone();
            e.set_translated_flags(true);
            crate::status::set(e, crate::status::Status::Pending);
            let name = self
                .update
                .dict_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            e.prov = crate::provenance::dict(
                crate::dicts::Role::Mod,
                crate::basedb::FillLevel::FormId,
                &name,
            );
        }
        let mut op = op_from_before(OpKind::ApplyBase, &before, &mut doc.tm.entries);
        let n = op.changes.len();
        if n > 0 {
            op.label_arg = Some(self.i18n.t_arg("op-update-take", "n", &n.to_string()));
            doc.undo.push(op);
            self.pinned_rows.extend(before.iter().map(|(i, _)| *i));
        }
        let touched: Vec<usize> = before.iter().map(|(i, _)| *i).collect();
        self.refresh_anomaly_lines(&touched);
        let mut msg = self.i18n.t_arg("update-taken", "n", &n.to_string());
        if kept > 0 {
            msg = self.i18n.dash(
                &msg,
                &self.i18n.t_arg("update-kept", "n", &kept.to_string()),
            );
        }
        self.notify_ok(msg.clone());
        self.update.message = Some(msg);
        // The comparison moves on: the lines taken are no longer « changed ».
        self.open_update_window();
    }

    /// « Oublier les supprimées » (R-B07-04): purge the old data of the file.
    fn update_forget(&mut self) {
        let Some(path) = self.update.dict_path.clone() else {
            return;
        };
        match crate::basedb::read_rows_and_old(&path) {
            Ok((lang, rows, _old)) => match crate::basedb::write_ximdb_full(
                rows,
                Vec::new(),
                &lang,
                &path,
                self.config.backup_copies,
            ) {
                Ok(_) => {
                    self.invalidate_dict(&path);
                    self.notify_ok(self.i18n.t("update-forgotten"));
                    self.open_update_window();
                }
                Err(e) => {
                    self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()))
                }
            },
            Err(e) => self.notify_err(self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string())),
        }
    }

    fn update_window(&mut self, ctx: &egui::Context) {
        if !self.update.open {
            return;
        }
        let serial = self.docs.get(self.active).map(|d| d.serial);
        if serial.is_none() {
            self.update.open = false;
            return;
        }
        if serial != self.update.doc_serial {
            self.open_update_window();
        }
        let id = egui::ViewportId::from_hash_of("ximod-update");
        let geo = self.config.update_win;
        let just_opened = self.update.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(640.0), geo[3].max(400.0)]))
        } else {
            None
        };
        let doc_name = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let dict_name = self
            .update
            .dict_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let title = self
            .i18n
            .t_args("update-title", &[("doc", &doc_name), ("dict", &dict_name)]);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([640.0, 400.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.update.needs_place = false;
            let esc = vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            egui::CentralPanel::default().show(vctx, |ui| {
                if self.update_body(ui) {
                    close = true;
                }
            });
            if esc || vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.update_win = [x, y, w, h];
            }
        });
        if close {
            self.update.open = false;
            let _ = self.config.save();
        }
    }

    /// Body of the update window; returns true to close it.
    fn update_body(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::ui::update_window::Tab;
        let mut close = false;
        let t = |s: &Self, k: &str| s.i18n.t(k);
        let doc_name = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let n_doc = self
            .docs
            .get(self.active)
            .map(|d| d.tm.entries.iter().filter(|e| !e.unused).count())
            .unwrap_or(0);

        // No dictionary yet.
        let Some(c) = self.update.comparison.clone() else {
            ui.label(
                self.update
                    .message
                    .clone()
                    .unwrap_or_else(|| t(self, "update-no-dict")),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t(self, "dict-save-mod")).clicked() {
                    let idx = self.active;
                    if self.write_mod_dict(idx, true) {
                        self.open_update_window();
                    }
                }
                if ui.button(t(self, "btn-close")).clicked() {
                    close = true;
                }
            });
            return close;
        };

        // Summary.
        let date = self
            .update
            .dict_modified
            .map(|t| crate::backup::display_datetime(&self.i18n, t))
            .unwrap_or_else(|| "—".into());
        let fmt = |n: usize| crate::ui::export_dialog::fmt_count(&self.i18n, n);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(self.i18n.t_args(
                "update-summary",
                &[
                    ("date", &date),
                    ("doc", &doc_name),
                    ("old", &fmt(c.dict_rows)),
                    ("new", &fmt(n_doc)),
                ],
            ));
            ui.strong(self.i18n.t_args(
                "update-counts",
                &[
                    ("added", &fmt(c.added.len())),
                    ("removed", &fmt(c.removed.len())),
                    ("changed", &fmt(c.changed.len())),
                    ("same", &fmt(c.same)),
                ],
            ));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (tab, key, n) in [
                (Tab::Added, "update-tab-added", c.added.len()),
                (Tab::Removed, "update-tab-removed", c.removed.len()),
                (Tab::Changed, "update-tab-changed", c.changed.len()),
            ] {
                let label = self.i18n.t_arg(key, "n", &n.to_string());
                if ui.selectable_label(self.update.tab == tab, label).clicked() {
                    self.update.tab = tab;
                }
            }
        });

        // Bottom: hint and buttons (drawn first).
        let mut action: Option<&'static str> = None;
        let takeable = c
            .changed
            .iter()
            .filter(|ch| !ch.old_trans.is_empty())
            .count();
        egui::TopBottomPanel::bottom("ximod-update-bottom")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t(self, "update-hint")).weak().small());
                if let Some(m) = &self.update.message {
                    ui.label(egui::RichText::new(m).weak().small());
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            takeable > 0,
                            egui::Button::new(
                                egui::RichText::new(self.i18n.t_arg(
                                    "update-take",
                                    "n",
                                    &takeable.to_string(),
                                ))
                                .strong(),
                            ),
                        )
                        .clicked()
                    {
                        action = Some("take");
                    }
                    let old_n = c.removed.iter().filter(|r| !r.date.is_empty()).count();
                    if ui
                        .add_enabled(old_n > 0, egui::Button::new(t(self, "update-forget")))
                        .clicked()
                    {
                        self.update.confirm_forget = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t(self, "btn-close")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if self.update.confirm_forget {
            let old_n = c.removed.iter().filter(|r| !r.date.is_empty()).count();
            let mut choice: Option<bool> = None;
            egui::Window::new(t(self, "update-forget-title"))
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label(
                        self.i18n
                            .t_arg("update-forget-confirm", "n", &old_n.to_string()),
                    );
                    ui.add_space(8.0);
                    let (ok, cancel) =
                        confirm_buttons(ui, &t(self, "btn-validate"), &t(self, "btn-cancel"));
                    if ok {
                        choice = Some(true);
                    }
                    if cancel {
                        choice = Some(false);
                    }
                });
            match choice {
                Some(true) => {
                    self.update.confirm_forget = false;
                    action = Some("forget");
                }
                Some(false) => self.update.confirm_forget = false,
                None => {}
            }
        }

        // Tables.
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let table_h = ui.available_height().max(80.0);
        let mut open_line: Option<usize> = None;
        let doc = &self.docs[self.active];
        let one = |s: &str| crate::ui::edit_window::one_line(s, 60);
        // The old / new source differences (A-08), computed as rows show.
        let diffs = std::cell::RefCell::new(std::mem::take(&mut self.update.diffs));
        egui::ScrollArea::vertical()
            .id_salt("ximod-update-table")
            .max_height(table_h)
            .show(ui, |ui| match self.update.tab {
                Tab::Added => {
                    egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .sense(egui::Sense::click())
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(
                            egui_extras::Column::initial(170.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::exact(90.0))
                        .column(egui_extras::Column::remainder().clip(true))
                        .header(row_h, |mut h| {
                            for k in ["update-col-edid", "update-col-field", "update-col-source"] {
                                h.col(|ui| {
                                    ui.strong(self.i18n.t(k));
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, c.added.len(), |mut row| {
                                let idx = c.added[row.index()];
                                let e = &doc.tm.entries[idx];
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&e.edid).monospace())
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!(
                                                "{}:{}",
                                                e.rec_sig, e.field_sig
                                            ))
                                            .monospace(),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(one(&e.source))
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                if row.response().double_clicked() {
                                    open_line = Some(idx);
                                }
                            });
                        });
                }
                Tab::Removed => {
                    egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(
                            egui_extras::Column::initial(170.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::exact(90.0))
                        .column(
                            egui_extras::Column::initial(220.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::remainder().clip(true))
                        .column(egui_extras::Column::exact(90.0))
                        .header(row_h, |mut h| {
                            for k in [
                                "update-col-edid",
                                "update-col-field",
                                "update-col-source",
                                "update-col-trans",
                                "update-col-date",
                            ] {
                                h.col(|ui| {
                                    ui.strong(self.i18n.t(k));
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, c.removed.len(), |mut row| {
                                let r = &c.removed[row.index()];
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&r.row.edid).monospace(),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!(
                                                "{}:{}",
                                                r.row.rec, r.row.field
                                            ))
                                            .monospace(),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(one(&r.row.source))
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(one(&r.row.trans))
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    let d = if r.date.is_empty() {
                                        self.i18n.t("update-pending-write")
                                    } else {
                                        r.date.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(d).weak().small())
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                            });
                        });
                }
                Tab::Changed => {
                    egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .sense(egui::Sense::click())
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(
                            egui_extras::Column::initial(140.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::exact(90.0))
                        .column(
                            egui_extras::Column::initial(180.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(
                            egui_extras::Column::initial(180.0)
                                .at_least(80.0)
                                .clip(true),
                        )
                        .column(egui_extras::Column::remainder().clip(true))
                        .header(row_h, |mut h| {
                            for k in [
                                "update-col-edid",
                                "update-col-field",
                                "update-col-old-source",
                                "update-col-new-source",
                                "update-col-old-trans",
                            ] {
                                h.col(|ui| {
                                    ui.strong(self.i18n.t(k));
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, c.changed.len(), |mut row| {
                                let ch = &c.changed[row.index()];
                                let e = &doc.tm.entries[ch.idx];
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&e.edid).monospace())
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!(
                                                "{}:{}",
                                                e.rec_sig, e.field_sig
                                            ))
                                            .monospace(),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                // Old and new source with their differences
                                // highlighted (A-08, T-A08-05).
                                let mut d = diffs.borrow_mut();
                                let diff = d.entry(ch.idx).or_insert_with(|| {
                                    crate::diff::compute(&ch.old_source, &e.source)
                                });
                                row.col(|ui| {
                                    let j =
                                        crate::ui::diff_view::one_line_job(ui, &diff.line, true);
                                    ui.add(egui::Label::new(j).truncate().selectable(false))
                                        .on_hover_text(&ch.old_source);
                                });
                                row.col(|ui| {
                                    let j =
                                        crate::ui::diff_view::one_line_job(ui, &diff.entry, false);
                                    ui.add(egui::Label::new(j).truncate().selectable(false))
                                        .on_hover_text(&e.source);
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(one(&ch.old_trans))
                                            .truncate()
                                            .selectable(false),
                                    )
                                    .on_hover_text(&ch.old_trans);
                                });
                                if row.response().double_clicked() {
                                    open_line = Some(ch.idx);
                                }
                            });
                        });
                }
            });
        self.update.diffs = diffs.into_inner();
        if let Some(idx) = open_line {
            self.open_edit_window(idx);
        }
        match action {
            Some("take") => self.update_take(),
            Some("forget") => self.update_forget(),
            _ => {}
        }
        close
    }

    // ------------------------------------------------------------------
    // Heuristique par mots (B-01)
    // ------------------------------------------------------------------

    /// The word index of a dictionary file: cached, or built in the
    /// background (`None` while it builds).
    fn fuzzy_index_for(
        &mut self,
        path: &std::path::Path,
        db: &Arc<crate::basedb::BaseDb>,
    ) -> Option<Arc<crate::fuzzy::FuzzyIndex>> {
        if let Some(ix) = self.fuzzy_cache.get(path) {
            return Some(ix.clone());
        }
        if !self.fuzzy_jobs.contains_key(path) {
            let (tx, rx) = std::sync::mpsc::channel();
            let db = db.clone();
            std::thread::spawn(move || {
                let rows = db.rows_for_fuzzy();
                let total = rows.len();
                let tx2 = tx.clone();
                let ix = crate::fuzzy::FuzzyIndex::build(rows, total, |d, t| {
                    tx2.send(FuzzyMsg::Progress(d, t)).is_ok()
                });
                let _ = tx.send(FuzzyMsg::Done(ix.map(Arc::new)));
            });
            self.fuzzy_jobs
                .insert(path.to_path_buf(), FuzzyJob { rx, progress: 0.0 });
        }
        None
    }

    /// Collect the finished indexes; the lowest progress of the running
    /// ones, if any.
    fn fuzzy_poll(&mut self, ctx: &egui::Context) -> Option<f32> {
        let mut done: Vec<(PathBuf, Option<Arc<crate::fuzzy::FuzzyIndex>>)> = Vec::new();
        let mut min_progress: Option<f32> = None;
        for (path, job) in self.fuzzy_jobs.iter_mut() {
            loop {
                match job.rx.try_recv() {
                    Ok(FuzzyMsg::Progress(d, t)) => {
                        job.progress = if t == 0 { 1.0 } else { d as f32 / t as f32 }
                    }
                    Ok(FuzzyMsg::Done(ix)) => {
                        done.push((path.clone(), ix));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        done.push((path.clone(), None));
                        break;
                    }
                }
            }
            if !done.iter().any(|(p, _)| p == path) {
                min_progress =
                    Some(min_progress.map_or(job.progress, |m: f32| m.min(job.progress)));
            }
        }
        for (path, ix) in done {
            self.fuzzy_jobs.remove(&path);
            if let Some(ix) = ix {
                self.fuzzy_cache.insert(path, ix);
            }
        }
        if !self.fuzzy_jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        min_progress
    }

    /// A dictionary file changed: its index and its word index are stale.
    fn invalidate_dict(&mut self, path: &std::path::Path) {
        self.file_dbs.invalidate(Some(path));
        self.fuzzy_cache.remove(path);
        self.frag_cache.remove(path);
    }

    /// The suggestions of a line (R-B01-04): from every active dictionary,
    /// best score first then stack order, at most eight. `None` while an
    /// index builds.
    fn vocab_suggestions(
        &mut self,
        line: usize,
    ) -> Option<Vec<(String, crate::fuzzy::Suggestion)>> {
        let Some(doc) = self.docs.get(self.active) else {
            return Some(Vec::new());
        };
        let (path, settings, source) = (
            doc.path.clone(),
            doc.settings.clone(),
            doc.tm
                .entries
                .get(line)
                .map(|e| e.source.clone())
                .unwrap_or_default(),
        );
        let Some(stack) = self.resolve_stack(&path, &settings) else {
            return Some(Vec::new());
        };
        let pct = self.config.fuzzy_threshold;
        let mut all: Vec<(usize, String, crate::fuzzy::Suggestion)> = Vec::new();
        let mut pending = false;
        for (li, l) in stack.levels.iter().enumerate() {
            match self.fuzzy_index_for(&l.path, &l.db) {
                Some(ix) => {
                    for sg in ix.suggest(&source, pct, 8) {
                        all.push((li, l.name(), sg));
                    }
                }
                None => pending = true,
            }
        }
        if pending {
            return None;
        }
        all.sort_by(|a, b| b.2.score.cmp(&a.2.score).then(a.0.cmp(&b.0)));
        all.dedup_by(|a, b| a.2.source == b.2.source && a.2.trans == b.2.trans);
        all.truncate(8);
        Some(all.into_iter().map(|(_, n, s)| (n, s)).collect())
    }

    /// « Utiliser » a suggestion or a candidate: the translation, En
    /// attente (D-16), with the provenance given (« heuristique » and its
    /// score, or the dictionary of the candidate, B-08).
    fn vocab_use(&mut self, line: usize, text: String, prov: String) {
        self.edit_set_trans(line, text, "vocab-use", &prov);
    }

    /// The entry the differences frame compares with the line (A-08): the
    /// one picked, else the first suggestion, else the first candidate
    /// (B-08) when there are several; `None` without any.
    fn vocab_current_pick(&self) -> Option<DiffPick> {
        let n_sug = self
            .vocab
            .suggestions
            .as_ref()
            .map(|l| l.len())
            .unwrap_or(0);
        let n_cand = self.vocab.candidates.len();
        match self.vocab.diff_pick {
            Some(DiffPick::Suggestion(i)) if i < n_sug => Some(DiffPick::Suggestion(i)),
            Some(DiffPick::Candidate(i)) if i < n_cand => Some(DiffPick::Candidate(i)),
            _ if n_sug > 0 => Some(DiffPick::Suggestion(0)),
            _ if n_cand >= 2 => Some(DiffPick::Candidate(0)),
            _ => None,
        }
    }

    /// Compute the differences for the current pick when it changed.
    fn vocab_refresh_diff(&mut self, line: usize) {
        let pick = self.vocab_current_pick();
        if self.vocab.diff.as_ref().map(|(p, _)| *p) == pick {
            return;
        }
        let Some(pick) = pick else {
            self.vocab.diff = None;
            return;
        };
        let Some(e) = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
        else {
            self.vocab.diff = None;
            return;
        };
        let diff = match pick {
            DiffPick::Suggestion(i) => {
                let sg = &self.vocab.suggestions.as_ref().unwrap()[i].1;
                crate::diff::compute(&e.source, &sg.source)
            }
            DiffPick::Candidate(i) => {
                crate::diff::compute(&e.trans, &self.vocab.candidates[i].trans)
            }
        };
        self.vocab.diff = Some((pick, diff));
    }

    /// A thin frame around the row the differences frame compares.
    fn mark_picked_row(ui: &egui::Ui, rect: egui::Rect) {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            2.0,
            egui::Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
        );
    }

    /// The candidate translations of a line's source across the stack
    /// (B-08), for the Vocabulaire tab.
    fn vocab_candidates(&mut self, line: usize) -> Vec<crate::ambiguity::Candidate> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        let Some(source) = doc.tm.entries.get(line).map(|e| e.source.clone()) else {
            return Vec::new();
        };
        match self.resolve_stack(&path, &settings) {
            Some(stack) => stack.candidates(&source),
            None => Vec::new(),
        }
    }

    /// Replace the translation of a line, En attente, as one undoable
    /// operation labelled `label_key` (Vocabulaire and instant translation),
    /// with the provenance `prov`.
    fn edit_set_trans(&mut self, line: usize, text: String, label_key: &str, prov: &str) {
        self.close_edit_session();
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        let Some(e) = doc.tm.entries.get(line) else {
            return;
        };
        if crate::vmad_lock::is_file_locked(e) {
            return;
        }
        let before = vec![(line, EntryState::capture(e))];
        let e = &mut doc.tm.entries[line];
        e.trans = text;
        e.prov = prov.to_string();
        e.set_translated_flags(!e.trans.is_empty());
        crate::status::set(e, crate::status::Status::Pending);
        let mut op = op_from_before(OpKind::Edit, &before, &mut doc.tm.entries);
        if !op.changes.is_empty() {
            op.label_arg = Some(self.i18n.t(label_key));
            doc.undo.push(op);
            self.pinned_rows.insert(line);
        }
        self.refresh_anomaly_lines(&[line]);
    }

    /// The fragment levels of the active document (B-14): text-only and
    /// personal first, indexes built once per file and session.
    fn instant_levels(&mut self) -> Vec<crate::instant::Level> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        let (path, settings) = (doc.path.clone(), doc.settings.clone());
        let Some(stack) = self.resolve_stack(&path, &settings) else {
            return Vec::new();
        };
        let mut levels = Vec::new();
        for l in &stack.levels {
            let ix = match self.frag_cache.get(&l.path) {
                Some(ix) => ix.clone(),
                None => {
                    let ix = Arc::new(crate::instant::FragmentIndex::build(&l.db));
                    self.frag_cache.insert(l.path.clone(), ix.clone());
                    ix
                }
            };
            if ix.is_empty() {
                continue;
            }
            let role = if l.db.is_text_only() && l.role != crate::dicts::Role::Personal {
                crate::dicts::Role::TextOnly
            } else {
                l.role
            };
            levels.push(crate::instant::Level {
                name: l.name(),
                role,
                index: ix,
            });
        }
        let mut levels = crate::instant::order_levels(levels);
        // The glossary of the pair first (R-B15-04).
        if let Some(a) = self.assistant()
            && !a.glossary.is_empty()
        {
            let mut ix = crate::instant::FragmentIndex::default();
            for e in a.glossary.entries() {
                let t = e.term.text(None);
                if !t.trim().is_empty() {
                    ix.add(&e.term.src, t);
                }
            }
            levels.insert(
                0,
                crate::instant::Level {
                    name: self.i18n.t("assist-tab-glossary"),
                    role: crate::dicts::Role::TextOnly,
                    index: Arc::new(ix),
                },
            );
        }
        levels
    }

    /// « 2 fragments traduits, 1 mot inconnu (open) ».
    fn instant_report(&self, o: &crate::instant::Outcome) -> String {
        let list = if o.unknown.is_empty() {
            String::new()
        } else {
            let mut u: Vec<&str> = o.unknown.iter().map(String::as_str).collect();
            u.dedup();
            u.truncate(6);
            format!(" ({})", u.join(", "))
        };
        self.i18n.t_args(
            "instant-done",
            &[
                ("n", &o.fragments.len().to_string()),
                ("unknown", &o.unknown.len().to_string()),
                ("list", &list),
            ],
        )
    }

    /// F9 / Ctrl+F9 in the edit window (R-B14-01, R-B14-03): the source
    /// selection — or the whole source, in one or two parts — translated
    /// by fragments into the Translation area.
    fn edit_instant(
        &mut self,
        vctx: &egui::Context,
        win: &mut crate::ui::edit_window::EditWindow,
        two_steps: bool,
    ) {
        use crate::instant::{adjust_case, at_sentence_start, translate};
        let line = win.line;
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let Some(e) = doc.tm.entries.get(line) else {
            return;
        };
        let source = e.source.clone();
        let trans = e.trans.clone();
        let src_chars: Vec<char> = source.chars().collect();
        let src_id = egui::Id::new(("ximod-edit-src-te", line));
        let trans_id = egui::Id::new(("ximod-edit-trans", line));
        // The source selection and cursor, kept while the focus is elsewhere.
        let src_range = egui::TextEdit::load_state(vctx, src_id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| {
                let (a, b) = (r.primary.index, r.secondary.index);
                (a.min(b).min(src_chars.len()), a.max(b).min(src_chars.len()))
            });
        // The whole source covered by the glossary: its translation,
        // composed (R-B15-04).
        let whole_arm = !matches!(src_range, Some((a, b)) if a < b)
            && !(two_steps && matches!(src_range, Some((c, _)) if c > 0 && c < src_chars.len()));
        let bases = self.assist_bases();
        if whole_arm
            && let Some(a) = self.assistant()
            && let Ok(t) = a.run(&source, true, true, &bases)
        {
            let prov = t.rule.prov();
            let message = self.i18n.t_arg(
                "instant-glossary",
                "path",
                &self.assist_outcome_path(&t, &a),
            );
            if !trans.trim().is_empty() && !self.instant_no_confirm {
                win.confirm = Some(crate::ui::edit_window::InstantConfirm {
                    text: t.text,
                    message,
                    prov,
                });
                return;
            }
            let n = t.text.chars().count();
            self.edit_set_trans(line, t.text, "instant-label", &prov);
            win.cursor_at = Some(n);
            win.message = Some(message);
            win.focus_request = true;
            return;
        }
        let levels = self.instant_levels();
        let (text, sel_start, outcome) = match src_range {
            Some((a, b)) if a < b => {
                let piece: String = src_chars[a..b].iter().collect();
                let o = translate(&piece, &levels);
                (piece, a, o)
            }
            Some((c, _)) if two_steps && c > 0 && c < src_chars.len() => {
                // A cursor inside a word cuts after that word.
                let mut c = c;
                while c < src_chars.len()
                    && src_chars[c].is_alphanumeric()
                    && src_chars[c - 1].is_alphanumeric()
                {
                    c += 1;
                }
                let left: String = src_chars[..c].iter().collect();
                let right: String = src_chars[c..].iter().collect();
                let mut o1 = translate(left.trim(), &levels);
                let o2 = translate(right.trim(), &levels);
                o1.text = format!("{} {}", o1.text.trim(), o2.text.trim());
                o1.fragments.extend(o2.fragments);
                o1.unknown.extend(o2.unknown);
                (source.clone(), 0, o1)
            }
            _ => (source.clone(), 0, translate(&source, &levels)),
        };
        if outcome.fragments.is_empty() {
            let shown: String = text.chars().take(40).collect();
            win.message = Some(self.i18n.t_arg("instant-none", "text", shown.trim()));
            return;
        }
        let message = self.instant_report(&outcome);
        let whole = src_range.is_none_or(|(a, b)| a >= b);
        if whole {
            // The whole source: the result replaces the translation.
            let new_text = adjust_case(&outcome.text, &text, true, true);
            let prov = crate::provenance::fuzzy(outcome.coverage());
            if !trans.trim().is_empty() && !self.instant_no_confirm {
                win.confirm = Some(crate::ui::edit_window::InstantConfirm {
                    text: new_text,
                    message,
                    prov,
                });
                return;
            }
            let n = new_text.chars().count();
            self.edit_set_trans(line, new_text, "instant-label", &prov);
            win.cursor_at = Some(n);
        } else {
            // A selection: inserted at the cursor of the Translation area,
            // or in place of its selection.
            let tr_chars: Vec<char> = trans.chars().collect();
            let (a, b) = egui::TextEdit::load_state(vctx, trans_id)
                .and_then(|st| st.cursor.char_range())
                .map(|r| {
                    let (a, b) = (r.primary.index, r.secondary.index);
                    (a.min(b).min(tr_chars.len()), a.max(b).min(tr_chars.len()))
                })
                .unwrap_or((tr_chars.len(), tr_chars.len()));
            let piece = adjust_case(
                &outcome.text,
                &text,
                at_sentence_start(&trans, a),
                at_sentence_start(&source, sel_start),
            );
            let mut new_text: String = tr_chars[..a].iter().collect();
            new_text.push_str(&piece);
            let after: String = tr_chars[b..].iter().collect();
            new_text.push_str(&after);
            let pos = a + piece.chars().count();
            // A piece inserted in the translation: the line stays the
            // translator's own text.
            self.edit_set_trans(line, new_text, "instant-label", crate::provenance::TYPED);
            win.cursor_at = Some(pos);
        }
        win.message = Some(message);
        win.focus_request = true;
    }

    /// The « Remplacer la traduction actuelle ? » dialog of F9 on the whole
    /// source (R-B14-01), with its « ne plus demander » for the session.
    fn edit_instant_confirm(
        &mut self,
        vctx: &egui::Context,
        win: &mut crate::ui::edit_window::EditWindow,
    ) {
        let Some(pending) = win.confirm.clone() else {
            return;
        };
        let mut no_more = self.instant_no_confirm;
        let (replace_l, cancel_l) = (self.i18n.t("instant-replace"), self.i18n.t("btn-cancel"));
        let mut choice: Option<bool> = None;
        egui::Window::new(self.i18n.t("instant-replace-title"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(vctx, |ui| {
                ui.add_space(4.0);
                ui.label(self.i18n.t("instant-replace-confirm"));
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width().max(320.0));
                    ui.add(egui::Label::new(&pending.text).wrap());
                });
                ui.label(egui::RichText::new(&pending.message).weak().small());
                ui.add_space(6.0);
                ui.checkbox(&mut no_more, self.i18n.t("instant-no-more"));
                ui.add_space(8.0);
                let (ok, cancel) = confirm_buttons(ui, &replace_l, &cancel_l);
                if ok {
                    choice = Some(true);
                }
                if cancel {
                    choice = Some(false);
                }
            });
        self.instant_no_confirm = no_more;
        match choice {
            Some(true) => {
                win.confirm = None;
                let n = pending.text.chars().count();
                self.edit_set_trans(win.line, pending.text, "instant-label", &pending.prov);
                win.cursor_at = Some(n);
                win.message = Some(pending.message);
                win.focus_request = true;
            }
            Some(false) => win.confirm = None,
            None => {}
        }
    }

    /// The Vocabulaire tab (B-01): suggestions of the line, with score,
    /// differing words, translation, provenance and « Utiliser ».
    fn vocab_tab(&mut self, ui: &mut egui::Ui, line: usize) -> Option<(String, String)> {
        let now = ui.input(|i| i.time);
        let (serial, revision) = self
            .docs
            .get(self.active)
            .map(|d| (d.serial, d.undo.revision))
            .unwrap_or((0, 0));
        let key = (serial, revision, line);
        // Recompute a tenth of a second after the line changes (R-B01-06).
        if self.vocab.key != Some(key) {
            match self.vocab.changed_at {
                None => {
                    self.vocab.changed_at = Some(now);
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(110));
                }
                Some(at) if now - at >= 0.1 => {
                    self.vocab.changed_at = None;
                    self.vocab.key = Some(key);
                    self.vocab.suggestions = self.vocab_suggestions(line);
                    self.vocab.fragments = self.vocab_fragments(line);
                    self.vocab.glossary = self.vocab_glossary(line);
                    self.vocab.pattern = self.vocab_pattern(line);
                    self.vocab.candidates = self.vocab_candidates(line);
                    self.vocab.diff_pick = None;
                    self.vocab.diff = None;
                }
                Some(_) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(60));
                }
            }
        } else if self.vocab.suggestions.is_none() {
            // An index was building: try again.
            self.vocab.suggestions = self.vocab_suggestions(line);
        }
        let progress = self.fuzzy_poll(ui.ctx());
        let pct = self.config.fuzzy_threshold;
        let mut use_text: Option<(String, String)> = None;
        let mut picked: Option<DiffPick> = None;
        let cur_pick = self.vocab_current_pick();
        // Everything scrolls together; the frame's margin is left out so
        // the fragments and the shortcuts stay above the button bar.
        let max_h = (ui.available_height() - 52.0).max(48.0);
        egui::ScrollArea::vertical()
            .id_salt("ximod-vocab")
            .max_height(max_h)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                // « Traductions candidates (n) » (B-08, R-B08-04): the distinct
                // translations of the source, each with its dictionaries and its
                // count; Utiliser takes one, En attente, with that provenance.
                if self.vocab.candidates.len() >= 2 {
                    let n = self.vocab.candidates.len().to_string();
                    ui.label(
                        egui::RichText::new(self.i18n.t_arg("vocab-candidates", "n", &n))
                            .strong()
                            .small(),
                    );
                    let use_l = self.i18n.t("vocab-use");
                    let current = self
                        .docs
                        .get(self.active)
                        .and_then(|d| d.tm.entries.get(line))
                        .map(|e| e.trans.trim().to_string())
                        .unwrap_or_default();
                    for (ci, c) in self.vocab.candidates.iter().enumerate() {
                        let row = ui.horizontal(|ui| {
                            let is_current = c.trans.trim() == current;
                            let text = egui::RichText::new(&c.trans);
                            let text = if is_current { text.strong() } else { text };
                            // A click compares the candidate with the line (A-08).
                            if ui
                                .add(
                                    egui::Label::new(text)
                                        .truncate()
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text(&c.trans)
                                .clicked()
                            {
                                picked = Some(DiffPick::Candidate(ci));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(!is_current, egui::Button::new(&use_l).small())
                                        .clicked()
                                    {
                                        use_text = Some((c.trans.clone(), c.prov()));
                                    }
                                    let dicts: Vec<String> = c
                                        .dicts
                                        .iter()
                                        .map(|(f, _, k)| {
                                            let times = self.i18n.t_arg(
                                                "vocab-candidate-count",
                                                "n",
                                                &k.to_string(),
                                            );
                                            self.i18n.paren(&crate::dict_stack::stem_of(f), &times)
                                        })
                                        .collect();
                                    ui.label(egui::RichText::new(dicts.join(" · ")).weak().small());
                                },
                            );
                        });
                        if cur_pick == Some(DiffPick::Candidate(ci)) {
                            Self::mark_picked_row(ui, row.response.rect);
                        }
                    }
                    ui.add_space(4.0);
                }
                ui.label(
                    egui::RichText::new(self.i18n.t_arg("vocab-title", "pct", &pct.to_string()))
                        .strong()
                        .small(),
                );
                match &self.vocab.suggestions {
                    None => {
                        let p = progress.unwrap_or(0.0);
                        ui.label(
                            egui::RichText::new(self.i18n.t_arg(
                                "vocab-indexing",
                                "pct",
                                &((p * 100.0) as u32).to_string(),
                            ))
                            .weak(),
                        );
                        ui.add(egui::ProgressBar::new(p).animate(true));
                    }
                    Some(list) if list.is_empty() => {
                        ui.label(egui::RichText::new(self.i18n.t("vocab-none")).weak());
                    }
                    Some(list) => {
                        let use_l = self.i18n.t("vocab-use");
                        let old_l = self.i18n.t("vocab-old");
                        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
                        {
                            for (si, (name, sg)) in list.iter().enumerate() {
                                let row = ui.horizontal(|ui| {
                                    ui.set_min_height(row_h);
                                    // Score: green at 100 %, yellow around 60 %, orange at the threshold.
                                    let t = sg.score as f32 / 100.0;
                                    let col = if t >= 0.9 {
                                        egui::Color32::from_rgb(183, 223, 180)
                                    } else if t >= 0.7 {
                                        egui::Color32::from_rgb(230, 230, 170)
                                    } else {
                                        egui::Color32::from_rgb(255, 226, 170)
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!(" {} % ", sg.score))
                                                .background_color(col)
                                                .color(egui::Color32::from_rgb(40, 40, 40))
                                                .monospace(),
                                        )
                                        .selectable(false),
                                    );
                                    // The source, differing words underlined.
                                    let mut job = egui::text::LayoutJob::default();
                                    let font = egui::TextStyle::Body.resolve(ui.style());
                                    let base = egui::TextFormat {
                                        font_id: font.clone(),
                                        color: ui.visuals().text_color(),
                                        ..Default::default()
                                    };
                                    let diff = egui::TextFormat {
                                        font_id: font.clone(),
                                        color: ui.visuals().warn_fg_color,
                                        underline: egui::Stroke::new(
                                            1.0_f32,
                                            ui.visuals().warn_fg_color,
                                        ),
                                        ..Default::default()
                                    };
                                    let differing: std::collections::HashSet<String> =
                                        sg.differing.iter().cloned().collect();
                                    for (i, piece) in sg.source.split(' ').enumerate() {
                                        if i > 0 {
                                            job.append(" ", 0.0, base.clone());
                                        }
                                        let norm = crate::fuzzy::words(piece)
                                            .into_iter()
                                            .next()
                                            .unwrap_or_default();
                                        let fmt = if differing.contains(&norm) {
                                            diff.clone()
                                        } else {
                                            base.clone()
                                        };
                                        job.append(piece, 0.0, fmt);
                                    }
                                    // A click compares this source with the line's (A-08).
                                    if ui
                                        .add(
                                            egui::Label::new(job)
                                                .truncate()
                                                .selectable(false)
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(&sg.source)
                                        .clicked()
                                    {
                                        picked = Some(DiffPick::Suggestion(si));
                                    }
                                    ui.label(egui::RichText::new(crate::icons::ARROW_RIGHT).weak());
                                    ui.add(
                                        egui::Label::new(&sg.trans).truncate().selectable(false),
                                    )
                                    .on_hover_text(&sg.trans);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.small_button(&use_l).clicked() {
                                                use_text = Some((
                                                    sg.trans.clone(),
                                                    crate::provenance::fuzzy(sg.score),
                                                ));
                                            }
                                            let mut origin = name.clone();
                                            if !sg.rec.is_empty() {
                                                origin = self.i18n.dot(
                                                    &origin,
                                                    &format!("{}:{}", sg.rec, sg.field),
                                                );
                                            }
                                            if sg.old {
                                                origin = self.i18n.paren(&origin, &old_l);
                                            }
                                            ui.label(egui::RichText::new(origin).weak().small());
                                        },
                                    );
                                });
                                if cur_pick == Some(DiffPick::Suggestion(si)) {
                                    Self::mark_picked_row(ui, row.response.rect);
                                }
                            }
                        }
                    }
                }
                // « Différences caractère par caractère » (A-08): the line
                // against the picked entry.
                if let Some((pick, diff)) = &self.vocab.diff {
                    let is_cand = matches!(pick, DiffPick::Candidate(_));
                    let (title, line_l, entry_l, swap_l, legend_l, word_l) = (
                        self.i18n.t("diff-title"),
                        self.i18n.t(if is_cand {
                            "diff-trans-line"
                        } else {
                            "diff-line"
                        }),
                        self.i18n.t(if is_cand {
                            "diff-candidate"
                        } else {
                            "diff-entry"
                        }),
                        self.i18n.t("diff-swap"),
                        self.i18n.t("diff-legend"),
                        self.i18n.t("diff-word-mode"),
                    );
                    let legend2_l = self.i18n.t("diff-legend-swapped");
                    ui.add_space(4.0);
                    let labels = crate::ui::diff_view::DiffLabels {
                        title: &title,
                        line: &line_l,
                        entry: &entry_l,
                        swap: &swap_l,
                        legend: &legend_l,
                        legend_swapped: &legend2_l,
                        word_mode: &word_l,
                    };
                    let mut swapped = self.vocab.diff_swapped;
                    crate::ui::diff_view::show(ui, diff, &mut swapped, &labels);
                    self.vocab.diff_swapped = swapped;
                }
            });
        if let Some(p) = picked {
            self.vocab.diff_pick = Some(p);
        }
        self.vocab_refresh_diff(line);
        // The glossary's terms in the line (R-B15-04).
        ui.add_space(2.0);
        if !self.vocab.glossary.is_empty() {
            let list: Vec<String> = self
                .vocab
                .glossary
                .iter()
                .map(|(s, t, g)| format!("{s} {} {t} ({g})", crate::icons::ARROW_RIGHT))
                .collect();
            ui.add(
                egui::Label::new(
                    egui::RichText::new(self.i18n.t_arg(
                        "vocab-glossary-terms",
                        "list",
                        &list.join(" · "),
                    ))
                    .small(),
                )
                .truncate(),
            )
            .on_hover_text(list.join("\n"));
        }
        // The pattern that applies (R-B16-06).
        if let Some((n, src, dst)) = &self.vocab.pattern {
            ui.label(
                egui::RichText::new(self.i18n.t_args(
                    "vocab-pattern",
                    &[("n", &n.to_string()), ("src", src), ("dst", dst)],
                ))
                .small(),
            );
        }
        // The fragments F9 would use (R-B14-05), and the shortcuts.
        let frags = if self.vocab.fragments.is_empty() {
            self.i18n.t("vocab-fragments-none")
        } else {
            let list: Vec<String> = self
                .vocab
                .fragments
                .iter()
                .map(|f| {
                    format!(
                        "{} {} {} ({})",
                        f.source,
                        crate::icons::ARROW_RIGHT,
                        f.trans,
                        crate::dict_stack::stem_of(&f.origin)
                    )
                })
                .collect();
            self.i18n
                .t_arg("vocab-fragments", "list", &list.join(" · "))
        };
        ui.add(egui::Label::new(egui::RichText::new(frags).small()).truncate())
            .on_hover_text(
                self.vocab
                    .fragments
                    .iter()
                    .map(|f| {
                        format!(
                            "{} {} {} ({})",
                            f.source,
                            crate::icons::ARROW_RIGHT,
                            f.trans,
                            f.origin
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        ui.label(
            egui::RichText::new(self.i18n.t("vocab-shortcuts"))
                .weak()
                .small(),
        );
        use_text
    }

    /// Open the propagation dialog on `origins` (A-12); false — and the
    /// status bar says so — when no other line has their source.
    fn open_propagate(&mut self, origins: &[usize], in_edit: bool) -> bool {
        self.close_edit_session();
        if self.docs.get(self.active).is_none() {
            return false;
        }
        // The lines of the other open files join (C-04, R-C04-03).
        let groups = self.propagate_groups(origins);
        if groups.is_empty() {
            self.status = self.i18n.t("propagate-none");
            return false;
        }
        self.propagate_dlg = Some(PropagateDialog { groups, in_edit });
        true
    }

    /// After F5 or Ctrl+↓ on one line (R-A12-02): the dialog, when the
    /// option is on and the line has duplicates that differ.
    fn propose_propagation(&mut self, in_edit: bool) {
        if !self.config.propose_propagation || self.propagate_dlg.is_some() {
            return;
        }
        let targets = self.selection.targets();
        let [line] = targets[..] else {
            return;
        };
        let has = self
            .docs
            .get(self.active)
            .is_some_and(|d| crate::propagate::has_differing_duplicates(&d.tm.entries, line));
        if has {
            self.open_propagate(&[line], in_edit);
        }
    }

    /// Appliquer of the propagation dialog: one undoable operation.
    fn propagate_apply(&mut self) {
        let Some(dlg) = self.propagate_dlg.take() else {
            return;
        };
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        let before: Vec<(usize, EntryState)> = dlg
            .groups
            .iter()
            .flat_map(|g| g.targets.iter().filter(|t| t.checked).map(|t| t.idx))
            .filter_map(|i| doc.tm.entries.get(i).map(|e| (i, EntryState::capture(e))))
            .collect();
        let before: Vec<(usize, EntryState)> = before
            .into_iter()
            .filter(|(i, _)| {
                dlg.groups.iter().any(|g| {
                    g.targets
                        .iter()
                        .any(|t| t.checked && t.idx == *i && t.file.is_none())
                })
            })
            .collect();
        let touched = crate::propagate::apply(&mut doc.tm.entries, &dlg.groups);
        let op = op_from_before(OpKind::Propagate, &before, &mut doc.tm.entries);
        let mut n = op.changes.len();
        if n > 0 {
            doc.undo.push(op);
        }
        // One operation in each other file (C-04, as B-25).
        let mut files: Vec<usize> = dlg
            .groups
            .iter()
            .flat_map(|g| {
                g.targets
                    .iter()
                    .filter(|t| t.checked)
                    .filter_map(|t| t.file)
            })
            .collect();
        files.sort_unstable();
        files.dedup();
        let mut in_others = 0;
        for di in files {
            let Some(d) = self.docs.get_mut(di) else {
                continue;
            };
            let before: Vec<(usize, EntryState)> = dlg
                .groups
                .iter()
                .flat_map(|g| {
                    g.targets
                        .iter()
                        .filter(|t| t.checked && t.file == Some(di))
                        .map(|t| t.idx)
                })
                .filter_map(|i| d.tm.entries.get(i).map(|e| (i, EntryState::capture(e))))
                .collect();
            crate::propagate::apply_file(&mut d.tm.entries, &dlg.groups, Some(di));
            let op = op_from_before(OpKind::Propagate, &before, &mut d.tm.entries);
            if !op.changes.is_empty() {
                n += op.changes.len();
                d.undo.push(op);
                in_others += 1;
            }
        }
        self.pinned_rows.extend(touched.iter().copied());
        self.refresh_anomaly_lines(&touched);
        self.status = self.i18n.t_arg("propagate-done", "n", &n.to_string());
        if in_others > 0 {
            self.status = self
                .i18n
                .dot(&self.status, &self.i18n.t("propagate-files-undo-note"));
        }
        if !touched.is_empty() {
            self.grid_scroll_to = Some(touched[0]);
        }
    }

    /// The propagation dialog (R-A12-01), drawn in `ctx` when it belongs
    /// there: the edit window's viewport when opened from it.
    fn propagate_dialog(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.propagate_dlg.as_ref() else {
            return;
        };
        if dlg.in_edit != self.edit_win_active_frame {
            return;
        }
        let many = dlg.groups.len() > 1;
        let total_targets: usize = dlg.groups.iter().map(|g| g.targets.len()).sum();
        let title = if many {
            self.i18n.t_args(
                "propagate-title-many",
                &[
                    ("groups", &dlg.groups.len().to_string()),
                    ("n", &total_targets.to_string()),
                ],
            )
        } else {
            let g = &dlg.groups[0];
            let text: String = g.source.chars().take(48).collect();
            let text = if g.source.chars().count() > 48 {
                format!("{}…", text.trim_end())
            } else {
                text
            };
            self.i18n.t_args(
                "propagate-title",
                &[("text", &text), ("n", &g.targets.len().to_string())],
            )
        };
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        let colors = self.config.status_colors;
        // The lines of other files are marked with their file (C-04).
        let is_all = self.docs.get(self.active).is_some_and(|d| d.all.is_some());
        let show_file = is_all
            || dlg
                .groups
                .iter()
                .any(|g| g.targets.iter().any(|t| t.file.is_some()));
        let mut headers = vec![self.i18n.t("propagate-col-apply")];
        if show_file {
            headers.push(self.i18n.t("col-file"));
        }
        headers.extend([
            self.i18n.t("col-grup"),
            self.i18n.t("col-field"),
            self.i18n.t("col-edid"),
            self.i18n.t("propagate-col-trans"),
            self.i18n.t("propagate-col-status"),
        ]);
        let (all_l, none_l, cancel_l) = (
            self.i18n.t("propagate-check-all"),
            self.i18n.t("propagate-check-none"),
            self.i18n.t("btn-cancel"),
        );
        let origin_l = self.i18n.t("propagate-origin");
        let mut dlg = self.propagate_dlg.take().unwrap();
        let mut action: Option<bool> = None;
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let entries = &doc.tm.entries;
        let docs = &self.docs;
        let own_name = doc.file_name();
        let checked: usize = dlg.groups.iter().map(|g| g.checked()).sum();
        let apply_l = self
            .i18n
            .t_arg("propagate-apply", "n", &checked.to_string());
        let row_h = ctx.style().text_styles[&egui::TextStyle::Body].size + 8.0;
        let screen = ctx.screen_rect();
        let (max_w, max_h) = (
            (screen.width() - 24.0).max(320.0),
            (screen.height() - 40.0).max(200.0),
        );
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([max_w.min(720.0), max_h.min(420.0)])
            .max_size([max_w, max_h])
            .show(ctx, |ui| {
                let (enter, esc) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    )
                });
                egui::TopBottomPanel::bottom("ximod-propagate-bottom").show_inside(ui, |ui| {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.small_button(&all_l).clicked() {
                            for g in dlg.groups.iter_mut() {
                                for t in g.targets.iter_mut() {
                                    t.checked = true;
                                }
                            }
                        }
                        if ui.small_button(&none_l).clicked() {
                            for g in dlg.groups.iter_mut() {
                                for t in g.targets.iter_mut() {
                                    t.checked = false;
                                }
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(&cancel_l).clicked() {
                                action = Some(false);
                            }
                            if ui
                                .add_enabled(
                                    checked > 0,
                                    egui::Button::new(egui::RichText::new(&apply_l).strong()),
                                )
                                .clicked()
                            {
                                action = Some(true);
                            }
                        });
                    });
                    ui.add_space(2.0);
                });
                egui::ScrollArea::vertical()
                    .id_salt("ximod-propagate-list")
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (gi, g) in dlg.groups.iter_mut().enumerate() {
                            // The origin line: what is propagated.
                            let (r, gg, b) = colors[g.status.index()];
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&origin_l).weak().small());
                                if many {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&g.source).strong())
                                            .truncate(),
                                    )
                                    .on_hover_text(&g.source);
                                    ui.label(crate::icons::ARROW_RIGHT);
                                }
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&g.trans).strong())
                                        .truncate(),
                                )
                                .on_hover_text(&g.trans);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!(
                                            " {} ",
                                            legends[g.status.index()]
                                        ))
                                        .background_color(egui::Color32::from_rgb(r, gg, b))
                                        .color(egui::Color32::from_rgb(40, 40, 40))
                                        .small(),
                                    )
                                    .selectable(false),
                                );
                            });
                            let avail_w = ui.available_width();
                            let file_w = if show_file { 150.0 } else { 0.0 };
                            let text_w = ((avail_w - 60.0 - 70.0 - 70.0 - 110.0 - 24.0 - file_w)
                                * 0.5)
                                .clamp(80.0, 320.0);
                            let mut tb = egui_extras::TableBuilder::new(ui)
                                .id_salt(("ximod-propagate-table", gi))
                                .striped(true)
                                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                .column(egui_extras::Column::exact(60.0));
                            if show_file {
                                tb = tb.column(egui_extras::Column::exact(file_w));
                            }
                            tb.column(egui_extras::Column::exact(70.0))
                                .column(egui_extras::Column::exact(70.0))
                                .column(egui_extras::Column::exact(text_w))
                                .column(egui_extras::Column::remainder())
                                .column(egui_extras::Column::exact(110.0))
                                .header(row_h, |mut h| {
                                    for t in &headers {
                                        h.col(|ui| {
                                            ui.label(egui::RichText::new(t).strong().small());
                                        });
                                    }
                                })
                                .body(|body| {
                                    body.rows(row_h, g.targets.len(), |mut row| {
                                        let t = &mut g.targets[row.index()];
                                        let (ents, file) = match t.file {
                                            Some(d) => match docs.get(d) {
                                                Some(d) => (&d.tm.entries, d.file_name()),
                                                None => return,
                                            },
                                            None => (entries, own_name.clone()),
                                        };
                                        let Some(e) = ents.get(t.idx) else {
                                            return;
                                        };
                                        let st = crate::status::derive(e);
                                        let (r, gg, b) = colors[st.index()];
                                        row.col(|ui| {
                                            ui.checkbox(&mut t.checked, "");
                                        });
                                        if show_file {
                                            row.col(|ui| {
                                                let f =
                                                    if is_all { e.plugin.clone() } else { file };
                                                let rt = if t.file.is_some() || is_all {
                                                    egui::RichText::new(&f).strong()
                                                } else {
                                                    egui::RichText::new(&f).weak()
                                                };
                                                ui.add(egui::Label::new(rt).truncate())
                                                    .on_hover_text(&f);
                                            });
                                        }
                                        row.col(|ui| {
                                            ui.label(&e.rec_sig);
                                        });
                                        row.col(|ui| {
                                            ui.label(&e.field_sig);
                                        });
                                        row.col(|ui| {
                                            ui.add(egui::Label::new(&e.edid).truncate())
                                                .on_hover_text(&e.edid);
                                        });
                                        row.col(|ui| {
                                            let text = if t.same {
                                                egui::RichText::new(&e.trans).weak()
                                            } else {
                                                egui::RichText::new(&e.trans)
                                            };
                                            ui.add(egui::Label::new(text).truncate())
                                                .on_hover_text(&e.trans);
                                        });
                                        row.col(|ui| {
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(10.0, 10.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                2.0,
                                                egui::Color32::from_rgb(r, gg, b),
                                            );
                                            ui.add(
                                                egui::Label::new(&legends[st.index()])
                                                    .truncate()
                                                    .selectable(false),
                                            );
                                        });
                                    });
                                });
                            ui.add_space(6.0);
                        }
                    });
                if enter && checked > 0 {
                    action = Some(true);
                }
                if esc {
                    action = Some(false);
                }
            });
        self.propagate_dlg = Some(dlg);
        match action {
            Some(true) => self.propagate_apply(),
            Some(false) => self.propagate_dlg = None,
            None => {}
        }
    }

    /// The glossary's terms that cover parts of the line (B-15).
    fn vocab_glossary(&mut self, line: usize) -> Vec<(String, String, String)> {
        let Some(source) = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
            .map(|e| e.source.clone())
        else {
            return Vec::new();
        };
        let Some(a) = self.assistant() else {
            return Vec::new();
        };
        a.glossary
            .terms_in(&source)
            .into_iter()
            .map(|e| (e.term.src.clone(), e.term.shown(), e.group.clone()))
            .collect()
    }

    /// The fragments the active dictionaries know in the line (B-14).
    fn vocab_fragments(&mut self, line: usize) -> Vec<crate::instant::Found> {
        let Some(source) = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
            .map(|e| e.source.clone())
        else {
            return Vec::new();
        };
        let levels = self.instant_levels();
        crate::instant::known_fragments(&source, &levels)
    }

    /// Update the main window title with the active file's directory (path
    /// without the file name), offset after the program name.
    fn update_title(&mut self, ctx: &egui::Context) {
        let base = self
            .i18n
            .t_arg("app-title-version", "version", crate::APP_VERSION);
        let title = match self.active_doc() {
            Some(doc) if doc.all.is_some() => {
                format!("{base}        {}", self.tab_name(self.active))
            }
            Some(doc) => {
                let f = doc.file_name();
                let named = match crate::games::plugin_display_name(&f) {
                    Some(clear) => self.i18n.paren(&f, clear),
                    None => f,
                };
                let mark = if doc.is_dirty() {
                    format!("{} ", crate::icons::DOT)
                } else {
                    String::new()
                };
                format!("{mark}{base}        {named}        {}", doc.dir())
            }
            None => base,
        };
        // Only when it changes: a viewport command asks egui for another
        // frame at once, so a title sent every frame kept the application
        // redrawing without pause (a full core, and the window manager
        // busy with it).
        if self.last_title.as_ref() != Some(&title) {
            self.last_title = Some(title.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    /// Capture the main window geometry so it can be persisted on exit.
    fn capture_main_geom(&mut self, ctx: &egui::Context) {
        if let Some((x, y, w, h)) = read_geometry(ctx) {
            self.config.window_x = x;
            self.config.window_y = y;
            self.config.window_width = w;
            self.config.window_height = h;
        }
    }

    /// True when the toolbar « Jeu » list holds a game — always the case with
    /// a document open. Opening files, saving and every dictionary command
    /// depend on it; the three other lists never block.
    /// A file whose game cannot be known (no header, nothing chosen in the
    /// toolbar): the « Jeu » list is asked for (D-95); nothing opens.
    fn ask_game(&mut self, path: &std::path::Path) -> bool {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let msg = self.i18n.t_arg("alert-game-unknown", "file", &name);
        self.status = msg.clone();
        self.alert = Some(msg);
        false
    }

    fn game_selected(&self) -> bool {
        self.active_doc().is_some() || !self.idle.game.trim().is_empty()
    }

    /// The (source, target) pair of the active settings, or an alert when one
    /// of the two lists is empty (dictionary commands need both).
    fn require_pair(&mut self) -> Option<(String, String)> {
        let s = self.active_settings();
        if s.source.trim().is_empty() || s.target.trim().is_empty() {
            let msg = self.i18n.t("alert-needs-languages");
            self.status = msg.clone();
            self.alert = Some(msg);
            return None;
        }
        Some((s.source.clone(), s.target.clone()))
    }

    /// Files given on the command line (« ouvrir avec », drag onto the
    /// executable): opened at the first frame, under the same rules as a drop.
    pub fn with_startup_files(mut self, files: Vec<PathBuf>) -> Self {
        self.startup_files = files;
        self
    }

    /// `--game <id>` on the command line: preset the « Jeu » list, so files
    /// given with it open at once (opening requires a game).
    pub fn with_startup_game(mut self, game: Option<String>) -> Self {
        if let Some(g) = game {
            self.idle.game = g.trim().to_string();
        }
        self
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let mut dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        dropped.append(&mut self.startup_files);
        // A FOMOD folder (or a mod archive) goes to the FOMOD window, with
        // or without a game (F-03, as in Architect).
        dropped.retain(|p| {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if p.is_dir() || matches!(ext.as_str(), "zip" | "7z") {
                self.open_fomod_translation();
                self.fomod_trans.open_path(p.clone());
                false
            } else {
                true
            }
        });
        // No game is needed beforehand (D-95): each file says its own, or
        // asks for it when it cannot.
        for p in dropped {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if matches!(ext.as_str(), "bsa" | "ba2") {
                self.open_archive(p);
            } else {
                self.load_path(p);
            }
        }
    }

    // ---------------------- independent windows ----------------------------

    fn open_flags(&mut self) {
        self.flag_filter.clear();
        self.flag_focus = 0;
        self.flag_filter_focus = true;
        self.show_flags = true;
        self.flags_needs_center = true;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-settings");
        let (w, h) = (self.config.settings_w, self.config.settings_h);
        let mut builder =
            egui::ViewportBuilder::default().with_title(self.i18n.t("settings-title"));
        // Apply size + position ONLY when opening (restored, or centered on first
        // launch) — re-applying every frame made the window jitter and fought
        // manual moves.
        let just_opened = self.settings_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.settings_x, self.config.settings_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
            self.settings_fit = true;
        }

        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.settings_needs_center = false;
            if just_opened || self.settings_focus_frames > 0 {
                // Bring it to the front (it would otherwise appear behind the
                // main window on first launch); kept up for a few frames at
                // the first start (F-02).
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.settings_focus_frames = self.settings_focus_frames.saturating_sub(1);
            }
            let (mut do_save, mut do_close) = self.settings_keyboard(vctx);
            // Enregistrer / Annuler live in a bottom panel, never pushed out
            // of view by a long tab (F-02).
            let mut bottom_h = 0.0_f32;
            egui::TopBottomPanel::bottom("ximod-settings-buttons").show(vctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let s = ui.button(self.i18n.t("btn-save"));
                    if self.settings_focus_dirty && self.settings_focus == Focus::Save {
                        s.request_focus();
                    }
                    if s.clicked() {
                        do_save = true;
                    }
                    let c = ui.button(self.i18n.t("btn-cancel"));
                    if self.settings_focus_dirty && self.settings_focus == Focus::Cancel {
                        c.request_focus();
                    }
                    if c.clicked() {
                        do_close = true;
                    }
                });
                ui.add_space(6.0);
                bottom_h = ui.min_rect().height();
            });
            // The body sits in a scroll area so, when the fitted size is clamped
            // to the monitor, everything stays reachable. Its natural content
            // size drives the auto-fit below.
            let mut content = egui::Vec2::ZERO;
            egui::CentralPanel::default().show(vctx, |ui| {
                let inner = egui::ScrollArea::both()
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        self.settings_body(ui);
                        ui.min_rect().size()
                    });
                content = inner.inner;
            });
            content.y += bottom_h + 8.0;
            // The control that has the keyboard focus wears a 2 px accent
            // outline, so the Tab ring is visible (F-02).
            if let Some(fid) = vctx.memory(|m| m.focused())
                && let Some(resp) = vctx.read_response(fid)
            {
                let accent = vctx.style().visuals.selection.stroke.color;
                vctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("ximod-settings-focus"),
                ))
                .rect_stroke(
                    resp.rect.expand(2.0),
                    4.0,
                    egui::Stroke::new(2.0_f32, accent),
                );
            }
            // Re-fit when the tab changes (their contents differ in size).
            if self.settings_tab != self.settings_last_tab {
                self.settings_last_tab = self.settings_tab;
                self.settings_fit = true;
            }
            // Auto-size to content, clamped to the monitor, on open / tab change.
            if self.settings_fit && content.x > 1.0 {
                let monitor = vctx
                    .input(|i| i.viewport().monitor_size)
                    .unwrap_or(egui::vec2(1920.0, 1080.0));
                let desired = egui::vec2(
                    (content.x + 30.0).min(monitor.x * 0.95),
                    (content.y + 30.0).min(monitor.y * 0.9),
                );
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                self.settings_fit = false;
            }
            // Focus has now been driven for this frame; stop forcing it so combos
            // can open and fields can be edited without being pulled back.
            self.settings_focus_dirty = false;
            if do_save {
                // An invalid profile keeps the settings open (D-03).
                if let Some(msg) = self.online_profiles_problem() {
                    self.notify_err(msg);
                    self.settings_tab = SettingsTab::Online;
                } else {
                    self.save_settings();
                    do_close = true;
                }
            }
            // Continuously remember position + size (persisted on program exit).
            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.settings_x = x;
                self.config.settings_y = y;
                self.config.settings_w = w;
                self.config.settings_h = h;
            }
            if do_close || vctx.input(|i| i.viewport().close_requested()) {
                self.show_settings = false;
            }
        });
    }

    /// Keyboard navigation for the Settings window. Returns `(save, close)`.
    ///
    /// Tab/Shift+Tab move a single focus (marking it dirty so egui's native focus
    /// is driven once), the arrows adjust the focused list/field, and Esc closes.
    /// Enter and Space are deliberately NOT consumed: egui then activates the
    /// focused control natively — switching a tab, opening a combo, editing a
    /// field, or pressing a button — exactly like a mouse click.
    fn settings_keyboard(&mut self, vctx: &egui::Context) -> (bool, bool) {
        let do_save = false;
        let mut do_close = false;
        let has_recent = !self.config.recent_files.is_empty();
        let tabs = [
            Focus::TabGeneral,
            Focus::TabRecent,
            Focus::TabOnline,
            Focus::TabAdvanced,
        ];
        let order: Vec<Focus> = match self.settings_tab {
            SettingsTab::General => ORDER_GENERAL.to_vec(),
            SettingsTab::Online => [&tabs[..], &[Focus::Save, Focus::Cancel]].concat(),
            SettingsTab::RecentFiles if has_recent => {
                [&tabs[..], &[Focus::Clear, Focus::Save, Focus::Cancel]].concat()
            }
            SettingsTab::RecentFiles => [&tabs[..], &[Focus::Save, Focus::Cancel]].concat(),
            SettingsTab::Advanced => {
                [&tabs[..], &[Focus::ConfigMode, Focus::Save, Focus::Cancel]].concat()
            }
        };
        if !order.contains(&self.settings_focus) {
            self.settings_focus = order[0];
        }
        let cur = order
            .iter()
            .position(|f| *f == self.settings_focus)
            .unwrap_or(0);

        vctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                self.settings_focus = order[(cur + 1) % order.len()];
                self.settings_focus_dirty = true;
            }
            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                self.settings_focus = order[(cur + order.len() - 1) % order.len()];
                self.settings_focus_dirty = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                do_close = true;
            }
            let aleft = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            let aright = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            let aup = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let adown = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            let prev = aup || aleft;
            let next = adown || aright;
            let dec = aleft || adown;
            let inc = aright || aup;

            match self.settings_focus {
                Focus::Language => {
                    let langs: Vec<String> = self
                        .country_languages
                        .languages_for(&self.temp_country)
                        .to_vec();
                    if !langs.is_empty() {
                        let idx = langs
                            .iter()
                            .position(|l| *l == self.temp_locale)
                            .unwrap_or(0);
                        if prev && idx > 0 {
                            self.temp_locale = langs[idx - 1].clone();
                        }
                        if next && idx + 1 < langs.len() {
                            self.temp_locale = langs[idx + 1].clone();
                        }
                    }
                }
                Focus::Theme => {
                    let v = Theme::variants();
                    let idx = v.iter().position(|t| *t == self.temp_theme).unwrap_or(0);
                    if prev && idx > 0 {
                        self.temp_theme = v[idx - 1];
                    }
                    if next && idx + 1 < v.len() {
                        self.temp_theme = v[idx + 1];
                    }
                }
                Focus::FontSize => {
                    if inc {
                        self.temp_font_size = (self.temp_font_size + 0.5).min(24.0);
                    }
                    if dec {
                        self.temp_font_size = (self.temp_font_size - 0.5).max(8.0);
                    }
                }
                Focus::MaxRecent => {
                    if inc {
                        self.temp_max_recent_files = (self.temp_max_recent_files + 1).min(20);
                    }
                    if dec {
                        self.temp_max_recent_files =
                            self.temp_max_recent_files.saturating_sub(1).max(1);
                    }
                }
                Focus::WinW => {
                    if inc {
                        self.temp_window_width = (self.temp_window_width + 10.0).min(3840.0);
                    }
                    if dec {
                        self.temp_window_width = (self.temp_window_width - 10.0).max(800.0);
                    }
                }
                Focus::WinH => {
                    if inc {
                        self.temp_window_height = (self.temp_window_height + 10.0).min(2160.0);
                    }
                    if dec {
                        self.temp_window_height = (self.temp_window_height - 10.0).max(600.0);
                    }
                }
                Focus::ConfigMode => {
                    use crate::config::ConfigMode;
                    if prev {
                        self.temp_config_mode = ConfigMode::User;
                    }
                    if next {
                        self.temp_config_mode = ConfigMode::Portable;
                    }
                }
                _ => {}
            }
        });
        (do_save, do_close)
    }

    fn settings_body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let g = ui.selectable_label(
                self.settings_tab == SettingsTab::General,
                self.i18n.t("settings-tab-general"),
            );
            if self.settings_focus_dirty && self.settings_focus == Focus::TabGeneral {
                g.request_focus();
            }
            if g.clicked() {
                self.settings_tab = SettingsTab::General;
                self.settings_focus = Focus::TabGeneral;
            }
            let r = ui.selectable_label(
                self.settings_tab == SettingsTab::RecentFiles,
                self.i18n.t("settings-tab-recent"),
            );
            if self.settings_focus_dirty && self.settings_focus == Focus::TabRecent {
                r.request_focus();
            }
            if r.clicked() {
                self.settings_tab = SettingsTab::RecentFiles;
                self.settings_focus = Focus::TabRecent;
            }
            let o = ui.selectable_label(
                self.settings_tab == SettingsTab::Online,
                self.i18n.t("settings-tab-online"),
            );
            if self.settings_focus_dirty && self.settings_focus == Focus::TabOnline {
                o.request_focus();
            }
            if o.clicked() {
                self.settings_tab = SettingsTab::Online;
                self.settings_focus = Focus::TabOnline;
            }
            let a = ui.selectable_label(
                self.settings_tab == SettingsTab::Advanced,
                self.i18n.t("settings-tab-advanced"),
            );
            if self.settings_focus_dirty && self.settings_focus == Focus::TabAdvanced {
                a.request_focus();
            }
            if a.clicked() {
                self.settings_tab = SettingsTab::Advanced;
                self.settings_focus = Focus::TabAdvanced;
            }
        });
        ui.separator();
        ui.add_space(6.0);

        match self.settings_tab {
            SettingsTab::General => self.settings_general(ui),
            SettingsTab::RecentFiles => self.settings_recent(ui),
            SettingsTab::Online => self.settings_online(ui),
            SettingsTab::Advanced => self.settings_advanced(ui),
        }
        ui.add_space(8.0);
    }

    /// A left-aligned label occupying a fixed width, so the controls that follow
    /// line up vertically across rows (used throughout the Settings form).
    fn settings_label(&self, ui: &mut egui::Ui, lw: f32, key: &str) {
        ui.allocate_ui_with_layout(
            egui::vec2(lw, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(self.i18n.t(key));
            },
        );
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) {
        // One fixed label column width → all controls align vertically.
        let lw = 170.0_f32;
        ui.horizontal_top(|ui| {
            // Flag button (opens the Flags window).
            let flag_size = egui::vec2(132.0, 88.0);
            let flag_path = self
                .countries
                .flag_for(&self.temp_country)
                .and_then(|f| crate::data::flags_dir().map(|d| d.join(f)))
                .filter(|p| p.is_file());
            let flag_resp = match &flag_path {
                Some(p) => ui.add_sized(
                    flag_size,
                    egui::ImageButton::new(
                        egui::Image::from_uri(format!("file://{}", p.display()))
                            .fit_to_exact_size(flag_size),
                    ),
                ),
                None => ui.add_sized(
                    flag_size,
                    egui::Button::new(egui::RichText::new("\u{1F3F3}").size(28.0)),
                ),
            };
            if self.settings_focus_dirty && self.settings_focus == Focus::Flag {
                flag_resp.request_focus();
            }
            if flag_resp
                .on_hover_text(self.i18n.t("settings-pick-country"))
                .clicked()
            {
                self.settings_focus = Focus::Flag;
                self.flags_purpose = FlagsPurpose::Locale;
                self.open_flags();
            }

            ui.add_space(10.0);

            ui.vertical(|ui| {
                ui.label(self.i18n.t("settings-country-name"));
                let mut endonym = self
                    .countries
                    .endonym_for(&self.temp_country, &self.temp_locale)
                    .unwrap_or("")
                    .to_string();
                ui.add_enabled(
                    false,
                    egui::TextEdit::multiline(&mut endonym)
                        .desired_width(240.0)
                        .desired_rows(2),
                );
            });

            ui.add_space(14.0);

            ui.vertical(|ui| {
                // Language (E-07): the selected country's languages first,
                // then every interface language, by endonym.
                let (country_langs, all_langs) = language_sections(
                    self.country_languages.languages_for(&self.temp_country),
                    self.i18n.available_locales(),
                    |c| self.i18n.display_name(c),
                );
                ui.horizontal(|ui| {
                    self.settings_label(ui, lw, "settings-language");
                    let current = self.i18n.display_name(&self.temp_locale);
                    {
                        let combo = egui::ComboBox::from_id_salt("settings_lang")
                            .selected_text(current)
                            .height(320.0)
                            .show_ui(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let sections = [
                                        ("settings-language-country", &country_langs),
                                        ("settings-language-all", &all_langs),
                                    ];
                                    for (title, list) in sections {
                                        if list.is_empty() {
                                            continue;
                                        }
                                        ui.label(
                                            egui::RichText::new(self.i18n.t(title))
                                                .small()
                                                .strong()
                                                .weak(),
                                        );
                                        for (locale, display) in list.iter() {
                                            if ui
                                                .selectable_label(
                                                    self.temp_locale == *locale,
                                                    display,
                                                )
                                                .clicked()
                                            {
                                                self.temp_locale = locale.clone();
                                            }
                                        }
                                        ui.separator();
                                    }
                                });
                            });
                        if self.settings_focus_dirty && self.settings_focus == Focus::Language {
                            combo.response.request_focus();
                        }
                        if combo.response.clicked() {
                            self.settings_focus = Focus::Language;
                        }
                    }
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    self.settings_label(ui, lw, "settings-theme");
                    let combo = egui::ComboBox::from_id_salt("settings_theme")
                        .selected_text(self.i18n.t(self.temp_theme.i18n_key()))
                        .show_ui(ui, |ui| {
                            for theme in Theme::variants() {
                                ui.selectable_value(
                                    &mut self.temp_theme,
                                    *theme,
                                    self.i18n.t(theme.i18n_key()),
                                );
                            }
                        });
                    if self.settings_focus_dirty && self.settings_focus == Focus::Theme {
                        combo.response.request_focus();
                    }
                    if combo.response.clicked() {
                        self.settings_focus = Focus::Theme;
                    }
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    self.settings_label(ui, lw, "settings-font-size");
                    let r = ui.add(
                        egui::DragValue::new(&mut self.temp_font_size)
                            .range(8.0..=24.0)
                            .speed(0.5),
                    );
                    if self.settings_focus_dirty && self.settings_focus == Focus::FontSize {
                        r.request_focus();
                    }
                    if r.clicked() {
                        self.settings_focus = Focus::FontSize;
                    }
                });
            });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.checkbox(&mut self.temp_toasts, self.i18n.t("settings-toasts"))
            .on_hover_text(self.i18n.t("settings-toasts-hint"));
        ui.checkbox(
            &mut self.temp_propose_propagation,
            self.i18n.t("settings-propose-propagation"),
        )
        .on_hover_text(self.i18n.t("settings-propose-propagation-hint"));
        // B-25: between the open files.
        ui.checkbox(
            &mut self.temp_propagate_files,
            self.i18n.t("settings-propagate-files"),
        )
        .on_hover_text(self.i18n.t("settings-propagate-files-tip"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-max-recent");
            let mut max_recent = self.temp_max_recent_files as i32;
            let r = ui.add(
                egui::DragValue::new(&mut max_recent)
                    .range(1..=20)
                    .speed(0.2),
            );
            if r.changed() {
                self.temp_max_recent_files = max_recent as usize;
            }
            if self.settings_focus_dirty && self.settings_focus == Focus::MaxRecent {
                r.request_focus();
            }
            if r.clicked() {
                self.settings_focus = Focus::MaxRecent;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-window-width");
            // Integer display/edit (rounds the persisted value if fractional).
            let mut w = self.temp_window_width.round() as i32;
            let r = ui.add(egui::DragValue::new(&mut w).range(800..=3840).speed(10));
            if r.changed() {
                self.temp_window_width = w as f32;
            }
            if self.settings_focus_dirty && self.settings_focus == Focus::WinW {
                r.request_focus();
            }
            if r.clicked() {
                self.settings_focus = Focus::WinW;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-window-height");
            let mut h = self.temp_window_height.round() as i32;
            let r = ui.add(egui::DragValue::new(&mut h).range(600..=2160).speed(10));
            if r.changed() {
                self.temp_window_height = h as f32;
            }
            if self.settings_focus_dirty && self.settings_focus == Focus::WinH {
                r.request_focus();
            }
            if r.clicked() {
                self.settings_focus = Focus::WinH;
            }
        });

        // ---- Base folders (auto-translation) ----
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-bases")).strong());
        ui.label(
            egui::RichText::new(self.i18n.t("settings-bases-hint"))
                .weak()
                .small(),
        );
        ui.add_space(4.0);
        let browse = self.i18n.t("settings-browse");
        let clear = self.i18n.t("btn-clear");
        let field_w = 360.0;
        // Game bases folder.
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-game-bases");
            ui.add(
                egui::TextEdit::singleline(&mut self.temp_game_bases_dir).desired_width(field_w),
            );
            if ui.button(&browse).clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_directory(browse_folder_start(&self.temp_game_bases_dir))
                    .pick_folder()
            {
                self.temp_game_bases_dir = p.display().to_string();
            }
            if ui.button(&clear).clicked() {
                self.temp_game_bases_dir.clear();
            }
        });
        ui.add_space(4.0);
        // Mod translations folder.
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-mod-bases");
            ui.add(egui::TextEdit::singleline(&mut self.temp_mod_bases_dir).desired_width(field_w));
            if ui.button(&browse).clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_directory(browse_folder_start(&self.temp_mod_bases_dir))
                    .pick_folder()
            {
                self.temp_mod_bases_dir = p.display().to_string();
            }
            if ui.button(&clear).clicked() {
                self.temp_mod_bases_dir.clear();
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-tools")).strong());
        ui.label(
            egui::RichText::new(self.i18n.t("settings-tools-hint"))
                .weak()
                .small(),
        );
        ui.add_space(4.0);
        // ESP-ESM Translator root folder (portable install).
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-eet-dir");
            ui.add(egui::TextEdit::singleline(&mut self.temp_eet_dir).desired_width(field_w));
            if ui.button(&browse).clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_directory(browse_folder_start(&self.temp_eet_dir))
                    .pick_folder()
            {
                self.temp_eet_dir = p.display().to_string();
            }
            if ui.button(&clear).clicked() {
                self.temp_eet_dir.clear();
            }
        });
        ui.add_space(4.0);
        // xTranslator root folder (portable install).
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-xtranslator-dir");
            ui.add(
                egui::TextEdit::singleline(&mut self.temp_xtranslator_dir).desired_width(field_w),
            );
            if ui.button(&browse).clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_directory(browse_folder_start(&self.temp_xtranslator_dir))
                    .pick_folder()
            {
                self.temp_xtranslator_dir = p.display().to_string();
            }
            if ui.button(&clear).clicked() {
                self.temp_xtranslator_dir.clear();
            }
        });
    }

    fn settings_recent(&mut self, ui: &mut egui::Ui) {
        if self.config.recent_files.is_empty() {
            ui.label(self.i18n.t("settings-no-recent"));
            return;
        }
        let mut remove: Option<usize> = None;
        for (idx, path) in self.config.recent_files.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}.", idx + 1));
                if Self::icon_button(ui, "delete.png", crate::icons::X, 18.0).clicked() {
                    remove = Some(idx);
                }
                // Truncated: the full path shows on hover by itself.
                ui.add(egui::Label::new(path.display().to_string()).truncate());
            });
        }
        ui.add_space(8.0);
        // Destructive: armed by a first click, done by a second (F-06).
        let clear = if self.clear_recent_armed {
            ui.button(
                egui::RichText::new(self.i18n.t("btn-click-again"))
                    .strong()
                    .color(ui.visuals().warn_fg_color),
            )
        } else {
            ui.button(self.i18n.t("btn-clear"))
        };
        if self.settings_focus_dirty && self.settings_focus == Focus::Clear {
            clear.request_focus();
        }
        if clear.clicked() {
            if self.clear_recent_armed {
                self.config.recent_files.clear();
                let _ = self.config.save();
                self.clear_recent_armed = false;
            } else {
                self.clear_recent_armed = true;
            }
        } else if self.clear_recent_armed && ui.input(|i| i.pointer.any_click()) {
            // A click elsewhere disarms.
            self.clear_recent_armed = false;
        }
        if let Some(idx) = remove
            && idx < self.config.recent_files.len()
        {
            self.config.recent_files.remove(idx);
            let _ = self.config.save();
        }
    }

    /// Paramètres ▸ Avancé (E-05): where the configuration lives. Two radios,
    /// the current folder in clear, a button opening it, and the notice that a
    /// change is applied at the next start (R-E05-03).
    fn settings_advanced(&mut self, ui: &mut egui::Ui) {
        use crate::config::ConfigMode;
        let paths = crate::config::paths::get();
        let lw = 170.0_f32;

        // ---- Sauvegardes (B-11, figure B.3 repère 1) ----
        ui.label(egui::RichText::new(self.i18n.t("settings-backups")).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.temp_autosave_enabled,
            self.i18n.t("settings-autosave"),
        );
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.add_enabled_ui(self.temp_autosave_enabled, |ui| {
                ui.label(self.i18n.t("settings-autosave-every"));
                let r = crate::backup::AUTOSAVE_MINUTES_RANGE;
                let mut v = self.temp_autosave_minutes as i32;
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .range(*r.start() as i32..=*r.end() as i32)
                            .speed(1),
                    )
                    .changed()
                {
                    self.temp_autosave_minutes = v as u32;
                }
                ui.label(self.i18n.t("settings-autosave-minutes"));
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("settings-backup-copies"));
            let r = crate::backup::COPIES_RANGE;
            let mut v = self.temp_backup_copies as i32;
            if ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(*r.start() as i32..=*r.end() as i32)
                        .speed(1),
                )
                .changed()
            {
                self.temp_backup_copies = v as usize;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "settings-autosave-dir");
            ui.add(
                egui::TextEdit::singleline(&mut self.temp_autosave_dir)
                    .hint_text(paths.autosave_dir.display().to_string())
                    .desired_width(300.0),
            );
            if ui.button(self.i18n.t("settings-browse")).clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_directory(browse_folder_start(&self.temp_autosave_dir))
                    .pick_folder()
            {
                self.temp_autosave_dir = p.display().to_string();
            }
            if ui.button(self.i18n.t("settings-open-folder")).clicked() {
                let dir = if self.temp_autosave_dir.trim().is_empty() {
                    paths.autosave_dir.clone()
                } else {
                    PathBuf::from(self.temp_autosave_dir.trim())
                };
                let _ = std::fs::create_dir_all(&dir);
                crate::fonts::open_path(&dir);
            }
        });
        ui.add_space(4.0);
        // The mod's own dictionary (B-03).
        ui.checkbox(
            &mut self.temp_mod_dict_auto,
            self.i18n.t("settings-mod-dict-auto"),
        )
        .on_hover_text(self.i18n.t("settings-mod-dict-auto-tip"));
        // The identity of the plugin (B-12).
        ui.checkbox(&mut self.temp_check_crc, self.i18n.t("settings-check-crc"))
            .on_hover_text(self.i18n.t("settings-check-crc-tip"));
        // The old data of the mod dictionaries (B-07).
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label(self.i18n.t("settings-old-data-months"));
            let mut v = self.temp_old_data_months as i32;
            if ui
                .add(egui::DragValue::new(&mut v).range(0..=240).speed(1))
                .changed()
            {
                self.temp_old_data_months = v.max(0) as u32;
            }
        });
        // The word heuristic (B-01).
        ui.horizontal(|ui| {
            ui.label(self.i18n.t("settings-fuzzy-threshold"));
            let mut v = self.temp_fuzzy_threshold as i32;
            if ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(5..=50)
                        .speed(1)
                        .suffix(" %"),
                )
                .changed()
            {
                self.temp_fuzzy_threshold = v.clamp(5, 50) as u8;
            }
        });
        // The normalized tier of the stack (B-19).
        ui.checkbox(&mut self.temp_normalize, self.i18n.t("settings-normalize"))
            .on_hover_text(self.i18n.t("settings-normalize-tip"));
        // The generalization of an Ignoré by hand (B-18, D-75).
        ui.checkbox(
            &mut self.temp_propose_generalization,
            self.i18n.t("settings-propose-generalization"),
        )
        .on_hover_text(self.i18n.t("settings-propose-generalization-tip"));
        // The personal dictionary (B-04).
        ui.checkbox(
            &mut self.temp_personal_autofill,
            self.i18n.t("settings-personal-autofill"),
        );
        ui.checkbox(
            &mut self.temp_personal_keep_field,
            self.i18n.t("settings-personal-keep-field"),
        )
        .on_hover_text(self.i18n.t("settings-personal-keep-field-tip"));

        // ---- Sorties (E-06): the original kept before an export
        // overwrites a file.
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-outputs")).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.temp_keep_original,
            self.i18n.t("settings-keep-original"),
        )
        .on_hover_text(self.i18n.t("settings-keep-original-tip"));
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label(self.i18n.t("settings-original-copies"));
            let r = crate::backup::ORIGINAL_COPIES_RANGE;
            let mut v = self.temp_original_copies as i32;
            if ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(*r.start() as i32..=*r.end() as i32)
                        .speed(1),
                )
                .changed()
            {
                self.temp_original_copies = v as usize;
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-config-location")).strong());
        ui.add_space(4.0);
        let user_path = paths
            .config_dir_for(ConfigMode::User)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".into());
        let user_label = self.i18n.t_arg("config-mode-user", "path", &user_path);
        let r1 = ui.radio_value(&mut self.temp_config_mode, ConfigMode::User, user_label);
        if self.settings_focus_dirty && self.settings_focus == Focus::ConfigMode {
            r1.request_focus();
        }
        let r2 = ui.radio_value(
            &mut self.temp_config_mode,
            ConfigMode::Portable,
            self.i18n.t("config-mode-portable"),
        );
        if r1.clicked() || r2.clicked() {
            self.settings_focus = Focus::ConfigMode;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let cur = paths.config_dir.display().to_string();
            ui.label(self.i18n.t_arg("config-mode-current", "path", &cur));
            if ui.button(self.i18n.t("settings-open-folder")).clicked() {
                crate::fonts::open_path(&paths.config_dir);
            }
        });
        if paths.read_only {
            let msg = self.i18n.t_arg(
                "config-mode-not-writable",
                "path",
                &paths.config_dir.display().to_string(),
            );
            ui.colored_label(egui::Color32::from_rgb(220, 80, 60), msg);
        }
        if self.temp_config_mode != paths.mode {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(self.i18n.t("config-mode-restart")).weak());
        }

        // ---- Annulation (A-01, R-A01-03) ----
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-undo")).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.settings_label(ui, 170.0, "settings-undo-depth");
            let mut v = self.temp_undo_depth as i32;
            let range = crate::undo::DEPTH_RANGE;
            if ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(*range.start() as i32..=*range.end() as i32)
                        .speed(1),
                )
                .changed()
            {
                self.temp_undo_depth = v as usize;
            }
        });
        ui.label(
            egui::RichText::new(self.i18n.t("settings-undo-hint"))
                .weak()
                .small(),
        );

        // ---- Historique (A-18) ----
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-history")).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.temp_history_enabled,
            self.i18n.t("settings-history-enabled"),
        );
        ui.horizontal(|ui| {
            self.settings_label(ui, 170.0, "settings-history-max");
            let range = crate::history::MAX_RANGE;
            ui.add_enabled(
                self.temp_history_enabled,
                egui::DragValue::new(&mut self.temp_history_max)
                    .range(*range.start()..=*range.end())
                    .speed(100),
            );
        });
        ui.label(
            egui::RichText::new(self.i18n.t("settings-history-hint"))
                .weak()
                .small(),
        );
    }

    fn save_settings(&mut self) {
        // Online (D-01 / D-02): the settings, then the keys to the keyring.
        self.config.online = self.temp_online.clone();
        self.online_save_keys();
        let locale_changed = self.config.locale != self.temp_locale;
        self.config.locale = self.temp_locale.clone();
        self.config.country = self.temp_country.clone();
        self.config.first_start_done = true;
        self.config.theme = self.temp_theme;
        self.config.font_size = self.temp_font_size;
        self.config.max_recent_files = self.temp_max_recent_files;
        self.config.window_width = self.temp_window_width;
        self.config.window_height = self.temp_window_height;

        // Base folders: persist and, if they changed, drop the cached index so it
        // is rebuilt from the new folders on the next load / reload.
        let bases_changed = self.config.game_bases_dir != self.temp_game_bases_dir
            || self.config.mod_bases_dir != self.temp_mod_bases_dir;
        self.config.game_bases_dir = self.temp_game_bases_dir.trim().to_string();
        self.config.mod_bases_dir = self.temp_mod_bases_dir.trim().to_string();
        if bases_changed {
            self.file_dbs.invalidate(None);
        }
        self.config.eet_dir = self.temp_eet_dir.trim().to_string();
        self.config.xtranslator_dir = self.temp_xtranslator_dir.trim().to_string();
        // Backups (B-11): the scheduler follows the new values at once.
        self.config.autosave_enabled = self.temp_autosave_enabled;
        self.config.toasts = self.temp_toasts;
        self.config.propose_propagation = self.temp_propose_propagation;
        self.config.propagate_files = self.temp_propagate_files;
        let mr = crate::backup::AUTOSAVE_MINUTES_RANGE;
        self.config.autosave_minutes = self.temp_autosave_minutes.clamp(*mr.start(), *mr.end());
        self.config.autosave_dir = self.temp_autosave_dir.trim().to_string();
        let cr = crate::backup::COPIES_RANGE;
        self.config.backup_copies = self.temp_backup_copies.clamp(*cr.start(), *cr.end());
        self.config.keep_original = self.temp_keep_original;
        self.config.check_crc = self.temp_check_crc;
        let or = crate::backup::ORIGINAL_COPIES_RANGE;
        self.config.original_copies = self.temp_original_copies.clamp(*or.start(), *or.end());
        self.config.mod_dict_auto = self.temp_mod_dict_auto;
        self.config.personal_autofill = self.temp_personal_autofill;
        self.config.personal_keep_field = self.temp_personal_keep_field;
        self.config.old_data_months = self.temp_old_data_months.min(240);
        self.config.fuzzy_threshold = self.temp_fuzzy_threshold.clamp(5, 50);
        self.config.normalize = self.temp_normalize;
        self.config.propose_generalization = self.temp_propose_generalization;
        self.autosave.set_minutes(self.config.autosave_minutes);
        self.autosave.set_dir(self.config.autosave_dir());
        let depth = self.temp_undo_depth.clamp(
            *crate::undo::DEPTH_RANGE.start(),
            *crate::undo::DEPTH_RANGE.end(),
        );
        self.config.history_enabled = self.temp_history_enabled;
        self.config.history_max = self.temp_history_max.clamp(
            *crate::history::MAX_RANGE.start(),
            *crate::history::MAX_RANGE.end(),
        );
        if depth != self.config.undo_depth {
            self.config.undo_depth = depth;
            for d in &mut self.docs {
                d.undo.set_capacity(depth);
            }
        }

        if locale_changed {
            self.i18n.set_locale(&self.temp_locale);
            // Generated comments are worded in the interface language.
            self.comment_cache.clear();
        }
        // Resize the MAIN window live so the chosen size takes effect now (and is
        // then persisted by the per-frame geometry capture).
        self.pending_main_resize =
            Some(egui::vec2(self.temp_window_width, self.temp_window_height));
        // Theme and font size are re-applied to the main context each frame in
        // `update`, so nothing else to do here.
        let _ = self.config.save();
        self.notify_ok(self.i18n.t("status-settings-saved"));

        // Configuration location (E-05): copy everything to the other place and
        // lay down / set aside the marker; the mode changes at the next start.
        let paths = crate::config::paths::get();
        if self.temp_config_mode != paths.mode {
            match paths.migrate_to(self.temp_config_mode) {
                Ok(dir) => {
                    self.alert = Some(format!(
                        "{}\n{}",
                        self.i18n.t("config-mode-restart"),
                        self.i18n
                            .t_arg("config-mode-current", "path", &dir.display().to_string())
                    ));
                }
                Err(crate::config::paths::MigrateError::NotWritable(dir)) => {
                    self.temp_config_mode = paths.mode;
                    self.alert = Some(self.i18n.t_arg(
                        "config-mode-not-writable",
                        "path",
                        &dir.display().to_string(),
                    ));
                }
                Err(e) => {
                    self.temp_config_mode = paths.mode;
                    self.alert = Some(format!("{}\n{e}", self.i18n.t("config-mode-failed")));
                }
            }
        }
    }

    fn flags_window(&mut self, ctx: &egui::Context) {
        if !self.show_flags {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-flags");
        let (w, h) = (self.config.flags_w, self.config.flags_h);
        let mut builder = egui::ViewportBuilder::default().with_title(self.i18n.t("flags-title"));
        let just_opened = self.flags_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.flags_x, self.config.flags_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }

        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.flags_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let mut close = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                close = self.flags_body(ui, vctx);
            });
            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.flags_x = x;
                self.config.flags_y = y;
                self.config.flags_w = w;
                self.config.flags_h = h;
            }
            if close || vctx.input(|i| i.viewport().close_requested()) {
                self.show_flags = false;
            }
        });
    }

    /// Draws the flag grid; returns true when the window should close.
    fn flags_body(&mut self, ui: &mut egui::Ui, vctx: &egui::Context) -> bool {
        let dir = crate::data::flags_dir();
        // The names in the interface language, sorted by them; the filter
        // also takes the English name, the endonyms and the code (E-11).
        let loc = self.i18n.current_locale().to_string();
        let entries: Vec<(String, String, std::path::PathBuf)> = match &dir {
            Some(d) => crate::data::props::country_rows(
                &self.countries,
                &self.country_names,
                &loc,
                &self.flag_filter,
            )
            .into_iter()
            .filter_map(|(a3, label)| {
                let c = self.countries.countries.iter().find(|c| c.a3 == a3)?;
                (!c.flag.is_empty()).then(|| (a3, label, d.join(&c.flag)))
            })
            .filter(|(_, _, p)| p.is_file())
            .collect(),
            None => Vec::new(),
        };

        // Keyboard navigation over the grid (arrows + Page/Home/End + Enter/Esc).
        let mut scroll_to: Option<f32> = None;
        if !entries.is_empty() {
            self.flag_focus = self.flag_focus.min(entries.len() - 1);
            let n = entries.len();
            // One "page" ≈ 4 rows.
            let page = FLAG_COLUMNS * 4;
            let mut k = self.flag_focus;
            let (left, right, up, down, pgup, pgdn, home, end, enter, esc) = vctx.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::PageUp),
                    i.key_pressed(egui::Key::PageDown),
                    i.key_pressed(egui::Key::Home),
                    i.key_pressed(egui::Key::End),
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if left && k > 0 {
                k -= 1;
            }
            if right && k + 1 < n {
                k += 1;
            }
            if up && k >= FLAG_COLUMNS {
                k -= FLAG_COLUMNS;
            }
            if down && k + FLAG_COLUMNS < n {
                k += FLAG_COLUMNS;
            }
            if pgup {
                k = k.saturating_sub(page);
            }
            if pgdn {
                k = (k + page).min(n - 1);
            }
            if home {
                k = 0;
            }
            if end {
                k = n - 1;
            }
            if k != self.flag_focus || pgup || pgdn || home || end || up || down {
                scroll_to = Some((k / FLAG_COLUMNS) as f32 * FLAG_ROW_H);
            }
            self.flag_focus = k;
            if esc {
                return true;
            }
            if enter {
                // Entrée chooses the focused flag for the purpose the window
                // was opened for (locale, Cible, interface translator) — it
                // used to set the locale's country whatever the purpose.
                let a3 = entries[self.flag_focus].0.clone();
                match self.flags_purpose {
                    FlagsPurpose::Locale => self.choose_country(a3),
                    FlagsPurpose::Target => self.choose_target_country(&a3),
                    FlagsPurpose::Translation => self.tr_choose_country(a3),
                }
                return true;
            }
        }

        ui.horizontal(|ui| {
            ui.label(self.i18n.t("flags-filter"));
            let resp =
                ui.add(egui::TextEdit::singleline(&mut self.flag_filter).desired_width(220.0));
            // The cursor is in the field as the window opens: type the
            // country, Entrée chooses the first match.
            if self.flag_filter_focus {
                self.flag_filter_focus = false;
                resp.request_focus();
            }
            if Self::icon_button(ui, "delete.png", crate::icons::X, 20.0).clicked() {
                self.flag_filter.clear();
            }
        });
        ui.separator();

        if entries.is_empty() {
            ui.label(self.i18n.t("flags-none"));
            return false;
        }

        let mut chosen: Option<String> = None;
        let rows = entries.len().div_ceil(FLAG_COLUMNS);
        let focus = self.flag_focus;
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(off) = scroll_to {
            area = area.vertical_scroll_offset(off);
        }
        area.show_rows(ui, FLAG_ROW_H, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for col in 0..FLAG_COLUMNS {
                        let idx = row * FLAG_COLUMNS + col;
                        let Some((a3, label, path)) = entries.get(idx) else {
                            break;
                        };
                        ui.vertical(|ui| {
                            // Single indicator: only the flag under the
                            // keyboard cursor is highlighted.
                            let selected = idx == focus;
                            let img = egui::Image::from_uri(format!("file://{}", path.display()))
                                .fit_to_exact_size(egui::vec2(FLAG_CELL_W, FLAG_CELL_H));
                            let resp = ui.add_sized(
                                egui::vec2(FLAG_CELL_W, FLAG_CELL_H),
                                egui::ImageButton::new(img).selected(selected),
                            );
                            // Never let a flag button keep egui's native focus
                            // ring — it would linger on the initially-focused
                            // one while our own cursor moves. Keyboard nav is
                            // handled entirely by `flag_focus` above.
                            resp.surrender_focus();
                            if resp.on_hover_text(label).clicked() {
                                chosen = Some(a3.clone());
                            }
                            ui.allocate_ui(egui::vec2(FLAG_CELL_W, 18.0), |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(label).small())
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                            });
                        });
                    }
                });
            }
        });

        if let Some(a3) = chosen {
            match self.flags_purpose {
                FlagsPurpose::Locale => self.choose_country(a3),
                FlagsPurpose::Target => self.choose_target_country(&a3),
                FlagsPurpose::Translation => self.tr_choose_country(a3),
            }
            return true;
        }
        false
    }

    /// Toolbar « Cible » button: the official languages of the country become
    /// the « Cible » list; a single language is selected at once, several
    /// leave the choice to the user.
    fn choose_target_country(&mut self, a3: &str) {
        let langs: Vec<String> = self
            .countries
            .by_a3(a3)
            .map(|c| c.languages.iter().map(|l| l.iso3.to_lowercase()).collect())
            .unwrap_or_default();
        let mut langs = langs;
        langs.dedup();
        let picked = if langs.len() == 1 {
            langs[0].clone()
        } else {
            String::new()
        };
        self.target_langs = langs;
        if self.target_langs.is_empty() {
            self.status = self.i18n.t("status-country-no-language");
            return;
        }
        if !picked.is_empty() {
            self.set_target(picked);
        } else {
            let names: Vec<String> = self
                .target_langs
                .iter()
                .map(|c| self.i18n.display_name(c))
                .collect();
            self.status = self
                .i18n
                .t_arg("status-country-languages", "langs", &names.join(", "));
        }
    }

    /// Set the target language of the active document (then re-run its stack)
    /// or of the next file.
    fn set_target(&mut self, code: String) {
        if self.active_doc().is_some() {
            let idx = self.active;
            if self.docs[idx].settings.target != code {
                self.docs[idx].settings.target = code;
                self.target_changed(idx);
            }
        } else {
            self.idle.target = code;
            self.idle.store_defaults(&mut self.config);
        }
    }

    fn choose_country(&mut self, a3: String) {
        self.temp_country = a3;
        let langs = self.country_languages.languages_for(&self.temp_country);
        if !langs.contains(&self.temp_locale) {
            // The first language of the country that has an interface.
            if let Some(first) = langs
                .iter()
                .find(|l| self.i18n.available_locales().contains(l))
            {
                self.temp_locale = first.clone();
            }
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-about");
        let [w, h] = ABOUT_SIZE;
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t("menu-about"))
            .with_resizable(false);
        let just_opened = self.about_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.about_x, self.config.about_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.about_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            // Fixed size → persist the position only.
            if let Some((x, y, _, _)) = read_geometry(vctx) {
                self.config.about_x = x;
                self.config.about_y = y;
            }
            let mut close = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.heading(crate::APP_NAME);
                    ui.label(
                        self.i18n
                            .t_arg("app-version", "version", crate::APP_VERSION),
                    );
                    ui.add_space(4.0);
                    ui.label(self.i18n.t("about-tagline"));
                    ui.add_space(10.0);
                    if ui.button(self.i18n.t("btn-ok")).clicked() {
                        close = true;
                    }
                });
            });
            if close
                || vctx
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
            {
                self.show_about = false;
            }
        });
    }

    /// The status-colors window (independent viewport): a legend of the 7
    /// statuses with editable swatches; clicking a swatch opens the color picker.
    fn status_colors_window(&mut self, ctx: &egui::Context) {
        if !self.show_status_colors {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-status-colors");
        let [w, h] = STATUS_COLORS_SIZE;
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t("status-colors-title"))
            .with_resizable(false);
        let just_opened = self.status_colors_needs_center;
        if just_opened {
            let pos = self.open_pos(
                self.config.status_colors_x,
                self.config.status_colors_y,
                w,
                h,
            );
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        if just_opened {
            self.status_colors_fitted = false;
        }
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.status_colors_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            // Fixed size → persist position only.
            if let Some((x, y, _, _)) = read_geometry(vctx) {
                self.config.status_colors_x = x;
                self.config.status_colors_y = y;
            }
            let mut reset = false;
            let mut content_h = 0.0_f32;
            egui::CentralPanel::default().show(vctx, |ui| {
                ui.label(self.i18n.t("status-colors-hint"));
                ui.add_space(6.0);
                // Four aligned columns: swatch · glyph · legend · RGB. Keeping the
                // glyph in its own column makes the legend column start at the same
                // x on every row (a glyph + legend in one label misaligned them).
                egui::Grid::new("status_colors_grid")
                    .num_columns(4)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for s in crate::status::Status::ALL {
                            let (r, g, b) = self.config.status_colors[s.index()];
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(48.0, 20.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                rect,
                                egui::Rounding::same(3.0),
                                egui::Color32::from_rgb(r, g, b),
                            );
                            ui.painter().rect_stroke(
                                rect,
                                egui::Rounding::same(3.0),
                                egui::Stroke::new(1.0_f32, egui::Color32::GRAY),
                            );
                            if resp
                                .on_hover_text(self.i18n.t("status-colors-click"))
                                .clicked()
                            {
                                self.color_picker.open_for(
                                    super::color_picker::PickerTarget::Status(s),
                                    (r, g, b),
                                );
                            }
                            ui.label(s.glyph());
                            ui.label(self.i18n.t(s.legend_key()));
                            ui.label(format!("{r}, {g}, {b}"));
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                if ui.button(self.i18n.t("status-colors-reset")).clicked() {
                    reset = true;
                }
                content_h = ui.min_rect().height();
            });
            // Fit the window height to its content once (independent of font size).
            if !self.status_colors_fitted && content_h > 1.0 {
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    w,
                    content_h + 16.0,
                )));
                self.status_colors_fitted = true;
            }
            if reset {
                for s in crate::status::Status::ALL {
                    self.config.status_colors[s.index()] = s.default_rgb();
                }
                let _ = self.config.save();
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.show_status_colors = false;
            }
        });
    }

    /// The color picker (independent viewport, fixed size, position persisted).
    fn color_picker_window(&mut self, ctx: &egui::Context) {
        if !self.color_picker.is_open() {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-color-picker");
        let [w, h] = PICKER_SIZE;
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t("picker-title"))
            .with_resizable(false);
        let just_opened = self.color_picker.needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.picker_x, self.config.picker_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.color_picker.needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if let Some((x, y, _, _)) = read_geometry(vctx) {
                self.config.picker_x = x;
                self.config.picker_y = y;
            }
            let mut committed = None;
            let mut content_h = 0.0_f32;
            egui::CentralPanel::default().show(vctx, |ui| {
                committed = self.color_picker.ui(ui, &self.i18n);
                content_h = ui.min_rect().height();
            });
            // Fit the window height to the active tab's content. The two tabs
            // differ in height, so re-fit whenever it changes (grow or shrink),
            // guarded by a 1px threshold to avoid resize churn.
            if content_h > 1.0 {
                let desired = content_h + 16.0;
                let cur_h = read_geometry(vctx).map(|(_, _, _, hh)| hh).unwrap_or(h);
                if (desired - cur_h).abs() > 1.0 {
                    vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        w, desired,
                    )));
                }
            }
            if let Some((target, rgb)) = committed {
                match target {
                    super::color_picker::PickerTarget::Status(status) => {
                        self.config.status_colors[status.index()] = rgb;
                    }
                    super::color_picker::PickerTarget::DictStripe => {
                        self.config.dictview_stripe = rgb;
                        self.config.dictview_stripe_custom = true;
                    }
                }
                let _ = self.config.save();
            }
            if !self.color_picker.is_open()
                || vctx
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
            {
                self.color_picker.close();
            }
        });
    }

    // --------------------------- spell checker -----------------------------

    /// Open the spell-check window: (re)load the dictionary for the target
    /// language if needed, then scan the active document.
    fn open_spell(&mut self) {
        self.show_spell = true;
        self.spell_needs_center = true;
        self.ensure_spell_loaded();
        self.run_spell_scan();
    }

    /// Load the checker for the current target language when it changed (or was
    /// never loaded). A missing dictionary leaves `self.spell` as `None`.
    fn ensure_spell_loaded(&mut self) {
        if self.spell_lang != self.active_settings().target {
            self.spell_lang = self.active_settings().target.clone();
            self.spell = crate::spell::SpellChecker::load_for_language(&self.spell_lang);
        }
    }

    /// Force a fresh reload attempt (e.g. after the user dropped new dictionary
    /// files), then rescan.
    fn reload_spell(&mut self) {
        self.spell_lang.clear();
        self.ensure_spell_loaded();
        self.run_spell_scan();
    }

    /// Scan the active document's translations, filling `spell_issues` with one
    /// entry per distinct misspelled word per line (with cached suggestions).
    fn run_spell_scan(&mut self) {
        self.spell_issues.clear();
        if self.spell.is_none() {
            self.spell_status = self.i18n.t("spell-no-dict");
            return;
        }
        if self.docs.get(self.active).is_none() {
            self.spell_status = self.i18n.t("status-no-doc");
            return;
        }
        let checker = self.spell.as_ref().unwrap();
        let mut count = 0usize;
        if let Some(doc) = self.docs.get(self.active) {
            for (i, e) in doc.tm.entries.iter().enumerate() {
                if e.trans.is_empty() {
                    continue;
                }
                let mut seen = std::collections::HashSet::new();
                for m in checker.check_text(&e.trans) {
                    if seen.insert(m.word.to_lowercase()) {
                        let suggestions = checker.suggest(&m.word);
                        self.spell_issues.push(SpellIssue {
                            entry: i,
                            word: m.word,
                            suggestions,
                        });
                        count += 1;
                    }
                }
            }
        }
        self.spell_status = if count == 0 {
            self.i18n.t("spell-clean")
        } else {
            self.i18n.t_arg("spell-issues", "n", &count.to_string())
        };
    }

    /// Apply a spell action requested from a window row, then rescan.
    fn apply_spell_action(&mut self, act: SpellAction) {
        match act {
            SpellAction::Fix(idx, to) => {
                if let Some(issue) = self.spell_issues.get(idx) {
                    let (entry, from) = (issue.entry, issue.word.clone());
                    if let Some(doc) = self.docs.get_mut(self.active)
                        && entry < doc.tm.entries.len()
                    {
                        let before = vec![(entry, EntryState::capture(&doc.tm.entries[entry]))];
                        replace_word(&mut doc.tm.entries[entry].trans, &from, &to);
                        if doc.tm.entries[entry].trans != before[0].1.trans {
                            doc.tm.entries[entry].prov = crate::provenance::REPLACE.to_string();
                        }
                        doc.undo
                            .push(op_from_before(OpKind::Edit, &before, &mut doc.tm.entries));
                    }
                }
            }
            SpellAction::Ignore(idx) => {
                if let Some(w) = self.spell_issues.get(idx).map(|i| i.word.clone())
                    && let Some(c) = self.spell.as_mut()
                {
                    c.ignore(&w);
                }
            }
            SpellAction::Add(idx) => {
                if let Some(w) = self.spell_issues.get(idx).map(|i| i.word.clone())
                    && let Some(c) = self.spell.as_mut()
                {
                    c.add_word(&w);
                }
            }
        }
        self.run_spell_scan();
    }

    /// The spell-check window (independent, resizable viewport; geometry saved).
    fn spell_window(&mut self, ctx: &egui::Context) {
        if !self.show_spell {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-spell");
        let (w, h) = (self.config.spell_w, self.config.spell_h);
        let mut builder = egui::ViewportBuilder::default().with_title(self.i18n.t("spell-title"));
        let just_opened = self.spell_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.spell_x, self.config.spell_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.spell_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let mut action: Option<SpellAction> = None;
            let mut do_recheck = false;
            let mut do_reload = false;
            egui::CentralPanel::default().show(vctx, |ui| {
                self.spell_body(ui, &mut action, &mut do_recheck, &mut do_reload);
            });
            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.spell_x = x;
                self.config.spell_y = y;
                self.config.spell_w = w;
                self.config.spell_h = h;
            }
            if let Some(act) = action {
                self.apply_spell_action(act);
            } else if do_recheck {
                self.ensure_spell_loaded();
                self.run_spell_scan();
            } else if do_reload {
                self.reload_spell();
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.show_spell = false;
            }
        });
    }

    /// Render the spell window's contents. Only reads `self`; requested actions
    /// are returned through the out-parameters and applied by the caller.
    fn spell_body(
        &self,
        ui: &mut egui::Ui,
        action: &mut Option<SpellAction>,
        do_recheck: &mut bool,
        do_reload: &mut bool,
    ) {
        ui.horizontal(|ui| match &self.spell {
            Some(c) => {
                ui.label(self.i18n.t("spell-dict"));
                ui.strong(c.code());
                if ui.button(self.i18n.t("spell-recheck")).clicked() {
                    *do_recheck = true;
                }
            }
            None => {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 60),
                    self.i18n.t("spell-none"),
                );
                if ui.button(self.i18n.t("spell-reload")).clicked() {
                    *do_reload = true;
                }
            }
        });

        // No dictionary → explain where to drop the .aff/.dic files and stop.
        if self.spell.is_none() {
            ui.add_space(6.0);
            let dir = crate::spell::dictionaries_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "assets/dictionaries".to_string());
            ui.label(self.i18n.t_arg("spell-none-hint", "dir", &dir));
            return;
        }

        ui.separator();
        ui.label(&self.spell_status);
        ui.add_space(4.0);
        if self.spell_issues.is_empty() {
            return;
        }

        let entries = self.docs.get(self.active).map(|d| &d.tm.entries);
        let ignore_lbl = self.i18n.t("spell-ignore");
        let add_lbl = self.i18n.t("spell-add");
        let row_h = 26.0;
        let n = self.spell_issues.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, n, |ui, range| {
                for idx in range {
                    let issue = &self.spell_issues[idx];
                    ui.horizontal(|ui| {
                        // Locator: entry number + source snippet (fixed width).
                        let loc = entries
                            .and_then(|e| e.get(issue.entry))
                            .map(|e| {
                                let s = e.source.replace('\n', " ");
                                let s: String = if s.chars().count() > 26 {
                                    s.chars().take(26).collect::<String>() + "…"
                                } else {
                                    s
                                };
                                format!("#{}  {s}", issue.entry + 1)
                            })
                            .unwrap_or_else(|| format!("#{}", issue.entry + 1));
                        ui.allocate_ui_with_layout(
                            egui::vec2(190.0, row_h),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(loc).weak())
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                            },
                        );
                        ui.colored_label(egui::Color32::from_rgb(210, 90, 90), &issue.word);
                        ui.separator();
                        // Suggestions (cached), then Ignore / Add.
                        for s in &issue.suggestions {
                            if ui.small_button(s).clicked() {
                                *action = Some(SpellAction::Fix(idx, s.clone()));
                            }
                        }
                        if ui.button(&ignore_lbl).clicked() {
                            *action = Some(SpellAction::Ignore(idx));
                        }
                        if ui.button(&add_lbl).clicked() {
                            *action = Some(SpellAction::Add(idx));
                        }
                    });
                }
            });
    }

    // --------------------------- auto-translation --------------------------

    /// Games offered in the toolbar "Jeu" selector: the detected/registered games
    /// (so a game can be chosen to create its FIRST dictionary) plus any
    /// `assets/bases/jeux/<name>` folder that already holds a dictionary (covers
    /// manually provided dictionaries for a game that isn't currently detected).
    fn selectable_games(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            self.config.games.keys().cloned().collect();
        for f in self.game_dict_folders() {
            set.insert(f);
        }
        set.into_iter().collect()
    }

    /// Sub-folder names under `assets/bases/jeux` that actually hold at least one
    /// `.ximdb`.
    fn game_dict_folders(&self) -> Vec<String> {
        let jeux = self.config.game_base_dir();
        let mut out: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&jeux) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let has_dict = std::fs::read_dir(&p)
                    .map(|r| {
                        r.flatten().any(|f| {
                            f.path()
                                .extension()
                                .and_then(|x| x.to_str())
                                .is_some_and(|x| x.eq_ignore_ascii_case("ximdb"))
                        })
                    })
                    .unwrap_or(false);
                if has_dict && let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                    out.push(n.to_string());
                }
            }
        }
        out.sort();
        out
    }

    /// Report an outcome: the status bar line, doubled by a toast when they
    /// are enabled (F-05).
    pub(crate) fn notify(&mut self, level: crate::ui::toasts::ToastLevel, msg: impl Into<String>) {
        let msg = self.i18n.localize(&msg.into());
        // The journal first (R-E01-03), at the level of the function called.
        let jl = match level {
            crate::ui::toasts::ToastLevel::Info | crate::ui::toasts::ToastLevel::Success => {
                crate::journal::Level::Info
            }
            crate::ui::toasts::ToastLevel::Warning => crate::journal::Level::Warn,
            crate::ui::toasts::ToastLevel::Error => crate::journal::Level::Error,
        };
        self.journal_add(jl, crate::journal::Kind::Message, &msg);
        self.status = msg.clone();
        if self.config.toasts {
            self.toasts.push(level, msg, self.frame_time);
        }
    }

    pub(crate) fn notify_ok(&mut self, msg: impl Into<String>) {
        self.notify(crate::ui::toasts::ToastLevel::Success, msg);
    }

    fn notify_warn(&mut self, msg: impl Into<String>) {
        self.notify(crate::ui::toasts::ToastLevel::Warning, msg);
    }

    pub(crate) fn notify_err(&mut self, msg: impl Into<String>) {
        self.notify(crate::ui::toasts::ToastLevel::Error, msg);
    }

    /// Raise a translated alert message (also mirrored to the status bar).
    fn alert_msg(&mut self, key: &str) {
        let msg = self.i18n.t(key);
        self.status = msg.clone();
        self.alert = Some(msg);
    }

    /// A blocking alert box (in-app modal, portable — no native dialog). Shows the
    /// pending [`Self::alert`] message with a single OK button. Used to make
    /// plugin-pair failures (missing files, name mismatch, no matching entry)
    /// clearly visible instead of only writing to the status bar.
    fn alert_window(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.alert.as_deref().map(|m| self.i18n.localize(m)) else {
            self.alert_logged = None;
            return;
        };
        if self.alert_logged.as_deref() != Some(msg.as_str()) {
            self.alert_logged = Some(msg.clone());
            self.journal_add(
                crate::journal::Level::Warn,
                crate::journal::Kind::Message,
                &msg,
            );
        }
        let title = self.i18n.t("alert-title");
        let ok = self.i18n.t("btn-ok");
        let mut close = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(msg);
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    if ui.button(ok.as_str()).clicked() {
                        close = true;
                    }
                });
            });
        if close
            || ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
        {
            self.alert = None;
        }
    }

    /// The "Is this really a dictionary for <game>?" confirmation (in-app modal).
    /// Shown while a [`PendingSave`] is queued; Valider writes it, Annuler drops
    /// it (no save). Kept in-app (no native task dialog) for portability.
    fn dict_confirm_window(&mut self, ctx: &egui::Context) {
        if self.pending_save.is_none() {
            return;
        }
        let active = self.active_settings();
        let game = active.game.trim();
        let label = {
            let g = crate::games::Game::from_id(game);
            if g == crate::games::Game::Unknown {
                game.to_string()
            } else {
                g.display_name().to_string()
            }
        };
        let title = self.i18n.t("dict-confirm-title");
        let msg = self.i18n.t_arg("dict-confirm-game", "game", &label);
        let valider = self.i18n.t("btn-validate");
        let annuler = self.i18n.t("btn-cancel");

        let mut do_save = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(msg);
                ui.add_space(12.0);
                let (p, c) = confirm_buttons(ui, &valider, &annuler);
                do_save = p;
                cancel = c;
            });

        if cancel {
            self.pending_save = None;
        } else if do_save && let Some(p) = self.pending_save.take() {
            self.perform_pending_save(p);
        }
    }

    /// Perform a confirmed [`PendingSave`] and report the result in the status bar.
    fn perform_pending_save(&mut self, p: PendingSave) {
        // Mod dictionaries get timestamped copies (B-11, R-B11-01); game
        // dictionaries are regenerated freely.
        let copies = |out: &std::path::Path, cfg: &AppConfig| {
            if out.starts_with(cfg.mod_base_dir()) {
                cfg.backup_copies
            } else {
                0
            }
        };
        match p {
            PendingSave::Rows { out, lang, rows } => {
                if let Some(parent) = out.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let copies = copies(&out, &self.config);
                self.status = match crate::basedb::write_ximdb_rotating(rows, &lang, &out, copies) {
                    Ok(n) => self.i18n.dash(
                        &self.i18n.t("dict-done"),
                        &self.i18n.t_args(
                            "dict-result-entries",
                            &[("n", &n.to_string()), ("path", &out.display().to_string())],
                        ),
                    ),
                    Err(e) => self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()),
                };
            }
            PendingSave::Compile { out, lang, files } => {
                if let Some(parent) = out.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let copies = copies(&out, &self.config);
                let _ = crate::backup::rotate_backup(&out, copies);
                let result = crate::basedb::compile_files(&files, &lang, &out);
                if result.is_ok() {
                    let _ = crate::backup::purge_backups(&out, copies);
                }
                self.status = match result {
                    Ok((read, n)) => self.i18n.dash(
                        &self.i18n.t("dict-done"),
                        &self.i18n.t_args(
                            "dict-result-files",
                            &[
                                ("files", &read.to_string()),
                                ("n", &n.to_string()),
                                ("path", &out.display().to_string()),
                            ],
                        ),
                    ),
                    Err(e) => self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()),
                };
            }
        }
    }

    /// Open the read-only dictionary viewer window (no file is loaded yet — the
    /// user opens one via the "Open…" button or by dropping a `.ximdb` file).
    fn open_dict_view_window(&mut self) {
        self.show_dict_view = true;
        self.dict_view_needs_center = true;
    }

    /// Ask for a `.ximdb` file and load it into the already-open viewer.
    fn pick_dict_view_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-ximdb"), &["ximdb"])
            .set_directory(self.bases_root())
            .pick_file()
        {
            self.load_dict_view_path(&path);
        }
    }

    /// Load a `.ximdb` file into the viewer (used by the Open button and by
    /// drag-and-drop). Errors are shown in an alert box.
    fn load_dict_view_path(&mut self, path: &std::path::Path) {
        match crate::basedb::ximdb_to_tm(path) {
            Ok(tm) => {
                let all: Vec<(String, String)> = tm
                    .entries
                    .into_iter()
                    .map(|e| (e.source, e.trans))
                    .collect();
                let title = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("dictionnaire")
                    .to_string();
                self.dict_view = Some(DictView {
                    title,
                    all,
                    filter: String::new(),
                    filtered: Vec::new(),
                    last_filter: None,
                    heights: Vec::new(),
                    heights_w: -1.0,
                });
            }
            Err(e) => {
                let msg = self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string());
                self.status = msg.clone();
                self.alert = Some(msg);
            }
        }
    }

    /// The read-only dictionary viewer window: an independent viewport with, in
    /// its header, the filter, the counter, an "Open…" button and a "Close"
    /// button (which unloads the current file). The window itself is closed by its
    /// title-bar cross or by Escape. A `.ximdb` file can also be dropped onto it.
    fn dict_view_window(&mut self, ctx: &egui::Context) {
        if !self.show_dict_view {
            return;
        }
        // Recompute the filtered index set when the filter text changed.
        if let Some(v) = self.dict_view.as_mut()
            && v.last_filter.as_deref() != Some(v.filter.as_str())
        {
            let f = v.filter.to_lowercase();
            v.filtered = if f.is_empty() {
                (0..v.all.len()).collect()
            } else {
                v.all
                    .iter()
                    .enumerate()
                    .filter(|(_, (s, t))| {
                        s.to_lowercase().contains(&f) || t.to_lowercase().contains(&f)
                    })
                    .map(|(i, _)| i)
                    .collect()
            };
            v.last_filter = Some(v.filter.clone());
            v.heights.clear(); // force a height recompute for the new row set
        }

        let base_title = self.i18n.t("dict-view-title");
        let title = match &self.dict_view {
            Some(v) => self.i18n.dash(&base_title, &v.title),
            None => base_title,
        };
        let lbl_filter = self.i18n.t("dict-view-filter");
        let lbl_src = self.i18n.t("dict-view-source");
        let lbl_trans = self.i18n.t("dict-view-trans");
        let lbl_close = self.i18n.t("btn-close");
        let lbl_open = self.i18n.t("dict-view-open");
        let lbl_empty = self.i18n.t("dict-view-empty");
        let lbl_drop = self.i18n.t("dict-view-drop");

        // Independent OS window (viewport). Geometry persisted in Config.ini.
        let id = egui::ViewportId::from_hash_of("ximod-dict-view");
        let (w, h) = (self.config.dictview_w, self.config.dictview_h);
        let mut builder = egui::ViewportBuilder::default().with_title(title);
        let just_opened = self.dict_view_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.dictview_x, self.config.dictview_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }

        let mut close_window = false;
        let mut do_open = false;
        let mut do_close_file = false;
        let mut dropped: Option<PathBuf> = None;

        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.dict_view_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                use egui_extras::{Column, TableBuilder};

                // --- Header: filter · counter · Open… · Close ---
                ui.horizontal(|ui| {
                    ui.label(lbl_filter.as_str());
                    let (shown, total) = match self.dict_view.as_mut() {
                        Some(v) => {
                            ui.add(egui::TextEdit::singleline(&mut v.filter).desired_width(240.0));
                            (v.filtered.len(), v.all.len())
                        }
                        None => {
                            let mut empty = String::new();
                            ui.add_enabled(
                                false,
                                egui::TextEdit::singleline(&mut empty).desired_width(240.0),
                            );
                            (0usize, 0usize)
                        }
                    };
                    ui.label(self.i18n.t_args(
                        "count-of",
                        &[("n", &shown.to_string()), ("total", &total.to_string())],
                    ));
                    ui.separator();
                    if ui.button(lbl_open.as_str()).clicked() {
                        do_open = true;
                    }
                    ui.add_enabled_ui(self.dict_view.is_some(), |ui| {
                        if ui.button(lbl_close.as_str()).clicked() {
                            do_close_file = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();

                if let Some(v) = self.dict_view.as_mut() {
                    // Two equal columns that grow/shrink with the window. Long text
                    // wraps onto several lines (word boundaries) instead of being
                    // truncated with a tooltip. Rows are zebra-striped for legibility.
                    let gap = ui.spacing().item_spacing.x;
                    let col_w = ((ui.available_width() - gap - 28.0) / 2.0).max(80.0);
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let header_h = font.size + 10.0;
                    let pad = 8.0;
                    // Zebra tint: the user's chosen color (Options ▸ stripe color)
                    // or, by default, the theme's faint background.
                    let tint = if self.config.dictview_stripe_custom {
                        let (r, g, b) = self.config.dictview_stripe;
                        egui::Color32::from_rgb(r, g, b)
                    } else {
                        ui.visuals().faint_bg_color
                    };

                    // Recompute the row heights only when the filtered set or the
                    // column width changed; otherwise reuse the cache so plain
                    // scrolling never re-lays-out the rows (the cause of the lag).
                    if v.heights.len() != v.filtered.len() || (v.heights_w - col_w).abs() > 0.5 {
                        let all = &v.all;
                        v.heights = v
                            .filtered
                            .iter()
                            .map(|&i| {
                                let (s, t) = &all[i];
                                let hs = crate::ui::grid::wrapped_height(ui, s, col_w, &font);
                                let ht = crate::ui::grid::wrapped_height(ui, t, col_w, &font);
                                hs.max(ht).max(font.size) + pad
                            })
                            .collect();
                        v.heights_w = col_w;
                    }

                    let all = &v.all;
                    let filtered = &v.filtered;
                    let heights = &v.heights;

                    TableBuilder::new(ui)
                        .striped(false) // painted manually below for a clear alternation
                        .cell_layout(egui::Layout::left_to_right(egui::Align::TOP))
                        .column(Column::exact(col_w))
                        .column(Column::exact(col_w))
                        .header(header_h, |mut header| {
                            header.col(|ui| {
                                ui.strong(lbl_src.as_str());
                            });
                            header.col(|ui| {
                                ui.strong(lbl_trans.as_str());
                            });
                        })
                        .body(|body| {
                            body.heterogeneous_rows(heights.iter().copied(), |mut row| {
                                let ri = row.index();
                                let (s, t) = &all[filtered[ri]];
                                let bg = if ri % 2 == 0 {
                                    tint
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                let paint = |ui: &mut egui::Ui, text: &str| {
                                    if bg != egui::Color32::TRANSPARENT {
                                        ui.painter().rect_filled(ui.max_rect(), 0.0, bg);
                                    }
                                    ui.add(
                                        egui::Label::new(text).wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                };
                                row.col(|ui| paint(ui, s));
                                row.col(|ui| paint(ui, t));
                            });
                        });
                } else {
                    // Empty state: prompt to open or drop a file.
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(lbl_empty.as_str()).weak());
                    });
                }

                // Drag-and-drop overlay while a file is hovered over the window.
                if vctx.input(|i| !i.raw.hovered_files.is_empty()) {
                    let rect = ui.max_rect();
                    let painter = ui.painter();
                    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(160));
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        lbl_drop.as_str(),
                        egui::FontId::proportional(18.0),
                        egui::Color32::WHITE,
                    );
                }
            });

            // Accept a dropped file (first one with a path).
            let files = vctx.input(|i| i.raw.dropped_files.clone());
            if let Some(p) = files.into_iter().find_map(|f| f.path) {
                dropped = Some(p);
            }

            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.dictview_x = x;
                self.config.dictview_y = y;
                self.config.dictview_w = w;
                self.config.dictview_h = h;
            }

            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                close_window = true;
            }
        });

        // Apply header/DnD actions after the viewport closure (avoids self aliasing).
        if do_close_file {
            self.dict_view = None;
        }
        if let Some(p) = dropped {
            self.load_dict_view_path(&p);
        }
        if do_open {
            self.pick_dict_view_file();
        }
        if close_window {
            self.show_dict_view = false;
        }
    }

    /// Drop the cached indexes and re-run the active document's dictionary
    /// stack from disk.
    fn reload_bases(&mut self) {
        self.file_dbs.invalidate(None);
        if self.docs.is_empty() {
            self.status = self.i18n.t("status-bases-reloaded");
            return;
        }
        let auto = self.restack_doc(self.active);
        self.status = self.i18n.dash(
            &self.i18n.t("status-bases-reloaded"),
            &self
                .i18n
                .t_arg("status-auto-translated", "n", &auto.to_string()),
        );
    }

    /// Consolidate the EET (and other) bases of the game folder into a native
    /// XIMOD base (pivot JSON), then rebuild the index.
    fn compile_game_bases(&mut self) {
        let Some((_, lang)) = self.require_pair() else {
            return;
        };
        let dir = self.config.game_base_dir();
        // One `.ximdb` per game (per source file), each tagged with the current
        // target language.
        match crate::basedb::compile_dir_per_file(&dir, &lang) {
            Ok((files, entries)) => {
                self.file_dbs.invalidate(None);
                self.status = self.i18n.colon(
                    &self.i18n.t_arg("status-bases-compiled-lang", "lang", &lang),
                    &self.i18n.t_args(
                        "status-bases-compiled-detail",
                        &[
                            ("files", &files.to_string()),
                            ("entries", &entries.to_string()),
                        ],
                    ),
                );
            }
            Err(e) => self.notify_err(
                self.i18n
                    .colon(&self.i18n.t("status-bases-compile-error"), &e.to_string()),
            ),
        }
    }

    // --------------------------- Dictionnaires menu ------------------------

    fn run_dict_action(&mut self, action: DictAction) {
        // The dictionaries of a file are worked on in its own tab (B-25).
        if self.all_active()
            && matches!(
                action,
                DictAction::ModUpdate | DictAction::SaveModDict | DictAction::ManageDicts
            )
        {
            self.status = self.i18n.t("all-mods-file-only");
            return;
        }
        match action {
            DictAction::ConvUnit(k) => self.dict_convert_unit(k),
            DictAction::ConvBatch(k) => self.dict_convert_batch(k),
            DictAction::FromGame => self.dict_build_dictionary(true),
            DictAction::FromMod => self.dict_build_dictionary(false),
            DictAction::FromStrings => self.dict_build_from_strings(),
            DictAction::FromPlugins => self.show_plugin_pair = true,
            DictAction::FromLocresCsv => self.show_locres_csv = true,
            DictAction::ExportGameStrings => self.dict_export_game_strings(),
            DictAction::DetectGames => self.dict_detect_games(),
            DictAction::AddGameManual => self.dict_add_game_manual(),
            DictAction::ViewDict => self.open_dict_view_window(),
            DictAction::ManageDicts => self.open_dicts_window(),
            DictAction::FromTextPairs => self.dict_from_text_pairs(),
            DictAction::EditPersonal => self.open_dict_editor(None),
            DictAction::EditDict => self.pick_dict_to_edit(),
            DictAction::ModUpdate => self.open_update_window(),
            DictAction::Compare => self.open_compare_window(),
            DictAction::SaveModDict => {
                let idx = self.active;
                self.write_mod_dict(idx, true);
            }
        }
    }

    /// Build a dictionary by pairing two languages' STRINGS files by string id.
    /// The user picks the source-language STRINGS then the target-language ones;
    /// entries with the same (kind, id) become (source → target) rows.
    fn dict_build_from_strings(&mut self) {
        let src = self.active_settings().source.clone();
        let dst = self.active_settings().target.clone();
        let filt = ["strings", "dlstrings", "ilstrings"];
        // Open where the games live (a detected game's parent folder), when known.
        let common = self.any_game_common_dir();
        let mut sfd = rfd::FileDialog::new()
            .set_title(self.i18n.t("dict-pick-src-strings"))
            .add_filter(self.i18n.t("dialog-strings-filter"), &filt);
        if let Some(d) = &common {
            sfd = sfd.set_directory(d);
        }
        let Some(src_files) = sfd.pick_files() else {
            return;
        };
        let mut tfd = rfd::FileDialog::new()
            .set_title(self.i18n.t("dict-pick-tgt-strings"))
            .add_filter(self.i18n.t("dialog-strings-filter"), &filt);
        if let Some(d) = &common {
            tfd = tfd.set_directory(d);
        }
        let Some(tgt_files) = tfd.pick_files() else {
            return;
        };

        // STRINGS carry no game/encoding; UTF-8 (SSE-era) is the common case and
        // the decoder falls back to Latin-1.
        let load = |files: &[PathBuf]| -> std::collections::HashMap<(u8, i32), String> {
            let mut map = std::collections::HashMap::new();
            for p in files {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                let Some(kind) = crate::tm::strings::kind_from_extension(&ext.to_uppercase())
                else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(p) else {
                    continue;
                };
                if let Ok(tm) = crate::tm::strings::load_as_tm(&bytes, kind, true) {
                    for e in tm.entries {
                        map.insert((e.list_index, e.idstexte), e.source);
                    }
                }
            }
            map
        };
        let source_map = load(&src_files);
        let target_map = load(&tgt_files);

        let mut rows = Vec::new();
        for ((li, id), s) in &source_map {
            if let Some(t) = target_map.get(&(*li, *id))
                && !t.is_empty()
                && t != s
            {
                rows.push(crate::basedb::BaseRow {
                    source: s.clone(),
                    trans: t.clone(),
                    ..Default::default()
                });
            }
        }

        // Derive a base name from the first source file (strip the `_<lang>` token).
        let name = src_files
            .first()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| s.rsplit_once('_').map(|(a, _)| a).unwrap_or(s).to_string())
            .unwrap_or_else(|| "jeu".to_string());
        // Dictionary built from the game's own STRINGS → game bases folder.
        self.finish_ximdb_build(rows, &name, &src, &dst, true);
    }

    /// Start detecting installed games on a **background thread**, reporting
    /// progress over a channel; the recap dialog opens when it finishes. Ignored
    /// if a detection is already running.
    fn dict_detect_games(&mut self) {
        if self.game_detect_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let results = crate::games::detect::detect_all_reporting(move |done, total, id| {
                let _ = tx2.send(DetectMsg::Step(done, total, id));
            });
            let _ = tx.send(DetectMsg::Done(results));
        });
        self.game_detect_rx = Some(rx);
        self.detect_progress = Some((0, crate::games::detect::STEP_COUNT, String::new()));
        self.game_detect = None;
    }

    /// Localized label for a detection source id.
    fn detect_step_label(&self, id: &str) -> String {
        match id {
            "steam" => self.i18n.t("games-step-steam"),
            "gog" => self.i18n.t("games-step-gog"),
            "gog-galaxy" => self.i18n.t("games-step-gog-galaxy"),
            "epic" => self.i18n.t("games-step-epic"),
            "xbox" => self.i18n.t("games-step-xbox"),
            "scan" => self.i18n.t("games-step-scan"),
            _ => String::new(),
        }
    }

    /// Poll the detection thread each frame; update progress and, on completion,
    /// open the recap dialog (or report "none").
    fn poll_game_detect(&mut self, ctx: &egui::Context) {
        if self.game_detect_rx.is_none() {
            return;
        }
        let mut steps: Vec<(usize, usize, &'static str)> = Vec::new();
        let mut finished: Option<Vec<crate::games::detect::DetectedGame>> = None;
        if let Some(rx) = &self.game_detect_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    DetectMsg::Step(d, t, id) => steps.push((d, t, id)),
                    DetectMsg::Done(v) => finished = Some(v),
                }
            }
        }
        if let Some((d, t, id)) = steps.last().copied() {
            self.detect_progress = Some((d, t, self.detect_step_label(id)));
        }
        if let Some(v) = finished {
            self.game_detect_rx = None;
            self.detect_progress = None;
            if v.is_empty() {
                self.status = self.i18n.t("games-none");
                self.game_detect = None;
            } else {
                self.status = format!("{} {}", v.len(), self.i18n.t("games-found"));
                self.game_detect = Some(v);
            }
        } else {
            // Keep the UI ticking while the worker runs.
            ctx.request_repaint();
        }
    }

    /// Modal shown while detection runs: a labelled progress bar.
    fn detect_progress_window(&mut self, ctx: &egui::Context) {
        let Some((done, total, ref label)) = self.detect_progress else {
            return;
        };
        let frac = if total > 0 {
            done as f32 / total as f32
        } else {
            0.0
        };
        let title = self.i18n.t("games-detecting");
        let label = label.clone();
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(280.0)
                        .show_percentage(),
                );
                if !label.is_empty() {
                    ui.add_space(4.0);
                    ui.label(label);
                }
                ui.add_space(4.0);
            });
    }

    /// Register a game folder chosen manually (validated by master file), locked
    /// so auto-detection won't overwrite it.
    fn dict_add_game_manual(&mut self) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title(self.i18n.t("games-pick-folder"))
            .pick_folder()
        else {
            return;
        };
        let hits = crate::games::detect::validate_manual(&dir);
        if hits.is_empty() {
            self.status = self.i18n.t("games-unrecognized");
            return;
        }
        let mut n = 0;
        for d in &hits {
            if self.config.set_game(
                d.game.id(),
                &d.root.to_string_lossy(),
                d.source.as_str(),
                true,
            ) {
                n += 1;
            }
        }
        let _ = self.config.save();
        self.status = self.i18n.t_arg("games-saved", "n", &n.to_string());
    }

    /// Recap dialog listing detected games (game · source · path) before writing
    /// them to `Config.ini`.
    fn dict_games_window(&mut self, ctx: &egui::Context) {
        if self.game_detect.is_none() {
            return;
        }
        let title = self.i18n.t("games-title");
        let lbl_save = self.i18n.t("games-save");
        let lbl_cancel = self.i18n.t("dict-conv-cancel");
        // Pre-render the rows (game · source · path) to avoid borrow conflicts.
        let rows: Vec<(String, String, String)> = self
            .game_detect
            .as_ref()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d.game.display_name().to_string(),
                    self.i18n.t(&format!(
                        "detect-source-{}",
                        d.source.as_str().to_lowercase()
                    )),
                    d.root.to_string_lossy().to_string(),
                )
            })
            .collect();
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::Grid::new("games_detect_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (game, source, path) in &rows {
                            ui.label(game);
                            ui.label(source);
                            ui.label(path);
                            ui.end_row();
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(lbl_save.as_str()).clicked() {
                        save = true;
                    }
                    if ui.button(lbl_cancel.as_str()).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.game_detect = None;
        } else if save && let Some(found) = self.game_detect.take() {
            let mut n = 0;
            for d in &found {
                if self.config.set_game(
                    d.game.id(),
                    &d.root.to_string_lossy(),
                    d.source.as_str(),
                    false,
                ) {
                    n += 1;
                }
            }
            let _ = self.config.save();
            self.status = self.i18n.t_arg("games-saved", "n", &n.to_string());
        }
    }

    /// Export a game dictionary (ximdb) as translated STRINGS: pick the ximdb, a
    /// folder of source-language STRINGS to translate, and an output folder. Each
    /// `<base>_<src>.<KIND>` becomes `<base>_<dst>.<KIND>` with the dictionary's
    /// translations applied (ids without a match keep their source text). This is
    /// what xTranslator reads to (re)build its own game cache.
    fn dict_export_game_strings(&mut self) {
        let Some(dictp) = rfd::FileDialog::new()
            .set_title(self.i18n.t("dict-xt-pick-ximdb"))
            .add_filter(self.i18n.t("dialog-ximdb"), &["ximdb"])
            .set_directory(self.bases_root())
            .pick_file()
        else {
            return;
        };
        // Pre-point the source-STRINGS picker at the detected game named in the
        // ximdb file (e.g. SkyrimSE_ENG-FRA.ximdb → the SkyrimSE Data/Strings).
        let stem = dictp.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut fd = rfd::FileDialog::new().set_title(self.i18n.t("dict-xt-pick-src-strings-dir"));
        if let Some(d) = self.game_strings_dir(parse_dict_name(stem).game) {
            fd = fd.set_directory(d);
        }
        let Some(indir) = fd.pick_folder() else {
            return;
        };
        let Some(outdir) = rfd::FileDialog::new()
            .set_title(self.i18n.t("dict-xt-pick-out-dir"))
            .pick_folder()
        else {
            return;
        };

        let Ok(tm) = crate::basedb::ximdb_to_tm(&dictp) else {
            self.status = self.i18n.t("dict-error");
            return;
        };
        let mut dict: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for e in &tm.entries {
            if !e.trans.is_empty() {
                dict.entry(e.source.clone())
                    .or_insert_with(|| e.trans.clone());
            }
        }
        if dict.is_empty() {
            self.alert_msg("dict-empty");
            return;
        }

        let src = self.active_settings().source.clone();
        let dst = self.active_settings().target.clone();
        let dst_name = crate::lang::bethesda_name(&dst).unwrap_or(&dst).to_string();
        // Skyrim LE: the STRINGS are in the pages of their languages (R-D09-06).
        let page_game = app_encoding::page_game(&self.active_settings());
        let (page_in, page_out) = if page_game {
            (
                Some(Codec {
                    page: self.pages().for_language(&src),
                    strict: false,
                }),
                Some(self.pages().for_language(&dst)),
            )
        } else {
            (None, None)
        };

        let (mut files_done, mut applied) = (0usize, 0usize);
        if let Ok(rd) = std::fs::read_dir(&indir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some((base, lang, kind)) = split_strings_name(name) else {
                    continue;
                };
                if !crate::lang::token_matches(&lang, &src) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(mut stm) = crate::tm::encoding::with_input(page_in, || {
                    crate::tm::strings::load_as_tm(&bytes, kind, true)
                }) else {
                    continue;
                };
                for e in &mut stm.entries {
                    if let Some(t) = dict.get(&e.source) {
                        e.trans = t.clone();
                        e.set_translated_flags(true);
                        applied += 1;
                    }
                }
                let out_bytes = crate::tm::encoding::with_output(page_out, || {
                    crate::tm::strings::write_from_tm(&stm, kind, true)
                });
                let out = outdir.join(format!("{base}_{dst_name}.{}", kind.extension()));
                if std::fs::write(&out, out_bytes).is_ok() {
                    files_done += 1;
                }
            }
        }
        self.status = if files_done == 0 {
            self.i18n.t("dict-xt-no-src-strings")
        } else {
            self.i18n.dash(
                &self.i18n.t("dict-done"),
                &self.i18n.t_args(
                    "dict-result-files",
                    &[
                        ("files", &files_done.to_string()),
                        ("n", &applied.to_string()),
                        ("path", &outdir.display().to_string()),
                    ],
                ),
            )
        };
    }

    /// Build a dictionary from a pair of plugins (original + translated): read the
    /// source plugin in the source language and the translated plugin in the target
    /// language, then pair records by (GRUP, Champ, FormID, EDID, index).
    fn build_plugin_pair(
        &mut self,
        src_plugin: &std::path::Path,
        tgt_plugin: &std::path::Path,
        to_game: bool,
    ) {
        let Some((src, dst)) = self.require_pair() else {
            return;
        };

        let src_tm = match Self::plugin_text_tm(src_plugin, &src) {
            Ok(tm) => tm,
            Err(e) => {
                let msg = self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string());
                self.status = msg.clone();
                self.alert = Some(msg);
                return;
            }
        };
        let tgt_tm = match Self::plugin_text_tm(tgt_plugin, &dst) {
            Ok(tm) => tm,
            Err(e) => {
                let msg = self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string());
                self.status = msg.clone();
                self.alert = Some(msg);
                return;
            }
        };

        // Index the translated plugin's text by record key.
        let mut tgt: std::collections::HashMap<(String, String, u32, String, i32), String> =
            std::collections::HashMap::new();
        for e in &tgt_tm.entries {
            tgt.insert(
                (
                    e.rec_sig.clone(),
                    e.field_sig.clone(),
                    e.formid,
                    e.edid.to_lowercase(),
                    e.index,
                ),
                e.source.clone(),
            );
        }
        let mut rows = Vec::new();
        for e in &src_tm.entries {
            let key = (
                e.rec_sig.clone(),
                e.field_sig.clone(),
                e.formid,
                e.edid.to_lowercase(),
                e.index,
            );
            if let Some(t) = tgt.get(&key)
                && !t.is_empty()
                && *t != e.source
                && !e.source.is_empty()
            {
                rows.push(crate::basedb::BaseRow {
                    rec: e.rec_sig.clone(),
                    field: e.field_sig.clone(),
                    formid: e.formid,
                    edid: e.edid.clone(),
                    source: e.source.clone(),
                    trans: t.clone(),
                    status: None,
                    ..Default::default()
                });
            }
        }
        if rows.is_empty() {
            self.alert_msg("dict-empty");
            return;
        }
        // The dictionary is named after the SOURCE PLUGIN (so a DLC like
        // Anchorage.esm → Anchorage_<SRC>-<DST>.ximdb, and the master like
        // Fallout3.esm → Fallout3_…). The sub-folder is the game chosen in the
        // toolbar "Jeu" selector: assets/bases/jeux/<Jeu>/ (Destination = Jeu) or
        // assets/bases/mods/<Jeu>/ (Destination = Mod). The write is deferred until
        // the user confirms the game.
        let sanitize = |s: &str| -> String {
            s.chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect()
        };
        let game_sub = sanitize(self.active_settings().game.trim());
        let stem = src_plugin
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mod");
        let base = sanitize(stem);
        let dir = self.dict_base_dir(to_game).join(&game_sub);
        let out = dir.join(format!(
            "{base}_{}-{}.ximdb",
            src.to_uppercase(),
            dst.to_uppercase()
        ));
        self.pending_save = Some(PendingSave::Rows {
            out,
            lang: dst.clone(),
            rows,
        });
    }

    /// The "dictionary from a plugin pair" dialog: two path fields (typed or via
    /// Browse), languages taken from the Settings Source/Target dropdowns. The
    /// paths persist for the whole session; Browse opens where the field points.
    fn plugin_pair_window(&mut self, ctx: &egui::Context) {
        if !self.show_plugin_pair {
            return;
        }
        let title = self.i18n.t("dict-from-plugins");
        let l_src = self.i18n.t("dict-plugin-src-label");
        let l_tgt = self.i18n.t("dict-plugin-tgt-label");
        let l_browse = self.i18n.t("settings-browse");
        let l_create = self.i18n.t("dict-plugin-create");
        let l_close = self.i18n.t("btn-close");
        let l_dest = self.i18n.t("dict-conv-kind");
        let l_jeux = self.i18n.t("dict-conv-jeux");
        let l_mods = self.i18n.t("dict-conv-mods");
        let langs = self.i18n.t_args(
            "dict-plugin-langs-pair",
            &[
                ("src", &self.active_settings().source.to_uppercase()),
                ("dst", &self.active_settings().target.to_uppercase()),
            ],
        );
        let filt = crate::games::registry().all_plugin_extensions();

        let mut do_build = false;
        let mut close = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(langs);
                ui.add_space(8.0);

                ui.label(l_src);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.pp_src_path).desired_width(400.0));
                    if ui.button(l_browse.as_str()).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter(
                                "Plugin",
                                &filt.iter().map(String::as_str).collect::<Vec<_>>(),
                            )
                            .set_directory(browse_start_for_file(&self.pp_src_path))
                            .pick_file()
                    {
                        self.pp_src_path = p.display().to_string();
                    }
                });
                ui.add_space(8.0);

                ui.label(l_tgt);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.pp_tgt_path).desired_width(400.0));
                    if ui.button(l_browse.as_str()).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter(
                                "Plugin",
                                &filt.iter().map(String::as_str).collect::<Vec<_>>(),
                            )
                            .set_directory(browse_start_for_file(&self.pp_tgt_path))
                            .pick_file()
                    {
                        self.pp_tgt_path = p.display().to_string();
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(l_dest.as_str());
                    ui.radio_value(&mut self.pp_to_game, true, l_jeux.as_str());
                    ui.radio_value(&mut self.pp_to_game, false, l_mods.as_str());
                });
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button(l_create.as_str()).clicked() {
                        do_build = true;
                    }
                    if ui.button(l_close.as_str()).clicked() {
                        close = true;
                    }
                });
            });

        // Ignore a build request while a previous save is still awaiting the
        // game-confirmation dialog, so it cannot be silently overwritten.
        if do_build && self.pending_save.is_none() {
            let s = self.pp_src_path.trim().to_string();
            let t = self.pp_tgt_path.trim().to_string();
            let (sp, tp) = (std::path::Path::new(&s), std::path::Path::new(&t));
            if s.is_empty() || t.is_empty() || !sp.is_file() || !tp.is_file() {
                self.alert_msg("dict-plugin-missing");
            } else if !plugin_names_match(sp, tp) {
                // The two plugins must be the same file (a translated mod may only
                // differ by a trailing language code). Otherwise it's a mistake.
                self.alert_msg("dict-plugin-name-mismatch");
            } else {
                // Queue the save (the game-confirmation dialog runs next). The
                // dialog stays open on purpose so several dictionaries can be
                // built in a row; it is closed only by the "Fermer" button. The
                // fields keep their contents for the whole session.
                self.build_plugin_pair(sp, tp, self.pp_to_game);
            }
        }
        if close {
            self.show_plugin_pair = false;
        }
    }

    /// Build a dictionary from a pair of UEExtractor locres CSVs (source and
    /// target language), joined by key. Used mainly for Oblivion Remastered, whose
    /// base-game text lives in UE5 locres files rather than in STRINGS or the ESM.
    /// The dictionary is named after the selected game (a game base such as
    /// `OblivionRD_ENG-FRA.ximdb`); the write is deferred to the confirmation dialog.
    fn build_locres_csv_pair(
        &mut self,
        src_csv: &std::path::Path,
        tgt_csv: &std::path::Path,
        to_game: bool,
    ) {
        let Some((src, dst)) = self.require_pair() else {
            return;
        };

        let pairs = match crate::locres::pair_files(src_csv, tgt_csv) {
            Ok(p) => p,
            Err(e) => {
                let msg = self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string());
                self.status = msg.clone();
                self.alert = Some(msg);
                return;
            }
        };
        if pairs.is_empty() {
            self.alert_msg("dict-empty");
            return;
        }
        // locres entries carry no plugin record context, so the lookup fields are
        // left empty; only source → translation matter for the game base.
        let rows: Vec<crate::basedb::BaseRow> = pairs
            .into_iter()
            .map(|(source, trans)| crate::basedb::BaseRow {
                source,
                trans,
                ..Default::default()
            })
            .collect();

        let sanitize = |s: &str| -> String {
            s.chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect()
        };
        let game_sub = sanitize(self.active_settings().game.trim());
        let base = if game_sub.is_empty() {
            "game".to_string()
        } else {
            game_sub.clone()
        };
        let dir = self.dict_base_dir(to_game).join(&game_sub);
        let out = dir.join(format!(
            "{base}_{}-{}.ximdb",
            src.to_uppercase(),
            dst.to_uppercase()
        ));
        self.pending_save = Some(PendingSave::Rows {
            out,
            lang: dst,
            rows,
        });
    }

    /// The "dictionary from a pair of locres CSVs" dialog: two path fields (typed
    /// or via Browse), languages taken from the main-window Source/Target
    /// selectors, destination Game/Mod. Stays open after each save (paths persist
    /// for the session); closed only by the "Close" button.
    fn locres_csv_window(&mut self, ctx: &egui::Context) {
        if !self.show_locres_csv {
            return;
        }
        let title = self.i18n.t("dict-from-locres");
        let l_src = self.i18n.t("dict-locres-src-label");
        let l_tgt = self.i18n.t("dict-locres-tgt-label");
        let l_browse = self.i18n.t("settings-browse");
        let l_create = self.i18n.t("dict-plugin-create");
        let l_close = self.i18n.t("btn-close");
        let l_dest = self.i18n.t("dict-conv-kind");
        let l_jeux = self.i18n.t("dict-conv-jeux");
        let l_mods = self.i18n.t("dict-conv-mods");
        let hint = self.i18n.t("dict-locres-hint");
        let langs = self.i18n.t_args(
            "dict-plugin-langs-pair",
            &[
                ("src", &self.active_settings().source.to_uppercase()),
                ("dst", &self.active_settings().target.to_uppercase()),
            ],
        );

        let mut do_build = false;
        let mut close = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(langs);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(hint).weak());
                ui.add_space(8.0);

                ui.label(l_src);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.lc_src_path).desired_width(400.0));
                    if ui.button(l_browse.as_str()).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter(self.i18n.t("dialog-locres"), &["csv", "json"])
                            .set_directory(browse_start_for_file(&self.lc_src_path))
                            .pick_file()
                    {
                        self.lc_src_path = p.display().to_string();
                    }
                });
                ui.add_space(8.0);

                ui.label(l_tgt);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.lc_tgt_path).desired_width(400.0));
                    if ui.button(l_browse.as_str()).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .add_filter(self.i18n.t("dialog-locres"), &["csv", "json"])
                            .set_directory(browse_start_for_file(&self.lc_tgt_path))
                            .pick_file()
                    {
                        self.lc_tgt_path = p.display().to_string();
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(l_dest.as_str());
                    ui.radio_value(&mut self.lc_to_game, true, l_jeux.as_str());
                    ui.radio_value(&mut self.lc_to_game, false, l_mods.as_str());
                });
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button(l_create.as_str()).clicked() {
                        do_build = true;
                    }
                    if ui.button(l_close.as_str()).clicked() {
                        close = true;
                    }
                });
            });

        // Ignore a build while a previous save awaits the confirmation dialog.
        if do_build && self.pending_save.is_none() {
            let s = self.lc_src_path.trim().to_string();
            let t = self.lc_tgt_path.trim().to_string();
            let (sp, tp) = (std::path::Path::new(&s), std::path::Path::new(&t));
            if s.is_empty() || t.is_empty() || !sp.is_file() || !tp.is_file() {
                self.alert_msg("dict-csv-missing");
            } else {
                self.build_locres_csv_pair(sp, tp, self.lc_to_game);
            }
        }
        if close {
            self.show_locres_csv = false;
        }
    }

    /// Read a plugin and fill each entry's `source` with its text in `language`
    /// (resolving external STRINGS for localized plugins).
    fn plugin_text_tm(path: &std::path::Path, language: &str) -> Result<TranslationMemory, String> {
        let mut tm = crate::plugin::read_file(path).map_err(|e| format!("{e}"))?;
        if tm.entries.iter().any(|e| e.idstexte >= 0) {
            Self::resolve_strings_for(path, &mut tm, language);
        }
        Ok(tm)
    }

    /// Default folder for a built dictionary: `assets/bases/jeux` for game
    /// dictionaries (`to_game`), `assets/bases/mods` for mod dictionaries. The
    /// folder is created if missing.
    fn dict_base_dir(&self, to_game: bool) -> PathBuf {
        let dir = if to_game {
            self.config.game_base_dir()
        } else {
            self.config.mod_base_dir()
        };
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// The `bases` root (parent of `bases/jeux` — i.e. `assets/bases`), used as
    /// the starting folder for the ximdb file/folder pickers so they open where
    /// the dictionaries live rather than at an arbitrary location.
    fn bases_root(&self) -> PathBuf {
        let g = self.dict_base_dir(true);
        g.parent().map(|p| p.to_path_buf()).unwrap_or(g)
    }

    /// The STRINGS folder of a detected game, when known: `<GamePath>/Data/Strings`
    /// (or `Data`, or the game root as it degrades). Used to pre-point the STRINGS
    /// pickers at the actual game the user is working on.
    fn game_strings_dir(&self, game: crate::games::Game) -> Option<PathBuf> {
        let root = PathBuf::from(self.config.game_path(game.id())?);
        let data = root.join(crate::games::detect::data_subdir(game));
        let strings = data.join("Strings");
        Some(if strings.is_dir() {
            strings
        } else if data.is_dir() {
            data
        } else {
            root
        })
    }

    /// The configured root folder of the external tool a conversion belongs to
    /// (ESP-ESM Translator for `.eet`, xTranslator for `.sst`), if set. Used to
    /// open the conversion pickers where those dictionaries live.
    fn conv_tool_dir(&self, kind: ConvKind) -> Option<PathBuf> {
        let d = if kind.is_eet() {
            self.config.eet_dir.trim()
        } else {
            self.config.xtranslator_dir.trim()
        };
        if d.is_empty() {
            return None;
        }
        let root = PathBuf::from(d);
        // Mod dictionaries live in a fixed sub-folder of each tool: `Bases Persos`
        // for ESP-ESM Translator, `UserDictionaries` for xTranslator. The pickers
        // open there and the ximdb → EET/SST outputs are written there.
        let sub = root.join(if kind.is_eet() {
            "Bases Persos"
        } else {
            "UserDictionaries"
        });
        Some(if sub.is_dir() { sub } else { root })
    }

    /// Fallback game for a conversion whose source folder is not a recognised
    /// `<Jeu>` folder: the toolbar selection when set, else Skyrim SE.
    fn fallback_conv_game(&self) -> crate::games::Game {
        match self.active_settings().game() {
            crate::games::Game::Unknown => crate::games::Game::SkyrimSE,
            g => g,
        }
    }

    /// Destination of a **ximdb → EET/SST** conversion for one `.ximdb` picked
    /// under `assets/bases/mods/<Jeu>/`: `<EET>/Bases Persos/<Jeu EET>/<stem>.eet`
    /// or `<xT>/UserDictionaries/<Jeu>/<stem>.sst` (rules of 30/09/2026). `None`
    /// when the file is not under a game folder or the game has no EET folder.
    fn conv_export_out_path(&self, kind: ConvKind, input: &std::path::Path) -> Option<PathBuf> {
        let game = game_from_folder(input, false);
        if game == crate::games::Game::Unknown {
            return None;
        }
        let tool = self.conv_tool_dir(kind)?;
        let sub = if kind.is_eet() {
            eet_folder_for(game)?.to_string()
        } else {
            game.id().to_string()
        };
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sortie");
        Some(tool.join(sub).join(format!("{stem}.{}", kind.dst_ext())))
    }

    /// A detected game's parent folder (typically `steamapps/common`), used to
    /// open folder pickers where the games actually live.
    fn any_game_common_dir(&self) -> Option<PathBuf> {
        self.config
            .games
            .values()
            .find(|e| !e.path.trim().is_empty())
            .and_then(|e| PathBuf::from(&e.path).parent().map(|p| p.to_path_buf()))
    }

    /// Write the rows as `<name>_<src>_<dst>.ximdb` directly into the game- or
    /// mod-bases folder (`to_game` selects which).
    fn finish_ximdb_build(
        &mut self,
        rows: Vec<crate::basedb::BaseRow>,
        name: &str,
        src: &str,
        dst: &str,
        to_game: bool,
    ) {
        if rows.is_empty() {
            self.alert_msg("dict-empty");
            return;
        }
        // The sub-folder is always the game chosen in the toolbar "Jeu" selector.
        // A game dictionary is named after the game; a mod dictionary after the
        // mod (the passed `name`). Naming matches the auto-generated dictionaries
        // (<Nom>_<SRC>-<DST>.ximdb). The write is deferred until the user confirms
        // the game in [`XimodApp::dict_confirm_window`].
        let sanitize = |s: &str| -> String {
            s.chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect()
        };
        let game_sub = sanitize(self.active_settings().game.trim());
        let (dir, base) = if to_game {
            (self.dict_base_dir(true).join(&game_sub), game_sub.clone())
        } else {
            (self.dict_base_dir(false).join(&game_sub), sanitize(name))
        };
        let out = dir.join(format!(
            "{base}_{}-{}.ximdb",
            src.to_uppercase(),
            dst.to_uppercase()
        ));
        self.pending_save = Some(PendingSave::Rows {
            out,
            lang: dst.to_string(),
            rows,
        });
    }

    /// Convert a single file `input` to `out` per `kind`. EET/SST → ximdb uses the
    /// current **target** language for the base tag; ximdb → EET/SST exports back.
    fn dict_convert_one(
        &self,
        kind: ConvKind,
        input: &std::path::Path,
        out: &std::path::Path,
    ) -> Result<usize, String> {
        if kind.to_ximdb() {
            let lang = self.active_settings().target.clone();
            let files = [input.to_path_buf()];
            let (_read, n) = crate::basedb::compile_files(&files, &lang, out)?;
            Ok(n)
        } else {
            crate::basedb::export_ximdb(input, out)
        }
    }

    /// Unitary conversion. For **EET/SST → ximdb** the destination is not asked
    /// for here: the file is picked, then the game / jeu-mod dialog decides
    /// whether it lands in `assets/bases/jeux` or `assets/bases/mods`. For
    /// **ximdb → EET/SST** a destination *folder* is chosen (the file keeps the
    /// source stem).
    fn dict_convert_unit(&mut self, kind: ConvKind) {
        if self.require_pair().is_none() {
            return;
        }
        // For ximdb → EET/SST the source is a `.ximdb` (open in `bases`); for
        // EET/SST → ximdb the source lives in the external tool's folder.
        let mut fd =
            rfd::FileDialog::new().add_filter(kind.src_ext().to_uppercase(), &[kind.src_ext()]);
        if kind.to_ximdb() {
            if let Some(d) = self.conv_tool_dir(kind) {
                fd = fd.set_directory(d);
            }
        } else {
            fd = fd.set_directory(self.dict_base_dir(false));
        }
        let Some(input) = fd.pick_file() else {
            return;
        };
        if kind.to_ximdb() {
            self.dict_conv = Some(PendingConv {
                kind,
                inputs: vec![input],
                batch: false,
                game: self.fallback_conv_game(),
            });
            return;
        }
        // ximdb → EET/SST: the destination is derived from the `<Jeu>` folder of
        // the picked file (no destination picker).
        let Some(out) = self.conv_export_out_path(kind, &input) else {
            self.alert_msg("dict-conv-not-in-game-folder");
            return;
        };
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.status = match self.dict_convert_one(kind, &input, &out) {
            Ok(n) => self.i18n.dash(
                &self.i18n.t("dict-done"),
                &self.i18n.t_args(
                    "dict-result-entries",
                    &[("n", &n.to_string()), ("path", &out.display().to_string())],
                ),
            ),
            Err(e) => self.i18n.colon(&self.i18n.t("dict-error"), &e.to_string()),
        };
    }

    /// Batch conversion: pick an input folder. For **EET/SST → ximdb** the
    /// matching files are gathered and the game / jeu-mod dialog then writes them
    /// to `assets/bases/jeux` or `assets/bases/mods`. For **ximdb → EET/SST** a
    /// destination folder is chosen and every file is converted into it.
    fn dict_convert_batch(&mut self, kind: ConvKind) {
        if self.require_pair().is_none() {
            return;
        }
        // ximdb → EET/SST reads its `.ximdb` from the `bases` folder; EET/SST →
        // ximdb reads from the external tool's folder.
        let mut fd = rfd::FileDialog::new();
        if kind.to_ximdb() {
            if let Some(d) = self.conv_tool_dir(kind) {
                fd = fd.set_directory(d);
            }
        } else {
            fd = fd.set_directory(self.dict_base_dir(false));
        }
        let Some(indir) = fd.pick_folder() else {
            return;
        };
        // Walk sub-folders too: mod dictionaries are laid out one game per
        // sub-folder (e.g. Bases Persos/<Jeu>/…, UserDictionaries/<Jeu>/…), so a
        // batch pointed at the root must reach every game's files.
        let inputs = collect_files_recursive(&indir, kind.src_ext());

        if kind.to_ximdb() {
            if inputs.is_empty() {
                self.status = self.i18n.t("dict-conv-none");
                return;
            }
            self.dict_conv = Some(PendingConv {
                kind,
                inputs,
                batch: true,
                game: self.fallback_conv_game(),
            });
            return;
        }

        // ximdb → EET/SST: every file goes to the tool's folder of its own game
        // (`Bases Persos/<Jeu EET>` or `UserDictionaries/<Jeu>`), decided by the
        // `<Jeu>` folder it was read from. Files outside a game folder are
        // counted as errors.
        let (mut files, mut total, mut errors) = (0usize, 0usize, 0usize);
        let mut last_dir: Option<PathBuf> = None;
        for path in &inputs {
            let Some(out) = self.conv_export_out_path(kind, path) else {
                errors += 1;
                continue;
            };
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
                last_dir = Some(parent.to_path_buf());
            }
            match self.dict_convert_one(kind, path, &out) {
                Ok(n) => {
                    files += 1;
                    total += n;
                }
                Err(_) => errors += 1,
            }
        }
        let shown = last_dir
            .or_else(|| self.conv_tool_dir(kind))
            .unwrap_or_default();
        self.status = self.i18n.dash(
            &self.i18n.t("dict-done"),
            &self.i18n.t_args(
                "dict-result-batch",
                &[
                    ("files", &files.to_string()),
                    ("n", &total.to_string()),
                    ("errors", &errors.to_string()),
                    ("path", &shown.display().to_string()),
                ],
            ),
        );
    }

    /// Run a pending **EET/SST → ximdb** conversion once confirmed. The result is
    /// always a **mod** base: `assets/bases/mods/<Jeu>/<NomDuMod>_<SRC>-<DST>.ximdb`,
    /// the game coming from the source file's `<Jeu>` folder (ESP-ESM Translator
    /// names mapped by [`EET_GAME_FOLDERS`], xTranslator ids as is), the mod name
    /// and languages from the file name. `SRC`/`DST` fall back to the current
    /// source/target languages, ISO 639-3 upper-cased.
    fn finish_dict_conv(&mut self, p: PendingConv) {
        let src = self.active_settings().source.to_uppercase();
        let dst = self.active_settings().target.to_uppercase();
        let (mut files, mut total, mut errors) = (0usize, 0usize, 0usize);
        let mut last_dir = self.dict_base_dir(false);
        for input in &p.inputs {
            let out = self.conv_out_path(&p, input, &src, &dst);
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
                last_dir = parent.to_path_buf();
            }
            match self.dict_convert_one(p.kind, input, &out) {
                Ok(n) => {
                    files += 1;
                    total += n;
                }
                Err(_) => errors += 1,
            }
        }
        self.status = if p.batch {
            self.i18n.dash(
                &self.i18n.t("dict-done"),
                &self.i18n.t_args(
                    "dict-result-batch",
                    &[
                        ("files", &files.to_string()),
                        ("n", &total.to_string()),
                        ("errors", &errors.to_string()),
                        ("path", &last_dir.display().to_string()),
                    ],
                ),
            )
        } else if errors > 0 {
            self.i18n.t("dict-error")
        } else {
            self.i18n.dash(
                &self.i18n.t("dict-done"),
                &self.i18n.t_args(
                    "dict-result-entries",
                    &[
                        ("n", &total.to_string()),
                        ("path", &last_dir.display().to_string()),
                    ],
                ),
            )
        };
    }

    /// Destination `.ximdb` path for one converted source file. The game, the
    /// language pair and the mod name are decoded from the **source file name**
    /// ([`parse_dict_name`]) so a folder that mixes several games / language
    /// pairs converts correctly file-by-file. `cfg_src`/`cfg_dst` (the current
    /// languages, ISO 639-3 upper-cased) are the fallback when the name carries
    /// no recognizable language pair. See [`XimodApp::finish_dict_conv`].
    fn conv_out_path(
        &self,
        p: &PendingConv,
        input: &std::path::Path,
        cfg_src: &str,
        cfg_dst: &str,
    ) -> PathBuf {
        let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let parts = parse_dict_name(stem);
        let (src, dst) = parts
            .langs
            .unwrap_or_else(|| (cfg_src.to_string(), cfg_dst.to_string()));
        // The game sub-folder comes from the source file's `<Jeu>` folder (EET
        // full names or xTranslator ids), falling back to the dialog's game when
        // the folder is not a game folder. Mod name and languages come from the
        // file name. Result: assets/bases/mods/<Jeu>/<NomDuMod>_<SRC>-<DST>.ximdb.
        let game = match game_from_folder(input, p.kind.is_eet()) {
            crate::games::Game::Unknown => p.game,
            g => g,
        };
        let name = if parts.core.is_empty() {
            stem
        } else {
            parts.core.as_str()
        };
        let safe: String = name
            .chars()
            .map(|c| if std::path::is_separator(c) { '_' } else { c })
            .collect();
        self.dict_base_dir(false)
            .join(game.id())
            .join(format!("{safe}_{src}-{dst}.ximdb"))
    }

    /// The game / jeu-mod dialog shown for a pending "vers ximdb" conversion.
    /// Lets the user pick the target game and whether the result is a game base
    /// (`assets/bases/jeux`) or a mod base (`assets/bases/mods`), then runs it.
    fn dict_conv_window(&mut self, ctx: &egui::Context) {
        if self.dict_conv.is_none() {
            return;
        }
        let title = self.i18n.t("dict-conv-title");
        let lbl_game = self.i18n.t("dict-conv-game");
        let lbl_ok = self.i18n.t("dict-conv-ok");
        let lbl_cancel = self.i18n.t("dict-conv-cancel");
        let lbl_hint = self.i18n.t("dict-conv-hint");
        let lbl_preview = self.i18n.t("dict-conv-preview");
        let info = {
            let p = self.dict_conv.as_ref().unwrap();
            if p.batch {
                format!("{} {}", self.i18n.t("dict-conv-batch-info"), p.inputs.len())
            } else {
                p.inputs
                    .first()
                    .and_then(|f| f.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
            }
        };
        // Live example of the resulting name (reflects the current selections;
        // egui redraws every frame, so it tracks the drop-down / radio).
        let preview = {
            let p = self.dict_conv.as_ref().unwrap();
            let cfg_src = self.active_settings().source.to_uppercase();
            let cfg_dst = self.active_settings().target.to_uppercase();
            p.inputs.first().map(|f| {
                let out = self.conv_out_path(p, f, &cfg_src, &cfg_dst);
                let fname = out
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let sub = out
                    .parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                format!("mods/{sub}/{fname}")
            })
        };
        // The game drop-down is only a fallback: shown when at least one source
        // file is not under a recognised `<Jeu>` folder.
        let needs_game = {
            let p = self.dict_conv.as_ref().unwrap();
            p.inputs
                .iter()
                .any(|f| game_from_folder(f, p.kind.is_eet()) == crate::games::Game::Unknown)
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let p = self.dict_conv.as_mut().unwrap();
                ui.add_space(4.0);
                ui.label(info.as_str());
                ui.add_space(2.0);
                ui.weak(lbl_hint.as_str());
                if needs_game {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(lbl_game.as_str());
                        egui::ComboBox::from_id_salt("dict_conv_game")
                            .selected_text(p.game.display_name())
                            .show_ui(ui, |ui| {
                                for g in crate::games::Game::ALL {
                                    if *g != crate::games::Game::Unknown {
                                        ui.selectable_value(&mut p.game, *g, g.display_name());
                                    }
                                }
                            });
                    });
                }
                if let Some(prev) = &preview {
                    ui.add_space(6.0);
                    ui.weak(format!("{} {}", lbl_preview, prev));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(lbl_ok.as_str()).clicked() {
                        confirm = true;
                    }
                    if ui.button(lbl_cancel.as_str()).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dict_conv = None;
        } else if confirm && let Some(p) = self.dict_conv.take() {
            self.finish_dict_conv(p);
        }
    }

    /// Build a consolidated dictionary from a game (a folder of base files) or a
    /// mod (one or more translation files), saved as `<name>_<src>_<dst>.ximdb`
    /// (languages from the current source/target selection).
    fn dict_build_dictionary(&mut self, from_game: bool) {
        let Some((src, dst)) = self.require_pair() else {
            return;
        };

        // Pick the input(s) and derive a default base name.
        let (files, name): (Vec<std::path::PathBuf>, String) = if from_game {
            let mut fd = rfd::FileDialog::new();
            if let Some(d) = self.any_game_common_dir() {
                fd = fd.set_directory(d);
            }
            let Some(dir) = fd.pick_folder() else {
                return;
            };
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("jeu")
                .to_string();
            (crate::basedb::raw_bases_in_dir(&dir), name)
        } else {
            let Some(picked) = rfd::FileDialog::new()
                .add_filter(
                    "EET/SST/DSD/JSON/XML",
                    &["eet", "sst", "dsd", "json", "xml"],
                )
                .pick_files()
            else {
                return;
            };
            let name = picked
                .first()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("mod")
                .to_string();
            (picked, name)
        };

        // Always rang under the game chosen in the toolbar "Jeu" selector: a game
        // dictionary is named after the game, a mod one after the mod. The compile
        // is deferred until the user confirms the game.
        let sanitize = |s: &str| -> String {
            s.chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect()
        };
        let game_sub = sanitize(self.active_settings().game.trim());
        let (dir, base) = if from_game {
            (self.dict_base_dir(true).join(&game_sub), game_sub.clone())
        } else {
            (self.dict_base_dir(false).join(&game_sub), sanitize(&name))
        };
        let out = dir.join(format!(
            "{base}_{}-{}.ximdb",
            src.to_uppercase(),
            dst.to_uppercase()
        ));
        self.pending_save = Some(PendingSave::Compile {
            out,
            lang: dst.clone(),
            files,
        });
    }

    // --------------------------- archive navigator -------------------------

    /// Open an archive file and show the navigator window.
    fn open_archive(&mut self, path: PathBuf) {
        match crate::archive::Archive::open(&path) {
            Ok(a) => {
                self.archive_status = format!(
                    "{} — {} · {} {}",
                    a.file_name(),
                    a.kind(),
                    a.entries().len(),
                    self.i18n.t("archive-entries")
                );
                self.archive = Some(a);
                self.archive_selected = None;
                self.archive_filter.clear();
                self.show_archive = true;
                self.archive_needs_center = true;
            }
            Err(e) => {
                self.status = self
                    .i18n
                    .colon(&self.i18n.t("archive-open-error"), &e.to_string());
            }
        }
    }

    /// Entry indices matching the current filter (substring + translatable-only).
    fn archive_visible(&self) -> Vec<usize> {
        let Some(a) = &self.archive else {
            return Vec::new();
        };
        let needle = self.archive_filter.trim().to_lowercase();
        a.entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (!self.archive_translatable_only || e.is_translatable())
                    && (needle.is_empty() || e.path.to_lowercase().contains(&needle))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Extract entry `index` to a temp file and open it in the translator.
    fn archive_open_entry(&mut self, index: usize) {
        // Phase 1 — pull the plugin (and, if it is a plugin, this archive's own
        // matching STRINGS) out while the archive is borrowed.
        let extracted: Option<ExtractedPlugin> = {
            let Some(a) = &self.archive else {
                return;
            };
            let Some(entry) = a.entries().get(index) else {
                return;
            };
            let name = entry.file_name().to_string();
            let plugin_bytes = match a.extract(index) {
                Ok(b) => b,
                Err(e) => {
                    self.status = self
                        .i18n
                        .colon(&self.i18n.t("archive-extract-error"), &e.to_string());
                    return;
                }
            };
            let is_plugin = crate::games::registry().is_plugin_extension(
                std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            );
            let mut strings = Vec::new();
            if is_plugin {
                let stem = std::path::Path::new(&name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                for (i, e) in a.entries().iter().enumerate() {
                    if crate::lang::match_strings_filename(e.file_name(), &stem).is_some()
                        && let Ok(sb) = a.extract(i)
                    {
                        strings.push((e.file_name().to_string(), sb));
                    }
                }
            }
            Some((name, plugin_bytes, strings))
        };
        let Some((name, plugin_bytes, strings)) = extracted else {
            return;
        };
        // Phase 2 — write to a temp folder and load (STRINGS sit next to the
        // plugin so the loose-file resolver picks them up).
        let dir = archive_temp_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.status = self
                .i18n
                .colon(&self.i18n.t("archive-extract-error"), &e.to_string());
            return;
        }
        for (fname, sb) in &strings {
            let _ = std::fs::write(dir.join(fname), sb);
        }
        let out = dir.join(&name);
        match std::fs::write(&out, &plugin_bytes) {
            Ok(_) => self.load_path(out),
            Err(e) => {
                self.status = self
                    .i18n
                    .colon(&self.i18n.t("archive-extract-error"), &e.to_string())
            }
        }
    }

    /// Path of a loose plugin sitting next to the open archive whose name matches
    /// it (ignoring case/separators) — e.g. `X.esl` beside `X.bsa`, or beside
    /// `X - Textures.bsa`. Prefers the most specific (longest) matching stem.
    fn associated_plugin_path(&self) -> Option<PathBuf> {
        let a = self.archive.as_ref()?;
        let dir = a.path().parent()?;
        let astem_norm = crate::lang::normalize_name(a.path().file_stem()?.to_str()?);
        let mut best: Option<PathBuf> = None;
        let mut best_len = 0usize;
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let p = entry.path();
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !crate::games::registry().is_plugin_extension(&ext) {
                continue;
            }
            let pnorm = p
                .file_stem()
                .and_then(|s| s.to_str())
                .map(crate::lang::normalize_name)
                .unwrap_or_default();
            if !pnorm.is_empty() && astem_norm.starts_with(&pnorm) && pnorm.len() > best_len {
                best_len = pnorm.len();
                best = Some(p);
            }
        }
        best
    }

    /// Open the loose plugin associated with the open archive; its STRINGS resolve
    /// from that same archive (via the sibling-archive resolver).
    fn open_associated_plugin(&mut self) {
        match self.associated_plugin_path() {
            Some(p) => self.load_path(p),
            None => self.status = self.i18n.t("archive-no-plugin"),
        }
    }

    // --------------------------- book preview ------------------------------

    /// The book-preview window: renders the selected book's source and
    /// translation side by side, honoring Bethesda markup.
    fn book_window(&mut self, ctx: &egui::Context) {
        if !self.show_book {
            return;
        }
        let Some(idx) = self.book_idx else {
            self.show_book = false;
            return;
        };
        // Tied to the document it was opened from; close if the tab changed or
        // the entry vanished.
        if self.active != self.book_doc
            || self
                .docs
                .get(self.active)
                .and_then(|d| d.tm.entries.get(idx))
                .is_none()
        {
            self.show_book = false;
            return;
        }
        let cur_trans = self.docs[self.active].tm.entries[idx].trans.clone();

        // (Re)parse when the entry or its translation changed.
        let stale = match &self.book_cache {
            Some((d, i, t)) => *d != self.book_doc || *i != idx || *t != cur_trans,
            None => true,
        };
        if stale {
            let doc = &self.docs[self.active];
            let e = &doc.tm.entries[idx];
            let src = e.source.clone();
            let edid = e.edid.clone();
            self.book_title = doc
                .tm
                .entries
                .iter()
                .find(|x| {
                    x.rec_sig == "BOOK"
                        && x.field_sig == "FULL"
                        && !edid.is_empty()
                        && x.edid == edid
                })
                .map(|x| {
                    if x.trans.is_empty() {
                        x.source.clone()
                    } else {
                        x.trans.clone()
                    }
                })
                .unwrap_or_default();
            self.book_src_blocks = crate::book::parse(&src);
            self.book_trans_blocks = crate::book::parse(&cur_trans);
            self.book_cache = Some((self.book_doc, idx, cur_trans));
        }

        let id = egui::ViewportId::from_hash_of("ximod-book");
        let (w, h) = (self.config.book_w, self.config.book_h);
        let mut builder = egui::ViewportBuilder::default().with_title(self.i18n.t("book-title"));
        let just_opened = self.book_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.book_x, self.config.book_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        let lbl_src = self.i18n.t("book-source");
        let lbl_trans = self.i18n.t("book-translation");
        let lbl_gamefonts = self.i18n.t("book-game-fonts");
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.book_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                ui.horizontal(|ui| {
                    if !self.book_title.is_empty() {
                        ui.heading(&self.book_title);
                    }
                    // Game-fonts toggle (only meaningful when fonts are installed).
                    if !self.book_fonts.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.book_use_game_fonts, &lbl_gamefonts);
                        });
                    }
                });
                ui.separator();
                let base = ui.visuals().text_color();
                let use_game = self.book_use_game_fonts && !self.book_fonts.is_empty();
                let col_w = ((ui.available_width() - 28.0) / 2.0).max(120.0);
                ui.columns(2, |cols| {
                    cols[0].label(egui::RichText::new(&lbl_src).strong());
                    egui::ScrollArea::vertical()
                        .id_salt("book_src")
                        .show(&mut cols[0], |ui| {
                            render_book(
                                ui,
                                &self.book_src_blocks,
                                col_w - 16.0,
                                base,
                                &self.book_fonts,
                                use_game,
                            );
                        });
                    cols[1].label(egui::RichText::new(&lbl_trans).strong());
                    egui::ScrollArea::vertical()
                        .id_salt("book_trans")
                        .show(&mut cols[1], |ui| {
                            render_book(
                                ui,
                                &self.book_trans_blocks,
                                col_w - 16.0,
                                base,
                                &self.book_fonts,
                                use_game,
                            );
                        });
                });
            });
            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.book_x = x;
                self.config.book_y = y;
                self.config.book_w = w;
                self.config.book_h = h;
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.show_book = false;
            }
        });
    }

    /// Extract entry `index` to a user-chosen location on disk.
    fn archive_extract_entry(&mut self, index: usize, parent: &eframe::Frame) {
        let Some(a) = &self.archive else {
            return;
        };
        let Some(entry) = a.entries().get(index) else {
            return;
        };
        let name = entry.file_name().to_string();
        let bytes = match a.extract(index) {
            Ok(b) => b,
            Err(e) => {
                self.status = self
                    .i18n
                    .colon(&self.i18n.t("archive-extract-error"), &e.to_string());
                return;
            }
        };
        if let Some(dest) = rfd::FileDialog::new()
            .set_parent(parent)
            .set_file_name(&name)
            .save_file()
        {
            match std::fs::write(&dest, &bytes) {
                Ok(_) => {
                    self.status = self.i18n.colon(
                        &self.i18n.t("archive-extracted"),
                        &dest.display().to_string(),
                    )
                }
                Err(e) => {
                    self.status = self
                        .i18n
                        .colon(&self.i18n.t("archive-extract-error"), &e.to_string())
                }
            }
        }
    }

    /// The archive-navigator window (independent, resizable; geometry saved).
    fn archive_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.show_archive {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-archive");
        let (w, h) = (self.config.archive_w, self.config.archive_h);
        let mut builder = egui::ViewportBuilder::default().with_title(self.i18n.t("archive-title"));
        let just_opened = self.archive_needs_center;
        if just_opened {
            let pos = self.open_pos(self.config.archive_x, self.config.archive_y, w, h);
            builder = builder
                .with_inner_size([w, h])
                .with_position(pos)
                .with_active(true);
        }
        let mut open_idx: Option<usize> = None;
        let mut extract_idx: Option<usize> = None;
        let mut open_plugin = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            self.archive_needs_center = false;
            if just_opened {
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                self.archive_body(ui, &mut open_idx, &mut extract_idx, &mut open_plugin);
            });
            if let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.archive_x = x;
                self.config.archive_y = y;
                self.config.archive_w = w;
                self.config.archive_h = h;
            }
            if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.show_archive = false;
            }
        });
        if let Some(i) = open_idx {
            self.archive_open_entry(i);
        }
        if let Some(i) = extract_idx {
            self.archive_extract_entry(i, frame);
        }
        if open_plugin {
            self.open_associated_plugin();
        }
    }

    fn archive_body(
        &mut self,
        ui: &mut egui::Ui,
        open_idx: &mut Option<usize>,
        extract_idx: &mut Option<usize>,
        open_plugin: &mut bool,
    ) {
        ui.label(&self.archive_status);
        ui.separator();

        // Precompute labels so the table closures borrow no `self`.
        let lbl_filter = self.i18n.t("toolbar-filter");
        let lbl_trans = self.i18n.t("archive-translatable-only");
        let lbl_open = self.i18n.t("archive-open-in-translator");
        let lbl_extract = self.i18n.t("archive-extract");
        let lbl_open_plugin = self.i18n.t("archive-open-plugin");
        let lbl_open_plugin_hint = self.i18n.t("archive-open-plugin-hint");
        let col_path = self.i18n.t("archive-col-path");
        let col_size = self.i18n.t("archive-col-size");
        // Loose plugin next to the archive (its STRINGS resolve from this archive).
        let has_assoc = self.associated_plugin_path().is_some();

        ui.horizontal(|ui| {
            ui.label(&lbl_filter);
            ui.add(
                egui::TextEdit::singleline(&mut self.archive_filter)
                    .desired_width(180.0)
                    .hint_text("…"),
            );
            ui.checkbox(&mut self.archive_translatable_only, &lbl_trans);
        });
        ui.add_space(4.0);

        let visible = self.archive_visible();
        let sel = self.archive_selected.filter(|i| visible.contains(i));
        let selected = self.archive_selected;

        let Some(a) = &self.archive else {
            return;
        };
        let entries = a.entries();

        ui.horizontal(|ui| {
            let can = sel.is_some();
            let translatable = sel.map(|i| entries[i].is_translatable()).unwrap_or(false);
            if ui
                .add_enabled(can && translatable, egui::Button::new(&lbl_open))
                .clicked()
            {
                *open_idx = sel;
            }
            if ui
                .add_enabled(can, egui::Button::new(&lbl_extract))
                .clicked()
            {
                *extract_idx = sel;
            }
            if ui
                .add_enabled(has_assoc, egui::Button::new(&lbl_open_plugin))
                .on_hover_text(&lbl_open_plugin_hint)
                .clicked()
            {
                *open_plugin = true;
            }
        });
        ui.add_space(4.0);

        use egui_extras::{Column, TableBuilder};
        let text_h = egui::TextStyle::Body.resolve(ui.style()).size + 8.0;
        let mut clicked: Option<usize> = None;
        let mut double: Option<usize> = None;
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::remainder().at_least(220.0))
            .column(Column::exact(90.0))
            .column(Column::exact(44.0))
            .header(text_h, |mut h| {
                h.col(|ui| {
                    ui.strong(&col_path);
                });
                h.col(|ui| {
                    ui.strong(&col_size);
                });
                h.col(|ui| {
                    ui.strong("Zip");
                });
            })
            .body(|body| {
                body.rows(text_h, visible.len(), |mut row| {
                    let idx = visible[row.index()];
                    let e = &entries[idx];
                    let is_sel = selected == Some(idx);
                    row.col(|ui| {
                        let color = if e.is_translatable() {
                            egui::Color32::from_rgb(120, 190, 120)
                        } else {
                            ui.visuals().text_color()
                        };
                        let resp =
                            ui.selectable_label(is_sel, egui::RichText::new(&e.path).color(color));
                        if resp.clicked() {
                            clicked = Some(idx);
                        }
                        if resp.double_clicked() {
                            double = Some(idx);
                        }
                    });
                    row.col(|ui| {
                        ui.label(self.i18n.size(e.size));
                    });
                    row.col(|ui| {
                        ui.label(if e.compressed {
                            crate::icons::CHECK
                        } else {
                            ""
                        });
                    });
                });
            });

        if let Some(i) = clicked {
            self.archive_selected = Some(i);
        }
        if let Some(i) = double {
            self.archive_selected = Some(i);
            if entries[i].is_translatable() {
                *open_idx = Some(i);
            } else {
                *extract_idx = Some(i);
            }
        }
    }
}

impl XimodApp {
    /// Run a section of the frame, logging it when it is slow and
    /// `XIMOD_FRAME_LOG` is set.
    fn note_asker(&mut self, name: &str) {
        if !self.repaint_askers.iter().any(|n| n == name) {
            self.repaint_askers.push(name.to_string());
        }
    }

    fn timed(&mut self, ctx: &egui::Context, name: &str, f: impl FnOnce(&mut Self)) {
        if self.frame_log == 0 {
            f(self);
            return;
        }
        let started = std::time::Instant::now();
        let asked_before = ctx.has_requested_repaint();
        f(self);
        let ms = started.elapsed().as_millis();
        if ms >= self.frame_log / 4 {
            tracing::warn!("slow section {name}: {ms} ms");
        }
        // Who asks for the next frame (once per five seconds, per section).
        if !asked_before
            && ctx.has_requested_repaint()
            && !self.repaint_askers.iter().any(|n| n == name)
        {
            self.repaint_askers.push(name.to_string());
        }
    }

    fn update_inner(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Apply the theme EVERY frame so it wins over eframe's system-theme
        // following (which otherwise makes a Dark setting render Light/System).
        // The Settings window previews the theme being chosen, like the font
        // size (F-02); Annuler falls back to the saved one on the next frame.
        let theme = if self.show_settings {
            self.temp_theme
        } else {
            self.config.theme
        };
        let dark = match theme {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => !matches!(ctx.input(|i| i.raw.system_theme), Some(egui::Theme::Light)),
        };
        if self.frame_log > 0 {
            // Why this frame is drawn (egui keeps the causes of the previous
            // pass, with the file and line that asked).
            for c in ctx.repaint_causes() {
                self.note_asker(&format!("{}:{} {}", c.file, c.line, c.reason));
            }
        }
        ctx.set_visuals(crate::ui::theme::visuals_for(dark));

        // Notices queued by the startup worker (behind the splash): shown once,
        // as soon as no other alert is pending.
        if self.alert.is_none() {
            let notices = crate::take_startup_notices();
            if !notices.is_empty() {
                let lines: Vec<String> = notices
                    .iter()
                    .map(|n| match n {
                        crate::StartupNotice::FieldsFallback { game, path } => self
                            .i18n
                            .t_args("fields-fallback", &[("game", game), ("path", path)]),
                    })
                    .collect();
                self.alert = Some(lines.join("\n"));
            }
        }

        // Live font size: while the Settings window is open, preview the value
        // being edited (temp_font_size); otherwise use the saved size. Applied
        // only when it actually changes, so closing the dialog without saving
        // reverts the preview to the saved size on the next frame.
        let desired_font = if self.show_settings {
            self.temp_font_size
        } else {
            self.config.font_size
        };
        if (self.applied_font_size - desired_font).abs() > f32::EPSILON {
            apply_font_size(ctx, desired_font);
            self.applied_font_size = desired_font;
        }

        // Apply a resize requested from Settings to the main window.
        if let Some(sz) = self.pending_main_resize.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(sz));
        }

        // Poll the background game-detection thread (and keep its progress
        // window visible) regardless of which view is active.
        self.timed(ctx, "poll_game_detect", |a| a.poll_game_detect(ctx));
        self.timed(ctx, "detect_progress_window", |a| {
            a.detect_progress_window(ctx)
        });

        // Explorers open in their own independent windows (viewports); the
        // translator window keeps running behind them. Several can be open at once.
        self.timed(ctx, "show_tool_windows", |a| {
            a.show_tool_windows(ctx, frame)
        });
        self.timed(ctx, "fields_window", |a| a.fields_window(ctx));
        self.timed(ctx, "formats_window", |a| a.formats_window(ctx));
        self.timed(ctx, "matrices_window", |a| a.matrices_window(ctx));
        self.timed(ctx, "rules_window", |a| a.rules_window(ctx));
        self.timed(ctx, "derive_follow", |a| a.derive_follow());
        self.timed(ctx, "props_window", |a| a.props_window(ctx));
        self.timed(ctx, "assist_window", |a| a.assist_window(ctx));
        self.timed(ctx, "generalize_dialog", |a| a.generalize_dialog(ctx));
        self.timed(ctx, "translation_window", |a| a.translation_window(ctx));
        self.timed(ctx, "tool_windows", |a| a.tool_windows(ctx, frame));

        self.timed(ctx, "update_title", |a| a.update_title(ctx));
        self.timed(ctx, "capture_main_geom", |a| a.capture_main_geom(ctx));
        self.timed(ctx, "handle_drops", |a| a.handle_drops(ctx));

        self.drain_history();
        self.timed(ctx, "autosave_tick", |a| a.autosave_tick(ctx));

        // Ctrl+Alt+Z, then Ctrl+Z / Ctrl+Y (⌘ on macOS), consumed before the
        // grid so the TextEdit's own undo never sees them (A-01, R-A01-01).
        let (hist_prev, undo, redo) = ctx.input_mut(|i| {
            let prev = i.consume_shortcut(&SHORTCUT_HISTORY_PREV);
            let undo = i.consume_shortcut(&SHORTCUT_UNDO);
            let redo = i.consume_shortcut(&SHORTCUT_REDO) || i.consume_shortcut(&SHORTCUT_REDO_ALT);
            (prev, undo, redo)
        });
        if hist_prev && !self.docs.is_empty() && !self.modal_open() {
            let targets = self.selection.targets();
            self.history_revert_previous(&targets);
        }
        if undo {
            self.do_undo();
        }
        if redo {
            self.do_redo();
        }
        self.timed(ctx, "status_keys", |a| a.status_keys(ctx));
        self.timed(ctx, "handle_menu_shortcuts", |a| {
            a.handle_menu_shortcuts(ctx, frame)
        });
        // A tab or document change ends the cell edit in progress.
        if self
            .edit_session
            .as_ref()
            .is_some_and(|s| s.doc != self.active || s.doc >= self.docs.len())
        {
            self.close_edit_session();
        }

        self.timed(ctx, "menu_bar", |a| a.menu_bar(ctx, frame));
        self.timed(ctx, "toolbar", |a| a.toolbar(ctx));
        self.timed(ctx, "filter_bar", |a| a.filter_bar(ctx));
        self.timed(ctx, "tabs_bar", |a| a.tabs_bar(ctx));
        // The anomalies belong to one document (D-05): another active
        // document drops them (and the « Anomalies » filter with them).
        if self.anomaly_serial.is_some()
            && self.anomaly_serial != self.docs.get(self.active).map(|d| d.serial)
        {
            self.anomaly_serial = None;
            self.anomalies.clear();
            self.anomaly_set = Default::default();
            self.filter_anomalies = false;
            self.quality.report = None;
            self.quality.rows.clear();
            self.quality.selected.clear();
            self.quality.message = None;
        }
        // Recompute the filtered view after the filter bar, before it is used by
        // the status bar and the grid.
        self.timed(ctx, "refresh_view", |a| a.refresh_view(ctx));
        self.timed(ctx, "status_bar", |a| a.status_bar(ctx));
        self.timed(ctx, "central", |a| a.central(ctx));
        self.timed(ctx, "edit_window", |a| a.edit_window(ctx));
        self.timed(ctx, "replace_window", |a| a.replace_window(ctx));
        self.timed(ctx, "quality_window", |a| a.quality_window(ctx));
        self.timed(ctx, "dicts_window", |a| a.dicts_window(ctx));
        self.timed(ctx, "dict_editor_window", |a| a.dict_editor_window(ctx));
        self.timed(ctx, "apply_dialog_window", |a| a.apply_dialog_window(ctx));
        self.timed(ctx, "assist_dialog_window", |a| a.assist_dialog_window(ctx));
        self.timed(ctx, "update_window", |a| a.update_window(ctx));
        self.timed(ctx, "dialog_window", |a| a.dialog_window(ctx));
        self.timed(ctx, "fuzzy_poll", |a| {
            a.fuzzy_poll(ctx);
        });
        if self.edit_win.is_none() {
            // The edit window closed with the dialog open: it moves here.
            if let Some(d) = self.propagate_dlg.as_mut() {
                d.in_edit = false;
            }
            self.timed(ctx, "propagate_dialog", |a| a.propagate_dialog(ctx));
        }
        self.timed(ctx, "personal_prompt_window", |a| {
            a.personal_prompt_window(ctx)
        });
        self.timed(ctx, "render_fomod_translation", |a| {
            a.render_fomod_translation(ctx)
        });
        self.frame_time = ctx.input(|i| i.time);
        self.toasts.show(ctx);

        self.timed(ctx, "settings_window", |a| a.settings_window(ctx));
        self.timed(ctx, "flags_window", |a| a.flags_window(ctx));
        self.timed(ctx, "about_window", |a| a.about_window(ctx));
        self.timed(ctx, "status_colors_window", |a| a.status_colors_window(ctx));
        self.timed(ctx, "color_picker_window", |a| a.color_picker_window(ctx));
        self.timed(ctx, "spell_window", |a| a.spell_window(ctx));
        self.timed(ctx, "plugin_pair_window", |a| a.plugin_pair_window(ctx));
        self.timed(ctx, "locres_csv_window", |a| a.locres_csv_window(ctx));
        self.timed(ctx, "archive_window", |a| a.archive_window(ctx, frame));
        self.timed(ctx, "book_window", |a| a.book_window(ctx));
        self.timed(ctx, "dict_conv_window", |a| a.dict_conv_window(ctx));
        self.timed(ctx, "dict_confirm_window", |a| a.dict_confirm_window(ctx));
        self.timed(ctx, "dict_view_window", |a| a.dict_view_window(ctx));
        self.timed(ctx, "dict_games_window", |a| a.dict_games_window(ctx));
        self.timed(ctx, "reload_window", |a| a.reload_window(ctx));
        self.timed(ctx, "output_confirm_window", |a| {
            a.output_confirm_window(ctx)
        });
        self.timed(ctx, "resume_window", |a| a.resume_window(ctx));
        self.timed(ctx, "export_window", |a| a.export_window(ctx, frame));
        self.timed(ctx, "savepart_window", |a| a.savepart_window(ctx, frame));
        self.timed(ctx, "encoding_dialog", |a| a.encoding_dialog(ctx));
        self.timed(ctx, "crc_window", |a| a.crc_window(ctx));
        self.timed(ctx, "import_dialog_window", |a| a.import_dialog_window(ctx));
        self.timed(ctx, "scan_window", |a| a.scan_window(ctx));
        self.timed(ctx, "compare_window", |a| a.compare_window(ctx));
        self.timed(ctx, "review_dialog_window", |a| a.review_dialog_window(ctx));
        self.timed(ctx, "history_confirm_window", |a| {
            a.history_confirm_window(ctx)
        });
        self.timed(ctx, "history_window", |a| a.history_window(ctx));
        self.timed(ctx, "search_window", |a| a.search_window(ctx));
        self.timed(ctx, "scripts_window", |a| a.scripts_window(ctx));
        self.timed(ctx, "concord_window", |a| a.concord_window(ctx));
        self.timed(ctx, "journal_window", |a| a.journal_window(ctx));
        self.timed(ctx, "online_windows", |a| a.online_windows(ctx));
        self.timed(ctx, "close_prompt_window", |a| {
            a.close_prompt_window(ctx, frame)
        });
        self.timed(ctx, "alert_window", |a| a.alert_window(ctx));
        // Quitting with unsaved documents asks first (F-06).
        if ctx.input(|i| i.viewport().close_requested())
            && !self.exit_confirmed
            && self.docs.iter().any(|d| d.is_dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.exit_prompt = true;
        }
    }
}

impl eframe::App for XimodApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Slow frames are logged when XIMOD_FRAME_LOG is set (a diagnostic,
        // English like the rest of the log).
        let started = std::time::Instant::now();
        self.update_inner(ctx, frame);
        if self.frame_log > 0 {
            let ms = started.elapsed().as_millis();
            if ms >= self.frame_log {
                tracing::warn!("slow frame: {ms} ms");
            }
            // The frame rate, every five seconds: an interface at rest draws
            // only on an event; a steady rate means something asks for
            // repaints.
            self.frame_count += 1;
            if ctx.has_requested_repaint() {
                self.frame_asked += 1;
            }
            if ctx.input(|i| !i.raw.events.is_empty()) {
                self.frame_events += 1;
            }
            if self.frame_since.elapsed() >= std::time::Duration::from_secs(5) {
                tracing::warn!(
                    "frames: {} in {:.1} s ({} asked by the application, {} with input events; asked by {:?})",
                    self.frame_count,
                    self.frame_since.elapsed().as_secs_f32(),
                    self.frame_asked,
                    self.frame_events,
                    self.repaint_askers
                );
                self.repaint_askers.clear();
                self.frame_count = 0;
                self.frame_asked = 0;
                self.frame_events = 0;
                self.frame_since = std::time::Instant::now();
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A normal exit drops every open document's rescue files (R-B11-04);
        // a crash leaves them for the resume dialog.
        for d in &self.docs {
            let _ = self.autosave.discard(&d.path);
        }
        let _ = self.config.save();
    }
}

// ------------------------------ helpers ------------------------------------

/// Render parsed book blocks into `ui`, wrapping at `wrap` and using `base` as
/// the text color (paragraphs keep their alignment; handwritten/italic runs are
/// italicized; page breaks are separators; images show a placeholder line).
fn render_book(
    ui: &mut egui::Ui,
    blocks: &[crate::book::Block],
    wrap: f32,
    base: egui::Color32,
    fonts: &crate::fonts::BookFonts,
    use_game_fonts: bool,
) {
    use crate::book::{Align, Block};
    let font = egui::TextStyle::Body.resolve(ui.style());
    // Default printed book face for untagged text (when game fonts are on).
    let default_fam = if use_game_fonts {
        fonts.family_for("$SkyrimBooks")
    } else {
        None
    };
    for b in blocks {
        match b {
            Block::Paragraph { runs, align } => {
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = wrap;
                job.halign = match align {
                    Align::Left => egui::Align::LEFT,
                    Align::Center => egui::Align::Center,
                    Align::Right => egui::Align::RIGHT,
                };
                for r in runs {
                    // Map the requested game face to a user font when available;
                    // otherwise fall back to the default face and italicize
                    // handwritten runs as a visual cue.
                    let fam = if use_game_fonts {
                        r.face
                            .as_deref()
                            .and_then(|f| fonts.family_for(f))
                            .or(default_fam)
                    } else {
                        None
                    };
                    let font_id = match fam {
                        Some(f) => egui::FontId::new(font.size, egui::FontFamily::Name(f.into())),
                        None => font.clone(),
                    };
                    job.append(
                        &r.text,
                        0.0,
                        egui::TextFormat {
                            font_id,
                            color: base,
                            italics: r.italic && fam.is_none(),
                            ..Default::default()
                        },
                    );
                }
                if !job.text.trim().is_empty() {
                    ui.label(job);
                    ui.add_space(6.0);
                }
            }
            Block::PageBreak => {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            Block::Image(src) => {
                ui.weak(format!("🖼 [{src}]"));
                ui.add_space(4.0);
            }
        }
    }
}

/// Close the (top-level) menu owned by `ui` as soon as the pointer leaves its
/// popup — so a menu opened by a click disappears on mouse-out, without needing a
/// second click. `child` is the rectangle of an open sub-menu popup, if any, so
/// the parent stays open while a child sub-menu (e.g. « Récents ») is hovered.
///
/// The keep-open zone is the popup rectangle (unioned with any open child popup),
/// extended upward to bridge the small gap to the menu-bar button, plus a little
/// tolerance around the edges. Nested sub-menus are still closed by egui's own
/// hover handling; only the click-opened top-level menu needs this.
/// Make a drop-down menu size to its content: disable text wrapping so each item
/// takes its natural width (egui otherwise fixes menus at `spacing.menu_width`,
/// which we set to 0 — without this the items would wrap character by character).
/// Call it as the first statement inside every `menu_button` closure.
fn fit_menu(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
}

fn close_menu_if_pointer_left(ui: &mut egui::Ui, child: Option<egui::Rect>) {
    let popup = ui.min_rect();
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if menu_should_close(popup, child, pointer) {
        ui.close_menu();
    }
}

/// Pure geometry decision behind [`close_menu_if_pointer_left`]: should an open
/// menu close given its `popup` rect, an optional open `child` sub-menu rect, and
/// the current `pointer` position? The keep-open zone is `popup ∪ child`, extended
/// upward to bridge the gap to the menu-bar button, plus edge tolerance. When the
/// pointer position is unknown (mouse outside the window) the menu is kept open.
fn menu_should_close(
    popup: egui::Rect,
    child: Option<egui::Rect>,
    pointer: Option<egui::Pos2>,
) -> bool {
    // Upward bridge to reach the menu-bar button above the popup.
    const BRIDGE_UP: f32 = 28.0;
    // Edge tolerance so grazing the border doesn't flicker the menu shut.
    const PAD: f32 = 6.0;
    let mut zone = popup;
    if let Some(c) = child {
        zone = zone.union(c);
    }
    zone.min.y -= BRIDGE_UP;
    let zone = zone.expand(PAD);
    match pointer {
        Some(p) => !zone.contains(p),
        None => false,
    }
}

/// A temp directory for files extracted from archives before loading them.
fn archive_temp_dir() -> PathBuf {
    std::env::temp_dir()
        .join("ximod-translator")
        .join("archive")
}

/// Replace every whole-word occurrence of `from` with `to` in `text`, using the
/// same tokenization as the checker so only real words (never fragments inside
/// tags or identifiers) are touched. Works right-to-left to keep byte offsets
/// valid across replacements.
fn replace_word(text: &mut String, from: &str, to: &str) {
    let spans: Vec<(usize, usize)> = crate::spell::tokenize::tokens(text)
        .into_iter()
        .filter(|t| t.word == from)
        .map(|t| (t.start, t.end))
        .collect();
    for (s, e) in spans.into_iter().rev() {
        text.replace_range(s..e, to);
    }
}

/// Read the current viewport's outer position and inner size.
pub(crate) fn read_geometry(ctx: &egui::Context) -> Option<(f32, f32, f32, f32)> {
    ctx.input(|i| {
        let vp = i.viewport();
        let pos = vp.outer_rect.map(|r| r.min);
        let size = vp.inner_rect.map(|r| r.size());
        match (pos, size) {
            (Some(p), Some(s)) => Some((p.x, p.y, s.x, s.y)),
            _ => None,
        }
    })
}

/// Push a font size into egui's text styles so the "Taille de police" setting
/// actually changes the rendered text. The other text styles are derived
/// proportionally from the base (body) size.
///
/// egui 0.29 keeps a *separate* `Style` per theme (dark / light) and picks the
/// active one from the system theme, independently of the visuals we force each
/// frame. Writing the sizes into only the active theme's style (as the previous
/// version did) left the other theme — and often the one actually rendered — at
/// the default size, so the setting appeared to do nothing. We therefore rebuild
/// the text styles on **both** styles via `all_styles_mut`.
fn apply_font_size(ctx: &egui::Context, size: f32) {
    use egui::{FontFamily, FontId, TextStyle};
    let base = size.clamp(8.0, 24.0);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new((base * 0.85).round(), FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(base, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(base, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new((base * 0.95).round(), FontFamily::Monospace),
            ),
            (
                TextStyle::Heading,
                FontId::new((base * 1.3).round(), FontFamily::Proportional),
            ),
        ]
        .into();
    });
}

/// A language list showing the endonym of each code; `with_empty` adds the
/// empty position (« no language chosen ») at the head.
fn lang_combo(
    ui: &mut egui::Ui,
    i18n: &I18n,
    id: &str,
    current: &mut String,
    codes: &[String],
    with_empty: bool,
) {
    let selected = if current.is_empty() {
        " ".to_string()
    } else {
        i18n.display_name(current)
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            if with_empty {
                ui.selectable_value(current, String::new(), " ");
            }
            for code in codes {
                let name = i18n.display_name(code);
                ui.selectable_value(current, code.clone(), name);
            }
        });
}

/// The message key of an output mode's short label (toolbar, tab badges,
/// export dialog), E-07: the label follows the interface language.
pub fn output_mode_key(m: OutputMode) -> &'static str {
    match m {
        OutputMode::Strings => "output-mode-strings",
        OutputMode::Hybrid => "output-mode-hybrid",
        OutputMode::OverridePatch => "output-mode-override",
        OutputMode::FullPlugin => "output-mode-full",
        OutputMode::Dsd => "output-mode-dsd",
        OutputMode::Text => "output-text",
        OutputMode::Scripts => "output-scripts",
    }
}

fn describe_outcome(
    i18n: &crate::i18n::I18n,
    outcome: &WriteOutcome,
    path: &std::path::Path,
) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match outcome {
        WriteOutcome::Json => i18n.paren(name, &i18n.t("saved-pivot-json")),
        WriteOutcome::Csv => i18n.paren(name, "CSV"),
        WriteOutcome::Sst => i18n.paren(name, "SST"),
        WriteOutcome::Eet(EetWriteMode::Exact) => i18n.paren(name, &i18n.t("saved-eet-exact")),
        WriteOutcome::Eet(EetWriteMode::Constructed) => {
            i18n.paren(name, &i18n.t("saved-eet-constructed"))
        }
        WriteOutcome::Dsd(n) => i18n.paren(
            name,
            &i18n.t_args("saved-entries", &[("kind", "DSD"), ("n", &n.to_string())]),
        ),
        WriteOutcome::Xml(kind) => i18n.paren(name, &i18n.t_arg("saved-xml", "kind", kind)),
        WriteOutcome::Strings(kind, n) => i18n.paren(
            name,
            &i18n.t_args("saved-entries", &[("kind", kind), ("n", &n.to_string())]),
        ),
    }
}

#[cfg(test)]
mod menu_tests {
    use super::menu_should_close;
    use eframe::egui::{Rect, pos2};

    #[test]
    fn menu_closes_only_when_pointer_leaves() {
        let popup = Rect::from_min_max(pos2(10.0, 30.0), pos2(150.0, 200.0));
        // Pointer inside the popup → keep open.
        assert!(!menu_should_close(popup, None, Some(pos2(50.0, 100.0))));
        // Pointer in the upward bridge band (menu-bar button above popup) → keep open.
        assert!(!menu_should_close(popup, None, Some(pos2(50.0, 10.0))));
        // Pointer far away → close.
        assert!(menu_should_close(popup, None, Some(pos2(400.0, 400.0))));
        // Pointer over an open child sub-menu (« Récents ») to the right → keep open.
        let child = Rect::from_min_max(pos2(150.0, 40.0), pos2(320.0, 160.0));
        assert!(!menu_should_close(
            popup,
            Some(child),
            Some(pos2(250.0, 100.0))
        ));
        // Same point, but no child open → close.
        assert!(menu_should_close(popup, None, Some(pos2(250.0, 100.0))));
        // Pointer position unknown (mouse left the window) → keep open.
        assert!(!menu_should_close(popup, None, None));
    }
}

#[cfg(test)]
mod dict_name_tests {
    use super::{detect_game, game_from_folder, parse_dict_name};
    use crate::games::Game;

    #[test]
    fn game_from_parent_folder() {
        use std::path::Path;
        // xTranslator / XIMOD layout: the folder is the game id.
        assert_eq!(
            game_from_folder(
                Path::new("/x/UserDictionaries/SkyrimSE/MonMod_EN-FR.sst"),
                false
            ),
            Game::SkyrimSE
        );
        assert_eq!(
            game_from_folder(Path::new("/x/mods/Fallout4/MonMod_ENG-FRA.ximdb"), false),
            Game::Fallout4
        );
        // ESP-ESM Translator layout: full names with spaces, mapped by the table.
        assert_eq!(
            game_from_folder(
                Path::new("/x/Bases Persos/Skyrim SE/MonMod_1A2B3C4D.eet"),
                true
            ),
            Game::SkyrimSE
        );
        assert_eq!(
            game_from_folder(
                Path::new("/x/Bases Persos/Fallout New Vegas/MonMod.eet"),
                true
            ),
            Game::FalloutNV
        );
        assert_eq!(
            game_from_folder(Path::new("/x/Bases Persos/Skyrim/M.eet"), true),
            Game::Skyrim
        );
        // An EET name is not a XIMOD id and vice-versa.
        assert_eq!(
            game_from_folder(Path::new("/x/Bases Persos/Skyrim SE/M.eet"), false),
            Game::Unknown
        );
        assert_eq!(
            game_from_folder(Path::new("/x/UserDictionaries/SkyrimSE/M.sst"), true),
            Game::Unknown
        );
        // No parent folder / unrecognized name → Unknown (caller falls back).
        assert_eq!(
            game_from_folder(Path::new("MonMod_EN-FR.eet"), true),
            Game::Unknown
        );
    }

    #[test]
    fn eet_folder_table_round_trips() {
        for (name, g) in super::EET_GAME_FOLDERS {
            assert_eq!(super::game_from_eet_folder(name), *g);
            assert_eq!(super::eet_folder_for(*g), Some(*name));
        }
        assert_eq!(super::eet_folder_for(Game::OblivionRD), None);
    }

    #[test]
    fn detects_game_longest_first() {
        assert_eq!(detect_game("BDD_Fallout3_EN-FR"), Game::Fallout3);
        // "SkyrimSE" must win over the "Skyrim" substring.
        assert_eq!(detect_game("BDD_SkyrimSE_EN-DE"), Game::SkyrimSE);
        assert_eq!(detect_game("MonSuperMod"), Game::Unknown);
    }

    #[test]
    fn parses_game_dictionary_name() {
        let p = parse_dict_name("BDD_Fallout3_EN-FR");
        assert_eq!(p.game, Game::Fallout3);
        assert_eq!(p.langs, Some(("ENG".into(), "FRA".into())));
        // BDD, the game token and the language pair are all stripped.
        assert_eq!(p.core, "");
    }

    #[test]
    fn parses_language_variants() {
        // ISO 639-1 pair with a different target.
        let p = parse_dict_name("BDD_Morrowind_EN-ES");
        assert_eq!(p.game, Game::Morrowind);
        assert_eq!(p.langs, Some(("ENG".into(), "SPA".into())));
        // Already ISO 639-3 in the name.
        let p = parse_dict_name("SomeMod_ENG-FRA");
        assert_eq!(p.langs, Some(("ENG".into(), "FRA".into())));
        assert_eq!(p.core, "SomeMod");
        assert_eq!(p.game, Game::Unknown);
    }

    #[test]
    fn keeps_mod_name_with_separators() {
        let p = parse_dict_name("My_Cool_Mod_EN-FR");
        assert_eq!(p.core, "My_Cool_Mod");
        assert_eq!(p.langs, Some(("ENG".into(), "FRA".into())));
    }

    #[test]
    fn no_language_pair_falls_back() {
        let p = parse_dict_name("JustADictionary");
        assert_eq!(p.langs, None);
        assert_eq!(p.core, "JustADictionary");
        assert_eq!(p.game, Game::Unknown);
    }

    #[test]
    fn splits_strings_filename() {
        use super::split_strings_name;
        use crate::tm::strings::StringsKind;
        let (b, l, k) = split_strings_name("Skyrim_English.STRINGS").unwrap();
        assert_eq!(b, "Skyrim");
        assert_eq!(l, "English");
        assert_eq!(k, StringsKind::Strings);
        // Case-insensitive extension, dashed Creation Club base kept intact.
        let (b, l, k) = split_strings_name("ccbgssse001-fish_french.dlstrings").unwrap();
        assert_eq!(b, "ccbgssse001-fish");
        assert_eq!(l, "french");
        assert_eq!(k, StringsKind::DlStrings);
        assert!(split_strings_name("notastrings.txt").is_none());
        assert!(split_strings_name("nolang.strings").is_none());
    }

    #[test]
    fn plugin_pair_name_matching() {
        use super::plugin_names_match;
        use std::path::Path;
        // Vanilla masters and DLC (same name, different language folder).
        assert!(plugin_names_match(
            Path::new("x/EN/Oblivion.esm"),
            Path::new("x/DE/Oblivion.esm")
        ));
        assert!(plugin_names_match(
            Path::new("EN/Anchorage.esm"),
            Path::new("DE/Anchorage.esm")
        ));
        // Translated mods: a trailing language code is ignored on either side.
        assert!(plugin_names_match(
            Path::new("MonMod.esp"),
            Path::new("MonMod_FR.esp")
        ));
        assert!(plugin_names_match(
            Path::new("MonMod_EN.esp"),
            Path::new("MonMod_de.esp")
        ));
        // A real word that merely looks like it ends in a language is not stripped.
        assert!(plugin_names_match(
            Path::new("Weapons of the German.esp"),
            Path::new("Weapons of the German_FR.esp"),
        ));
        // Genuinely different plugins must be rejected.
        assert!(!plugin_names_match(
            Path::new("MonMod.esp"),
            Path::new("AutreMod_FR.esp")
        ));
        assert!(!plugin_names_match(
            Path::new("Anchorage.esm"),
            Path::new("PointLookout.esm")
        ));
    }

    #[test]
    fn strips_eet_hash_from_mod_name() {
        // ESP-ESM Translator appends a hex hash to mod-dictionary names.
        let p = parse_dict_name("MonMod_2A3F5C1B_EN-FR");
        assert_eq!(p.core, "MonMod");
        assert_eq!(p.langs, Some(("ENG".into(), "FRA".into())));
        // Longer hex hash, already ISO 639-3 in the name.
        let p = parse_dict_name("MyMod_deadbeefcafe_ENG-FRA");
        assert_eq!(p.core, "MyMod");
        // A short/non-hex token is kept (e.g. a year inside a mod name).
        let p = parse_dict_name("Mod2024_EN-FR");
        assert_eq!(p.core, "Mod2024");
    }
}

/// The clipboard's text, if the system clipboard is reachable (M-01, paste
/// from the row menu). `None` when empty or unavailable (headless, Wayland
/// without the data-control protocol…).
fn read_clipboard() -> Option<String> {
    crate::slow_io("clipboard", || {
        arboard::Clipboard::new()
            .ok()?
            .get_text()
            .ok()
            .filter(|t| !t.is_empty())
    })
}

/// The clipboard for the row menu's Coller entries, read on a thread of
/// its own so a slow clipboard owner never stalls the interface (Windows
/// renders some formats on demand): the last text read, refreshed at most
/// once a second while a popup is open.
#[derive(Default)]
struct ClipboardWatch {
    text: Option<String>,
    rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    asked: Option<std::time::Instant>,
}

impl ClipboardWatch {
    fn current(&mut self, popup_open: bool) -> Option<String> {
        if let Some(rx) = &self.rx
            && let Ok(t) = rx.try_recv()
        {
            self.text = t;
            self.rx = None;
        }
        if popup_open
            && self.rx.is_none()
            && self
                .asked
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1))
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.rx = Some(rx);
            self.asked = Some(std::time::Instant::now());
            std::thread::spawn(move || {
                let _ = tx.send(read_clipboard());
            });
        }
        self.text.clone()
    }
}

#[cfg(test)]
mod mod_dict_tests {
    use super::XimodApp;
    use crate::status::{self, Status};
    use crate::tm::TmEntry;

    fn e(formid: u32, edid: &str, trans: &str, st: Option<Status>, unused: bool) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: "WEAP".into(),
            field_sig: "FULL".into(),
            formid,
            edid: edid.into(),
            source: "Sword".into(),
            trans: trans.into(),
            unused,
            ..Default::default()
        };
        if let Some(st) = st {
            status::set(&mut e, st);
        }
        e
    }

    /// T-B03-02: the working dictionary takes every line that is not « À
    /// traduire » — Validé, En attente, Ignoré without a text — with its
    /// status, and leaves out the untranslated and the never-shown lines.
    #[test]
    fn mod_dict_rows_keep_the_statuses_and_skip_the_rest() {
        let entries = vec![
            e(1, "A", "Épée", Some(Status::Validated), false),
            e(2, "B", "", None, false),
            e(3, "C", "Épé", Some(Status::Pending), false),
            e(4, "D", "", Some(Status::Ignored), false),
            e(5, "E", "Épée", Some(Status::Validated), true),
        ];
        let rows = XimodApp::mod_dict_rows(&entries);
        let got: Vec<(u32, &str)> = rows
            .iter()
            .map(|r| (r.formid, r.status.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(got, vec![(1, "validated"), (3, "pending"), (4, "ignored")]);
        assert!(
            rows[2].trans.is_empty(),
            "an Ignoré line keeps its empty text"
        );
    }

    /// C-08: the dictionary keeps the work under the VMAD switch, not the
    /// lock; a line locked by a rule is kept as Ignoré.
    #[test]
    fn mod_dict_rows_see_through_the_vmad_switch() {
        let mut v = e(7, "Q", "Aide", Some(Status::Validated), false);
        v.field_sig = "VMAD".into();
        crate::vmad_lock::lock_file(&mut v);
        let mut w = e(8, "R", "", None, false);
        w.field_sig = "VMAD".into();
        crate::vmad_lock::lock_file(&mut w);
        let rows = XimodApp::mod_dict_rows(&[v, w]);
        let got: Vec<(u32, &str)> = rows
            .iter()
            .map(|r| (r.formid, r.status.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(got, vec![(7, "validated")]);
    }
}

/// The two sections of the interface-language list (E-07): the languages of
/// the country that have an interface file, then every interface language
/// sorted by endonym. Items are `(code, endonym)`.
pub(crate) type LangItems = Vec<(String, String)>;

pub(crate) fn language_sections(
    country: &[String],
    available: &[String],
    name: impl Fn(&str) -> String,
) -> (LangItems, LangItems) {
    let firsts: LangItems = country
        .iter()
        .filter(|c| available.contains(c))
        .map(|c| (c.clone(), name(c)))
        .collect();
    let mut all: LangItems = available.iter().map(|c| (c.clone(), name(c))).collect();
    all.sort_by_key(|(_, n)| sort_key(n));
    (firsts, all)
}

/// Sort key of an endonym: lower case, common Latin diacritics folded, so
/// that « Čeština » sorts with C and « Íslenska » with I.
fn sort_key(name: &str) -> String {
    const FROM: &str = "àáâãäåāăąçćčďđèéêëēėęěìíîïīįłľĺñńňòóôõöøōőřŕšśşșťţțùúûüūůűųýÿžźż";
    const TO: &str = "aaaaaaaaacccddeeeeeeeeiiiiiilllnnnoooooooorrsssstttuuuuuuuuyyzzz";
    name.to_lowercase()
        .chars()
        .map(|c| match FROM.chars().position(|f| f == c) {
            Some(i) => TO.chars().nth(i).unwrap_or(c),
            None => c,
        })
        .collect()
}

#[cfg(test)]
mod language_sections_tests {
    use super::language_sections;

    #[test]
    fn country_first_then_all_by_endonym() {
        let avail: Vec<String> = ["deu", "fra", "eng", "jpn", "ces"]
            .map(String::from)
            .to_vec();
        let country: Vec<String> = ["fra", "oci"].map(String::from).to_vec();
        let name = |c: &str| {
            match c {
                "deu" => "Deutsch",
                "fra" => "Français",
                "eng" => "English",
                "jpn" => "日本語",
                "ces" => "Čeština",
                _ => c,
            }
            .to_string()
        };
        let (first, all) = language_sections(&country, &avail, name);
        assert_eq!(first, vec![("fra".to_string(), "Français".to_string())]);
        let codes: Vec<&str> = all.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, ["ces", "deu", "eng", "fra", "jpn"]);
        let (first, all) = language_sections(&[], &avail, name);
        assert!(first.is_empty());
        assert_eq!(all.len(), 5);
    }
}
