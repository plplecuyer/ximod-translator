//! Native transparent splash screen implementation
//!
//! Platform-specific implementations for Windows, Linux (X11), and macOS.
//! Creates a truly transparent borderless window that displays the splash image.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Shared progress the splash reads while a background task (game detection +
/// dictionary generation) runs. Lock-free so the splash render loop never
/// blocks the worker.
#[derive(Debug)]
pub struct SplashState {
    frac_bits: AtomicU32,
    done: AtomicBool,
}

impl Default for SplashState {
    fn default() -> Self {
        Self {
            frac_bits: AtomicU32::new(0),
            done: AtomicBool::new(false),
        }
    }
}

impl SplashState {
    pub fn new() -> Self {
        Self::default()
    }
    /// Report progress in `[0, 1]` (worker thread).
    pub fn set_fraction(&self, f: f32) {
        self.frac_bits
            .store(f.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    /// Current progress fraction (splash thread).
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.frac_bits.load(Ordering::Relaxed))
    }
    /// Signal the work is finished, so the splash can dismiss.
    pub fn finish(&self) {
        self.done.store(true, Ordering::Relaxed);
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

/// Progress-bar geometry as fractions of the splash image. `BAR_Y_FRAC` places
/// the bar's top just below the baked-in "Version 1.0" text — adjust it to match
/// your `splash.png`.
pub const BAR_Y_FRAC: f32 = 0.72;
pub const BAR_W_FRAC: f32 = 0.60;

/// Draw a progress bar (centered horizontally, at [`BAR_Y_FRAC`]) onto the
/// already-scaled splash RGBA. Pure and platform-independent so it is unit
/// tested; the backends just re-upload the result each frame.
pub fn composite_progress(img: &mut image::RgbaImage, frac: f32) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    if w < 8 || h < 8 {
        return;
    }
    let bar_w = (w as f32 * BAR_W_FRAC).round() as i64;
    let bar_h = ((h as f32 * 0.014).round() as i64).max(4);
    let x0 = ((w - bar_w) / 2).max(0);
    let y0 = (h as f32 * BAR_Y_FRAC).round() as i64;
    let fill_w = (bar_w as f32 * frac.clamp(0.0, 1.0)).round() as i64;

    let track = image::Rgba([40u8, 44, 52, 220]);
    let border = image::Rgba([210u8, 214, 222, 235]);
    let fill = image::Rgba([90u8, 170, 240, 255]);
    let mut put = |x: i64, y: i64, c: image::Rgba<u8>| {
        if x >= 0 && y >= 0 && x < w && y < h {
            img.put_pixel(x as u32, y as u32, c);
        }
    };
    for yy in 0..bar_h {
        for xx in 0..bar_w {
            put(x0 + xx, y0 + yy, if xx < fill_w { fill } else { track });
        }
    }
    // 1px border around the track.
    for xx in -1..=bar_w {
        put(x0 + xx, y0 - 1, border);
        put(x0 + xx, y0 + bar_h, border);
    }
    for yy in -1..=bar_h {
        put(x0 - 1, y0 + yy, border);
        put(x0 + bar_w, y0 + yy, border);
    }
}

/// Inner fill geometry of the overlay PNGs (fractions of the 1024-wide design):
/// the fill starts at x = 214 and spans 596 px.
const FILL_LEFT_FRAC: f32 = 214.0 / 1024.0;
const FILL_W_FRAC: f32 = 596.0 / 1024.0;

/// Alpha-composite `src` over `dst` in place (straight, non-premultiplied).
fn blend_over(dst: &mut image::Rgba<u8>, src: image::Rgba<u8>) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        *dst = src;
        return;
    }
    let inv = 255 - sa;
    for i in 0..3 {
        dst[i] = ((src[i] as u32 * sa + dst[i] as u32 * inv) / 255) as u8;
    }
    dst[3] = (sa + dst[3] as u32 * inv / 255) as u8;
}

