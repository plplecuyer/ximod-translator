//! Outils ▸ Traduire l'interface… (E-10): the window driver of the
//! interface translation editor — loading the three files, saving
//! `main.ftl` (in `assets/locales` or, when it is not writable, in
//! `<config>/locales`, D-35), the reference data, the package and the mail.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;

use super::{XimodApp, read_geometry};
use crate::ui::translation_editor::{self as te, Pending, TrAction, TrEditor};

impl XimodApp {
    /// Open the editor (or bring it forward).
    pub(crate) fn open_translation_editor(&mut self) {
        if self.tr.is_some() {
            self.tr_needs_center = true;
            return;
        }
        let target = self.i18n.current_locale().to_string();
        let displayed = if target == "eng" { "fra" } else { "eng" }.to_string();
        let mut ed = TrEditor::default();
        ed.country = self.config.country.clone();
        ed.target = target;
        ed.displayed = displayed;
        ed.author = self.config.translator.clone();
        self.tr_load(&mut ed, None);
        self.tr = Some(Box::new(ed));
        self.tr_needs_center = true;
    }

    /// The folder the language is saved to: the user's copy when it exists,
    /// else `assets/locales` when writable, else `<config>/locales`.
    fn tr_save_dir(&self, code: &str) -> PathBuf {
        te::choose_save_dir(
            self.i18n.locales_dir(),
            self.i18n.user_locales_dir(),
            code,
            te::dir_writable,
        )
    }

    /// Read the reference, displayed and target files into the editor.
    /// `keep` carries translations typed before a change of the displayed
    /// language.
    fn tr_load(&self, ed: &mut TrEditor, keep: Option<HashMap<String, String>>) {
        let read = |code: &str| {
            if code.is_empty() {
                return String::new();
            }
            std::fs::read_to_string(self.i18n.locale_file(code)).unwrap_or_default()
        };
        let reference = read("eng");
        let displayed = read(&ed.displayed);
        let mut target = read(&ed.target);
        if let Some(values) = &keep {
            // Typed translations take the place of the file's.
            let mut extra = String::new();
            for (k, v) in values {
                extra.push_str(&format!("{k} = {v}\n"));
            }
            let mut msgs = te::messages(&target);
            for (k, v) in te::messages(&extra) {
                msgs.insert(k, v);
            }
            target = msgs
                .iter()
                .map(|(k, (v, c))| {
                    let mut s = format!("{k} = {v}\n");
                    for l in c {
                        s.push_str(l);
                        s.push('\n');
                    }
                    s
                })
                .collect();
        } else {
            let meta = te::parse_metadata(&read(&ed.target));
            ed.font = meta.get("font").cloned().unwrap_or_default();
            if let Some(a) = meta.get("author").filter(|a| !a.is_empty()) {
                ed.author = a.clone();
            }
            self.tr_refresh_meta(ed, &meta);
            ed.dirty = false;
        }
        ed.entries = te::build_entries(&reference, &displayed, &target);
        ed.save_path = self.tr_save_dir(&ed.target).join("main.ftl");
        ed.refresh_view(true);
    }

    /// The endonyms and the font of a (country, language) selection: the
    /// country's from `countryEndonyms.tsv` then Countries.json, the
    /// language's from the file header then Languages.json.
    fn tr_refresh_meta(&self, ed: &mut TrEditor, meta: &HashMap<String, String>) {
        if ed.font.is_empty() {
            ed.font = self
                .i18n
                .font_for(&ed.target)
                .unwrap_or_default()
                .to_string();
        }
        let lang_dir = self.i18n.locale_file(&ed.target);
        let lang_dir = lang_dir.parent().map(PathBuf::from).unwrap_or_default();
        let tsv = te::read_tsv_country_endonym(&lang_dir, &ed.country);
        let exact = self
            .countries
            .by_a3(&ed.country)
            .and_then(|c| c.languages.iter().find(|l| l.iso3 == ed.target))
            .map(|l| l.country_endonym.clone())
            .filter(|s| !s.is_empty());
        match tsv.or(exact) {
            Some(v) => {
                ed.endonym = v;
                ed.endonym_authoritative = true;
            }
            None => {
                ed.endonym = self
                    .countries
                    .endonym_for(&ed.country, &ed.target)
                    .unwrap_or("")
                    .to_string();
                ed.endonym_authoritative = false;
            }
        }
        ed.lang_endonym = match meta.get("langname") {
            Some(v) if !v.is_empty() => v.clone(),
            _ if ed.target.is_empty() => String::new(),
            _ => self.i18n.display_name(&ed.target),
        };
        ed.font_loaded = ed.font.clone();
        ed.endonym_loaded = ed.endonym.clone();
        ed.lang_endonym_loaded = ed.lang_endonym.clone();
    }

