//! « Empaqueter une traduction… » (M-03): an independent window that builds,
//! from the files a translator exported, the folder and zip a mod manager
//! (Mod Organizer 2, Vortex) installs for the game, with the notes that
//! family of game needs (manual § 19).

use std::path::PathBuf;

use crate::i18n::I18n;
use crate::packaging::{self, PackageSpec};
use eframe::egui;
use ximod_games::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Close,
}

pub struct PackageWindow {
    pub spec: PackageSpec,
    pub description: String,
    message: Option<(String, bool)>,
    last_zip: Option<PathBuf>,
}

impl PackageWindow {
    pub fn new(game: Game, lang: &str, author: &str) -> Self {
        let spec = PackageSpec {
            game: if game == Game::Unknown {
                Game::SkyrimSE
            } else {
                game
            },
            mod_name: String::new(),
            version: "1.0".into(),
            author: author.to_string(),
            lang: lang.to_string(),
            website: String::new(),
            files: Vec::new(),
            dsd_plugin: String::new(),
            out_dir: dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from(".")),
            with_fomod_info: true,
        };
        PackageWindow {
            spec,
            description: String::new(),
            message: None,
            last_zip: None,
        }
    }

    /// Preset from a file the application just exported.
    pub fn with_file(mut self, file: PathBuf) -> Self {
        if let Some(parent) = file.parent() {
            self.spec.out_dir = parent.to_path_buf();
        }
        if self.spec.mod_name.is_empty()
            && let Some(stem) = file.file_stem().and_then(|s| s.to_str())
        {
            self.spec.mod_name = stem.to_string();
        }
        self.spec.files.push(file);
        self
    }

    pub fn body(
        &mut self,
        ui: &mut egui::Ui,
        i18n: &I18n,
        parent: Option<&eframe::Frame>,
    ) -> Outcome {
        let mut outcome = Outcome::None;
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            outcome = Outcome::Close;
        }
        let dialog = |title: String| {
            let mut fd = rfd::FileDialog::new().set_title(title);
            if let Some(p) = parent {
                fd = fd.set_parent(p);
            }
            fd
        };
        egui::Grid::new("pack_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label(i18n.t("pack-game"));
                egui::ComboBox::from_id_salt("pack_game")
                    .selected_text(self.spec.game.display_name())
                    .show_ui(ui, |ui| {
                        for g in Game::ALL {
                            if *g != Game::Unknown {
                                ui.selectable_value(&mut self.spec.game, *g, g.display_name());
                            }
                        }
                    });
                ui.end_row();
                ui.label(i18n.t("pack-mod-name"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.spec.mod_name)
                        .desired_width(320.0)
                        .hint_text(i18n.t("pack-mod-name-hint")),
                );
                ui.end_row();
                ui.label(i18n.t("pack-version"));
                ui.add(egui::TextEdit::singleline(&mut self.spec.version).desired_width(100.0));
                ui.end_row();
                ui.label(i18n.t("pack-author"));
                ui.add(egui::TextEdit::singleline(&mut self.spec.author).desired_width(320.0));
                ui.end_row();
                ui.label(i18n.t("pack-lang"));
                ui.add(egui::TextEdit::singleline(&mut self.spec.lang).desired_width(60.0));
                ui.end_row();
                ui.label(i18n.t("pack-website"));
                ui.add(egui::TextEdit::singleline(&mut self.spec.website).desired_width(320.0));
                ui.end_row();
                ui.label(i18n.t("pack-out-dir"));
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(self.spec.out_dir.display().to_string()).truncate());
                    if ui.button(i18n.t("settings-browse")).clicked()
                        && let Some(d) = dialog(i18n.t("pack-out-dir"))
                            .set_directory(&self.spec.out_dir)
                            .pick_folder()
                    {
                        self.spec.out_dir = d;
                    }
                });
                ui.end_row();
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(i18n.t("pack-files"));
            if ui.button(i18n.t("pack-add-files")).clicked()
                && let Some(files) = dialog(i18n.t("pack-add-files"))
                    .set_directory(&self.spec.out_dir)
                    .pick_files()
            {
                for f in files {
                    if !self.spec.files.contains(&f) {
                        if self.spec.mod_name.is_empty()
                            && let Some(stem) = f.file_stem().and_then(|s| s.to_str())
                        {
                            self.spec.mod_name = stem.to_string();
                        }
                        self.spec.files.push(f);
                    }
                }
            }
        });
        let mut remove: Option<usize> = None;
        for (i, f) in self.spec.files.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button(crate::icons::X).clicked() {
                    remove = Some(i);
                }
                ui.add(egui::Label::new(f.display().to_string()).truncate());
            });
        }
        if let Some(i) = remove {
            self.spec.files.remove(i);
        }
        if self.spec.files.iter().any(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        }) {
            ui.horizontal(|ui| {
                ui.label(i18n.t("pack-dsd-plugin"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.spec.dsd_plugin)
                        .desired_width(220.0)
                        .hint_text(i18n.t("pack-dsd-plugin-hint")),
                );
            });
        }
        ui.separator();
        ui.checkbox(&mut self.spec.with_fomod_info, i18n.t("pack-with-info"));
        if self.spec.with_fomod_info {
            ui.horizontal(|ui| {
                ui.label(i18n.t("pack-description"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.description)
                        .desired_width(420.0)
                        .hint_text(i18n.t("pack-description-hint")),
                );
            });
        }
        ui.separator();
        // Preview.
        let plan = packaging::plan(&self.spec);
        ui.strong(i18n.t_arg("pack-preview", "name", &packaging::package_name(&self.spec)));
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for c in &plan.copies {
                    ui.monospace(format!("  {}", c.dest));
                }
                if self.spec.with_fomod_info {
                    ui.monospace("  fomod/info.xml");
                }
                for u in &plan.unknown {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 60, 60),
                        i18n.t_arg("pack-unknown", "file", &u.display().to_string()),
                    );
                }
            });
        for note in &plan.notes {
            ui.add(
                egui::Label::new(egui::RichText::new(format!("• {}", i18n.t(note))).small()).wrap(),
            );
        }
        ui.separator();
        ui.horizontal(|ui| {
            let can = !plan.copies.is_empty();
            if ui
                .add_enabled(can, egui::Button::new(i18n.t("pack-build")))
                .clicked()
            {
                let desc = if self.description.trim().is_empty() {
                    i18n.t_args(
                        "pack-default-description",
                        &[
                            ("mod", &self.spec.mod_name),
                            ("lang", &self.spec.lang.to_uppercase()),
                        ],
                    )
                } else {
                    self.description.clone()
                };
                match packaging::build(&self.spec, &desc) {
                    Ok((folder, zip)) => {
                        self.message = Some((
                            i18n.t_args(
                                "pack-done",
                                &[
                                    ("folder", &folder.display().to_string()),
                                    ("zip", &zip.display().to_string()),
                                ],
                            ),
                            false,
                        ));
                        self.last_zip = Some(zip);
                    }
                    Err(e) => {
                        self.message =
                            Some((i18n.colon(&i18n.t("pack-error"), &format!("{e:#}")), true))
                    }
                }
            }
            if let Some(z) = &self.last_zip
                && ui.button(i18n.t("pack-open-folder")).clicked()
                && let Some(dir) = z.parent()
            {
                crate::fonts::open_url(&dir.display().to_string());
            }
            if ui.button(i18n.t("btn-close")).clicked() {
                outcome = Outcome::Close;
            }
        });
        if let Some((msg, err)) = &self.message {
            let color = if *err {
                egui::Color32::from_rgb(200, 60, 60)
            } else {
                ui.visuals().text_color()
            };
            ui.add(egui::Label::new(egui::RichText::new(msg).color(color)).wrap());
        }
        outcome
    }
}
