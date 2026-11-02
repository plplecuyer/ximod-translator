//! The translation grid: a virtualized table of translation-memory entries.
//!
//! Rows are colored by status across every column, the source/translation cells
//! wrap long text (so the translation column can never be pushed off-screen),
//! and the EDID / Source / Traduction columns grow and shrink with the window
//! width. Virtualization is preserved via `heterogeneous_rows`, which renders
//! only the visible rows even though each row has its own (wrapped) height.

use eframe::egui::{self, Color32};
use egui_extras::{Column, TableBuilder};

use crate::status::{self, Status};
use crate::tm::TmEntry;
use crate::undo::EntryState;

/// The pastel colors used per status, indexed by [`Status::index`].
pub type StatusColors = [(u8, u8, u8); status::COUNT];

/// The color for an entry given the configured (customizable) palette.
pub fn status_color(e: &TmEntry, colors: &StatusColors) -> Color32 {
    let (r, g, b) = colors[status::derive(e).index()];
    Color32::from_rgb(r, g, b)
}

/// The Ignoré colour (lines never shown in game, whatever their status).
fn ignored_color(colors: &StatusColors) -> Color32 {
    let (r, g, b) = colors[Status::Ignored.index()];
    Color32::from_rgb(r, g, b)
}

/// A readable text/glyph color for a pastel background (dark, theme-independent).
fn on_pastel() -> Color32 {
    Color32::from_rgb(35, 35, 35)
}

/// Fixed column widths (status / status buttons / GRUP / Champ), in points.
const ST_W: f32 = 40.0;
/// Four small buttons and their gaps (A-04).
const BTN_W: f32 = 96.0;
const GRUP_W: f32 = 66.0;
const CHAMP_W: f32 = 64.0;

/// The optional columns of the grid (Options ▸ Colonnes, D-42).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OptColumns {
    /// The four status buttons right of St (A-04).
    pub status_buttons: bool,
    pub review: bool,
    pub provenance: bool,
    pub comment: bool,
    /// The Fichier column of the « Tous les mods » tab (B-25).
    pub file: bool,
}

/// Width of the Fichier column (B-25).
pub const FILE_W: f32 = 150.0;

/// Widths of the flexible columns, in points.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Widths {
    edid: f32,
    src: f32,
    tgt: f32,
    review: f32,
    prov: f32,
    com: f32,
}

/// Compute the flexible column widths (EDID, Source, Traduction, and the
/// optional Provenance and Commentaire columns, B-09 and C-03) from the
/// available width, so they grow/shrink with the window.
fn flex_widths(avail: f32, gap: f32, opt: OptColumns) -> Widths {
    let cols =
        6.0 + opt.review as u8 as f32 + opt.provenance as u8 as f32 + opt.comment as u8 as f32;
    let buttons = if opt.status_buttons { BTN_W + gap } else { 0.0 };
    let file = if opt.file { FILE_W + gap } else { 0.0 };
    let fixed = ST_W + buttons + file + GRUP_W + CHAMP_W + gap * cols + 16.0;
    let flex = (avail - fixed).max(280.0);
    let edid = (flex * 0.20).clamp(80.0, 220.0);
    let com = if opt.comment {
        (flex * 0.22).clamp(120.0, 360.0)
    } else {
        0.0
    };
    let prov = if opt.provenance {
        (flex * 0.17).clamp(120.0, 240.0)
    } else {
        0.0
    };
    let rest = (flex - edid - com - prov).max(180.0);
    // The review shares the text width with the source and the translation.
    let parts = if opt.review { 3.0 } else { 2.0 };
    let src = (rest / parts).floor();
    let review = if opt.review { src } else { 0.0 };
    let tgt = (rest - src - review).floor();
    Widths {
        edid,
        src,
        tgt,
        review,
        prov,
        com,
    }
}

/// Index of the Relecture, Provenance and Commentaire titles in
/// [`GridLabels::headers`].
pub const H_REVIEW: usize = 6;
pub const H_PROV: usize = 7;
pub const H_COMMENT: usize = 8;
/// The Fichier title (B-25).
pub const H_FILE: usize = 9;

/// The four buttons of the status column (A-04), in their order: Validé,
/// En attente, Personnalisé, Ignoré.
pub const STATUS_BUTTONS: [Status; 4] = [
    Status::Validated,
    Status::Pending,
    Status::Custom,
    Status::Ignored,
];

/// The texts of the status buttons column (A-04).
#[derive(Debug, Clone, Default)]
pub struct StatusButtonLabels {
    /// « Valider — F5 »… per button of [`STATUS_BUTTONS`].
    pub tips: [String; 4],
    /// Why a greyed button is greyed (« Une ligne sans traduction ne peut
    /// pas être Validé. »), per button.
    pub needs_trans: [String; 4],
}

/// The filter row under the headers (A-10), while it shows: the texts
/// being typed, the errors of the applied ones, and the ≠ of the Relecture
/// cell.
pub struct FilterRow<'a> {
    pub filters: &'a mut crate::filter::ColumnFilters,
    /// The localized error of each applied field, for the red text and
    /// its tooltip.
    pub errors: &'a [Option<String>; crate::filter::N_COLS],
    /// « Seulement les lignes dont la relecture diffère » (A-17), in the
    /// Relecture cell.
    pub review_diff: &'a mut bool,
    pub review_diff_tip: &'a str,
    /// The eraser, left of the row.
    pub clear_tip: &'a str,
}

/// The lines a status button of line `idx` works on (R-A04-02/04): the
/// selection when the line is in it, the line alone otherwise.
pub fn button_targets(selection: &Selection, idx: usize) -> Vec<usize> {
    if selection.rows.contains(&idx) {
        selection.targets()
    } else {
        vec![idx]
    }
}

/// The review text in its cell: a dark blue when it differs from the
/// translation, grey when it is the same (R-A17-01) — readable on every
/// status pastel.
fn review_color(differs: bool) -> Color32 {
    if differs {
        Color32::from_rgb(20, 60, 170)
    } else {
        Color32::from_rgb(115, 115, 115)
    }
}

/// A word-wrap on the glyph advances of a font, for the height of a text
/// at a width without laying it out: the advance of each character is
/// read once (an array for ASCII, a map for the rest).
struct Measurer<'a> {
    fonts: &'a egui::text::Fonts,
    font: &'a egui::FontId,
    row: f32,
    ascii: [f32; 128],
    other: std::collections::HashMap<char, f32>,
}

impl<'a> Measurer<'a> {
    fn new(fonts: &'a egui::text::Fonts, font: &'a egui::FontId) -> Self {
        Measurer {
            fonts,
            font,
            row: fonts.row_height(font),
            ascii: [-1.0; 128],
            other: std::collections::HashMap::new(),
        }
    }

    fn advance(&mut self, c: char) -> f32 {
        if (c as u32) < 128 {
            let i = c as usize;
            if self.ascii[i] < 0.0 {
                self.ascii[i] = self.fonts.glyph_width(self.font, c);
            }
            self.ascii[i]
        } else {
            let (fonts, font) = (self.fonts, self.font);
            *self
                .other
                .entry(c)
                .or_insert_with(|| fonts.glyph_width(font, c))
        }
    }

    /// The number of rows a text takes when wrapped at `width`: a break
    /// at the last space of the row, inside a word that is wider than
    /// the row (egui's rule).
    fn rows(&mut self, text: &str, width: f32) -> usize {
        let width = width.max(1.0);
        let mut rows = 1usize;
        let mut x = 0.0f32; // the row so far
        let mut word = 0.0f32; // the current word, part of `x`
        for c in text.chars() {
            if c == '\n' {
                rows += 1;
                x = 0.0;
                word = 0.0;
                continue;
            }
            let a = self.advance(c);
            if x + a > width && x > 0.0 {
                rows += 1;
                if c.is_whitespace() {
                    // The row ends on the space: it is not carried over.
                    x = 0.0;
                    word = 0.0;
                    continue;
                } else if word > 0.0 && word < x {
                    // The word moves to the next row.
                    x = word;
                } else {
                    x = 0.0;
                    word = 0.0;
                }
            }
            x += a;
            if c.is_whitespace() {
                word = 0.0;
            } else {
                word += a;
            }
        }
        rows
    }

