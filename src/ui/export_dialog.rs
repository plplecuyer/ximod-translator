//! The « Lignes exportées » dialog (S-04, figure B.1): the rule, the optional
//! statuses to include (unchecked at each opening, R-S04-03), the warning once
//! one is checked, and the live count (R-S04-05). Drawn by
//! `XimodApp::export_window`; kept as a free function so it can be previewed
//! and tested on its own.

use eframe::egui;

use crate::export::{ExportCounts, ExportPolicy};
use crate::i18n::I18n;
use crate::status::Status;

/// Width of the dialog's content.
pub const WIDTH: f32 = 520.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancel,
    Export,
}

/// A count with thousands grouped the way the interface language does it
/// (« 1 022 » in French, "1,022" otherwise).
pub fn fmt_count(i18n: &I18n, n: usize) -> String {
    let digits = n.to_string();
    let sep = if i18n.current_locale() == "fra" {
        '\u{00A0}'
    } else {
        ','
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// The count line: « N lignes seront exportées (a validées + b …) ».
pub fn count_line(i18n: &I18n, counts: &ExportCounts) -> String {
    let others: Vec<String> = ExportPolicy::optional_statuses()
        .filter(|s| counts.by_status[s.index()] > 0)
        .map(|s| {
            format!(
                "{} {}",
                fmt_count(i18n, counts.by_status[s.index()]),
                i18n.t(s.legend_key()).to_lowercase()
            )
        })
        .collect();
    if others.is_empty() {
        i18n.t_arg(
            "export-count-validated",
            "validated",
            &fmt_count(i18n, counts.validated),
        )
    } else {
        i18n.t_args(
            "export-count",
            &[
                ("total", &fmt_count(i18n, counts.total)),
                ("validated", &fmt_count(i18n, counts.validated)),
                ("others", &others.join(" + ")),
            ],
        )
    }
}

/// What the override patch of a plugin will contain (S-01, figure B.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideInfo {
    /// The original is localized: the patch is written delocalized.
    pub localized: bool,
    pub game: String,
    pub records: usize,
    pub unresolved: usize,
    pub esl_eligible: bool,
}

/// What localizing a plugin will write (C-09, figure B.2): the lines of
/// the files, the source-language option and the table used.
pub struct LocalizeInfo<'a> {
    pub intro: &'a str,
    pub files_title: &'a str,
    /// (file, what it holds).
    pub files: &'a [(String, String)],
    pub source_label: &'a str,
    pub source_too: &'a mut bool,
    pub table: &'a str,
    /// « champs inconnus : n », when there are.
    pub unknown: Option<&'a str>,
}

/// Débogage en jeu (A-22): the preview of the first line written with its
/// identifier, the warning when translations already start with one, and
/// « voir les lignes ».
pub struct IdsInfo<'a> {
    pub preview: Option<&'a str>,
    /// What a debugging export writes, in place of the count of the
    /// exported translations: every line shown, prefixed.
    pub count: Option<&'a str>,
    pub present: Option<&'a str>,
    pub show_rows: &'a mut bool,
}

/// The parts of the dialog that depend on the document.
#[derive(Default)]
pub struct Extra<'a> {
    /// A text document (C-05): what an untranslated line becomes.
    pub untranslated_copy: Option<&'a mut bool>,
    /// A full plugin proposed in place of the open one (E-06).
    pub overwrite_note: Option<&'a str>,
    /// Lines with characters the page of writing cannot hold (D-09): the
    /// refusal, and the button that replaces them by « ? ».
    pub encoding_bad: Option<&'a str>,
    /// A plugin to localize (C-09).
    pub localize: Option<LocalizeInfo<'a>>,
    /// Exported VMAD lines the output does not write (C-08).
    pub vmad_note: Option<&'a str>,
    /// The debugging export (A-22).
    pub ids: Option<IdsInfo<'a>>,
}

