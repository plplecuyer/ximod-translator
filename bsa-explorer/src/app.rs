//! BSA/BA2 Explorer — the egui application: several archives open at once (tabs),
//! a folder tree, a virtualized file table with checkboxes, a text/image preview
//! pane (including DDS textures), and extraction preserving the internal tree.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use crate::archive::Archive;
use crate::audio::{self, Converter};
use crate::compare::{self, Diff, Status};
use crate::preview::{self, Preview};
use crate::tree::{self, TreeNode};

/// A play/transport command emitted by the audio preview UI, applied after the
/// panels are drawn (to keep `&mut self` borrows simple).
enum AudioCmd {
    Play,
    PauseToggle,
    Stop,
    Seek(f32),
}

/// UI icons (PNG, embedded in the binary), loaded once as egui textures. Replaces
/// the Unicode glyphs (✖, ▸/▾, ✓, —) that don't render in every font.
#[derive(Clone)]
struct Icons {
    chevron_right: egui::TextureHandle,
    chevron_down: egui::TextureHandle,
    delete: egui::TextureHandle,
    ok: egui::TextureHandle,
    em_dash: egui::TextureHandle,
}

impl Icons {
    fn load(ctx: &egui::Context) -> Self {
        fn tex(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
            let img = image::load_from_memory(bytes)
                .expect("icône PNG intégrée invalide")
                .to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, &img);
            ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
        }
        Self {
            chevron_right: tex(
                ctx,
                "ico_chevron_right",
                include_bytes!("../assets/images/icons/chevron_right.png"),
            ),
            chevron_down: tex(
                ctx,
                "ico_chevron_down",
                include_bytes!("../assets/images/icons/chevron_down.png"),
            ),
            delete: tex(
                ctx,
                "ico_delete",
                include_bytes!("../assets/images/icons/delete.png"),
            ),
            ok: tex(
                ctx,
                "ico_ok",
                include_bytes!("../assets/images/icons/ok.png"),
            ),
            em_dash: tex(
                ctx,
                "ico_em_dash",
                include_bytes!("../assets/images/icons/em_dash.png"),
            ),
        }
    }
}

/// A small, borderless image button of a fixed square size. Returns the response.
fn icon_button(ui: &mut egui::Ui, tex: &egui::TextureHandle, side: f32) -> egui::Response {
    ui.add(
        egui::ImageButton::new(egui::Image::new(tex).fit_to_exact_size(egui::vec2(side, side)))
            .frame(false),
    )
}

/// A non-interactive icon of a fixed square size (for the "compressed" column).
fn icon_image(ui: &mut egui::Ui, tex: &egui::TextureHandle, side: f32) {
    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(side, side)));
}

/// One open archive plus its per-tab UI state.
struct Loaded {
    archive: Archive,
    tree: TreeNode,
    /// Selected folder (full `/`-path; empty = root).
    selected_folder: String,
    /// Expanded folder full-paths in the tree.
    expanded: HashSet<String>,
    /// Text filter over file paths (case-insensitive). Non-empty → searches the
    /// whole archive, not just the selected folder.
    filter: String,
    /// Include sub-folders of the selected folder in the table.
    recursive: bool,
    /// Entry indices ticked for extraction.
    checked: HashSet<usize>,
    /// Entry index shown in the preview pane.
    selected_file: Option<usize>,
    /// Entry indices currently displayed in the table (recomputed each frame).
    visible: Vec<usize>,
    /// Which entry `preview` was built for (to rebuild lazily on change).
    preview_for: Option<usize>,
    preview: PreviewView,
}

impl Loaded {
    fn new(archive: Archive) -> Self {
        let tree = tree::build(archive.entries());
        Self {
            archive,
            tree,
            selected_folder: String::new(),
            expanded: HashSet::new(),
            filter: String::new(),
            recursive: false,
            checked: HashSet::new(),
            selected_file: None,
            visible: Vec::new(),
            preview_for: None,
            preview: PreviewView::None,
        }
    }

    /// Recompute the list of visible entry indices from folder/filter/recursive.
    fn recompute_visible(&mut self) {
        let entries = self.archive.entries();
        let filter = self.filter.trim().to_lowercase();
        let mut v: Vec<usize> = if !filter.is_empty() {
            (0..entries.len())
                .filter(|&i| entries[i].path.to_lowercase().contains(&filter))
                .collect()
        } else if self.selected_folder.is_empty() {
            // Root selected → show the whole archive (files live under folders).
            (0..entries.len()).collect()
        } else if let Some(node) = tree::find(&self.tree, &self.selected_folder) {
            if self.recursive {
                let mut out = Vec::new();
                tree::subtree_files(node, &mut out);
                out
            } else {
                node.files.clone()
            }
        } else {
            Vec::new()
        };
        v.sort_by(|&a, &b| entries[a].path.cmp(&entries[b].path));
        self.visible = v;
    }
}

/// What the preview pane currently shows.
enum PreviewView {
    None,
    Text {
        body: String,
        truncated: bool,
    },
    Image {
        texture: egui::TextureHandle,
        note: String,
    },
    Audio {
        ext: String,
    },
    Unsupported {
        ext: String,
        size: usize,
    },
    Error(String),
}

#[derive(Default)]
pub struct BsaExplorerApp {
    archives: Vec<Loaded>,
    active: usize,
    error: Option<String>,
    notice: Option<String>,
    icons: Option<Icons>,

