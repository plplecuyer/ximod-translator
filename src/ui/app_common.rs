//! Lignes communes (C-04): the Communes tab of the edit window — the lines
//! with the same source in the file, in the other open files (B-25) and in
//! the dictionaries — and the propagation dialog (A-12) extended to the
//! other open files.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use eframe::egui;

use super::XimodApp;
use crate::common::{self, SourceIndex};

/// A line of a file in the tab: its document (by serial) and its index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    serial: u64,
    idx: usize,
}

/// What the differences frame compares with the line (A-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Line(usize),
    Pair(usize),
}

/// What the user asked of the tab.
pub(crate) enum CommonAction {
    /// Appliquer à toutes (Ctrl+F6).
    Apply,
    /// Aller à la ligne / double-clic: (document serial, line).
    GoTo(u64, usize),
}

/// The state of the Communes tab and the source indexes of the files.
#[derive(Default)]
pub(crate) struct CommonState {
    /// Signature of what `rows` was computed for.
    key: Option<u64>,
    /// Per document serial: (lines, sources revision, index).
    indexes: HashMap<u64, (usize, u64, Arc<SourceIndex>)>,
    rows: Vec<Row>,
    here: usize,
    others: usize,
    /// The distinct translations of the dictionaries (B-08), and how many.
    pairs: Vec<crate::ambiguity::Candidate>,
    pairs_total: usize,
    pick: Option<Pick>,
    diff: Option<(Pick, crate::diff::Diff)>,
    diff_swapped: bool,
    /// Bring the differences frame into view once (just picked).
    scroll_diff: bool,
}

impl XimodApp {
    /// The files the Communes tab and the propagation look in, the active
    /// one first then the tabs' order: the other open files with the same
    /// source and target languages (as B-25). In « Tous les mods » the
    /// synthesis alone, its lines of other plugins counting as other
    /// files.
    fn common_files(&self) -> (Vec<usize>, bool) {
        let active = self.active;
        let Some(doc) = self.docs.get(active) else {
            return (Vec::new(), false);
        };
        if doc.all.is_some() {
            return (vec![active], true);
        }
        let (src, dst) = (&doc.settings.source, &doc.settings.target);
        let mut order = vec![active];
        order.extend(self.docs.iter().enumerate().filter_map(|(i, d)| {
            (i != active
                && d.all.is_none()
                && !dst.trim().is_empty()
                && &d.settings.source == src
                && &d.settings.target == dst)
                .then_some(i)
        }));
        (order, false)
    }

    /// The source index of a document, built once per version of its lines
    /// (R-C04-04).
    fn common_index(&mut self, di: usize) -> Arc<SourceIndex> {
        let d = &self.docs[di];
        let (serial, len, rev) = (d.serial, d.tm.entries.len(), d.sources_rev);
        if let Some((l, r, ix)) = self.common.indexes.get(&serial)
            && *l == len
            && *r == rev
        {
            return ix.clone();
        }
        let start = std::time::Instant::now();
        let ix = Arc::new(SourceIndex::build(&d.tm.entries));
        tracing::debug!(
            "common: index of {} lines, {} sources in {:?}",
            len,
            ix.len(),
            start.elapsed()
        );
        // The indexes of closed documents go.
        let live: Vec<u64> = self.docs.iter().map(|d| d.serial).collect();
        self.common.indexes.retain(|s, _| live.contains(s));
        self.common.indexes.insert(serial, (len, rev, ix.clone()));
        ix
    }

    /// The propagation groups of `origins` in the active document, joined
    /// by the lines of the other open files (R-C04-03); the empty ones
    /// left out.
    pub(crate) fn propagate_groups(&mut self, origins: &[usize]) -> Vec<crate::propagate::Group> {
        let (order, _) = self.common_files();
        let Some(&active) = order.first() else {
            return Vec::new();
        };
        let mut groups = crate::propagate::groups_open(&self.docs[active].tm.entries, origins);
        for &di in &order[1..] {
            let ix = self.common_index(di);
            crate::propagate::extend(&mut groups, di, &self.docs[di].tm.entries, &ix);
        }
        crate::propagate::drop_empty(&mut groups);
        groups
    }

