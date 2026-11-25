//! Menus that behave like the system's (Windows recette of 14/11/2026).
//!
//! - A sub-menu closes as soon as the pointer moves to another entry of its
//!   menu, enabled or not.
//! - A sub-menu opens on the left of its menu when the right side lacks
//!   room, instead of being pushed back over its menu.
//! - A context menu opens on the left of the pointer, or above it, near the
//!   right or bottom edge of the window.
//!
//! egui 0.29 could do none of this, and the 0.4 line drew its own sub-menus
//! here. Since egui 0.32 the menus are popups that try the other side when
//! one lacks room and keep one open sub-menu per menu (D-103, R-D103-03): the
//! functions below are thin wrappers kept so that the call sites read the
//! same, and so that a disabled sub-menu stays a plain disabled entry; only
//! [`context_menu`] still chooses its side, egui pushing a menu that fits on
//! no side back over the pointer.

use eframe::egui::{self, Pos2, Rect, RectAlign, Response, Ui, Vec2, WidgetText};

/// A sub-menu entry « text ⏵ » of the menu `ui`. Disabled, it is a plain
/// disabled entry that never opens. Returns what `add` returned when the
/// sub-menu is shown.
pub fn submenu<R>(
    ui: &mut Ui,
    enabled: bool,
    text: impl Into<WidgetText>,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if enabled {
        ui.menu_button(text, add).inner
    } else {
        ui.add_enabled(false, egui::Button::new(text));
        None
    }
}

/// Close the whole menu from one of its entries.
pub fn close(ui: &mut Ui) {
    ui.close();
}

/// `response.context_menu(add)`, placed like the system's: on the right of
/// the pointer, on its left when the right lacks room; below it, above it
/// when the bottom lacks room. egui pushes a menu that fits on no side back
/// into the window, over the pointer; the side chosen here keeps the
/// pointer out of the menu horizontally.
pub fn context_menu(response: &Response, add: impl FnOnce(&mut Ui)) {
    let ctx = response.ctx.clone();
    let key = egui::Id::unique(("ximod_context_menu", response.id));
    if response.secondary_clicked()
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        ctx.data_mut(|d| d.insert_temp(key, (p, None::<Vec2>)));
    }
    let mut popup = egui::Popup::context_menu(response);
    let state = ctx.data(|d| d.get_temp::<(Pos2, Option<Vec2>)>(key));
    if let Some((at, Some(size))) = state {
        popup = popup
            .align(context_align(at, size, ctx.content_rect()))
            .align_alternatives(&[]);
    }
    if let Some(shown) = popup.show(add)
        && let Some((at, _)) = state
    {
        let size = shown.response.rect.size();
        ctx.data_mut(|d| d.insert_temp(key, (at, Some(size))));
    }
}

/// The side of a context menu of `size` opened at `at` in `screen`.
pub fn context_align(at: Pos2, size: Vec2, screen: Rect) -> RectAlign {
    let left = at.x + size.x > screen.right() && at.x - size.x >= screen.left();
    let above = at.y + size.y > screen.bottom() && at.y - size.y >= screen.top();
    match (above, left) {
        (false, false) => RectAlign::BOTTOM_START,
        (false, true) => RectAlign::BOTTOM_END,
        (true, false) => RectAlign::TOP_START,
        (true, true) => RectAlign::TOP_END,
    }
}

/// A menu of the menu bar.
pub fn bar_menu<R>(
    ui: &mut Ui,
    title: impl Into<WidgetText>,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    ui.menu_button(title, add).inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_menu_keeps_the_pointer_out() {
        let screen = Rect::from_min_max(Pos2::ZERO, Pos2::new(1000.0, 700.0));
        let size = Vec2::new(300.0, 500.0);
        let at = |x, y| context_align(Pos2::new(x, y), size, screen);
        assert_eq!(at(100.0, 100.0), RectAlign::BOTTOM_START);
        // Near the right edge: on the left of the pointer.
        assert_eq!(at(900.0, 100.0), RectAlign::BOTTOM_END);
        // Near the bottom: above it; near both: above and on the left.
        assert_eq!(at(100.0, 600.0), RectAlign::TOP_START);
        assert_eq!(at(900.0, 600.0), RectAlign::TOP_END);
        // Too tall for either side: below, egui keeps it in the window.
        assert_eq!(at(900.0, 300.0), RectAlign::BOTTOM_END);
    }
}
