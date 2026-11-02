//! XIMOD Translator — cross-platform mod translation tool for Bethesda games.
//!
//! Thin launcher: builds the native window (icon, size) and hands control to
//! [`ximod_translator::ui::XimodApp`]. All subsystems live in the library crate.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use eframe::egui;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use ximod_translator::{
    APP_NAME, APP_VERSION, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH, ScreenInfo, config,
    get_splash_path, icon, splash, ui,
};

fn main() -> eframe::Result<()> {
    // Where everything the user produces is written (E-05): decided once, before
    // the configuration is read — and before the log, which goes to
    // `logs/ximod.log` there (the Windows build has no console).
    let paths = config::paths::init();
    let log_file = (!paths.read_only)
        .then(|| std::fs::File::create(paths.logs_dir.join("ximod.log")).ok())
        .flatten();
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "ximod_translator=info".into())
    };
    let registry = tracing_subscriber::registry().with(tracing_subscriber::fmt::layer());
    match log_file {
        Some(f) => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(f)),
            )
            .with(filter())
            .init(),
        None => registry.with(filter()).init(),
    }

    tracing::info!("{APP_NAME} v{APP_VERSION} starting…");
    tracing::info!(
        "configuration: {} (mode {}{})",
        paths.config_dir.display(),
        paths.mode.as_str(),
        if paths.read_only { ", read-only" } else { "" }
    );

    let cfg = config::AppConfig::load().unwrap_or_default();
    // Files to open (« ouvrir avec », drag onto the executable).
    // `--game <id>` presets the toolbar « Jeu » list (a shortcut can then open
    // files directly, since opening requires a game); the rest are files.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut game: Option<String> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--fomod" {
            // The FOMOD window on that folder or archive (F-03).
            if let Some(p) = args.next() {
                files.push(std::path::PathBuf::from(p));
            }
        } else if a == "--game" {
            game = args.next().map(|g| g.to_string_lossy().into_owned());
        } else if let Some(g) = a.to_str().and_then(|s| s.strip_prefix("--game=")) {
            game = Some(g.to_string());
        } else {
            files.push(std::path::PathBuf::from(a));
        }
    }
    let window_width = if cfg.window_width > 0.0 {
        cfg.window_width
    } else {
        DEFAULT_WINDOW_WIDTH
    };
    let window_height = if cfg.window_height > 0.0 {
        cfg.window_height
    } else {
        DEFAULT_WINDOW_HEIGHT
    };

    // Detect the PRIMARY screen so the splash and every window can be centered
    // on it, even in a dual-monitor setup.
    let screen = detect_primary_screen();

    // Startup work (detect installed games + (re)build game dictionaries from
    // their STRINGS) runs on a background thread and is visualized by the splash
    // progress bar. It runs even when the splash is disabled.
    let splash_state = std::sync::Arc::new(splash::SplashState::new());
    let worker_state = splash_state.clone();
    let worker = std::thread::spawn(move || {
        ximod_translator::run_startup_generation(&worker_state);
    });

    // Native transparent splash, shown ALONE (no window) before the main window,
    // then dissolved. Its progress bar tracks the background work above.
    if cfg.splash_screen_seconds > 0
        && let Some(path) = get_splash_path()
    {
        let splash_config = splash::SplashConfig {
            image_path: path,
            display_duration: Duration::from_secs(cfg.splash_screen_seconds as u64),
            fade_duration: Duration::from_millis(500),
            screen_width: screen.width,
            screen_height: screen.height,
            progress: Some(splash_state.clone()),
        };
        match splash::show_splash(splash_config) {
            Ok(()) => tracing::info!("splash done"),
            Err(e) => tracing::warn!("splash error (continuing): {e}"),
        }
    }

    // Make sure the background work finished before building the UI, so the
    // freshly generated dictionaries and updated Config.ini are on disk.
    let _ = worker.join();

    // Restore the last position, or center on the primary screen on first launch
    // (a fresh Config.ini has window_x/y = -1).
    let (win_x, win_y) = if cfg.window_x >= 0.0 && cfg.window_y >= 0.0 {
        (cfg.window_x, cfg.window_y)
    } else {
        (
            ((screen.width - window_width) / 2.0).max(0.0),
            ((screen.height - window_height) / 2.0).max(0.0),
        )
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([window_width, window_height])
        .with_min_inner_size([800.0, 600.0])
        .with_position([win_x, win_y])
        .with_title(format!("{APP_NAME} v{APP_VERSION}"));
    if let Some(app_icon) = icon::create_viewport_icon() {
        viewport = viewport.with_icon(app_icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(
                ui::XimodApp::new(cc, screen)
                    .with_startup_game(game)
                    .with_startup_files(files),
            ))
        }),
    )
}

/// Detect the primary screen's dimensions (best-effort, per platform). Falls
/// back to 1920×1080 when detection fails.
fn detect_primary_screen() -> ScreenInfo {
    let mut info = ScreenInfo::default();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             [System.Windows.Forms.Screen]::PrimaryScreen.Bounds | Format-List Width,Height",
        ]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        if let Ok(output) = cmd.output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("Width") {
                    if let Some(val) = v.split(':').nth(1) {
                        info.width = val.trim().parse().unwrap_or(info.width);
                    }
                } else if let Some(v) = line.strip_prefix("Height") {
                    if let Some(val) = v.split(':').nth(1) {
                        info.height = val.trim().parse().unwrap_or(info.height);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("xrandr").args(["--current"]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains(" connected")
                    && (line.contains("primary") || !stdout.contains("primary"))
                {
                    if let Some(res) = line
                        .split_whitespace()
                        .find(|s| s.contains('x') && s.contains('+'))
                    {
                        let parts: Vec<&str> = res.split(['x', '+']).collect();
                        if parts.len() >= 2 {
                            info.width = parts[0].parse().unwrap_or(info.width);
                            info.height = parts[1].parse().unwrap_or(info.height);
                        }
                    }
                    break;
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("system_profiler")
            .args(["SPDisplaysDataType"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Resolution:") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 {
                        info.width = parts[0].parse().unwrap_or(info.width);
                        info.height = parts[2].parse().unwrap_or(info.height);
                    }
                    break;
                }
            }
        }
    }

    tracing::info!("primary screen: {}x{}", info.width, info.height);
    info
}
