//! egui GUI for SWF Explorer: open a SWF, edit translations in a table, and save
//! a translated SWF. Mirrors the look of BSA Explorer / ESM Explorer.

use std::path::PathBuf;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::exchange::{self, Row};
use crate::swf::Swf;
use crate::text;

#[derive(Default)]
pub struct SwfTextApp {
    path: Option<PathBuf>,
    header: String,
    rows: Vec<Row>,
    filter: String,
    only_reinjectable: bool,
    hide_keys: bool,
    hide_code: bool,
    status: Option<String>,
    error: Option<String>,
}

impl SwfTextApp {
    /// Optionally open a SWF passed on the command line.
    pub fn with_initial(path: Option<PathBuf>) -> Self {
        let mut app = Self::default();
        if let Some(p) = path {
            app.load(p);
        }
        app
    }

    fn load(&mut self, path: PathBuf) {
        self.error = None;
        self.status = None;
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| {
                Swf::parse(&b)
                    .map_err(|e| format!("{e:#}"))
                    .map(|swf| (swf, b.len()))
            }) {
            Ok((swf, size)) => {
                let entries = text::extract(&swf);
                let reinj = entries.iter().filter(|e| e.reinjectable).count();
                let keys = entries.iter().filter(|e| e.scaleform_key).count();
                self.header = format!(
                    "{} — {} v{}, {} tags · {} textes ({} réinjectables, {} clés $) · {} Ko",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    swf.kind_label(),
                    swf.version,
                    swf.tags.len(),
                    entries.len(),
                    reinj,
                    keys,
                    size / 1024,
                );
                self.rows = exchange::rows_from(&entries);
                self.path = Some(path);
            }
            Err(e) => self.error = Some(format!("Ouverture impossible : {e}")),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Flash / Scaleform", &["swf", "gfx", "cfx"])
            .pick_file()
        {
            self.load(p);
        }
    }