    // --- Audio preview state ---
    /// External decoder found once at startup (vgmstream / ffmpeg / none).
    converter: Option<Converter>,
    converter_checked: bool,
    /// Whether xWMAEncode is available (decodes every xWMA, incl. Skyrim LE).
    /// Detected once alongside `converter`; lets `.xwm`/`.fuz` play even when it
    /// is the only tool present.
    has_xwmaencode: bool,
    /// System audio output; created lazily, `None` if unavailable (e.g. headless).
    audio_out: Option<(OutputStream, OutputStreamHandle)>,
    /// Current playback sink and the clip's total duration.
    sink: Option<Sink>,
    audio_total: Option<Duration>,
    /// Cached decoded WAV for the current entry: (key, bytes).
    audio_cache: Option<(String, Vec<u8>)>,
    /// Status/error line shown in the audio panel.
    audio_status: Option<String>,
    /// Transport command from the UI, applied after drawing.
    audio_cmd: Option<AudioCmd>,

    // --- Archive comparison ---
    show_compare: bool,
    compare_center: bool,
    compare_a: usize,
    compare_b: usize,
    compare_result: Option<Diff>,
    compare_filter: String,
    /// Category visibility: [Added, Modified, Removed, Unchanged].
    compare_cat: [bool; 4],
}

impl BsaExplorerApp {
    /// Open a path from the command line / startup.
    pub fn open_path(&mut self, path: &Path) {
        self.load(path.to_path_buf());
    }

