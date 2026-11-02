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
}

impl Out<'_> {
    /// A reply: its object on stdout with `--json`, its lines otherwise;
    /// its messages on stderr.
    pub fn reply(&mut self, i18n: &crate::i18n::I18n, r: &Reply) {
        if !self.quiet {
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
        let _ = writeln!(self.err, "{msg}");
        if self.json {
            let j = serde_json::json!({ "command": command, "code": FAIL, "error": msg });
            let _ = writeln!(self.out, "{j}");
        }
    }
}

fn init_log(verbose: bool) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let filter = if verbose {
        "ximod_translator=debug"
    } else {
        "ximod_translator=error"
    };
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(tracing_subscriber::EnvFilter::new(filter))
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
    init_log(g.verbose);
    let mut o = Out {
        out,
        err,
        json: g.json,
        quiet: g.quiet,
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
    match commands::run(&mut ctx, &g, &command, &p.files, &p.opts) {
        Ok(r) => {
            o.reply(&ctx.i18n, &r);
            r.code
        }
        Err(f) => {
            o.fail(&ctx.i18n, &command, &f);
            FAIL
        }
    }
}
