//! Base pays / langues (E-11): Options › Propriétés…, the read-only explorer
//! of the reference tables shared with XIMOD Architect (`ui/properties.rs`
//! there) — a Pays tab and a Langues tab that answer each other, filters,
//! keyboard navigation, flags, fonts. It never changes anything.

use eframe::egui::{self, RichText};

use super::{XimodApp, read_geometry as read_geometry_of};
use crate::data::props::{self, SpokenIn};

/// The tabs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropTab {
    #[default]
    Countries,
    Languages,
}

/// Identity of a table (address and length): the reference tables are only
/// replaced wholesale, so a new identity means new data.
type SrcId = (usize, usize);

/// (filter, interface language, table) of the country list.
type CountryKey = (String, String, SrcId);

fn src_id<T>(v: &[T]) -> SrcId {
    (v.as_ptr() as usize, v.len())
}

/// The window's state.
#[derive(Default)]
pub(crate) struct PropsWindow {
    pub open: bool,
    pub needs_place: bool,
    pub tab: PropTab,
    pub country_filter: String,
    pub country_sel: Option<String>,
    pub country_cursor: usize,
    country_visible: usize,
    pub lang_filter: String,
    pub lang_sel: Option<String>,
    pub lang_cursor: usize,
    lang_visible: usize,
    /// The filter takes the focus (opening, change of tab).
    focus_filter: bool,
    /// The cursor was set from the other tab: scroll the list to it.
    reveal: u8,
    caches: Caches,
}

/// What the window would otherwise compute at every frame.
#[derive(Default)]
struct Caches {
    flags_dir: Option<Option<std::path::PathBuf>>,
    /// (filter, locale, table) → (alpha-3, name shown).
    countries: Option<(CountryKey, Vec<(String, String)>)>,
    /// (filter, table) → indices into the language table.
    languages: Option<((String, SrcId), Vec<usize>)>,
    /// (language, locale, table) → the countries where it is spoken.
    spoken_in: Option<((String, String, SrcId), Vec<SpokenIn>)>,
    country_row_h: f32,
    lang_row_h: f32,
}

/// A list of rows of which only those inside the viewport are built (6 777
/// languages). Returns the row clicked and the number of rows fully visible.
/// `scroll`: the cursor moved this frame (and how to align it).
fn virtual_list(
    ui: &mut egui::Ui,
    id_salt: &str,
    n: usize,
    cursor: usize,
    scroll: Option<Option<egui::Align>>,
    row_h: &mut f32,
    label: impl Fn(usize) -> String,
) -> (Option<usize>, usize) {
    let h = if *row_h > 0.0 {
        *row_h
    } else {
        let sp = ui.spacing();
        sp.interact_size
            .y
            .max(ui.text_style_height(&egui::TextStyle::Button) + 2.0 * sp.button_padding.y)
    };
    let pitch = h + ui.spacing().item_spacing.y;
    let mut clicked = None;
    let mut measured: Option<f32> = None;
    let out = egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show_rows(ui, h, n, |ui, range| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            if let Some(align) = scroll {
                let top = ui.max_rect().top() + (cursor as f32 - range.start as f32) * pitch;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(ui.max_rect().left(), top),
                    egui::vec2(ui.max_rect().width(), h),
                );
                ui.scroll_to_rect(rect, align);
            }
            for idx in range {
                let resp = ui.selectable_label(idx == cursor, label(idx));
                if measured.is_none() {
                    measured = Some(resp.rect.height());
                }
                if resp.clicked() {
                    clicked = Some(idx);
                }
            }
        });
    if let Some(m) = measured
        && (m - h).abs() > 0.01
    {
        *row_h = m;
        ui.ctx().request_repaint();
    }
    let visible = ((out.inner_rect.height() / pitch.max(1.0)).floor() as usize).max(1);
    (clicked, visible)
}

