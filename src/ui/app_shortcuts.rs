//! Paramètres › Raccourcis (E-15, D-123): the commands with their place and
//! shortcut, a capture on click, the conflicts and the reserved
//! combinations, the return to the origin.

use eframe::egui;

use super::XimodApp;
use crate::shortcuts::{self, Cmd, Refusal, Table};

/// What the tab is doing.
#[derive(Default)]
pub(crate) struct ShortcutsTab {
    /// The table being edited.
    pub table: Table,
    pub filter: String,
    /// The command whose shortcut is being captured.
    pub capture: Option<Cmd>,
    /// A shortcut captured that another command has: `(command, shortcut,
    /// the other)`, until Remplacer or Annuler.
    pub conflict: Option<(Cmd, egui::KeyboardShortcut, Cmd)>,
    /// Why the last capture was refused.
    pub refusal: Option<String>,
}

impl XimodApp {
    /// The Paramètres open: the shortcuts in force.
    pub(crate) fn shortcuts_tab_load(&mut self) {
        self.sc_tab = ShortcutsTab {
            table: Table::with_overrides(&self.config.shortcuts),
            ..Default::default()
        };
    }

    /// Enregistrer: `[Shortcuts]` and the table in force.
    pub(crate) fn shortcuts_tab_save(&mut self) {
        self.config.shortcuts = self.sc_tab.table.overrides();
        shortcuts::install(self.sc_tab.table.clone());
        self.sc_tab.capture = None;
        self.sc_tab.conflict = None;
    }

    /// Whether a capture holds the keyboard (the Paramètres' own keys then
    /// wait: Échap cancels the capture, not the window).
    pub(crate) fn shortcuts_capturing(&self) -> bool {
        self.sc_tab.capture.is_some()
    }