    fn load(&mut self, path: PathBuf) {
        // If already open, just activate it.
        if let Some(pos) = self.archives.iter().position(|l| l.archive.path() == path) {
            self.active = pos;
            return;
        }
        match Archive::open(&path) {
            Ok(a) => {
                self.archives.push(Loaded::new(a));
                self.active = self.archives.len() - 1;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Ouverture impossible : {e}")),
        }
    }

    fn close(&mut self, index: usize) {
        if index < self.archives.len() {
            self.archives.remove(index);
            if self.archives.is_empty() {
                self.active = 0;
            } else if self.active >= self.archives.len() {
                self.active = self.archives.len() - 1;
            }
        }
    }

    fn open_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Archives Bethesda", &["bsa", "ba2"])
            .add_filter("Tous les fichiers", &["*"])
            .pick_files()
        {
            for p in paths {
                self.load(p);
            }
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
        for p in dropped {
            self.load(p);
        }
    }

    fn draw_drag_hint(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drag_hint"),
        ));
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "Déposez une archive .bsa / .ba2",
            egui::FontId::proportional(26.0),
            egui::Color32::WHITE,
        );
    }

    /// Rebuild the preview for the active tab's selected file if it changed.
    fn sync_preview(&mut self, ctx: &egui::Context) {
        {
            let Some(loaded) = self.archives.get_mut(self.active) else {
                return;
            };
            if loaded.selected_file == loaded.preview_for {
                return;
            }
            loaded.preview_for = loaded.selected_file;
            loaded.preview = match loaded.selected_file {
                None => PreviewView::None,
                Some(i) => build_preview(ctx, &loaded.archive, i),
            };
        }
        // The selection changed: stop any playback and drop the cached clip so a
        // new file starts fresh.
        if let Some(s) = &self.sink {
            s.stop();
        }
        self.sink = None;
        self.audio_total = None;
        self.audio_cache = None;
        self.audio_status = None;
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
            if ui
                .add_enabled(
                    self.archives.len() >= 2,
                    egui::Button::new("Comparer des archives…"),
                )
                .clicked()
            {
                ui.close_menu();
                self.open_compare();
            }
            ui.separator();
            let has_active = !self.archives.is_empty();
            if ui
                .add_enabled(
                    has_active,
                    egui::Button::new("Exporter la liste (toute l’archive)…"),
                )
                .clicked()
            {
                ui.close_menu();
                self.export_list(false);
            }
            if ui
                .add_enabled(
                    has_active,
                    egui::Button::new("Exporter la liste (vue filtrée)…"),
                )
                .clicked()
            {
                ui.close_menu();
                self.export_list(true);
            }
            ui.separator();
            if ui
                .add_enabled(
                    !self.archives.is_empty(),
                    egui::Button::new("Fermer l’archive active"),
                )
                .clicked()
            {
                ui.close_menu();
                self.close(self.active);
            }
            if ui
                .add_enabled(!self.archives.is_empty(), egui::Button::new("Tout fermer"))
                .clicked()
            {
                ui.close_menu();
                self.archives.clear();
                self.active = 0;
            }
            ui.separator();
            if ui.button("Quitter").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            close_menu_if_pointer_left(ui, None);
        });
        ui.menu_button("À propos", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.label("BSA/BA2 Explorer");
            ui.label("Explorateur & extracteur d’archives Bethesda (.bsa/.ba2)");
            ui.label("Outil de XIMOD Translator");
            close_menu_if_pointer_left(ui, None);
        });
    }

    fn tabs_bar(&mut self, ctx: &egui::Context) {
        if self.archives.is_empty() {
            return;
        }
        let mut activate = None;
        let mut close = None;
        let delete = self.icons.as_ref().map(|i| i.delete.clone());
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, l) in self.archives.iter().enumerate() {
                        let selected = i == self.active;
                        let label = format!("{} [{}]", l.archive.file_name(), l.archive.kind());
                        if ui.selectable_label(selected, label).clicked() {
                            activate = Some(i);
                        }
                        let closed = match &delete {
                            Some(tex) => icon_button(ui, tex, 14.0).clicked(),
                            None => ui.small_button("x").clicked(),
                        };
                        if closed {
                            close = Some(i);
                        }
                        ui.separator();
                    }
                });
            });
        });
        if let Some(i) = activate {
            self.active = i;
        }
        if let Some(i) = close {
            self.close(i);
        }
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        let icons = self.icons.clone();
        let Some(loaded) = self.archives.get_mut(self.active) else {
            return;
        };
        ui.label(egui::RichText::new("Dossiers").weak());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Loaded {
                    tree,
                    selected_folder,
                    expanded,
                    ..
                } = loaded;
                let root_label = format!("(racine) ({})", tree.subtree_files);
                if ui
                    .selectable_label(selected_folder.is_empty(), root_label)
                    .clicked()
                {
                    *selected_folder = String::new();
                }
                draw_tree_node(ui, tree, selected_folder, expanded, icons.as_ref());
            });
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Aperçu").weak());
        ui.separator();
        // Audio needs `&mut self` (transport + playback state); pull its ext out
        // first so we don't hold an archives borrow across the audio controls.
        let audio_ext = match self.archives.get(self.active).map(|l| &l.preview) {
            Some(PreviewView::Audio { ext }) => Some(ext.clone()),
            _ => None,
        };
        if let Some(ext) = audio_ext {
            self.audio_panel(ui, &ext);
            return;
        }
        let Some(loaded) = self.archives.get(self.active) else {
            return;
        };
        match &loaded.preview {
            PreviewView::None => {
                ui.weak("Sélectionnez un fichier pour l’aperçu.");
            }
            PreviewView::Audio { .. } => {} // handled above
            PreviewView::Text { body, truncated } => {
                if *truncated {
                    ui.colored_label(
                        egui::Color32::from_rgb(0x9A, 0x6A, 0x00),
                        "Aperçu tronqué (fichier volumineux).",
                    );
                }
                egui::ScrollArea::both()
                    .id_salt("preview_text")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut body.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
            }
            PreviewView::Image { texture, note } => {
                ui.label(note);
                egui::ScrollArea::both()
                    .id_salt("preview_img")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Image::new(texture)
                                .max_width(ui.available_width().max(64.0))
                                .fit_to_original_size(1.0),
                        );
                    });
            }
            PreviewView::Unsupported { ext, size } => {
                ui.weak(format!(
                    "Aucun aperçu pour « .{ext} » ({}). Extrayez le fichier pour l’ouvrir.",
                    human_size(*size as u64)
                ));
            }
            PreviewView::Error(e) => {
                ui.colored_label(egui::Color32::from_rgb(0xB0, 0x24, 0x18), e);
            }
        }
    }

    /// Preview pane for an audio entry: decoder status + transport controls.
    fn audio_panel(&mut self, ui: &mut egui::Ui, ext: &str) {
        ui.label(format!("Fichier audio « .{ext} »"));
        if audio::is_native_ext(ext) {
            ui.weak("Lecture directe (format standard).");
        } else if ext == "wem" {
            // Wwise Vorbis is decoded in pure Rust; no external tool required.
            ui.weak("Décodeur .wem Wwise Vorbis intégré (Rust).");
            if let Some(c) = &self.converter {
                ui.weak(format!("Repli pour .wem non-Vorbis : {}", c.label()));
            }
        } else {
            match &self.converter {
                Some(c) => {
                    ui.weak(format!("Décodeur : {}", c.label()));
                }
                None => {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xB0, 0x24, 0x18),
                        "Aucun décodeur trouvé pour ce format.",
                    );
                    ui.weak("Installez ffmpeg (.xwm/.fuz) ou vgmstream-cli, à côté de l’exécutable ou dans le PATH.");
                }
            }
            // Skyrim LE (2011) voices are 32 kHz xWMA that only Microsoft's own
            // decoder handles correctly; ffmpeg/vgmstream garble or silence them.
            ui.weak(
                "Voix Skyrim LE (32 kHz) : placez xWMAEncode.exe à côté de l’exécutable \
                 (le seul décodeur correct pour ces fichiers ; utilisé en priorité s’il est présent).",
            );
        }

        // For game-specific containers, offer one-click links to the external
        // decoders (opens the default browser on their download pages).
        if !audio::is_native_ext(ext) {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Décodeurs :");
                if ui
                    .button("Télécharger vgmstream")
                    .on_hover_text("vgmstream — décode .wem / .xwm / .fuz\nhttps://vgmstream.org/")
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(
                        "https://github.com/vgmstream/vgmstream/releases/latest",
                    ));
                }
                if ui
                    .button("Télécharger FFmpeg")
                    .on_hover_text("FFmpeg — décode .xwm / .fuz\nhttps://ffmpeg.org/download.html")
                    .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab("https://ffmpeg.org/download.html"));
                }
            });
        }

        let playing = self.sink.as_ref().map(|s| !s.empty()).unwrap_or(false);
        let paused = self.sink.as_ref().map(|s| s.is_paused()).unwrap_or(false);
        // Native formats and .wem play without any external decoder; others need one.
        // xwm/fuz can also be decoded by xWMAEncode alone (Skyrim LE), even with
        // no vgmstream/ffmpeg present.
        let can_play = audio::plays_without_external(ext)
            || self.converter.is_some()
            || (matches!(ext, "xwm" | "fuz") && self.has_xwmaencode);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_play, egui::Button::new("Écouter"))
                .clicked()
            {
                self.audio_cmd = Some(AudioCmd::Play);
            }
            if ui
                .add_enabled(
                    playing,
                    egui::Button::new(if paused { "Reprendre" } else { "Pause" }),
                )
                .clicked()
            {
                self.audio_cmd = Some(AudioCmd::PauseToggle);
            }
            if ui.add_enabled(playing, egui::Button::new("Stop")).clicked() {
                self.audio_cmd = Some(AudioCmd::Stop);
            }
        });

        if let (Some(sink), Some(total)) = (&self.sink, self.audio_total) {
            let pos = sink.get_pos().as_secs_f32();
            let tot = total.as_secs_f32().max(0.001);
            let mut frac = (pos / tot).clamp(0.0, 1.0);
            ui.add_space(4.0);
            ui.label(format!("{} / {}", fmt_time(pos), fmt_time(tot)));
            if ui
                .add(egui::Slider::new(&mut frac, 0.0..=1.0).show_value(false))
                .changed()
            {
                self.audio_cmd = Some(AudioCmd::Seek(frac));
            }
            // Keep the elapsed time / slider moving while it plays.
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }

        if let Some(status) = &self.audio_status {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::from_rgb(0xB0, 0x24, 0x18), status);
        }
    }

    /// Apply a transport command emitted by [`Self::audio_panel`].
    fn apply_audio_cmd(&mut self) {
        let Some(cmd) = self.audio_cmd.take() else {
            return;
        };
        match cmd {
            AudioCmd::Play => self.audio_play(),
            AudioCmd::PauseToggle => {
                if let Some(s) = &self.sink {
                    if s.is_paused() {
                        s.play();
                    } else {
                        s.pause();
                    }
                }
            }
            AudioCmd::Stop => {
                if let Some(s) = &self.sink {
                    s.stop();
                }
                self.sink = None;
            }
            AudioCmd::Seek(frac) => {
                if let (Some(s), Some(total)) = (&self.sink, self.audio_total) {
                    let _ = s.try_seek(total.mul_f32(frac));
                }
            }
        }
    }

    /// Decode the selected audio entry (cached) and start playback.
    fn audio_play(&mut self) {
        self.audio_status = None;

        // Which entry, and its cache key.
        let (path, index, key) = {
            let Some(loaded) = self.archives.get(self.active) else {
                return;
            };
            let Some(i) = loaded.selected_file else {
                return;
            };
            let path = loaded.archive.entries()[i].path.clone();
            let key = format!("{}::{i}", loaded.archive.path().display());
            (path, i, key)
        };

        // Decoded WAV, from cache or freshly converted.
        let wav = if self
            .audio_cache
            .as_ref()
            .map(|(k, _)| *k == key)
            .unwrap_or(false)
        {
            self.audio_cache.as_ref().unwrap().1.clone()
        } else {
            let bytes = {
                let Some(loaded) = self.archives.get(self.active) else {
                    return;
                };
                match loaded.archive.extract(index) {
                    Ok(b) => b,
                    Err(e) => {
                        self.audio_status = Some(e);
                        return;
                    }
                }
            };
            match audio::decode(&path, &bytes, self.converter.as_ref()) {
                Ok(w) => {
                    self.audio_cache = Some((key, w.clone()));
                    w
                }
                Err(e) => {
                    self.audio_status = Some(e);
                    return;
                }
            }
        };

        // Ensure a system audio output exists.
        if self.audio_out.is_none() {
            match OutputStream::try_default() {
                Ok(pair) => self.audio_out = Some(pair),
                Err(e) => {
                    self.audio_status = Some(format!("Sortie audio indisponible : {e}"));
                    return;
                }
            }
        }
        let handle = &self.audio_out.as_ref().unwrap().1;
        let sink = match Sink::try_new(handle) {
            Ok(s) => s,
            Err(e) => {
                self.audio_status = Some(format!("Lecture impossible : {e}"));
                return;
            }
        };
        let decoder = match rodio::Decoder::new(Cursor::new(wav)) {
            Ok(d) => d,
            Err(e) => {
                self.audio_status = Some(format!("WAV illisible : {e}"));
                return;
            }
        };
        self.audio_total = decoder.total_duration();
        sink.append(decoder);
        sink.play();
        self.sink = Some(sink);
    }

    /// Open the comparison window, defaulting A = active, B = another open archive.
    fn open_compare(&mut self) {
        self.show_compare = true;
        self.compare_center = true;
        self.compare_a = self.active;
        self.compare_b = if self.active == 0 { 1 } else { 0 };
        self.compare_result = None;
        self.compare_filter.clear();
        self.compare_cat = [true, true, true, false]; // added, modified, removed, (unchanged off)
    }

    fn compare_window(&mut self, ctx: &egui::Context) {
        if !self.show_compare {
            return;
        }
        let labels: Vec<String> = self
            .archives
            .iter()
            .map(|l| format!("{} [{}]", l.archive.file_name(), l.archive.kind()))
            .collect();
        let mut open = true;
        let mut win = egui::Window::new("Comparer des archives")
            .open(&mut open)
            .resizable(true)
            .default_width(720.0)
            .default_height(460.0);
        if self.compare_center {
            win = win.anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]);
            self.compare_center = false;
        }
        win.show(ctx, |ui| {
            // Archive pickers + run.
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("cmp_a")
                    .selected_text(labels.get(self.compare_a).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (i, lab) in labels.iter().enumerate() {
                            ui.selectable_value(&mut self.compare_a, i, lab);
                        }
                    });
                ui.label("→");
                egui::ComboBox::from_id_salt("cmp_b")
                    .selected_text(labels.get(self.compare_b).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (i, lab) in labels.iter().enumerate() {
                            ui.selectable_value(&mut self.compare_b, i, lab);
                        }
                    });
                if ui.button("Comparer").clicked()
                    && let (Some(a), Some(b)) = (
                        self.archives.get(self.compare_a),
                        self.archives.get(self.compare_b),
                    )
                {
                    self.compare_result =
                        Some(compare::compare(a.archive.entries(), b.archive.entries()));
                }
            });
            ui.separator();

            let Some(diff) = self.compare_result.clone() else {
                ui.weak("Choisissez deux archives (A = gauche, B = droite) puis « Comparer ».");
                return;
            };

            // Category toggles + counts + filter + export.
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut self.compare_cat[0],
                    format!("Ajoutés ({})", diff.added),
                );
                ui.checkbox(
                    &mut self.compare_cat[1],
                    format!("Modifiés ({})", diff.modified),
                );
                ui.checkbox(
                    &mut self.compare_cat[2],
                    format!("Supprimés ({})", diff.removed),
                );
                ui.checkbox(
                    &mut self.compare_cat[3],
                    format!("Inchangés ({})", diff.unchanged),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Filtre :");
                ui.add(
                    egui::TextEdit::singleline(&mut self.compare_filter)
                        .hint_text("nom / chemin")
                        .desired_width(240.0),
                );
                if !self.compare_filter.is_empty() && ui.button("×").clicked() {
                    self.compare_filter.clear();
                }
                if ui.button("Exporter (.txt)…").clicked() {
                    self.export_compare(&diff);
                }
            });
            ui.separator();

            // Filtered rows.
            let needle = self.compare_filter.trim().to_lowercase();
            let cat = self.compare_cat;
            let rows: Vec<&crate::compare::DiffRow> = diff
                .rows
                .iter()
                .filter(|r| cat_shown(&cat, r.status))
                .filter(|r| needle.is_empty() || r.path.to_lowercase().contains(&needle))
                .collect();

            let row_h = 20.0;
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(90.0))
                .column(Column::remainder().clip(true))
                .column(Column::exact(90.0))
                .column(Column::exact(90.0))
                .header(20.0, |mut h| {
                    h.col(|ui| {
                        ui.strong("Statut");
                    });
                    h.col(|ui| {
                        ui.strong("Chemin");
                    });
                    h.col(|ui| {
                        ui.strong("Taille A");
                    });
                    h.col(|ui| {
                        ui.strong("Taille B");
                    });
                })
                .body(|body| {
                    body.rows(row_h, rows.len(), |mut row| {
                        let r = rows[row.index()];
                        row.col(|ui| {
                            ui.colored_label(status_color(r.status), r.status.label());
                        });
                        row.col(|ui| {
                            ui.label(&r.path).on_hover_text(&r.path);
                        });
                        row.col(|ui| {
                            ui.label(r.size_a.map(human_size).unwrap_or_else(|| "—".into()));
                        });
                        row.col(|ui| {
                            ui.label(r.size_b.map(human_size).unwrap_or_else(|| "—".into()));
                        });
                    });
                });
        });
        self.show_compare = open;
    }

    /// Prompt for a path and write the current comparison to a text file.
    fn export_compare(&mut self, diff: &Diff) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("comparaison.txt")
            .add_filter("Texte", &["txt"])
            .save_file()
        else {
            return;
        };
        let a = self
            .archives
            .get(self.compare_a)
            .map(|l| l.archive.file_name())
            .unwrap_or_default();
        let b = self
            .archives
            .get(self.compare_b)
            .map(|l| l.archive.file_name())
            .unwrap_or_default();
        let mut s = String::new();
        s.push_str(&format!("# Comparaison\n# A : {a}\n# B : {b}\n"));
        s.push_str(&format!(
            "# Ajoutés {} · Modifiés {} · Supprimés {} · Inchangés {}\n\n",
            diff.added, diff.modified, diff.removed, diff.unchanged
        ));
        for r in &diff.rows {
            if !cat_shown(&self.compare_cat, r.status) {
                continue;
            }
            let sa = r
                .size_a
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            let sb = r
                .size_b
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            s.push_str(&format!(
                "{:<9} {:>12} {:>12}  {}\n",
                r.status.label(),
                sa,
                sb,
                r.path
            ));
        }
        match std::fs::write(&path, s) {
            Ok(_) => self.notice = Some(format!("Comparaison exportée → {}", path.display())),
            Err(e) => self.error = Some(format!("Erreur d’export : {e}")),
        }
    }

    /// Export the active archive's file list to a `.csv` or `.txt` file (format
    /// chosen by the save dialog's extension). `only_visible` exports the current
    /// filtered/folder view; otherwise the whole archive.
    fn export_list(&mut self, only_visible: bool) {
        let Some(loaded) = self.archives.get(self.active) else {
            return;
        };
        let full = loaded.archive.file_name();
        let stem = full.rsplit_once('.').map(|(a, _)| a).unwrap_or(&full);
        let suffix = if only_visible { "_liste_vue" } else { "_liste" };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{stem}{suffix}.csv"))
            .add_filter("CSV", &["csv"])
            .add_filter("Texte", &["txt"])
            .save_file()
        else {
            return;
        };
        let indices: Vec<usize> = if only_visible {
            loaded.visible.clone()
        } else {
            (0..loaded.archive.entries().len()).collect()
        };
        let csv = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("csv"))
            .unwrap_or(false);
        let content = build_file_list(&loaded.archive, &indices, csv);
        let n = indices.len();
        match std::fs::write(&path, content) {
            Ok(_) => {
                self.notice = Some(format!(
                    "Liste exportée ({n} fichiers) → {}",
                    path.display()
                ))
            }
            Err(e) => self.error = Some(format!("Erreur d’export : {e}")),
        }
    }

    /// `--export-list <archive> <out.csv|out.txt>`: write the whole file list
    /// (format chosen by the output extension). Returns the number of entries.
    pub fn run_export_list(path: &Path, out: &Path) -> Result<usize, String> {
        let a = Archive::open(path)?;
        let indices: Vec<usize> = (0..a.entries().len()).collect();
        let csv = out
            .extension()
            .map(|e| e.eq_ignore_ascii_case("csv"))
            .unwrap_or(false);
        let content = build_file_list(&a, &indices, csv);
        std::fs::write(out, content).map_err(|e| format!("écriture : {e}"))?;
        Ok(a.entries().len())
    }

    /// `--compare <a> <b>`: print the diff counts. Returns (added, modified, removed, unchanged).
    pub fn run_compare(a: &Path, b: &Path) -> Result<(usize, usize, usize, usize), String> {
        let aa = Archive::open(a)?;
        let bb = Archive::open(b)?;
        let d = compare::compare(aa.entries(), bb.entries());
        for r in &d.rows {
            if r.status == Status::Unchanged {
                continue;
            }
            println!("{:<9} {}", r.status.label(), r.path);
        }
        Ok((d.added, d.modified, d.removed, d.unchanged))
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let Some(loaded) = self.archives.get_mut(self.active) else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            if ui.button("Tout cocher (vue)").clicked() {
                for &i in &loaded.visible {
                    loaded.checked.insert(i);
                }
            }
            if ui.button("Tout décocher (vue)").clicked() {
                for i in &loaded.visible {
                    loaded.checked.remove(i);
                }
            }
            ui.separator();
            ui.checkbox(&mut loaded.recursive, "Inclure les sous-dossiers");
            ui.separator();
            ui.label(format!("{} coché(s)", loaded.checked.len()));
        });
        ui.horizontal_wrapped(|ui| {
            let has_checked = !loaded.checked.is_empty();
            if ui
                .add_enabled(has_checked, egui::Button::new("Extraire la sélection…"))
                .clicked()
            {
                let mut idx: Vec<usize> = loaded.checked.iter().copied().collect();
                idx.sort_unstable();
                self.notice = extract_dialog(&loaded.archive, &idx);
            }
            if ui
                .add_enabled(
                    !loaded.archive.entries().is_empty(),
                    egui::Button::new("Extraire tout…"),
                )
                .clicked()
            {
                let idx: Vec<usize> = (0..loaded.archive.entries().len()).collect();
                self.notice = extract_dialog(&loaded.archive, &idx);
            }
        });
    }

    fn file_table(&mut self, ui: &mut egui::Ui) {
        let icons = self.icons.clone();
        let Some(loaded) = self.archives.get_mut(self.active) else {
            ui.weak("Ouvrez une archive (Fichier → Ouvrir…) ou glissez-la sur la fenêtre.");
            return;
        };
        // Filter row.
        ui.horizontal(|ui| {
            ui.label("Filtre :");
            ui.add(
                egui::TextEdit::singleline(&mut loaded.filter)
                    .hint_text("nom / chemin")
                    .desired_width(240.0),
            );
            if !loaded.filter.is_empty() {
                let cleared = match &icons {
                    Some(ic) => icon_button(ui, &ic.delete, 14.0).clicked(),
                    None => ui.button("x").clicked(),
                };
                if cleared {
                    loaded.filter.clear();
                }
            }
            if loaded.filter.trim().is_empty() && !loaded.selected_folder.is_empty() {
                ui.separator();
                ui.weak(format!("Dossier : {}", loaded.selected_folder));
            }
        });
        ui.separator();

        let ok_ico = icons.as_ref().map(|i| i.ok.clone());
        let dash_ico = icons.as_ref().map(|i| i.em_dash.clone());
        let Loaded {
            archive,
            checked,
            selected_file,
            visible,
            ..
        } = loaded;
        let entries = archive.entries();
        let row_h = 20.0;
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(22.0))
            .column(Column::remainder().clip(true))
            .column(Column::exact(90.0))
            .column(Column::exact(70.0))
            .column(Column::exact(110.0))
            .header(20.0, |mut h| {
                h.col(|_| {});
                h.col(|ui| {
                    ui.strong("Nom");
                });
                h.col(|ui| {
                    ui.strong("Taille");
                });
                h.col(|ui| {
                    ui.strong("Compr.");
                });
                h.col(|ui| {
                    ui.strong("Type");
                });
            })
            .body(|body| {
                body.rows(row_h, visible.len(), |mut row| {
                    let i = visible[row.index()];
                    let e = &entries[i];
                    row.col(|ui| {
                        let mut c = checked.contains(&i);
                        if ui.checkbox(&mut c, "").changed() {
                            if c {
                                checked.insert(i);
                            } else {
                                checked.remove(&i);
                            }
                        }
                    });
                    row.col(|ui| {
                        let resp = ui.selectable_label(*selected_file == Some(i), e.file_name());
                        if resp.clicked() {
                            *selected_file = Some(i);
                        }
                        let tip = match e.detail() {
                            Some(d) => format!("{}\n{d}", e.path),
                            None => e.path.clone(),
                        };
                        resp.on_hover_text(tip);
                    });
                    row.col(|ui| {
                        ui.label(human_size(e.size));
                    });
                    row.col(|ui| {
                        let tex = if e.compressed { &ok_ico } else { &dash_ico };
                        match tex {
                            Some(t) => icon_image(ui, t, 14.0),
                            None => {
                                ui.label(if e.compressed { "o" } else { "-" });
                            }
                        }
                    });
                    row.col(|ui| {
                        ui.label(e.kind_label());
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(loaded) = self.archives.get(self.active) {
                ui.label(format!("📦 {}", loaded.archive.file_name()));
                ui.separator();
                ui.label(loaded.archive.kind());
                ui.separator();
                ui.label(format!("{} fichiers", loaded.archive.entries().len()));
                ui.separator();
                ui.label(human_size(loaded.archive.total_size()));
                ui.separator();
                ui.label(format!("{} affiché(s)", loaded.visible.len()));
            } else {
                ui.label("Aucune archive — Fichier → Ouvrir… ou glisser-déposer");
            }
        });
    }

    // ---- Headless CLI entry points ----

    /// `--list <archive>`: print every entry.
    pub fn run_list(path: &Path) -> Result<usize, String> {
        let a = Archive::open(path)?;
        println!(
            "# {} — {} ({} fichiers)",
            a.file_name(),
            a.kind(),
            a.entries().len()
        );
        for e in a.entries() {
            let c = if e.compressed { "C" } else { "-" };
            println!("{c} {:>12}  {}  {}", e.size, e.kind_label(), e.path);
        }
        Ok(a.entries().len())
    }

    /// `--extract <archive> <outdir> [substr]`: extract all entries (or those
    /// whose path contains `substr`), preserving the internal tree.
    pub fn run_extract(
        path: &Path,
        outdir: &Path,
        substr: Option<&str>,
    ) -> Result<(usize, usize), String> {
        let a = Archive::open(path)?;
        let needle = substr.map(|s| s.to_lowercase());
        let idx: Vec<usize> = a
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| match &needle {
                Some(n) => e.path.to_lowercase().contains(n),
                None => true,
            })
            .map(|(i, _)| i)
            .collect();
        let (ok, ko, _errs) = extract_to(&a, &idx, outdir);
        Ok((ok, ko))
    }
    /// Per-frame UI below the shell's unified menu bar (tabs, panels, windows).
    pub fn body(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.icons.is_none() {
            self.icons = Some(Icons::load(ctx));
        }
        if !self.converter_checked {
            self.converter = audio::detect();
            self.has_xwmaencode = audio::has_xwmaencode();
            self.converter_checked = true;
        }
        self.handle_dropped_files(ctx);
        if let Some(l) = self.archives.get_mut(self.active) {
            l.recompute_visible();
        }
        self.sync_preview(ctx);

        self.tabs_bar(ctx);

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));

        if !self.archives.is_empty() {
            egui::SidePanel::left("tree")
                .resizable(true)
                .default_width(260.0)
                .min_width(160.0)
                .show(ctx, |ui| self.tree_panel(ui));

            egui::SidePanel::right("preview")
                .resizable(true)
                .default_width(340.0)
                .min_width(180.0)
                .show(ctx, |ui| self.preview_panel(ui));

            egui::TopBottomPanel::bottom("foot").show(ctx, |ui| self.footer(ui));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(0xB0, 0x24, 0x18), err);
                ui.separator();
            }
            let mut clear = false;
            if let Some(msg) = &self.notice {
                let delete = self.icons.as_ref().map(|i| i.delete.clone());
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(0x1B, 0x7F, 0x3A), msg);
                    let dismissed = match &delete {
                        Some(tex) => icon_button(ui, tex, 14.0).clicked(),
                        None => ui.small_button("x").clicked(),
                    };
                    if dismissed {
                        clear = true;
                    }
                });
                ui.separator();
            }
            if clear {
                self.notice = None;
            }
            self.file_table(ui);
        });

        self.compare_window(ctx);
        self.apply_audio_cmd();
        self.draw_drag_hint(ctx);
    }
}

