//! The dialogues of the active document in the application (C-01, C-02,
//! C-03, C-13): the context pass and the masters' name indexes in the
//! background, the dialogue map rebuilt when the document changes, the
//! Dialogues window (F10) and the Dialogue tab of the edit window.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use super::{XimodApp, read_geometry};
use crate::dialogue::DialogueMap;
use crate::plugin::context::{ContextMap, Family};
use crate::ui::dialog_window::{CtxJob, CtxMsg, DialogAction, Feed, MasterJob, VoiceLine};

impl XimodApp {
    /// Outils ▸ Dialogues… / F10: open the window, or bring it to the front.
    pub(crate) fn open_dialog_window(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        if !self.dialogs.open {
            self.dialogs.needs_place = true;
            self.dialogs.follow_grid = self.config.dialog_follow_grid;
            self.dialogs.split = self.config.dialog_split;
        }
        self.dialogs.open = true;
        // Select the reply of the grid's current line, if any.
        if let (Some(map), Some(cur)) = (self.dialogs.map.clone(), self.selection.cursor)
            && let Some(info) = map.info_of_line(cur)
        {
            self.dialogs.select_info(&map, info);
        }
    }

    /// The game family of the active document, when it is a plugin.
    fn dialog_family(&self) -> Option<Family> {
        let doc = self.docs.get(self.active)?;
        if doc.tm.meta.source_format != "plugin" {
            return None;
        }
        let game = doc.settings.game();
        if game == crate::games::Game::Unknown || game == crate::games::Game::Morrowind {
            return None;
        }
        Some(Family::of(game))
    }