    /// The height of a text wrapped at `width` (what a galley would give).
    fn height(&mut self, text: &str, width: f32) -> f32 {
        if text.is_empty() {
            return self.font.size + 4.0;
        }
        self.rows(text, width) as f32 * self.row
    }
}

/// The row heights of the grid, kept from one frame to the next: laying
/// out the source, the translation and the review of every visible line
/// is the cost of a frame on a large file (50 ms a frame with 50 000
/// lines, 800 ms when the column widths change). The heights are
/// recomputed when the document, the view, the widths or the font change;
/// when only the content moved (a cell typed in, an operation), only the
/// lines whose text changed are laid out again.
#[derive(Default)]
pub struct HeightCache {
    key: Option<u64>,
    revision: u64,
    /// Per visible row: a hash of its texts and its height.
    sigs: Vec<u64>,
    heights: Vec<f32>,
}

impl HeightCache {
    /// The heights of `visible`, from the cache or recomputed.
    #[allow(clippy::too_many_arguments)]
    fn heights(
        &mut self,
        ui: &egui::Ui,
        entries: &[TmEntry],
        visible: &[usize],
        doc: (u64, u64),
        w: &Widths,
        review: bool,
        font: &egui::FontId,
        pad: f32,
    ) -> Vec<f32> {
        use std::hash::{Hash, Hasher};
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            doc.0.hash(&mut h);
            visible.hash(&mut h);
            w.src.to_bits().hash(&mut h);
            w.tgt.to_bits().hash(&mut h);
            w.review.to_bits().hash(&mut h);
            review.hash(&mut h);
            font.hash(&mut h);
            h.finish()
        };
        let sig = |e: &TmEntry| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            e.source.hash(&mut h);
            e.trans.hash(&mut h);
            e.review.hash(&mut h);
            h.finish()
        };
        let full = self.key != Some(key) || self.heights.len() != visible.len();
        let moved = self.revision != doc.1;
        if !full && !moved {
            return self.heights.clone();
        }
        let started = std::time::Instant::now();
        // The heights come from a word-wrap on the glyph advances (the
        // measure egui lays the text out with), without building a galley:
        // ten times cheaper than a layout, exact enough for a row height.
        ui.fonts(|f| {
            let mut m = Measurer::new(f, font);
            let height = |m: &mut Measurer<'_>, e: &TmEntry| {
                let hs = m.height(&e.source, w.src);
                // The editors wrap inside their margin (4 points a side, 2
                // above and below): their text is measured at that width.
                let ht = m.height(&e.trans, w.tgt - 8.0) + 4.0;
                let hr = if review {
                    m.height(&e.review, w.review - 8.0) + 4.0
                } else {
                    0.0
                };
                hs.max(ht).max(hr).max(font.size) + pad
            };
            if full {
                self.key = Some(key);
                self.revision = doc.1;
                self.sigs = visible.iter().map(|&i| sig(&entries[i])).collect();
                self.heights = visible
                    .iter()
                    .map(|&i| height(&mut m, &entries[i]))
                    .collect();
                tracing::debug!(
                    "grid heights: {} rows in {} ms",
                    visible.len(),
                    started.elapsed().as_millis()
                );
            } else {
                self.revision = doc.1;
                for (n, &i) in visible.iter().enumerate() {
                    let s = sig(&entries[i]);
                    if self.sigs[n] != s {
                        self.sigs[n] = s;
                        self.heights[n] = height(&mut m, &entries[i]);
                    }
                }
            }
        });
        self.heights.clone()
    }
}

/// Height of a text laid out (wrapped) at `width`, cached by egui's galley cache.
pub fn wrapped_height(ui: &egui::Ui, text: &str, width: f32, font: &egui::FontId) -> f32 {
    if text.is_empty() {
        return font.size + 4.0;
    }
    let galley = ui.fonts(|f| {
        f.layout(
            text.to_owned(),
            font.clone(),
            Color32::BLACK,
            width.max(1.0),
        )
    });
    galley.size().y
}

/// Draw the grid. `visible` is the list of indices into `entries` to display
/// (already filtered by the caller). `colors` is the status palette.
///
/// Returns `Some(idx)` — the index (into `entries`) of the row whose translation
/// was edited this frame — so the caller can propagate that translation to the
/// hidden duplicates in the unique-lines view.
/// Translated labels the grid needs: status legends (context menu), the book
/// preview entry and the six column headers (engine-dependent, S-05).
/// Texts of the row context menu (M-01), built once per frame by the
/// application from the i18n bundle.
#[derive(Debug, Default, Clone)]
pub struct MenuLabels {
    pub edit: String,
    pub copy_source: String,
    pub copy_trans: String,
    pub paste_one: String,
    /// « Coller dans les { $n } lignes sélectionnées » — `{n}` replaced here.
    pub paste_many: String,
    pub paste_disabled: String,
    pub trans_eq_source: String,
    pub status_one: String,
    pub status_many: String,
    pub validate_next: String,
    /// « Filtrer sur cet enregistrement ({formid}) ».
    pub filter_record: String,
    pub filter_source: String,
    pub filter_speaker: String,
    pub book: String,
    pub book_disabled: String,
    /// Dictionnaires (lot 3).
    pub add_personal: String,
    pub key_personal: String,
    pub apply_dicts: String,
    pub propagate: String,
    pub key_propagate: String,
    /// Traduire en ligne (D-01).
    pub online: String,
    pub key_online: String,
    /// The Relecture sub-menu (A-17) and its shortcut (Ctrl+Maj+R).
    pub review: String,
    pub review_accept: String,
    pub review_from_trans: String,
    pub review_clear: String,
    pub key_review: String,
    /// The Historique sub-menu (A-18) and its shortcut (Ctrl+Alt+Z).
    pub history: String,
    pub history_previous: String,
    pub history_show: String,
    pub key_history: String,
    /// Appliquer aux fichiers ouverts (B-25), offered with two files open.
    pub propagate_files: Option<String>,
    /// Concordance… (A-20) and its shortcut (Ctrl+K).
    pub concord: String,
    pub key_concord: String,
    /// Shortcut texts shown at the right of the entries.
    pub key_edit: String,
    pub key_copy: String,
    pub key_paste: String,
    pub key_eq: String,
    pub key_status: [String; 4],
    pub key_validate_next: String,
    /// Text of the clipboard, if any (read once per frame when a menu is open).
    pub clipboard: Option<String>,
}

/// What a menu entry asks the application to do (M-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Edit,
    CopySource,
    CopyTrans,
    PasteOne,
    PasteMany,
    TransEqSource,
    Status(Status),
    ValidateNext,
    FilterRecord,
    FilterSource,
    Book,
    /// Ajouter à mon dictionnaire personnel (B-04).
    AddPersonal,
    /// Appliquer les dictionnaires à la sélection (B-06).
    ApplyDicts,
    /// Appliquer à toutes les lignes de même source… (A-12).
    Propagate,
    /// Filtrer sur ce PNJ (C-02).
    FilterSpeaker,
    /// Traduire en ligne (D-01).
    Online,
    /// Relecture ▸ Traduction = Relecture, Relecture = Traduction,
    /// Effacer la relecture (A-17).
    ReviewAccept,
    ReviewFromTrans,
    ReviewClear,
    /// Historique ▸ Revenir à la traduction précédente, Afficher
    /// l'historique de la ligne (A-18).
    HistoryPrevious,
    HistoryShow,
    /// Concordance… (A-20).
    Concordance,
    /// Appliquer aux fichiers ouverts (B-25).
    PropagateFiles,
}