    /// The editor's country chosen in the flags window.
    pub(crate) fn tr_choose_country(&mut self, a3: String) {
        let Some(mut ed) = self.tr.take() else { return };
        ed.country = a3;
        let langs = self.country_languages.languages_for(&ed.country).to_vec();
        if !langs.contains(&ed.target) {
            let first = langs
                .iter()
                .find(|l| self.i18n.available_locales().contains(l))
                .or(langs.first())
                .cloned();
            if let Some(first) = first {
                if ed.dirty {
                    ed.confirm = Some(Pending::Target(first));
                } else {
                    ed.target = first;
                    self.tr_load(&mut ed, None);
                }
            }
        } else {
            let meta = te::parse_metadata(
                &std::fs::read_to_string(self.i18n.locale_file(&ed.target)).unwrap_or_default(),
            );
            let font = ed.font.clone();
            self.tr_refresh_meta(&mut ed, &meta);
            ed.font = font;
        }
        self.tr = Some(ed);
    }

    /// Ask for a font inside `assets/fonts` (relative path kept).
    fn tr_pick_font(&mut self, ed: &mut TrEditor) {
        let Some(root) = crate::fonts::fonts_dir() else {
            ed.message = self.i18n.t("tr-font-dir-missing");
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(self.i18n.t("dialog-fonts"), &["ttf", "otf", "ttc"])
            .set_directory(&root)
            .pick_file()
        else {
            return;
        };
        let rel = path.canonicalize().ok().and_then(|abs| {
            root.canonicalize()
                .ok()
                .and_then(|r| abs.strip_prefix(r).ok().map(PathBuf::from))
        });
        match rel {
            Some(r) => {
                ed.font = r.to_string_lossy().replace('\\', "/");
                ed.dirty = true;
                ed.message.clear();
            }
            None => ed.message = self.i18n.t("tr-font-outside"),
        }
    }

    /// Register the language's font as the preview family (the endonyms and
    /// the translations are drawn with it).
    fn tr_apply_preview_font(&mut self, ctx: &egui::Context, font: &str) {
        if self.tr_preview_font.as_deref() == Some(font) {
            return;
        }
        let all = crate::data::LanguagesData::load().all_fonts();
        let mut defs = crate::fonts::build_font_definitions_with_preview(&all, Some(font));
        self.book_fonts = crate::fonts::load_book_fonts(&mut defs);
        ctx.set_fonts(defs);
        self.tr_preview_font = Some(font.to_string());
    }

    /// Write the endonyms and the font into Languages.json / Countries.json
    /// when the user changed them (structure and unknown fields kept).
    fn tr_update_reference_data(&mut self, ed: &TrEditor) -> bool {
        let lang = ed.target.clone();
        let mut written = false;
        if let Some(path) = crate::data::find_data_file("Languages.json")
            && let Ok(mut doc) = std::fs::read_to_string(&path)
                .map_err(|_| ())
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|_| ()))
        {
            let mut changed = false;
            if let Some(arr) = doc.get_mut("languages").and_then(|v| v.as_array_mut())
                && let Some(entry) = arr
                    .iter_mut()
                    .find(|e| e.get("iso639_3").and_then(|v| v.as_str()) == Some(&lang))
            {
                if ed.lang_endonym != ed.lang_endonym_loaded
                    && !ed.lang_endonym.is_empty()
                    && entry.get("name").and_then(|v| v.as_str()) != Some(ed.lang_endonym.as_str())
                {
                    entry["name"] = serde_json::Value::String(ed.lang_endonym.clone());
                    changed = true;
                }
                if ed.font != ed.font_loaded
                    && !ed.font.is_empty()
                    && entry.get("font").and_then(|v| v.as_str()) != Some(ed.font.as_str())
                {
                    entry["font"] = serde_json::Value::String(ed.font.clone());
                    changed = true;
                }
            }
            if changed && let Ok(s) = serde_json::to_string_pretty(&doc) {
                written |= std::fs::write(&path, s + "\n").is_ok();
            }
        }
        if (ed.endonym_authoritative || ed.endonym != ed.endonym_loaded)
            && !ed.country.is_empty()
            && !ed.endonym.is_empty()
            && let Some(path) = crate::data::find_data_file("Countries.json")
            && let Ok(mut doc) = std::fs::read_to_string(&path)
                .map_err(|_| ())
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|_| ()))
        {
            let mut changed = false;
            if let Some(country) = doc
                .get_mut("countries")
                .and_then(|v| v.as_array_mut())
                .and_then(|a| {
                    a.iter_mut()
                        .find(|c| c.get("a3").and_then(|v| v.as_str()) == Some(ed.country.as_str()))
                })
                && let Some(langs) = country.get_mut("languages").and_then(|v| v.as_array_mut())
            {
                match langs
                    .iter_mut()
                    .find(|l| l.get("iso639_3").and_then(|v| v.as_str()) == Some(lang.as_str()))
                {
                    Some(l) => {
                        if l.get("countryEndonym").and_then(|v| v.as_str())
                            != Some(ed.endonym.as_str())
                        {
                            l["countryEndonym"] = serde_json::Value::String(ed.endonym.clone());
                            changed = true;
                        }
                    }
                    None => {
                        langs.push(serde_json::json!({
                            "iso639_3": lang,
                            "countryEndonym": ed.endonym,
                        }));
                        changed = true;
                    }
                }
            }
            if changed && let Ok(s) = serde_json::to_string_pretty(&doc) {
                written |= std::fs::write(&path, s + "\n").is_ok();
            }
        }
        if written {
            self.countries = crate::data::CountriesData::load();
            self.i18n.reload_languages();
        }
        written
    }

    /// Save the language (R-E10-04).
    fn tr_save(&mut self, ed: &mut TrEditor) {
        if ed.target.is_empty() || !self.i18n.has_language(&ed.target) {
            ed.message = self.i18n.t("tr-lang-not-iso");
            return;
        }
        let read =
            |code: &str| std::fs::read_to_string(self.i18n.locale_file(code)).unwrap_or_default();
        let reference = read("eng");
        let target = read(&ed.target);
        let meta = te::Meta {
            language: ed.target.clone(),
            langname: ed.lang_endonym.clone(),
            font: ed.font.clone(),
            author: ed.author.clone(),
        };
        let text = te::render_file(&reference, &target, &ed.entries, &meta);
        let dir = self.tr_save_dir(&ed.target);
        let path = dir.join("main.ftl");
        let res = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, text));
        if let Err(e) = res {
            ed.message = self.i18n.t_arg("err-save", "error", &e.to_string());
            self.notify_err(ed.message.clone());
            return;
        }
        let data = self.tr_update_reference_data(ed);
        if !ed.country.is_empty() && !ed.endonym.is_empty() {
            let _ = te::upsert_tsv(&dir, &ed.country, &ed.target, &ed.endonym);
        }
        // The language appears at once; the current one is reloaded.
        self.i18n.rescan();
        self.comment_cache.clear();
        let (done, _) = ed.progress();
        let mut msg = self.i18n.t_num("tr-saved", done as i64);
        if data {
            msg = self.i18n.dash(&msg, &self.i18n.t("tr-data-updated"));
        }
        ed.message = self.i18n.dash(
            &msg,
            &self
                .i18n
                .t_arg("tr-saved-where", "path", &path.display().to_string()),
        );
        ed.save_path = path;
        ed.dirty = false;
        ed.font_loaded = ed.font.clone();
        ed.endonym_loaded = ed.endonym.clone();
        ed.lang_endonym_loaded = ed.lang_endonym.clone();
        self.notify_ok(msg);
    }

    /// Envoyer… : the zip in the configuration folder, the folder shown and
    /// the pre-filled mail opened (R-E10-05).
    fn tr_send(&mut self, ed: &mut TrEditor) {
        let lang_dir = ed.save_path.parent().map(PathBuf::from).unwrap_or_default();
        if !lang_dir.join("main.ftl").is_file() {
            ed.message = self.i18n.t("tr-send-unsaved");
            return;
        }
        let out_dir = crate::config::paths::get().config_dir.clone();
        let zip = out_dir.join(format!("ximod-translator_{}.zip", ed.target));
        match std::fs::create_dir_all(&out_dir)
            .and_then(|_| te::build_zip(&lang_dir, &ed.target, &zip))
        {
            Ok(()) => {
                crate::fonts::open_path(&out_dir);
                crate::fonts::open_url(&te::mailto(&ed.lang_endonym, &ed.target, &ed.author, &zip));
                ed.message = self.i18n.t_arg(
                    "tr-sent",
                    "file",
                    &zip.file_name().unwrap_or_default().to_string_lossy(),
                );
            }
            Err(e) => ed.message = self.i18n.t_arg("err-generic", "error", &e.to_string()),
        }
    }

    /// The editor window (independent viewport, `[Window] TrWin*`).
    pub(crate) fn translation_window(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.tr.take() else { return };
        self.tr_apply_preview_font(ctx, &ed.font.clone());
        let id = egui::ViewportId::from_hash_of("ximod-translate-ui");
        let [sx, sy, sw, sh] = self.config.tr_win;
        let just_opened = self.tr_needs_center;
        let place = just_opened.then(|| {
            (
                self.open_pos_cascade(sx, sy),
                [
                    if sw > 0.0 { sw } else { 1100.0 },
                    if sh > 0.0 { sh } else { 720.0 },
                ],
            )
        });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("{} — {}", self.i18n.t("tr-title"), crate::APP_NAME))
            .with_min_inner_size([820.0, 520.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        // What the body needs.
        let flag = self
            .countries
            .flag_for(&ed.country)
            .and_then(|f| crate::data::flags_dir().map(|d| d.join(f)))
            .filter(|p| p.is_file());
        let country_label = if ed.country.is_empty() {
            self.i18n.t("tr-no-country")
        } else {
            format!(
                "{} ({})",
                self.countries
                    .endonym_for(&ed.country, self.i18n.current_locale())
                    .unwrap_or(""),
                ed.country
            )
        };
        let label = |code: &str| self.i18n.paren(&self.i18n.display_name(code), code);
        let mut targets: Vec<(String, String)> = self
            .country_languages
            .languages_for(&ed.country)
            .iter()
            .filter(|c| self.i18n.has_language(c))
            .map(|c| (c.clone(), label(c)))
            .collect();
        if targets.is_empty() {
            targets = self
                .i18n
                .available_locales()
                .iter()
                .map(|c| (c.clone(), label(c)))
                .collect();
        }
        let mut displayed: Vec<(String, String)> = self
            .i18n
            .available_locales()
            .iter()
            .map(|c| (c.clone(), label(c)))
            .collect();
        displayed.sort_by_key(|(_, n)| n.to_lowercase());
        let feed = te::Feed {
            i18n: &self.i18n,
            flag,
            country_label,
            targets,
            displayed,
            can_translate: !ed.target.is_empty() && self.i18n.has_language(&ed.target),
        };
        let mut actions = Vec::new();
        let mut geometry = None;
        let mut close_requested = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            egui::CentralPanel::default().show(vctx, |ui| {
                actions = te::body(ui, &mut ed, &feed);
            });
            if !just_opened {
                geometry = read_geometry(vctx);
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close_requested = true;
            }
        });
        self.tr_needs_center = false;
        if let Some((x, y, w, h)) = geometry {
            self.config.tr_win = [x, y, w, h];
        }
        if close_requested {
            if ed.dirty {
                ed.confirm = Some(Pending::Close);
                ctx.send_viewport_cmd_to(id, egui::ViewportCommand::CancelClose);
            } else {
                actions.push(TrAction::Close);
            }
        }
        let mut keep = true;
        for a in actions {
            match a {
                TrAction::PickCountry => {
                    self.flags_purpose = super::FlagsPurpose::Translation;
                    self.open_flags();
                }
                TrAction::SetTarget(code) => {
                    if ed.dirty {
                        ed.confirm = Some(Pending::Target(code));
                    } else {
                        ed.target = code;
                        ed.font.clear();
                        self.tr_load(&mut ed, None);
                    }
                }
                TrAction::SetDisplayed(code) => {
                    let dirty = ed.dirty;
                    let values = ed.values();
                    ed.displayed = code;
                    self.tr_load(&mut ed, Some(values));
                    ed.dirty = dirty;
                }
                TrAction::BrowseFont => self.tr_pick_font(&mut ed),
                TrAction::GoogleFonts => crate::fonts::open_url("https://fonts.google.com/"),
                TrAction::Save => {
                    if !ed.target.is_empty() {
                        self.tr_save(&mut ed);
                    }
                }
                TrAction::Reload => {
                    ed.font.clear();
                    self.tr_load(&mut ed, None);
                }
                TrAction::Send => self.tr_send(&mut ed),
                TrAction::Close => keep = false,
            }
        }
        if keep {
            self.tr = Some(ed);
        } else {
            let _ = self.config.save();
            // Back to the interface font only.
            self.tr_apply_preview_font(ctx, "");
        }
    }
}
