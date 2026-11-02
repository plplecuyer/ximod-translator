//! Build script for XIMOD Translator.
//!
//! Windows: embeds the application icon and version metadata into the
//! executable. Other platforms need no special build steps.

fn main() {
    #[cfg(target_os = "windows")]
    {
        let icon_path = "assets/icons/ximod-translator.ico";
        if std::path::Path::new(icon_path).exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(icon_path);
            res.set("ProductName", "XIMOD Translator");
            res.set("FileDescription", "Mod translation tool for Bethesda games");
            res.set("LegalCopyright", "Copyright © 2026 XIMOD Team");
            res.set("CompanyName", "XIMOD");
            if let Err(e) = res.compile() {
                eprintln!("Warning: failed to compile Windows resources: {e}");
            }
        } else {
            println!(
                "cargo:warning=Icon not found at {icon_path}; building without a custom icon."
            );
        }
    }

    println!("cargo:rerun-if-changed=assets/icons/ximod-translator.ico");
    println!("cargo:rerun-if-changed=build.rs");
}