/// Overlay the user-supplied frame (`cadre`) and fill (`barre`, cropped to
/// `fraction`) onto `frame`. The three images share the same (scaled) size, so
/// the bar renders exactly as designed (rounded corners, exact colors).
pub fn apply_overlay(
    frame: &mut image::RgbaImage,
    cadre: &image::RgbaImage,
    barre: &image::RgbaImage,
    fraction: f32,
) {
    let (w, h) = (frame.width(), frame.height());
    if cadre.dimensions() != (w, h) || barre.dimensions() != (w, h) {
        return;
    }
    let cutoff =
        ((FILL_LEFT_FRAC + FILL_W_FRAC * fraction.clamp(0.0, 1.0)) * w as f32).round() as u32;
    for y in 0..h {
        for x in 0..w {
            let mut px = *frame.get_pixel(x, y);
            if x < cutoff {
                blend_over(&mut px, *barre.get_pixel(x, y));
            }
            blend_over(&mut px, *cadre.get_pixel(x, y));
            frame.put_pixel(x, y, px);
        }
    }
}

/// Load the optional bar overlays (`cadre.png`, `barre.png`) sitting next to the
/// splash image. Both must decode for overlay mode to engage.
fn load_overlays(splash: &std::path::Path) -> Option<(image::RgbaImage, image::RgbaImage)> {
    let dir = splash.parent()?;
    let cadre = image::open(dir.join("cadre.png")).ok()?.to_rgba8();
    let barre = image::open(dir.join("barre.png")).ok()?.to_rgba8();
    Some((cadre, barre))
}

/// Pre-scaled `(cadre, barre)` overlays passed to a backend, if available.
type Overlay<'a> = Option<(&'a image::RgbaImage, &'a image::RgbaImage)>;

/// Splash screen configuration
pub struct SplashConfig {
    pub image_path: std::path::PathBuf,
    pub display_duration: Duration,
    pub fade_duration: Duration,
    pub screen_width: f32,
    pub screen_height: f32,
    /// Live progress to display while background work runs. When set, the splash
    /// stays until [`SplashState::finish`] is called (bounded by a hard cap) and
    /// its bar tracks [`SplashState::fraction`]. `None` = fixed-duration splash.
    pub progress: Option<Arc<SplashState>>,
}

/// Hard cap on how long the splash waits for the background work, so a hung task
/// can never keep the app from starting.
const PROGRESS_MAX_WAIT: Duration = Duration::from_secs(600);

/// Show the native transparent splash screen
/// Returns when the splash is complete
pub fn show_splash(config: SplashConfig) -> Result<(), String> {
    // Load the image first to get dimensions
    let image_data = std::fs::read(&config.image_path)
        .map_err(|e| format!("Failed to read splash image: {}", e))?;

    let image = image::load_from_memory(&image_data)
        .map_err(|e| format!("Failed to decode splash image: {}", e))?;

    let rgba = image.to_rgba8();
    let img_width = rgba.width();
    let img_height = rgba.height();

    // Calculate splash size (26% of screen width, max 1024, keep aspect ratio)
    let max_size = (config.screen_width * 0.26).min(1024.0);
    let scale = (max_size / img_width as f32)
        .min(max_size / img_height as f32)
        .min(1.0);
    let splash_width = (img_width as f32 * scale) as u32;
    let splash_height = (img_height as f32 * scale) as u32;

    // Center on screen
    let x = ((config.screen_width - splash_width as f32) / 2.0) as i32;
    let y = ((config.screen_height - splash_height as f32) / 2.0) as i32;

    tracing::info!(
        "Showing splash: {}x{} at ({}, {})",
        splash_width,
        splash_height,
        x,
        y
    );

    // When progress is requested, load the bar overlays (cadre.png + barre.png)
    // next to the splash image and pre-scale them to the splash size.
    let overlays = if config.progress.is_some() {
        load_overlays(&config.image_path).map(|(c, b)| {
            let f = image::imageops::FilterType::Lanczos3;
            (
                image::imageops::resize(&c, splash_width, splash_height, f),
                image::imageops::resize(&b, splash_width, splash_height, f),
            )
        })
    } else {
        None
    };
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    let overlay_ref = overlays.as_ref().map(|(c, b)| (c, b));

    // Call platform-specific implementation
    #[cfg(target_os = "windows")]
    return windows::show_splash_window(
        &rgba,
        overlay_ref,
        splash_width,
        splash_height,
        x,
        y,
        &config,
    );

    #[cfg(target_os = "linux")]
    return linux::show_splash_window(
        &rgba,
        overlay_ref,
        splash_width,
        splash_height,
        x,
        y,
        &config,
    );

    #[cfg(target_os = "macos")]
    return macos::show_splash_window(
        &rgba,
        overlay_ref,
        splash_width,
        splash_height,
        x,
        y,
        &config,
    );

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        tracing::warn!("Native splash not supported on this platform, using fallback");
        wait_out(&config);
        Ok(())
    }
}

