//! Batch files (D-06, R-D06-03, decision D-92): the commands chained on
//! several files. The keys at the head hold for every job, a job may
//! override them; each step is a command with its options; `"files"`
//! takes a pattern. The whole file is validated before the first step —
//! format, files, steps, options, values — so that an invalid batch writes
//! nothing. `stopOnError` stops at the first code 2; otherwise the batch
//! goes on and its code is the highest met.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::args::Values;
use super::commands;
use super::work::{Pair, Source, Work};
use super::{FAIL, Fail, Global, OK, Out, Reply, WARN};
use crate::headless::TreeExtra;
use crate::l10n;

/// The file format of the batches.
pub const FORMAT: u64 = 1;

/// The steps a job may chain.
pub const STEPS: &[&str] = &[
    "info",
    "apply",
    "assist",
    "translate",
    "check",
    "export",
    "manifest",
    "package",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    pub opts: Values,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub file: PathBuf,
    pub pair: Pair,
    /// The folder of the job's outputs: `<out>/<file name without extension>`.
    pub out: PathBuf,
    pub stop_on_error: bool,
    pub steps: Vec<Step>,
}

/// `writeDict` → `write-dict`.
fn kebab(k: &str) -> String {
    let mut s = String::new();
    for c in k.chars() {
        if c.is_ascii_uppercase() {
            s.push('-');
            s.push(c.to_ascii_lowercase());
        } else {
            s.push(c);
        }
    }
    s
}

