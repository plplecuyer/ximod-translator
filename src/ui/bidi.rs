//! A text shown in its visual order (E-08): the lines cut on the logical
//! text, each line's runs placed by [`crate::text::bidi`], each run shaped
//! by egui. A right-to-left text is aligned to the right; the layout of the
//! windows is not mirrored.

use std::sync::Arc;

use eframe::egui;

use crate::text::bidi;

/// A text laid out for display: its lines, top to bottom.
pub struct Laid {
    pub rows: Vec<Arc<egui::Galley>>,
    pub size: egui::Vec2,
    /// Aligned to the right.
    pub rtl: bool,
}

/// The lines of a paragraph cut at `max_width` on its words (the spaces
/// stay at the end of their word); a word wider than the line is cut on
/// its characters.
fn cut_lines(
    fonts: &mut egui::epaint::FontsView<'_>,
    para: &str,
    font: &egui::FontId,
    max_width: f32,
) -> Vec<std::ops::Range<usize>> {
    let width = |f: &mut egui::epaint::FontsView<'_>, s: &str| -> f32 {
        f.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    };
    let mut words: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, c) in para.char_indices() {
        if c.is_whitespace() {
            in_space = true;
        } else if in_space {
            words.push(start..i);
            start = i;
            in_space = false;
        }
    }
    if start < para.len() {
        words.push(start..para.len());
    }
    let mut lines = Vec::new();
    let mut line: Option<std::ops::Range<usize>> = None;
    for w in words {
        let candidate = match &line {
            Some(l) => l.start..w.end,
            None => w.clone(),
        };
        let fits =
            !max_width.is_finite() || width(fonts, para[candidate.clone()].trim_end()) <= max_width;
        if fits {
            line = Some(candidate);
            continue;
        }
        if let Some(l) = line.take() {
            lines.push(l);
        }
        // The word alone: cut on its characters when it is too wide.
        if width(fonts, para[w.clone()].trim_end()) <= max_width {
            line = Some(w);
        } else {
            let mut s = w.start;
            let mut last_fit = s;
            for (i, c) in para[w.clone()].char_indices() {
                let end = w.start + i + c.len_utf8();
                if width(fonts, &para[s..end]) > max_width && last_fit > s {
                    lines.push(s..last_fit);
                    s = last_fit;
                }
                last_fit = end;
            }
            line = Some(s..w.end);
        }
    }
    if let Some(l) = line {
        lines.push(l);
    }
    if lines.is_empty() {
        lines.push(0..para.len());
    }
    lines
}

/// Lay out `text` (in logical order) for display: `base_rtl` is the
/// direction of its column (a document text) or of its first strong
/// character (a text of the interface).
pub fn layout(
    ui: &egui::Ui,
    text: &str,
    base_rtl: bool,
    font: &egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> Laid {
    let rtl = base_rtl;
    let mut rows = Vec::new();
    ui.fonts_mut(|f| {
        for para in text.split('\n') {
            let para = para.strip_suffix('\r').unwrap_or(para);
            if para.is_empty() {
                rows.push(f.layout_no_wrap(String::new(), font.clone(), color));
                continue;
            }
            let lines = cut_lines(f, para, font, max_width);
            let iso = bidi::isolates(para);
            for runs in bidi::visual_lines(para, base_rtl, &iso, &lines) {
                let mut job = egui::text::LayoutJob::default();
                for r in &runs {
                    for piece in bidi::pieces(para, r) {
                        job.append(&piece, 0.0, egui::TextFormat::simple(font.clone(), color));
                    }
                }
                rows.push(f.layout_job(job));
            }
        }
    });
    let w = rows.iter().map(|g| g.size().x).fold(0.0_f32, f32::max);
    let h = rows.iter().map(|g| g.size().y).sum::<f32>();
    Laid {
        rows,
        size: egui::vec2(w, h),
        rtl,
    }
}

/// Paint a laid text in `rect`, from its top, to the right edge when it is
/// right to left.
pub fn paint(ui: &egui::Ui, laid: &Laid, rect: egui::Rect, color: egui::Color32) {
    let mut y = rect.top();
    for g in &laid.rows {
        let x = if laid.rtl {
            rect.right() - g.size().x
        } else {
            rect.left()
        };
        ui.painter().galley(egui::pos2(x, y), g.clone(), color);
        y += g.size().y;
    }
}

/// A label in visual order, as wide as the space given, the text on the
/// right when it is right to left.
pub fn label(
    ui: &mut egui::Ui,
    text: &str,
    base_rtl: bool,
    font: &egui::FontId,
    color: egui::Color32,
) -> egui::Response {
    let width = ui.available_width();
    let laid = layout(ui, text, base_rtl, font, color, width);
    let w = if base_rtl { width } else { laid.size.x };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, laid.size.y), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui, &laid, rect, color);
    }
    resp
}

