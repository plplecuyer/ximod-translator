//! Development preview of the « Traduire un FOMOD » window (M-02): loads
//! `tests/fixtures/fomod`; with `XIMOD_SHOT=<file.png>` a screenshot is saved
//! after a few frames (headless checks under Xvfb).
use eframe::egui;
use ximod_translator::i18n::I18n;
use ximod_translator::ui::fomod_window::FomodWindow;

struct App {
    i18n: I18n,
    win: FomodWindow,
    frames: u32,
    shot: Option<String>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::light());
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = self.win.body(ui, &self.i18n, Some(frame));
        });
        self.frames += 1;
        if let Some(path) = &self.shot {
            if self.frames == 6 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = image {
                let buf: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                image::save_buffer(path, &buf, img.width() as u32, img.height() as u32, image::ColorType::Rgba8).unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result<()> {
    let shot = std::env::var("XIMOD_SHOT").ok();
    let mut i18n = I18n::new();
    i18n.set_locale(&std::env::var("XIMOD_LOCALE").unwrap_or_else(|_| "fra".into()));
    let mut win = FomodWindow::new("fr");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fomod");
    win.preload(&dir, &i18n);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native("fomod preview", options, Box::new(move |_cc| Ok(Box::new(App { i18n, win, frames: 0, shot }))))
}