/// Whether a comparison status is enabled in the `[Added, Modified, Removed, Unchanged]` toggles.
fn cat_shown(cat: &[bool; 4], status: Status) -> bool {
    match status {
        Status::Added => cat[0],
        Status::Modified => cat[1],
        Status::Removed => cat[2],
        Status::Unchanged => cat[3],
    }
}

/// Colour for a comparison status.
fn status_color(status: Status) -> egui::Color32 {
    match status {
        Status::Added => egui::Color32::from_rgb(0x1B, 0x7F, 0x3A),
        Status::Removed => egui::Color32::from_rgb(0xB0, 0x24, 0x18),
        Status::Modified => egui::Color32::from_rgb(0x9A, 0x6A, 0x00),
        Status::Unchanged => egui::Color32::GRAY,
    }
}

/// Format seconds as `m:ss`.
fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Recursively render the folder tree under `node`.
fn draw_tree_node(
    ui: &mut egui::Ui,
    node: &TreeNode,
    selected: &mut String,
    expanded: &mut HashSet<String>,
    icons: Option<&Icons>,
) {
    for child in &node.children {
        let has_children = !child.children.is_empty();
        ui.horizontal(|ui| {
            if has_children {
                let is_exp = expanded.contains(&child.full);
                let toggled = match icons {
                    Some(ic) => {
                        let tex = if is_exp {
                            &ic.chevron_down
                        } else {
                            &ic.chevron_right
                        };
                        icon_button(ui, tex, 14.0).clicked()
                    }
                    None => ui.small_button(if is_exp { "v" } else { ">" }).clicked(),
                };
                if toggled {
                    if is_exp {
                        expanded.remove(&child.full);
                    } else {
                        expanded.insert(child.full.clone());
                    }
                }
            } else {
                ui.add_space(18.0);
            }
            let label = format!("{} ({})", child.name, child.subtree_files);
            if ui
                .selectable_label(*selected == child.full, label)
                .clicked()
            {
                *selected = child.full.clone();
            }
        });
        if has_children && expanded.contains(&child.full) {
            ui.indent(&child.full, |ui| {
                draw_tree_node(ui, child, selected, expanded, icons)
            });
        }
    }
}