/// The keys of a list (arrows, Tab / Maj+Tab, Page up / down, Home, End,
/// Enter), consumed so the filter does not see them: (moved, Enter, how to
/// scroll the cursor into view).
fn list_keys(
    ctx: &egui::Context,
    cursor: &mut usize,
    n: usize,
    page: usize,
) -> (bool, bool, Option<egui::Align>) {
    if n == 0 {
        *cursor = 0;
        return (false, false, None);
    }
    if *cursor >= n {
        *cursor = n - 1;
    }
    let start = *cursor;
    let mut enter = false;
    let mut align = None;
    let page = page.max(1);
    ctx.input_mut(|i| {
        use egui::{Align, Key, Modifiers};
        let m = Modifiers::NONE;
        if i.consume_key(Modifiers::SHIFT, Key::Tab) || i.consume_key(m, Key::ArrowUp) {
            *cursor = cursor.saturating_sub(1);
        }
        if (i.consume_key(m, Key::Tab) || i.consume_key(m, Key::ArrowDown)) && *cursor + 1 < n {
            *cursor += 1;
        }
        if i.consume_key(m, Key::PageDown) {
            *cursor = (*cursor + page).min(n - 1);
            align = Some(Align::TOP);
        }
        if i.consume_key(m, Key::PageUp) {
            *cursor = cursor.saturating_sub(page);
            align = Some(Align::TOP);
        }
        if i.consume_key(m, Key::Home) {
            *cursor = 0;
            align = Some(Align::TOP);
        }
        if i.consume_key(m, Key::End) {
            *cursor = n - 1;
            align = Some(Align::BOTTOM);
        }
        if i.consume_key(m, Key::Enter) {
            enter = true;
        }
    });
    (*cursor != start, enter, align)
}

/// The list (a third of the width) and the details, side by side, in the
/// space left.
fn two_columns(ui: &mut egui::Ui) -> (egui::Rect, [egui::Ui; 2]) {
    let rect = ui.available_rect_before_wrap();
    let lw = (rect.width() * 0.34).clamp(200.0, 320.0);
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(lw, rect.height()));
    let right = egui::Rect::from_min_max(egui::pos2(rect.min.x + lw + 10.0, rect.min.y), rect.max);
    let cols = [
        ui.new_child(egui::UiBuilder::new().max_rect(left)),
        ui.new_child(egui::UiBuilder::new().max_rect(right)),
    ];
    (rect, cols)
}

/// The family of a font file (`Noto_Sans/static/NotoSans-Regular.ttf` →
/// `Noto Sans`).
fn font_family(rel: &str) -> String {
    rel.split(['/', '\\'])
        .next()
        .unwrap_or(rel)
        .replace('_', " ")
}

/// A flag of `assets/images/svg`, or an empty frame.
fn flag(ui: &mut egui::Ui, dir: Option<&std::path::Path>, file: Option<&str>, h: f32) {
    let size = egui::vec2(h * 1.5, h);
    match dir
        .zip(file)
        .map(|(d, f)| d.join(f))
        .filter(|p| p.is_file())
    {
        Some(p) => {
            ui.add(
                egui::Image::from_uri(format!("file://{}", p.display())).fit_to_exact_size(size),
            );
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_stroke(
                rect,
                1.0,
                egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
            );
        }
    }
}

impl XimodApp {
    /// Options › Propriétés…
    pub(crate) fn open_props(&mut self) {
        self.props_win.needs_place = true;
        self.props_win.focus_filter = true;
        self.props_win.open = true;
    }

