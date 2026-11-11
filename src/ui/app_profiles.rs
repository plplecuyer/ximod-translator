//! The profiles of the OpenAI-compatible connections in Paramètres › En
//! ligne (D-03, figure B.2): the profile chosen — connection, prompt, form
//! of the answer, sampling, its own guards —, new, duplicate, remove,
//! restore; saved with the settings.

use crate::ui::theme::ComboField as _;
use std::ops::RangeInclusive;

use eframe::egui;

use super::XimodApp;
use crate::online::Service;
use crate::online::profile::{self, Answer, Origin, Warning};

/// A value that the profile may leave to the common setting: a box, then
/// the number when it is ticked.
fn opt_num<T>(
    ui: &mut egui::Ui,
    v: &mut Option<T>,
    range: RangeInclusive<T>,
    default: T,
    speed: f64,
) -> bool
where
    T: egui::emath::Numeric,
{
    let mut on = v.is_some();
    let mut changed = ui.checkbox(&mut on, "").changed();
    if changed {
        *v = on.then_some(default);
    }
    match v {
        Some(x) => {
            changed |= ui
                .add(egui::DragValue::new(x).range(range).speed(speed))
                .changed();
        }
        None => {
            ui.add_enabled(false, egui::Label::new(egui::RichText::new("—").weak()));
        }
    }
    changed
}

/// A label of the width of the settings' labels.
fn label_w(ui: &mut egui::Ui, lw: f32, text: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(lw, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(text);
        },
    );
}