/// Extract `index` from `archive` and decode it for preview, uploading any image
/// as an egui texture.
fn build_preview(ctx: &egui::Context, archive: &Archive, index: usize) -> PreviewView {
    let path = archive.entries()[index].path.clone();
    let ext = path
        .rsplit('/')
        .next()
        .unwrap_or(&path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    // Audio is decoded on demand when the user clicks play, not on selection.
    if audio::is_audio_ext(&ext) {
        return PreviewView::Audio { ext };
    }
    let bytes = match archive.extract(index) {
        Ok(b) => b,
        Err(e) => return PreviewView::Error(e),
    };
    match preview::decode(&path, &bytes) {
        Preview::Text { body, truncated } => PreviewView::Text { body, truncated },
        Preview::Image(img) => {
            let color =
                egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
            let texture = ctx.load_texture("bsa_preview", color, egui::TextureOptions::LINEAR);
            PreviewView::Image {
                texture,
                note: img.note,
            }
        }
        Preview::Unsupported { ext, size } => PreviewView::Unsupported { ext, size },
        Preview::Error(e) => PreviewView::Error(e),
    }
}

/// Prompt for an output folder and extract `indices`, returning a status line.
fn extract_dialog(archive: &Archive, indices: &[usize]) -> Option<String> {
    let dir = rfd::FileDialog::new().pick_folder()?;
    let (ok, ko, _errs) = extract_to(archive, indices, &dir);
    Some(if ko == 0 {
        format!("Extrait {ok} fichier(s) → {}", dir.display())
    } else {
        format!("Extrait {ok} fichier(s), {ko} échec(s) → {}", dir.display())
    })
}

/// Write `indices` of `archive` under `outdir`, preserving the internal tree.
/// Returns (succeeded, failed, error messages).
fn extract_to(archive: &Archive, indices: &[usize], outdir: &Path) -> (usize, usize, Vec<String>) {
    let (mut ok, mut ko) = (0usize, 0usize);
    let mut errs = Vec::new();
    for &i in indices {
        let rel = &archive.entries()[i].path;
        match archive.extract(i) {
            Ok(bytes) => {
                let dest = outdir.join(safe_relative(rel));
                if let Some(parent) = dest.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    ko += 1;
                    errs.push(format!("{rel}: {e}"));
                    continue;
                }
                match std::fs::write(&dest, &bytes) {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        ko += 1;
                        errs.push(format!("{rel}: {e}"));
                    }
                }
            }
            Err(e) => {
                ko += 1;
                errs.push(format!("{rel}: {e}"));
            }
        }
    }
    (ok, ko, errs)
}