    /// The keys of a capture (R-E15-02): the first combination pressed.
    /// Échap cancels, Retour arrière removes the shortcut; every key event
    /// of the frame is consumed.
    pub(crate) fn shortcuts_capture_keys(&mut self, vctx: &egui::Context) {
        let Some(cmd) = self.sc_tab.capture else {
            return;
        };
        // egui turns Ctrl+C / X / V into events of their own: refused as
        // reserved (R-E15-04).
        let clip = vctx.input_mut(|i| {
            let k = i.events.iter().find_map(|e| match e {
                egui::Event::Copy => Some(egui::Key::C),
                egui::Event::Cut => Some(egui::Key::X),
                egui::Event::Paste(_) => Some(egui::Key::V),
                _ => None,
            });
            if k.is_some() {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Paste(_)
                            | egui::Event::Key { .. }
                            | egui::Event::Text(_)
                    )
                });
            }
            k
        });
        if let Some(k) = clip {
            let text = super::shortcut_text(
                vctx,
                &egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, k),
                &self.i18n,
            );
            self.sc_tab.refusal = Some(self.i18n.t_arg("sc-reserved", "key", &text));
            self.sc_tab.capture = None;
            return;
        }
        let pressed = vctx.input_mut(|i| {
            // egui 0.37 also sends the modifier keys themselves: the
            // combination is the first other key.
            let first = i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } if !is_modifier_key(*key) => Some((*key, *physical_key, *modifiers)),
                _ => None,
            });
            if first.is_some() {
                i.events
                    .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
            }
            first
        });
        let Some((key, physical, m)) = pressed else {
            return;
        };
        let plain = !(m.command || m.ctrl || m.mac_cmd || m.alt || m.shift);
        if plain && key == egui::Key::Escape {
            self.sc_tab.capture = None;
            return;
        }
        if plain && key == egui::Key::Backspace {
            self.sc_tab.table.set(cmd, None);
            self.sc_tab.capture = None;
            return;
        }
        // The physical key when the system gave no letter (Ctrl+J as
        // Entrée), as the shortcut is then matched (keys.rs).
        let key = match physical {
            Some(p) if !crate::ui::keys::is_letter(key) && crate::ui::keys::is_letter(p) => p,
            _ => key,
        };
        let mut mods = egui::Modifiers::NONE;
        if m.command || m.ctrl || m.mac_cmd {
            mods = mods.plus(egui::Modifiers::COMMAND);
        }
        if m.shift {
            mods = mods.plus(egui::Modifiers::SHIFT);
        }
        if m.alt {
            mods = mods.plus(egui::Modifiers::ALT);
        }
        let sc = egui::KeyboardShortcut::new(mods, key);
        let text = super::shortcut_text(vctx, &sc, &self.i18n);
        match shortcuts::check(&sc) {
            Err(Refusal::Reserved) => {
                self.sc_tab.refusal = Some(self.i18n.t_arg("sc-reserved", "key", &text));
                self.sc_tab.capture = None;
                return;
            }
            Err(Refusal::NeedsModifier) => {
                self.sc_tab.refusal = Some(self.i18n.t("sc-needs-modifier"));
                return;
            }
            Ok(()) => {}
        }
        self.sc_tab.refusal = None;
        self.sc_tab.capture = None;
        match self.sc_tab.table.conflict(cmd, &sc) {
            Some(other) => self.sc_tab.conflict = Some((cmd, sc, other)),
            None => self.sc_tab.table.set(cmd, Some(sc)),
        }
    }

    /// The tab.
    pub(crate) fn settings_shortcuts(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add(
            egui::TextEdit::singleline(&mut self.sc_tab.filter)
                .hint_text(self.i18n.t("sc-search-hint"))
                .desired_width(360.0),
        );
        ui.add_space(6.0);
        let needle = self.sc_tab.filter.trim().to_lowercase();
        let rows: Vec<(Cmd, String, String, String)> = Cmd::ALL
            .iter()
            .map(|c| {
                let label = self.i18n.t(&c.label_key());
                let place = self.i18n.t(c.place().label_key());
                let key = self
                    .sc_tab
                    .table
                    .get(*c)
                    .map(|s| super::shortcut_text(&ctx, &s, &self.i18n))
                    .unwrap_or_default();
                (*c, label, place, key)
            })
            .filter(|(_, l, p, k)| {
                needle.is_empty()
                    || l.to_lowercase().contains(&needle)
                    || p.to_lowercase().contains(&needle)
                    || k.to_lowercase().contains(&needle)
            })
            .collect();
        let capture_tip = self.i18n.t("sc-capture-tip");
        let none = self.i18n.t("sc-none");
        let mut start: Option<Cmd> = None;
        let mut reset: Option<Cmd> = None;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("shortcuts_grid")
                    .num_columns(4)
                    .min_col_width(170.0)
                    .striped(true)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(self.i18n.t("sc-col-command")).strong());
                        ui.label(egui::RichText::new(self.i18n.t("sc-col-place")).strong());
                        ui.label(egui::RichText::new(self.i18n.t("sc-col-key")).strong());
                        ui.label("");
                        ui.end_row();
                        for (c, label, place, key) in &rows {
                            ui.label(label);
                            ui.label(egui::RichText::new(place).weak());
                            let changed = !self.sc_tab.table.is_origin(*c);
                            let text = if self.sc_tab.capture == Some(*c) {
                                egui::RichText::new(self.i18n.t("sc-capture"))
                                    .color(ui.visuals().hyperlink_color)
                            } else if key.is_empty() {
                                egui::RichText::new(&none).weak()
                            } else if changed {
                                egui::RichText::new(key).strong()
                            } else {
                                egui::RichText::new(key)
                            };
                            // The cell, left-aligned on a fixed width: a click
                            // starts the capture.
                            let cell = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(220.0, ui.spacing().interact_size.y),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| ui.add(egui::Label::new(text).sense(egui::Sense::click())),
                                )
                                .inner;
                            if cell
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(&capture_tip)
                                .clicked()
                            {
                                start = Some(*c);
                            }
                            if changed {
                                let origin = c
                                    .origin()
                                    .map(|s| super::shortcut_text(&ctx, &s, &self.i18n))
                                    .unwrap_or_default();
                                if ui
                                    .small_button(crate::icons::UNDO)
                                    .on_hover_text(self.i18n.t_arg("sc-reset-tip", "key", &origin))
                                    .clicked()
                                {
                                    reset = Some(*c);
                                }
                            } else {
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(c) = start {
            self.sc_tab.capture = Some(c);
            self.sc_tab.conflict = None;
            self.sc_tab.refusal = None;
        }
        if let Some(c) = reset {
            self.sc_tab.table.set(c, c.origin());
        }
        // A conflict (R-E15-03): replace, or keep both as they were.
        if let Some((c, sc, other)) = self.sc_tab.conflict {
            ui.add_space(6.0);
            let key = super::shortcut_text(&ctx, &sc, &self.i18n);
            let warn = ui.visuals().warn_fg_color;
            egui::Frame::group(ui.style())
                .stroke(egui::Stroke::new(1.0_f32, warn))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(self.i18n.t_args(
                            "sc-conflict",
                            &[("key", &key), ("cmd", &self.i18n.t(&other.label_key()))],
                        ));
                        if ui.button(self.i18n.t("sc-do-replace")).clicked() {
                            self.sc_tab.table.set(other, None);
                            self.sc_tab.table.set(c, Some(sc));
                            self.sc_tab.conflict = None;
                        }
                        if ui.button(self.i18n.t("btn-cancel")).clicked() {
                            self.sc_tab.conflict = None;
                        }
                    });
                });
        }
        if let Some(msg) = &self.sc_tab.refusal {
            ui.add_space(4.0);
            ui.colored_label(ui.visuals().warn_fg_color, msg);
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(self.i18n.t("sc-diagnostic"))
                .weak()
                .small(),
        );
        ui.add_space(6.0);
        if ui
            .add_enabled(
                self.sc_tab.table != Table::default(),
                egui::Button::new(self.i18n.t("sc-reset-all")),
            )
            .clicked()
        {
            self.sc_tab.table = Table::default();
            self.sc_tab.conflict = None;
            self.sc_tab.capture = None;
        }
    }
}

/// Ctrl, Maj, Alt or the system key pressed alone.
fn is_modifier_key(k: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        k,
        ShiftLeft
            | ShiftRight
            | ControlLeft
            | ControlRight
            | AltLeft
            | AltRight
            | SuperLeft
            | SuperRight
    )
}
