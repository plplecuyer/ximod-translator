//! Outils ▸ Dialogues… (C-01, C-02, C-13): the tree of the document's
//! dialogues on the left, the selected reply on the right. The state lives
//! here; the application feeds the window with the dialogue map and the
//! document's lines and applies the actions it returns.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui;

use crate::dialogue::{DialogueMap, NodeKind, SpeakerKind};
use crate::i18n::I18n;
use crate::icons;
use crate::plugin::context::ContextMap;
use crate::tm::TmEntry;

/// A background context pass.
pub struct CtxJob {
    pub serial: u64,
    pub rx: std::sync::mpsc::Receiver<CtxMsg>,
    pub progress: f32,
}

pub enum CtxMsg {
    Progress(usize, usize),
    Done(Box<Result<ContextMap, String>>),
}

/// A background name index of a master.
pub struct MasterJob {
    pub rx: std::sync::mpsc::Receiver<CtxMsg>,
    pub progress: f32,
}

#[derive(Default)]
pub struct DialogWindow {
    pub open: bool,
    pub needs_place: bool,
    /// The map of the active document, rebuilt when its lines change.
    pub map: Option<Arc<DialogueMap>>,
    /// (document serial, undo revision, masters known, voices generation)
    /// the map was built for.
    pub map_key: Option<(u64, u64, usize, u64)>,
    /// The voice map of the active document (C-13), the serial it was built
    /// for, and a generation bumped by « Rafraîchir ».
    pub voices: Option<Arc<crate::voices::VoiceMap>>,
    pub voices_for: Option<u64>,
    pub voices_gen: u64,
    /// Context maps by document serial.
    pub ctx_cache: HashMap<u64, Arc<ContextMap>>,
    pub ctx_job: Option<CtxJob>,
    /// Documents whose context failed (no second try this session).
    pub ctx_failed: HashSet<u64>,
    /// Name indexes of the masters, by file.
    pub masters: HashMap<std::path::PathBuf, Arc<ContextMap>>,
    pub master_jobs: HashMap<std::path::PathBuf, MasterJob>,
    /// Masters looked for and not found (by name), per session.
    pub masters_missing: HashSet<String>,
    // ---- view state
    pub selected: Option<u32>,
    pub expanded: HashSet<usize>,
    pub expanded_for: Option<u64>,
    pub follow_grid: bool,
    pub split: f32,
    pub filter_quest: Option<u32>,
    pub filter_npc: Option<String>,
    pub search: String,
    pub search_typed_at: Option<f64>,
    pub search_applied: String,
    pub unvoiced_only: bool,
    /// The grid line last followed, to react to changes only.
    pub followed_line: Option<usize>,
    /// Scroll the tree to the selected node on the next frame.
    pub scroll_to_selected: bool,
    pub message: Option<String>,
}

/// One response's voice line (C-13): the text, whether a voice file was
/// found, the tooltip (paths), and the folder of a loose file.
#[derive(Debug, Clone, Default)]
pub struct VoiceLine {
    pub text: String,
    pub ok: bool,
    pub tip: String,
    pub folder: Option<std::path::PathBuf>,
}

/// What the window asks the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogAction {
    None,
    Close,
    /// Select a document line in the grid (and scroll to it).
    SelectLine(usize),
    /// Open the edit window on a line.
    OpenLine(usize),
    /// Filter the grid on a speaker key.
    FilterGrid(String),
    /// Copy text to the clipboard.
    Copy(String),
    /// Rescan the voice files.
    RefreshVoices,
    /// Open a loose voice file's folder (C-13).
    OpenFolder(std::path::PathBuf),
    /// Copy the list of the unvoiced replies (C-13).
    CopyUnvoiced,
    /// Open the Dialogues window (from the edit window's tab).
    OpenWindow,
}

impl DialogWindow {
    /// Select a reply and expand its ancestors.
    pub fn select_info(&mut self, map: &DialogueMap, info: u32) {
        self.selected = Some(info);
        if let Some(&n) = map.info_node.get(&info) {
            let mut p = map.nodes[n].parent;
            while let Some(i) = p {
                self.expanded.insert(i);
                p = map.nodes[i].parent;
            }
            self.scroll_to_selected = true;
        }
    }