impl XimodApp {
    /// The name of a service in the settings (a profile as edited).
    pub(crate) fn online_settings_label(&self, s: &Service) -> String {
        match s {
            Service::OpenAI(id) => {
                let name = self
                    .online
                    .profiles
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| p.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| id.clone());
                self.i18n.t_arg("online-service-profile", "name", &name)
            }
            other => self.i18n.t(other.label_key()),
        }
    }

    pub(crate) fn online_settings_key_optional(&self, s: &Service) -> bool {
        match s {
            Service::OpenAI(id) => self
                .online
                .profiles
                .iter()
                .find(|p| p.id == *id)
                .is_none_or(|p| p.key_optional),
            _ => false,
        }
    }

    /// The editor of a profile (R-D03-07).
    pub(crate) fn settings_profile(&mut self, ui: &mut egui::Ui, id: &str, lw: f32) {
        let Some(k) = self.online.profiles.iter().position(|p| p.id == id) else {
            return;
        };
        let t = |key: &str| self.i18n.t(key);
        let origin = match self.online.profiles[k].origin {
            Origin::Shipped => t("glossary-origin-shipped"),
            Origin::UserCopy => t("glossary-origin-user"),
            Origin::User => t("online-profile-own"),
        };
        let dirty = self.online.profiles_dirty.contains(id);
        let path = if self.online.profiles[k].origin == Origin::Shipped && !dirty {
            format!("assets/data/{}/{id}.json", profile::DIR)
        } else {
            format!("data/{}/{id}.json", profile::DIR)
        };
        let l_file = self.i18n.t_args(
            "online-profile-file",
            &[("path", &path), ("origin", &origin)],
        );
        let (l_name, l_prompt, l_vars, l_answer, l_json, l_lines, l_sep) = (
            t("online-profile-name"),
            t("online-prompt"),
            t("online-prompt-vars"),
            t("online-answer"),
            t("online-answer-json"),
            t("online-answer-lines"),
            t("online-separator"),
        );
        let (l_temp, l_tokens, l_pen, l_guards, l_lines_n, l_chars, l_rpm, l_timeout, l_optional) = (
            t("online-temperature"),
            t("online-max-tokens"),
            t("online-penalties"),
            t("online-profile-guards"),
            t("online-max-lines"),
            t("online-max-chars"),
            t("online-max-rpm"),
            t("online-timeout"),
            t("online-key-optional-check"),
        );
        let (l_new, l_dup, l_del, l_restore) = (
            t("online-profile-new"),
            t("online-profile-dup"),
            t("online-profile-del"),
            t("online-profile-restore"),
        );
        let l_endpoint_hint = "http://localhost:11434/v1";
        let mut changed = false;
        ui.label(egui::RichText::new(l_file).weak().small());
        {
            let p = &mut self.online.profiles[k];
            ui.horizontal(|ui| {
                label_w(ui, lw, &l_name);
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut p.name).desired_width(240.0))
                    .changed();
                ui.label(
                    egui::RichText::new(format!("openai:{}", p.id))
                        .weak()
                        .monospace(),
                );
            });
        }
        let (l_endpoint, l_model) = (t("online-endpoint"), t("online-model"));
        let p = &mut self.online.profiles[k];
        ui.horizontal(|ui| {
            label_w(ui, lw, &l_endpoint);
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut p.endpoint)
                        .hint_text(l_endpoint_hint)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(360.0),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            label_w(ui, lw, &l_model);
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut p.model)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(240.0),
                )
                .changed();
            changed |= ui.checkbox(&mut p.key_optional, &l_optional).changed();
        });
        ui.label(&l_prompt);
        egui::ScrollArea::vertical()
            .id_salt("ximod-profile-prompt")
            .max_height(110.0)
            .show(ui, |ui| {
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut p.prompt)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
            });
        ui.label(egui::RichText::new(&l_vars).weak().small());
        ui.horizontal_wrapped(|ui| {
            ui.label(&l_answer);
            egui::ComboBox::from_id_salt("ximod-profile-answer")
                .selected_text(match p.answer {
                    Answer::Json => &l_json,
                    Answer::Lines => &l_lines,
                })
                .show_field(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut p.answer, Answer::Json, &l_json)
                        .changed();
                    changed |= ui
                        .selectable_value(&mut p.answer, Answer::Lines, &l_lines)
                        .changed();
                });
            ui.label(&l_sep);
            changed |= ui
                .add_enabled(
                    p.answer == Answer::Lines,
                    egui::TextEdit::singleline(&mut p.separator)
                        .hint_text("⟨⟨n⟩⟩")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(70.0),
                )
                .changed();
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(&l_temp);
            changed |= opt_num(ui, &mut p.temperature, 0.0..=2.0, 0.2, 0.05);
            ui.add_space(8.0);
            ui.label(&l_tokens);
            changed |= opt_num(ui, &mut p.max_tokens, 1..=1_000_000, 4096, 64.0);
            ui.add_space(8.0);
            ui.label(&l_pen);
            changed |= opt_num(ui, &mut p.frequency_penalty, -2.0..=2.0, 0.0, 0.05);
            changed |= opt_num(ui, &mut p.presence_penalty, -2.0..=2.0, 0.0, 0.05);
        });
        ui.label(egui::RichText::new(&l_guards).weak());
        ui.horizontal_wrapped(|ui| {
            ui.label(&l_lines_n);
            changed |= opt_num(ui, &mut p.max_lines, 1..=500, 20, 1.0);
            ui.add_space(8.0);
            ui.label(&l_chars);
            changed |= opt_num(ui, &mut p.max_chars, 100..=100_000, 3000, 50.0);
            ui.add_space(8.0);
            ui.label(&l_rpm);
            changed |= opt_num(ui, &mut p.max_rpm, 1..=600, 60, 1.0);
            ui.add_space(8.0);
            ui.label(&l_timeout);
            changed |= opt_num(ui, &mut p.timeout, 5..=600, 120, 1.0);
        });
        if changed {
            self.online.profiles_dirty.insert(id.to_string());
        }
        // The problem of the profile, at once.
        match self.online.profiles[k].validate() {
            Err(e) => {
                let m = self.online_profile_invalid(&e);
                ui.colored_label(ui.visuals().error_fg_color, m);
            }
            Ok(w) if w.contains(&Warning::NoTarget) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    self.i18n.t("online-warn-no-dst"),
                );
            }
            Ok(_) => {}
        }
        // New, duplicate, remove, restore.
        let origin = self.online.profiles[k].origin;
        let mut action = 0u8;
        ui.horizontal(|ui| {
            if ui.button(&l_new).clicked() {
                action = 1;
            }
            if ui.button(&l_dup).clicked() {
                action = 2;
            }
            if ui
                .add_enabled(origin == Origin::User, egui::Button::new(&l_del))
                .clicked()
            {
                action = 3;
            }
            if ui
                .add_enabled(
                    origin == Origin::UserCopy || (origin == Origin::Shipped && dirty),
                    egui::Button::new(&l_restore),
                )
                .clicked()
            {
                action = 4;
            }
        });
        ui.add_space(4.0);
        let taken: Vec<String> = self.online.profiles.iter().map(|p| p.id.clone()).collect();
        match action {
            1 | 2 => {
                let mut p = if action == 1 {
                    profile::shipped(&profile::shipped_dir(), "local").unwrap_or_default()
                } else {
                    self.online.profiles[k].clone()
                };
                p.name = if action == 1 {
                    self.i18n.t("online-profile-new-name")
                } else {
                    self.i18n.t_arg("online-profile-copy-name", "name", &p.name)
                };
                p.id = profile::new_id(&p.name, &taken);
                p.origin = Origin::User;
                let new = p.id.clone();
                self.online.profiles.push(p);
                self.online.profiles_dirty.insert(new.clone());
                self.online.profiles_removed.retain(|r| *r != new);
                self.online.settings_service = Service::OpenAI(new);
            }
            3 => {
                self.online.profiles.remove(k);
                self.online.profiles_dirty.remove(id);
                self.online.profiles_removed.push(id.to_string());
                self.online.settings_service = Service::DeepL;
            }
            4 => {
                if let Some(p) = profile::shipped(&profile::shipped_dir(), id) {
                    self.online.profiles[k] = p;
                }
                self.online.profiles_dirty.remove(id);
                if origin == Origin::UserCopy {
                    self.online.profiles_removed.push(id.to_string());
                }
            }
            _ => {}
        }
    }
}
