//! Display of the special characters (A-11, R-A11-07): LF « ¶ », CR
//! « ↵ », TAB « → », no-break space « ° », in a muted colour, in the grid
//! and in the edit window. The text itself is never changed: an editable
//! field keeps its buffer and gets a layouter that substitutes one glyph
//! for one character (so the cursor keeps its positions) plus a « ¶ »
//! painted after each line break; a read-only label gets the glyphs in its
//! layout job, line breaks kept.

use eframe::egui::{self, Color32, FontId, TextFormat, text::LayoutJob};

/// The glyph shown for a special character, if any.
fn glyph(c: char) -> Option<char> {
    match c {
        '\r' => Some(crate::icons::RETURN_CHAR),
        '\t' => Some(crate::icons::ARROW_RIGHT_CHAR),
        '\u{a0}' => Some('°'),
        _ => None,
    }
}

/// A layout job of `text` with the special characters as muted glyphs.
/// `one_to_one`: every substitution keeps the character count (line feeds
/// stay line feeds); otherwise a « ¶ » precedes each line feed.
pub fn job(
    text: &str,
    font: FontId,
    color: Color32,
    muted: Color32,
    wrap_width: f32,
    one_to_one: bool,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let plain = TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let mark = TextFormat {
        font_id: font,
        color: muted,
        ..Default::default()
    };
    let mut run = String::new();
    for c in text.chars() {
        if let Some(g) = glyph(c) {
            if !run.is_empty() {
                job.append(&run, 0.0, plain.clone());
                run.clear();
            }
            job.append(&g.to_string(), 0.0, mark.clone());
        } else if c == '\n' && !one_to_one {
            if !run.is_empty() {
                job.append(&run, 0.0, plain.clone());
                run.clear();
            }
            job.append("¶\n", 0.0, mark.clone());
        } else {
            run.push(c);
        }
    }
    if !run.is_empty() {
        job.append(&run, 0.0, plain);
    }
    job
}

/// The layouter of an editable field: one glyph per character.
pub fn layouter(
    font: FontId,
    color: Color32,
    muted: Color32,
) -> impl FnMut(&egui::Ui, &str, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &str, wrap_width: f32| {
        let job = job(text, font.clone(), color, muted, wrap_width, true);
        ui.fonts(|f| f.layout_job(job))
    }
}

/// Paint a « ¶ » after every line that ends with a line feed, from the
/// galley of a text field drawn at `pos`.
pub fn paint_pilcrows(
    ui: &egui::Ui,
    galley: &egui::Galley,
    pos: egui::Pos2,
    font: &FontId,
    muted: Color32,
    clip_right: f32,
) {
    for row in &galley.rows {
        if row.ends_with_newline {
            let x = pos.x + row.rect.right() + 1.0;
            if x + 8.0 <= clip_right {
                ui.painter().text(
                    egui::pos2(x, pos.y + row.rect.top()),
                    egui::Align2::LEFT_TOP,
                    "¶",
                    font.clone(),
                    muted,
                );
            }
        }
    }
}

/// The muted colour of the glyphs for the current visuals.
pub fn muted(visuals: &egui::Visuals) -> Color32 {
    visuals.weak_text_color().gamma_multiply(0.9)
}