pub struct GridLabels<'a> {
    /// Game of the document: field limits and encoding of the counter (A-05).
    pub game: crate::games::Game,
    /// The page of code the document is written in (D-09): the bytes of
    /// the length badges.
    pub page: crate::codepage::Page,
    /// Tooltips of the length badges, with `{chars}` `{max}` `{bytes}`
    /// `{field}` placeholders.
    pub len_over_tip: &'a str,
    pub len_newline_tip: &'a str,
    /// The bytes unit of the length badge (« o », « B », E-07).
    pub bytes_unit: &'a str,
    /// Alias / number anomaly texts (A-07): missing tag, unknown tag,
    /// missing number, unknown number (`{tag}` / `{n}` placeholders), order.
    pub alias_tips: &'a [String; 5],
    /// Special characters shown as glyphs (A-11, R-A11-07).
    pub show_special: bool,
    /// Lines in anomaly after the last check (D-05): the tooltip per line.
    pub anomalies: &'a std::collections::HashMap<usize, String>,
    /// « Statut — provenance » of a line (B-09), for the status cell.
    pub prov_tip: &'a dyn Fn(&TmEntry) -> Option<String>,
    /// The Provenance column (B-09): the short label of a line; `None`
    /// hides the column.
    pub prov_short: Option<&'a dyn Fn(&TmEntry) -> String>,
    pub menu: &'a MenuLabels,
    pub legends: &'a [String; status::COUNT],
    pub book_label: &'a str,
    /// The column titles: the six fixed ones, then Relecture
    /// ([`H_REVIEW`]), Provenance ([`H_PROV`]) and Commentaire
    /// ([`H_COMMENT`]).
    pub headers: &'a [String; 10],
    /// The Fichier column (« Tous les mods », B-25).
    pub file_col: bool,
    /// The Relecture column (A-17).
    pub review: bool,
    /// The status buttons column (A-04); `None` hides it.
    pub status_buttons: Option<&'a StatusButtonLabels>,
    /// Shown in the empty body when filters leave no line (R-A10-06).
    pub empty_msg: Option<&'a str>,
    /// The entries of Options ▸ Colonnes (title, shown), offered again on
    /// a right-click of the header (D-42).
    pub col_menu: &'a [(String, bool)],
    /// Annuler / Rétablir entries of the context menu (A-01) and whether
    /// they are available.
    pub undo_label: &'a str,
    pub redo_label: &'a str,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Tooltip of a line « never shown in game » (field mode unused).
    pub unused_tip: &'a str,
    /// Tooltip of a line held by its file's VMAD switch (C-08).
    pub vmad_off_tip: &'a str,
    /// Tooltip of the Champ cell: the script and the property of a VMAD
    /// line (C-08).
    pub field_tip: &'a dyn Fn(&TmEntry) -> Option<String>,
    /// The Commentaire column (C-03): the text of a line, generated and
    /// cached by the application; `None` hides the column.
    pub comment: Option<&'a CommentFn<'a>>,
}

/// The comment of a line (index, entry) → text, C-03.
pub type CommentFn<'a> = dyn Fn(usize, &TmEntry) -> std::sync::Arc<str> + 'a;

/// What happened in the grid this frame (A-01): the application turns it
/// into undoable operations rather than the grid acting silently.
#[derive(Debug, Default)]
pub struct GridOutput {
    /// The translation cell whose text changed this frame.
    pub edited: Option<usize>,
    /// A row was clicked (entry index) with the modifiers held: the
    /// application updates the selection (A-03).
    pub row_clicked: Option<(usize, egui::Modifiers)>,
    /// A row was right-clicked: it joins the selection if it is not in it.
    pub row_right_clicked: Option<usize>,
    /// A row was double-clicked outside its translation cell: the edit
    /// window opens on it (R-A02-01).
    pub row_double_clicked: Option<usize>,
    /// A context-menu entry was chosen, on this row.
    pub menu_action: Option<(usize, MenuAction)>,
    /// The focus request was honoured (the row was drawn and focused).
    pub focus_applied: bool,
    /// The translation cell that gained / lost keyboard focus.
    pub focus_gained: Option<usize>,
    pub focus_lost: Option<usize>,
    /// A status chosen from the context menu: the line, its state before,
    /// and the new status (already applied).
    pub status_set: Option<(usize, EntryState, Status)>,
    pub undo_requested: bool,
    pub redo_requested: bool,
    /// An entry of the header's column menu was toggled (its index in
    /// [`GridLabels::col_menu`]).
    pub column_toggled: Option<usize>,
    /// The review cell (A-17) that changed, gained or lost the focus.
    pub review_edited: Option<usize>,
    pub review_focus_gained: Option<usize>,
    pub review_focus_lost: Option<usize>,
    /// A status button of the column (A-04) was clicked: the line and the
    /// status asked; the application applies it to the line or its
    /// selection.
    pub status_button: Option<(usize, Status)>,
    /// A field of the filter row (A-10) changed this frame / Entrée was
    /// pressed in one / the eraser or the ≠ toggle was used.
    pub filter_typed: bool,
    pub filter_enter: bool,
    pub filter_now: bool,
}

/// The lines selected in the grid (entry indices) and the current line.
#[derive(Debug, Default, Clone)]
pub struct Selection {
    pub rows: std::collections::BTreeSet<usize>,
    /// The line the keyboard works on (also the end of a Shift range).
    pub cursor: Option<usize>,
    /// Start of a Shift range.
    pub anchor: Option<usize>,
}

impl Selection {
    pub fn set_single(&mut self, idx: usize) {
        self.rows.clear();
        self.rows.insert(idx);
        self.cursor = Some(idx);
        self.anchor = Some(idx);
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.cursor = None;
        self.anchor = None;
    }

    /// Apply a click with modifiers, in the order of `visible`.
    pub fn click(&mut self, idx: usize, mods: egui::Modifiers, visible: &[usize]) {
        if mods.shift {
            let anchor = self.anchor.unwrap_or(idx);
            let (a, b) = (
                visible.iter().position(|v| *v == anchor),
                visible.iter().position(|v| *v == idx),
            );
            if let (Some(a), Some(b)) = (a, b) {
                if !mods.command {
                    self.rows.clear();
                }
                for &v in &visible[a.min(b)..=a.max(b)] {
                    self.rows.insert(v);
                }
                self.cursor = Some(idx);
                return;
            }
        }
        if mods.command {
            if !self.rows.remove(&idx) {
                self.rows.insert(idx);
            }
            self.cursor = Some(idx);
            self.anchor = Some(idx);
            return;
        }
        self.set_single(idx);
    }

    /// The selected lines, or the cursor line alone.
    pub fn targets(&self) -> Vec<usize> {
        if self.rows.is_empty() {
            self.cursor.into_iter().collect()
        } else {
            self.rows.iter().copied().collect()
        }
    }
}

/// The line after the last target (dir > 0) or before the first (dir < 0)
/// in the order of `visible`; `None` at the edge of the view.
pub fn next_in_view(visible: &[usize], targets: &[usize], dir: i32) -> Option<usize> {
    let edge = if dir > 0 {
        targets.iter().max().copied()
    } else {
        targets.iter().min().copied()
    }?;
    let pos = visible.iter().position(|v| *v == edge)?;
    let next = if dir > 0 {
        pos + 1
    } else {
        pos.checked_sub(1)?
    };
    visible.get(next).copied()
}

