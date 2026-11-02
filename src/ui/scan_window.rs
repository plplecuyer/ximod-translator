//! Fichier › Analyser un dossier… (D-07, figure B.6): an independent
//! window — the folder and the kinds of files, the analysis in the
//! background with its progress and its stop, the account, a line per
//! file (what the stack would make of it), the files opened on demand,
//! the CSV.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use eframe::egui;

use super::{XimodApp, read_geometry};
use crate::headless::Kind;
use crate::scan::{self, DictState, FileReport, Msg, Options, Scanner, State};

/// The files listed above which the analysis asks first (R-D07-01).
const MANY: usize = 500;

/// The space between two columns of a table.
const GAP: f32 = 8.0;

/// One row of a table: its cells in fixed widths, truncated, the row
/// highlighted when selected; clickable as a whole.
pub(crate) fn table_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    cells: &[egui::RichText],
    widths: &[f32],
    h: f32,
    selected: bool,
) -> egui::Response {
    let total: f32 = widths.iter().sum::<f32>() + GAP * widths.len() as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total, h), egui::Sense::hover());
    let resp = ui.interact(rect, id, egui::Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let mut x = rect.left();
    for (cell, w) in cells.iter().zip(widths) {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*w, h));
        ui.put(
            r,
            egui::Label::new(cell.clone()).truncate().selectable(false),
        );
        x += w + GAP;
    }
    resp
}

#[derive(Default)]
pub(crate) struct ScanWin {
    pub open: bool,
    pub needs_place: bool,
    pub dir: String,
    pub opts: Option<Options>,
    pub reports: Vec<FileReport>,
    pub total: usize,
    pub run: Option<(Receiver<Msg>, Arc<AtomicBool>)>,
    pub secs: Option<(f32, bool)>,
    /// The files listed, waiting for a « yes » above [`MANY`].
    pub confirm: Option<Vec<(PathBuf, Kind)>>,
    pub selected: BTreeSet<usize>,
    pub anchor: Option<usize>,
    /// The CSV file to write.
    pub csv: String,
    pub message: Option<String>,
    /// Fermer was clicked.
    pub closing: bool,
}

impl XimodApp {
    /// Fichier › Analyser un dossier….
    pub(crate) fn open_scan_window(&mut self) {
        if self.scan_win.opts.is_none() {
            self.scan_win.opts = Some(Options {
                subdirs: self.config.scan_subdirs,
                scripts: self.config.scan_scripts,
                ..Options::default()
            });
            self.scan_win.dir = self.config.scan_last_dir.clone();
        }
        self.scan_win.open = true;
        self.scan_win.needs_place = true;
    }

    fn scan_start(&mut self, files: Vec<(PathBuf, Kind)>) {
        let (game, src, dst) = self.scan_pair_pub();
        let root = PathBuf::from(self.scan_win.dir.trim());
        let scanner = Scanner::new(self.config.clone(), &root, &game, &src, &dst);
        let stop = Arc::new(AtomicBool::new(false));
        self.scan_win.reports.clear();
        self.scan_win.selected.clear();
        self.scan_win.total = files.len();
        self.scan_win.secs = None;
        self.scan_win.message = None;
        let rx = scan::start(scanner, files, stop.clone());
        self.scan_win.run = Some((rx, stop));
    }

    fn scan_list(&mut self) {
        let root = PathBuf::from(self.scan_win.dir.trim());
        if !root.is_dir() {
            self.scan_win.message = Some(self.i18n.t_arg(
                "err-open",
                "error",
                &root.display().to_string(),
            ));
            return;
        }
        let opts = self.scan_win.opts.unwrap_or_default();
        self.config.scan_last_dir = self.scan_win.dir.trim().to_string();
        self.config.scan_subdirs = opts.subdirs;
        self.config.scan_scripts = opts.scripts;
        let _ = self.config.save();
        let formats = crate::textfmt::Formats::load();
        let files = scan::list(&root, opts, &formats);
        if self.scan_win.csv.trim().is_empty() {
            self.scan_win.csv = root.join("ximod-scan.csv").display().to_string();
        }
        if files.len() > MANY {
            self.scan_win.confirm = Some(files);
        } else {
            self.scan_start(files);
        }
    }