    /// Whether a node (or one of its descendants) passes the filters.
    fn visible(
        &self,
        map: &DialogueMap,
        entries: &[TmEntry],
        n: usize,
        memo: &mut HashMap<usize, bool>,
    ) -> bool {
        if let Some(&v) = memo.get(&n) {
            return v;
        }
        let node = &map.nodes[n];
        let own = match node.kind {
            NodeKind::Info(f) | NodeKind::Response(f, _) => self.info_matches(map, entries, f, n),
            _ => false,
        };
        let v = own
            || node
                .children
                .clone()
                .iter()
                .any(|c| self.visible(map, entries, *c, memo));
        // A quest filter: the quest nodes themselves.
        let v = v && self.quest_allows(map, n);
        memo.insert(n, v);
        v
    }

    fn quest_allows(&self, map: &DialogueMap, n: usize) -> bool {
        let Some(q) = self.filter_quest else {
            return true;
        };
        let mut cur = Some(n);
        while let Some(i) = cur {
            if let NodeKind::Quest(f) = map.nodes[i].kind {
                return f == q;
            }
            if map.nodes[i].kind == NodeKind::NoQuest {
                return false;
            }
            cur = map.nodes[i].parent;
        }
        false
    }

    fn info_matches(&self, map: &DialogueMap, entries: &[TmEntry], info: u32, n: usize) -> bool {
        if let Some(key) = &self.filter_npc {
            let ok = map
                .speakers
                .get(&info)
                .map(|l| l.iter().any(|s| s.key() == *key))
                .unwrap_or(false);
            if !ok {
                return false;
            }
        }
        if self.unvoiced_only && map.nodes[n].unvoiced == 0 {
            return false;
        }
        let needle = self.search_applied.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        // The lines of the reply: responses and prompt.
        let lines: Vec<usize> = map
            .line_node
            .iter()
            .filter(|(_, node)| **node == n || map.nodes[**node].parent == Some(n))
            .map(|(l, _)| *l)
            .collect();
        lines.iter().any(|l| {
            entries
                .get(*l)
                .map(|e| {
                    e.source.to_lowercase().contains(&needle)
                        || e.trans.to_lowercase().contains(&needle)
                })
                .unwrap_or(false)
        }) || map.nodes[n].label.to_lowercase().contains(&needle)
    }
}

/// Everything the body needs from the application.
pub struct Feed<'a> {
    pub map: &'a DialogueMap,
    pub entries: &'a [TmEntry],
    pub i18n: &'a I18n,
    pub colors: [(u8, u8, u8); crate::status::COUNT],
    /// Progress of a master index being built, if any.
    pub master_progress: Option<(String, f32)>,
    /// Voice lines of the selected reply, per response (C-13); `None` until
    /// the voices are mapped.
    pub voices: Option<Vec<VoiceLine>>,
    pub voices_note: Option<String>,
    /// Conditions of the selected reply, decoded (C-02): (text, raw).
    pub conditions: Vec<(String, String)>,
}