/// A selected row: its colour pulled toward the selection colour.
/// A navigation key of the grid (F-04), active while no field has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

impl NavKey {
    /// Consume the navigation key pressed this frame, if any.
    pub fn consume(i: &mut egui::InputState) -> Option<NavKey> {
        use egui::{Key, Modifiers};
        let pairs = [
            (Key::ArrowUp, NavKey::Up),
            (Key::ArrowDown, NavKey::Down),
            (Key::PageUp, NavKey::PageUp),
            (Key::PageDown, NavKey::PageDown),
            (Key::Home, NavKey::Home),
            (Key::End, NavKey::End),
        ];
        pairs
            .iter()
            .find(|(k, _)| i.consume_key(Modifiers::NONE, *k))
            .map(|(_, n)| *n)
    }
}

/// The line a navigation key leads to, in the view order: the entry index
/// of the new line, `None` when the view is empty. Without a current line,
/// Down / PageDown / End land on the first / last line, Up / PageUp / Home
/// on the first.
pub fn nav_target(
    visible: &[usize],
    current: Option<usize>,
    key: NavKey,
    page: usize,
) -> Option<usize> {
    if visible.is_empty() {
        return None;
    }
    let last = visible.len() - 1;
    let page = page.max(1);
    let pos = current.and_then(|c| visible.iter().position(|&v| v == c));
    let new = match (key, pos) {
        (NavKey::Home, _) | (NavKey::Up, None) | (NavKey::PageUp, None) => 0,
        (NavKey::End, _) => last,
        (NavKey::Down, None) | (NavKey::PageDown, None) => 0,
        (NavKey::Up, Some(p)) => p.saturating_sub(1),
        (NavKey::Down, Some(p)) => (p + 1).min(last),
        (NavKey::PageUp, Some(p)) => p.saturating_sub(page),
        (NavKey::PageDown, Some(p)) => (p + page).min(last),
    };
    Some(visible[new])
}

/// Navigation keys of any table (F-04, shared with Architect): the new row
/// position among `n`, or `None` when nothing moved. Silent while a text
/// field is being typed in (`editing`).
pub fn table_nav_keys(
    ctx: &egui::Context,
    pos: Option<usize>,
    n: usize,
    page: usize,
    editing: bool,
) -> Option<usize> {
    if n == 0 || editing {
        return None;
    }
    let key = ctx.input_mut(NavKey::consume)?;
    let visible: Vec<usize> = (0..n).collect();
    nav_target(&visible, pos, key, page).filter(|&p| pos != Some(p))
}

fn selected_tint(base: Color32, sel: Color32) -> Color32 {
    let t = 0.35;
    let mix = |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t).round() as u8;
    Color32::from_rgb(
        mix(base.r(), sel.r()),
        mix(base.g(), sel.g()),
        mix(base.b(), sel.b()),
    )
}

/// What the row menu needs to know about its line (M-01).
#[derive(Debug, Clone, Copy)]
pub struct MenuRow {
    pub idx: usize,
    pub cur: Status,
    pub unused: bool,
    pub formid: u32,
    pub has_trans: bool,
    /// The line has a review (A-17).
    pub has_review: bool,
    pub is_book: bool,
    /// Lines the menu acts on (1 when the line is outside the selection).
    pub sel_count: usize,
    /// A dialogue reply (INFO): « Filtrer sur ce PNJ » applies.
    pub is_info: bool,
}

