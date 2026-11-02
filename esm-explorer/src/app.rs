//! ESM Explorer — egui application.
//!
//! Uses a lazy [`PluginIndex`](crate::esm::PluginIndex): the file is memory-mapped
//! and indexed header-only (instant even for 1.35 GB), and record fields (EDID,
//! name, cell coordinates, a cell's references) are decoded on demand and cached.
//! The virtualized tables only ever decode what is on screen, so scrolling stays
//! smooth. Filtering/sorting by name decodes the current subset incrementally,
//! time-boxed per frame, with a progress readout — the UI never freezes.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use memmap2::Mmap;

use crate::esm::index::RefRow;
use crate::esm::{
    PluginIndex, StringTable, SubRecord, category, games, keywords, lang, stringtable,
};
use rust_xlsxwriter::{Format, Workbook};

/// Record types exported to the spreadsheet (item-like records with a name/model).
const ITEM_TYPES: &[&str] = &[
    "WEAP", "ARMO", "AMMO", "ALCH", "INGR", "MISC", "BOOK", "KEYM", "SLGM", "NOTE", "SCRL", "APPA",
    "FURN", "LIGH", "WOOP",
];

/// Morrowind (TES3) record types that carry translatable text — the headless
/// `--xlsx` default for a TES3 plugin. Dialogue (`INFO`/`DIAL`), books, game
/// settings (`GMST`), descriptions (`CLAS`/`RACE`/`BSGN`/`SKIL`/`MGEF`), and
/// every named object/actor.
const TES3_TEXT_TYPES: &[&str] = &[
    "GMST", "INFO", "DIAL", "BOOK", "BSGN", "CLAS", "RACE", "FACT", "SKIL", "MGEF", "SPEL", "ALCH",
    "APPA", "ARMO", "WEAP", "CLOT", "INGR", "MISC", "LOCK", "PROB", "REPA", "LIGH", "ACTI", "CONT",
    "DOOR", "CREA", "NPC_", "CELL", "REGN",
];

/// Time budget for lazy name decoding per frame (keeps the UI at frame rate).
const DECODE_BUDGET: Duration = Duration::from_millis(6);
/// Minimum interval between view rebuilds while an incremental decode runs.
const REBUILD_THROTTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortCol {
    Edid,
    FormId,
    Name,
    Type,
}
impl SortCol {
    fn key(self) -> u8 {
        match self {
            SortCol::Edid => 0,
            SortCol::FormId => 1,
            SortCol::Name => 2,
            SortCol::Type => 3,
        }
    }
    fn needs_names(self) -> bool {
        matches!(self, SortCol::Edid | SortCol::Name)
    }
}

/// State tied to one loaded plugin.
struct Loaded {
    file_name: String,
    plugin_path: PathBuf,
    mmap: Mmap,
    index: PluginIndex,
    /// record index → (EDID, display name), decoded lazily.
    names: HashMap<usize, (String, String)>,
    /// record index → exterior coordinates, decoded lazily.
    coords: HashMap<usize, Option<(i32, i32)>>,
    /// Resolved external STRINGS (localized plugins), and their metadata.
    strings: StringTable,
    languages: Vec<String>,
    lang: Option<String>,
    strings_sources: Vec<String>,
}

impl Loaded {
    /// Resolve a record's FULL name: inline text, or — for localized plugins —
    /// the STRINGS text for its id (falling back to the bare id when unresolved).
    fn resolve_name(&self, fields: &[SubRecord]) -> String {
        let Some(full) = fields.iter().find(|s| s.sig == "FULL") else {
            return String::new();
        };
        if self.index.localized {
            match full.as_u32() {
                Some(id) if id != 0 => self
                    .strings
                    .get(id)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("[{id:08X}]")),
                _ => String::new(),
            }
        } else {
            full.as_zstring()
        }
    }

    fn compute_name(&self, i: usize) -> (String, String) {
        // Morrowind (TES3): defer to the index's TES3 mapping (EDID = NAME, or
        // INAM for INFO; display name = FNAM). Morrowind is never localized, so
        // there is no STRINGS resolution to apply.
        if self.index.tes3 {
            return self.index.edid_name(&self.mmap[..], i, false);
        }
        let fields = self.index.record_fields(&self.mmap[..], i);
        let edid = fields
            .iter()
            .find(|s| s.sig == "EDID")
            .map(|s| s.as_zstring())
            .unwrap_or_default();
        (edid, self.resolve_name(&fields))
    }

    fn ensure_name(&mut self, i: usize) {
        if !self.names.contains_key(&i) {
            let v = self.compute_name(i);
            self.names.insert(i, v);
        }
    }
    fn ensure_coords(&mut self, i: usize) {
        if !self.coords.contains_key(&i) {
            let v = self.index.coords(&self.mmap[..], i);
            self.coords.insert(i, v);
        }
    }
    fn name_of(&self, i: usize) -> (&str, &str) {
        self.names
            .get(&i)
            .map(|(e, n)| (e.as_str(), n.as_str()))
            .unwrap_or(("", ""))
    }
}

pub struct EsmExplorerApp {
    loaded: Option<Loaded>,
    error: Option<String>,

    // Object Window state.
    filter: String,
    type_filter: Option<String>,
    sort_col: SortCol,
    sort_asc: bool,
    ow_view: Vec<usize>,
    ow_key: Option<(Option<String>, String, u8, bool)>,
    ow_base_type: Option<Option<String>>,
    ow_cursor: usize,
    ow_decoding: bool,
    ow_last_rebuild: Option<Instant>,

    // Cell View state.
    selected_world: Option<u32>,
    cell_filter: String,
    selected_cell: Option<u32>,
    cv_view: Vec<usize>, // indices into index.cells
    cv_key: Option<(Option<u32>, String)>,
    cv_decoded_for: Option<Option<u32>>,
    sel_refs: Vec<RefRow>,
    sel_refs_for: Option<u32>,

    // File Explorer state (record-type → text extractor).
    fe_selected: HashSet<String>,
    fe_filter: String,
    fe_status: Option<String>,

    // Record detail view.
    selected_record: Option<usize>,
    show_detail: bool,
    detail_hex: bool,

    /// Transient success notice (e.g. after a spreadsheet export).
    notice: Option<String>,
}

impl Default for EsmExplorerApp {
    fn default() -> Self {
        Self {
            loaded: None,
            error: None,
            filter: String::new(),
            type_filter: None,
            sort_col: SortCol::FormId,
            sort_asc: true,
            ow_view: Vec::new(),
            ow_key: None,
            ow_base_type: None,
            ow_cursor: 0,
            ow_decoding: false,
            ow_last_rebuild: None,
            selected_world: None,
            cell_filter: String::new(),
            selected_cell: None,
            cv_view: Vec::new(),
            cv_key: None,
            cv_decoded_for: None,
            sel_refs: Vec::new(),
            sel_refs_for: None,
            fe_selected: HashSet::new(),
            fe_filter: String::new(),
            fe_status: None,
            selected_record: None,
            show_detail: false,
            detail_hex: false,
            notice: None,
        }
    }
}

impl EsmExplorerApp {
    pub fn open_path(&mut self, path: &PathBuf) {
        self.load_path(path);
    }

