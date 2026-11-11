//! `ximod-cli`, the command line (D-06, decisions D-91 / D-92): the
//! operations of the library — those the interface calls — as commands,
//! with return codes (0 done, 1 done with reservations, 2 not done), a
//! readable or JSON output and batch files. The configuration is read as
//! the interface reads it and never written; no window, no recent files,
//! no autosave.

pub mod args;
pub mod batch;
pub mod commands;
pub mod work;

use std::io::Write;

use serde_json::{Map, Value};

use self::args::{ArgError, Parsed};
use self::work::Ctx;

/// Done.
pub const OK: i32 = 0;
/// Done with reservations (check errors, lines left with `--strict`,
/// translate stopped by the threshold).
pub const WARN: i32 = 1;
/// Not done (file missing or unreadable, unknown option, invalid format,
/// service unreachable, write refused).
pub const FAIL: i32 = 2;

/// What a command says: its code, its lines for a reader, its messages for
/// the error output, its object for `--json`.
#[derive(Debug, Default)]
pub struct Reply {
    pub code: i32,
    pub lines: Vec<String>,
    pub notes: Vec<String>,
    pub json: Map<String, Value>,
}

impl Reply {
    pub fn new(command: &str) -> Reply {
        let mut json = Map::new();
        json.insert("command".into(), Value::from(command));
        Reply {
            code: OK,
            json,
            ..Default::default()
        }
    }

    pub fn set(&mut self, key: &str, v: impl Into<Value>) {
        self.json.insert(key.to_string(), v.into());
    }

    pub fn line(&mut self, s: String) {
        self.lines.push(s);
    }

    pub fn worse(&mut self, code: i32) {
        self.code = self.code.max(code);
    }
}

/// A command not done: the message (an encoded one is translated when
/// shown, crate::l10n).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail(pub String);

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail(s)
    }
}

pub type Res = Result<Reply, Fail>;

/// The options common to the commands of a session.
#[derive(Debug, Clone, Default)]
pub struct Global {
    pub json: bool,
    pub quiet: bool,
    pub verbose: bool,
    /// `--log <file>`: the journal appended to that file (E-01).
    pub log: Option<String>,
    pub game: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl Global {
    fn of(p: &Parsed) -> Global {
        let c = &p.common;
        Global {
            json: c.has("json"),
            quiet: c.has("quiet"),
            verbose: c.has("verbose"),
            log: c.get("log").map(str::to_string),
            game: c.get("game").map(str::to_string),
            from: c.get("from").map(str::to_string),
            to: c.get("to").map(str::to_string),
        }
    }
}

/// Where the output goes; the messages are translated in the language of
/// the session.
pub struct Out<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub json: bool,
    pub quiet: bool,
    /// `--verbose`: the journal on stderr, as the run goes.
    pub verbose: bool,
    /// `--log`: the journal appended to that file.
    pub log: Option<std::fs::File>,
}

impl Out<'_> {
    /// A reply: its object on stdout with `--json`, its lines otherwise;
    /// its messages on stderr.
    pub fn reply(&mut self, i18n: &crate::i18n::I18n, r: &Reply) {
        // With --verbose, the messages already went through the journal.
        if !self.quiet && !self.verbose {
            for n in &r.notes {
                let _ = writeln!(self.err, "{}", i18n.localize(n));
            }
        }
        if self.json {
            let mut j = r.json.clone();
            j.insert("code".into(), Value::from(r.code));
            let _ = writeln!(
                self.out,
                "{}",
                serde_json::to_string(&Value::Object(j)).unwrap_or_default()
            );
        } else if !self.quiet {
            for l in &r.lines {
                let _ = writeln!(self.out, "{}", i18n.localize(l));
            }
        }
    }

    /// A command not done: its message on stderr (and its object with
    /// `--json`).
    pub fn fail(&mut self, i18n: &crate::i18n::I18n, command: &str, f: &Fail) {
        let msg = i18n.localize(&f.0);
        if !self.verbose {
            let _ = writeln!(self.err, "{msg}");
        }
        if self.json {
            let j = serde_json::json!({ "command": command, "code": FAIL, "error": msg });
            let _ = writeln!(self.out, "{j}");
        }
    }
}

impl Out<'_> {
    /// The new entries of the journal: on stderr with `--verbose`, in the
    /// file of `--log`; their objects for `--json` (R-E01-07).
    pub fn journal(&mut self, ctx: &mut work::Ctx) -> Vec<Value> {
        let entries = ctx.journal.take_unflushed();
        if entries.is_empty() {
            return Vec::new();
        }
        let text = crate::journal::tsv(&entries, |l| ctx.i18n.t(l.i18n_key()));
        if self.verbose {
            let _ = write!(self.err, "{text}");
        }
        if let Some(f) = &mut self.log {
            let _ = f.write_all(text.as_bytes());
        }
        entries.iter().map(crate::journal::json).collect()
    }
}

/// The nature of the entries of a command.
pub fn journal_kind(command: &str) -> crate::journal::Kind {
    use crate::journal::Kind;
    match command {
        "import" | "info" => Kind::Open,
        "apply" => Kind::Apply,
        "assist" => Kind::Assist,
        "translate" => Kind::Online,
        "check" => Kind::Check,
        "export" | "manifest" => Kind::Export,
        "package" => Kind::Package,
        "scan" | "compare" => Kind::Scan,
        "convert" => Kind::Tool,
        _ => Kind::Message,
    }
}