/// The body of the window.
pub fn body(ui: &mut egui::Ui, st: &mut DialogWindow, feed: &Feed) -> DialogAction {
    let map = feed.map;
    let i18n = feed.i18n;
    let mut action = DialogAction::None;
    // ---- keyboard
    let (esc, enter, left, right) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::ALT,
                egui::Key::ArrowLeft,
            ),
            i.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::ALT,
                egui::Key::ArrowRight,
            ),
        )
    });
    if esc {
        return DialogAction::Close;
    }
    if let Some(sel) = st.selected {
        if (left || right)
            && let Some(next) = map.step(sel, if right { 1 } else { -1 })
        {
            st.select_info(map, next);
            if let Some(l) = map.nodes[map.info_node[&next]].line {
                action = DialogAction::SelectLine(l);
            }
        }
        if enter
            && let Some(&n) = map.info_node.get(&sel)
            && let Some(l) = map.nodes[n].line
        {
            action = DialogAction::OpenLine(l);
        }
    }
    // ---- status bar (bottom) and filters (top)
    let unvoiced: usize = map.roots.iter().map(|r| map.nodes[*r].unvoiced).sum();
    let done: usize = map.roots.iter().map(|r| map.nodes[*r].done).sum();
    let total: usize = map.roots.iter().map(|r| map.nodes[*r].total).sum();
    let speakers = map.speaker_counts(i18n);
    let mut filters: Vec<String> = Vec::new();
    if let Some(q) = st.filter_quest {
        filters.push(map.res.edid_or_full(q));
    }
    if let Some(k) = &st.filter_npc
        && let Some(s) = speakers.iter().find(|s| s.0 == *k)
    {
        filters.push(s.1.clone());
    }
    if !st.search_applied.trim().is_empty() {
        filters.push(i18n.t_arg("text-quoted", "text", st.search_applied.trim()));
    }
    if st.unvoiced_only {
        filters.push(i18n.t("dialog-unvoiced-only"));
    }
    let status = i18n.t_args(
        "dialog-status",
        &[
            ("lines", &total.to_string()),
            ("done", &done.to_string()),
            ("unvoiced", &unvoiced.to_string()),
            (
                "npcs",
                &speakers
                    .iter()
                    .filter(|s| !s.0.starts_with("unknown"))
                    .count()
                    .to_string(),
            ),
            (
                "filter",
                &if filters.is_empty() {
                    i18n.t("dialog-filter-none")
                } else {
                    filters.join(" · ")
                },
            ),
        ],
    );
    egui::TopBottomPanel::bottom("ximod-dialog-status").show_inside(ui, |ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            // The voices (C-13): rescan, and the list for the mod's author,
            // laid out first so the status text is what gets cut.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        unvoiced > 0,
                        egui::Button::new(i18n.t("dialog-copy-unvoiced")).small(),
                    )
                    .clicked()
                {
                    action = DialogAction::CopyUnvoiced;
                }
                if ui
                    .small_button(i18n.t("dialog-refresh"))
                    .on_hover_text(i18n.t("dialog-refresh-tip"))
                    .clicked()
                {
                    action = DialogAction::RefreshVoices;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // One line, cut with an ellipsis rather than overflowing
                    // the buttons; the whole text on hover.
                    let mut full = status.clone();
                    if let Some((m, p)) = &feed.master_progress {
                        full.push_str(" — ");
                        full.push_str(&i18n.t_args(
                            "dialog-master-indexing",
                            &[("master", m), ("pct", &((p * 100.0) as u32).to_string())],
                        ));
                    }
                    let message = st.message.clone();
                    if let Some(m) = &message {
                        full.push_str(" — ");
                        full.push_str(m);
                    }
                    let text = if message.is_some() {
                        egui::RichText::new(&full)
                            .small()
                            .color(ui.visuals().warn_fg_color)
                    } else {
                        egui::RichText::new(&full).small()
                    };
                    ui.add(egui::Label::new(text).truncate())
                        .on_hover_text(&full);
                });
            });
        });
        ui.add_space(2.0);
    });
    egui::TopBottomPanel::top("ximod-dialog-filters").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Quests.
            let quests: Vec<(u32, String)> = map
                .roots
                .iter()
                .filter_map(|r| match map.nodes[*r].kind {
                    NodeKind::Quest(q) => Some((q, map.nodes[*r].label.clone())),
                    _ => None,
                })
                .collect();
            let q_label = st
                .filter_quest
                .and_then(|q| quests.iter().find(|x| x.0 == q).map(|x| x.1.clone()))
                .unwrap_or_else(|| i18n.t("dialog-quest-all"));
            egui::ComboBox::from_id_salt("ximod-dialog-quest")
                .selected_text(crate::ui::edit_window::one_line(&q_label, 28))
                .width(190.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(st.filter_quest.is_none(), i18n.t("dialog-quest-all"))
                        .clicked()
                    {
                        st.filter_quest = None;
                    }
                    for (q, l) in &quests {
                        if ui
                            .selectable_label(st.filter_quest == Some(*q), l)
                            .clicked()
                        {
                            st.filter_quest = Some(*q);
                        }
                    }
                });
            // Speakers.
            let n_label = st
                .filter_npc
                .as_ref()
                .and_then(|k| speakers.iter().find(|s| s.0 == *k).map(|s| s.1.clone()))
                .unwrap_or_else(|| i18n.t("dialog-npc-all"));
            egui::ComboBox::from_id_salt("ximod-dialog-npc")
                .selected_text(crate::ui::edit_window::one_line(&n_label, 24))
                .width(170.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(st.filter_npc.is_none(), i18n.t("dialog-npc-all"))
                        .clicked()
                    {
                        st.filter_npc = None;
                    }
                    for (key, label, gender, n) in &speakers {
                        let g = gender_short(i18n, *gender);
                        let text = i18n.t_args(
                            "dialog-npc-item",
                            &[("name", &format!("{label}{g}")), ("n", &n.to_string())],
                        );
                        if ui
                            .selectable_label(st.filter_npc.as_deref() == Some(key), text)
                            .clicked()
                        {
                            st.filter_npc = Some(key.clone());
                        }
                    }
                });
            if let Some(k) = st.filter_npc.clone()
                && ui
                    .small_button(i18n.t("dialog-filter-grid"))
                    .on_hover_text(i18n.t("dialog-filter-grid-tip"))
                    .clicked()
            {
                action = DialogAction::FilterGrid(k);
            }
            // Search.
            let resp = ui.add(
                egui::TextEdit::singleline(&mut st.search)
                    .hint_text(i18n.t("dialog-search-hint"))
                    .desired_width(180.0),
            );
            if resp.changed() {
                st.search_typed_at = Some(ui.input(|i| i.time));
            }
            if !st.search.is_empty() && ui.small_button(icons::X).clicked() {
                st.search.clear();
                st.search_applied.clear();
                st.search_typed_at = None;
            }
            ui.checkbox(&mut st.unvoiced_only, i18n.t("dialog-unvoiced-only"));
            ui.checkbox(&mut st.follow_grid, i18n.t("dialog-follow"));
        });
        ui.add_space(4.0);
    });
    // The search applies a quarter of a second after the last keystroke.
    if let Some(at) = st.search_typed_at {
        let now = ui.input(|i| i.time);
        if now - at >= 0.25 {
            st.search_applied = st.search.clone();
            st.search_typed_at = None;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(60));
        }
    }

    // ---- the two panes
    let total_w = ui.available_width();
    let left_w = (total_w * st.split).clamp(220.0, (total_w - 260.0).max(220.0));
    egui::SidePanel::left("ximod-dialog-tree")
        .resizable(true)
        .default_width(left_w)
        .min_width(220.0)
        .show_inside(ui, |ui| {
            st.split = (ui.available_width() / total_w).clamp(0.2, 0.8);
            tree(ui, st, feed, &mut action);
        });
    egui::CentralPanel::default().show_inside(ui, |ui| {
        detail(ui, st, feed, &mut action);
    });
    action
}

