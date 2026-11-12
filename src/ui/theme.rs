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

    // Buttons stand out from fields and lists (requested on 11/11/2026):
    // egui draws no relief, so a button gets a raised fill, between the
    // panel and the fields, and a dark outline; text fields keep the
    // sunken fill (`extreme_bg_color`) and combo boxes take it too through
    // [`ComboField::show_field`].
    let (raised, raised_hover, outline) = if dark {
        (
            Color32::from_gray(72),
            Color32::from_gray(88),
            Color32::from_gray(0),
        )
    } else {
        (
            Color32::from_gray(214),
            Color32::from_gray(200),
            Color32::from_gray(25),
        )
    };
    visuals.widgets.inactive.weak_bg_fill = raised;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, outline);
    visuals.widgets.hovered.weak_bg_fill = raised_hover;

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

/// The violet of a spelling mark (A-16), readable on both themes.
pub fn spelling_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(190, 140, 255)
    } else {
        Color32::from_rgb(130, 60, 200)
    }
}

/// The look of a field (sunken fill, the outline of the theme) on the
/// widgets of `visuals`: what a combo box borrows so it reads as a list to
/// fill in, not as a button.
pub fn field_look(visuals: &mut egui::Visuals) {
    let fill = visuals.extreme_bg_color;
    visuals.widgets.inactive.weak_bg_fill = fill;
    visuals.widgets.hovered.weak_bg_fill = fill;
}

/// `ComboBox::show_field`: [`egui::ComboBox::show_ui`] with the look of a
/// field ([`field_look`]); the popup keeps the look of the application.
pub trait ComboField {
    fn show_field<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>>;
}

impl ComboField for egui::ComboBox {
    fn show_field<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        ui.scope(|ui| {
            field_look(ui.visuals_mut());
            self.show_ui(ui, add_contents)
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_differ_from_fields_in_both_themes() {
        for dark in [false, true] {
            let v = visuals_for(dark);
            let button = v.widgets.inactive.weak_bg_fill;
            assert_ne!(button, v.extreme_bg_color, "dark={dark}");
            assert_ne!(button, v.panel_fill, "dark={dark}");
            assert!(v.widgets.inactive.bg_stroke.width >= 1.0);
            let mut f = v.clone();
            field_look(&mut f);
            assert_eq!(f.widgets.inactive.weak_bg_fill, v.extreme_bg_color);
            // The outline stays: a field is framed too.
            assert_eq!(f.widgets.inactive.bg_stroke, v.widgets.inactive.bg_stroke);
        }
    }
}