    /// Recompute the tab when the line, a file or the open files changed
    /// (R-C04-04).
    pub(crate) fn common_update(&mut self, line: usize) {
        let (order, split) = self.common_files();
        if order.is_empty() {
            return;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        line.hash(&mut h);
        split.hash(&mut h);
        for &di in &order {
            let d = &self.docs[di];
            (d.serial, d.undo.revision, d.sources_rev, d.tm.entries.len()).hash(&mut h);
        }
        let key = h.finish();
        if self.common.key == Some(key) {
            return;
        }
        let start = std::time::Instant::now();
        let indexes: Vec<Arc<SourceIndex>> =
            order.iter().map(|&di| self.common_index(di)).collect();
        let found = {
            let files: Vec<common::File<'_>> = order
                .iter()
                .zip(&indexes)
                .map(|(&di, ix)| common::File {
                    entries: &self.docs[di].tm.entries,
                    index: ix,
                })
                .collect();
            common::gather(&files, 0, line, split)
        };
        let st = &mut self.common;
        st.key = Some(key);
        st.rows = found
            .hits
            .iter()
            .map(|h| Row {
                serial: self.docs[order[h.file]].serial,
                idx: h.idx,
            })
            .collect();
        st.here = found.here;
        st.others = found.others;
        st.pick = None;
        st.diff = None;
        // The dictionaries of the line's own file (B-08).
        let mut pairs = self.common_pairs(line);
        let st = &mut self.common;
        st.pairs_total = pairs.len();
        pairs.truncate(common::MAX_PAIRS);
        st.pairs = pairs;
        tracing::debug!(
            "common: line {} — {} lines, {} pairs in {:?}",
            line,
            found.total(),
            self.common.pairs_total,
            start.elapsed()
        );
    }

    /// The distinct translations of the line's source in the dictionaries
    /// of its file (R-C04-01).
    fn common_pairs(&mut self, line: usize) -> Vec<crate::ambiguity::Candidate> {
        let active = self.active;
        let (di, li) = if self.docs[active].all.is_some() {
            match self.all_origin(line) {
                Some(o) => o,
                None => return Vec::new(),
            }
        } else {
            (active, line)
        };
        let d = &self.docs[di];
        let Some(e) = d.tm.entries.get(li) else {
            return Vec::new();
        };
        if e.unused || common::key(&e.source).is_empty() {
            return Vec::new();
        }
        let (path, settings, source) = (d.path.clone(), d.settings.clone(), e.source.clone());
        match self.resolve_stack(&path, &settings) {
            Some(stack) => stack.candidates(&source),
            None => Vec::new(),
        }
    }

    /// The number in the tab's title: the lines of the files.
    pub(crate) fn common_count(&self) -> usize {
        self.common.here + self.common.others
    }

    /// The text a pick compares with the line's translation.
    fn common_pick_text(&self, pick: Pick) -> Option<String> {
        match pick {
            Pick::Line(r) => {
                let row = self.common.rows.get(r)?;
                let di = self.doc_of_serial(row.serial)?;
                Some(self.docs[di].tm.entries.get(row.idx)?.trans.clone())
            }
            Pick::Pair(p) => Some(self.common.pairs.get(p)?.trans.clone()),
        }
    }

    /// The Communes tab (R-C04-01 to R-C04-03).
    pub(crate) fn common_tab(&mut self, ui: &mut egui::Ui, line: usize) -> Option<CommonAction> {
        let active = self.active;
        let e = self.docs[active].tm.entries.get(line)?;
        let (current, unused) = (e.trans.clone(), e.unused);
        let is_all = self.docs[active].all.is_some();
        let colors = self.config.status_colors;
        let legends: [String; crate::status::COUNT] =
            std::array::from_fn(|i| self.i18n.t(crate::status::Status::ALL[i].legend_key()));
        let headers = [
            self.i18n.t("common-col-status"),
            self.i18n.t("common-col-file"),
            self.i18n.t("common-col-record"),
            self.i18n.t("common-col-edid"),
            self.i18n.t("common-col-trans"),
            self.i18n.t("common-col-prov"),
        ];
        let differ_c = egui::Color32::from_rgb(20, 60, 170);
        let differ_c = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(120, 160, 255)
        } else {
            differ_c
        };
        let mut action: Option<CommonAction> = None;
        let mut picked: Option<Pick> = None;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let max_h = (ui.available_height() - 58.0).max(48.0);

        // The rows, read before the table borrows the UI.
        struct Shown {
            serial: u64,
            idx: usize,
            st: crate::status::Status,
            file: String,
            rec: String,
            edid: String,
            trans: String,
            prov: String,
        }
        let shown: Vec<Shown> = self
            .common
            .rows
            .iter()
            .filter_map(|r| {
                let di = self.doc_of_serial(r.serial)?;
                let d = &self.docs[di];
                let e = d.tm.entries.get(r.idx)?;
                let rec = if e.index > 1 {
                    format!("{}:{} #{}", e.rec_sig, e.field_sig, e.index)
                } else {
                    format!("{}:{}", e.rec_sig, e.field_sig)
                };
                Some(Shown {
                    serial: r.serial,
                    idx: r.idx,
                    st: crate::status::derive(e),
                    file: if is_all {
                        e.plugin.clone()
                    } else {
                        d.file_name()
                    },
                    rec,
                    edid: e.edid.clone(),
                    trans: e.trans.clone(),
                    prov: crate::provenance::short_label(&self.i18n, e),
                })
            })
            .collect();
        let cur_pick = self.common.pick;

        egui::ScrollArea::vertical()
            .id_salt("ximod-common")
            .max_height(max_h)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if shown.is_empty() {
                    ui.label(egui::RichText::new(self.i18n.t("common-none")).weak());
                } else {
                    if self.common_count() > shown.len() {
                        ui.label(
                            egui::RichText::new(self.i18n.t_args(
                                "common-limit",
                                &[
                                    ("n", &shown.len().to_string()),
                                    ("total", &self.common_count().to_string()),
                                ],
                            ))
                            .weak()
                            .small(),
                        );
                    }
                    let avail_w = ui.available_width();
                    let fixed = 34.0 + 90.0 + 110.0 + 110.0;
                    let rest = (avail_w - fixed - 30.0).max(160.0);
                    let file_w = (rest * 0.3).clamp(110.0, 240.0);
                    let trans_w = (rest - file_w) * 0.6;
                    egui_extras::TableBuilder::new(ui)
                        .id_salt("ximod-common-table")
                        .striped(true)
                        .vscroll(false)
                        .sense(egui::Sense::click())
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(egui_extras::Column::exact(34.0))
                        .column(egui_extras::Column::exact(file_w))
                        .column(egui_extras::Column::exact(90.0))
                        .column(egui_extras::Column::exact(110.0))
                        .column(egui_extras::Column::exact(trans_w.max(80.0)))
                        .column(egui_extras::Column::remainder())
                        .header(row_h, |mut h| {
                            for t in &headers {
                                h.col(|ui| {
                                    ui.label(egui::RichText::new(t).strong().small());
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, shown.len(), |mut row| {
                                let ri = row.index();
                                let s = &shown[ri];
                                row.set_selected(cur_pick == Some(Pick::Line(ri)));
                                let (r, g, b) = colors[s.st.index()];
                                row.col(|ui| {
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(12.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        2.0,
                                        egui::Color32::from_rgb(r, g, b),
                                    );
                                    resp.on_hover_text(&legends[s.st.index()]);
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&s.file).weak())
                                            .truncate()
                                            .selectable(false),
                                    )
                                    .on_hover_text(&s.file);
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&s.rec).monospace())
                                            .truncate()
                                            .selectable(false),
                                    );
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(&s.edid).truncate().selectable(false))
                                        .on_hover_text(&s.edid);
                                });
                                row.col(|ui| {
                                    // Another translation in blue, the same in
                                    // grey (R-C04-02, the colours of A-17).
                                    let text = ew_line(&s.trans);
                                    let rt = if s.trans == current {
                                        egui::RichText::new(text).weak()
                                    } else {
                                        egui::RichText::new(text).color(differ_c)
                                    };
                                    ui.add(egui::Label::new(rt).truncate().selectable(false))
                                        .on_hover_text(&s.trans);
                                });
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&s.prov).small())
                                            .truncate()
                                            .selectable(false),
                                    )
                                    .on_hover_text(&s.prov);
                                });
                                let resp = row.response();
                                if resp.double_clicked() {
                                    action = Some(CommonAction::GoTo(s.serial, s.idx));
                                } else if resp.clicked() {
                                    picked = Some(Pick::Line(ri));
                                }
                            });
                        });
                }
                // The dictionaries, folded (R-C04-01).
                if self.common.pairs_total > 0 {
                    ui.add_space(4.0);
                    let title =
                        self.i18n
                            .t_arg("common-dicts", "n", &self.common.pairs_total.to_string());
                    egui::CollapsingHeader::new(egui::RichText::new(title).small().strong())
                        .id_salt("ximod-common-dicts")
                        .default_open(false)
                        .show(ui, |ui| {
                            for (pi, c) in self.common.pairs.iter().enumerate() {
                                let row = ui.horizontal(|ui| {
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
                                    let rt = egui::RichText::new(ew_line(&c.trans));
                                    let rt = if c.trans.trim() == current.trim() {
                                        rt.weak()
                                    } else {
                                        rt.color(differ_c.gamma_multiply(0.75))
                                    };
                                    if ui
                                        .add(
                                            egui::Label::new(rt)
                                                .truncate()
                                                .selectable(false)
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(&c.trans)
                                        .clicked()
                                    {
                                        picked = Some(Pick::Pair(pi));
                                    }
                                    ui.label(egui::RichText::new(dicts.join(" · ")).weak().small());
                                });
                                if cur_pick == Some(Pick::Pair(pi)) {
                                    Self::mark_picked_row(ui, row.response.rect);
                                }
                            }
                        });
                }
                // « Différences caractère par caractère » (A-08, R-C04-02).
                if let Some((_, diff)) = &self.common.diff {
                    let (title, line_l, entry_l, swap_l, legend_l, legend2_l, word_l) = (
                        self.i18n.t("diff-title"),
                        self.i18n.t("diff-trans-line"),
                        self.i18n.t("common-diff-entry"),
                        self.i18n.t("diff-swap"),
                        self.i18n.t("common-diff-legend"),
                        self.i18n.t("common-diff-legend-swapped"),
                        self.i18n.t("diff-word-mode"),
                    );
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
                    let mut swapped = self.common.diff_swapped;
                    let shown = ui
                        .scope(|ui| crate::ui::diff_view::show(ui, diff, &mut swapped, &labels))
                        .response;
                    if self.common.scroll_diff {
                        shown.scroll_to_me(Some(egui::Align::Max));
                        self.common.scroll_diff = false;
                    }
                    self.common.diff_swapped = swapped;
                }
            });
        if let Some(p) = picked {
            self.common.pick = Some(p);
            self.common.diff = match self.common_pick_text(p) {
                Some(t) if t != current => Some((p, crate::diff::compute(&current, &t))),
                _ => None,
            };
            self.common.scroll_diff = self.common.diff.is_some();
        }

        // The summary and the actions (3).
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(self.i18n.t_args(
                "common-summary",
                &[
                    ("here", &self.common.here.to_string()),
                    ("others", &self.common.others.to_string()),
                    ("dicts", &self.common.pairs_total.to_string()),
                ],
            ))
            .weak()
            .small(),
        );
        let goto = match self.common.pick {
            Some(Pick::Line(r)) => self.common.rows.get(r).copied(),
            _ => None,
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !unused && !current.is_empty(),
                    egui::Button::new(egui::RichText::new(self.i18n.t("common-apply")).strong()),
                )
                .clicked()
            {
                action = Some(CommonAction::Apply);
            }
            if ui
                .add_enabled(
                    goto.is_some(),
                    egui::Button::new(self.i18n.t("common-goto")),
                )
                .clicked()
                && let Some(r) = goto
            {
                action = Some(CommonAction::GoTo(r.serial, r.idx));
            }
            ui.label(
                egui::RichText::new(self.i18n.t("common-goto-hint"))
                    .weak()
                    .small(),
            );
        });
        action
    }

    /// Aller à la ligne (R-C04-03): the line's file becomes the active tab,
    /// the line is selected and the edit window shows it.
    pub(crate) fn common_go_to(
        &mut self,
        win: &mut crate::ui::edit_window::EditWindow,
        serial: u64,
        idx: usize,
    ) {
        let Some(di) = self.doc_of_serial(serial) else {
            return;
        };
        if di != self.active {
            self.close_edit_session();
            self.active = di;
            self.selection.clear();
        }
        self.edit_go_to(win, idx);
    }
}

/// One line of a text for a cell.
fn ew_line(text: &str) -> String {
    crate::ui::edit_window::one_line(text, 200)
}