    /// Headless extraction (used by the `--extract` CLI mode and tests): build the
    /// index for `plugin` and write the selected record types to `out`. `sigs`
    /// empty means every type.
    #[allow(clippy::field_reassign_with_default)]
    pub fn run_extract(plugin: &Path, out: &Path, sigs: &[String]) -> std::io::Result<usize> {
        let file = std::fs::File::open(plugin)?;
        let mmap = unsafe { Mmap::map(&file) }?;
        let index = PluginIndex::build(&mmap[..])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let file_name = plugin
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (strings, languages, lang, strings_sources) = if index.localized {
            let load = stringtable::load(plugin, index.game, "English");
            (load.table, load.languages, load.chosen, load.sources)
        } else {
            (StringTable::default(), Vec::new(), None, Vec::new())
        };
        let loaded = Loaded {
            file_name,
            plugin_path: plugin.to_path_buf(),
            mmap,
            index,
            names: HashMap::new(),
            coords: HashMap::new(),
            strings,
            languages,
            lang,
            strings_sources,
        };
        let mut app = EsmExplorerApp::default();
        app.fe_selected = if sigs.is_empty() {
            loaded.index.types.iter().map(|(s, _)| s.clone()).collect()
        } else {
            sigs.iter().cloned().collect()
        };
        app.extract_types(&loaded, out)
    }