    /// Drain the analysis' messages.
    fn scan_poll(&mut self, ctx: &egui::Context) {
        let mut done = false;
        if let Some((rx, _)) = &self.scan_win.run {
            loop {
                match rx.try_recv() {
                    Ok(Msg::File(r)) => self.scan_win.reports.push(*r),
                    Ok(Msg::Done { secs, stopped }) => {
                        self.scan_win.secs = Some((secs, stopped));
                        done = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                }
            }
        }
        if done {
            self.scan_win.run = None;
        }
    }

    /// Open files of the analysis with their game and the pair.
    fn scan_open(&mut self, idx: &[usize]) {
        let (_, src, dst) = self.scan_pair_pub();
        let paths: Vec<(PathBuf, String)> = idx
            .iter()
            .filter_map(|&i| self.scan_win.reports.get(i))
            .map(|r| (r.path.clone(), r.game.clone()))
            .collect();
        for (p, game) in paths {
            let mut s = crate::doc_settings::DocSettings::defaults(&self.config);
            s.game = game;
            s.source = src.clone();
            s.target = dst.clone();
            s.output = None;
            self.load_path_with(p, Some(s));
        }
    }

    fn scan_state_label(&self, r: &FileReport) -> String {
        match r.state {
            State::Done => self.i18n.t("scan-state-done"),
            State::Almost => self.i18n.t("scan-state-almost"),
            State::Todo => self.i18n.t("scan-state-todo"),
            State::Update => self.i18n.t("scan-state-update"),
            State::Source => self.i18n.t("scan-state-source"),
            State::Error => self.i18n.t("scan-state-error"),
            State::Dictionary => self.i18n.t("scan-dict-is"),
        }
    }

    fn scan_dict_label(&self, d: DictState) -> String {
        self.i18n.t(match d {
            DictState::Known => "scan-dict-known",
            DictState::Other => "scan-dict-other",
            DictState::Absent => "scan-dict-none",
            DictState::Is => "scan-dict-is",
        })
    }

    /// The window, each frame while it is open.
    pub(crate) fn scan_window(&mut self, ctx: &egui::Context) {
        if !self.scan_win.open {
            return;
        }
        self.scan_poll(ctx);
        let id = egui::ViewportId::from_hash_of("ximod-scan");
        let [sx, sy, sw, sh] = self.config.scan_win;
        let just_opened = self.scan_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(760.0), sh.max(420.0)],
            )
        });
        let dir_label = if self.scan_win.dir.trim().is_empty() {
            "—".to_string()
        } else {
            self.scan_win.dir.trim().to_string()
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t_arg("scan-title", "dir", &dir_label))
            .with_min_inner_size([760.0, 420.0]);
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
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.scan_win.needs_place = false;
            egui::CentralPanel::default().show(vctx, |ui| self.scan_body(ui));
            if std::mem::take(&mut self.scan_win.closing)
                || vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.scan_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            if let Some((_, stop)) = &self.scan_win.run {
                stop.store(true, Ordering::Relaxed);
            }
            self.scan_win.open = false;
            let _ = self.config.save();
        }
    }

    fn scan_body(&mut self, ui: &mut egui::Ui) {
        let t = |k: &str| self.i18n.t(k);
        let running = self.scan_win.run.is_some();
        let (l_dir, l_browse, l_run, l_stop) = (
            t("scan-dir"),
            t("scan-browse"),
            t("scan-run"),
            t("online-stop"),
        );
        let (l_plugins, l_texts, l_scripts, l_dsd, l_subdirs) = (
            t("scan-plugins"),
            t("scan-texts"),
            t("scan-scripts"),
            t("scan-dsd"),
            t("scan-subdirs"),
        );
        let (game, src, dst) = self.scan_pair_pub();
        let pair = self.i18n.t_args(
            "scan-pair",
            &[
                ("game", &game),
                ("src", &src.to_uppercase()),
                ("dst", &dst.to_uppercase()),
            ],
        );
        let mut do_list = false;
        let mut do_stop = false;
        ui.horizontal(|ui| {
            ui.label(&l_dir);
            ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut self.scan_win.dir).desired_width(520.0),
            );
            if ui
                .add_enabled(!running, egui::Button::new(&l_browse))
                .clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                self.scan_win.dir = p.display().to_string();
            }
            if running {
                if ui.button(&l_stop).clicked() {
                    do_stop = true;
                }
            } else if ui.button(&l_run).clicked() {
                do_list = true;
            }
        });
        let mut opts = self.scan_win.opts.unwrap_or_default();
        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut opts.plugins, &l_plugins);
                ui.checkbox(&mut opts.texts, &l_texts);
                ui.checkbox(&mut opts.scripts, &l_scripts);
                ui.checkbox(&mut opts.dsd, &l_dsd);
                ui.checkbox(&mut opts.subdirs, &l_subdirs);
                ui.label(egui::RichText::new(&pair).weak());
            });
        });
        self.scan_win.opts = Some(opts);
        // The question above MANY files.
        if let Some(files) = &self.scan_win.confirm {
            let n = files.len();
            let msg = self.i18n.t_arg("scan-many", "n", &n.to_string());
            let (yes, no) = (t("btn-yes"), t("btn-no"));
            let mut answer = None;
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, msg);
                if ui.button(&yes).clicked() {
                    answer = Some(true);
                }
                if ui.button(&no).clicked() {
                    answer = Some(false);
                }
            });
            match answer {
                Some(true) => {
                    if let Some(files) = self.scan_win.confirm.take() {
                        self.scan_start(files);
                    }
                }
                Some(false) => self.scan_win.confirm = None,
                None => {}
            }
        }
        let t = |k: &str| self.i18n.t(k);
        // The account.
        let tot = scan::totals(&self.scan_win.reports);
        if running {
            let done = self.scan_win.reports.len();
            let frac = if self.scan_win.total == 0 {
                1.0
            } else {
                done as f32 / self.scan_win.total as f32
            };
            ui.add(egui::ProgressBar::new(frac).text(self.i18n.t_args(
                "scan-progress",
                &[
                    ("done", &done.to_string()),
                    ("total", &self.scan_win.total.to_string()),
                ],
            )));
        }
        if let Some((secs, stopped)) = self.scan_win.secs {
            let mut s = self.i18n.t_args(
                "scan-summary",
                &[
                    ("files", &tot.files.to_string()),
                    ("secs", &format!("{secs:.1}")),
                    ("lines", &tot.lines.to_string()),
                    ("done", &tot.filled.to_string()),
                    ("left", &tot.left.to_string()),
                    ("locked", &tot.locked.to_string()),
                ],
            );
            if stopped {
                s = self.i18n.dash(&s, &t("scan-stopped"));
            }
            ui.label(egui::RichText::new(s).strong());
        }
        if let Some(m) = &self.scan_win.message {
            ui.label(egui::RichText::new(m).weak());
        }
        ui.separator();
        // The buttons, under the table.
        let (l_open, l_open_todo, l_export, l_close) = (
            t("scan-open"),
            t("scan-open-todo"),
            t("scan-export"),
            t("btn-close"),
        );
        let mut close = false;
        let mut open_sel = false;
        let mut open_todo = false;
        let mut export = false;
        egui::TopBottomPanel::bottom("ximod-scan-bottom")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.scan_win.selected.is_empty(),
                            egui::Button::new(&l_open),
                        )
                        .clicked()
                    {
                        open_sel = true;
                    }
                    let any_todo = self.scan_win.reports.iter().any(|r| {
                        r.kind == Kind::Plugin && matches!(r.state, State::Todo | State::Almost)
                    });
                    if ui
                        .add_enabled(any_todo && !running, egui::Button::new(&l_open_todo))
                        .clicked()
                    {
                        open_todo = true;
                    }
                    ui.separator();
                    ui.add(egui::TextEdit::singleline(&mut self.scan_win.csv).desired_width(360.0));
                    if ui
                        .add_enabled(
                            !self.scan_win.reports.is_empty() && !running,
                            egui::Button::new(&l_export),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&l_close).clicked() {
                            close = true;
                        }
                    });
                });
            });
        // The table.
        let heads = [
            t("scan-col-file"),
            t("scan-col-game"),
            t("scan-col-lines"),
            t("scan-col-done"),
            t("scan-col-left"),
            t("scan-col-locked"),
            t("scan-col-dict"),
            t("scan-col-state"),
        ];
        let rows: Vec<[String; 8]> = self
            .scan_win
            .reports
            .iter()
            .map(|r| {
                let pct = (r.filled * 100)
                    .checked_div(r.lines)
                    .map(|p| format!(" ({p} %)"))
                    .unwrap_or_default();
                // The file's name first, its folder after (the end of a
                // long path is what the truncation would hide).
                let (dir, name) = match r.rel.rsplit_once('/') {
                    Some((d, n)) => (d.to_string(), n.to_string()),
                    None => (String::new(), r.rel.clone()),
                };
                let first = if dir.is_empty() {
                    name
                } else {
                    format!("{name}  —  {dir}")
                };
                [
                    first,
                    r.game.clone(),
                    r.lines.to_string(),
                    format!("{}{pct}", r.filled),
                    r.left.to_string(),
                    r.locked.to_string(),
                    self.scan_dict_label(r.dict),
                    self.scan_state_label(r),
                ]
            })
            .collect();
        let tips: Vec<String> = self
            .scan_win
            .reports
            .iter()
            .map(|r| match &r.error {
                Some(e) => self.i18n.localize(e),
                None => std::iter::once(r.rel.clone())
                    .chain(r.per_level.iter().map(|(n, k)| format!("{n} : {k}")))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect();
        let widths = [0.0f32, 90.0, 60.0, 110.0, 70.0, 80.0, 100.0, 120.0];
        let mut clicked: Option<(usize, egui::Modifiers, bool)> = None;
        let text_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        let fixed: f32 = widths[1..].iter().sum::<f32>() + GAP * 9.0 + 16.0;
        let first_w = (ui.available_width() - fixed).max(200.0);
        egui::ScrollArea::horizontal()
            .id_salt("ximod-scan-h")
            .show(ui, |ui| {
                let ws: Vec<f32> = (0..widths.len())
                    .map(|k| if k == 0 { first_w } else { widths[k] })
                    .collect();
                let head: Vec<egui::RichText> = heads
                    .iter()
                    .map(|h| egui::RichText::new(h).strong())
                    .collect();
                table_row(
                    ui,
                    egui::Id::new("ximod-scan-head"),
                    &head,
                    &ws,
                    text_h,
                    false,
                );
                egui::ScrollArea::vertical()
                    .id_salt("ximod-scan-v")
                    .auto_shrink([false, false])
                    .show_rows(ui, text_h, rows.len(), |ui, range| {
                        for i in range {
                            let sel = self.scan_win.selected.contains(&i);
                            let cells: Vec<egui::RichText> = rows[i]
                                .iter()
                                .enumerate()
                                .map(|(k, c)| {
                                    let t = egui::RichText::new(c);
                                    if k == 0 { t } else { t.small() }
                                })
                                .collect();
                            let resp = table_row(
                                ui,
                                egui::Id::new(("ximod-scan-row", i)),
                                &cells,
                                &ws,
                                text_h,
                                sel,
                            );
                            let resp = if tips[i].is_empty() {
                                resp
                            } else {
                                resp.on_hover_text(&tips[i])
                            };
                            if resp.double_clicked() {
                                clicked = Some((i, egui::Modifiers::NONE, true));
                            } else if resp.clicked() {
                                clicked = Some((i, ui.input(|x| x.modifiers), false));
                            }
                        }
                    });
            });
        if let Some((i, mods, double)) = clicked {
            if double {
                self.scan_open(&[i]);
            } else if mods.shift
                && let Some(a) = self.scan_win.anchor
            {
                for k in a.min(i)..=a.max(i) {
                    self.scan_win.selected.insert(k);
                }
            } else if mods.command {
                if !self.scan_win.selected.remove(&i) {
                    self.scan_win.selected.insert(i);
                }
                self.scan_win.anchor = Some(i);
            } else {
                self.scan_win.selected.clear();
                self.scan_win.selected.insert(i);
                self.scan_win.anchor = Some(i);
            }
        }
        if close {
            self.scan_win.closing = true;
        }
        if do_stop && let Some((_, stop)) = &self.scan_win.run {
            stop.store(true, Ordering::Relaxed);
        }
        if do_list {
            self.scan_list();
        }
        if open_sel {
            let idx: Vec<usize> = self.scan_win.selected.iter().copied().collect();
            self.scan_open(&idx);
        }
        if open_todo {
            let idx: Vec<usize> = self
                .scan_win
                .reports
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.kind == Kind::Plugin && matches!(r.state, State::Todo | State::Almost)
                })
                .map(|(i, _)| i)
                .collect();
            self.scan_open(&idx);
        }
        if export {
            let path = PathBuf::from(self.scan_win.csv.trim());
            let csv = scan::to_csv(&self.scan_win.reports);
            self.scan_win.message = Some(
                match crate::config::paths::write_atomic(&path, csv.as_bytes()) {
                    Ok(()) => self
                        .i18n
                        .t_arg("scan-exported", "path", &path.display().to_string()),
                    Err(e) => self.i18n.t_arg("err-save", "error", &e.to_string()),
                },
            );
        }
    }
}