fn gender_short(i18n: &I18n, g: crate::dialogue::Gender) -> String {
    match g {
        crate::dialogue::Gender::Male => format!(" ({})", i18n.t("gender-m-short")),
        crate::dialogue::Gender::Female => format!(" ({})", i18n.t("gender-f-short")),
        crate::dialogue::Gender::Unknown => String::new(),
    }
}

/// The tree pane.
fn tree(ui: &mut egui::Ui, st: &mut DialogWindow, feed: &Feed, action: &mut DialogAction) {
    let map = feed.map;
    let mut memo: HashMap<usize, bool> = HashMap::new();
    let filtering = st.filter_quest.is_some()
        || st.filter_npc.is_some()
        || !st.search_applied.trim().is_empty()
        || st.unvoiced_only;
    // Flatten the visible nodes.
    let mut rows: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = map.roots.iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        if filtering && !st.visible(map, feed.entries, n, &mut memo) {
            continue;
        }
        rows.push(n);
        let open = st.expanded.contains(&n)
            || (filtering && !matches!(map.nodes[n].kind, NodeKind::Info(_)));
        if open {
            for c in map.nodes[n].children.iter().rev() {
                stack.push(*c);
            }
        }
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let selected_node = st.selected.and_then(|s| map.info_node.get(&s).copied());
    let scroll_to = st.scroll_to_selected.then_some(selected_node).flatten();
    st.scroll_to_selected = false;
    let mut toggle: Option<usize> = None;
    let mut pick: Option<usize> = None;
    let mut open_line: Option<usize> = None;
    egui::ScrollArea::both()
        .id_salt("ximod-dialog-tree-scroll")
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            for ri in range {
                let n = rows[ri];
                let node = &map.nodes[n];
                let is_sel = selected_node == Some(n)
                    || match node.kind {
                        NodeKind::Response(f, _) => st.selected == Some(f) && false,
                        _ => false,
                    };
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width().max(400.0), row_h),
                    egui::Sense::click(),
                );
                if is_sel {
                    ui.painter().rect_filled(
                        rect,
                        2.0,
                        ui.visuals().selection.bg_fill.gamma_multiply(0.6),
                    );
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                if scroll_to == Some(n) {
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                let indent = 14.0 * node.depth as f32;
                let mut x = rect.left() + 4.0 + indent;
                let font = egui::TextStyle::Body.resolve(ui.style());
                let painter = ui.painter();
                // Expander.
                if !node.children.is_empty() {
                    let open = st.expanded.contains(&n) || filtering;
                    let glyph = if open {
                        icons::CARET_DOWN
                    } else {
                        icons::CARET_RIGHT
                    };
                    let g = painter.layout_no_wrap(
                        glyph.to_string(),
                        font.clone(),
                        ui.visuals().weak_text_color(),
                    );
                    let gr = egui::Rect::from_min_size(
                        egui::pos2(x, rect.center().y - g.size().y / 2.0),
                        g.size(),
                    );
                    painter.galley(gr.min, g, ui.visuals().weak_text_color());
                    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| p.x < x + 16.0)
                    {
                        toggle = Some(n);
                    }
                }
                x += 16.0;
                // Status square of a reply / response.
                if matches!(node.kind, NodeKind::Info(_) | NodeKind::Response(_, _)) {
                    let col = status_color(feed, node);
                    let sq = egui::Rect::from_min_size(
                        egui::pos2(x, rect.center().y - 6.0),
                        egui::vec2(10.0, 12.0),
                    );
                    painter.rect_filled(sq, 2.0, col);
                    painter.rect_stroke(
                        sq,
                        2.0,
                        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
                    );
                    x += 16.0;
                }
                let bold = node.depth == 0;
                let mut text = egui::RichText::new(&node.label);
                if bold {
                    text = text.strong();
                }
                let galley = painter.layout_no_wrap(
                    text.text().to_string(),
                    font.clone(),
                    ui.visuals().text_color(),
                );
                let extra_w = if node.extra.is_empty() {
                    0.0
                } else {
                    let eg = painter.layout_no_wrap(
                        node.extra.clone(),
                        egui::TextStyle::Small.resolve(ui.style()),
                        ui.visuals().weak_text_color(),
                    );
                    let ex = egui::pos2(
                        rect.right() - 6.0 - eg.size().x,
                        rect.center().y - eg.size().y / 2.0,
                    );
                    painter.galley(ex, eg.clone(), ui.visuals().weak_text_color());
                    eg.size().x + 10.0
                };
                let avail = (rect.right() - 6.0 - extra_w - x).max(40.0);
                let galley = if galley.size().x > avail {
                    painter.layout(
                        text.text().to_string(),
                        font.clone(),
                        ui.visuals().text_color(),
                        avail,
                    )
                } else {
                    galley
                };
                let clip = egui::Rect::from_min_max(
                    egui::pos2(x, rect.top()),
                    egui::pos2(x + avail, rect.bottom()),
                );
                let p2 = painter.with_clip_rect(clip);
                let ty = rect.center().y
                    - galley.rows.first().map(|r| r.height()).unwrap_or(row_h) / 2.0;
                let color = if bold {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().text_color()
                };
                p2.galley(egui::pos2(x, ty), galley, color);
                let _ = resp.on_hover_text(&node.label);
                // Clicks.
                let r = ui.interact(
                    rect,
                    ui.id().with(("ximod-dialog-row", n)),
                    egui::Sense::click(),
                );
                if r.double_clicked() {
                    if let Some(l) = node.line {
                        open_line = Some(l);
                    }
                } else if r.clicked() {
                    let on_expander = r
                        .interact_pointer_pos()
                        .is_some_and(|p| p.x < rect.left() + 4.0 + indent + 16.0)
                        && !node.children.is_empty();
                    if on_expander {
                        toggle = Some(n);
                    } else {
                        pick = Some(n);
                    }
                }
            }
        });
    if let Some(n) = toggle
        && !st.expanded.remove(&n)
    {
        st.expanded.insert(n);
    }
    if let Some(n) = pick {
        let node = &map.nodes[n];
        match node.kind {
            NodeKind::Info(f) | NodeKind::Response(f, _) => {
                st.selected = Some(f);
                if let Some(l) = node.line {
                    *action = DialogAction::SelectLine(l);
                }
            }
            _ => {
                if !st.expanded.remove(&n) {
                    st.expanded.insert(n);
                }
                if let Some(l) = node.line {
                    *action = DialogAction::SelectLine(l);
                }
            }
        }
    }
    if let Some(l) = open_line {
        *action = DialogAction::OpenLine(l);
    }
}

