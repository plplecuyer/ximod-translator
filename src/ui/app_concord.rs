//! Outils ▸ Concordance… (A-20): a term looked for in the active
//! dictionaries of the file, in a background thread; the pairs found, and
//! the chosen translation copied or put in the line being edited.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use eframe::egui;

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::concord::{self, Found, Options};

/// What the thread sends back: the generation of the search and its
/// result (`None` when cancelled).
type Reply = (u64, Option<Found>);

/// The window while it is open.
pub(crate) struct ConcordWindow {
    pub open: bool,
    pub needs_place: bool,
    pub term: String,
    pub opts: Options,
    /// Put the focus in the term field (opening, Ctrl+K).
    pub focus_term: bool,
    /// The last change of the term or of an option (a search follows
    /// 150 ms later).
    pub changed_at: Option<f64>,
    /// What the last search was launched for: term, options, document.
    launched: Option<(String, Options, u64)>,
    /// Generation of the last search; older replies are dropped.
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    channel: Option<(mpsc::Sender<Reply>, mpsc::Receiver<Reply>)>,
    pub searching: bool,
    /// The term has fewer than two characters.
    pub too_short: bool,
    /// The last complete result and the matcher of its term (highlight).
    pub found: Option<Found>,
    matcher: Option<regex::Regex>,
    /// The dictionaries searched, by name, for the scope line.
    pub scope: Vec<String>,
    /// The pair chosen (index in `found.pairs`).
    pub picked: Option<usize>,
    pub message: Option<String>,
}

impl Default for ConcordWindow {
    fn default() -> Self {
        ConcordWindow {
            open: false,
            needs_place: false,
            term: String::new(),
            opts: Options {
                in_trans: true,
                ..Default::default()
            },
            focus_term: false,
            changed_at: None,
            launched: None,
            generation: 0,
            cancel: None,
            channel: None,
            searching: false,
            too_short: false,
            found: None,
            matcher: None,
            scope: Vec::new(),
            picked: None,
            message: None,
        }
    }
}

impl ConcordWindow {
    /// Stop the search running, if any.
    fn cancel_running(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.searching = false;
    }
}

/// The decimal separator of the interface language, for the seconds.
fn secs_text(locale: &str, secs: f32) -> String {
    let s = format!("{secs:.2}");
    if matches!(
        locale,
        "eng" | "gle" | "mlt" | "jpn" | "kor" | "zho" | "roh"
    ) {
        s
    } else {
        s.replace('.', ",")
    }
}

impl XimodApp {
    /// Outils ▸ Concordance… / Ctrl+K: open with the term of the moment,
    /// or replace the term of the open window (R-A20-05). `line`: the line
    /// of the row menu, otherwise the line of the edit window or the
    /// grid's current line.
    pub(crate) fn open_concord_window(&mut self, ctx: &egui::Context, line: Option<usize>) {
        if self.docs.is_empty() {
            return;
        }
        let term = self.concord_initial_term(ctx, line);
        let w = &mut self.concord_win;
        w.needs_place = !w.open;
        w.open = true;
        w.focus_term = true;
        if !term.is_empty() {
            w.term = term;
        }
        w.picked = None;
        w.message = None;
        // At once: no wait for a term given.
        w.changed_at = None;
        w.launched = None;
    }

