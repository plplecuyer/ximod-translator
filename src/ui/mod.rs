//! User-interface module.

pub mod app;
pub mod apply_dialog;
pub mod bidi;
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
pub mod keys;
pub mod menus;
pub mod package_window;
pub mod quality_window;
pub mod replace_window;
pub mod special;
pub mod theme;
pub mod toasts;
pub mod translation_editor;
pub mod update_window;

pub use app::{XimodApp, output_mode_key};

/// The `file://` address egui's file loader reads `path` from (flags, book
/// images). `format!("file://{}", path.display())` only works for a Unix
/// path: on Windows egui 0.37 takes `file://F:\…` for a network path
/// `\\F:\…` and finds nothing (flags missing in Propriétés and in the country
/// window, 17/11/2026). The path is made absolute (the development layout
/// gives `assets/images/svg` relative to the working directory).
pub fn file_uri(path: &std::path::Path) -> String {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    file_uri_of(&abs.to_string_lossy(), cfg!(windows))
}

/// [`file_uri`] of an absolute path written for Windows (`windows`) or Unix.
fn file_uri_of(abs: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{abs}");
    }
    let p = abs.replace('\\', "/");
    if let Some(unc) = p.strip_prefix("//?/UNC/") {
        // \\?\UNC\server\share\… → file://server/share/…
        format!("file://{unc}")
    } else if let Some(local) = p.strip_prefix("//?/") {
        format!("file:///{local}")
    } else if let Some(unc) = p.strip_prefix("//") {
        format!("file://{unc}")
    } else {
        format!("file:///{p}")
    }
}

#[cfg(test)]
mod tests {
    use super::file_uri_of;

    #[test]
    fn file_uris_egui_reads_on_every_system() {
        assert_eq!(
            file_uri_of("/home/p/assets/images/svg/fr.svg", false),
            "file:///home/p/assets/images/svg/fr.svg"
        );
        assert_eq!(
            file_uri_of(r"F:\Users\plple\ximod\assets\images\svg\fr.svg", true),
            "file:///F:/Users/plple/ximod/assets/images/svg/fr.svg"
        );
        assert_eq!(file_uri_of(r"\\?\F:\x\fr.svg", true), "file:///F:/x/fr.svg");
        assert_eq!(
            file_uri_of(r"\\nas\partage\fr.svg", true),
            "file://nas/partage/fr.svg"
        );
        assert_eq!(
            file_uri_of(r"\\?\UNC\nas\partage\fr.svg", true),
            "file://nas/partage/fr.svg"
        );
    }
}
