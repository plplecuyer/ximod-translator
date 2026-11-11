//! Dictionnaires › Comparer deux dictionnaires… (D-07, figure B.7): an
//! independent window — A and B (a file, or a document open), the pairing
//! and the folding, the account, the four lists, the CSV report and the
//! merge into a dictionary.

use crate::ui::theme::ComboField as _;
use std::path::PathBuf;

use eframe::egui;

use super::{XimodApp, read_geometry};
use crate::compare::{self, Category, Mode, Prefer, Report};

/// One side: a file, or a document open (its serial).
#[derive(Default, Clone)]
pub(crate) struct SideSel {
    pub path: String,
    pub doc: Option<u64>,
}

pub(crate) struct CompareWin {
    pub open: bool,
    pub needs_place: bool,
    pub a: SideSel,
    pub b: SideSel,
    pub mode: Mode,
    pub fold: bool,
    pub report: Option<Report>,
    pub tab: Category,
    pub prefer: Prefer,
    pub csv: String,
    pub merge_out: String,
    pub message: Option<String>,
    pub error: Option<String>,
    /// The names of A and B in the report.
    pub names: (String, String),
    /// Fermer was clicked.
    pub closing: bool,
}

impl Default for CompareWin {
    fn default() -> Self {
        CompareWin {
            open: false,
            needs_place: false,
            a: SideSel::default(),
            b: SideSel::default(),
            mode: Mode::Record,
            fold: true,
            report: None,
            tab: Category::Diff,
            prefer: Prefer::A,
            csv: String::new(),
            merge_out: String::new(),
            message: None,
            error: None,
            names: (String::new(), String::new()),
            closing: false,
        }
    }
}

impl XimodApp {
    /// Dictionnaires › Comparer deux dictionnaires….
    pub(crate) fn open_compare_window(&mut self) {
        // The active document as A, the first time.
        if self.compare_win.a.path.is_empty()
            && self.compare_win.a.doc.is_none()
            && let Some(d) = self.active_doc()
        {
            self.compare_win.a.doc = Some(d.serial);
        }
        self.compare_win.open = true;
        self.compare_win.needs_place = true;
    }

    /// A side's lines and name.
    fn compare_side(&self, s: &SideSel) -> Result<(crate::tm::TranslationMemory, String), String> {
        if let Some(serial) = s.doc
            && let Some(d) = self.docs.iter().find(|d| d.serial == serial)
        {
            return Ok((d.tm.clone(), d.file_name()));
        }
        let p = PathBuf::from(s.path.trim());
        if !p.is_file() {
            return Err(self
                .i18n
                .t_arg("err-open", "error", &p.display().to_string()));
        }
        let (_, src, _) = self.scan_pair_pub();
        let tm = compare::read_side(&p, &src).map_err(|e| self.i18n.localize(&e))?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok((tm, name))
    }