    /// The term at the launch (R-A20 « Interface », 1): the text selected
    /// in the Source or the Translation of the edit window, otherwise the
    /// word under the cursor of the area that has the focus, otherwise the
    /// whole source of the current line.
    fn concord_initial_term(&self, ctx: &egui::Context, line: Option<usize>) -> String {
        let Some(doc) = self.docs.get(self.active) else {
            return String::new();
        };
        let edit_line = self.edit_win.as_ref().map(|w| w.line);
        let line = line.or(edit_line).or(self.selection.cursor);
        // The areas of the edit window count when it shows that line.
        let in_edit = (self.edit_win.is_some() || self.edit_win_active_frame)
            && (edit_line.is_none() || edit_line == line);
        let Some(e) = line.and_then(|l| doc.tm.entries.get(l).map(|e| (l, e))) else {
            return String::new();
        };
        let (line, e) = e;
        if in_edit {
            let areas = [
                (
                    egui::Id::new(("ximod-edit-src-te", line)),
                    e.source.as_str(),
                ),
                (egui::Id::new(("ximod-edit-trans", line)), e.trans.as_str()),
            ];
            let range_of = |id: egui::Id, text: &str| {
                let n = text.chars().count();
                egui::TextEdit::load_state(ctx, id)
                    .and_then(|st| st.cursor.char_range())
                    .map(|r| {
                        let (a, b) = (r.primary.index, r.secondary.index);
                        (a.min(b).min(n), a.max(b).min(n))
                    })
            };
            // A selection, in the Source first.
            for (id, text) in areas {
                if let Some((a, b)) = range_of(id, text)
                    && a < b
                {
                    let piece: String = text.chars().skip(a).take(b - a).collect();
                    if !piece.trim().is_empty() {
                        return piece.trim().to_string();
                    }
                }
            }
            // The word under the cursor of the area focused last.
            for (id, text) in areas {
                if self.edit_last_text == Some(id)
                    && let Some((c, _)) = range_of(id, text)
                    && let Some(w) = concord::word_at(text, c)
                {
                    return w;
                }
            }
        }
        e.source.trim().to_string()
    }