fn status_color(feed: &Feed, node: &crate::dialogue::Node) -> egui::Color32 {
    let Some(l) = node.line else {
        return egui::Color32::from_gray(200);
    };
    let Some(e) = feed.entries.get(l) else {
        return egui::Color32::from_gray(200);
    };
    let st = crate::status::derive(e);
    let (r, g, b) = feed.colors[st.index()];
    // A reply with several responses of different statuses: grey.
    if node.total > 1 && node.done != 0 && node.done != node.total {
        return egui::Color32::from_gray(190);
    }
    egui::Color32::from_rgb(r, g, b)
}

/// The detail pane.
fn detail(ui: &mut egui::Ui, st: &mut DialogWindow, feed: &Feed, action: &mut DialogAction) {
    let map = feed.map;
    let i18n = feed.i18n;
    let Some(sel) = st.selected else {
        ui.label(egui::RichText::new(i18n.t("dialog-pick")).weak());
        return;
    };
    let Some(info) = map.res.ctx.infos.get(&sel) else {
        return;
    };
    let (quest, topic) = map.where_of(sel, i18n);
    let topic_edid = map
        .res
        .ctx
        .topics
        .get(&info.topic)
        .map(|t| t.edid.clone())
        .unwrap_or_default();
    let n_in_topic = map
        .res
        .ctx
        .topics
        .get(&info.topic)
        .map(|t| {
            (
                t.infos
                    .iter()
                    .position(|f| *f == sel)
                    .map(|p| p + 1)
                    .unwrap_or(0),
                t.infos.len(),
            )
        })
        .unwrap_or((0, 0));
    // Bottom buttons.
    egui::TopBottomPanel::bottom("ximod-dialog-detail-bottom").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(i18n.t("dialog-nav-hint"))
                .weak()
                .small(),
        );
        ui.horizontal(|ui| {
            let prev = map.step(sel, -1);
            let next = map.step(sel, 1);
            if ui
                .add_enabled(
                    prev.is_some(),
                    egui::Button::new(format!("{} {}", icons::CARET_LEFT, i18n.t("dialog-prev"))),
                )
                .clicked()
            {
                let p = prev.unwrap();
                st.select_info(map, p);
                if let Some(l) = map.nodes[map.info_node[&p]].line {
                    *action = DialogAction::SelectLine(l);
                }
            }
            if ui
                .add_enabled(
                    next.is_some(),
                    egui::Button::new(format!("{} {}", i18n.t("dialog-next"), icons::CARET_RIGHT)),
                )
                .clicked()
            {
                let nx = next.unwrap();
                st.select_info(map, nx);
                if let Some(l) = map.nodes[map.info_node[&nx]].line {
                    *action = DialogAction::SelectLine(l);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let line = map.info_node.get(&sel).and_then(|n| map.nodes[*n].line);
                if ui
                    .add_enabled(
                        line.is_some(),
                        egui::Button::new(egui::RichText::new(i18n.t("dialog-open")).strong()),
                    )
                    .clicked()
                {
                    *action = DialogAction::OpenLine(line.unwrap());
                }
            });
        });
        ui.add_space(2.0);
    });
    egui::ScrollArea::vertical()
        .id_salt("ximod-dialog-detail")
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(i18n.t_args(
                    "dialog-info-head",
                    &[
                        ("formid", &format!("{sel:08X}")),
                        ("topic", &topic_edid),
                        ("n", &n_in_topic.0.to_string()),
                        ("total", &n_in_topic.1.to_string()),
                    ],
                ))
                .weak()
                .small(),
            );
            // Speaker.
            let speakers = map.speakers.get(&sel).cloned().unwrap_or_default();
            let sp_text = speaker_line(i18n, &speakers, map);
            ui.label(egui::RichText::new(sp_text).strong().size(15.0));
            let target = speakers
                .first()
                .filter(|s| s.origin == crate::dialogue::SpeakerOrigin::Scene)
                .map(|_| i18n.t("dialog-target-scene"))
                .unwrap_or_else(|| i18n.t("dialog-target-player"));
            ui.label(
                egui::RichText::new(i18n.t_args(
                    "dialog-quest-line",
                    &[("quest", &quest), ("topic", &topic), ("target", &target)],
                ))
                .weak(),
            );
            ui.add_space(6.0);
            // The player's prompt: RNAM or the topic text.
            let prompt_line = map
                .line("INFO", "RNAM", sel, 1)
                .or_else(|| map.line("DIAL", "FULL", info.topic, 1));
            if let Some(l) = prompt_line
                && let Some(e) = feed.entries.get(l)
            {
                ui.label(
                    egui::RichText::new(i18n.t("dialog-player"))
                        .strong()
                        .small(),
                );
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = if e.trans.is_empty() {
                        e.source.clone()
                    } else {
                        format!("{}  {}  {}", e.source, icons::ARROW_RIGHT, e.trans)
                    };
                    let r = ui.add(egui::Label::new(text).wrap().sense(egui::Sense::click()));
                    if r.double_clicked() {
                        *action = DialogAction::OpenLine(l);
                    } else if r.clicked() {
                        *action = DialogAction::SelectLine(l);
                    }
                });
            }
            // Responses.
            ui.label(
                egui::RichText::new(i18n.t("dialog-responses"))
                    .strong()
                    .small(),
            );
            if let Some(sh) = info.shared {
                ui.label(
                    egui::RichText::new(i18n.t_arg(
                        "dialog-shared",
                        "formid",
                        &format!("{sh:08X}"),
                    ))
                    .weak()
                    .small(),
                );
            }
            for (pos, r) in info.responses.iter().enumerate() {
                let line = r
                    .line_index
                    .and_then(|ix| map.line("INFO", "NAM1", sel, ix));
                let emotion = map.emotion_label(i18n, r.emotion, r.emotion_value);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        let head = if emotion.is_empty() {
                            format!("{}", r.number)
                        } else {
                            i18n.t_args(
                                "dialog-response",
                                &[("n", &r.number.to_string()), ("emotion", &emotion)],
                            )
                        };
                        ui.label(egui::RichText::new(head).strong());
                        if let Some(v) = feed.voices.as_ref().and_then(|v| v.get(pos)) {
                            let col = if v.ok {
                                crate::ui::theme::Palette::of(ui.visuals()).success
                            } else {
                                ui.visuals().error_fg_color
                            };
                            let r = ui.label(egui::RichText::new(&v.text).small().color(col));
                            if !v.tip.is_empty() {
                                r.on_hover_text(&v.tip);
                            }
                        }
                    });
                    match line.and_then(|l| feed.entries.get(l).map(|e| (l, e))) {
                        Some((l, e)) => {
                            let r1 = ui.add(
                                egui::Label::new(&e.source)
                                    .wrap()
                                    .sense(egui::Sense::click()),
                            );
                            let tr = if e.trans.is_empty() {
                                egui::RichText::new(format!(
                                    "{} ({})",
                                    icons::ARROW_RIGHT,
                                    i18n.t("dialog-untranslated")
                                ))
                                .color(ui.visuals().warn_fg_color)
                            } else {
                                egui::RichText::new(format!("{} {}", icons::ARROW_RIGHT, e.trans))
                            };
                            let r2 =
                                ui.add(egui::Label::new(tr).wrap().sense(egui::Sense::click()));
                            if r1.double_clicked() || r2.double_clicked() {
                                *action = DialogAction::OpenLine(l);
                            } else if r1.clicked() || r2.clicked() {
                                *action = DialogAction::SelectLine(l);
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new(&r.text).weak());
                        }
                    }
                    if !r.note.is_empty() {
                        ui.label(
                            egui::RichText::new(i18n.colon("NAM2", &r.note))
                                .weak()
                                .small(),
                        );
                    }
                });
            }
            // Conditions.
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(i18n.t("dialog-conditions"))
                    .strong()
                    .small(),
            );
            if feed.conditions.is_empty() {
                ui.label(egui::RichText::new(i18n.t("dialog-no-conditions")).weak());
            } else {
                for (text, raw) in &feed.conditions {
                    ui.add(egui::Label::new(text).wrap()).on_hover_text(raw);
                }
            }
            // Voice.
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(i18n.t("dialog-voice")).strong().small());
                if let Some(n) = &feed.voices_note {
                    ui.label(egui::RichText::new(n).weak().small());
                }
            });
            match &feed.voices {
                Some(v) if !v.is_empty() => {
                    for line in v {
                        let col = if line.ok {
                            crate::ui::theme::Palette::of(ui.visuals()).success
                        } else {
                            ui.visuals().error_fg_color
                        };
                        ui.horizontal(|ui| {
                            let r = ui.label(egui::RichText::new(&line.text).color(col));
                            if !line.tip.is_empty() {
                                r.on_hover_text(&line.tip);
                            }
                            if let Some(folder) = &line.folder
                                && ui.small_button(i18n.t("dialog-open-folder")).clicked()
                            {
                                *action = DialogAction::OpenFolder(folder.clone());
                            }
                        });
                    }
                }
                _ => {
                    ui.label(
                        egui::RichText::new(i18n.t("voice-not-mapped"))
                            .weak()
                            .small(),
                    );
                }
            }
        });
}

