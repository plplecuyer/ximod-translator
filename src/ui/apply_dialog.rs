//! Appliquer les dictionnaires (B-06): state of the dialog; the drawing
//! and the operation live in `app.rs`, the engine in `apply.rs`.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::apply::{Options, Plan};
use crate::replace::Scope;

/// Where the translations come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Stack,
    File(PathBuf),
}

pub enum Msg {
    Progress(usize, usize),
    Done(Plan),
}

pub struct Job {
    pub rx: Receiver<Msg>,
    pub progress: f32,
}

/// What the plan was computed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanKey {
    pub serial: u64,
    pub revision: u64,
    pub scope: Scope,
    pub lines: usize,
    pub opts: Options,
    pub source: Source,
}

pub struct ApplyDialog {
    pub open: bool,
    pub source: Source,
    /// The file picked last, kept for the session.
    pub last_file: Option<PathBuf>,
    pub scope: Scope,
    pub opts: Options,
    pub plan: Option<Plan>,
    pub plan_key: Option<PlanKey>,
    pub job: Option<Job>,
    /// Time of the last change (plan recomputed 250 ms later).
    pub changed_at: Option<f64>,
    pub message: Option<String>,
}

impl Default for ApplyDialog {
    fn default() -> Self {
        ApplyDialog {
            open: false,
            source: Source::Stack,
            last_file: None,
            scope: Scope::Document,
            opts: Options::default(),
            plan: None,
            plan_key: None,
            job: None,
            changed_at: None,
            message: None,
        }
    }
}

impl ApplyDialog {
    /// Open with a scope and, from the context menu, preset rules; the
    /// other settings stay as last used (R-B06-08).
    pub fn open_with(&mut self, scope: Scope, preset: Option<Options>) {
        self.open = true;
        self.scope = scope;
        if let Some(o) = preset {
            self.opts = o;
        }
        self.plan = None;
        self.plan_key = None;
        self.job = None;
        self.changed_at = None;
        self.message = None;
    }
}
