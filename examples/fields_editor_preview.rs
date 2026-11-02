//! Development preview of the fields editor (A-06): opens the window alone
//! and, when `XIMOD_SHOT=<file.png>` is set, saves a screenshot after a few
//! frames then exits (used for headless visual checks under Xvfb).
use eframe::egui;
use ximod_translator::i18n::I18n;
use ximod_translator::ui::fields_editor::FieldsEditor;

struct App {
    i18n: I18n,
    editor: FieldsEditor,
    frames: u32,
    shot: Option<String>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::light());
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = self.editor.body(ui, &self.i18n);
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
                image::save_buffer(
                    path,
                    &buf,
                    img.width() as u32,
                    img.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result<()> {
    let shot = std::env::var("XIMOD_SHOT").ok();
    let game = std::env::var("XIMOD_GAME")
        .ok()
        .map(|g| ximod_translator::games::Game::from_id(&g))
        .unwrap_or(ximod_translator::games::Game::SkyrimSE);
    let data = ximod_translator::config::assets_data_dir();
    let user = std::env::var("XIMOD_USER_FIELDS")
        .ok()
        .map(std::path::PathBuf::from);
    ximod_translator::games::spec::init(&data.join("games"), &data.join("fields"), user.as_deref());
    let mut i18n = I18n::new();
    i18n.set_locale("fra");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "fields editor preview",
        options,
        Box::new(move |_cc| {
            let mut editor = FieldsEditor::new(game);
            if let Some(n) = std::env::var("XIMOD_SELECT")
                .ok()
                .and_then(|v| v.parse().ok())
            {
                editor.select(n);
            }
            Ok(Box::new(App {
                i18n,
                editor,
                frames: 0,
                shot,
            }))
        }),
    )
}