    fn export_dialog(&mut self) {
        let default = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}_textes.csv"))
            .unwrap_or_else(|| "textes.csv".into());
        if let Some(p) = rfd::FileDialog::new()
            .set_file_name(default)
            .add_filter("CSV", &["csv"])
            .add_filter("JSON", &["json"])
            .save_file()
        {
            match exchange::write(&p, &self.rows) {
                Ok(_) => self.status = Some(format!("Textes exportés → {}", p.display())),
                Err(e) => self.error = Some(format!("Export impossible : {e:#}")),
            }
        }
    }

    fn import_dialog(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("Traductions", &["csv", "json"])
            .pick_file()
        else {
            return;
        };
        match exchange::read(&p) {
            Ok(imported) => {
                let mut n = 0usize;
                for imp in &imported {
                    if imp.traduction.is_empty() {
                        continue;
                    }
                    if let Some(r) = self.rows.iter_mut().find(|r| r.id == imp.id) {
                        r.traduction = imp.traduction.clone();
                        n += 1;
                    }
                }
                self.status = Some(format!(
                    "{n} traduction(s) importée(s) depuis {}",
                    p.display()
                ));
            }
            Err(e) => self.error = Some(format!("Import impossible : {e:#}")),
        }
    }

    /// Re-parse the original SWF, inject the filled translations, save a new SWF.
    fn save_dialog(&mut self) {
        let Some(src) = self.path.clone() else {
            self.error = Some("Ouvrez d’abord un SWF.".into());
            return;
        };
        let edits = exchange::edits_map(&self.rows);
        if edits.is_empty() {
            self.error = Some("Aucune traduction saisie à réinjecter.".into());
            return;
        }
        // Keep the source extension (.swf / .gfx / .cfx) so the output stays the
        // same container the game expects.
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("swf")
            .to_lowercase();
        let default = src
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}_fr.{ext}"))
            .unwrap_or_else(|| format!("traduit.{ext}"));
        let Some(out) = rfd::FileDialog::new()
            .set_file_name(default)
            .add_filter("Flash / Scaleform", &["swf", "gfx", "cfx"])
            .save_file()
        else {
            return;
        };
        let result = (|| -> anyhow::Result<(usize, usize)> {
            let bytes = std::fs::read(&src)?;
            let mut swf = Swf::parse(&bytes)?;
            let rep = text::inject(&mut swf, &edits);
            let out_bytes = swf.write()?;
            std::fs::write(&out, &out_bytes)?;
            Ok((rep.applied, rep.skipped))
        })();
        match result {
            Ok((applied, skipped)) => {
                self.status = Some(format!(
                    "{applied} texte(s) réinjecté(s), {skipped} ignoré(s) → {}",
                    out.display()
                ));
            }
            Err(e) => self.error = Some(format!("Enregistrement impossible : {e:#}")),
        }
    }

    fn visible_rows(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| !self.only_reinjectable || r.reinjectable)
            .filter(|(_, r)| !self.hide_keys || !r.cle_scaleform)
            .filter(|(_, r)| !self.hide_code || r.categorie != "code")
            .filter(|(_, r)| {
                needle.is_empty()
                    || r.texte.to_lowercase().contains(&needle)
                    || r.traduction.to_lowercase().contains(&needle)
                    || r.id.contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

impl SwfTextApp {
    /// Menu entries for this tool, placed inside the shell's unified menu bar.
    pub fn menu_contents(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.menu_button("Fichier", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if ui.button("Ouvrir SWF…").clicked() {
                ui.close_menu();
                self.open_dialog();
            }
            ui.separator();
            let has = !self.rows.is_empty();
            if ui
                .add_enabled(has, egui::Button::new("Exporter les textes…"))
                .clicked()
            {
                ui.close_menu();
                self.export_dialog();
            }
            if ui
                .add_enabled(has, egui::Button::new("Importer des traductions…"))
                .clicked()
            {
                ui.close_menu();
                self.import_dialog();
            }
            ui.separator();
            if ui
                .add_enabled(has, egui::Button::new("Enregistrer le SWF traduit…"))
                .clicked()
            {
                ui.close_menu();
                self.save_dialog();
            }
        });
        ui.menu_button("À propos", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.label("SWF Explorer");
            ui.label("Textes traduisibles des SWF/GFX Flash & Scaleform");
            ui.label("Outil de XIMOD Translator");
        });
    }

    /// Per-frame UI below the shell's unified menu bar.
    pub fn body(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(0xB0, 0x24, 0x18), e);
            } else if let Some(s) = &self.status {
                ui.label(s);
            } else {
                ui.label("Ouvrez un SWF, saisissez les traductions, puis « Enregistrer le SWF traduit… ».");
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.rows.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Glissez un .swf ici ou utilisez « Ouvrir SWF… ».");
                });
                self.handle_drop(ctx);
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.header);
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Filtre :");
                ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(220.0));
                if !self.filter.is_empty() && ui.button("×").clicked() {
                    self.filter.clear();
                }
                ui.checkbox(&mut self.only_reinjectable, "Réinjectables seulement");
                ui.checkbox(&mut self.hide_keys, "Masquer les clés $");
                ui.checkbox(&mut self.hide_code, "Masquer le code (AS)");
            });
            ui.add_space(4.0);

            let visible = self.visible_rows();
            let row_h = 24.0;
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(60.0)) // id
                .column(Column::auto().at_least(70.0)) // type
                .column(Column::auto().at_least(64.0)) // catégorie
                .column(Column::auto().at_least(24.0)) // réinj
                .column(Column::auto().at_least(24.0)) // $
                .column(Column::remainder().at_least(160.0).clip(true)) // texte
                .column(Column::remainder().at_least(200.0)) // traduction
                .header(20.0, |mut h| {
                    h.col(|ui| {
                        ui.strong("id");
                    });
                    h.col(|ui| {
                        ui.strong("Type");
                    });
                    h.col(|ui| {
                        ui.strong("Cat.");
                    });
                    h.col(|ui| {
                        ui.strong("Réinj.");
                    });
                    h.col(|ui| {
                        ui.strong("$");
                    });
                    h.col(|ui| {
                        ui.strong("Texte original");
                    });
                    h.col(|ui| {
                        ui.strong("Traduction");
                    });
                })
                .body(|body| {
                    body.rows(row_h, visible.len(), |mut row| {
                        let idx = visible[row.index()];
                        let (id, kind, cat, reinj, key, texte) = {
                            let r = &self.rows[idx];
                            (
                                r.id.clone(),
                                r.kind.clone(),
                                r.categorie.clone(),
                                r.reinjectable,
                                r.cle_scaleform,
                                r.texte.clone(),
                            )
                        };
                        row.col(|ui| {
                            ui.label(&id);
                        });
                        row.col(|ui| {
                            ui.label(&kind);
                        });
                        row.col(|ui| {
                            let color = match cat.as_str() {
                                "code" => egui::Color32::from_rgb(0xB0, 0x24, 0x18),
                                "texte" => egui::Color32::from_rgb(0x2E, 0x7D, 0x32),
                                _ => egui::Color32::GRAY,
                            };
                            ui.colored_label(color, &cat);
                        });
                        row.col(|ui| {
                            ui.label(if reinj { "✔" } else { "—" });
                        });
                        row.col(|ui| {
                            ui.label(if key { "$" } else { "" });
                        });
                        row.col(|ui| {
                            // `truncate()` already shows the full text as a tooltip when
                            // clipped — don't add a second `on_hover_text` (double tooltip).
                            ui.add(egui::Label::new(&texte).truncate());
                        });
                        row.col(|ui| {
                            if reinj {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.rows[idx].traduction)
                                        .hint_text("traduction…")
                                        .desired_width(f32::INFINITY),
                                );
                            } else {
                                ui.weak("(non réinjectable en V1)");
                            }
                        });
                    });
                });
        });

        self.handle_drop(ctx);
    }
}

impl SwfTextApp {
    fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(p) = dropped.into_iter().find(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e.to_lowercase().as_str(), "swf" | "gfx" | "cfx"))
                .unwrap_or(false)
        }) {
            self.load(p);
        }
    }
}