    pub(crate) fn props_window(&mut self, ctx: &egui::Context) {
        if !self.props_win.open {
            if self.props_win.caches.flags_dir.is_some() {
                self.props_win.caches = Caches::default();
            }
            return;
        }
        let id = egui::ViewportId::from_hash_of("ximod-props");
        let [sx, sy, sw, sh] = self.config.props_win;
        let just_opened = self.props_win.needs_place;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [sw.max(600.0), sh.max(400.0)],
            )
        });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(self.i18n.t("prop-title"))
            .with_min_inner_size([600.0, 400.0]);
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
            self.props_win.needs_place = false;
            if vctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                close = true;
            }
            egui::TopBottomPanel::bottom("props_bottom").show(vctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.i18n.t("prop-keys-hint")).weak().small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(self.i18n.t("btn-close")).clicked() {
                            close = true;
                        }
                    });
                });
            });
            egui::CentralPanel::default().show(vctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, key) in [
                        (PropTab::Countries, "prop-tab-countries"),
                        (PropTab::Languages, "prop-tab-languages"),
                    ] {
                        if ui
                            .selectable_label(self.props_win.tab == tab, self.i18n.t(key))
                            .clicked()
                            && self.props_win.tab != tab
                        {
                            self.props_win.tab = tab;
                            self.props_win.focus_filter = true;
                        }
                    }
                });
                ui.separator();
                match self.props_win.tab {
                    PropTab::Countries => self.props_countries(ui, vctx),
                    PropTab::Languages => self.props_languages(ui, vctx),
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry_of(vctx) {
                self.config.props_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        if close {
            self.props_win.open = false;
            let _ = self.config.save();
        }
    }

    /// The filter line of a tab: the field, ✕, « n sur N ».
    fn props_filter(&mut self, ui: &mut egui::Ui, countries: bool, shown: usize, total: usize) {
        let l_filter = self.i18n.t("prop-filter");
        let l_clear = self.i18n.t("btn-clear");
        let count = self.i18n.t_args(
            "prop-filter-count",
            &[
                (
                    "shown",
                    &crate::ui::export_dialog::fmt_count(&self.i18n, shown),
                ),
                (
                    "total",
                    &crate::ui::export_dialog::fmt_count(&self.i18n, total),
                ),
            ],
        );
        let w = &mut self.props_win;
        let text = if countries {
            &mut w.country_filter
        } else {
            &mut w.lang_filter
        };
        ui.horizontal(|ui| {
            ui.label(&l_filter);
            let resp = ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
            if w.focus_filter {
                w.focus_filter = false;
                resp.request_focus();
            }
            if ui
                .small_button(crate::icons::X)
                .on_hover_text(&l_clear)
                .clicked()
            {
                text.clear();
                resp.request_focus();
            }
            ui.label(RichText::new(count).weak().small());
        });
        ui.add_space(4.0);
    }

    fn props_countries(&mut self, ui: &mut egui::Ui, vctx: &egui::Context) {
        let loc = self.i18n.current_locale().to_string();
        let src = src_id(&self.countries.countries);
        let key = (self.props_win.country_filter.clone(), loc.clone(), src);
        if self
            .props_win
            .caches
            .countries
            .as_ref()
            .is_none_or(|(k, _)| *k != key)
        {
            let rows = props::country_rows(&self.countries, &self.country_names, &loc, &key.0);
            // A new filter: the cursor on the country chosen, else the top.
            let w = &mut self.props_win;
            w.country_cursor = w
                .country_sel
                .as_ref()
                .and_then(|a3| rows.iter().position(|r| r.0 == *a3))
                .unwrap_or(0);
            w.reveal = 3;
            w.caches.countries = Some((key, rows));
        }
        let list = self
            .props_win
            .caches
            .countries
            .as_ref()
            .map(|c| c.1.clone())
            .unwrap_or_default();
        let total = self.countries.countries.len();
        self.props_filter(ui, true, list.len(), total);
        let mut goto_lang: Option<String> = None;
        let (rect, mut cols) = two_columns(ui);
        (|cols: &mut [egui::Ui; 2]| {
            let w = &mut self.props_win;
            let page = if w.country_visible > 0 {
                w.country_visible
            } else {
                15
            };
            let (moved, enter, align) = list_keys(vctx, &mut w.country_cursor, list.len(), page);
            if enter && let Some((a3, _)) = list.get(w.country_cursor) {
                w.country_sel = Some(a3.clone());
            }
            let (clicked, visible) = virtual_list(
                &mut cols[0],
                "ximod-props-countries",
                list.len(),
                w.country_cursor,
                if moved {
                    Some(align)
                } else {
                    (w.reveal > 0).then_some(Some(egui::Align::Center))
                },
                &mut w.caches.country_row_h,
                |i| format!("{}  ({})", list[i].1, list[i].0),
            );
            if let Some(i) = clicked {
                w.country_cursor = i;
                w.country_sel = Some(list[i].0.clone());
            }
            w.country_visible = visible;
            w.reveal = w.reveal.saturating_sub(1);
            let flags_dir = w
                .caches
                .flags_dir
                .get_or_insert_with(crate::data::flags_dir)
                .clone();
            let sel = w.country_sel.clone();
            let ui = &mut cols[1];
            let Some(c) = sel.as_deref().and_then(|a3| self.countries.by_a3(a3)) else {
                ui.label(RichText::new(self.i18n.t("prop-select-country")).weak());
                return;
            };
            egui::ScrollArea::both()
                .id_salt("ximod-props-country")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    flag(
                        ui,
                        flags_dir.as_deref(),
                        self.countries.flag_for(&c.a3),
                        64.0,
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(props::country_name(&self.country_names, c, &loc))
                            .strong()
                            .size(15.0),
                    );
                    ui.label(RichText::new(format!("{} — {}", c.name_en, c.a3)).weak());
                    ui.add_space(8.0);
                    ui.label(RichText::new(self.i18n.t("prop-official-langs")).strong());
                    let (yes, no) = (self.i18n.t("glossary-yes"), self.i18n.t("glossary-no"));
                    let l_none = self.i18n.t("prop-no-font");
                    egui::Grid::new("ximod-props-official")
                        .num_columns(5)
                        .striped(true)
                        .show(ui, |ui| {
                            for k in [
                                "prop-col-iso",
                                "prop-col-name",
                                "prop-endonym",
                                "prop-font",
                                "prop-col-interface",
                            ] {
                                ui.label(RichText::new(self.i18n.t(k)).strong());
                            }
                            ui.end_row();
                            for cl in &c.languages {
                                let code = match self.i18n.iso3_to_iso1(&cl.iso3) {
                                    Some(i1) => format!("{} / {i1}", cl.iso3),
                                    None => cl.iso3.clone(),
                                };
                                ui.label(RichText::new(code).monospace());
                                if ui.link(self.i18n.display_name(&cl.iso3)).clicked() {
                                    goto_lang = Some(cl.iso3.clone());
                                }
                                ui.label(&cl.country_endonym);
                                ui.label(
                                    RichText::new(
                                        self.i18n
                                            .font_for(&cl.iso3)
                                            .map(font_family)
                                            .unwrap_or_else(|| l_none.clone()),
                                    )
                                    .small(),
                                );
                                let has = self.i18n.available_locales().contains(&cl.iso3);
                                ui.label(if has { &yes } else { &no });
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                    let spoken = props::spoken_languages(&self.country_languages, c);
                    ui.label(
                        RichText::new(format!(
                            "{} ({})",
                            self.i18n.t("prop-spoken-langs"),
                            spoken.len()
                        ))
                        .strong(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for iso3 in spoken {
                            if ui
                                .link(format!("{} ({iso3})", self.i18n.display_name(iso3)))
                                .clicked()
                            {
                                goto_lang = Some(iso3.to_string());
                            }
                        }
                    });
                });
        })(&mut cols);
        ui.allocate_rect(rect, egui::Sense::hover());
        if let Some(iso3) = goto_lang {
            self.props_goto_lang(&iso3);
        }
    }

    fn props_languages(&mut self, ui: &mut egui::Ui, vctx: &egui::Context) {
        let loc = self.i18n.current_locale().to_string();
        let langs = self.i18n.languages();
        let src = src_id(&langs.languages);
        let key = (self.props_win.lang_filter.clone(), src);
        if self
            .props_win
            .caches
            .languages
            .as_ref()
            .is_none_or(|(k, _)| *k != key)
        {
            let rows = props::language_rows(langs, &key.0);
            // A new filter: the cursor on the language chosen, else the top.
            let w = &mut self.props_win;
            w.lang_cursor = w
                .lang_sel
                .as_ref()
                .and_then(|iso3| {
                    rows.iter()
                        .position(|i| langs.languages.get(*i).is_some_and(|e| e.iso3 == *iso3))
                })
                .unwrap_or(0);
            w.reveal = 3;
            w.caches.languages = Some((key, rows));
        }
        let list = self
            .props_win
            .caches
            .languages
            .as_ref()
            .map(|c| c.1.clone())
            .unwrap_or_default();
        let total = langs.languages.len();
        self.props_filter(ui, false, list.len(), total);
        let mut goto_country: Option<String> = None;
        let (rect, mut cols) = two_columns(ui);
        (|cols: &mut [egui::Ui; 2]| {
            let langs = self.i18n.languages();
            let w = &mut self.props_win;
            let page = if w.lang_visible > 0 {
                w.lang_visible
            } else {
                15
            };
            let (moved, enter, align) = list_keys(vctx, &mut w.lang_cursor, list.len(), page);
            let (clicked, visible) = virtual_list(
                &mut cols[0],
                "ximod-props-langs",
                list.len(),
                w.lang_cursor,
                if moved {
                    Some(align)
                } else {
                    (w.reveal > 0).then_some(Some(egui::Align::Center))
                },
                &mut w.caches.lang_row_h,
                |i| match langs.languages.get(list[i]) {
                    Some(e) => format!("{}  ({})", e.name, e.iso3),
                    None => String::new(),
                },
            );
            if let Some(i) = clicked {
                w.lang_cursor = i;
            }
            if (enter || clicked.is_some())
                && let Some(e) = list
                    .get(w.lang_cursor)
                    .and_then(|i| langs.languages.get(*i))
            {
                w.lang_sel = Some(e.iso3.clone());
            }
            w.lang_visible = visible;
            w.reveal = w.reveal.saturating_sub(1);
            let flags_dir = w
                .caches
                .flags_dir
                .get_or_insert_with(crate::data::flags_dir)
                .clone();
            let sel = w.lang_sel.clone();
            let ui = &mut cols[1];
            let Some(e) = sel
                .as_deref()
                .and_then(|iso3| langs.languages.iter().find(|e| e.iso3 == iso3))
            else {
                ui.label(RichText::new(self.i18n.t("prop-select-lang")).weak());
                return;
            };
            let ckey = (
                e.iso3.clone(),
                loc.clone(),
                src_id(&self.countries.countries),
            );
            if w.caches.spoken_in.as_ref().is_none_or(|(k, _)| *k != ckey) {
                let rows = props::spoken_in(
                    &e.iso3,
                    langs,
                    &self.countries,
                    &self.country_languages,
                    &self.country_names,
                    &loc,
                );
                w.caches.spoken_in = Some((ckey, rows));
            }
            let spoken = w
                .caches
                .spoken_in
                .as_ref()
                .map(|c| c.1.as_slice())
                .unwrap_or(&[]);
            let i18n = &self.i18n;
            egui::ScrollArea::both()
                .id_salt("ximod-props-lang")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.label(RichText::new(&e.name).strong().size(15.0));
                    egui::Grid::new("ximod-props-lang-grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label(i18n.t("prop-lang-endonym"));
                            ui.label(&e.name);
                            ui.end_row();
                            ui.label(i18n.t("prop-codes"));
                            let code = if e.iso1.is_empty() {
                                e.iso3.clone()
                            } else {
                                format!("{} / {}", e.iso3, e.iso1)
                            };
                            ui.label(RichText::new(code).monospace());
                            ui.end_row();
                            ui.label(i18n.t("prop-font-label"));
                            let font = if e.font.is_empty() {
                                i18n.t("prop-no-font")
                            } else {
                                font_family(&e.font)
                            };
                            let r = ui.label(font);
                            if !e.font.is_empty() {
                                r.on_hover_text(&e.font);
                            }
                            ui.end_row();
                            ui.label(i18n.t("prop-interface"));
                            let has = i18n.available_locales().contains(&e.iso3);
                            ui.label(if has {
                                i18n.t_arg(
                                    "prop-interface-yes",
                                    "path",
                                    &format!("assets/locales/{}", e.iso3),
                                )
                            } else {
                                i18n.t("glossary-no")
                            });
                            ui.end_row();
                        });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("{} ({})", i18n.t("prop-spoken-in"), spoken.len()))
                            .strong(),
                    );
                    let (yes, no) = (i18n.t("glossary-yes"), i18n.t("glossary-no"));
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    egui::Grid::new("ximod-props-spoken")
                        .num_columns(4)
                        .striped(true)
                        .max_col_width(240.0)
                        .show(ui, |ui| {
                            for k in [
                                "prop-col-flag",
                                "prop-col-country",
                                "prop-local-name",
                                "prop-col-official",
                            ] {
                                ui.label(RichText::new(i18n.t(k)).strong());
                            }
                            ui.end_row();
                            for s in spoken {
                                flag(
                                    ui,
                                    flags_dir.as_deref(),
                                    self.countries.flag_for(&s.a3),
                                    18.0,
                                );
                                if ui.link(&s.name).clicked() {
                                    goto_country = Some(s.a3.clone());
                                }
                                ui.label(&s.local);
                                ui.label(if s.official { &yes } else { &no });
                                ui.end_row();
                            }
                        });
                });
        })(&mut cols);
        ui.allocate_rect(rect, egui::Sense::hover());
        if let Some(a3) = goto_country {
            self.props_goto_country(&a3);
        }
    }

    /// The Langues tab on a language (from a country).
    fn props_goto_lang(&mut self, iso3: &str) {
        let w = &mut self.props_win;
        w.tab = PropTab::Languages;
        w.lang_filter.clear();
        w.lang_sel = Some(iso3.to_string());
        let rows = props::language_rows(self.i18n.languages(), "");
        w.lang_cursor = rows
            .iter()
            .position(|i| {
                self.i18n
                    .languages()
                    .languages
                    .get(*i)
                    .is_some_and(|e| e.iso3 == iso3)
            })
            .unwrap_or(0);
        w.reveal = 3;
    }

    /// The Pays tab on a country (from a language).
    fn props_goto_country(&mut self, a3: &str) {
        let loc = self.i18n.current_locale().to_string();
        let w = &mut self.props_win;
        w.tab = PropTab::Countries;
        w.country_filter.clear();
        w.country_sel = Some(a3.to_string());
        let rows = props::country_rows(&self.countries, &self.country_names, &loc, "");
        w.country_cursor = rows.iter().position(|r| r.0 == a3).unwrap_or(0);
        w.reveal = 3;
    }
}