/// Draw the dialog's content; `policy` and `esl` are updated in place.
pub fn body(
    ui: &mut egui::Ui,
    i18n: &I18n,
    policy: &mut ExportPolicy,
    counts: &ExportCounts,
    override_info: Option<&OverrideInfo>,
    esl: &mut bool,
    extra: Extra<'_>,
) -> Outcome {
    let Extra {
        untranslated_copy,
        overwrite_note,
        encoding_bad,
        localize,
        vmad_note,
        ids,
    } = extra;
    let mut outcome = Outcome::Open;
    let labels: Vec<(Status, String)> = ExportPolicy::optional_statuses()
        .map(|s| {
            let mut l = i18n.t(s.legend_key());
            if s == Status::BySimilarity {
                l = format!("{l} {}", i18n.t("export-to-review"));
            }
            (s, l)
        })
        .collect();

    ui.set_width(WIDTH);
    ui.add_space(4.0);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(i18n.t("export-lines-group")).strong());
        // The rule, wrapped at spaces to the frame's width (A.3.4).
        ui.add(egui::Label::new(i18n.t("export-rule-hint")).wrap_mode(egui::TextWrapMode::Wrap));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n.t("export-include-label")).strong());
        ui.columns(2, |cols| {
            for (i, (s, label)) in labels.iter().enumerate() {
                let col = &mut cols[i % 2];
                let mut on = policy.include[s.index()];
                if col.checkbox(&mut on, label).changed() {
                    *policy = policy.with(*s, on);
                }
            }
        });
        if policy.includes_any() {
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(255, 244, 214))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgb(220, 160, 60),
                ))
                .inner_margin(6.0)
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} {}",
                                crate::icons::WARNING,
                                i18n.t("export-include-warning")
                            ))
                            .color(egui::Color32::from_rgb(140, 80, 0)),
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
        }
    });
    // ---- Override patch (S-01): delocalized when the original is localized ----
    if let Some(info) = override_info {
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            if info.localized {
                ui.label(
                    egui::RichText::new(i18n.t_arg("export-override-title", "game", &info.game))
                        .strong(),
                );
                ui.add(
                    egui::Label::new(i18n.t("export-override-hint"))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
            } else {
                ui.label(egui::RichText::new(i18n.t("export-override-plain")).strong());
            }
            ui.add_space(4.0);
            let line = i18n.t_args(
                "export-override-counts",
                &[
                    ("unresolved", &fmt_count(i18n, info.unresolved)),
                    ("records", &fmt_count(i18n, info.records)),
                ],
            );
            if info.unresolved > 0 {
                ui.colored_label(egui::Color32::from_rgb(200, 120, 40), line);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(i18n.t("export-override-unresolved-hint")).weak(),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
            } else {
                ui.label(line);
            }
            ui.add_space(4.0);
            ui.add_enabled(
                info.esl_eligible,
                egui::Checkbox::new(esl, i18n.t("export-override-esl-check")),
            );
        });
    }
    // A text document (C-05, decision D-25): what an untranslated line
    // becomes in the written file.
    if let Some(copy) = untranslated_copy {
        ui.add_space(6.0);
        ui.checkbox(copy, i18n.t("text-untranslated-copy"));
    }
    // Localizing a plugin (C-09): what will be written.
    if let Some(info) = localize {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(egui::RichText::new(info.intro).weak())
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(info.files_title).strong().small());
            egui::Grid::new("ximod-localize-files")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for (f, what) in info.files {
                        ui.label(egui::RichText::new(f).monospace().small());
                        ui.label(egui::RichText::new(what).weak().small());
                        ui.end_row();
                    }
                });
        });
        ui.checkbox(info.source_too, info.source_label);
        ui.add(
            egui::Label::new(egui::RichText::new(info.table).weak().small())
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        if let Some(u) = info.unknown {
            ui.colored_label(ui.visuals().warn_fg_color, egui::RichText::new(u).small());
        }
    }
    // A full plugin is proposed in place of the open one (E-06).
    if let Some(note) = overwrite_note {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(egui::RichText::new(note).weak()).wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
    // VMAD lines that the STRINGS do not carry (C-08).
    if let Some(note) = vmad_note {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(egui::RichText::new(note).color(ui.visuals().warn_fg_color))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
    // Débogage en jeu (A-22, figure B.5): unchecked at each opening, the
    // document is not modified.
    let mut count_text = count_line(i18n, counts);
    if let Some(info) = ids {
        if policy.ids.is_some()
            && let Some(c) = info.count
        {
            count_text = c.to_string();
        }
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(i18n.t("export-debug-group")).strong());
            let mut on = policy.ids.is_some();
            if ui.checkbox(&mut on, i18n.t("export-ids")).changed() {
                policy.ids = on.then_some(crate::export::IdForm::FormId);
            }
            if let Some(form) = policy.ids {
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    for f in crate::export::IdForm::ALL {
                        if ui.radio(form == f, i18n.t(f.i18n_key())).clicked() {
                            policy.ids = Some(f);
                        }
                    }
                });
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(i18n.t("export-ids-note"))
                            .weak()
                            .small(),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                if let Some(p) = info.preview {
                    ui.add(
                        egui::Label::new(egui::RichText::new(p).monospace().small())
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                }
            }
            if let Some(present) = info.present {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("{} {present}", crate::icons::WARNING),
                    );
                    if ui.link(i18n.t("export-ids-present-rows")).clicked() {
                        *info.show_rows = true;
                    }
                });
            }
        });
    }
    ui.add_space(8.0);
    ui.add(egui::Label::new(count_text).wrap_mode(egui::TextWrapMode::Wrap));
    // Characters impossible in the page of writing (R-D09-05, D-60): the
    // export is refused unless they are replaced.
    if let Some(bad) = encoding_bad {
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(egui::RichText::new(bad).color(ui.visuals().error_fg_color))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let export_l = i18n.t(if encoding_bad.is_some() {
                "export-encoding-replace"
            } else {
                "btn-export"
            });
            // A debugging export writes the source of the other lines too.
            if ui
                .add_enabled(
                    counts.total > 0 || policy.ids.is_some(),
                    egui::Button::new(export_l),
                )
                .clicked()
            {
                outcome = Outcome::Export;
            }
            if ui.button(i18n.t("btn-cancel")).clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_line_lists_included_statuses() {
        let mut i18n = I18n::new();
        i18n.set_locale("fra");
        let mut c = ExportCounts {
            validated: 812,
            total: 812,
            ..Default::default()
        };
        assert_eq!(
            count_line(&i18n, &c),
            "812 lignes seront exportées (validées)."
        );
        c.by_status[Status::BySimilarity.index()] = 210;
        c.total = 1022;
        assert_eq!(
            count_line(&i18n, &c),
            "1\u{00A0}022 lignes seront exportées (812 validées + 210 traduit par similarité)."
        );
        assert_eq!(fmt_count(&i18n, 1234567), "1\u{00A0}234\u{00A0}567");
        assert_eq!(fmt_count(&i18n, 999), "999");
        i18n.set_locale("eng");
        assert_eq!(fmt_count(&i18n, 1022), "1,022");
    }
}