    fn compare_run(&mut self) {
        self.compare_win.error = None;
        self.compare_win.message = None;
        let a = self.compare_side(&self.compare_win.a.clone());
        let b = self.compare_side(&self.compare_win.b.clone());
        match (a, b) {
            (Ok((ta, na)), Ok((tb, nb))) => {
                let r = compare::compare(&ta, &tb, self.compare_win.mode, self.compare_win.fold);
                // The default files: next to A, under its name.
                let (game, _, dst) = self.scan_pair_pub();
                let stem = std::path::Path::new(&na)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "merge".into());
                let dir = std::path::Path::new(self.compare_win.a.path.trim())
                    .parent()
                    .map(|p| p.to_path_buf())
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| self.config.mod_base_dir());
                // Proposed again at each comparison: a merge never goes to the
                // files of an earlier pair by mistake.
                {
                    self.compare_win.csv = dir
                        .join(format!("{stem}_compare.csv"))
                        .display()
                        .to_string();
                }
                {
                    let mut s = crate::doc_settings::DocSettings::defaults(&self.config);
                    s.game = game;
                    s.target = dst;
                    let mods = self.config.mod_base_dir().join(s.game.trim());
                    self.compare_win.merge_out = mods
                        .join(format!("{stem}_{}.ximdb", s.pair_suffix()))
                        .display()
                        .to_string();
                }
                self.compare_win.names = (na, nb);
                self.compare_win.report = Some(r);
            }
            (Err(e), _) | (_, Err(e)) => {
                self.compare_win.error = Some(e);
                self.compare_win.report = None;
            }
        }
    }

    /// The game and pair of the toolbar (shared with the analysis).
    pub(crate) fn scan_pair_pub(&self) -> (String, String, String) {
        let s = match self.active_doc() {
            Some(d) => d.settings.clone(),
            None => crate::doc_settings::DocSettings::for_new_doc(
                crate::games::Game::Unknown,
                &self.idle,
                &self.config,
            ),
        };
        (s.game, s.source, s.target)
    }

    /// The line of an open document in the grid.
    fn compare_reveal(&mut self, serial: u64, line: usize) {
        let Some(i) = self.docs.iter().position(|d| d.serial == serial) else {
            return;
        };
        self.close_edit_session();
        if self.active != i {
            self.active = i;
            self.selection.clear();
        }
        self.selection.set_single(line);
        self.grid_scroll_to = Some(line);
    }

    pub(crate) fn compare_window(&mut self, ctx: &egui::Context) {
        if !self.compare_win.open {
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-compare");
        let [sx, sy, sw, sh] = self.config.compare_win;
        let just_opened = self.compare_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(780.0), sh.max(460.0)],
            )
        });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t("compare-title"))
            .with_min_inner_size([780.0, 460.0]);
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
            self.compare_win.needs_place = false;
            egui::CentralPanel::default().show(vctx, |ui| self.compare_body(ui));
            if std::mem::take(&mut self.compare_win.closing)
                || vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.compare_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.compare_win.open = false;
            let _ = self.config.save();
        }
    }

    /// A line « A : [document ▾] [file] [Parcourir…] ».
    fn compare_side_ui(&mut self, ui: &mut egui::Ui, b: bool) {
        let label = self.i18n.t(if b { "compare-b" } else { "compare-a" });
        let none = self.i18n.t("compare-file");
        let docs: Vec<(u64, String)> = self
            .docs
            .iter()
            .filter(|d| d.all.is_none())
            .map(|d| {
                (
                    d.serial,
                    self.i18n.t_arg("compare-open-doc", "name", &d.file_name()),
                )
            })
            .collect();
        let browse = self.i18n.t("scan-browse");
        let side = if b {
            &mut self.compare_win.b
        } else {
            &mut self.compare_win.a
        };
        ui.horizontal(|ui| {
            ui.add_sized(
                [24.0, 18.0],
                egui::Label::new(egui::RichText::new(&label).strong()),
            );
            let shown = side
                .doc
                .and_then(|s| docs.iter().find(|d| d.0 == s))
                .map(|d| d.1.clone())
                .unwrap_or_else(|| none.clone());
            egui::ComboBox::from_id_salt(if b { "ximod-cmp-b" } else { "ximod-cmp-a" })
                .selected_text(shown)
                .width(260.0)
                .show_field(ui, |ui| {
                    ui.selectable_value(&mut side.doc, None, &none);
                    for (s, n) in &docs {
                        ui.selectable_value(&mut side.doc, Some(*s), n);
                    }
                });
            ui.add_enabled(
                side.doc.is_none(),
                egui::TextEdit::singleline(&mut side.path).desired_width(420.0),
            );
            if ui
                .add_enabled(side.doc.is_none(), egui::Button::new(&browse))
                .clicked()
                && let Some(p) = rfd::FileDialog::new().pick_file()
            {
                side.path = p.display().to_string();
            }
        });
    }

    fn compare_body(&mut self, ui: &mut egui::Ui) {
        self.compare_side_ui(ui, false);
        self.compare_side_ui(ui, true);
        let t = |k: &str| self.i18n.t(k);
        let l_close = t("btn-close");
        let (l_mode, l_rec, l_text, l_fold, l_run) = (
            t("compare-mode"),
            t("compare-mode-record"),
            t("compare-mode-text"),
            t("compare-fold"),
            t("compare-run"),
        );
        let mut run = false;
        ui.horizontal(|ui| {
            ui.label(&l_mode);
            ui.radio_value(&mut self.compare_win.mode, Mode::Record, &l_rec);
            ui.radio_value(&mut self.compare_win.mode, Mode::Text, &l_text);
            ui.checkbox(&mut self.compare_win.fold, &l_fold);
            if ui.button(&l_run).clicked() {
                run = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(&l_close).clicked() {
                    self.compare_win.closing = true;
                }
            });
        });
        if run {
            self.compare_run();
        }
        let t = |k: &str| self.i18n.t(k);
        if let Some(e) = &self.compare_win.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let Some(r) = self.compare_win.report.clone() else {
            return;
        };
        let n = |c| r.count(c).to_string();
        let mut summary = self.i18n.t_args(
            "compare-summary",
            &[
                ("a", &r.a_lines.to_string()),
                ("b", &r.b_lines.to_string()),
                ("common", &r.common().to_string()),
                ("same", &n(Category::Same)),
                ("diff", &n(Category::Diff)),
                ("onlyA", &n(Category::OnlyA)),
                ("onlyB", &n(Category::OnlyB)),
            ],
        );
        if r.mode == Some(Mode::Text) && self.compare_win.mode == Mode::Record {
            summary = self.i18n.dash(&summary, &t("compare-text-fallback"));
        }
        ui.label(egui::RichText::new(summary).strong());
        ui.horizontal(|ui| {
            for (c, k) in [
                (Category::Diff, "compare-tab-diff"),
                (Category::Same, "compare-tab-same"),
                (Category::OnlyA, "compare-tab-a"),
                (Category::OnlyB, "compare-tab-b"),
            ] {
                let label = self.i18n.t_arg(k, "n", &r.count(c).to_string());
                ui.selectable_value(&mut self.compare_win.tab, c, label);
            }
        });
        ui.separator();
        // The bottom: report and merge.
        let (l_export, l_merge) = (t("compare-export"), t("compare-merge"));
        let (pa, pb) = (
            self.i18n.t_arg("compare-prefer", "side", "A"),
            self.i18n.t_arg("compare-prefer", "side", "B"),
        );
        let mut export = false;
        let mut merge = false;
        egui::TopBottomPanel::bottom("ximod-compare-bottom")
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.compare_win.csv).desired_width(380.0),
                    );
                    if ui.button(&l_export).clicked() {
                        export = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.compare_win.merge_out)
                            .desired_width(380.0),
                    );
                    ui.radio_value(&mut self.compare_win.prefer, Prefer::A, &pa);
                    ui.radio_value(&mut self.compare_win.prefer, Prefer::B, &pb);
                    if ui.button(&l_merge).clicked() {
                        merge = true;
                    }
                });
                if let Some(m) = &self.compare_win.message {
                    ui.label(egui::RichText::new(m).weak());
                }
            });
        // The list of the tab.
        let rows: Vec<&compare::Row> = r
            .rows
            .iter()
            .filter(|x| x.category == self.compare_win.tab)
            .collect();
        let heads = [
            t("col-grup"),
            t("col-edid"),
            t("col-source"),
            t("compare-col-a"),
            t("compare-col-b"),
        ];
        let text_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        let fixed = 80.0 + 200.0 + 6.0 * 8.0 + 16.0;
        let w3 = ((ui.available_width() - fixed) / 3.0).max(150.0);
        let widths = [80.0, 200.0, w3, w3, w3];
        let mut reveal: Option<(bool, usize)> = None;
        let head: Vec<egui::RichText> = heads
            .iter()
            .map(|h| egui::RichText::new(h).strong())
            .collect();
        super::scan_window::table_row(
            ui,
            egui::Id::new("ximod-compare-head"),
            &head,
            &widths,
            text_h,
            false,
        );
        egui::ScrollArea::vertical()
            .id_salt("ximod-compare-rows")
            .auto_shrink([false, false])
            .show_rows(ui, text_h, rows.len(), |ui, range| {
                for i in range {
                    let row = rows[i];
                    let cells = [
                        format!("{} {}", row.rec, row.field),
                        row.edid.clone(),
                        crate::ui::edit_window::one_line(&row.source, 80),
                        crate::ui::edit_window::one_line(&row.a.join(" | "), 80),
                        crate::ui::edit_window::one_line(&row.b.join(" | "), 80),
                    ];
                    let cells: Vec<egui::RichText> =
                        cells.into_iter().map(egui::RichText::new).collect();
                    let resp = super::scan_window::table_row(
                        ui,
                        egui::Id::new(("ximod-compare-row", i)),
                        &cells,
                        &widths,
                        text_h,
                        false,
                    );
                    if resp.double_clicked() {
                        reveal = match (row.line_a, row.line_b) {
                            (Some(l), _) if self.compare_win.a.doc.is_some() => Some((false, l)),
                            (_, Some(l)) if self.compare_win.b.doc.is_some() => Some((true, l)),
                            _ => None,
                        };
                    }
                }
            });
        if let Some((b, line)) = reveal {
            let serial = if b {
                self.compare_win.b.doc
            } else {
                self.compare_win.a.doc
            };
            if let Some(s) = serial {
                self.compare_reveal(s, line);
            }
        }
        if export {
            let path = PathBuf::from(self.compare_win.csv.trim());
            let (na, nb) = self.compare_win.names.clone();
            let csv = compare::to_csv(&r, &na, &nb);
            self.compare_win.message = Some(
                match crate::config::paths::write_atomic(&path, csv.as_bytes()) {
                    Ok(()) => self
                        .i18n
                        .t_arg("scan-exported", "path", &path.display().to_string()),
                    Err(e) => self.i18n.t_arg("err-save", "error", &e.to_string()),
                },
            );
        }
        if merge {
            let path = PathBuf::from(self.compare_win.merge_out.trim());
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let (rows, amb) = compare::merge(&r, self.compare_win.prefer, &file);
            let (_, _, dst) = self.scan_pair_pub();
            self.compare_win.message = Some(match compare::write_merge(rows, &dst, &path) {
                Ok(n) => {
                    self.file_dbs.invalidate(Some(&path));
                    self.i18n.t_args(
                        "compare-merged",
                        &[
                            ("path", &path.display().to_string()),
                            ("n", &n.to_string()),
                            ("amb", &amb.to_string()),
                        ],
                    )
                }
                Err(e) => self
                    .i18n
                    .t_arg("err-save", "error", &self.i18n.localize(&e)),
            });
        }
    }
}