/// « Locuteur : Ralof (homme, MaleNord) — ANAM ».
pub fn speaker_line(
    i18n: &I18n,
    speakers: &[crate::dialogue::Speaker],
    map: &DialogueMap,
) -> String {
    let _ = map;
    let mut parts: Vec<String> = Vec::new();
    for s in speakers {
        let name = match &s.kind {
            SpeakerKind::Unknown => i18n.t("speaker-unknown"),
            SpeakerKind::Voice(_) => i18n.t_arg("speaker-voice", "name", &s.name),
            SpeakerKind::Alias { .. } => i18n.t_arg("speaker-alias", "name", &s.name),
            SpeakerKind::Faction(_) => i18n.t_arg("speaker-faction", "name", &s.name),
            SpeakerKind::Race(_) => i18n.t_arg("speaker-race", "name", &s.name),
            SpeakerKind::Npc(_) => s.name.clone(),
        };
        let gender = match s.gender {
            crate::dialogue::Gender::Male => i18n.t("gender-m"),
            crate::dialogue::Gender::Female => i18n.t("gender-f"),
            crate::dialogue::Gender::Unknown => i18n.t("gender-unknown"),
        };
        let voice = s
            .voice
            .as_ref()
            .map(|v| format!(", {v}"))
            .unwrap_or_default();
        let origin = i18n.t(match s.origin {
            crate::dialogue::SpeakerOrigin::Anam => "speaker-origin-anam",
            crate::dialogue::SpeakerOrigin::Scene => "speaker-origin-scene",
            crate::dialogue::SpeakerOrigin::Condition => "speaker-origin-condition",
            crate::dialogue::SpeakerOrigin::None => "speaker-origin-none",
        });
        if s.kind == SpeakerKind::Unknown {
            parts.push(i18n.dash(
                &i18n.t_args("speaker-line-unknown", &[("name", &name)]),
                &origin,
            ));
        } else {
            parts.push(i18n.t_args(
                "speaker-line",
                &[
                    ("name", &name),
                    ("gender", &gender),
                    ("voice", &voice),
                    ("origin", &origin),
                ],
            ));
        }
    }
    parts.join(&format!("  {}  ", i18n.t("cond-or")))
}