/// The report of a command in the journal: its lines (info), its messages
/// (warnings) or its failure (error).
pub fn journal_result(
    ctx: &mut work::Ctx,
    command: &str,
    doc: Option<&str>,
    res: Result<&Reply, &Fail>,
) {
    use crate::journal::Level;
    let kind = journal_kind(command);
    match res {
        Ok(r) => {
            for l in &r.lines {
                let t = ctx.i18n.localize(l);
                ctx.journal.push(Level::Info, kind, doc, &t);
            }
            for n in &r.notes {
                let t = ctx.i18n.localize(n);
                ctx.journal.push(Level::Warn, kind, doc, &t);
            }
        }
        Err(f) => {
            let t = ctx.i18n.localize(&f.0);
            ctx.journal.push(Level::Error, kind, doc, &t);
        }
    }
}

/// The file name of a path, for the journal.
fn doc_name(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The technical trace: `RUST_LOG` alone sets it; the journal does not go
/// there (it has its own outputs, --verbose and --log).
fn init_log() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ximod_translator=error"));
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(filter)
        .try_init();
}

/// The folders, before anything is read: a portable folder given to
/// `--config` is taken as the folder of the executable (R-D06-01).
fn init_paths(config: Option<&str>) {
    let dir = config.map(std::path::PathBuf::from);
    if let Some(d) = &dir
        && d.join(crate::config::paths::CONFIG_FILE).is_file()
        && d.join("assets").is_dir()
    {
        // SAFETY: called once at the start of the program, before any
        // thread is started.
        unsafe { std::env::set_var(crate::config::paths::EXE_DIR_ENV, d) };
        crate::config::paths::init_read_only(None);
        return;
    }
    crate::config::paths::init_read_only(dir.as_deref());
}

fn session(p: &Parsed) -> Ctx {
    init_paths(p.common.get("config"));
    let cfg = crate::config::AppConfig::load_read_only();
    let mut i18n = crate::i18n::I18n::new();
    let lang = p
        .common
        .get("lang")
        .map(str::to_string)
        .unwrap_or_else(|| cfg.locale.clone());
    i18n.set_locale(if lang.trim().is_empty() { "eng" } else { &lang });
    Ctx {
        formats: crate::textfmt::Formats::load(),
        cfg,
        cache: Default::default(),
        i18n,
        journal: crate::journal::Journal::new().without_trace(),
    }
}

/// The help of the program or of a command, from the language files.
pub fn help(i18n: &crate::i18n::I18n, command: Option<&str>) -> Vec<String> {
    let mut v = Vec::new();
    match command {
        None => {
            v.push(i18n.t("cli-usage"));
            v.push(i18n.t("cli-commands"));
            for c in args::COMMANDS {
                v.push(format!(
                    "  {:<10} {}",
                    c.name,
                    i18n.t(&format!("cli-help-{}", c.name))
                ));
            }
            v.push(i18n.t("cli-options-common"));
            v.push(i18n.t("cli-help-hint"));
        }
        Some(c) => {
            v.push(i18n.t(&format!("cli-syntax-{c}")));
            v.push(i18n.t(&format!("cli-help-{c}")));
            v.push(i18n.t("cli-options-common"));
        }
    }
    v
}

/// Run a command line (the program name left out); the return code.
pub fn run(argv: Vec<String>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let parsed = args::parse(&argv);
    let p = match &parsed {
        Ok(p) => p.clone(),
        Err(_) => Parsed::default(),
    };
    // The common options read even when the line is refused (language,
    // configuration, JSON).
    let pre = args::parse_common_only(&argv);
    let mut ctx = session(if parsed.is_ok() { &p } else { &pre });
    let g = Global::of(if parsed.is_ok() { &p } else { &pre });
    init_log();
    let log = match &g.log {
        Some(p) => match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            Ok(f) => Some(f),
            Err(e) => {
                let msg = ctx
                    .i18n
                    .localize(&crate::l10n!("cli-err-log", path = p, error = e));
                let _ = writeln!(err, "{msg}");
                return FAIL;
            }
        },
        None => None,
    };
    let mut o = Out {
        out,
        err,
        json: g.json,
        quiet: g.quiet,
        verbose: g.verbose,
        log,
    };
    let p = match parsed {
        Ok(p) => p,
        Err(e) => {
            let msg = match e {
                ArgError::Unknown(a) | ArgError::NoValue(a) => {
                    crate::l10n!("cli-err-usage", arg = a)
                }
                ArgError::Command(a) => crate::l10n!("cli-err-command", arg = a),
            };
            o.fail(&ctx.i18n, "", &Fail(msg));
            return FAIL;
        }
    };
    if p.version {
        let _ = writeln!(o.out, "{} {}", crate::APP_NAME, crate::APP_VERSION);
        return OK;
    }
    let Some(command) = p.command.clone() else {
        for l in help(&ctx.i18n, None) {
            let _ = writeln!(o.out, "{l}");
        }
        return OK;
    };
    if p.common.has("help") {
        for l in help(&ctx.i18n, Some(&command)) {
            let _ = writeln!(o.out, "{l}");
        }
        return OK;
    }
    let spec = args::command(&command).expect("parsed command");
    if p.files.len() < spec.files.0 || p.files.len() > spec.files.1 {
        o.fail(
            &ctx.i18n,
            &command,
            &Fail(crate::l10n!("cli-err-positional", command = command)),
        );
        return FAIL;
    }
    if command == "batch" {
        return batch::run(&mut ctx, &g, &p.files[0], &mut o);
    }
    let res = commands::run(&mut ctx, &g, &command, &p.files, &p.opts);
    let doc = doc_name(&p.files[0]);
    journal_result(&mut ctx, &command, Some(&doc), res.as_ref());
    let entries = o.journal(&mut ctx);
    match res {
        Ok(mut r) => {
            if g.json && g.verbose {
                r.set("journal", Value::Array(entries));
            }
            o.reply(&ctx.i18n, &r);
            r.code
        }
        Err(f) => {
            o.fail(&ctx.i18n, &command, &f);
            FAIL
        }
    }
}