    fn load_path(&mut self, path: &PathBuf) {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(format!("Impossible d’ouvrir le fichier : {e}"));
                return;
            }
        };
        // SAFETY: the file stays open for the lifetime of the mmap; we only read.
        let mmap = match unsafe { Mmap::map(&file) } {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(format!("Échec du mappage mémoire : {e}"));
                return;
            }
        };
        let index = match PluginIndex::build(&mmap[..]) {
            Ok(i) => i,
            Err(e) => {
                self.error = Some(format!("Erreur de lecture du plugin : {e}"));
                return;
            }
        };
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Load external STRINGS for localized plugins (loose files + archives).
        let (strings, languages, lang, strings_sources) = if index.localized {
            let load = stringtable::load(path, index.game, "English");
            (load.table, load.languages, load.chosen, load.sources)
        } else {
            (StringTable::default(), Vec::new(), None, Vec::new())
        };

        self.loaded = Some(Loaded {
            file_name,
            plugin_path: path.clone(),
            mmap,
            index,
            names: HashMap::new(),
            coords: HashMap::new(),
            strings,
            languages,
            lang,
            strings_sources,
        });
        // Reset all view state.
        self.filter.clear();
        self.type_filter = None;
        self.sort_col = SortCol::FormId;
        self.sort_asc = true;
        self.ow_key = None;
        self.ow_base_type = None;
        self.ow_cursor = 0;
        self.ow_decoding = false;
        self.selected_world = None;
        self.cell_filter.clear();
        self.selected_cell = None;
        self.cv_key = None;
        self.cv_decoded_for = None;
        self.sel_refs.clear();
        self.sel_refs_for = None;
        self.fe_selected.clear();
        self.fe_filter.clear();
        self.fe_status = None;
        self.selected_record = None;
        self.show_detail = false;
        self.notice = None;
        self.error = None;
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Plugins Bethesda (.esp/.esm/.esl)", &["esp", "esm", "esl"])
            .add_filter("Tous les fichiers", &["*"])
            .pick_file()
        {
            self.load_path(&path);
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let is_plugin = |p: &PathBuf| {
            matches!(
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("esp") | Some("esm") | Some("esl")
            )
        };
        let path = dropped
            .iter()
            .find(|p| is_plugin(p))
            .cloned()
            .unwrap_or_else(|| dropped[0].clone());
        self.load_path(&path);
    }

    fn draw_drag_hint(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drag_hint"),
        ));
        let screen = ctx.screen_rect();
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(150));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "Déposez un plugin (.esp / .esm / .esl)",
            egui::FontId::proportional(26.0),
            egui::Color32::WHITE,
        );
    }

    // ---- Object Window view synchronization ----------------------------

    fn base_objs<'a>(&self, loaded: &'a Loaded) -> &'a [usize] {
        match &self.type_filter {
            Some(t) => loaded
                .index
                .type_lists
                .get(t)
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            None => &loaded.index.obj_indices,
        }
    }

    fn sync_object_view(&mut self, ctx: &egui::Context, loaded: &mut Loaded) {
        let name_path = !self.filter.trim().is_empty() || self.sort_col.needs_names();
        let key = (
            self.type_filter.clone(),
            self.filter.clone(),
            self.sort_col.key(),
            self.sort_asc,
        );
        let key_changed = self.ow_key.as_ref() != Some(&key);
        if key_changed {
            self.ow_key = Some(key);
        }
        // Reset the decode cursor when the base (type) subset changes.
        if self.ow_base_type.as_ref() != Some(&self.type_filter) {
            self.ow_base_type = Some(self.type_filter.clone());
            self.ow_cursor = 0;
        }

        if !name_path {
            if key_changed {
                self.rebuild_object_view_fast(loaded);
            }
            self.ow_decoding = false;
            return;
        }

        // Name path: decode the current subset incrementally, time-boxed.
        let base_len = self.base_objs(loaded).len();
        let mut progressed = false;
        if self.ow_cursor < base_len {
            let deadline = Instant::now() + DECODE_BUDGET;
            let mut c = self.ow_cursor;
            while c < base_len {
                let i = self.base_objs(loaded)[c];
                if !loaded.names.contains_key(&i) {
                    let v = loaded.compute_name(i);
                    loaded.names.insert(i, v);
                    progressed = true;
                }
                c += 1;
                if Instant::now() >= deadline {
                    break;
                }
            }
            self.ow_cursor = c;
        }
        let done = self.ow_cursor >= base_len;
        self.ow_decoding = !done;

        let throttled = self
            .ow_last_rebuild
            .map(|t| t.elapsed() >= REBUILD_THROTTLE)
            .unwrap_or(true);
        if key_changed || done || (progressed && throttled) {
            self.rebuild_object_view_named(loaded);
            self.ow_last_rebuild = Some(Instant::now());
        }
        if !done {
            ctx.request_repaint();
        }
    }

    fn rebuild_object_view_fast(&mut self, loaded: &Loaded) {
        let idx = &loaded.index;
        let mut view = match (&self.type_filter, self.sort_col) {
            (Some(t), _) => idx.type_lists.get(t).cloned().unwrap_or_default(),
            (None, SortCol::FormId) => idx.by_formid_all.clone(),
            (None, SortCol::Type) => idx.by_type_concat.clone(),
            (None, _) => idx.by_formid_all.clone(),
        };
        if !self.sort_asc {
            view.reverse();
        }
        self.ow_view = view;
    }

    fn rebuild_object_view_named(&mut self, loaded: &Loaded) {
        let idx = &loaded.index;
        let needle = self.filter.trim().to_lowercase();
        let mut view: Vec<usize> = self
            .base_objs(loaded)
            .iter()
            .copied()
            .filter(|&i| {
                if needle.is_empty() {
                    return true;
                }
                let (edid, name) = loaded.name_of(i);
                edid.to_lowercase().contains(&needle)
                    || name.to_lowercase().contains(&needle)
                    || idx.records[i].sig_str().to_lowercase().contains(&needle)
                    || format!("{:08x}", idx.records[i].formid).contains(&needle)
            })
            .collect();

        view.sort_by(|&a, &b| {
            let ord = match self.sort_col {
                SortCol::FormId => idx.records[a].formid.cmp(&idx.records[b].formid),
                SortCol::Type => idx.records[a]
                    .sig_str()
                    .cmp(&idx.records[b].sig_str())
                    .then(idx.records[a].formid.cmp(&idx.records[b].formid)),
                SortCol::Edid => loaded
                    .name_of(a)
                    .0
                    .to_lowercase()
                    .cmp(&loaded.name_of(b).0.to_lowercase()),
                SortCol::Name => loaded
                    .name_of(a)
                    .1
                    .to_lowercase()
                    .cmp(&loaded.name_of(b).1.to_lowercase()),
            };
            if self.sort_asc { ord } else { ord.reverse() }
        });
        self.ow_view = view;
    }

    // ---- Cell View synchronization -------------------------------------

    fn sync_cell_view(&mut self, _ctx: &egui::Context, loaded: &mut Loaded) {
        let sel = self.selected_world;

        // Worldspace names are few; decode them once so the combo can read them.
        let worlds: Vec<usize> = loaded.index.worlds.clone();
        for wi in worlds {
            loaded.ensure_name(wi);
        }

        // Decode the names + coordinates of the selected world's cells once.
        if self.cv_decoded_for != Some(sel) {
            let tes3 = loaded.index.tes3;
            let cell_recs: Vec<usize> = loaded
                .index
                .cells
                .iter()
                .filter(|c| match sel {
                    // Morrowind has no worldspaces: show every cell.
                    None => tes3 || c.interior,
                    Some(w) => c.world == Some(w),
                })
                .map(|c| c.rec)
                .collect();
            for i in cell_recs {
                loaded.ensure_name(i);
                loaded.ensure_coords(i);
            }
            self.cv_decoded_for = Some(sel);
            self.cv_key = None; // force rebuild
        }

        let key = (sel, self.cell_filter.clone());
        if self.cv_key.as_ref() != Some(&key) {
            self.cv_key = Some(key);
            self.rebuild_cell_view(loaded);
        }

        // References of the selected cell, decoded on selection.
        if self.sel_refs_for != self.selected_cell {
            self.sel_refs = match self.selected_cell {
                Some(c) => loaded.index.cell_refs(&loaded.mmap[..], c),
                None => Vec::new(),
            };
            self.sel_refs_for = self.selected_cell;
        }
    }

    fn rebuild_cell_view(&mut self, loaded: &Loaded) {
        let idx = &loaded.index;
        let sel = self.selected_world;
        let needle = self.cell_filter.trim().to_lowercase();
        let mut view: Vec<usize> = idx
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| match sel {
                None => idx.tes3 || c.interior,
                Some(w) => c.world == Some(w),
            })
            .filter(|(_, c)| {
                if needle.is_empty() {
                    return true;
                }
                let (edid, name) = loaded.name_of(c.rec);
                edid.to_lowercase().contains(&needle)
                    || name.to_lowercase().contains(&needle)
                    || format!("{:08x}", idx.records[c.rec].formid).contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        view.sort_by(|&a, &b| {
            let (ra, rb) = (idx.cells[a].rec, idx.cells[b].rec);
            loaded
                .name_of(ra)
                .0
                .to_lowercase()
                .cmp(&loaded.name_of(rb).0.to_lowercase())
                .then(idx.records[ra].formid.cmp(&idx.records[rb].formid))
        });
        self.cv_view = view;
    }

    /// Menu entries for this tool, placed inside the shell's unified menu bar.
    pub fn menu_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("Fichier", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if ui.button("Ouvrir…").clicked() {
                ui.close_menu();
                self.open_dialog();
            }
            ui.separator();
            if ui.button("Fermer").clicked() {
                ui.close_menu();
                self.loaded = None;
            }
            ui.separator();
            if ui.button("Quitter").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            close_menu_if_pointer_left(ui, None);
        });
        ui.menu_button("À propos", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.label("ESM Explorer");
            ui.label("Explorateur de plugins Bethesda (.esp/.esm/.esl)");
            ui.label("Outil de XIMOD Translator");
            close_menu_if_pointer_left(ui, None);
        });
    }

    /// Per-frame UI below the shell's unified menu bar (panels, tables, windows).
    pub fn body(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ctx);

        let mut loaded = self.loaded.take();
        if let Some(l) = &mut loaded {
            self.sync_object_view(ctx, l);
            self.sync_cell_view(ctx, l);
        }

        // ---- Status bar ----
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(l) = &loaded {
                    ui.label(format!("📄 {}", l.file_name));
                    ui.separator();
                    ui.label(l.index.game.display_name());
                    if let Some(fv) = l.index.form_version {
                        ui.separator();
                        ui.label(format!("form ver {fv}"));
                    }
                    ui.separator();
                    if l.index.localized {
                        match &l.lang {
                            Some(lg) => ui.label(format!(
                                "localisé · {} ({} STRINGS)",
                                lang_display(lg),
                                l.strings.len()
                            )),
                            None => ui.label("localisé · STRINGS introuvables"),
                        };
                    } else {
                        ui.label("inline");
                    }
                    ui.separator();
                    ui.label(format!("{} enreg.", l.index.obj_indices.len()));
                    ui.separator();
                    ui.label(format!(
                        "{} worldspaces · {} cellules",
                        l.index.worlds.len(),
                        l.index.cells.len()
                    ));
                    if self.ow_decoding {
                        ui.separator();
                        let base = self.base_objs(l).len().max(1);
                        let pct = (self.ow_cursor * 100 / base).min(100);
                        ui.label(format!("décodage {pct}%"));
                    }
                } else {
                    ui.label("Aucun plugin chargé — Fichier → Ouvrir… ou glisser-déposer");
                }
            });
        });

        // File Explorer (full-height right column), above the bottom Cell View.
        if let Some(l) = &loaded {
            egui::SidePanel::right("fileexplorer")
                .resizable(true)
                .default_width(300.0)
                .min_width(210.0)
                .show(ctx, |ui| self.file_explorer(ui, l));
        }

        if let Some(l) = &mut loaded {
            egui::TopBottomPanel::bottom("cellview")
                .resizable(true)
                .default_height(300.0)
                .min_height(140.0)
                .show(ctx, |ui| self.cell_view(ui, l));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(0xB0, 0x24, 0x18), err);
                ui.separator();
            }
            let mut clear_notice = false;
            if let Some(msg) = &self.notice {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(0x1B, 0x7F, 0x3A), msg);
                    if ui.small_button("✖").clicked() {
                        clear_notice = true;
                    }
                });
                ui.separator();
            }
            if clear_notice {
                self.notice = None;
            }
            match &mut loaded {
                Some(l) => self.object_window(ui, l),
                None => {
                    ui.weak("Ouvrez un plugin pour explorer ses enregistrements.");
                }
            }
        });

        // Record detail (floating window), follows the current selection.
        if let Some(l) = &loaded {
            self.record_detail_window(ctx, l);
        }

        self.loaded = loaded;
        self.draw_drag_hint(ctx);
    }
}

/// Close the (top-level) menu owned by `ui` as soon as the pointer leaves its
/// popup — so a menu opened by a click disappears on mouse-out, without needing a
/// second click. `child` is the rectangle of an open sub-menu popup, if any, so
/// the parent stays open while a child sub-menu is being hovered.
///
/// The keep-open zone is the popup rectangle (unioned with any open child popup),
/// extended upward to bridge the small gap to the menu-bar button, plus a little
/// tolerance around the edges. Nested sub-menus are still closed by egui's own
/// hover handling; only the click-opened top-level menu needs this.
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