    /// The window, each frame while it is open.
    pub(crate) fn concord_window(&mut self, ctx: &egui::Context) {
        if !self.concord_win.open {
            return;
        }
        if self.docs.is_empty() {
            self.concord_win.cancel_running();
            self.concord_win.open = false;
            return;
        }
        self.concord_poll();
        let id = egui::ViewportId::from_hash_of("ximod-concord");
        let geo = self.config.concord_win;
        let just_opened = self.concord_win.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(640.0), geo[3].max(380.0)]))
        } else {
            None
        };
        let term = self.concord_win.term.trim().to_string();
        let title = if term.is_empty() {
            self.i18n.t("concord-title-none")
        } else {
            self.i18n.t_arg("concord-title", "term", &term)
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([640.0, 380.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut close = false;
        let refocus = self.concord_win.focus_term && !just_opened;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            } else if refocus {
                // Ctrl+K on the open window: to the front.
                vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.concord_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::bottom("ximod-concord-hint").show(vctx, |ui| {
                let key = super::shortcut_text(vctx, &super::MENU_SHORTCUTS.concord, &self.i18n);
                ui.label(
                    egui::RichText::new(self.i18n.t_arg("concord-hint", "key", &key))
                        .weak()
                        .small(),
                );
            });
            egui::CentralPanel::default().show(vctx, |ui| {
                self.concord_body(ui);
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.concord_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.concord_win.cancel_running();
            self.concord_win.open = false;
            let _ = self.config.save();
        }
    }

    /// A reply of the thread, if one came.
    fn concord_poll(&mut self) {
        let Some((_, rx)) = &self.concord_win.channel else {
            return;
        };
        let mut last = None;
        while let Ok(r) = rx.try_recv() {
            last = Some(r);
        }
        let Some((generation, found)) = last else {
            return;
        };
        if generation != self.concord_win.generation {
            return;
        }
        self.concord_win.searching = false;
        self.concord_win.cancel = None;
        if let Some(f) = found {
            tracing::info!(
                "concordance: {} pairs, {} rows scanned in {:.3} s{}",
                f.pairs.len(),
                f.scanned,
                f.secs,
                if f.limited { " (limit reached)" } else { "" }
            );
            self.concord_win.found = Some(f);
            self.concord_win.picked = None;
        }
    }

    /// Launch the search of the term in a thread (R-A20-03); the one
    /// running is cancelled.
    fn concord_launch(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let (path, settings, serial) = (doc.path.clone(), doc.settings.clone(), doc.serial);
        let term = self.concord_win.term.trim().to_string();
        let opts = self.concord_win.opts;
        self.concord_win.launched = Some((term.clone(), opts, serial));
        self.concord_win.cancel_running();
        self.concord_win.generation += 1;
        let Some(re) = concord::matcher(&term, opts.whole_word) else {
            self.concord_win.too_short = true;
            self.concord_win.found = None;
            self.concord_win.matcher = None;
            self.concord_win.picked = None;
            return;
        };
        self.concord_win.too_short = false;
        let levels: Vec<concord::Level> = match self.resolve_stack(&path, &settings) {
            Some(stack) => stack
                .levels
                .iter()
                .map(|l| concord::Level {
                    name: crate::dict_stack::stem_of(&l.name()),
                    role: l.role,
                    db: l.db.clone(),
                })
                .collect(),
            None => Vec::new(),
        };
        self.concord_win.scope = levels
            .iter()
            .filter(|l| !opts.mod_only || l.role == crate::dicts::Role::Mod)
            .map(|l| l.name.clone())
            .collect();
        self.concord_win.matcher = Some(re.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        self.concord_win.cancel = Some(cancel.clone());
        self.concord_win.searching = true;
        let tx = self
            .concord_win
            .channel
            .get_or_insert_with(mpsc::channel)
            .0
            .clone();
        let generation = self.concord_win.generation;
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("ximod-concord".into())
            .spawn(move || {
                let found = concord::search(&levels, &re, opts, &cancel);
                let _ = tx.send((generation, found));
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            tracing::warn!("concordance: thread not started: {e}");
            self.concord_win.searching = false;
        }
    }

    /// Body of the window.
    fn concord_body(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        let mut changed = false;
        let mut relaunch = false;
        // ---- 1. The term and 2. the options.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.i18n.t("concord-term")).strong());
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.concord_win.term)
                    .id(egui::Id::new("ximod-concord-term"))
                    .desired_width((ui.available_width() - 8.0).clamp(160.0, 460.0)),
            );
            if std::mem::take(&mut self.concord_win.focus_term) {
                r.request_focus();
                // The whole term selected: typing replaces it.
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    let n = self.concord_win.term.chars().count();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(n),
                    )));
                    st.store(ui.ctx(), r.id);
                }
            }
            if r.changed() {
                changed = true;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                relaunch = true;
                r.request_focus();
            }
        });
        // The options on their own row: long labels in some languages.
        ui.horizontal_wrapped(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let o = &mut self.concord_win.opts;
            changed |= ui
                .checkbox(&mut o.whole_word, self.i18n.t("concord-whole-word"))
                .changed();
            changed |= ui
                .checkbox(&mut o.in_trans, self.i18n.t("concord-in-trans"))
                .changed();
            changed |= ui
                .checkbox(&mut o.mod_only, self.i18n.t("concord-mod-only"))
                .changed();
        });
        if changed {
            self.concord_win.changed_at = Some(now);
        }
        // A search 150 ms after the last change, at once on Entrée.
        let serial = self.docs.get(self.active).map(|d| d.serial).unwrap_or(0);
        let wanted = (
            self.concord_win.term.trim().to_string(),
            self.concord_win.opts,
            serial,
        );
        let due = match self.concord_win.changed_at {
            Some(at) if now - at < 0.15 => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(60));
                false
            }
            _ => self.concord_win.launched.as_ref() != Some(&wanted),
        };
        if relaunch || due {
            self.concord_win.changed_at = None;
            self.concord_launch(ui.ctx());
        }
        // The scope line: the dictionaries, the rows read and the time,
        // once a search has run.
        let w = &self.concord_win;
        let scope = if w.too_short {
            None
        } else if w.scope.is_empty() && w.launched.is_some() {
            Some(self.i18n.t("concord-no-dicts"))
        } else {
            w.found.as_ref().map(|f| {
                self.i18n.t_args(
                    "concord-scope",
                    &[
                        ("list", &w.scope.join(", ")),
                        (
                            "lines",
                            &crate::ui::export_dialog::fmt_count(&self.i18n, f.scanned),
                        ),
                        ("secs", &secs_text(self.i18n.current_locale(), f.secs)),
                    ],
                )
            })
        };
        if let Some(scope) = scope {
            ui.label(egui::RichText::new(scope).weak().small());
        }
        // The result line.
        let head = if w.too_short {
            self.i18n.t("concord-too-short")
        } else if w.searching {
            self.i18n.t("concord-searching")
        } else {
            match &w.found {
                Some(f) if f.pairs.is_empty() => self.i18n.t_arg("concord-none", "term", &wanted.0),
                Some(f) if f.limited => self.i18n.t_args(
                    "concord-results-limit",
                    &[
                        ("n", &f.pairs.len().to_string()),
                        ("max", &concord::MAX_PAIRS.to_string()),
                    ],
                ),
                Some(f) => self
                    .i18n
                    .t_arg("concord-results", "n", &f.pairs.len().to_string()),
                None => String::new(),
            }
        };
        ui.horizontal(|ui| {
            if w.searching {
                ui.spinner();
            }
            ui.label(egui::RichText::new(head).strong());
        });
        // ---- 4. The pair chosen, at the bottom.
        let pair = self
            .concord_win
            .found
            .as_ref()
            .zip(self.concord_win.picked)
            .and_then(|(f, i)| f.pairs.get(i).cloned());
        let can_insert = self.edit_win.is_some();
        let mut copy: Option<String> = None;
        let mut insert: Option<String> = None;
        egui::TopBottomPanel::bottom("ximod-concord-pair")
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(self.i18n.t("concord-pair")).strong());
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &pair {
                        Some(p) => {
                            ui.add(egui::Label::new(&p.source).wrap());
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&p.trans)
                                        .color(ui.visuals().hyperlink_color),
                                )
                                .wrap(),
                            );
                        }
                        None => {
                            ui.label(egui::RichText::new("—").weak());
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if let Some(m) = &self.concord_win.message {
                        ui.label(egui::RichText::new(m).weak());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add_enabled(
                            pair.is_some() && can_insert,
                            egui::Button::new(
                                egui::RichText::new(self.i18n.t("concord-insert")).strong(),
                            ),
                        );
                        let r = if pair.is_some() && !can_insert {
                            r.on_disabled_hover_text(self.i18n.t("concord-insert-tip"))
                        } else {
                            r
                        };
                        if r.clicked()
                            && let Some(p) = &pair
                        {
                            insert = Some(p.trans.clone());
                        }
                        if ui
                            .add_enabled(
                                pair.is_some(),
                                egui::Button::new(self.i18n.t("concord-copy")),
                            )
                            .clicked()
                            && let Some(p) = &pair
                        {
                            copy = Some(p.trans.clone());
                        }
                    });
                });
            });
        // ---- 3. The pairs.
        let picked = self.concord_win.picked;
        let mut clicked: Option<usize> = None;
        let mut double: Option<usize> = None;
        if let Some(f) = &self.concord_win.found
            && !self.concord_win.too_short
        {
            let headers = [
                self.i18n.t("concord-col-source"),
                self.i18n.t("concord-col-trans"),
                self.i18n.t("concord-col-dict"),
                self.i18n.t("concord-col-record"),
                self.i18n.t("concord-col-count"),
            ];
            let text_only = self.i18n.t("concord-record-text");
            let re = self.concord_win.matcher.clone();
            let mark = ui.visuals().warn_fg_color.gamma_multiply(0.35);
            let font = egui::TextStyle::Body.resolve(ui.style());
            let color = ui.visuals().text_color();
            let lit = |text: &str| -> egui::text::LayoutJob {
                let ranges = re
                    .as_ref()
                    .map(|r| concord::hits(r, text))
                    .unwrap_or_default();
                let plain = egui::TextFormat::simple(font.clone(), color);
                let hit = egui::TextFormat {
                    background: mark,
                    ..plain.clone()
                };
                let mut job = egui::text::LayoutJob::default();
                let mut at = 0;
                for (a, b) in ranges {
                    if a > at {
                        job.append(&text[at..a], 0.0, plain.clone());
                    }
                    job.append(&text[a..b], 0.0, hit.clone());
                    at = b;
                }
                if at < text.len() {
                    job.append(&text[at..], 0.0, plain);
                }
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job
            };
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
            egui_extras::TableBuilder::new(ui)
                .id_salt("ximod-concord-table")
                .striped(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(egui_extras::Column::remainder().at_least(120.0).clip(true))
                .column(egui_extras::Column::remainder().at_least(120.0).clip(true))
                .column(
                    egui_extras::Column::initial(150.0)
                        .at_least(60.0)
                        .clip(true),
                )
                .column(egui_extras::Column::initial(84.0).at_least(50.0).clip(true))
                .column(egui_extras::Column::exact(36.0))
                .max_scroll_height(f32::INFINITY)
                .auto_shrink([false, false])
                .header(row_h, |mut h| {
                    for t in &headers {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, f.pairs.len(), |mut row| {
                        let i = row.index();
                        let p = &f.pairs[i];
                        row.set_selected(picked == Some(i));
                        row.col(|ui| {
                            ui.add(egui::Label::new(lit(&p.source)).selectable(false))
                                .on_hover_text(&p.source);
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(lit(&p.trans)).selectable(false))
                                .on_hover_text(&p.trans);
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(&p.dict).truncate().selectable(false));
                        });
                        row.col(|ui| {
                            let rec = if p.record.is_empty() {
                                egui::RichText::new(&text_only).weak()
                            } else {
                                egui::RichText::new(&p.record).monospace()
                            };
                            ui.add(egui::Label::new(rec).truncate().selectable(false));
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(p.count.to_string()).selectable(false));
                        });
                        let r = row.response();
                        if r.clicked() {
                            clicked = Some(i);
                        }
                        if r.double_clicked() {
                            double = Some(i);
                        }
                    });
                });
        }
        if let Some(i) = clicked.or(double) {
            self.concord_win.picked = Some(i);
            self.concord_win.message = None;
        }
        if let Some(i) = double
            && let Some(p) = self.concord_win.found.as_ref().and_then(|f| f.pairs.get(i))
        {
            copy = Some(p.trans.clone());
        }
        if let Some(t) = copy {
            ui.ctx().copy_text(t);
            self.concord_win.message = Some(self.i18n.t("concord-copied"));
        }
        if let Some(t) = insert {
            self.concord_insert(ui.ctx(), t);
        }
    }

    /// Dans la ligne (R-A20-04, D-53): the translation of the pair in the
    /// Translation area of the edit window — in place of its selection,
    /// otherwise at its cursor, otherwise as the whole text of an empty
    /// line; one undoable operation, provenance « saisie ».
    fn concord_insert(&mut self, ctx: &egui::Context, piece: String) {
        let Some(line) = self.edit_win.as_ref().map(|w| w.line) else {
            return;
        };
        let Some(e) = self
            .docs
            .get(self.active)
            .and_then(|d| d.tm.entries.get(line))
        else {
            return;
        };
        if e.unused {
            return;
        }
        let trans = e.trans.clone();
        let (new_text, pos) = if trans.trim().is_empty() {
            let n = piece.chars().count();
            (piece, n)
        } else {
            let chars: Vec<char> = trans.chars().collect();
            let (a, b) = egui::TextEdit::load_state(ctx, egui::Id::new(("ximod-edit-trans", line)))
                .and_then(|st| st.cursor.char_range())
                .map(|r| {
                    let (a, b) = (r.primary.index, r.secondary.index);
                    (a.min(b).min(chars.len()), a.max(b).min(chars.len()))
                })
                .unwrap_or((chars.len(), chars.len()));
            concord::insert(&chars, a, b, &piece)
        };
        self.edit_set_trans(line, new_text, "concord-op", crate::provenance::TYPED);
        if let Some(w) = self.edit_win.as_mut() {
            w.cursor_at = Some(pos);
            w.focus_request = true;
            w.message = None;
        }
        self.concord_win.message = Some(self.i18n.t("concord-inserted"));
        self.status = self.i18n.t("concord-inserted");
    }
}