/// The row context menu (M-01), on a grid cell or on the Translation area
/// of the edit window (A-02). `full` carries the Annuler / Rétablir labels
/// and availability; `None` restricts the menu to the Édition and Statut
/// groups (R-A02-07). The chosen entry lands in `out`.
pub fn row_menu(
    ui: &mut egui::Ui,
    out: &mut GridOutput,
    row: &MenuRow,
    menu: &MenuLabels,
    legends: &[String; status::COUNT],
    colors: &StatusColors,
    full: Option<(&str, &str, bool, bool)>,
) {
    let MenuRow {
        idx,
        cur,
        unused,
        formid,
        has_trans,
        has_review,
        is_book,
        sel_count,
        is_info,
    } = *row;
    let clip_ok = menu.clipboard.as_deref().is_some_and(|t| !t.is_empty());
    let key = |ui: &mut egui::Ui, label: &str, k: &str, enabled: bool, tip: &str| -> bool {
        let resp = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(k));
        let resp = if enabled || tip.is_empty() {
            resp
        } else {
            resp.on_disabled_hover_text(tip)
        };
        resp.clicked()
    };
    let mut pick: Option<MenuAction> = None;
    // Édition
    if key(ui, &menu.edit, &menu.key_edit, !unused, "") {
        pick = Some(MenuAction::Edit);
    }
    if key(ui, &menu.copy_source, "", true, "") {
        pick = Some(MenuAction::CopySource);
    }
    if key(ui, &menu.copy_trans, &menu.key_copy, has_trans, "") {
        pick = Some(MenuAction::CopyTrans);
    }
    if sel_count > 1 {
        let label = menu.paste_many.replace("{n}", &sel_count.to_string());
        if key(
            ui,
            &label,
            &menu.key_paste,
            clip_ok && !unused,
            &menu.paste_disabled,
        ) {
            pick = Some(MenuAction::PasteMany);
        }
    } else if key(
        ui,
        &menu.paste_one,
        &menu.key_paste,
        clip_ok && !unused,
        &menu.paste_disabled,
    ) {
        pick = Some(MenuAction::PasteOne);
    }
    if key(ui, &menu.trans_eq_source, &menu.key_eq, !unused, "") {
        pick = Some(MenuAction::TransEqSource);
    }
    // Relecture (A-17): on the line or the selection.
    ui.add_enabled_ui(!unused, |ui| {
        ui.menu_button(&menu.review, |ui| {
            if key(ui, &menu.review_accept, &menu.key_review, has_review, "") {
                pick = Some(MenuAction::ReviewAccept);
            }
            if key(ui, &menu.review_from_trans, "", has_trans, "") {
                pick = Some(MenuAction::ReviewFromTrans);
            }
            if key(ui, &menu.review_clear, "", has_review, "") {
                pick = Some(MenuAction::ReviewClear);
            }
        });
        // Historique (A-18).
        ui.menu_button(&menu.history, |ui| {
            if key(ui, &menu.history_previous, &menu.key_history, true, "") {
                pick = Some(MenuAction::HistoryPrevious);
            }
            if key(ui, &menu.history_show, "", true, "") {
                pick = Some(MenuAction::HistoryShow);
            }
        });
    });
    ui.separator();
    // Statut
    let head = if sel_count > 1 {
        menu.status_many.replace("{n}", &sel_count.to_string())
    } else {
        menu.status_one.clone()
    };
    ui.label(egui::RichText::new(head).weak().small());
    for (k, st) in [
        Status::Validated,
        Status::Pending,
        Status::Custom,
        Status::Ignored,
    ]
    .iter()
    .enumerate()
    {
        let (r, g, b) = colors[st.index()];
        let mark = if *st == cur {
            format!("{} ", crate::icons::DOT)
        } else {
            "   ".to_string()
        };
        // The legend on its pastel, dark text: readable whatever the theme.
        let text = egui::RichText::new(format!(" {mark}{} ", legends[st.index()]))
            .background_color(Color32::from_rgb(r, g, b))
            .color(on_pastel());
        let resp = ui.add_enabled(
            !unused,
            egui::Button::new(text).shortcut_text(&menu.key_status[k]),
        );
        if resp.clicked() {
            pick = Some(MenuAction::Status(*st));
        }
    }
    if key(
        ui,
        &menu.validate_next,
        &menu.key_validate_next,
        !unused,
        "",
    ) {
        pick = Some(MenuAction::ValidateNext);
    }
    // Dictionnaires (lot 3): on the grid and in the edit window.
    ui.separator();
    if full.is_some() && key(ui, &menu.apply_dicts, "", !unused, "") {
        pick = Some(MenuAction::ApplyDicts);
    }
    if key(
        ui,
        &menu.add_personal,
        &menu.key_personal,
        !unused && has_trans,
        "",
    ) {
        pick = Some(MenuAction::AddPersonal);
    }
    if key(
        ui,
        &menu.propagate,
        &menu.key_propagate,
        !unused && has_trans,
        "",
    ) {
        pick = Some(MenuAction::Propagate);
    }
    if let Some(label) = &menu.propagate_files
        && key(ui, label, "", !unused && has_trans, "")
    {
        pick = Some(MenuAction::PropagateFiles);
    }
    if key(ui, &menu.online, &menu.key_online, !unused, "") {
        pick = Some(MenuAction::Online);
    }
    if key(ui, &menu.concord, &menu.key_concord, true, "") {
        pick = Some(MenuAction::Concordance);
    }
    if let Some((undo_label, redo_label, can_undo, can_redo)) = full {
        ui.separator();
        // Navigation
        let fr = menu
            .filter_record
            .replace("{formid}", &format!("{formid:08X}"));
        if key(ui, &fr, "", formid != 0, "") {
            pick = Some(MenuAction::FilterRecord);
        }
        if key(ui, &menu.filter_source, "", true, "") {
            pick = Some(MenuAction::FilterSource);
        }
        if key(ui, &menu.filter_speaker, "", is_info, "") {
            pick = Some(MenuAction::FilterSpeaker);
        }
        ui.separator();
        // Outils de la ligne
        if key(ui, &menu.book, "", is_book, &menu.book_disabled) {
            pick = Some(MenuAction::Book);
        }
        ui.separator();
        if ui
            .add_enabled(can_undo, egui::Button::new(undo_label))
            .clicked()
        {
            out.undo_requested = true;
            pick = None;
            ui.close_menu();
        }
        if ui
            .add_enabled(can_redo, egui::Button::new(redo_label))
            .clicked()
        {
            out.redo_requested = true;
            pick = None;
            ui.close_menu();
        }
    }
    if let Some(a) = pick {
        out.menu_action = Some((idx, a));
        ui.close_menu();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut egui::Ui,
    entries: &mut [TmEntry],
    visible: &[usize],
    colors: &StatusColors,
    labels: &GridLabels<'_>,
    scroll_to: Option<usize>,
    selection: &Selection,
    focus_request: Option<usize>,
    cache: &mut HeightCache,
    doc: (u64, u64),
    mut filter_row: Option<&mut FilterRow<'_>>,
) -> GridOutput {
    let GridLabels {
        game,
        page,
        len_over_tip,
        len_newline_tip,
        bytes_unit,
        alias_tips,
        show_special,
        anomalies,
        prov_tip,
        prov_short,
        menu,
        legends,
        book_label,
        headers,
        undo_label,
        redo_label,
        can_undo,
        can_redo,
        unused_tip,
        vmad_off_tip,
        field_tip,
        comment,
        col_menu,
        review,
        status_buttons,
        empty_msg,
        file_col,
    } = *labels;
    let muted = crate::ui::special::muted(ui.visuals());
    let _ = book_label;
    let mut out = GridOutput::default();
    let gap = ui.spacing().item_spacing.x;
    let opt = OptColumns {
        status_buttons: status_buttons.is_some(),
        review,
        provenance: prov_short.is_some(),
        comment: comment.is_some(),
        file: file_col,
    };
    let w = flex_widths(ui.available_width(), gap, opt);
    let (edid_w, src_w, tgt_w, com_w) = (w.edid, w.src, w.tgt, w.com);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let title_h = font.size + 10.0;
    // The filter row (A-10) lives in the header, under the titles.
    let filter_h = if filter_row.is_some() {
        font.size + 14.0
    } else {
        0.0
    };
    let header_h = title_h + filter_h;
    let dark = on_pastel();
    let sel_color = ui.visuals().selection.bg_fill;

    // Per-row heights: the tallest of the wrapped source, the wrapped
    // translation and the review, and one text line — plus a little
    // vertical padding; kept between frames (the cache).
    let pad = 8.0;
    let heights = cache.heights(ui, entries, visible, doc, &w, review, &font, pad);

    let body_top = ui.cursor().top() + header_h;
    let body_left = ui.cursor().left();
    let mut table = TableBuilder::new(ui)
        .striped(false)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::TOP));
    // After undo / redo: bring the first touched line into view (R-A01-04).
    if let Some(pos) = scroll_to.and_then(|idx| visible.iter().position(|v| *v == idx)) {
        table = table.scroll_to_row(pos, Some(egui::Align::Center));
    }
    table = table.column(Column::exact(ST_W));
    if opt.status_buttons {
        table = table.column(Column::exact(BTN_W));
    }
    if opt.file {
        table = table.column(Column::exact(FILE_W));
    }
    table = table
        .column(Column::exact(GRUP_W))
        .column(Column::exact(CHAMP_W))
        .column(Column::exact(edid_w))
        .column(Column::exact(src_w))
        .column(Column::exact(tgt_w));
    if opt.review {
        table = table.column(Column::exact(w.review));
    }
    if opt.provenance {
        table = table.column(Column::exact(w.prov));
    }
    if opt.comment {
        table = table.column(Column::exact(com_w));
    }
    use crate::filter::FilterCol as FC;
    // The titles and, per column, the field of the filter row (A-10):
    // none under St and the status buttons (R-A10-01).
    let mut shown: Vec<(&str, Option<FC>)> = vec![
        (&headers[0], None),
        (&headers[1], Some(FC::Record)),
        (&headers[2], Some(FC::Field)),
        (&headers[3], Some(FC::Edid)),
        (&headers[4], Some(FC::Source)),
        (&headers[5], Some(FC::Trans)),
    ];
    if opt.status_buttons {
        // The buttons speak for themselves: no title.
        shown.insert(1, ("", None));
    }
    if opt.file {
        // Second after St and the buttons (R-B25-04).
        let at = if opt.status_buttons { 2 } else { 1 };
        shown.insert(at, (&headers[H_FILE], Some(FC::File)));
    }
    if opt.review {
        shown.push((&headers[H_REVIEW], Some(FC::Review)));
    }
    if opt.provenance {
        shown.push((&headers[H_PROV], Some(FC::Prov)));
    }
    if opt.comment {
        shown.push((&headers[H_COMMENT], Some(FC::Comment)));
    }
    let mut toggled: Option<usize> = None;
    table
        .header(header_h, |mut header| {
            for (n, (title, fcol)) in shown.iter().enumerate() {
                header.col(|ui| {
                    let cell = ui.max_rect();
                    ui.vertical(|ui| {
                        ui.strong(*title);
                        let Some(fr) = filter_row.as_deref_mut() else {
                            return;
                        };
                        ui.add_space(2.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        match fcol {
                            Some(c) => {
                                let err = &fr.errors[c.index()];
                                let has_review_btn = *c == FC::Review;
                                let btn_w = if has_review_btn { 26.0 } else { 0.0 };
                                ui.horizontal(|ui| {
                                    let id = ui.id().with(("ximod-col-filter", c.index()));
                                    let mut edit =
                                        egui::TextEdit::singleline(fr.filters.get_mut(*c))
                                            .id(id)
                                            .desired_width((cell.width() - 6.0 - btn_w).max(20.0));
                                    if err.is_some() {
                                        edit = edit.text_color(ui.visuals().error_fg_color);
                                    }
                                    let focused = ui.memory(|m| m.has_focus(id));
                                    // Entrée applies at once; Échap empties
                                    // the field (R-A10-04) — egui drops the
                                    // focus on Échap before the frame, so it
                                    // shows as a focus lost with the key down.
                                    let enter = focused
                                        && ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                        });
                                    let esc_down = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                    let resp = ui.add(edit);
                                    let esc = esc_down && resp.lost_focus();
                                    let resp = match err {
                                        Some(e) => resp.on_hover_text(e),
                                        None => resp,
                                    };
                                    if resp.changed() {
                                        out.filter_typed = true;
                                    }
                                    if esc && !fr.filters.get(*c).is_empty() {
                                        fr.filters.get_mut(*c).clear();
                                        out.filter_now = true;
                                    }
                                    if enter {
                                        out.filter_enter = true;
                                    }
                                    if has_review_btn
                                        && ui
                                            .selectable_label(*fr.review_diff, "≠")
                                            .on_hover_text(fr.review_diff_tip)
                                            .clicked()
                                    {
                                        *fr.review_diff = !*fr.review_diff;
                                        out.filter_now = true;
                                    }
                                });
                            }
                            None if n == 0 => {
                                // The eraser, left of the row (R-A10-05).
                                let active = !fr.filters.is_empty() || *fr.review_diff;
                                if ui
                                    .add_enabled(
                                        active,
                                        egui::Button::new(crate::icons::BACKSPACE).small(),
                                    )
                                    .on_hover_text(fr.clear_tip)
                                    .clicked()
                                {
                                    fr.filters.clear();
                                    *fr.review_diff = false;
                                    out.filter_now = true;
                                }
                            }
                            None => {}
                        }
                    });
                    // The columns menu on a right-click of the title (D-42);
                    // the filter field below keeps its own clicks.
                    let title_rect =
                        egui::Rect::from_min_size(cell.min, egui::vec2(cell.width(), title_h));
                    let resp = ui.interact(
                        title_rect,
                        ui.id().with(("ximod-grid-header", n)),
                        egui::Sense::click(),
                    );
                    resp.context_menu(|ui| {
                        for (i, (label, on)) in col_menu.iter().enumerate() {
                            let mut v = *on;
                            if ui.checkbox(&mut v, label).changed() {
                                toggled = Some(i);
                                ui.close_menu();
                            }
                        }
                    });
                });
            }
        })
        .body(|body| {
            body.heterogeneous_rows(heights.into_iter(), |mut row| {
                let idx = visible[row.index()];
                let (cur, color, is_book, unused, frozen) = {
                    let e = &entries[idx];
                    let is_book = e.rec_sig == "BOOK"
                        || crate::book::has_markup(&e.source)
                        || crate::book::has_markup(&e.trans);
                    // A line never shown in game is red (the Ignoré colour)
                    // whatever its status, and cannot be edited.
                    let color = if e.unused {
                        ignored_color(colors)
                    } else {
                        status_color(e, colors)
                    };
                    // Held by the file's VMAD switch (C-08): not editable
                    // line by line either.
                    let frozen = e.unused || crate::vmad_lock::is_file_locked(e);
                    (status::derive(e), color, is_book, e.unused, frozen)
                };
                let frozen_tip = if unused { unused_tip } else { vmad_off_tip };
                let is_selected = selection.rows.contains(&idx) || selection.cursor == Some(idx);
                let color = if is_selected {
                    selected_tint(color, sel_color)
                } else {
                    color
                };

                // Selection as the menu will see it: a right-click on a line
                // outside the selection selects it alone (R-M01-01).
                let in_sel = selection.rows.contains(&idx);
                let sel_count = if in_sel {
                    selection.rows.len().max(1)
                } else {
                    1
                };
                let formid = entries[idx].formid;
                let has_trans = !entries[idx].trans.is_empty();
                let has_review = !entries[idx].review.is_empty();
                // The row context menu (M-01): one menu, the same on every cell.
                let menu_row = MenuRow {
                    idx,
                    cur,
                    unused,
                    formid,
                    has_trans,
                    has_review,
                    is_book,
                    sel_count,
                    is_info: entries[idx].rec_sig == "INFO",
                };
                let menu_ui = |ui: &mut egui::Ui, out: &mut GridOutput| {
                    row_menu(
                        ui,
                        out,
                        &menu_row,
                        menu,
                        legends,
                        colors,
                        Some((undo_label, redo_label, can_undo, can_redo)),
                    );
                };

                // --- Status cell: colored swatch + glyph. ---
                row.col(|ui| {
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, 0.0, color);
                    ui.painter().text(
                        rect.center_top() + egui::vec2(0.0, font.size * 0.7),
                        egui::Align2::CENTER_CENTER,
                        cur.glyph(),
                        egui::FontId::proportional(12.0),
                        dark,
                    );
                    // « ≠ » under the glyph for an ambiguous line (B-08).
                    if crate::ambiguity::is_marked(&entries[idx]) {
                        ui.painter().text(
                            egui::pos2(rect.right() - 2.0, rect.bottom() - 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            "≠",
                            egui::FontId::proportional(11.0),
                            dark,
                        );
                    }
                    // « [!] » next to the swatch for a line in anomaly (D-05).
                    let anomaly_tip = anomalies.get(&idx);
                    if anomaly_tip.is_some() {
                        ui.painter().text(
                            egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
                            egui::Align2::RIGHT_TOP,
                            "[!]",
                            egui::FontId::proportional(9.0),
                            ui.visuals().error_fg_color,
                        );
                    }
                    let resp = ui.interact(rect, ui.id().with(("st", idx)), egui::Sense::click());
                    // « Statut — provenance » (B-09, R-B09-04).
                    let origin = prov_tip(&entries[idx]);
                    let resp = if anomaly_tip.is_some() || origin.is_some() {
                        resp.on_hover_ui(|ui| {
                            if let Some(tip) = anomaly_tip {
                                ui.label(tip);
                            }
                            if let Some(o) = &origin {
                                ui.label(o);
                            }
                        })
                    } else {
                        resp
                    };
                    if resp.clicked() {
                        // The status cell takes the click before the row does.
                        out.row_clicked = Some((idx, resp.ctx.input(|i| i.modifiers)));
                    }
                    if resp.secondary_clicked() {
                        out.row_right_clicked = Some(idx);
                    }
                    if resp.double_clicked() {
                        out.row_double_clicked = Some(idx);
                    }
                    let resp = if frozen {
                        resp.on_hover_text(frozen_tip)
                    } else {
                        resp
                    };
                    resp.context_menu(|ui| menu_ui(ui, &mut out));
                });

                // --- Status buttons (A-04): four small buttons on their
                //     pastel, the current one pressed, an impossible one
                //     greyed with the reason; none on a line never shown. ---
                if let Some(btn) = status_buttons {
                    row.col(|ui| {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                        if frozen {
                            return;
                        }
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.spacing_mut().button_padding = egui::vec2(4.0, 1.0);
                        let e = &entries[idx];
                        for (k, st) in STATUS_BUTTONS.iter().enumerate() {
                            let (r, g, b) = colors[st.index()];
                            let text = egui::RichText::new(st.glyph()).color(dark).size(12.0);
                            let button = egui::Button::new(text)
                                .fill(Color32::from_rgb(r, g, b))
                                .selected(*st == cur)
                                .min_size(egui::vec2(20.0, font.size + 4.0));
                            let enabled = st.applicable_to(e);
                            let resp = ui
                                .add_enabled(enabled, button)
                                .on_hover_text(&btn.tips[k])
                                .on_disabled_hover_text(&btn.needs_trans[k]);
                            if resp.clicked() {
                                out.status_button = Some((idx, *st));
                            }
                        }
                    });
                }

                // Helper: paint the cell background then show a (dark) label.
                let paint_label = |ui: &mut egui::Ui, text: &str, wrap: bool| {
                    ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                    let mode = if wrap {
                        egui::TextWrapMode::Wrap
                    } else {
                        egui::TextWrapMode::Truncate
                    };
                    // Not selectable: the click goes to the row (A-03), the
                    // text is copied from the menu or the edit window.
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).color(dark))
                            .wrap_mode(mode)
                            .selectable(false),
                    );
                };

                if opt.file {
                    row.col(|ui| paint_label(ui, &entries[idx].plugin, false));
                }
                row.col(|ui| paint_label(ui, &entries[idx].rec_sig, false));
                row.col(|ui| {
                    paint_label(ui, &entries[idx].field_sig, false);
                    if let Some(tip) = field_tip(&entries[idx]) {
                        ui.interact(
                            ui.max_rect(),
                            ui.id().with(("fld", idx)),
                            egui::Sense::hover(),
                        )
                        .on_hover_text(tip);
                    }
                });
                // A truncated label already shows its full text on hover: no
                // explicit tooltip, or it would appear twice.
                row.col(|ui| paint_label(ui, &entries[idx].edid, false));
                row.col(|ui| {
                    if show_special && crate::replace::has_special(&entries[idx].source) {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                        let job = crate::ui::special::job(
                            &entries[idx].source,
                            font.clone(),
                            dark,
                            muted,
                            src_w,
                            false,
                        );
                        ui.add(egui::Label::new(job).wrap().selectable(false));
                    } else {
                        paint_label(ui, &entries[idx].source, true);
                    }
                });

                // --- Translation: editable, frameless (so the row color shows),
                //     multiline so long text wraps instead of overflowing. ---
                row.col(|ui| {
                    ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                    if frozen {
                        // Not translatable: the translation kept under the
                        // VMAD switch, else the source, greyed, no editor.
                        let e = &entries[idx];
                        let shown = if unused || e.trans.is_empty() {
                            &e.source
                        } else {
                            &e.trans
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(shown).color(dark).weak())
                                .wrap()
                                .selectable(false),
                        )
                        .on_hover_text(frozen_tip);
                        return;
                    }
                    ui.visuals_mut().override_text_color = Some(dark);
                    // The editor fills the cell: a click anywhere in a tall
                    // row (long source) starts the edit, not only its first line.
                    let cell_h = ui.max_rect().height();
                    let mut edit = egui::TextEdit::multiline(&mut entries[idx].trans)
                        .frame(false)
                        .desired_rows(1)
                        .min_size(egui::vec2(tgt_w, cell_h))
                        .desired_width(tgt_w);
                    let mut layouter = crate::ui::special::layouter(font.clone(), dark, muted);
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
                    if focus_request == Some(idx) {
                        resp.request_focus();
                        out.focus_applied = true;
                    }
                    if resp.gained_focus() {
                        out.focus_gained = Some(idx);
                    }
                    if resp.lost_focus() {
                        out.focus_lost = Some(idx);
                    }
                    if resp.changed() {
                        out.edited = Some(idx);
                    }
                    if resp.secondary_clicked() {
                        out.row_right_clicked = Some(idx);
                    }
                    // Length badges (A-05, R-A05-03): nothing while the
                    // translation fits; « 34 / 32 » in red when over a limit,
                    // [!] in orange on a forbidden line break; details on hover.
                    let report = crate::length::Report::of_page(&entries[idx], game, page);
                    let tags = crate::tags::Report::of(&entries[idx], game);
                    let resp = if report.is_anomaly() || tags.is_anomaly() {
                        let e = &entries[idx];
                        let field = format!("{}:{}", e.rec_sig, e.field_sig);
                        let cell = ui.max_rect();
                        let small = egui::FontId::proportional((font.size * 0.85).max(9.0));
                        let mut x = cell.right() - 4.0;
                        let mut tip = String::new();
                        if report.over() {
                            let badge = report.badge(bytes_unit);
                            let r = ui.painter().text(
                                egui::pos2(x, cell.top() + 2.0),
                                egui::Align2::RIGHT_TOP,
                                &badge,
                                small.clone(),
                                ui.visuals().error_fg_color,
                            );
                            x = r.left() - 6.0;
                            let max = report
                                .limits
                                .max_chars
                                .or(report.limits.max_bytes)
                                .unwrap_or(0);
                            tip.push_str(
                                &len_over_tip
                                    .replace("{chars}", &report.chars.to_string())
                                    .replace("{max}", &max.to_string())
                                    .replace("{bytes}", &report.bytes.to_string())
                                    .replace("{field}", &field),
                            );
                        }
                        if report.bad_newline {
                            let r = ui.painter().text(
                                egui::pos2(x, cell.top() + 2.0),
                                egui::Align2::RIGHT_TOP,
                                "[!]",
                                small.clone(),
                                ui.visuals().warn_fg_color,
                            );
                            x = r.left() - 6.0;
                            if !tip.is_empty() {
                                tip.push('\n');
                            }
                            tip.push_str(&len_newline_tip.replace("{field}", &field));
                        }
                        // Aliases and numbers (A-07): « ⚠ », red when something
                        // is missing or unknown, orange for an order difference;
                        // the edit window's tab has the detail and the repairs.
                        if tags.is_anomaly() {
                            let color = if tags.is_error() {
                                ui.visuals().error_fg_color
                            } else {
                                ui.visuals().warn_fg_color
                            };
                            ui.painter().text(
                                egui::pos2(x, cell.top() + 2.0),
                                egui::Align2::RIGHT_TOP,
                                crate::icons::WARNING,
                                small,
                                color,
                            );
                            if !tip.is_empty() {
                                tip.push('\n');
                            }
                            let mut parts: Vec<String> = Vec::new();
                            for t in &tags.missing_tags {
                                parts.push(alias_tips[0].replace("{tag}", t));
                            }
                            for t in &tags.unknown_tags {
                                parts.push(alias_tips[1].replace("{tag}", t));
                            }
                            for n in &tags.missing_numbers {
                                parts.push(alias_tips[2].replace("{n}", n));
                            }
                            for n in &tags.unknown_numbers {
                                parts.push(alias_tips[3].replace("{n}", n));
                            }
                            if tags.order_differs {
                                parts.push(alias_tips[4].clone());
                            }
                            tip.push_str(&parts.join("\n"));
                        }
                        resp.on_hover_text(tip)
                    } else {
                        resp
                    };
                    resp.context_menu(|ui| menu_ui(ui, &mut out));
                });
                // --- Relecture (A-17): editable like the translation, in
                //     blue when it differs from it, grey when it is the same.
                if review {
                    row.col(|ui| {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                        if frozen {
                            return;
                        }
                        let differs = crate::review::differs(&entries[idx]);
                        ui.visuals_mut().override_text_color = Some(review_color(differs));
                        let cell_h = ui.max_rect().height();
                        let output = egui::TextEdit::multiline(&mut entries[idx].review)
                            .id(ui.id().with(("ximod-review", idx)))
                            .frame(false)
                            .desired_rows(1)
                            .min_size(egui::vec2(w.review, cell_h))
                            .desired_width(w.review)
                            .show(ui);
                        let resp = output.response;
                        if resp.gained_focus() {
                            out.review_focus_gained = Some(idx);
                        }
                        if resp.lost_focus() {
                            out.review_focus_lost = Some(idx);
                        }
                        if resp.changed() {
                            out.review_edited = Some(idx);
                        }
                        if resp.secondary_clicked() {
                            out.row_right_clicked = Some(idx);
                        }
                        resp.context_menu(|ui| menu_ui(ui, &mut out));
                    });
                }
                // --- Provenance (B-09): the short label, the whole one on hover.
                if let Some(short_of) = prov_short {
                    row.col(|ui| {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                        let text = short_of(&entries[idx]);
                        let resp = ui.add(
                            egui::Label::new(egui::RichText::new(&text).color(dark))
                                .wrap_mode(egui::TextWrapMode::Truncate)
                                .selectable(false),
                        );
                        if let Some(full) = prov_tip(&entries[idx]) {
                            resp.on_hover_text(full);
                        }
                    });
                }
                // --- Commentaire (C-03): generated on first display, cut at
                //     200 characters with the whole text on hover. ---
                if let Some(comment_of) = comment {
                    row.col(|ui| {
                        ui.painter().rect_filled(ui.max_rect(), 0.0, color);
                        let text = comment_of(idx, &entries[idx]);
                        let shown = crate::comments::shown(&text);
                        let label = egui::Label::new(
                            egui::RichText::new(shown.as_deref().unwrap_or(&text)).color(dark),
                        )
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .selectable(false);
                        let resp = ui.add(label);
                        if shown.is_some() {
                            resp.on_hover_text(text.to_string());
                        }
                    });
                }
                // A click anywhere else on the row selects it (A-03); a
                // right-click opens the same menu (M-01).
                let rr = row.response();
                if rr.clicked() {
                    let mods = rr.ctx.input(|i| i.modifiers);
                    out.row_clicked = Some((idx, mods));
                }
                if rr.secondary_clicked() {
                    out.row_right_clicked = Some(idx);
                }
                if rr.double_clicked() {
                    out.row_double_clicked = Some(idx);
                }
                rr.context_menu(|ui| menu_ui(ui, &mut out));
            });
        });
    out.column_toggled = toggled;
    // No line left: the active filters, in the empty body (R-A10-06).
    if visible.is_empty()
        && let Some(msg) = empty_msg
    {
        ui.painter().text(
            egui::pos2(body_left + 12.0, body_top + 12.0),
            egui::Align2::LEFT_TOP,
            msg,
            font.clone(),
            ui.visuals().weak_text_color(),
        );
    }
    out
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn clicks_build_single_toggle_and_range_selections() {
        let visible = vec![10, 11, 12, 13, 14];
        let mut s = Selection::default();
        s.click(11, egui::Modifiers::NONE, &visible);
        assert_eq!(s.targets(), vec![11]);
        s.click(13, egui::Modifiers::COMMAND, &visible);
        assert_eq!(s.targets(), vec![11, 13]);
        s.click(13, egui::Modifiers::COMMAND, &visible);
        assert_eq!(s.targets(), vec![11], "toggle off");
        s.click(14, egui::Modifiers::SHIFT, &visible);
        assert_eq!(
            s.targets(),
            vec![13, 14],
            "range from the anchor (the last Ctrl-click)"
        );
        s.click(11, egui::Modifiers::NONE, &visible);
        s.click(14, egui::Modifiers::SHIFT, &visible);
        assert_eq!(
            s.targets(),
            vec![11, 12, 13, 14],
            "range from a plain click"
        );
        s.click(10, egui::Modifiers::NONE, &visible);
        assert_eq!(s.targets(), vec![10]);
        let empty = Selection {
            cursor: Some(12),
            ..Default::default()
        };
        assert_eq!(empty.targets(), vec![12], "cursor alone");
    }

    #[test]
    fn navigation_keys_follow_the_view_order() {
        let v = [10usize, 20, 30, 40, 50];
        assert_eq!(nav_target(&v, Some(30), NavKey::Up, 2), Some(20));
        assert_eq!(nav_target(&v, Some(30), NavKey::Down, 2), Some(40));
        assert_eq!(nav_target(&v, Some(30), NavKey::PageUp, 2), Some(10));
        assert_eq!(nav_target(&v, Some(30), NavKey::PageDown, 10), Some(50));
        assert_eq!(nav_target(&v, Some(10), NavKey::Up, 2), Some(10));
        assert_eq!(nav_target(&v, Some(50), NavKey::Down, 2), Some(50));
        assert_eq!(nav_target(&v, Some(30), NavKey::Home, 2), Some(10));
        assert_eq!(nav_target(&v, Some(30), NavKey::End, 2), Some(50));
        // No current line (or one outside the view): a landing line.
        assert_eq!(nav_target(&v, None, NavKey::Down, 2), Some(10));
        assert_eq!(nav_target(&v, Some(99), NavKey::End, 2), Some(50));
        assert_eq!(nav_target(&[], Some(1), NavKey::Down, 2), None);
    }

    #[test]
    fn next_line_follows_the_view_order() {
        let visible = vec![3, 7, 9, 20];
        assert_eq!(next_in_view(&visible, &[7], 1), Some(9));
        assert_eq!(
            next_in_view(&visible, &[3, 9], 1),
            Some(20),
            "after the last of the selection"
        );
        assert_eq!(
            next_in_view(&visible, &[3, 9], -1),
            None,
            "before the first: edge"
        );
        assert_eq!(next_in_view(&visible, &[20], 1), None);
        assert_eq!(next_in_view(&visible, &[9], -1), Some(7));
        assert_eq!(next_in_view(&visible, &[], 1), None);
    }

    /// A status button works on the selection when its line is in it, on
    /// its line alone otherwise (R-A04-02, R-A04-04); the column takes its
    /// width from the text columns and gives it back when hidden
    /// (R-A04-01, T-A04-02).
    #[test]
    fn status_buttons_targets_and_width() {
        let mut s = Selection::default();
        let visible: Vec<usize> = (0..20).collect();
        for i in 0..12 {
            s.click(i, egui::Modifiers::COMMAND, &visible);
        }
        assert_eq!(button_targets(&s, 5), (0..12).collect::<Vec<_>>());
        assert_eq!(button_targets(&s, 15), vec![15], "a line outside");
        let off = flex_widths(1200.0, 4.0, OptColumns::default());
        let on = flex_widths(
            1200.0,
            4.0,
            OptColumns {
                status_buttons: true,
                ..Default::default()
            },
        );
        let total = |w: &Widths| w.edid + w.src + w.tgt;
        assert!((total(&off) - total(&on) - BTN_W - 4.0).abs() < 2.0);
        assert_eq!(
            flex_widths(1200.0, 4.0, OptColumns::default()),
            off,
            "hidden again: the same widths"
        );
    }
}

