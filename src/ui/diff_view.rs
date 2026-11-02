//! The « Différences caractère par caractère » frame (A-08): the two texts
//! with their differing runs highlighted, green for what the reference
//! has in addition, red for what the other has, readable in both themes;
//! ⇄ Inverser reads the difference from the entry.

use eframe::egui::{self, Color32};

use crate::diff::{Diff, visible_blanks};

/// The texts of the frame, localized by the application.
pub struct DiffLabels<'a> {
    pub title: &'a str,
    /// The label of the line's text and of the entry's.
    pub line: &'a str,
    pub entry: &'a str,
    pub swap: &'a str,
    /// The legend, and its reading once swapped.
    pub legend: &'a str,
    pub legend_swapped: &'a str,
    pub word_mode: &'a str,
}

/// A row of the frame: label, runs, highlight colour.
type Row<'a> = (&'a str, &'a [(String, bool)], Color32);

/// The highlight colours (plus, minus) and the text colour on them.
fn colours(dark: bool) -> (Color32, Color32, Color32) {
    if dark {
        (
            Color32::from_rgb(40, 105, 55),
            Color32::from_rgb(135, 55, 55),
            Color32::from_rgb(235, 235, 235),
        )
    } else {
        (
            Color32::from_rgb(190, 236, 190),
            Color32::from_rgb(250, 195, 195),
            Color32::from_rgb(30, 30, 30),
        )
    }
}

/// One text as a wrapped layout: its differing runs on `bg`, blanks made
/// visible in them.
fn job(
    ui: &egui::Ui,
    runs: &[(String, bool)],
    bg: Color32,
    on_bg: Color32,
) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width().max(80.0);
    let plain = egui::TextFormat {
        font_id: font.clone(),
        color: ui.visuals().text_color(),
        ..Default::default()
    };
    let marked = egui::TextFormat {
        font_id: font,
        color: on_bg,
        background: bg,
        ..Default::default()
    };
    for (text, differs) in runs {
        if *differs {
            let shown = visible_blanks(
                text,
                "·",
                crate::icons::RETURN,
                &crate::icons::ARROW_RIGHT_CHAR.to_string(),
            );
            job.append(&shown, 0.0, marked.clone());
        } else {
            job.append(text, 0.0, plain.clone());
        }
    }
    job
}

/// Draw the frame. `swapped` is toggled by ⇄ Inverser.
pub fn show(ui: &mut egui::Ui, diff: &Diff, swapped: &mut bool, labels: &DiffLabels<'_>) {
    let (plus, minus, on_bg) = colours(ui.visuals().dark_mode);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(labels.title).strong().small());
        // The reference first (its extras in green), then the other (red).
        let rows: [Row<'_>; 2] = if *swapped {
            [
                (labels.entry, &diff.entry, plus),
                (labels.line, &diff.line, minus),
            ]
        } else {
            [
                (labels.line, &diff.line, plus),
                (labels.entry, &diff.entry, minus),
            ]
        };
        let label_w = 150.0;
        for (label, runs, bg) in rows {
            ui.horizontal_top(|ui| {
                ui.allocate_ui(egui::vec2(label_w, 0.0), |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(label).weak())
                            .truncate()
                            .selectable(false),
                    );
                });
                let j = job(ui, runs, bg, on_bg);
                ui.add(egui::Label::new(j).selectable(false));
            });
        }
        ui.horizontal(|ui| {
            if ui.small_button(labels.swap).clicked() {
                *swapped = !*swapped;
            }
            let legend = if *swapped {
                labels.legend_swapped
            } else {
                labels.legend
            };
            ui.label(egui::RichText::new(legend).weak().small());
        });
        if diff.word_mode {
            ui.label(egui::RichText::new(labels.word_mode).weak().small());
        }
    });
}

/// One text on one line, its differing runs highlighted (the Mise à jour
/// du mod window, R-A08: old and new source), truncated by the cell.
pub fn one_line_job(
    ui: &egui::Ui,
    runs: &[(String, bool)],
    plus_side: bool,
) -> egui::text::LayoutJob {
    let (plus, minus, on_bg) = colours(ui.visuals().dark_mode);
    let flat: Vec<(String, bool)> = runs
        .iter()
        .map(|(t, d)| (t.replace(['\r', '\n'], " "), *d))
        .collect();
    let mut j = job(ui, &flat, if plus_side { plus } else { minus }, on_bg);
    j.wrap.max_width = f32::INFINITY;
    j
}