/// Block for the splash's lifetime with no window: wait for the background work
/// (bounded) when there is progress, else just the fixed duration.
#[allow(dead_code)]
fn wait_out(config: &SplashConfig) {
    match &config.progress {
        Some(state) => {
            let start = Instant::now();
            while !state.is_done() && start.elapsed() < PROGRESS_MAX_WAIT {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        None => std::thread::sleep(config.display_duration),
    }
}

// ============================================================================
// Windows Implementation
// ============================================================================
#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::mem;
    use std::ptr;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WS_EX_LAYERED: u32 = 0x00080000;
    const WS_EX_TOPMOST: u32 = 0x00000008;
    const WS_EX_TOOLWINDOW: u32 = 0x00000080;
    const WS_POPUP: u32 = 0x80000000;
    const ULW_ALPHA: u32 = 0x00000002;
    const AC_SRC_OVER: u8 = 0x00;
    const AC_SRC_ALPHA: u8 = 0x01;
    const DIB_RGB_COLORS: u32 = 0;

    pub fn show_splash_window(
        rgba: &image::RgbaImage,
        overlay: Overlay,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        config: &SplashConfig,
    ) -> Result<(), String> {
        unsafe {
            // Get module handle
            let hinstance = GetModuleHandleW(ptr::null());
            if hinstance.is_null() {
                return Err("Failed to get module handle".to_string());
            }

            // Register window class
            let class_name: Vec<u16> = "XimodSplash\0".encode_utf16().collect();

            let wc = WNDCLASSEXW {
                cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(splash_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: ptr::null_mut(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: ptr::null_mut(),
            };

            if RegisterClassExW(&wc) == 0 {
                // Class might already be registered, continue
            }

            // Create layered window
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                ptr::null(),
                WS_POPUP,
                x,
                y,
                width as i32,
                height as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            );

            if hwnd.is_null() {
                return Err("Failed to create splash window".to_string());
            }

            // Create compatible DC and bitmap
            let hdc_screen = GetDC(ptr::null_mut());
            let hdc_mem = CreateCompatibleDC(hdc_screen);

            // Create DIB section for the image
            let mut bmi: BITMAPINFO = mem::zeroed();
            bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = width as i32;
            bmi.bmiHeader.biHeight = -(height as i32); // Top-down
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;

            let mut bits: *mut std::ffi::c_void = ptr::null_mut();
            let hbitmap =
                CreateDIBSection(hdc_mem, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);

            if hbitmap.is_null() || bits.is_null() {
                DestroyWindow(hwnd);
                ReleaseDC(ptr::null_mut(), hdc_screen);
                DeleteDC(hdc_mem);
                return Err("Failed to create DIB section".to_string());
            }

            let old_bitmap = SelectObject(hdc_mem, hbitmap);

            // Scale and copy image data (RGBA to BGRA with premultiplied alpha)
            let scaled =
                image::imageops::resize(rgba, width, height, image::imageops::FilterType::Lanczos3);

            let pixel_data = bits as *mut u8;
            for (i, pixel) in scaled.pixels().enumerate() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;

                // Premultiply alpha for UpdateLayeredWindow
                let r_pm = ((r * a) / 255) as u8;
                let g_pm = ((g * a) / 255) as u8;
                let b_pm = ((b * a) / 255) as u8;

                let offset = i * 4;
                *pixel_data.add(offset) = b_pm; // Blue
                *pixel_data.add(offset + 1) = g_pm; // Green
                *pixel_data.add(offset + 2) = r_pm; // Red
                *pixel_data.add(offset + 3) = a as u8; // Alpha
            }

            // Re-fill the DIB bits from an (optionally bar-composited) RGBA frame.
            let refill_bits = |img: &image::RgbaImage| {
                // `data` is the DIB section sized width*height*4 (BGRA, premul).
                let data = bits as *mut u8;
                for (i, pixel) in img.pixels().enumerate() {
                    let (r, g, b, a) = (
                        pixel[0] as u32,
                        pixel[1] as u32,
                        pixel[2] as u32,
                        pixel[3] as u32,
                    );
                    let off = i * 4;
                    *data.add(off) = ((b * a) / 255) as u8;
                    *data.add(off + 1) = ((g * a) / 255) as u8;
                    *data.add(off + 2) = ((r * a) / 255) as u8;
                    *data.add(off + 3) = a as u8;
                }
            };

            // Update layered window with full opacity initially
            update_layered_window(hwnd, hdc_screen, hdc_mem, x, y, width, height, 255);

            // Show window
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            UpdateWindow(hwnd);

            let pump = || {
                // Standard message pump (within the fn's outer unsafe block).
                let mut msg: MSG = mem::zeroed();
                while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            };

            // ---- Display phase: fixed duration, or until the work is done ----
            let start_time = Instant::now();
            loop {
                pump();
                let elapsed = start_time.elapsed();
                let displaying = match &config.progress {
                    Some(s) => {
                        !((s.is_done() && elapsed >= config.display_duration)
                            || elapsed >= PROGRESS_MAX_WAIT)
                    }
                    None => elapsed < config.display_duration,
                };
                if !displaying {
                    break;
                }
                if let Some(s) = &config.progress {
                    let mut frame = scaled.clone();
                    match overlay {
                        Some((cadre, barre)) => {
                            apply_overlay(&mut frame, cadre, barre, s.fraction())
                        }
                        None => composite_progress(&mut frame, s.fraction()),
                    }
                    refill_bits(&frame);
                    update_layered_window(hwnd, hdc_screen, hdc_mem, x, y, width, height, 255);
                }
                std::thread::sleep(Duration::from_millis(33));
            }

            // ---- Fade-out phase ----
            let fade_start = Instant::now();
            loop {
                pump();
                let fe = fade_start.elapsed();
                if fe >= config.fade_duration {
                    break;
                }
                let alpha =
                    ((1.0 - fe.as_secs_f32() / config.fade_duration.as_secs_f32()) * 255.0) as u8;
                update_layered_window(hwnd, hdc_screen, hdc_mem, x, y, width, height, alpha);
                std::thread::sleep(Duration::from_millis(16));
            }

            // Cleanup
            SelectObject(hdc_mem, old_bitmap);
            DeleteObject(hbitmap);
            DeleteDC(hdc_mem);
            ReleaseDC(ptr::null_mut(), hdc_screen);
            DestroyWindow(hwnd);
            UnregisterClassW(class_name.as_ptr(), hinstance);

            Ok(())
        }
    }

    unsafe fn update_layered_window(
        hwnd: HWND,
        hdc_screen: HDC,
        hdc_mem: HDC,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        alpha: u8,
    ) {
        let mut pt_pos = POINT { x, y };
        let mut pt_src = POINT { x: 0, y: 0 };
        let mut size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA,
        };

        // SAFETY: called only from within this module's Windows splash setup
        // with valid device contexts and window handle (edition 2024 requires an
        // explicit unsafe block inside an unsafe fn).
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                hdc_screen,
                &mut pt_pos,
                &mut size,
                hdc_mem,
                &mut pt_src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }

    unsafe extern "system" fn splash_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: standard window-procedure fallback with the parameters the OS
        // passed in.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

// ============================================================================
// Linux (X11) Implementation
// ============================================================================
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::mem;
    use std::os::raw::*;
    use std::ptr;
    use x11::xlib::*;

    pub fn show_splash_window(
        rgba: &image::RgbaImage,
        overlay: Overlay,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        config: &SplashConfig,
    ) -> Result<(), String> {
        unsafe {
            // Open display
            let display = XOpenDisplay(ptr::null());
            if display.is_null() {
                return Err("Failed to open X display".to_string());
            }

            let screen = XDefaultScreen(display);
            let root = XRootWindow(display, screen);
            let visual = XDefaultVisual(display, screen);
            let depth = XDefaultDepth(display, screen);

            // Try to find a 32-bit visual for transparency
            let mut vinfo: XVisualInfo = mem::zeroed();
            let has_argb = XMatchVisualInfo(display, screen, 32, TrueColor, &mut vinfo) != 0;

            let (use_visual, use_depth, colormap) = if has_argb {
                let cmap = XCreateColormap(display, root, vinfo.visual, AllocNone);
                (vinfo.visual, 32, cmap)
            } else {
                tracing::warn!("No 32-bit visual available, transparency may not work");
                (visual, depth, XDefaultColormap(display, screen))
            };

            // Window attributes
            let mut attrs: XSetWindowAttributes = mem::zeroed();
            attrs.colormap = colormap;
            attrs.border_pixel = 0;
            attrs.background_pixel = 0;
            attrs.override_redirect = True;

            let attr_mask = CWColormap | CWBorderPixel | CWBackPixel | CWOverrideRedirect;

            // Create window
            let window = XCreateWindow(
                display,
                root,
                x,
                y,
                width,
                height,
                0,
                use_depth,
                InputOutput as c_uint,
                use_visual,
                attr_mask,
                &mut attrs,
            );

            if window == 0 {
                XCloseDisplay(display);
                return Err("Failed to create X window".to_string());
            }

            // Scale image
            let scaled =
                image::imageops::resize(rgba, width, height, image::imageops::FilterType::Lanczos3);

            // Create XImage
            let mut pixel_data: Vec<u32> = Vec::with_capacity((width * height) as usize);
            for pixel in scaled.pixels() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;
                // ARGB format for X11
                pixel_data.push((a << 24) | (r << 16) | (g << 8) | b);
            }

            let ximage = XCreateImage(
                display,
                use_visual,
                use_depth as c_uint,
                ZPixmap,
                0,
                pixel_data.as_mut_ptr() as *mut c_char,
                width,
                height,
                32,
                0,
            );

            if ximage.is_null() {
                XDestroyWindow(display, window);
                XCloseDisplay(display);
                return Err("Failed to create XImage".to_string());
            }

            // Prevent XDestroyImage from freeing our data
            // We'll handle it ourselves

            // Create GC
            let gc = XCreateGC(display, window, 0, ptr::null_mut());

            // Map window
            XMapRaised(display, window);
            XFlush(display);

            // Draw initial image
            XPutImage(display, window, gc, ximage, 0, 0, 0, 0, width, height);
            XFlush(display);

            let pump = || {
                // Drain pending X events; redraw on Expose (outer unsafe block).
                while XPending(display) > 0 {
                    let mut event: XEvent = mem::zeroed();
                    XNextEvent(display, &mut event);
                    if event.type_ == Expose {
                        XPutImage(display, window, gc, ximage, 0, 0, 0, 0, width, height);
                        XFlush(display);
                    }
                }
            };

            // ---- Display phase: fixed duration, or until the work is done ----
            let start_time = Instant::now();
            loop {
                pump();
                let elapsed = start_time.elapsed();
                let displaying = match &config.progress {
                    Some(s) => {
                        !((s.is_done() && elapsed >= config.display_duration)
                            || elapsed >= PROGRESS_MAX_WAIT)
                    }
                    None => elapsed < config.display_duration,
                };
                if !displaying {
                    break;
                }
                if let Some(s) = &config.progress {
                    let mut frame = scaled.clone();
                    match overlay {
                        Some((cadre, barre)) => {
                            apply_overlay(&mut frame, cadre, barre, s.fraction())
                        }
                        None => composite_progress(&mut frame, s.fraction()),
                    }
                    for (i, pixel) in frame.pixels().enumerate() {
                        let (r, g, b, a) = (
                            pixel[0] as u32,
                            pixel[1] as u32,
                            pixel[2] as u32,
                            pixel[3] as u32,
                        );
                        pixel_data[i] = (a << 24) | (r << 16) | (g << 8) | b;
                    }
                    XPutImage(display, window, gc, ximage, 0, 0, 0, 0, width, height);
                    XFlush(display);
                }
                std::thread::sleep(Duration::from_millis(33));
            }

            // ---- Fade-out phase (modulate the base image's alpha) ----
            let fade_start = Instant::now();
            loop {
                pump();
                let fe = fade_start.elapsed();
                if fe >= config.fade_duration {
                    break;
                }
                let alpha_mult = 1.0 - fe.as_secs_f32() / config.fade_duration.as_secs_f32();
                for (i, pixel) in scaled.pixels().enumerate() {
                    let (r, g, b) = (pixel[0] as u32, pixel[1] as u32, pixel[2] as u32);
                    let a = (pixel[3] as f32 * alpha_mult) as u32;
                    pixel_data[i] = (a << 24) | (r << 16) | (g << 8) | b;
                }
                XPutImage(display, window, gc, ximage, 0, 0, 0, 0, width, height);
                XFlush(display);
                std::thread::sleep(Duration::from_millis(16));
            }

            // Cleanup
            (*ximage).data = ptr::null_mut(); // Prevent double-free
            XDestroyImage(ximage);
            XFreeGC(display, gc);
            XDestroyWindow(display, window);
            if has_argb {
                XFreeColormap(display, colormap);
            }
            XCloseDisplay(display);

            Ok(())
        }
    }
}

