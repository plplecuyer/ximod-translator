//! Online translation in the application (D-01, D-02): Ctrl+G on the
//! line or the selection, the « Traduire en ligne… » dialog for a batch,
//! the job polled each frame and applied as one undoable operation, the
//! « Tester… » of the settings, and the FOMOD window's request.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use eframe::egui;

use super::XimodApp;
use crate::online::job::{JobInput, JobMsg};
pub use crate::online::mass::Sent;
use crate::online::mass::{self, Outcome, Reason, Skipped, Summary};
use crate::online::profile::{self, Profile};
use crate::online::protect;
use crate::online::quota::Item;
use crate::online::{Error, Service};
use crate::status::Status;
use crate::tags::TagSet;
use crate::undo::{OpKind, record};

/// The portée of the dialog (R-D01-03, R-D04-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    Line,
    Selection,
    View,
    /// Every line of the document, whatever the filters (D-04).
    #[default]
    Document,
}

impl Scope {
    /// The value of `[Online] LastScope`.
    pub fn id(self) -> &'static str {
        match self {
            Scope::Line => "line",
            Scope::Selection => "selection",
            Scope::View => "view",
            Scope::Document => "document",
        }
    }

    pub fn from_id(s: &str) -> Scope {
        match s.trim() {
            "line" => Scope::Line,
            "selection" => Scope::Selection,
            "view" => Scope::View,
            _ => Scope::Document,
        }
    }
}

/// Where the answers go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Lines of a document (its serial).
    Doc(u64),
    /// The FOMOD window's row (unit index).
    Fomod(usize),
}

/// A job in flight, then its result until the dialog is closed.
pub struct Run {
    pub service: Service,
    pub rx: Receiver<JobMsg>,
    pub stop: Arc<AtomicBool>,
    pub target: Target,
    /// Line → item key.
    pub lines: Vec<(usize, usize)>,
    pub items: Vec<Sent>,
    pub answers: HashMap<usize, Result<String, Error>>,
    pub done: usize,
    pub total: usize,
    pub received: usize,
    pub cached: usize,
    pub finished: bool,
    pub stopped: bool,
    /// Received, refused (by reason), left by the stop, once applied.
    pub summary: Option<Summary>,
}

/// One row of the dialog's journal.
#[derive(Debug, Clone)]
pub struct LogRow {
    pub source: String,
    pub result: Result<String, String>,
}

#[derive(Default)]
pub struct OnlineState {
    pub dialog_open: bool,
    pub scope: Option<Scope>,
    pub only_untranslated: bool,
    pub run: Option<Run>,
    pub session_chars: usize,
    pub log: Vec<LogRow>,
    pub message: Option<String>,
    /// Lines whose Validé / Personnalisé translation would be replaced:
    /// asked before (R-D01-02).
    pub confirm_overwrite: Option<(Vec<usize>, usize, bool)>,
    /// Over the session threshold: asked before (R-D02-04).
    pub confirm_session: Option<(Vec<usize>, usize, bool)>,
    /// « Tester… » in flight, and its last result per service.
    pub test: Option<(Service, Receiver<Result<String, Error>>)>,
    pub test_result: HashMap<Service, Result<String, Error>>,
    /// The keys typed in the settings, per service, and where each lives.
    pub keys: HashMap<Service, String>,
    pub keys_loaded: HashMap<Service, String>,
    pub key_store: HashMap<Service, crate::online::secrets::Store>,
    pub show_key: bool,
    pub settings_service: Service,
    /// The profiles as edited in the settings (D-03), those changed, those
    /// removed, the problem of the one shown.
    pub profiles: Vec<Profile>,
    pub profiles_dirty: std::collections::HashSet<String>,
    pub profiles_removed: Vec<String>,
    /// The cache's answers for the dialog's estimate: (service, scope,
    /// only, lines) → distinct sources and lines it serves.
    pub cache_estimate: Option<(String, Scope, bool, usize, usize, usize)>,
}

impl XimodApp {
    /// The usable service and its key, or `None` (R-D01-01).
    pub(crate) fn online_usable(&self) -> Option<(Service, String)> {
        let service = self.config.online.service.clone()?;
        let (key, _) = crate::online::secrets::get(&service);
        match key {
            Some(k) => Some((service, k)),
            None if service.key_optional() => Some((service, String::new())),
            None => None,
        }
    }

    /// The settings of the default service (its profile's guards).
    fn online_cfg(&self) -> crate::online::OnlineConfig {
        match &self.config.online.service {
            Some(s) => self.config.online.for_service(s),
            None => self.config.online.clone(),
        }
    }

    /// The language codes of the active document for a service.
    fn online_pair(&self) -> Option<(String, String)> {
        let doc = self.active_doc()?;
        let src = doc.settings.source.trim().to_string();
        let dst = doc.settings.target.trim().to_string();
        if src.is_empty() || dst.is_empty() {
            return None;
        }
        Some((self.iso1_of(&src), self.iso1_of(&dst)))
    }