/// Turn a `/`-separated archive path into a safe relative `PathBuf`, dropping any
/// absolute roots or `..` components so extraction stays inside the output dir.
/// Build the file-list export for `indices` of `archive`, as CSV (semicolon-
/// separated, Excel-friendly) or aligned plain text (mirroring `--list`).
fn build_file_list(archive: &Archive, indices: &[usize], csv: bool) -> String {
    let entries = archive.entries();
    let mut s = String::new();
    if csv {
        s.push_str("Chemin;Nom;Taille;Compressé;Type;Largeur;Hauteur;Mips;DXGI\r\n");
        for &i in indices {
            let Some(e) = entries.get(i) else {
                continue;
            };
            let (w, h, m, f) = match &e.texture {
                Some(t) => (
                    t.width.to_string(),
                    t.height.to_string(),
                    t.mips.to_string(),
                    t.format.to_string(),
                ),
                None => (String::new(), String::new(), String::new(), String::new()),
            };
            let ty = {
                let x = e.ext();
                if x.is_empty() {
                    "fichier".to_string()
                } else {
                    x
                }
            };
            let cols = [
                csv_field(&e.path),
                csv_field(e.file_name()),
                e.size.to_string(),
                if e.compressed { "oui" } else { "non" }.to_string(),
                csv_field(&ty),
                w,
                h,
                m,
                f,
            ];
            s.push_str(&cols.join(";"));
            s.push_str("\r\n");
        }
    } else {
        s.push_str(&format!(
            "# {} — {} ({} fichiers)\n",
            archive.file_name(),
            archive.kind(),
            indices.len()
        ));
        for &i in indices {
            let Some(e) = entries.get(i) else {
                continue;
            };
            let c = if e.compressed { "C" } else { "-" };
            s.push_str(&format!(
                "{c} {:>12}  {}  {}\n",
                e.size,
                e.kind_label(),
                e.path
            ));
        }
    }
    s
}