#[cfg(test)]
mod height_tests {
    use super::*;

    /// The word-wrap estimate of the row heights gives the rows egui's
    /// layout gives, on short, long, broken and unbreakable texts.
    #[test]
    fn measurer_follows_the_layout() {
        let fonts = egui::text::Fonts::new(1.0, 1024, egui::FontDefinitions::default());
        fonts.begin_pass(1.0, 1024);
        let font = egui::FontId::proportional(14.0);
        let mut m = Measurer::new(&fonts, &font);
        let texts = [
            "Iron Sword",
            "A long sentence that will certainly wrap on several rows at a narrow width.",
            "Two\nlines",
            "Averyveryveryverylongwordwithoutanyspaceinsideit",
            "  leading spaces and trailing   ",
            "Ancient journey north cloak ancient says ancient journey whiterun",
        ];
        let (mut exact, mut total) = (0, 0);
        for width in [60.0, 120.0, 200.0, 400.0] {
            for t in texts {
                let galley = fonts.layout(t.to_owned(), font.clone(), Color32::BLACK, width);
                let rows = galley.rows.len();
                let est = m.rows(t, width);
                // Never below (a row would be cut), at most one row above.
                assert!(
                    est == rows || est == rows + 1,
                    "{t:?} at {width}: egui {rows} rows, estimate {est}"
                );
                exact += (est == rows) as usize;
                total += 1;
            }
        }
        assert!(exact * 10 >= total * 8, "{exact} exact of {total}");
    }
}