// ============================================================================
// macOS Implementation
// ============================================================================
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use cocoa::appkit::*;
    use cocoa::base::*;
    use cocoa::foundation::*;
    use objc::runtime::*;
    use objc::*;

    pub fn show_splash_window(
        rgba: &image::RgbaImage,
        overlay: Overlay,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        config: &SplashConfig,
    ) -> Result<(), String> {
        unsafe {
            // Initialize NSApplication
            let app = NSApp();
            app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);

            // Scale image
            let scaled =
                image::imageops::resize(rgba, width, height, image::imageops::FilterType::Lanczos3);

            // Create NSImage from pixel data
            let ns_image = create_ns_image(&scaled, width, height)?;

            // Create window
            let frame = NSRect::new(
                NSPoint::new(x as f64, y as f64),
                NSSize::new(width as f64, height as f64),
            );

            let style_mask = NSWindowStyleMask::NSBorderlessWindowMask;

            let window: id = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                frame,
                style_mask,
                NSBackingStoreBuffered,
                NO,
            );

            if window.is_null() {
                return Err("Failed to create NSWindow".to_string());
            }

            // Configure window for transparency
            window.setOpaque_(NO);
            window.setBackgroundColor_(NSColor::clearColor(nil));
            window.setLevel_(NSFloatingWindowLevel as i64);
            window.setHasShadow_(NO);

            // Create image view
            let content_rect = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width as f64, height as f64),
            );

            let image_view: id = NSImageView::alloc(nil).initWithFrame_(content_rect);
            image_view.setImage_(ns_image);
            image_view.setImageScaling_(NSImageScaleProportionallyUpOrDown);

            window.setContentView_(image_view);
            window.makeKeyAndOrderFront_(nil);

            let pump = || {
                // Drain the app's pending events each frame (outer unsafe block).
                loop {
                    let event: id = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                        NSEventMaskAny.bits(),
                        NSDate::distantPast(nil),
                        NSDefaultRunLoopMode,
                        YES,
                    );
                    if event.is_null() {
                        break;
                    }
                    app.sendEvent_(event);
                }
            };

            // ---- Display phase: fixed duration, or until the work is done ----
            let start_time = Instant::now();
            loop {
                pump();
                let elapsed = start_time.elapsed();
                let displaying = match &config.progress {
                    Some(s) => {
                        !((s.is_done() && elapsed >= config.display_duration)
                            || elapsed >= PROGRESS_MAX_WAIT)
                    }
                    None => elapsed < config.display_duration,
                };
                if !displaying {
                    break;
                }
                if let Some(s) = &config.progress {
                    let mut frame = scaled.clone();
                    match overlay {
                        Some((cadre, barre)) => {
                            apply_overlay(&mut frame, cadre, barre, s.fraction())
                        }
                        None => composite_progress(&mut frame, s.fraction()),
                    }
                    if let Ok(img) = create_ns_image(&frame, width, height) {
                        image_view.setImage_(img);
                    }
                }
                std::thread::sleep(Duration::from_millis(33));
            }

            // ---- Fade-out phase ----
            let fade_start = Instant::now();
            loop {
                pump();
                let fe = fade_start.elapsed();
                if fe >= config.fade_duration {
                    break;
                }
                let alpha = 1.0 - fe.as_secs_f32() / config.fade_duration.as_secs_f32();
                window.setAlphaValue_(alpha as f64);
                std::thread::sleep(Duration::from_millis(16));
            }

            // Cleanup
            window.close();

            Ok(())
        }
    }

    // Ported Cocoa code: the whole body is a sequence of Objective-C message
    // sends; keep edition-2024's unsafe-in-unsafe-fn lint quiet without rewrapping
    // every call.
    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe fn create_ns_image(
        rgba: &image::RgbaImage,
        width: u32,
        height: u32,
    ) -> Result<id, String> {
        let size = NSSize::new(width as f64, height as f64);
        let image: id = NSImage::alloc(nil).initWithSize_(size);

        if image.is_null() {
            return Err("Failed to create NSImage".to_string());
        }

        // Create bitmap representation
        let bitmap: id = msg_send![class!(NSBitmapImageRep), alloc];

        let bitmap: id = msg_send![
            bitmap,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
            pixelsWide: width as i64
            pixelsHigh: height as i64
            bitsPerSample: 8_i64
            samplesPerPixel: 4_i64
            hasAlpha: YES
            isPlanar: NO
            colorSpaceName: NSDeviceRGBColorSpace
            bytesPerRow: (width * 4) as i64
            bitsPerPixel: 32_i64
        ];

        if bitmap.is_null() {
            return Err("Failed to create NSBitmapImageRep".to_string());
        }

        // Copy pixel data
        let bitmap_data: *mut u8 = msg_send![bitmap, bitmapData];
        if !bitmap_data.is_null() {
            for (i, pixel) in rgba.pixels().enumerate() {
                let offset = i * 4;
                *bitmap_data.add(offset) = pixel[0]; // R
                *bitmap_data.add(offset + 1) = pixel[1]; // G
                *bitmap_data.add(offset + 2) = pixel[2]; // B
                *bitmap_data.add(offset + 3) = pixel[3]; // A
            }
        }

        image.addRepresentation_(bitmap);

        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_progress_draws_fill_and_track() {
        let mut img = image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 0, 0]));
        composite_progress(&mut img, 0.5);
        // Geometry: bar_w=120, x0=40, fill_w=60, y0=72, bar_h=4.
        let filled = img.get_pixel(50, 73); // inside the filled part
        let track = img.get_pixel(110, 73); // inside the empty (track) part
        assert_eq!(filled.0, [90, 170, 240, 255]);
        assert_eq!(track.0, [40, 44, 52, 220]);
        // A pixel well outside the bar stays transparent.
        assert_eq!(img.get_pixel(5, 5).0, [0, 0, 0, 0]);
    }

    #[test]
    fn apply_overlay_crops_bar_to_fraction() {
        let mut frame = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
        let barre = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 180, 200, 255]));
        let mut cadre = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
        cadre.put_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        apply_overlay(&mut frame, &cadre, &barre, 0.5);
        // cutoff ≈ 50: left of it shows the bar, right stays transparent.
        assert_eq!(frame.get_pixel(5, 5).0, [0, 180, 200, 255]);
        assert_eq!(frame.get_pixel(60, 5).0, [0, 0, 0, 0]);
        // The frame pixel is drawn on top.
        assert_eq!(frame.get_pixel(10, 10).0, [255, 255, 255, 255]);
    }

    #[test]
    fn splash_state_roundtrips() {
        let s = SplashState::new();
        assert!(!s.is_done());
        s.set_fraction(0.42);
        assert!((s.fraction() - 0.42).abs() < 1e-6);
        s.finish();
        assert!(s.is_done());
    }
}