/// Rich decoded value of a subrecord for the detail view: resolved STRINGS text
/// for localized string fields, printable text verbatim, small numbers with their
/// integer/float/FormID interpretations, else a short hex preview.
fn field_value(
    sub: &SubRecord,
    rec_sig: &str,
    game: crate::esm::Game,
    localized: bool,
    strings: &StringTable,
) -> String {
    // Localized string id (only for genuinely translatable record/field pairs).
    if localized
        && sub.data.len() == 4
        && games::registry()
            .fields(game)
            .is_translatable(rec_sig, &sub.sig)
    {
        let id = u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]);
        return match strings.get(id) {
            Some(t) => format!("“{}”", t.replace(['\n', '\r'], " ")),
            None => format!("[lstring {id:08X}]"),
        };
    }
    // Printable inline text.
    let z = sub.as_zstring();
    let total = z.chars().count().max(1);
    let no_control = z
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'));
    let has_alnum = z.chars().any(|c| c.is_ascii_alphanumeric());
    let high = z.chars().filter(|&c| (c as u32) >= 0x7F).count();
    if !z.trim().is_empty() && no_control && has_alnum && high * 10 <= total * 4 {
        return format!("“{}”", z.replace(['\n', '\r'], " "));
    }
    match sub.data.len() {
        4 => {
            let u = u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]);
            let f = f32::from_bits(u);
            format!("0x{u:08X}  ·  u32 {u}  ·  f32 {f}  ·  FormID 0x{u:08X}")
        }
        2 => {
            let u = u16::from_le_bytes([sub.data[0], sub.data[1]]);
            format!("0x{u:04X}  ·  u16 {u}")
        }
        1 => format!("0x{:02X}  ·  u8 {}", sub.data[0], sub.data[0]),
        0 => "(vide)".to_string(),
        _ => {
            let preview: String = sub
                .data
                .iter()
                .take(16)
                .map(|b| format!("{b:02X} "))
                .collect();
            format!("{}… ({} octets)", preview.trim_end(), sub.data.len())
        }
    }
}

/// Classic hex dump: 16 bytes per line with offset and ASCII gutter.
fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let off = i * 16;
        let mut hex = String::new();
        let mut ascii = String::new();
        for (j, &b) in chunk.iter().enumerate() {
            hex.push_str(&format!("{b:02X} "));
            if j == 7 {
                hex.push(' ');
            }
            ascii.push(if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        out.push_str(&format!("{off:06X}  {hex:<49} {ascii}\n"));
    }
    out
}

/// Whether `text` would be clipped in the current table cell (its rendered width
/// exceeds the column's available width). Used to add a full-text tooltip only
/// when a row is actually truncated.
fn is_truncated(ui: &egui::Ui, text: &str, mono: bool) -> bool {
    if text.is_empty() {
        return false;
    }
    let style = if mono {
        egui::TextStyle::Monospace
    } else {
        egui::TextStyle::Body
    };
    let font = style.resolve(ui.style());
    let color = ui.style().visuals.text_color();
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, color)
        .size()
        .x;
    width > ui.available_width() + 0.5
}

/// A language token's display name (Bethesda name if known, else capitalized).
fn lang_display(tok: &str) -> String {
    if let Some(name) = lang::bethesda_name(tok) {
        return name.to_string();
    }
    let mut c = tok.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => tok.to_string(),
    }
}

/// Human-readable representation of a subrecord for the text extraction:
/// resolved STRINGS text for localized string fields, printable strings verbatim,
/// 4-byte values as hex (FormID / string id), other binary as a byte count.
///
/// A field is a localized string id only when the `(record, field)` pair is
/// actually translatable for this game — using the signature alone would
/// misread FormID fields (NAME, CNAM…) that share a signature with text fields.
fn field_repr(
    sub: &SubRecord,
    rec_sig: &str,
    game: crate::esm::Game,
    localized: bool,
    strings: &StringTable,
) -> String {
    if localized
        && sub.data.len() == 4
        && games::registry()
            .fields(game)
            .is_translatable(rec_sig, &sub.sig)
    {
        let id = u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]);
        return match strings.get(id) {
            Some(t) => t.replace(['\n', '\r'], " "),
            None => format!("[{id:08X}]"),
        };
    }
    let z = sub.as_zstring();
    let total = z.chars().count().max(1);
    let no_control = z
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'));
    let has_alnum = z.chars().any(|c| c.is_ascii_alphanumeric());
    let high = z.chars().filter(|&c| (c as u32) >= 0x7F).count();
    // Text if it looks like real text: no control bytes, contains ASCII letters,
    // and is not mostly high bytes (rejects short binary that decodes to Latin-1).
    let printable = !z.trim().is_empty() && no_control && has_alnum && high * 10 <= total * 4;
    if printable {
        z.replace(['\n', '\r'], " ")
    } else if sub.data.len() == 4 {
        format!(
            "0x{:08X}",
            u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]])
        )
    } else {
        format!("<{} octets>", sub.data.len())
    }
}