    fn iso1_of(&self, code: &str) -> String {
        self.i18n
            .iso3_to_iso1(code)
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| crate::online::iso1(code))
    }

    /// Ctrl+G on the grid / the edit window (R-D01-02): the service when
    /// one is usable, the browser otherwise (as in lot 2).
    pub(crate) fn online_line(&mut self, lines: Vec<usize>) {
        if lines.is_empty() {
            return;
        }
        if self.online_usable().is_none() {
            // The browser, as in lot 2.
            let Some(doc) = self.active_doc() else {
                return;
            };
            if let Some(e) = doc.tm.entries.get(lines[0]) {
                let iso1 = self.iso1_of(&doc.settings.target);
                crate::fonts::open_url(&crate::ui::fomod_window::google_url(&e.source, &iso1));
            }
            self.status = self.i18n.t("online-no-service");
            return;
        }
        self.online_begin(lines, false, false);
    }

    /// The lines a scope covers, before the selection of what is sent.
    fn online_scope_base(&self, scope: Scope) -> Vec<usize> {
        match scope {
            Scope::Line => self.selection.cursor.into_iter().collect(),
            Scope::Selection => self.selection.targets(),
            Scope::View => self.visible.clone(),
            Scope::Document => self
                .active_doc()
                .map(|d| (0..d.tm.entries.len()).collect())
                .unwrap_or_default(),
        }
    }

    /// The lines of a scope that a job sends, and those it leaves
    /// (R-D01-03, R-D04-01 / 02).
    pub(crate) fn online_scope_lines(
        &self,
        scope: Scope,
        only_untranslated: bool,
    ) -> (Vec<usize>, Skipped) {
        let Some(doc) = self.active_doc() else {
            return (Vec::new(), Skipped::default());
        };
        mass::select(
            &doc.tm.entries,
            self.online_scope_base(scope),
            only_untranslated,
        )
    }

    /// Start a job on lines, after the confirmations (R-D01-02, R-D02-04).
    pub(crate) fn online_begin(&mut self, lines: Vec<usize>, confirmed: bool, show_dialog: bool) {
        if self.online.run.as_ref().is_some_and(|r| !r.finished) {
            self.status = self.i18n.t("online-busy");
            return;
        }
        let Some((service, key)) = self.online_usable() else {
            self.status = self.i18n.t("online-no-service");
            return;
        };
        let Some((src, dst)) = self.online_pair() else {
            self.status = self.i18n.t("dict-stack-no-target");
            return;
        };
        let Some(doc) = self.active_doc() else {
            return;
        };
        if !confirmed {
            let protected = mass::protected_lines(&doc.tm.entries, &lines);
            if protected > 0 {
                self.online.confirm_overwrite = Some((lines, protected, show_dialog));
                return;
            }
            let chars: usize = lines
                .iter()
                .filter_map(|&i| doc.tm.entries.get(i))
                .map(|e| e.source.chars().count())
                .sum();
            if self.config.online.session_warn > 0
                && self.online.session_chars > self.config.online.session_warn
            {
                self.online.confirm_session = Some((lines, chars, show_dialog));
                return;
            }
        }
        let game = doc.settings.game();
        let opts = protect::Options {
            tags: self.config.online.protect_tags,
            numbers: self.config.online.protect_numbers,
            newlines: self.config.online.protect_newlines,
        };
        // Distinct sources, sent once (R-D01-03).
        let prepared = mass::prepare(&doc.tm.entries, &lines, game, opts);
        let job_items = prepared.job_items();
        let mass::Prepared {
            items,
            lines: targets,
        } = prepared;
        if items.is_empty() {
            self.status = self.i18n.t("online-nothing");
            return;
        }
        let serial = doc.serial;
        let mut cfg = self.config.online.for_service(&service);
        cfg.game = doc.settings.game_label();
        let input = JobInput {
            service: service.clone(),
            cfg,
            key,
            src: src.clone(),
            dst: dst.clone(),
            items: job_items,
            cache_path: Some(crate::online::cache::Cache::path_for(&service, &src, &dst)),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let rx = crate::online::job::start(input, stop.clone());
        self.online.log.clear();
        self.online.message = None;
        self.online.run = Some(Run {
            service,
            rx,
            stop,
            target: Target::Doc(serial),
            lines: targets,
            items,
            answers: HashMap::new(),
            done: 0,
            total: 0,
            received: 0,
            cached: 0,
            finished: false,
            stopped: false,
            summary: None,
        });
        if show_dialog {
            self.online.dialog_open = true;
        } else {
            self.status = self.i18n.t("online-sending");
        }
    }

    /// The FOMOD window's Ctrl+G (R-D01-06) with a service.
    pub(crate) fn online_fomod(
        &mut self,
        unit: usize,
        text: String,
        src_iso3: &str,
        dst_iso3: &str,
    ) -> bool {
        let Some((service, key)) = self.online_usable() else {
            return false;
        };
        if self.online.run.as_ref().is_some_and(|r| !r.finished) {
            return false;
        }
        let opts = protect::Options {
            tags: self.config.online.protect_tags,
            numbers: self.config.online.protect_numbers,
            newlines: self.config.online.protect_newlines,
        };
        let (p, tokens) = protect::protect(&text, &TagSet::any(), opts);
        let (src, dst) = (self.iso1_of(src_iso3), self.iso1_of(dst_iso3));
        let input = JobInput {
            service: service.clone(),
            cfg: self.config.online.for_service(&service),
            key,
            src: src.clone(),
            dst: dst.clone(),
            items: vec![Item {
                key: 0,
                text: p.clone(),
            }],
            cache_path: Some(crate::online::cache::Cache::path_for(&service, &src, &dst)),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let rx = crate::online::job::start(input, stop.clone());
        self.online.run = Some(Run {
            service,
            rx,
            stop,
            target: Target::Fomod(unit),
            lines: vec![(unit, 0)],
            items: vec![Sent {
                protected: p,
                tokens,
                original: text,
            }],
            answers: HashMap::new(),
            done: 0,
            total: 0,
            received: 0,
            cached: 0,
            finished: false,
            stopped: false,
            summary: None,
        });
        true
    }

    /// Drain the job's messages; apply at the end (R-D01-03).
    pub(crate) fn online_poll(&mut self, ctx: &egui::Context) {
        // The settings' test.
        if let Some((service, rx)) = &self.online.test {
            match rx.try_recv() {
                Ok(r) => {
                    let s = service.clone();
                    self.online.test_result.insert(s, r);
                    self.online.test = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let s = service.clone();
                    self.online
                        .test_result
                        .insert(s, Err(Error::Network(crate::l10n!("err-job-stopped"))));
                    self.online.test = None;
                }
            }
        }
        let mut finished = false;
        if let Some(run) = self.online.run.as_mut() {
            if run.finished {
                return;
            }
            loop {
                match run.rx.try_recv() {
                    Ok(JobMsg::Progress {
                        done,
                        total,
                        received,
                    }) => {
                        run.done = done;
                        run.total = total;
                        run.received = received;
                    }
                    Ok(JobMsg::Answer {
                        key,
                        result,
                        cached,
                    }) => {
                        if cached {
                            run.cached += 1;
                        }
                        run.answers.insert(key, result);
                    }
                    Ok(JobMsg::Done {
                        stopped,
                        sent_chars,
                    }) => {
                        run.finished = true;
                        run.stopped = stopped;
                        self.online.session_chars += sent_chars;
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        run.finished = true;
                        finished = true;
                        break;
                    }
                }
            }
        }
        if finished {
            self.online_apply();
        }
    }

    /// The text of a refusal.
    pub(crate) fn online_reason_text(&self, r: &Reason) -> String {
        match r {
            Reason::TokenLost => self.i18n.t("online-refused-token"),
            Reason::Empty => self.i18n.t("online-empty-answer"),
            Reason::Unchanged => self.i18n.t("online-unchanged-answer"),
            Reason::TooLong => self.i18n.t("online-line-too-long"),
            Reason::Service { code, message } => self.i18n.t_args(
                "online-refused-error",
                &[("code", &code.to_string()), ("message", message)],
            ),
        }
    }

    /// The answers into the lines, one operation (R-D01-03 / 04,
    /// R-D04-03), the journal for the dialog.
    fn online_apply(&mut self) {
        let Some(run) = self.online.run.as_ref() else {
            return;
        };
        // Restore each distinct answer once.
        let outcomes = mass::outcomes(&run.items, &run.answers);
        let mut log = Vec::with_capacity(outcomes.len());
        for (sent, o) in run.items.iter().zip(&outcomes) {
            let result = match o {
                Outcome::Taken(t) => Ok(crate::ui::edit_window::one_line(t, 80)),
                Outcome::Refused(r) => Err(self.online_reason_text(r)),
                Outcome::Left => Err(self.i18n.t("online-stopped-line")),
            };
            log.push(LogRow {
                source: crate::ui::edit_window::one_line(&sent.original, 80),
                result,
            });
        }
        self.online.log = log;
        self.online.cache_estimate = None;
        let service = run.service.clone();
        let target = run.target.clone();
        let lines_of_run = run.lines.clone();
        let summary = match target {
            Target::Doc(serial) => {
                // A cell being typed in is closed first (its own operation).
                self.close_edit_session();
                let Some(idx) = self.docs.iter().position(|d| d.serial == serial) else {
                    return;
                };
                let doc = &mut self.docs[idx];
                let label = service.short_label(&self.i18n);
                let prov = crate::provenance::online(&service.id());
                record(
                    &mut doc.undo,
                    &mut doc.tm.entries,
                    OpKind::Online,
                    Some(label.clone()),
                    |entries| mass::apply(entries, &lines_of_run, &outcomes, &prov),
                )
            }
            Target::Fomod(unit) => {
                let mut s = Summary::default();
                match outcomes.first() {
                    Some(Outcome::Taken(t)) => {
                        self.fomod_trans.set_unit_target(unit, t);
                        s.received = 1;
                    }
                    Some(Outcome::Refused(r)) => {
                        s.refused = 1;
                        s.by_reason.insert(r.clone(), 1);
                    }
                    _ => s.left = 1,
                }
                s
            }
        };
        let (taken, refused) = (summary.received, summary.refused);
        if let Some(run) = self.online.run.as_mut() {
            run.summary = Some(summary);
        }
        self.status = self.i18n.t_args(
            "online-done",
            &[
                ("lines", &taken.to_string()),
                ("refused", &refused.to_string()),
            ],
        );
        if taken > 0 {
            self.notify_ok(self.status.clone());
        }
        self.comment_cache.clear();
    }

    /// Paramètres ▸ Tester… (R-D01-01), in a thread; a profile as edited
    /// in the settings (D-03).
    pub(crate) fn online_test(&mut self, service: Service, edited: Option<Profile>) {
        let cfg = self.config.online.clone();
        let key = self.online.keys.get(&service).cloned().unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        let s = service.clone();
        std::thread::spawn(move || {
            let r = match edited {
                Some(p) => {
                    let mut c = cfg.clone();
                    if let Some(t) = p.timeout {
                        c.timeout_secs = t;
                    }
                    crate::online::Connector::test(&crate::online::openai::OpenAI::new(p), &c, &key)
                }
                None => crate::online::connector(&s).test(&cfg.for_service(&s), &key),
            };
            let _ = tx.send(r);
        });
        self.online.test_result.remove(&service);
        self.online.test = Some((service, rx));
    }

    /// The services of the settings: DeepL, Azure, the profiles as
    /// edited, Google.
    fn online_settings_services(&self) -> Vec<Service> {
        let mut v = vec![Service::DeepL, Service::Azure];
        v.extend(
            self.online
                .profiles
                .iter()
                .map(|p| Service::OpenAI(p.id.clone())),
        );
        v.push(Service::Google);
        v
    }

    /// Load the keys and the profiles into the settings (once per opening).
    pub(crate) fn online_load_keys(&mut self) {
        profile::reload();
        self.online.profiles = profile::all();
        self.online.profiles_dirty.clear();
        self.online.profiles_removed.clear();
        self.online.keys.clear();
        self.online.keys_loaded.clear();
        self.online.key_store.clear();
        for s in self.online_settings_services() {
            let (k, store) = crate::online::secrets::get(&s);
            let k = k.unwrap_or_default();
            self.online.keys.insert(s.clone(), k.clone());
            self.online.keys_loaded.insert(s.clone(), k);
            self.online.key_store.insert(s, store);
        }
        if !self
            .online_settings_services()
            .contains(&self.online.settings_service)
        {
            self.online.settings_service = Service::DeepL;
        }
    }

    /// Store the profiles and the keys that changed (Paramètres ▸
    /// Enregistrer).
    pub(crate) fn online_save_keys(&mut self) {
        let dir = profile::user_dir();
        for id in std::mem::take(&mut self.online.profiles_removed) {
            if let Err(e) = profile::remove(&dir, &id) {
                tracing::warn!("profile {id}: {e}");
            }
            // A profile removed loses its key; a restored one keeps it.
            if !self.online.profiles.iter().any(|p| p.id == id) {
                let s = Service::OpenAI(id);
                let _ = crate::online::secrets::set(&s, "");
                self.online.keys.remove(&s);
            }
        }
        let dirty: Vec<String> = self.online.profiles_dirty.drain().collect();
        for id in dirty {
            let Some(p) = self.online.profiles.iter().find(|p| p.id == id).cloned() else {
                continue;
            };
            match profile::save(&dir, &p) {
                Ok(path) => self.notify_ok(self.i18n.t_arg(
                    "online-profile-saved",
                    "path",
                    &path.display().to_string(),
                )),
                Err(e) => {
                    let reason = self.online_profile_invalid(&e);
                    self.notify_err(self.i18n.t_args(
                        "online-profile-invalid",
                        &[("name", &p.name), ("reason", &reason)],
                    ));
                }
            }
        }
        profile::reload();
        for s in self.online_settings_services() {
            let now = self.online.keys.get(&s).cloned().unwrap_or_default();
            let before = self.online.keys_loaded.get(&s).cloned().unwrap_or_default();
            if now.trim() != before.trim() {
                match crate::online::secrets::set(&s, &now) {
                    Ok(store) => {
                        self.online.key_store.insert(s.clone(), store);
                        self.online.keys_loaded.insert(s, now.trim().to_string());
                    }
                    Err(e) => self.notify_err(self.i18n.t_args(
                        "err-service",
                        &[("service", &s.label(&self.i18n)), ("error", &e.to_string())],
                    )),
                }
            }
        }
        // A default service whose profile is gone.
        if let Some(s) = &self.config.online.service
            && !Service::all().contains(s)
        {
            self.config.online.service = None;
        }
    }

    /// The first changed profile that cannot be saved, said.
    pub(crate) fn online_profiles_problem(&mut self) -> Option<String> {
        let bad = self
            .online
            .profiles
            .iter()
            .filter(|p| self.online.profiles_dirty.contains(&p.id))
            .find_map(|p| p.validate().err().map(|e| (p.clone(), e)))?;
        self.online.settings_service = Service::OpenAI(bad.0.id.clone());
        let reason = self.online_profile_invalid(&bad.1);
        Some(self.i18n.t_args(
            "online-profile-invalid",
            &[("name", &bad.0.name), ("reason", &reason)],
        ))
    }

    /// The words of an invalid profile (R-D03-07).
    pub(crate) fn online_profile_invalid(&self, e: &profile::Invalid) -> String {
        use profile::Invalid;
        match e {
            Invalid::Json(m) => self.i18n.t_arg("glossary-err-json", "error", m),
            Invalid::Format(n) => self.i18n.t_arg("glossary-err-format", "n", &n.to_string()),
            Invalid::Id(id) => self.i18n.t_arg("online-err-id", "id", id),
            Invalid::Endpoint => self.i18n.t("online-err-endpoint"),
            Invalid::Model => self.i18n.t("online-err-model"),
            Invalid::Range { field, min, max } => self.i18n.t_args(
                "online-err-range",
                &[
                    ("field", field),
                    ("min", &min.to_string()),
                    ("max", &max.to_string()),
                ],
            ),
        }
    }

    /// The test result as a sentence.
    pub(crate) fn online_test_text(&self, service: &Service) -> Option<(bool, String)> {
        let i18n = &self.i18n;
        let r = self.online.test_result.get(service)?;
        Some(match r {
            Ok(s) => {
                if let Some(rest) = s.strip_prefix("usage:") {
                    let (used, limit) = rest.split_once('/').unwrap_or((rest, "?"));
                    (
                        true,
                        i18n.t_args(
                            "online-test-usage",
                            &[
                                (
                                    "used",
                                    &crate::ui::export_dialog::fmt_count(
                                        i18n,
                                        used.parse().unwrap_or(0),
                                    ),
                                ),
                                (
                                    "limit",
                                    &crate::ui::export_dialog::fmt_count(
                                        i18n,
                                        limit.parse().unwrap_or(0),
                                    ),
                                ),
                            ],
                        ),
                    )
                } else if let Some(rest) = s.strip_prefix("control:") {
                    (true, i18n.t_arg("online-test-control", "text", rest))
                } else if let Some(rest) = s.strip_prefix("model:") {
                    let mut parts = rest.splitn(3, crate::online::openai::TEST_SEP);
                    let model = parts.next().unwrap_or("");
                    let known = parts.next().unwrap_or("") == "known";
                    let text = parts.next().unwrap_or("");
                    let mut m = i18n.t_arg("online-test-control", "text", text);
                    if !known {
                        m.push_str(&format!(
                            " — {}",
                            i18n.t_arg("online-test-model-unlisted", "model", model)
                        ));
                    }
                    (true, m)
                } else {
                    (true, i18n.t("online-test-ok"))
                }
            }
            Err(e) => (
                false,
                i18n.t_args(
                    "online-test-fail",
                    &[("code", &e.code().to_string()), ("message", &e.message())],
                ),
            ),
        })
    }

    /// The « Traduire en ligne… » dialog and the two confirmations.
    pub(crate) fn online_windows(&mut self, ctx: &egui::Context) {
        self.online_poll(ctx);
        // Confirmations first.
        if let Some((lines, n, show)) = self.online.confirm_overwrite.clone() {
            let msg = self
                .i18n
                .t_arg("online-overwrite-confirm", "n", &n.to_string());
            match self.simple_confirm(ctx, "ximod-online-overwrite", &msg) {
                Some(true) => {
                    self.online.confirm_overwrite = None;
                    // Drop the protected lines from the job.
                    let keep: Vec<usize> = {
                        let doc = self.active_doc();
                        lines
                            .iter()
                            .copied()
                            .filter(|&i| {
                                doc.and_then(|d| d.tm.entries.get(i)).is_some_and(|e| {
                                    e.trans.is_empty()
                                        || !matches!(
                                            crate::status::derive(e),
                                            Status::Validated | Status::Custom
                                        )
                                })
                            })
                            .collect()
                    };
                    // Oui = replace them too.
                    let _ = keep;
                    self.online_begin(lines, true, show);
                }
                Some(false) => self.online.confirm_overwrite = None,
                None => {}
            }
            return;
        }
        if let Some((lines, chars, show)) = self.online.confirm_session.clone() {
            let msg = self.i18n.t_args(
                "online-session-over",
                &[
                    ("chars", &self.online.session_chars.to_string()),
                    ("n", &chars.to_string()),
                ],
            );
            match self.simple_confirm(ctx, "ximod-online-session", &msg) {
                Some(true) => {
                    self.online.confirm_session = None;
                    self.online_begin(lines, true, show);
                }
                Some(false) => self.online.confirm_session = None,
                None => {}
            }
            return;
        }
        if !self.online.dialog_open {
            return;
        }
        self.online_dialog(ctx);
    }

    /// Oui / Non in a small modal; `Some(choice)` when answered.
    fn simple_confirm(&self, ctx: &egui::Context, id: &str, msg: &str) -> Option<bool> {
        let mut choice = None;
        let oui = self.i18n.t("btn-yes");
        let non = self.i18n.t("btn-no");
        egui::Window::new(self.i18n.t("online-title"))
            .id(egui::Id::new(id))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(4.0);
                ui.add(egui::Label::new(msg).wrap());
                ui.add_space(10.0);
                let (p, c) = super::confirm_buttons(ui, &oui, &non);
                if p {
                    choice = Some(true);
                } else if c {
                    choice = Some(false);
                }
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(false);
        }
        choice
    }

    /// The lines of the scope the cache will serve (R-D04-02), computed
    /// again only when the scope or its lines change.
    fn online_cache_estimate(&mut self, scope: Scope, lines: &[usize]) -> usize {
        let Some(service) = self.config.online.service.clone() else {
            return 0;
        };
        let only = self.online.only_untranslated;
        let id = service.id();
        let first = lines.first().copied().unwrap_or(usize::MAX);
        if let Some((s, sc, o, n, f, cached)) = &self.online.cache_estimate
            && *s == id
            && *sc == scope
            && *o == only
            && *n == lines.len()
            && *f == first
        {
            return *cached;
        }
        let cached = match (self.online_pair(), self.active_doc()) {
            (Some((src, dst)), Some(doc)) => {
                let path = crate::online::cache::Cache::path_for(&service, &src, &dst);
                if path.exists() {
                    let cache = crate::online::cache::Cache::open_at(path, &dst);
                    let opts = protect::Options {
                        tags: self.config.online.protect_tags,
                        numbers: self.config.online.protect_numbers,
                        newlines: self.config.online.protect_newlines,
                    };
                    mass::prepare(&doc.tm.entries, lines, doc.settings.game(), opts)
                        .cached_lines(&cache)
                } else {
                    0
                }
            }
            _ => 0,
        };
        self.online.cache_estimate = Some((id, scope, only, lines.len(), first, cached));
        cached
    }

    fn online_dialog(&mut self, ctx: &egui::Context) {
        let scope = self.online.scope.unwrap_or_default();
        let (lines, skipped) = self.online_scope_lines(scope, self.online.only_untranslated);
        let cached = self.online_cache_estimate(scope, &lines);
        let i18n = &self.i18n;
        let doc_name = self.active_doc().map(|d| d.file_name()).unwrap_or_default();
        let title = i18n.dash(&i18n.t("online-title"), &doc_name);
        let running = self.online.run.as_ref().is_some_and(|r| !r.finished);
        let n_sel = self.selection.targets().len();
        let n_view = self.visible.len();
        let (n_doc, n_total) = (
            self.online_scope_lines(Scope::Document, true).0.len(),
            self.active_doc().map(|d| d.tm.entries.len()).unwrap_or(0),
        );
        // The estimate of the scope.
        let (est_lines, est_chars, est_tags, est_nums, est_nl, est_requests, est_unique) = {
            let doc = self.active_doc();
            let cfg = &self.online_cfg();
            let opts = protect::Options {
                tags: cfg.protect_tags,
                numbers: cfg.protect_numbers,
                newlines: cfg.protect_newlines,
            };
            let mut seen = std::collections::HashSet::new();
            let mut items = Vec::new();
            let (mut tags, mut nums, mut nl) = (0, 0, 0);
            if let Some(doc) = doc {
                let game = doc.settings.game();
                for &i in &lines {
                    if let Some(e) = doc.tm.entries.get(i) {
                        if !seen.insert(e.source.as_str()) {
                            continue;
                        }
                        let set = TagSet::for_entry(e, game);
                        let (p, toks) = protect::protect(&e.source, &set, opts);
                        for t in &toks {
                            if t.name.starts_with("a_") {
                                tags += 1;
                            } else if t.name.starts_with("n_") {
                                nums += 1;
                            } else {
                                nl += 1;
                            }
                        }
                        items.push(Item {
                            key: items.len(),
                            text: p,
                        });
                    }
                }
            }
            let plan = crate::online::quota::plan(items, cfg.max_chars, cfg.max_lines);
            (
                lines.len(),
                plan.chars(),
                tags,
                nums,
                nl,
                plan.batches.len() + plan.too_long.len(),
                plan.lines() + plan.too_long.len(),
            )
        };
        let service_label = self
            .config
            .online
            .service
            .as_ref()
            .map(|s| s.label(i18n))
            .unwrap_or_else(|| i18n.t("online-key-none"));
        let pair = self
            .active_doc()
            .map(|d| {
                format!(
                    "{} {} {}",
                    i18n.display_name(&d.settings.source),
                    crate::icons::ARROW_RIGHT,
                    i18n.display_name(&d.settings.target)
                )
            })
            .unwrap_or_default();
        let cfg = self.online_cfg();
        let session_chars = self.online.session_chars;
        let mut start = false;
        let mut stop = false;
        let mut close = false;
        let mut new_scope = scope;
        let mut only = self.online.only_untranslated;
        let log = self.online.log.clone();
        let reasons: Vec<(String, usize)> = self
            .online
            .run
            .as_ref()
            .and_then(|r| r.summary.as_ref())
            .map(|s| {
                s.by_reason
                    .iter()
                    .map(|(r, n)| (self.online_reason_text(r), *n))
                    .collect()
            })
            .unwrap_or_default();
        let run_info = self.online.run.as_ref().map(|r| {
            (
                r.done,
                r.total,
                r.received,
                r.cached,
                r.finished,
                r.stopped,
                r.summary.clone(),
            )
        });
        egui::Window::new(title)
            .id(egui::Id::new("ximod-online-dialog"))
            .collapsible(false)
            .resizable(true)
            .order(egui::Order::Foreground)
            .default_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(620.0);
                ui.add_enabled_ui(!running, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(i18n.t("online-scope")).strong());
                        ui.radio_value(&mut new_scope, Scope::Line, i18n.t("online-scope-line"));
                        ui.radio_value(
                            &mut new_scope,
                            Scope::Selection,
                            i18n.t_arg("online-scope-selection", "n", &n_sel.to_string()),
                        );
                        ui.radio_value(
                            &mut new_scope,
                            Scope::View,
                            i18n.t_arg("online-scope-view", "n", &n_view.to_string()),
                        );
                        ui.radio_value(
                            &mut new_scope,
                            Scope::Document,
                            i18n.t_args(
                                "online-scope-document",
                                &[("n", &n_doc.to_string()), ("total", &n_total.to_string())],
                            ),
                        );
                    });
                    ui.checkbox(&mut only, i18n.t("online-only-untranslated"));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(i18n.t("online-service")).strong());
                        ui.label(&service_label);
                        ui.label(egui::RichText::new(&pair).weak());
                    });
                });
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(i18n.t("online-estimate-title")).strong());
                    ui.label(i18n.t_args(
                        "online-estimate",
                        &[
                            ("lines", &est_lines.to_string()),
                            ("unique", &est_unique.to_string()),
                            ("chars", &est_chars.to_string()),
                            ("requests", &est_requests.to_string()),
                            ("batch", &cfg.max_lines.to_string()),
                            ("max", &cfg.max_chars.to_string()),
                        ],
                    ));
                    ui.label(
                        egui::RichText::new(i18n.t_args(
                            "online-protected",
                            &[
                                ("tags", &est_tags.to_string()),
                                ("numbers", &est_nums.to_string()),
                                ("newlines", &est_nl.to_string()),
                            ],
                        ))
                        .weak(),
                    );
                    ui.label(
                        egui::RichText::new(i18n.t_args(
                            "online-skipped",
                            &[
                                ("locked", &skipped.locked.to_string()),
                                ("done", &skipped.done.to_string()),
                            ],
                        ))
                        .weak(),
                    );
                    if cached > 0 {
                        ui.label(
                            egui::RichText::new(i18n.t_arg(
                                "online-cache-will",
                                "n",
                                &cached.to_string(),
                            ))
                            .weak(),
                        );
                    }
                    ui.label(
                        egui::RichText::new(i18n.t_args(
                            "online-session",
                            &[
                                ("chars", &session_chars.to_string()),
                                ("limit", &cfg.session_warn.to_string()),
                            ],
                        ))
                        .weak(),
                    );
                    ui.label(egui::RichText::new(i18n.t("online-status-hint")).weak());
                });
                ui.add_space(6.0);
                if let Some((done, total, received, cached, finished, stopped, summary)) = run_info
                {
                    let frac = if total == 0 {
                        1.0
                    } else {
                        done as f32 / total as f32
                    };
                    ui.add(egui::ProgressBar::new(frac).text(i18n.t_args(
                        "online-progress",
                        &[
                            ("done", &done.to_string()),
                            ("total", &total.to_string()),
                            ("lines", &received.to_string()),
                        ],
                    )));
                    if cached > 0 {
                        ui.label(
                            egui::RichText::new(i18n.t_arg(
                                "online-cached",
                                "n",
                                &cached.to_string(),
                            ))
                            .weak(),
                        );
                    }
                    if finished {
                        let sm = summary.unwrap_or_default();
                        let mut s = i18n.t_args(
                            "online-done-detail",
                            &[
                                ("received", &sm.received.to_string()),
                                ("refused", &sm.refused.to_string()),
                                ("left", &sm.left.to_string()),
                            ],
                        );
                        if stopped {
                            s.push_str(&format!(
                                " — {}",
                                i18n.t_arg("online-stopped", "done", &done.to_string())
                            ));
                        }
                        ui.label(s);
                        for (text, n) in &reasons {
                            ui.label(
                                egui::RichText::new(i18n.t_args(
                                    "online-refused-by",
                                    &[("reason", text), ("n", &n.to_string())],
                                ))
                                .small()
                                .color(ui.visuals().error_fg_color),
                            );
                        }
                    }
                    if !log.is_empty() {
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                egui::Grid::new("ximod-online-log")
                                    .num_columns(3)
                                    .striped(true)
                                    .spacing([8.0, 2.0])
                                    .show(ui, |ui| {
                                        for row in &log {
                                            match &row.result {
                                                Ok(t) => {
                                                    ui.label(
                                                        egui::RichText::new(crate::icons::CHECK)
                                                            .color(
                                                                crate::ui::theme::Palette::of(
                                                                    ui.visuals(),
                                                                )
                                                                .success,
                                                            ),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(&row.source).small(),
                                                    );
                                                    ui.label(egui::RichText::new(t).small());
                                                }
                                                Err(e) => {
                                                    ui.label(
                                                        egui::RichText::new(crate::icons::X)
                                                            .color(ui.visuals().error_fg_color),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(&row.source).small(),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(e)
                                                            .small()
                                                            .color(ui.visuals().error_fg_color),
                                                    );
                                                }
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Fermer waits for the end of the sending (R-D04-03).
                        if ui
                            .add_enabled(!running, egui::Button::new(i18n.t("btn-close")))
                            .clicked()
                        {
                            close = true;
                        }
                        if ui
                            .add_enabled(running, egui::Button::new(i18n.t("online-stop")))
                            .clicked()
                        {
                            stop = true;
                        }
                        if ui
                            .add_enabled(
                                !running && est_lines > 0,
                                egui::Button::new(i18n.t("online-translate")),
                            )
                            .clicked()
                        {
                            start = true;
                        }
                    });
                });
            });
        self.online.scope = Some(new_scope);
        self.online.only_untranslated = only;
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !running {
            close = true;
        }
        // Entrée = Traduire, once per opening.
        if !running
            && est_lines > 0
            && self.online.run.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            start = true;
        }
        if stop && let Some(r) = &self.online.run {
            r.stop.store(true, Ordering::Relaxed);
        }
        if start {
            self.online.run = None;
            if self.config.online.last_scope != scope.id() {
                self.config.online.last_scope = scope.id().to_string();
                let _ = self.config.save();
            }
            self.online_begin(lines, false, true);
        }
        if close {
            self.online.dialog_open = false;
            if self.online.run.as_ref().is_some_and(|r| r.finished) {
                self.online.run = None;
            }
        }
    }

    /// Outils ▸ Traduire en ligne… (Ctrl+Maj+G).
    pub(crate) fn open_online_dialog(&mut self) {
        if self.active_doc().is_none() {
            self.status = self.i18n.t("status-no-doc");
            return;
        }
        if self.online_usable().is_none() {
            self.status = self.i18n.t("online-no-service");
            self.notify_warn(self.status.clone());
            return;
        }
        self.online.only_untranslated = true;
        // A selection first; otherwise the scope chosen last, the whole
        // document by default (R-D04-01).
        let last = Scope::from_id(&self.config.online.last_scope);
        self.online.scope = Some(if self.selection.targets().len() > 1 {
            Scope::Selection
        } else {
            match last {
                Scope::Selection => Scope::Document,
                Scope::Line if self.selection.cursor.is_none() => Scope::Document,
                other => other,
            }
        });
        self.online.cache_estimate = None;
        // A finished run is not shown again.
        if self.online.run.as_ref().is_some_and(|r| r.finished) {
            self.online.run = None;
            self.online.log.clear();
        }
        self.online.dialog_open = true;
    }
}