/// `*` and `?` in a file name, ignoring case.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The files a pattern takes (the wildcards in the file name only), in
/// the order of their names.
pub fn expand(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let full = base.join(pattern);
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.contains(['*', '?']) {
        return if full.is_file() {
            vec![full]
        } else {
            Vec::new()
        };
    }
    let dir = full.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| glob_match(&name, n))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn step_of(v: &Value) -> Result<Step, String> {
    let (name, opts) = match v {
        Value::String(s) => (s.clone(), None),
        Value::Object(m) if m.len() == 1 => {
            let (k, o) = m.iter().next().expect("one key");
            (k.clone(), Some(o))
        }
        other => return Err(l10n!("cli-batch-step-unknown", step = other)),
    };
    if !STEPS.contains(&name.as_str()) {
        return Err(l10n!("cli-batch-step-unknown", step = name));
    }
    let allowed = super::args::step_options(&name).unwrap_or_default();
    let mut values = Values::default();
    if let Some(o) = opts {
        let Value::Object(m) = o else {
            return Err(l10n!("cli-batch-step-unknown", step = name));
        };
        for (k, v) in m {
            let key = kebab(k);
            if !allowed.contains(&key.as_str()) {
                return Err(l10n!("cli-batch-option", step = name, option = k));
            }
            match v {
                Value::Bool(true) => values.set(&key, ""),
                Value::Bool(false) | Value::Null => {}
                Value::String(s) => values.set(&key, s),
                Value::Number(n) => values.set(&key, &n.to_string()),
                Value::Array(a) => {
                    let items: Vec<String> = a
                        .iter()
                        .map(|x| match x {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    if key == "statuses" {
                        values.set(&key, &items.join(","));
                    } else {
                        for i in items {
                            values.set(&key, &i);
                        }
                    }
                }
                Value::Object(_) => {
                    return Err(l10n!("cli-batch-option", step = name, option = k));
                }
            }
        }
    }
    commands::validate(&name, &values).map_err(|f| f.0)?;
    Ok(Step { name, opts: values })
}

/// Read and validate a batch; the reason when it is refused.
pub fn parse(text: &str, base: &Path, defaults: &Pair) -> Result<Vec<Job>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| l10n!("cli-batch-json", error = e))?;
    let Value::Object(head) = &v else {
        return Err(l10n!("cli-batch-json", error = "{}"));
    };
    let format = head.get("format").and_then(Value::as_u64).unwrap_or(0);
    if format != FORMAT {
        return Err(l10n!("cli-batch-format", format = format));
    }
    let text_of = |m: &serde_json::Map<String, Value>, k: &str| -> Option<String> {
        m.get(k).and_then(Value::as_str).map(str::to_string)
    };
    let pair0 = Pair {
        game: text_of(head, "game").unwrap_or_else(|| defaults.game.clone()),
        from: text_of(head, "from").unwrap_or_else(|| defaults.from.clone()),
        to: text_of(head, "to").unwrap_or_else(|| defaults.to.clone()),
    };
    let out0 = text_of(head, "out").unwrap_or_else(|| "out".into());
    let stop0 = head
        .get("stopOnError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let jobs = head
        .get("jobs")
        .and_then(Value::as_array)
        .filter(|j| !j.is_empty())
        .ok_or_else(|| l10n!("cli-batch-no-jobs"))?;
    let mut out = Vec::new();
    for (n, j) in jobs.iter().enumerate() {
        let Value::Object(m) = j else {
            return Err(l10n!("cli-batch-no-file", job = n + 1));
        };
        let steps: Vec<Step> = m
            .get("steps")
            .and_then(Value::as_array)
            .ok_or_else(|| l10n!("cli-batch-no-steps", job = n + 1))?
            .iter()
            .map(step_of)
            .collect::<Result<_, _>>()?;
        if steps.is_empty() {
            return Err(l10n!("cli-batch-no-steps", job = n + 1));
        }
        let pattern = text_of(m, "file")
            .or_else(|| text_of(m, "files"))
            .ok_or_else(|| l10n!("cli-batch-no-file", job = n + 1))?;
        let files = expand(base, &pattern);
        if files.is_empty() {
            return Err(l10n!("cli-err-file", path = base.join(&pattern).display()));
        }
        let pair = Pair {
            game: text_of(m, "game").unwrap_or_else(|| pair0.game.clone()),
            from: text_of(m, "from").unwrap_or_else(|| pair0.from.clone()),
            to: text_of(m, "to").unwrap_or_else(|| pair0.to.clone()),
        };
        let out_dir = base.join(text_of(m, "out").unwrap_or_else(|| out0.clone()));
        let stop = m
            .get("stopOnError")
            .and_then(Value::as_bool)
            .unwrap_or(stop0);
        for f in files {
            let stem = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(Job {
                out: out_dir.join(stem),
                file: f,
                pair: pair.clone(),
                stop_on_error: stop,
                steps: steps.clone(),
            });
        }
    }
    Ok(out)
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Run a batch file; the highest code met.
pub fn run(ctx: &mut super::work::Ctx, g: &Global, file: &str, o: &mut Out) -> i32 {
    let path = PathBuf::from(file);
    let Ok(text) = std::fs::read_to_string(&path) else {
        o.fail(
            &ctx.i18n,
            "batch",
            &Fail(l10n!("cli-err-file", path = path.display())),
        );
        return FAIL;
    };
    let base = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let defaults = commands::pair(ctx, g);
    let jobs = match parse(&text, &base, &defaults) {
        Ok(j) => j,
        Err(reason) => {
            o.fail(
                &ctx.i18n,
                "batch",
                &Fail(l10n!("cli-err-batch", reason = reason)),
            );
            return FAIL;
        }
    };
    let steps: usize = jobs.iter().map(|j| j.steps.len()).sum();
    let mut start = Reply::new("batch");
    start.line(l10n!("cli-batch-start", jobs = jobs.len(), steps = steps));
    start.set("jobs", jobs.len());
    start.set("steps", steps);
    start.set("event", "start");
    o.reply(&ctx.i18n, &start);
    let (mut ok, mut warn, mut failed) = (0usize, 0usize, 0usize);
    let mut code = OK;
    'jobs: for (n, job) in jobs.iter().enumerate() {
        let label = format!(
            "{} {}",
            n + 1,
            job.file
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        // The stack of the job: the options of its first apply step.
        let extra = job
            .steps
            .iter()
            .find(|s| s.name == "apply")
            .map(|s| TreeExtra {
                dicts: s
                    .opts
                    .all("dict")
                    .into_iter()
                    .map(|d| base.join(d))
                    .collect(),
                bases: s
                    .opts
                    .all("base")
                    .into_iter()
                    .map(|d| base.join(d))
                    .collect(),
                no_personal: s.opts.has("no-personal"),
            })
            .unwrap_or_default();
        let needs_doc = job.steps.iter().any(|s| s.name != "package");
        let mut work = if needs_doc {
            match Work::open(ctx, &job.file, &job.pair, Source::Tree(extra)) {
                Ok(w) => Some(w),
                Err(e) => {
                    let f = Fail(e);
                    say_step(ctx, o, &label, n, &job.steps[0].name, Err(&f));
                    failed += 1;
                    code = FAIL;
                    if job.stop_on_error {
                        break 'jobs;
                    }
                    continue;
                }
            }
        } else {
            None
        };
        for s in &job.steps {
            let res = match (&mut work, s.name.as_str()) {
                (_, "package") => {
                    let plugin = job
                        .file
                        .file_name()
                        .map(|x| x.to_string_lossy().to_string());
                    let _ = std::fs::create_dir_all(&job.out);
                    let mut opts = s.opts.clone();
                    if !opts.has("name") {
                        opts.set(
                            "name",
                            &job.file
                                .file_stem()
                                .map(|x| x.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        );
                    }
                    let pair = job.pair.clone();
                    commands::run_package(ctx, &pair, &job.out, &opts, plugin)
                }
                (Some(w), name) => commands::step(ctx, w, name, &s.opts, Some(&job.out)),
                (None, _) => Err(Fail(l10n!("cli-err-file", path = job.file.display()))),
            };
            match &res {
                Ok(r) if r.code == OK => ok += 1,
                Ok(r) => {
                    warn += 1;
                    code = code.max(r.code.min(WARN));
                    if r.code >= FAIL {
                        code = FAIL;
                    }
                }
                Err(_) => {
                    failed += 1;
                    code = FAIL;
                }
            }
            say_step(ctx, o, &label, n, &s.name, res.as_ref());
            let failed_now = match &res {
                Err(_) => true,
                Ok(r) => r.code >= FAIL,
            };
            if failed_now {
                if job.stop_on_error {
                    break 'jobs;
                }
                // The rest of the job depends on this step.
                continue 'jobs;
            }
        }
    }
    let mut end = Reply::new("batch");
    end.code = code;
    end.line(l10n!(
        "cli-batch-end",
        ok = ok,
        warn = warn,
        failed = failed,
        code = code
    ));
    end.set("event", "end");
    end.set("ok", ok);
    end.set("warn", warn);
    end.set("failed", failed);
    o.reply(&ctx.i18n, &end);
    code
}

fn say_step(
    ctx: &super::work::Ctx,
    o: &mut Out,
    label: &str,
    n: usize,
    step: &str,
    res: Result<&Reply, &Fail>,
) {
    match res {
        Ok(r) => {
            let mut r2 = Reply {
                code: r.code,
                lines: Vec::new(),
                notes: r.notes.clone(),
                json: r.json.clone(),
            };
            let first = r.lines.first().cloned().unwrap_or_default();
            r2.line(l10n!(
                "cli-batch-step",
                job = label,
                step = step,
                result = first
            ));
            // The other lines of export (one per file) stay readable.
            if step == "export" {
                for l in r.lines.iter().skip(1) {
                    r2.line(l10n!(
                        "cli-batch-step",
                        job = label,
                        step = step,
                        result = l
                    ));
                }
            }
            r2.set("job", n + 1);
            r2.set("step", step);
            o.reply(&ctx.i18n, &r2);
        }
        Err(f) => {
            let msg = ctx.i18n.localize(&f.0);
            let line = ctx.i18n.localize(&l10n!(
                "cli-batch-step",
                job = label,
                step = step,
                result = msg
            ));
            let _ = writeln_err(o, &line);
            if o.json {
                let j = serde_json::json!({ "command": step, "job": n + 1, "step": step,
                                            "code": FAIL, "error": msg });
                let _ = writeln!(o.out, "{j}");
            }
        }
    }
}

fn writeln_err(o: &mut Out, s: &str) -> std::io::Result<()> {
    writeln!(o.err, "{s}")
}

/// The path of a job's outputs, as the manual shows it.
pub fn job_dir_text(j: &Job) -> String {
    path_text(&j.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(glob_match("*.esp", "Apocalypse - Magic of Skyrim.esp"));
        assert!(glob_match("mod?.ESP", "mod1.esp"));
        assert!(!glob_match("*.esp", "mod.esm"));
        assert!(glob_match("a*b*c", "aXXbYc"));
        assert_eq!(kebab("writeDict"), "write-dict");
        assert_eq!(kebab("failOn"), "fail-on");
    }

    #[test]
    fn a_batch_is_refused_whole() {
        let dir = std::env::temp_dir().join(format!("ximod-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::write(dir.join("mods").join("a.esp"), b"x").unwrap();
        std::fs::write(dir.join("mods").join("b.esp"), b"x").unwrap();
        let d = Pair::default();
        let ok = r#"{"format":1,"to":"fra","out":"out","jobs":[{"files":"mods/*.esp",
            "steps":["apply",{"assist":{"derive":true}},"check",
            {"export":{"output":"dsd","statuses":["validated","pending"]}},"manifest"]}]}"#;
        let jobs = parse(ok, &dir, &d).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].steps.len(), 5);
        assert_eq!(jobs[1].out, dir.join("out").join("b"));
        assert_eq!(jobs[0].pair.to, "fra");
        assert_eq!(
            jobs[0].steps[3].opts.get("statuses"),
            Some("validated,pending")
        );
        for bad in [
            r#"{"format":2,"jobs":[{"file":"mods/a.esp","steps":["apply"]}]}"#,
            r#"{"format":1,"jobs":[]}"#,
            r#"{"format":1,"jobs":[{"file":"mods/zz.esp","steps":["apply"]}]}"#,
            r#"{"format":1,"jobs":[{"file":"mods/a.esp","steps":["frob"]}]}"#,
            r#"{"format":1,"jobs":[{"file":"mods/a.esp","steps":[{"export":{"output":"dsdx"}}]}]}"#,
            r#"{"format":1,"jobs":[{"file":"mods/a.esp","steps":[{"check":{"bogus":1}}]}]}"#,
            r#"{"format":1,"jobs":[{"file":"mods/a.esp","steps":[{"translate":{"service":"deepl"}}]}]}"#,
            "not json",
        ] {
            assert!(parse(bad, &dir, &d).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
