//! Development preview of the « Lignes exportées » dialog (S-04): with
//! `XIMOD_SHOT=<file.png>` a screenshot is saved after a few frames (headless
//! checks under Xvfb); `XIMOD_INCLUDE=1` pre-checks « par similarité ».
use eframe::egui;
use ximod_translator::export::{ExportCounts, ExportPolicy};
use ximod_translator::i18n::I18n;
use ximod_translator::status::Status;
use ximod_translator::ui::export_dialog;

struct App {
    i18n: I18n,
    policy: ExportPolicy,
    frames: u32,
    shot: Option<String>,
    esl: bool,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::light());
        let mut counts = ExportCounts {
            validated: 812,
            total: 812,
            excluded: 210,
            ..Default::default()
        };
        if self.policy.include[Status::BySimilarity.index()] {
            counts.by_status[Status::BySimilarity.index()] = 210;
            counts.total = 1022;
            counts.excluded = 0;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let mode = if std::env::var("XIMOD_OVERRIDE").is_ok() {
                "Patch override"
            } else {
                "STRINGS"
            };
            egui::Window::new(format!("{} — {mode}", self.i18n.t("export-title")))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(export_dialog::WIDTH)
                .show(ctx, |ui| {
                    let info =
                        std::env::var("XIMOD_OVERRIDE")
                            .ok()
                            .map(|_| export_dialog::OverrideInfo {
                                localized: true,
                                game: "Starfield".into(),
                                records: 812,
                                unresolved: 3,
                                esl_eligible: true,
                            });
                    let _ = export_dialog::body(
                        ui,
                        &self.i18n,
                        &mut self.policy,
                        &counts,
                        info.as_ref(),
                        &mut self.esl,
                        Default::default(),
                    );
                });
            let _ = ui;
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
    let mut i18n = I18n::new();
    i18n.set_locale(&std::env::var("XIMOD_LOCALE").unwrap_or_else(|_| "fra".into()));
    let mut policy = ExportPolicy::validated_only();
    if std::env::var("XIMOD_INCLUDE").is_ok() {
        policy = policy.with(Status::BySimilarity, true);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "export dialog preview",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App {
                i18n,
                policy,
                frames: 0,
                shot,
                esl: true,
            }))
        }),
    )
}