/// A label of a document text: in visual order when it needs it, a plain
/// wrapped label otherwise (the cost is nil for the other languages).
pub fn doc_label(ui: &mut egui::Ui, text: &str, base_rtl: bool) -> egui::Response {
    let color = ui.visuals().text_color();
    doc_label_colored(ui, text, base_rtl, color)
}

/// [`doc_label`] in a colour of its own.
pub fn doc_label_colored(
    ui: &mut egui::Ui,
    text: &str,
    base_rtl: bool,
    color: egui::Color32,
) -> egui::Response {
    if bidi::needed(text, base_rtl) {
        let font = egui::TextStyle::Body.resolve(ui.style());
        label(ui, text, base_rtl, &font, color)
    } else {
        ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap())
    }
}

/// A document text on one line in visual order (a cell of a table): the
/// line feeds become spaces; the ranges of `hits` take the format `hit` (a
/// word of a right-to-left script is highlighted whole).
pub fn line_job(
    text: &str,
    base_rtl: bool,
    plain: &egui::TextFormat,
    hits: &[(usize, usize)],
    hit: Option<&egui::TextFormat>,
) -> egui::text::LayoutJob {
    let one: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let mut job = egui::text::LayoutJob::default();
    let in_hit = |a: usize, b: usize| hits.iter().any(|&(x, y)| x < b && a < y);
    for run in bidi::visual_line(&one, base_rtl) {
        for p in bidi::pieces_at(&one, &run) {
            match hit {
                // A left-to-right piece is cut at the highlights.
                Some(h) if !run.rtl => {
                    let mut cuts = vec![p.range.start, p.range.end];
                    for &(x, y) in hits {
                        for c in [x, y] {
                            if c > p.range.start && c < p.range.end {
                                cuts.push(c);
                            }
                        }
                    }
                    cuts.sort_unstable();
                    cuts.dedup();
                    for w in cuts.windows(2) {
                        let f = if in_hit(w[0], w[1]) { h } else { plain };
                        job.append(&one[w[0]..w[1]], 0.0, f.clone());
                    }
                }
                Some(h) if in_hit(p.range.start, p.range.end) => {
                    job.append(&p.text, 0.0, h.clone());
                }
                _ => job.append(&p.text, 0.0, plain.clone()),
            }
        }
    }
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// A document text on one line for a label: in visual order when it needs
/// it, as it is otherwise.
pub fn line_text(ui: &egui::Ui, text: &str, base_rtl: bool) -> egui::WidgetText {
    if bidi::needed(text, base_rtl) {
        let plain = egui::TextFormat::simple(
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        line_job(text, base_rtl, &plain, &[], None).into()
    } else {
        text.into()
    }
}

/// A text of the interface that may quote a document text (a finding of
/// the checks): its direction is that of its first strong character.
pub fn ui_line_text(ui: &egui::Ui, text: &str) -> egui::WidgetText {
    line_text(ui, text, bidi::first_strong_is_rtl(text))
}