    /// The Data folders where the masters of the active document may be.
    fn dialog_data_dirs(&self) -> Vec<PathBuf> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        let game = doc.settings.game();
        let mut out = Vec::new();
        for (id, entry) in &self.config.games {
            if crate::games::Game::from_id(id) == game {
                out.push(PathBuf::from(&entry.path).join(crate::games::detect::data_subdir(game)));
            }
        }
        out
    }

    /// Collect the background jobs (context pass, masters).
    pub(crate) fn dialog_poll(&mut self, ctx: &egui::Context) {
        let mut repaint = false;
        if let Some(job) = self.dialogs.ctx_job.as_mut() {
            let mut done: Option<(u64, Result<ContextMap, String>)> = None;
            loop {
                match job.rx.try_recv() {
                    Ok(CtxMsg::Progress(d, t)) => {
                        job.progress = if t == 0 { 1.0 } else { d as f32 / t as f32 };
                    }
                    Ok(CtxMsg::Done(r)) => {
                        done = Some((job.serial, *r));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        repaint = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        done = Some((job.serial, Err(crate::l10n!("err-job-stopped"))));
                        break;
                    }
                }
            }
            if let Some((serial, r)) = done {
                self.dialogs.ctx_job = None;
                match r {
                    Ok(map) => {
                        self.dialogs.ctx_cache.insert(serial, Arc::new(map));
                    }
                    Err(e) => {
                        tracing::warn!("context pass: {e}");
                        self.dialogs.ctx_failed.insert(serial);
                    }
                }
            }
        }
        let mut finished: Vec<(PathBuf, Option<ContextMap>)> = Vec::new();
        for (path, job) in self.dialogs.master_jobs.iter_mut() {
            loop {
                match job.rx.try_recv() {
                    Ok(CtxMsg::Progress(d, t)) => {
                        job.progress = if t == 0 { 1.0 } else { d as f32 / t as f32 };
                    }
                    Ok(CtxMsg::Done(r)) => {
                        finished.push((path.clone(), (*r).ok()));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        repaint = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished.push((path.clone(), None));
                        break;
                    }
                }
            }
        }
        for (path, m) in finished {
            self.dialogs.master_jobs.remove(&path);
            match m {
                Some(m) => {
                    self.dialogs.masters.insert(path, Arc::new(m));
                }
                None => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.dialogs.masters_missing.insert(name);
                }
            }
        }
        if repaint {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    /// Start the context pass of the active document, when it has none yet.
    fn dialog_start_ctx(&mut self, serial: u64, path: PathBuf, family: Family) {
        if self.dialogs.ctx_job.is_some() || self.dialogs.ctx_failed.contains(&serial) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let r = ContextMap::read_file_with(&path, family, false, &mut |d, t| {
                tx2.send(CtxMsg::Progress(d, t)).is_ok()
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(CtxMsg::Done(Box::new(r)));
        });
        self.dialogs.ctx_job = Some(CtxJob {
            serial,
            rx,
            progress: 0.0,
        });
    }

    /// Start the name indexes of the masters of a context map, when missing.
    fn dialog_start_masters(&mut self, ctx: &ContextMap, family: Family) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let plugin_dir = doc
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let data_dirs = self.dialog_data_dirs();
        let lang = doc.settings.source.clone();
        let utf8 = crate::games::registry().spec(doc.settings.game()).utf8;
        for name in &ctx.masters {
            if self.dialogs.masters_missing.contains(name) {
                continue;
            }
            let Some(path) = crate::plugin::names::find_master(name, &plugin_dir, &data_dirs)
            else {
                self.dialogs.masters_missing.insert(name.clone());
                continue;
            };
            if self.dialogs.masters.contains_key(&path)
                || self.dialogs.master_jobs.contains_key(&path)
            {
                continue;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let p = path.clone();
            let lang = lang.clone();
            std::thread::spawn(move || {
                let tx2 = tx.clone();
                let r =
                    crate::plugin::names::load_or_build(&p, family, &lang, utf8, &mut |d, t| {
                        tx2.send(CtxMsg::Progress(d, t)).is_ok()
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(CtxMsg::Done(Box::new(r)));
            });
            self.dialogs
                .master_jobs
                .insert(path, MasterJob { rx, progress: 0.0 });
        }
    }

    /// The masters of a context map, as the resolver wants them.
    fn dialog_masters_of(&self, ctx: &ContextMap) -> Vec<Option<Arc<ContextMap>>> {
        let Some(doc) = self.docs.get(self.active) else {
            return Vec::new();
        };
        let plugin_dir = doc
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let data_dirs = self.dialog_data_dirs();
        ctx.masters
            .iter()
            .map(|name| {
                crate::plugin::names::find_master(name, &plugin_dir, &data_dirs)
                    .and_then(|p| self.dialogs.masters.get(&p).cloned())
            })
            .collect()
    }

    /// The dialogue map of the active document: starts what is missing,
    /// rebuilds the map when the document's lines changed. `None` while the
    /// context is read, or when the document has no plugin dialogue.
    pub(crate) fn dialog_map(&mut self, ctx: &egui::Context) -> Option<Arc<DialogueMap>> {
        self.dialog_poll(ctx);
        let family = self.dialog_family()?;
        let (serial, revision, path) = {
            let doc = self.docs.get(self.active)?;
            (doc.serial, doc.undo.revision, doc.path.clone())
        };
        let Some(cmap) = self.dialogs.ctx_cache.get(&serial).cloned() else {
            self.dialog_start_ctx(serial, path, family);
            return None;
        };
        self.dialog_start_masters(&cmap, family);
        let known = self.dialogs.masters.len();
        // The voice map (C-13), at the first demand and after « Rafraîchir ».
        self.dialog_voices_ensure(serial, &path, family);
        let key = (serial, revision, known, self.dialogs.voices_gen);
        if self.dialogs.map_key != Some(key) || self.dialogs.map.is_none() {
            let masters = self.dialog_masters_of(&cmap);
            let doc = self.docs.get(self.active)?;
            let voices = self.dialogs.voices.clone();
            let map = DialogueMap::build_with(
                cmap,
                masters,
                &doc.tm.entries,
                &self.i18n,
                voices.as_deref(),
            );
            // The expanded set belongs to a document.
            if self.dialogs.expanded_for != Some(serial) {
                self.dialogs.expanded.clear();
                self.dialogs.expanded_for = Some(serial);
                for r in &map.roots {
                    self.dialogs.expanded.insert(*r);
                }
                self.dialogs.selected = None;
            }
            self.dialogs.map = Some(Arc::new(map));
            self.dialogs.map_key = Some(key);
        }
        self.dialogs.map.clone()
    }

    /// The dialogue map for a synchronous need (the generated comments of a
    /// save, R-C03-06): the current map, else the context read on the spot
    /// — without the masters not loaded yet — and kept for the window.
    pub(crate) fn dialog_map_blocking(&mut self) -> Option<Arc<DialogueMap>> {
        if let Some(map) = self.dialogs.map.clone() {
            return Some(map);
        }
        let family = self.dialog_family()?;
        let (serial, path) = {
            let doc = self.docs.get(self.active)?;
            (doc.serial, doc.path.clone())
        };
        if !self.dialogs.ctx_cache.contains_key(&serial) {
            if self.dialogs.ctx_failed.contains(&serial) {
                return None;
            }
            match ContextMap::read_file(&path, family) {
                Ok(m) => {
                    self.dialogs.ctx_cache.insert(serial, Arc::new(m));
                }
                Err(e) => {
                    tracing::warn!("context pass: {e}");
                    self.dialogs.ctx_failed.insert(serial);
                    return None;
                }
            }
        }
        let cmap = self.dialogs.ctx_cache.get(&serial).cloned()?;
        let masters = self.dialog_masters_of(&cmap);
        let doc = self.docs.get(self.active)?;
        Some(Arc::new(DialogueMap::build(
            cmap,
            masters,
            &doc.tm.entries,
            &self.i18n,
        )))
    }

    /// Progress text of the context pass, for the status bar.
    pub(crate) fn dialog_progress_text(&self) -> Option<String> {
        let job = self.dialogs.ctx_job.as_ref()?;
        let doc = self.docs.get(self.active)?;
        Some(self.i18n.t_args(
            "dialog-indexing",
            &[
                ("file", &doc.file_name()),
                ("pct", &((job.progress * 100.0) as u32).to_string()),
            ],
        ))
    }

    fn dialog_master_progress(&self) -> Option<(String, f32)> {
        self.dialogs.master_jobs.iter().next().map(|(p, j)| {
            (
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                j.progress,
            )
        })
    }

    /// The conditions of a reply, in clear and raw (C-02).
    pub(crate) fn dialog_conditions(&self, map: &DialogueMap, info: u32) -> Vec<(String, String)> {
        crate::conditions::describe(
            map.conditions(info),
            &map.res,
            &self.conditions_table_for(map),
            &self.i18n,
        )
    }

    fn conditions_table_for(&self, map: &DialogueMap) -> Arc<crate::conditions::Table> {
        let family = map.res.ctx.family.unwrap_or(Family::Skyrim);
        crate::conditions::Table::load(family)
    }

    /// The Dialogues window (R-C01-07).
    pub(crate) fn dialog_window(&mut self, ctx: &egui::Context) {
        if !self.dialogs.open {
            return;
        }
        if self.docs.is_empty() {
            self.dialogs.open = false;
            let _ = self.config.save();
            return;
        }
        let map = self.dialog_map(ctx);
        // Follow the grid (R-C01-05).
        if self.dialogs.follow_grid
            && let Some(map) = &map
        {
            let cur = self.selection.cursor;
            if cur != self.dialogs.followed_line {
                self.dialogs.followed_line = cur;
                if let Some(info) = cur.and_then(|l| map.info_of_line(l))
                    && self.dialogs.selected != Some(info)
                {
                    self.dialogs.select_info(map, info);
                }
            }
        }
        let id = egui::ViewportId::from_hash_of("ximod-dialogues");
        let geo = self.config.dialog_win;
        let just_opened = self.dialogs.needs_place;
        let place = if just_opened {
            let pos = self.open_pos_cascade(geo[0], geo[1]);
            Some((pos, [geo[2].max(640.0), geo[3].max(420.0)]))
        } else {
            None
        };
        let (file, game) = self
            .docs
            .get(self.active)
            .map(|d| (d.file_name(), d.settings.game().display_name().to_string()))
            .unwrap_or_default();
        let title = self
            .i18n
            .t_args("dialog-title", &[("doc", &file), ("game", &game)]);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([640.0, 420.0]);
        if let Some((pos, size)) = place {
            builder = builder
                .with_position(pos)
                .with_inner_size(size)
                .with_active(true);
        }
        let mut action = DialogAction::None;
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |vctx, _class| {
            if let Some((pos, size)) = place {
                vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
                vctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
            self.dialogs.needs_place = false;
            egui::CentralPanel::default().show(vctx, |ui| match &map {
                Some(map) if !map.is_empty() => {
                    let selected = self.dialogs.selected;
                    let conditions = selected
                        .map(|s| self.dialog_conditions(map, s))
                        .unwrap_or_default();
                    let (voices, voices_note) = selected
                        .map(|s| self.dialog_voices_of(map, s))
                        .unwrap_or((None, None));
                    let feed = Feed {
                        map,
                        entries: &self.docs[self.active].tm.entries,
                        i18n: &self.i18n,
                        colors: self.config.status_colors,
                        master_progress: self.dialog_master_progress(),
                        voices,
                        voices_note,
                        conditions,
                    };
                    action = crate::ui::dialog_window::body(ui, &mut self.dialogs, &feed);
                }
                Some(_) => {
                    ui.label(self.i18n.t("dialog-none"));
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        close = true;
                    }
                }
                None => {
                    if self.dialog_family().is_none() {
                        ui.label(self.i18n.t("dialog-none"));
                    } else {
                        let p = self
                            .dialogs
                            .ctx_job
                            .as_ref()
                            .map(|j| j.progress)
                            .unwrap_or(0.0);
                        ui.label(self.dialog_progress_text().unwrap_or_default());
                        ui.add(egui::ProgressBar::new(p).animate(true));
                    }
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        close = true;
                    }
                }
            });
            if !just_opened && let Some((x, y, w, h)) = read_geometry(vctx) {
                self.config.dialog_win = [x, y, w, h];
            }
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
        });
        self.config.dialog_follow_grid = self.dialogs.follow_grid;
        self.config.dialog_split = self.dialogs.split;
        match action {
            DialogAction::Close => close = true,
            other => self.dialog_apply(ctx, other),
        }
        if close {
            self.dialogs.open = false;
            let _ = self.config.save();
        }
    }

    /// Apply what the window asked.
    pub(crate) fn dialog_apply(&mut self, ctx: &egui::Context, action: DialogAction) {
        match action {
            DialogAction::None | DialogAction::Close => {}
            DialogAction::OpenWindow => self.open_dialog_window(),
            DialogAction::SelectLine(l) => {
                self.close_edit_session();
                self.selection.set_single(l);
                self.grid_scroll_to = Some(l);
                self.pinned_rows.insert(l);
                self.dialogs.followed_line = Some(l);
                if let Some(w) = self.edit_win.as_mut() {
                    w.go_to(l);
                }
            }
            DialogAction::OpenLine(l) => {
                self.dialogs.followed_line = Some(l);
                self.open_edit_window(l);
            }
            DialogAction::FilterGrid(key) => {
                self.filter_speaker = Some(key);
            }
            DialogAction::Copy(text) => {
                ctx.copy_text(text);
            }
            DialogAction::RefreshVoices => {
                self.voices_refresh();
            }
            DialogAction::OpenFolder(p) => {
                crate::voices::open_folder(&p);
            }
            DialogAction::CopyUnvoiced => {
                if let (Some(map), Some(doc)) =
                    (self.dialogs.map.clone(), self.docs.get(self.active))
                {
                    let lines = map.unvoiced_list(&doc.tm.entries, &self.i18n);
                    let n = lines.len();
                    ctx.copy_text(lines.join("\n"));
                    self.dialogs.message =
                        Some(self.i18n.t_arg("dialog-copied", "n", &n.to_string()));
                }
            }
        }
    }

    /// Menu de la ligne ▸ Filtrer sur ce PNJ (R-C02-06).
    pub(crate) fn filter_on_speaker_of(&mut self, ctx: &egui::Context, line: usize) {
        let Some(map) = self.dialog_map(ctx) else {
            self.status = self.i18n.t("filter-speaker-none");
            return;
        };
        let key = map
            .info_of_line(line)
            .and_then(|info| map.speaker(info))
            .filter(|s| s.kind != crate::dialogue::SpeakerKind::Unknown)
            .map(|s| s.key());
        match key {
            Some(k) => {
                self.filter_speaker = Some(k);
                self.filter_formid = None;
                self.filter_source_exact = None;
            }
            None => self.status = self.i18n.t("filter-speaker-none"),
        }
    }

    /// The name shown on the « PNJ : … » chip.
    pub(crate) fn speaker_name_of_key(&self, key: &str) -> String {
        self.dialogs
            .map
            .as_ref()
            .and_then(|m| {
                m.speaker_counts(&self.i18n)
                    .into_iter()
                    .find(|s| s.0 == key)
                    .map(|s| s.1)
            })
            .unwrap_or_else(|| key.to_string())
    }

    /// The lines of the grid's speaker filter, for the view criteria.
    pub(crate) fn speaker_filter_lines(&self) -> Option<crate::filter::AnomalySet> {
        let key = self.filter_speaker.as_ref()?;
        let map = self.dialogs.map.as_ref()?;
        let mut set: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for info in map.infos_of_speaker(key) {
            for (l, n) in &map.line_node {
                let mut cur = Some(*n);
                while let Some(i) = cur {
                    match map.nodes[i].kind {
                        crate::dialogue::NodeKind::Info(f)
                        | crate::dialogue::NodeKind::Response(f, _)
                            if f == info =>
                        {
                            set.insert(*l);
                            break;
                        }
                        crate::dialogue::NodeKind::Info(_)
                        | crate::dialogue::NodeKind::Response(_, _) => break,
                        _ => cur = map.nodes[i].parent,
                    }
                }
            }
        }
        Some(crate::filter::AnomalySet(Arc::new(set)))
    }

    /// Build the voice map of the active document when it has none yet
    /// (R-C13-01): the loose files and the plugin's archives, listed
    /// without extraction.
    fn dialog_voices_ensure(&mut self, serial: u64, path: &std::path::Path, family: Family) {
        if self.dialogs.voices_for == Some(serial) {
            return;
        }
        let spec = crate::dialogue::DialogueSpec::load(family).voice;
        let map = crate::voices::VoiceMap::build(path, family, &spec);
        self.dialogs.voices = Some(Arc::new(map));
        self.dialogs.voices_for = Some(serial);
    }

    /// The voice files of a reply, per response (C-13): `None` until
    /// mapped; the note says why nothing can be marked.
    pub(crate) fn dialog_voices_of(
        &self,
        map: &DialogueMap,
        info: u32,
    ) -> (Option<Vec<VoiceLine>>, Option<String>) {
        let i18n = &self.i18n;
        let Some(voices) = self.dialogs.voices.as_ref() else {
            return (None, None);
        };
        if !voices.has_sources() {
            return (
                Some(Vec::new()),
                Some(i18n.t_arg("voice-none-for-mod", "plugin", &voices.plugin)),
            );
        }
        let Some(inf) = map.res.ctx.infos.get(&info) else {
            return (None, None);
        };
        let mut lines = Vec::new();
        for r in &inf.responses {
            let n = r.number.to_string();
            let line = match voices.status(info, r.number) {
                crate::voices::Status::Voiced(files) => {
                    let mut text = String::new();
                    let mut tips = Vec::new();
                    let mut folder = None;
                    for (k, f) in files.iter().enumerate() {
                        let (where_l, tip) = match &f.location {
                            crate::voices::Where::Loose(p) => {
                                folder.get_or_insert(p.clone());
                                (i18n.t("voice-where-loose"), p.display().to_string())
                            }
                            crate::voices::Where::Archive(a) => (
                                i18n.t_arg("voice-where-archive", "archive", a),
                                i18n.colon(a, &f.path),
                            ),
                        };
                        if k > 0 {
                            text.push_str(" · ");
                        }
                        text.push_str(&i18n.t_args(
                            "voice-ok",
                            &[
                                ("n", &n),
                                ("type", &f.voice_type),
                                ("file", &f.name),
                                ("where", &where_l),
                            ],
                        ));
                        tips.push(tip);
                    }
                    VoiceLine {
                        text: format!("{} {text}", crate::icons::CHECK),
                        ok: true,
                        tip: tips.join("\n"),
                        folder,
                    }
                }
                crate::voices::Status::LipOnly(files) => VoiceLine {
                    text: format!(
                        "{} {}",
                        crate::icons::X,
                        i18n.t_args("voice-lip-only", &[("n", &n), ("file", &files[0].name)])
                    ),
                    ok: false,
                    tip: files
                        .iter()
                        .map(|f| match &f.location {
                            crate::voices::Where::Loose(p) => p.display().to_string(),
                            crate::voices::Where::Archive(a) => i18n.colon(a, &f.path),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    folder: None,
                },
                crate::voices::Status::Missing => VoiceLine {
                    text: format!(
                        "{} {}",
                        crate::icons::X,
                        i18n.t_arg("voice-missing", "n", &n)
                    ),
                    ok: false,
                    tip: String::new(),
                    folder: None,
                },
                crate::voices::Status::Unresolved(count) => VoiceLine {
                    text: i18n.dot(
                        &n,
                        &i18n.t_arg("voice-unresolved-starfield", "n", &count.to_string()),
                    ),
                    ok: true,
                    tip: String::new(),
                    folder: None,
                },
            };
            lines.push(line);
        }
        let note = voices
            .unresolved
            .then(|| i18n.t_arg("voice-unresolved-starfield", "n", &voices.files.to_string()));
        (Some(lines), note)
    }

    /// « Rafraîchir » (R-C13-01): the voice map is rebuilt at the next
    /// demand, and the comments with it.
    pub(crate) fn voices_refresh(&mut self) {
        self.dialogs.voices_for = None;
        self.dialogs.voices = None;
        self.dialogs.voices_gen += 1;
        self.comment_cache.clear();
    }

    /// The Dialogue tab of the edit window (R-C01-08).
    pub(crate) fn dialog_tab(&mut self, ui: &mut egui::Ui, line: usize) -> Option<DialogAction> {
        let ctx = ui.ctx().clone();
        let Some(map) = self.dialog_map(&ctx) else {
            if self.dialog_family().is_none() {
                ui.label(egui::RichText::new(self.i18n.t("dialog-tab-not-dialogue")).weak());
            } else {
                ui.label(
                    egui::RichText::new(self.dialog_progress_text().unwrap_or_default()).weak(),
                );
            }
            return None;
        };
        let i18n = &self.i18n;
        let entries = &self.docs[self.active].tm.entries;
        let Some(info) = map.info_of_line(line) else {
            // A topic or a quest line.
            if let Some(&n) = map.line_node.get(&line) {
                let node = &map.nodes[n];
                ui.label(egui::RichText::new(&node.label).strong());
                ui.label(egui::RichText::new(&node.extra).weak());
            } else {
                ui.label(egui::RichText::new(i18n.t("dialog-tab-not-dialogue")).weak());
            }
            return None;
        };
        let mut action: Option<DialogAction> = None;
        let inf = map.res.ctx.infos.get(&info)?;
        let speakers = map.speakers.get(&info).cloned().unwrap_or_default();
        let (quest, topic) = map.where_of(info, i18n);
        let which = inf.responses.iter().position(|r| {
            r.line_index
                .and_then(|ix| map.line("INFO", "NAM1", info, ix))
                == Some(line)
        });
        let emotion = which
            .and_then(|p| inf.responses.get(p))
            .map(|r| map.emotion_label(i18n, r.emotion, r.emotion_value))
            .unwrap_or_default();
        let mut head = format!(
            "{} · {} · {}",
            crate::ui::dialog_window::speaker_line(i18n, &speakers, &map),
            quest,
            topic
        );
        if let Some(p) = which {
            head.push_str(&format!(
                " · {}",
                i18n.t_args(
                    "dialog-response-of",
                    &[
                        ("n", &(p + 1).to_string()),
                        ("total", &inf.responses.len().to_string())
                    ]
                )
            ));
        }
        if !emotion.is_empty() {
            head = self.i18n.dot(&head, &emotion);
        }
        ui.add(egui::Label::new(egui::RichText::new(head).strong().small()).wrap());
        // The player's words.
        let prompt_line = map
            .line("INFO", "RNAM", info, 1)
            .or_else(|| map.line("DIAL", "FULL", inf.topic, 1));
        if let Some(e) = prompt_line.and_then(|l| entries.get(l)) {
            let said = i18n.t_args(
                "dialog-player-says",
                &[("label", &i18n.t("dialog-player")), ("text", &e.source)],
            );
            let t = if e.trans.is_empty() {
                said
            } else {
                format!(
                    "{said} {} {}",
                    crate::icons::ARROW_RIGHT,
                    i18n.t_arg("text-quoted", "text", &e.trans)
                )
            };
            ui.add(egui::Label::new(egui::RichText::new(t).small()).wrap());
        }
        // Before / after.
        let neighbour = |dir: i32| -> Option<String> {
            let other = map.step(info, dir)?;
            let n = *map.info_node.get(&other)?;
            Some(map.nodes[n].label.clone())
        };
        let before = neighbour(-1).unwrap_or_else(|| "—".to_string());
        let after = neighbour(1).unwrap_or_else(|| "—".to_string());
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "{} {}      {} {}",
                    i18n.t("dialog-before"),
                    before,
                    i18n.t("dialog-after"),
                    after
                ))
                .weak()
                .small(),
            )
            .wrap(),
        );
        // Conditions.
        let conds = self.dialog_conditions(&map, info);
        let cond_text = if conds.is_empty() {
            i18n.t("dialog-no-conditions")
        } else {
            conds
                .iter()
                .map(|(t, _)| t.clone())
                .collect::<Vec<_>>()
                .join(" · ")
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{} {cond_text}", i18n.t("dialog-conditions"))).small(),
            )
            .wrap(),
        );
        // Voice and note.
        let (voices, note) = self.dialog_voices_of(&map, info);
        let voice_text = match voices {
            Some(v) if !v.is_empty() => v
                .iter()
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
                .join(" · "),
            _ => note.unwrap_or_else(|| i18n.t("voice-not-mapped")),
        };
        let nam2 = which
            .and_then(|p| inf.responses.get(p))
            .map(|r| r.note.clone())
            .unwrap_or_default();
        let mut tail = format!("{} {voice_text}", i18n.t("dialog-voice"));
        if !nam2.is_empty() {
            tail.push_str(&format!("      {} {nam2}", i18n.t("dialog-note")));
        }
        ui.add(egui::Label::new(egui::RichText::new(tail).small()).wrap());
        ui.horizontal(|ui| {
            let prev = map.step(info, -1);
            let next = map.step(info, 1);
            if ui
                .add_enabled(
                    prev.is_some(),
                    egui::Button::new(format!(
                        "{} {}",
                        crate::icons::CARET_LEFT,
                        i18n.t("dialog-prev")
                    ))
                    .small(),
                )
                .clicked()
            {
                let l = map.nodes[map.info_node[&prev.unwrap()]].line;
                if let Some(l) = l {
                    action = Some(DialogAction::OpenLine(l));
                }
            }
            if ui
                .add_enabled(
                    next.is_some(),
                    egui::Button::new(format!(
                        "{} {}",
                        i18n.t("dialog-next"),
                        crate::icons::CARET_RIGHT
                    ))
                    .small(),
                )
                .clicked()
            {
                let l = map.nodes[map.info_node[&next.unwrap()]].line;
                if let Some(l) = l {
                    action = Some(DialogAction::OpenLine(l));
                }
            }
            if ui.small_button(i18n.t("menu-dialogues")).clicked() {
                action = Some(DialogAction::OpenWindow);
            }
        });
        action
    }
}