impl XimodApp {
    /// Paramètres ▸ En ligne (figure B.1): the services, the selected
    /// one's key and options, the guards, the network.
    pub(crate) fn settings_online(&mut self, ui: &mut egui::Ui) {
        // The texts first: the body borrows `self` mutably.
        let keys = [
            "online-services",
            "online-default-service",
            "online-key-none",
            "online-key",
            "online-hide",
            "online-show",
            "online-key-keyring",
            "online-key-file",
            "online-key-optional",
            "online-endpoint",
            "online-formality",
            "online-region",
            "online-model",
            "online-openai-hint",
            "online-google-hint",
            "online-test",
            "online-guards",
            "online-max-chars",
            "online-max-lines",
            "online-max-rpm",
            "online-warn-session",
            "online-warn-session-unit",
            "online-protect-tags",
            "online-protect-numbers",
            "online-protect-newlines",
            "online-network",
            "online-proxy",
            "online-timeout",
            "online-clear-cache",
            "online-settings-hint",
        ];
        let texts: HashMap<&str, String> = keys.iter().map(|k| (*k, self.i18n.t(k))).collect();
        let t = |k: &str| texts.get(k).cloned().unwrap_or_else(|| k.to_string());
        let lw = 150.0_f32;
        ui.label(egui::RichText::new(t("online-services")).strong());
        ui.add_space(2.0);
        // The services, the usable ones marked; a service per profile.
        let services = self.online_settings_services();
        let mut pick = self.online.settings_service.clone();
        ui.horizontal_wrapped(|ui| {
            for s in &services {
                let has_key = self
                    .online
                    .keys
                    .get(s)
                    .is_some_and(|k| !k.trim().is_empty());
                let usable = has_key || self.online_settings_key_optional(s);
                let mark = if usable {
                    crate::icons::CHECK
                } else {
                    crate::icons::DOT
                };
                let label = format!("{mark} {}", self.online_settings_label(s));
                if ui.selectable_label(pick == *s, label).clicked() {
                    pick = s.clone();
                }
            }
        });
        self.online.settings_service = pick.clone();
        ui.add_space(4.0);
        // Default service.
        ui.horizontal(|ui| {
            self.settings_label(ui, lw, "online-default-service");
            let cur = self
                .temp_online
                .service
                .as_ref()
                .map(|s| self.online_settings_label(s))
                .unwrap_or_else(|| t("online-key-none"));
            let labels: Vec<(Service, String)> = services
                .iter()
                .map(|s| (s.clone(), self.online_settings_label(s)))
                .collect();
            egui::ComboBox::from_id_salt("ximod-online-default")
                .selected_text(cur)
                .width(260.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.temp_online.service, None, " ");
                    for (s, l) in labels {
                        ui.selectable_value(&mut self.temp_online.service, Some(s), l);
                    }
                });
        });
        ui.add_space(6.0);
        let s = pick;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(self.online_settings_label(&s)).strong());
            ui.add_space(2.0);
            if let Service::OpenAI(id) = &s {
                self.settings_profile(ui, id, lw);
            }
            ui.horizontal(|ui| {
                self.settings_label(ui, lw, "online-key");
                let key = self.online.keys.entry(s.clone()).or_default();
                ui.add(
                    egui::TextEdit::singleline(key)
                        .password(!self.online.show_key)
                        .desired_width(300.0),
                );
                if ui
                    .button(if self.online.show_key {
                        t("online-hide")
                    } else {
                        t("online-show")
                    })
                    .clicked()
                {
                    self.online.show_key = !self.online.show_key;
                }
            });
            let store_key = match self.online.key_store.get(&s) {
                Some(crate::online::secrets::Store::Keyring) => "online-key-keyring",
                Some(crate::online::secrets::Store::File) => "online-key-file",
                _ if self.online_settings_key_optional(&s) => "online-key-optional",
                _ => "online-key-none",
            };
            ui.label(egui::RichText::new(t(store_key)).weak().small());
            match &s {
                Service::DeepL => {
                    ui.horizontal(|ui| {
                        self.settings_label(ui, lw, "online-endpoint");
                        egui::ComboBox::from_id_salt("ximod-deepl-endpoint")
                            .selected_text(&self.temp_online.deepl_endpoint)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for e in ["api-free.deepl.com", "api.deepl.com"] {
                                    ui.selectable_value(
                                        &mut self.temp_online.deepl_endpoint,
                                        e.to_string(),
                                        e,
                                    );
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        self.settings_label(ui, lw, "online-formality");
                        egui::ComboBox::from_id_salt("ximod-deepl-formality")
                            .selected_text(&self.temp_online.deepl_formality)
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                for e in ["default", "prefer_more", "prefer_less", "more", "less"] {
                                    ui.selectable_value(
                                        &mut self.temp_online.deepl_formality,
                                        e.to_string(),
                                        e,
                                    );
                                }
                            });
                    });
                }
                Service::Azure => {
                    ui.horizontal(|ui| {
                        self.settings_label(ui, lw, "online-region");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.temp_online.azure_region)
                                .hint_text("westeurope")
                                .desired_width(200.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        self.settings_label(ui, lw, "online-endpoint");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.temp_online.azure_endpoint)
                                .desired_width(360.0),
                        );
                    });
                }
                Service::OpenAI(_) => {
                    ui.label(egui::RichText::new(t("online-openai-hint")).weak().small());
                }
                Service::Google => {
                    ui.label(egui::RichText::new(t("online-google-hint")).weak().small());
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let testing = self.online.test.is_some();
                if ui
                    .add_enabled(!testing, egui::Button::new(t("online-test")))
                    .clicked()
                {
                    // The test uses the settings and the profile as typed.
                    let edited = match &s {
                        Service::OpenAI(id) => {
                            self.online.profiles.iter().find(|p| p.id == *id).cloned()
                        }
                        _ => None,
                    };
                    let saved =
                        std::mem::replace(&mut self.config.online, self.temp_online.clone());
                    self.online_test(s.clone(), edited);
                    self.config.online = saved;
                }
                if testing {
                    ui.spinner();
                }
                if let Some((ok, text)) = self.online_test_text(&s) {
                    let col = if ok {
                        crate::ui::theme::Palette::of(ui.visuals()).success
                    } else {
                        ui.visuals().error_fg_color
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).color(col)).wrap());
                }
            });
        });
        ui.add_space(6.0);
        // Guards (D-02).
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(t("online-guards")).strong());
            ui.horizontal(|ui| {
                ui.label(t("online-max-chars"));
                let mut v = self.temp_online.max_chars as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).range(100..=100_000).speed(50))
                    .changed()
                {
                    self.temp_online.max_chars = v as usize;
                }
                ui.add_space(12.0);
                ui.label(t("online-max-lines"));
                let mut v = self.temp_online.max_lines as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).range(1..=1000).speed(1))
                    .changed()
                {
                    self.temp_online.max_lines = v as usize;
                }
                ui.add_space(12.0);
                ui.label(t("online-max-rpm"));
                let mut v = self.temp_online.max_rpm as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).range(1..=600).speed(1))
                    .changed()
                {
                    self.temp_online.max_rpm = v as u32;
                }
            });
            ui.horizontal(|ui| {
                ui.label(t("online-warn-session"));
                let mut v = self.temp_online.session_warn as i32;
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .range(0..=100_000_000)
                            .speed(1000),
                    )
                    .changed()
                {
                    self.temp_online.session_warn = v as usize;
                }
                ui.label(t("online-warn-session-unit"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.temp_online.protect_tags, t("online-protect-tags"));
                ui.checkbox(
                    &mut self.temp_online.protect_numbers,
                    t("online-protect-numbers"),
                );
                ui.checkbox(
                    &mut self.temp_online.protect_newlines,
                    t("online-protect-newlines"),
                );
            });
        });
        ui.add_space(6.0);
        // Network.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(t("online-network")).strong());
            ui.horizontal(|ui| {
                self.settings_label(ui, lw, "online-proxy");
                let hint = std::env::var("HTTPS_PROXY")
                    .or_else(|_| std::env::var("https_proxy"))
                    .map(|p| self.i18n.colon("HTTPS_PROXY", &p))
                    .unwrap_or_else(|_| "http://proxy:3128".to_string());
                ui.add(
                    egui::TextEdit::singleline(&mut self.temp_online.proxy)
                        .hint_text(hint)
                        .desired_width(360.0),
                );
            });
            ui.horizontal(|ui| {
                self.settings_label(ui, lw, "online-timeout");
                let mut v = self.temp_online.timeout_secs as i32;
                if ui
                    .add(egui::DragValue::new(&mut v).range(5..=600).speed(1))
                    .changed()
                {
                    self.temp_online.timeout_secs = v as u64;
                }
                ui.add_space(16.0);
                if ui.button(t("online-clear-cache")).clicked() {
                    let n = crate::online::cache::Cache::clear_all();
                    self.notify_ok(self.i18n.t_arg("online-cache-cleared", "n", &n.to_string()));
                }
            });
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t("online-settings-hint"))
                .weak()
                .small(),
        );
    }
}
