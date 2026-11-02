//! The XIMOD look shared with Architect (F-06): a semantic palette
//! (accent, success, warning, danger) applied to egui's visuals of both
//! themes, soft rounding and a readable selection. Spacing is left to egui so
//! the grid keeps its density.

use eframe::egui::{self, Color32, Rounding, Stroke};

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub selection_bg: Color32,
}

impl Palette {
    pub const DARK: Palette = Palette {
        accent: Color32::from_rgb(86, 156, 214),
        success: Color32::from_rgb(110, 200, 120),
        warning: Color32::from_rgb(240, 190, 80),
        danger: Color32::from_rgb(240, 110, 110),
        selection_bg: Color32::from_rgba_premultiplied(40, 80, 120, 200),
    };

    pub const LIGHT: Palette = Palette {
        accent: Color32::from_rgb(30, 100, 190),
        success: Color32::from_rgb(30, 130, 50),
        warning: Color32::from_rgb(170, 110, 0),
        danger: Color32::from_rgb(190, 40, 40),
        selection_bg: Color32::from_rgba_premultiplied(190, 215, 240, 255),
    };

    pub fn of(visuals: &egui::Visuals) -> Palette {
        if visuals.dark_mode {
            Palette::DARK
        } else {
            Palette::LIGHT
        }
    }
}

/// egui visuals of one theme with the XIMOD palette.
pub fn visuals_for(dark: bool) -> egui::Visuals {
    let palette = if dark { Palette::DARK } else { Palette::LIGHT };
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.selection.bg_fill = palette.selection_bg;
    visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, palette.accent.gamma_multiply(0.7));
    visuals.error_fg_color = palette.danger;
    visuals.warn_fg_color = palette.warning;

    let r = Rounding::same(6.0);
    visuals.widgets.noninteractive.rounding = r;
    visuals.widgets.inactive.rounding = r;
    visuals.widgets.hovered.rounding = r;
    visuals.widgets.active.rounding = r;
    visuals.widgets.open.rounding = r;
    visuals.window_rounding = Rounding::same(8.0);
    visuals.menu_rounding = Rounding::same(6.0);
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 12.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if dark { 110 } else { 50 }),
    };
    visuals
}
