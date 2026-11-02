//! User-interface module.

pub mod app;
pub mod apply_dialog;
pub mod color_picker;
pub mod dialog_window;
pub mod dict_editor;
pub mod dicts_window;
pub mod diff_view;
pub mod edit_window;
pub mod export_dialog;
pub mod fields_editor;
pub mod fomod_window;
pub mod formats_editor;
pub mod grid;
pub mod package_window;
pub mod quality_window;
pub mod replace_window;
pub mod special;
pub mod theme;
pub mod toasts;
pub mod translation_editor;
pub mod update_window;

pub use app::{XimodApp, output_mode_key};