impl EsmExplorerApp {
    fn object_window(&mut self, ui: &mut egui::Ui, loaded: &mut Loaded) {
        ui.label(egui::RichText::new("ObjectWindow").weak());
        let mut new_lang: Option<String> = None;
        // `horizontal_wrapped` so the language selector wraps to a second line on
        // a narrow Object Window instead of being clipped off-screen.
        ui.horizontal_wrapped(|ui| {
            ui.label("Filtre :");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("EDID / Nom / FormID / Type")
                    .desired_width(220.0),
            );
            if !self.filter.is_empty() && ui.button("✖").clicked() {
                self.filter.clear();
            }
            // Language control for localized plugins — always shown (never
            // silently hidden) so the user can see the source language / that no
            // STRINGS were found. Inline plugins carry a single embedded language,
            // so no selector is offered for them.
            if loaded.index.localized {
                ui.separator();
                ui.label("Langue :");
                if loaded.languages.is_empty() {
                    ui.weak("(aucune STRINGS trouvée)");
                } else {
                    let current = loaded
                        .lang
                        .as_deref()
                        .map(lang_display)
                        .unwrap_or_else(|| "—".to_string());
                    egui::ComboBox::from_id_salt("strings_lang")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for l in &loaded.languages {
                                let selected = loaded.lang.as_deref() == Some(l.as_str());
                                if ui.selectable_label(selected, lang_display(l)).clicked() {
                                    new_lang = Some(l.clone());
                                }
                            }
                        });
                }
            }
        });
        ui.add_space(4.0);

        if let Some(l) = new_lang {
            self.set_language(loaded, &l);
        }

        let mut clicked_sort: Option<SortCol> = None;
        let mut clicked_type: Option<Option<String>> = None;
        let mut clicked_record: Option<usize> = None;

        ui.horizontal_top(|ui| {
            // Left: type category list.
            ui.vertical(|ui| {
                ui.set_width(160.0);
                egui::ScrollArea::vertical()
                    .id_salt("types")
                    .show(ui, |ui| {
                        if ui
                            .selectable_label(
                                self.type_filter.is_none(),
                                format!("Tous ({})", loaded.index.obj_indices.len()),
                            )
                            .clicked()
                        {
                            clicked_type = Some(None);
                        }
                        for (sig, count) in &loaded.index.types {
                            let selected = self.type_filter.as_deref() == Some(sig.as_str());
                            if ui
                                .selectable_label(selected, format!("{sig} ({count})"))
                                .clicked()
                            {
                                clicked_type = Some(Some(sig.clone()));
                            }
                        }
                    });
            });
            ui.separator();
            // Right: record table.
            ui.vertical(|ui| {
                let sort_col = self.sort_col;
                let sort_asc = self.sort_asc;
                let arrow = |col: SortCol| -> &'static str {
                    if sort_col == col {
                        if sort_asc { " ▲" } else { " ▼" }
                    } else {
                        ""
                    }
                };
                let mut header_btn = |ui: &mut egui::Ui, label: &str, col: SortCol| {
                    let txt = format!("{label}{}", arrow(col));
                    if ui
                        .add(
                            egui::Label::new(egui::RichText::new(txt).strong())
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        clicked_sort = Some(col);
                    }
                };
                let view = &self.ow_view;
                TableBuilder::new(ui)
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(
                        Column::initial(170.0)
                            .at_least(60.0)
                            .clip(true)
                            .resizable(true),
                    )
                    .column(
                        Column::initial(80.0)
                            .at_least(60.0)
                            .clip(true)
                            .resizable(true),
                    )
                    .column(
                        Column::initial(200.0)
                            .at_least(60.0)
                            .clip(true)
                            .resizable(true),
                    )
                    .column(Column::remainder().at_least(50.0).clip(true))
                    .header(20.0, |mut header| {
                        header.col(|ui| header_btn(ui, "EDID", SortCol::Edid));
                        header.col(|ui| header_btn(ui, "Form ID", SortCol::FormId));
                        header.col(|ui| header_btn(ui, "Name", SortCol::Name));
                        header.col(|ui| header_btn(ui, "Type", SortCol::Type));
                    })
                    .body(|body| {
                        body.rows(18.0, view.len(), |mut row| {
                            let i = view[row.index()];
                            loaded.ensure_name(i);
                            let (edid, name) = loaded.name_of(i);
                            let rec = &loaded.index.records[i];
                            let selected = self.selected_record == Some(i);
                            row.set_selected(selected);
                            let mut hit = false;
                            row.col(|ui| {
                                let trunc = is_truncated(ui, edid, false);
                                let r = ui.selectable_label(selected, edid);
                                let r = if trunc { r.on_hover_text(edid) } else { r };
                                hit |= r.clicked();
                            });
                            row.col(|ui| {
                                hit |= ui
                                    .selectable_label(selected, format!("{:08X}", rec.formid))
                                    .clicked();
                            });
                            row.col(|ui| {
                                let trunc = is_truncated(ui, name, false);
                                let r = ui.selectable_label(selected, name);
                                let r = if trunc { r.on_hover_text(name) } else { r };
                                hit |= r.clicked();
                            });
                            row.col(|ui| {
                                hit |= ui.selectable_label(selected, rec.sig_str()).clicked();
                            });
                            if hit {
                                clicked_record = Some(i);
                            }
                        });
                    });
            });
        });

        if let Some(col) = clicked_sort {
            if self.sort_col == col {
                self.sort_asc = !self.sort_asc;
            } else {
                self.sort_col = col;
                self.sort_asc = true;
            }
        }
        if let Some(t) = clicked_type {
            self.type_filter = t;
        }
        if let Some(i) = clicked_record {
            self.selected_record = Some(i);
            self.show_detail = true;
        }
    }

    fn cell_view(&mut self, ui: &mut egui::Ui, loaded: &mut Loaded) {
        ui.label(egui::RichText::new("CellView").weak());

        let mut new_world: Option<Option<u32>> = None;
        ui.horizontal(|ui| {
            ui.label("World Space");
            let current_label = match self.selected_world {
                None => "Interiors".to_string(),
                Some(w) => self.world_label(loaded, w),
            };
            egui::ComboBox::from_id_salt("worldspace")
                .selected_text(current_label)
                .width(240.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.selected_world.is_none(), "Interiors")
                        .clicked()
                    {
                        new_world = Some(None);
                    }
                    for &wi in &loaded.index.worlds {
                        let formid = loaded.index.records[wi].formid;
                        let label = self.world_label(loaded, formid);
                        if ui
                            .selectable_label(self.selected_world == Some(formid), label)
                            .clicked()
                        {
                            new_world = Some(Some(formid));
                        }
                    }
                });

            ui.separator();
            ui.label("Filtre cellule :");
            ui.add(
                egui::TextEdit::singleline(&mut self.cell_filter)
                    .hint_text("EditorID / Nom / FormID")
                    .desired_width(220.0),
            );
            if !self.cell_filter.is_empty() && ui.button("✖").clicked() {
                self.cell_filter.clear();
            }
        });
        ui.add_space(4.0);

        let mut clicked_cell: Option<u32> = None;
        let mut clicked_cell_rec: Option<usize> = None;
        let mut clicked_ref: Option<usize> = None;

        // Split the two tables with nested panels rather than `ui.columns`: a
        // panel clips its own region, so a wide EditorID/Name column is truncated
        // at the divider instead of overprinting the references table beside it.
        let refs_w = (ui.available_width() * 0.42).clamp(220.0, 640.0);
        egui::SidePanel::right("cv_refs")
            .resizable(true)
            .default_width(refs_w)
            .min_width(160.0)
            .show_inside(ui, |ui| {
                let refs = &self.sel_refs;
                TableBuilder::new(ui)
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(
                        Column::initial(150.0)
                            .at_least(60.0)
                            .clip(true)
                            .resizable(true),
                    )
                    .column(
                        Column::initial(80.0)
                            .at_least(60.0)
                            .clip(true)
                            .resizable(true),
                    )
                    .column(Column::remainder().at_least(60.0).clip(true))
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.strong("EditorID");
                        });
                        header.col(|ui| {
                            ui.strong("FormID");
                        });
                        header.col(|ui| {
                            ui.strong("Type");
                        });
                    })
                    .body(|body| {
                        body.rows(18.0, refs.len(), |mut row| {
                            let r = &refs[row.index()];
                            let selected = self.selected_record == Some(r.rec);
                            row.set_selected(selected);
                            let mut hit = false;
                            row.col(|ui| {
                                let trunc = is_truncated(ui, &r.edid, false);
                                let resp = ui.selectable_label(selected, &r.edid);
                                let resp = if trunc {
                                    resp.on_hover_text(&r.edid)
                                } else {
                                    resp
                                };
                                hit |= resp.clicked();
                            });
                            row.col(|ui| {
                                hit |= ui
                                    .selectable_label(selected, format!("{:08X}", r.formid))
                                    .clicked();
                            });
                            row.col(|ui| {
                                let trunc = is_truncated(ui, &r.type_sig, true);
                                let resp = ui.selectable_label(selected, &r.type_sig);
                                let resp = if trunc {
                                    resp.on_hover_text(&r.type_sig)
                                } else {
                                    resp
                                };
                                hit |= resp.clicked();
                            });
                            if hit {
                                clicked_ref = Some(r.rec);
                            }
                        });
                    });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let view = &self.cv_view;
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(
                    Column::initial(150.0)
                        .at_least(60.0)
                        .clip(true)
                        .resizable(true),
                )
                .column(
                    Column::initial(70.0)
                        .at_least(50.0)
                        .clip(true)
                        .resizable(true),
                )
                .column(
                    Column::initial(150.0)
                        .at_least(60.0)
                        .clip(true)
                        .resizable(true),
                )
                .column(Column::remainder().at_least(60.0).clip(true))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("EditorID");
                    });
                    header.col(|ui| {
                        ui.strong("FormID");
                    });
                    header.col(|ui| {
                        ui.strong("Name");
                    });
                    header.col(|ui| {
                        ui.strong("Location");
                    });
                })
                .body(|body| {
                    body.rows(18.0, view.len(), |mut row| {
                        let ci = view[row.index()];
                        let rec = loaded.index.cells[ci].rec;
                        let formid = loaded.index.records[rec].formid;
                        // Morrowind cells have no FormID (all 0); key selection and
                        // reference lookup by the record index instead.
                        let key = if loaded.index.tes3 {
                            rec as u32
                        } else {
                            formid
                        };
                        let (edid, name) = loaded.name_of(rec);
                        let loc = loaded
                            .coords
                            .get(&rec)
                            .and_then(|c| *c)
                            .map(|(x, y)| format!("({x}, {y})"))
                            .unwrap_or_default();
                        let selected = self.selected_cell == Some(key);
                        row.set_selected(selected);
                        let mut hit = false;
                        row.col(|ui| {
                            let trunc = is_truncated(ui, edid, false);
                            let r = ui.selectable_label(selected, edid);
                            let r = if trunc { r.on_hover_text(edid) } else { r };
                            hit |= r.clicked();
                        });
                        row.col(|ui| {
                            hit |= ui
                                .selectable_label(selected, format!("{formid:08X}"))
                                .clicked();
                        });
                        row.col(|ui| {
                            let trunc = is_truncated(ui, name, false);
                            let r = ui.selectable_label(selected, name);
                            let r = if trunc { r.on_hover_text(name) } else { r };
                            hit |= r.clicked();
                        });
                        row.col(|ui| {
                            let trunc = is_truncated(ui, &loc, false);
                            let r = ui.selectable_label(selected, &loc);
                            let r = if trunc { r.on_hover_text(&loc) } else { r };
                            hit |= r.clicked();
                        });
                        if hit {
                            clicked_cell = Some(key);
                            clicked_cell_rec = Some(rec);
                        }
                    });
                });
        });

        if let Some(w) = new_world {
            self.selected_world = w;
            self.selected_cell = None;
        }
        if let Some(c) = clicked_cell {
            self.selected_cell = Some(c);
        }
        // A cell click updates the selected record so the detail window follows,
        // but does not force it open (cell clicks primarily drive the ref list).
        if let Some(rec) = clicked_cell_rec {
            self.selected_record = Some(rec);
        }
        // A reference click opens the detail window on that record.
        if let Some(rec) = clicked_ref {
            self.selected_record = Some(rec);
            self.show_detail = true;
        }
    }

    fn file_explorer(&mut self, ui: &mut egui::Ui, loaded: &Loaded) {
        // Header: title, select all/none, filter.
        egui::TopBottomPanel::top("fe_head").show_inside(ui, |ui| {
            ui.label(egui::RichText::new("FileExplorer").weak());
            ui.horizontal(|ui| {
                if ui.button("Select All").clicked() {
                    self.fe_selected = loaded.index.types.iter().map(|(s, _)| s.clone()).collect();
                }
                if ui.button("Select None").clicked() {
                    self.fe_selected.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Filtre :");
                ui.add(
                    egui::TextEdit::singleline(&mut self.fe_filter)
                        .hint_text("GRUP")
                        .desired_width(120.0),
                );
                if !self.fe_filter.is_empty() && ui.button("✖").clicked() {
                    self.fe_filter.clear();
                }
            });
        });

        // Footer: selection summary + Extract (always visible at the bottom).
        egui::TopBottomPanel::bottom("fe_foot").show_inside(ui, |ui| {
            self.file_explorer_footer(ui, loaded);
        });

        // Middle: the scrolling checkbox list of GRUP types.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let needle = self.fe_filter.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("fe_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (sig, count) in &loaded.index.types {
                        if !needle.is_empty() && !sig.to_lowercase().contains(&needle) {
                            continue;
                        }
                        let mut checked = self.fe_selected.contains(sig);
                        if ui
                            .checkbox(&mut checked, format!("{sig} ({count})"))
                            .changed()
                        {
                            if checked {
                                self.fe_selected.insert(sig.clone());
                            } else {
                                self.fe_selected.remove(sig);
                            }
                        }
                    }
                });
        });
    }

    /// Bottom bar of the File Explorer (selection summary + Extract button),
    /// drawn as its own bottom strip so it stays visible under the list.
    fn file_explorer_footer(&mut self, ui: &mut egui::Ui, loaded: &Loaded) {
        let sel_records: usize = loaded
            .index
            .types
            .iter()
            .filter(|(s, _)| self.fe_selected.contains(s))
            .map(|(_, c)| c)
            .sum();
        ui.label(format!(
            "{} type(s) · {sel_records} enreg.",
            self.fe_selected.len()
        ));
        let enabled = sel_records > 0;
        let stem = Path::new(&loaded.file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extract".to_string());
        ui.horizontal(|ui| {
            ui.label("Extraire :");
            if ui
                .add_enabled(enabled, egui::Button::new("Fichier texte (.txt)"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{stem}_extract.txt"))
                    .add_filter("Texte", &["txt"])
                    .save_file()
            {
                match self.extract_types(loaded, &path) {
                    Ok(n) => {
                        self.fe_status = Some(format!("Extrait {n} enreg. → {}", path.display()));
                    }
                    Err(e) => self.fe_status = Some(format!("Erreur d’extraction : {e}")),
                }
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Tableur (.xlsx)"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{stem}_extract.xlsx"))
                    .add_filter("Classeur Excel / LibreOffice Calc", &["xlsx"])
                    .save_file()
            {
                match Self::export_xlsx(loaded, &path, &self.fe_selected) {
                    Ok((sheets, rows)) => {
                        self.fe_status = Some(format!(
                            "Tableur : {sheets} feuille(s), {rows} ligne(s) → {}",
                            path.display()
                        ));
                    }
                    Err(e) => self.fe_status = Some(format!("Erreur d’export tableur : {e}")),
                }
            }
        });
        if let Some(s) = &self.fe_status {
            ui.label(s);
        }
    }

    /// Write every record of the selected types to a readable text file.
    fn extract_types(&self, loaded: &Loaded, path: &Path) -> std::io::Result<usize> {
        let file = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(file);
        writeln!(w, "# Extraction — {}", loaded.file_name)?;
        writeln!(w, "# Jeu : {}", loaded.index.game.display_name())?;
        writeln!(w, "# Types sélectionnés : {}", self.fe_selected.len())?;
        writeln!(w, "# Généré par ESM Explorer")?;
        writeln!(w)?;

        let data = &loaded.mmap[..];
        let mut total = 0usize;
        for (sig, count) in &loaded.index.types {
            if !self.fe_selected.contains(sig) {
                continue;
            }
            writeln!(w, "== {sig} ({count}) ==")?;
            if let Some(list) = loaded.index.type_lists.get(sig) {
                for &i in list {
                    let rec = &loaded.index.records[i];
                    let rec_sig = rec.sig_str();
                    let fields = loaded.index.record_fields(data, i);
                    let edid = fields
                        .iter()
                        .find(|s| s.sig == "EDID")
                        .map(|s| s.as_zstring())
                        .unwrap_or_default();
                    writeln!(w, "[0x{:08X}] {edid}", rec.formid)?;
                    for sub in &fields {
                        if sub.sig == "EDID" {
                            continue;
                        }
                        writeln!(
                            w,
                            "    {}: {}",
                            sub.sig,
                            field_repr(
                                sub,
                                &rec_sig,
                                loaded.index.game,
                                loaded.index.localized,
                                &loaded.strings
                            )
                        )?;
                    }
                    total += 1;
                }
            }
            writeln!(w)?;
        }
        w.flush()?;
        Ok(total)
    }

    /// Reload the STRINGS for another language and invalidate name caches/views.
    fn set_language(&mut self, loaded: &mut Loaded, langtok: &str) {
        let load = stringtable::load(&loaded.plugin_path, loaded.index.game, langtok);
        loaded.strings = load.table;
        loaded.lang = load.chosen;
        loaded.strings_sources = load.sources;
        // Names embed resolved text — drop the caches and force view rebuilds.
        loaded.names.clear();
        self.ow_key = None;
        self.ow_base_type = None;
        self.ow_cursor = 0;
        self.cv_decoded_for = None;
        self.cv_key = None;
    }

    /// Floating window detailing the currently selected record: header facts and
    /// every subrecord (signature, size, decoded value), plus an optional hex view.
    fn record_detail_window(&mut self, ctx: &egui::Context, loaded: &Loaded) {
        let Some(i) = self.selected_record else {
            return;
        };
        if !self.show_detail || i >= loaded.index.records.len() {
            return;
        }
        let rec = &loaded.index.records[i];
        let rec_sig = rec.sig_str();
        let formid = rec.formid;
        let flags = rec.flags;
        let compressed = rec.compressed();
        let localized = loaded.index.localized;
        let game = loaded.index.game;
        let fields = loaded.index.record_fields(&loaded.mmap[..], i);

        let mut open = self.show_detail;
        let mut detail_hex = self.detail_hex;
        egui::Window::new(format!("Détail — {rec_sig} [0x{formid:08X}]"))
            .open(&mut open)
            .resizable(true)
            .default_width(580.0)
            .default_height(420.0)
            .show(ctx, |ui| {
                // Header facts.
                ui.horizontal(|ui| {
                    ui.strong("Signature :");
                    ui.monospace(&rec_sig);
                    ui.separator();
                    ui.strong("FormID :");
                    ui.monospace(format!("0x{formid:08X}"));
                });
                ui.horizontal(|ui| {
                    ui.strong("Flags :");
                    ui.monospace(format!("0x{flags:08X}"));
                    if compressed {
                        ui.label("• compressé");
                    }
                    ui.separator();
                    ui.strong("Sous-champs :");
                    ui.label(format!("{}", fields.len()));
                });
                ui.separator();

                // Subrecords table.
                egui::ScrollArea::vertical()
                    .id_salt("detail_fields")
                    .max_height(if detail_hex { 200.0 } else { 320.0 })
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                            .column(Column::auto().at_least(48.0))
                            .column(Column::auto().at_least(56.0))
                            .column(Column::remainder().at_least(200.0).clip(true))
                            .header(18.0, |mut h| {
                                h.col(|ui| {
                                    ui.strong("Champ");
                                });
                                h.col(|ui| {
                                    ui.strong("Taille");
                                });
                                h.col(|ui| {
                                    ui.strong("Valeur");
                                });
                            })
                            .body(|body| {
                                body.rows(18.0, fields.len(), |mut row| {
                                    let sub = &fields[row.index()];
                                    let val = field_value(
                                        sub,
                                        &rec_sig,
                                        game,
                                        localized,
                                        &loaded.strings,
                                    );
                                    row.col(|ui| {
                                        ui.monospace(&sub.sig);
                                    });
                                    row.col(|ui| {
                                        ui.monospace(format!("{}", sub.data.len()));
                                    });
                                    row.col(|ui| {
                                        let trunc = is_truncated(ui, &val, false);
                                        let r = ui.label(&val);
                                        if trunc {
                                            r.on_hover_text(&val);
                                        }
                                    });
                                });
                            });
                    });

                ui.separator();
                ui.checkbox(&mut detail_hex, "Vue hexadécimale du corps");
                if detail_hex {
                    let body = loaded.index.record_body(&loaded.mmap[..], i);
                    let shown = body.len().min(4096);
                    let dump = hex_dump(&body[..shown]);
                    egui::ScrollArea::vertical()
                        .id_salt("detail_hex")
                        .max_height(200.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(dump).monospace().size(12.0))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                    if body.len() > shown {
                        ui.weak(format!("… {} octets sur {} affichés", shown, body.len()));
                    }
                }
            });
        self.show_detail = open;
        self.detail_hex = detail_hex;
    }

    /// Headless spreadsheet export (used by the `--xlsx` CLI mode and tests).
    pub fn run_xlsx(plugin: &Path, out: &Path) -> Result<(usize, usize), String> {
        let file = std::fs::File::open(plugin).map_err(|e| e.to_string())?;
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| e.to_string())?;
        let index = PluginIndex::build(&mmap[..]).map_err(|e| e.to_string())?;
        let file_name = plugin
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (strings, languages, lang, strings_sources) = if index.localized {
            let load = stringtable::load(plugin, index.game, "English");
            (load.table, load.languages, load.chosen, load.sources)
        } else {
            (StringTable::default(), Vec::new(), None, Vec::new())
        };
        let loaded = Loaded {
            file_name,
            plugin_path: plugin.to_path_buf(),
            mmap,
            index,
            names: HashMap::new(),
            coords: HashMap::new(),
            strings,
            languages,
            lang,
            strings_sources,
        };
        // Headless default: the translatable types for the game — Morrowind's
        // text-bearing records for a TES3 plugin, else the standard item types.
        let default_types: &[&str] = if loaded.index.tes3 {
            TES3_TEXT_TYPES
        } else {
            ITEM_TYPES
        };
        let types: HashSet<String> = default_types.iter().map(|s| s.to_string()).collect();
        Self::export_xlsx(&loaded, out, &types)
    }

    /// Build the spreadsheet: one sheet per selected record type, columns
    /// FormID / EditorID / Nom / Description / Modèle / Icône / Catégorie. Weapons
    /// are sorted by their resolved function category. Only the record types in
    /// `types` (the File Explorer selection) are exported. Returns (sheets, rows).
    fn export_xlsx(
        loaded: &Loaded,
        path: &Path,
        types: &HashSet<String>,
    ) -> Result<(usize, usize), String> {
        let data = &loaded.mmap[..];
        let game = loaded.index.game;
        let localized = loaded.index.localized;
        let strings = &loaded.strings;

        // Keyword map (weapon categories). Keywords may come from the base master
        // (the common case for a mod), or — when the loaded file *is* a base master
        // such as Starfield.esm — from the plugin's own KYWD records. We merge both,
        // the plugin's own definitions taking priority.
        let data_dir = loaded
            .plugin_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut km = keywords::load(&data_dir, &loaded.index.masters, game);
        let mut own_keywords = 0usize;
        if let Some(kywd_list) = loaded.index.type_lists.get("KYWD") {
            for &i in kywd_list {
                let rec = &loaded.index.records[i];
                let fields = loaded.index.record_fields(data, i);
                if let Some(edid) = fields.iter().find(|s| s.sig == "EDID") {
                    km.insert(rec.formid, edid.as_zstring());
                    own_keywords += 1;
                }
            }
        }
        // Whether categories can be resolved at all: either an external base master
        // was parsed, or the loaded file defines keywords itself.
        let have_keywords = km.base_master.is_some() || own_keywords > 0;

        // Resolve a (possibly localized) string field to text.
        let resolve_str = |fields: &[SubRecord], sig: &str, look: &str| -> String {
            let Some(sub) = fields.iter().find(|s| s.sig == look) else {
                return String::new();
            };
            if localized
                && games::registry().fields(game).is_translatable(sig, look)
                && sub.data.len() == 4
            {
                let id = sub.as_u32().unwrap_or(0);
                return strings.get(id).map(str::to_string).unwrap_or_default();
            }
            sub.as_zstring()
        };
        let find_path = |fields: &[SubRecord], sigs: &[&str]| -> String {
            for s in sigs {
                if let Some(sub) = fields.iter().find(|x| x.sig == *s) {
                    let z = sub.as_zstring();
                    if !z.is_empty() {
                        return z;
                    }
                }
            }
            String::new()
        };

        struct Row {
            formid: String,
            edid: String,
            name: String,
            desc: String,
            model: String,
            icon: String,
            cat: String,
        }

        let mut wb = Workbook::new();
        let hdr = Format::new().set_bold();
        let headers = [
            "FormID",
            "EditorID",
            "Nom",
            "Description",
            "Modèle",
            "Icône",
            "Catégorie",
        ];
        let widths = [12.0, 26.0, 28.0, 42.0, 34.0, 24.0, 28.0];
        let mut sheets = 0usize;
        let mut total = 0usize;

        for (sig, _count) in &loaded.index.types {
            if !types.contains(sig) {
                continue;
            }
            let Some(list) = loaded.index.type_lists.get(sig) else {
                continue;
            };
            let mut rows: Vec<Row> = Vec::with_capacity(list.len());
            for &i in list {
                let rec = &loaded.index.records[i];
                let fields = loaded.index.record_fields(data, i);
                // Morrowind (TES3): different subrecords carry id and text. Map them
                // for translation — id in EditorID, the record's translatable text in
                // Nom / Description (FNAM name; DESC/TEXT/STRV/INFO-response body).
                if loaded.index.tes3 {
                    let get = |k: &str| {
                        fields
                            .iter()
                            .find(|s| s.sig == k)
                            .map(|s| s.as_zstring())
                            .unwrap_or_default()
                    };
                    let edid = if sig == "INFO" {
                        get("INAM")
                    } else {
                        get("NAME")
                    };
                    let mut name = get("FNAM");
                    if name.is_empty() && sig == "DIAL" {
                        name = get("NAME"); // dialogue topic text
                    }
                    let mut desc = get("DESC");
                    if desc.is_empty() {
                        desc = get("TEXT"); // book body
                    }
                    if desc.is_empty() {
                        desc = get("STRV"); // GMST string value
                    }
                    if desc.is_empty() && sig == "INFO" {
                        desc = get("NAME"); // dialogue response text
                    }
                    rows.push(Row {
                        formid: String::new(), // Morrowind has no FormID
                        edid,
                        name,
                        desc,
                        model: find_path(&fields, &["MODL"]),
                        icon: find_path(&fields, &["ITEX", "ICON"]),
                        cat: String::new(),
                    });
                    continue;
                }
                let edid = fields
                    .iter()
                    .find(|s| s.sig == "EDID")
                    .map(|s| s.as_zstring())
                    .unwrap_or_default();
                let name = resolve_str(&fields, sig, "FULL");
                let desc = resolve_str(&fields, sig, "DESC");
                let model = find_path(&fields, &["MODL", "MOD2", "MOD3", "MOD4"]);
                let icon = find_path(&fields, &["ICON", "MICO"]);
                let cat = if sig == "WEAP" {
                    let mut edids: Vec<String> = Vec::new();
                    if let Some(kwda) = fields.iter().find(|s| s.sig == "KWDA") {
                        for c in kwda.data.as_chunks::<4>().0.iter() {
                            let k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                            if let Some(e) = km.resolve(k) {
                                edids.push(e.to_string());
                            }
                        }
                    }
                    match category::weapon_category(&edids) {
                        Some(c) => c.to_string(),
                        // No category resolved. Distinguish "we had no keyword data
                        // at all to work with" (master absent and the plugin defines
                        // none) from "we had keywords but this weapon's don't map to
                        // a known function".
                        None if !have_keywords => "(master de base introuvable)".to_string(),
                        None => "Non classé".to_string(),
                    }
                } else {
                    String::new()
                };
                rows.push(Row {
                    formid: format!("0x{:08X}", rec.formid),
                    edid,
                    name,
                    desc,
                    model,
                    icon,
                    cat,
                });
            }
            if sig == "WEAP" {
                rows.sort_by(|a, b| {
                    a.cat
                        .cmp(&b.cat)
                        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            } else {
                rows.sort_by_key(|r| r.name.to_lowercase());
            }

            let ws = wb.add_worksheet();
            ws.set_name(sig.as_str()).map_err(|e| e.to_string())?;
            for (c, h) in headers.iter().enumerate() {
                ws.write_string_with_format(0, c as u16, *h, &hdr)
                    .map_err(|e| e.to_string())?;
            }
            for (c, w) in widths.iter().enumerate() {
                let _ = ws.set_column_width(c as u16, *w);
            }
            for (r, row) in rows.iter().enumerate() {
                let rr = (r + 1) as u32;
                let _ = ws.write_string(rr, 0, &row.formid);
                let _ = ws.write_string(rr, 1, &row.edid);
                let _ = ws.write_string(rr, 2, &row.name);
                let _ = ws.write_string(rr, 3, &row.desc);
                let _ = ws.write_string(rr, 4, &row.model);
                let _ = ws.write_string(rr, 5, &row.icon);
                let _ = ws.write_string(rr, 6, &row.cat);
            }
            sheets += 1;
            total += rows.len();
        }

        if sheets == 0 {
            let ws = wb.add_worksheet();
            let _ = ws.set_name("Info");
            let _ = ws.write_string(0, 0, "Aucun enregistrement pour les types sélectionnés.");
        }
        wb.save(path).map_err(|e| e.to_string())?;
        Ok((sheets, total))
    }

    fn world_label(&self, loaded: &Loaded, formid: u32) -> String {
        // Find the worldspace record for this FormID and use its decoded name.
        let rec = loaded
            .index
            .worlds
            .iter()
            .copied()
            .find(|&wi| loaded.index.records[wi].formid == formid);
        match rec {
            Some(wi) => {
                let (edid, name) = loaded.name_of(wi);
                if !name.is_empty() {
                    format!("{name} [{edid}]")
                } else if !edid.is_empty() {
                    edid.to_string()
                } else {
                    format!("{formid:08X}")
                }
            }
            None => format!("{formid:08X}"),
        }
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
        // Pointer over an open child sub-menu to the right → keep open.
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