/// Quote a CSV field (semicolon dialect) when it contains a separator, quote, or
/// newline, doubling embedded quotes per RFC 4180.
fn csv_field(v: &str) -> String {
    if v.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn safe_relative(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." || comp == ".." {
            continue;
        }
        out.push(comp);
    }
    out
}

/// Human-readable byte size.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    if bytes < 1024 {
        return format!("{bytes} o");
    }
    let mut f = bytes as f64;
    let mut i = 0;
    while f >= 1024.0 && i < UNITS.len() - 1 {
        f /= 1024.0;
        i += 1;
    }
    format!("{f:.1} {}", UNITS[i])
}

/// Close the (top-level) menu owned by `ui` as soon as the pointer leaves its
/// popup — a menu opened by a click disappears on mouse-out, without a second
/// click. Mirrors ESM Explorer / XIMOD Translator for a consistent feel.
fn close_menu_if_pointer_left(ui: &mut egui::Ui, child: Option<egui::Rect>) {
    let popup = ui.min_rect();
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if menu_should_close(popup, child, pointer) {
        ui.close_menu();
    }
}

/// Pure geometry decision behind [`close_menu_if_pointer_left`].
fn menu_should_close(
    popup: egui::Rect,
    child: Option<egui::Rect>,
    pointer: Option<egui::Pos2>,
) -> bool {
    const BRIDGE_UP: f32 = 28.0;
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

#[cfg(test)]
mod tests {
    use super::{csv_field, human_size, menu_should_close, safe_relative};
    use eframe::egui::{Rect, pos2};

    #[test]
    fn csv_field_quotes_when_needed() {
        // Plain values pass through unquoted.
        assert_eq!(csv_field("meshes/a.nif"), "meshes/a.nif");
        // Separator, quotes, and newlines force quoting; inner quotes are doubled.
        assert_eq!(csv_field("a;b"), "\"a;b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\r\nline2"), "\"line1\r\nline2\"");
    }

    #[test]
    fn human_size_scales() {
        assert_eq!(human_size(512), "512 o");
        assert_eq!(human_size(2048), "2.0 Ko");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 Mo");
    }

    #[test]
    fn safe_relative_strips_traversal() {
        assert_eq!(
            safe_relative("a/b/c.dds"),
            std::path::PathBuf::from("a/b/c.dds")
        );
        assert_eq!(
            safe_relative("../../etc/passwd"),
            std::path::PathBuf::from("etc/passwd")
        );
        assert_eq!(safe_relative("/abs/x"), std::path::PathBuf::from("abs/x"));
    }

    #[test]
    fn menu_closes_only_when_pointer_leaves() {
        let popup = Rect::from_min_max(pos2(10.0, 30.0), pos2(150.0, 200.0));
        assert!(!menu_should_close(popup, None, Some(pos2(50.0, 100.0))));
        assert!(!menu_should_close(popup, None, Some(pos2(50.0, 10.0))));
        assert!(menu_should_close(popup, None, Some(pos2(400.0, 400.0))));
        assert!(!menu_should_close(popup, None, None));
    }
}
